import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  describe,
  type Chat,
  type Message,
  type Model,
  type Preview,
  type PreviewEvent,
  type ServerEvent,
} from "./api";
import { play, type Playback } from "./audio";
import { Composer } from "./Composer";
import type { Reply } from "./voiceLoop";

// Voice mode brings a speech detector and ONNX Runtime; they load when it is first opened.
const Voice = lazy(() => import("./Voice").then((m) => ({ default: m.Voice })));
import { accept } from "./attachments";
import { Files } from "./Files";
import { nativeApp, VOICE_ENDED } from "./nativeApp";
import { touched } from "./fileView";
import { FolderIcon, MenuIcon, NewChatIcon } from "./icons";
import { ModelPicker } from "./ModelPicker";
import type { Choice } from "./modelGrid";
import { Personalization } from "./Personalization";
import { Sidebar } from "./Sidebar";
import { Thread } from "./Thread";

type Notice = { text: string; tone: "error" | "info" };

const MODEL_KEY = "covalent.model";
const EFFORT_KEY = "covalent.effort";

/** Remembered reads this viewer's last model and effort for new chats; storage may be unavailable. */
function remembered(): Choice | null {
  try {
    const model = localStorage.getItem(MODEL_KEY);
    return model ? { model, effort: localStorage.getItem(EFFORT_KEY) } : null;
  } catch {
    return null;
  }
}

function remember(choice: Choice) {
  try {
    localStorage.setItem(MODEL_KEY, choice.model);
    localStorage.setItem(EFFORT_KEY, choice.effort ?? "");
  } catch {
    // Remembering the choice is a convenience only.
  }
}

function idFromPath(): string | null {
  return /^\/c\/([0-9a-f]{16})$/.exec(location.pathname)?.[1] ?? null;
}

/** Upsert replaces a message by id, or appends it. */
function upsert(list: Message[], message: Message): Message[] {
  const i = list.findIndex((m) => m.id === message.id);
  if (i < 0) return [...list, message];
  const next = list.slice();
  next[i] = message;
  return next;
}

/** Reconcile adopts the server's list but keeps any reply an event already finished. */
function reconcile(server: Message[], local: Message[]): Message[] {
  return server.map((m) => {
    const seen = local.find((l) => l.id === m.id);
    return seen && m.status === "pending" && seen.status !== "pending" ? seen : m;
  });
}

/** Bump moves a chat to the top of the list as the most recently active. */
function bump(chats: Chat[], id: string): Chat[] {
  const chat = chats.find((c) => c.id === id);
  if (!chat) return chats;
  return [{ ...chat, updated_at_ms: Date.now() }, ...chats.filter((c) => c.id !== id)];
}

export function App() {
  const [chats, setChats] = useState<Chat[]>([]);
  const [chatId, setChatId] = useState<string | null>(idFromPath);
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState("");
  // Attachments wait here until the message they belong to is sent.
  const [attached, setAttached] = useState<File[]>([]);
  const [dropping, setDropping] = useState(false);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [personalizing, setPersonalizing] = useState(false);
  // openChat is the open chat as last read, including an archived one the sidebar omits.
  const [openChat, setOpenChat] = useState<Chat | null>(null);
  const [models, setModels] = useState<Model[]>([]);
  const [defaultChoice, setDefaultChoice] = useState<Choice | null>(null);
  const [newChoice, setNewChoice] = useState<Choice | null>(remembered);
  const current = useRef(chatId);
  // Previews change many times a second; the ref is applied at once and shown once per frame.
  const previewRef = useRef<Record<string, Preview>>({});
  const [previews, setPreviews] = useState<Record<string, Preview>>({});
  const queued = useRef<PreviewEvent[]>([]);
  const frame = useRef(0);
  const [voice, setVoice] = useState(false);
  const [speaking, setSpeaking] = useState<string | null>(null);
  const playback = useRef<Playback | null>(null);
  // Voice mode replaces the composer; voiceReply is the reply it is waiting on or reading.
  const [voiceMode, setVoiceMode] = useState(false);
  const [voiceReply, setVoiceReply] = useState<string | null>(null);
  // The files panel stays open from chat to chat; the file it shows belongs to one chat.
  const [filesOpen, setFilesOpen] = useState(false);
  const [file, setFile] = useState<string | null>(null);
  // Voice mode asks about replies from callbacks, between renders.
  const messagesRef = useRef<Message[]>([]);
  messagesRef.current = messages;

  const stopSpeaking = useCallback(() => {
    playback.current?.stop();
    playback.current = null;
    setSpeaking(null);
  }, []);

  const showPreviews = useCallback((next: Record<string, Preview>) => {
    previewRef.current = next;
    setPreviews(next);
  }, []);

  const select = useCallback(
    (id: string | null) => {
      current.current = id;
      setChatId(id);
      setMessages([]);
      setOpenChat(null);
      showPreviews({});
      setNotice(null);
      setSidebarOpen(false);
      stopSpeaking();
      setVoiceMode(false);
      setFile(null);
      setAttached([]);
    },
    [showPreviews, stopSpeaking],
  );

  const load = useCallback(
    async (id: string) => {
      try {
        const { chat, messages, previews } = await api.chat(id);
        if (current.current !== id) return;
        setOpenChat(chat);
        setMessages(messages);
        showPreviews(previews);
      } catch (error) {
        if (current.current === id) setNotice({ text: describe(error), tone: "error" });
      }
    },
    [showPreviews],
  );

  /** FlushPreviews applies queued preview events in order; a missed one means reloading the chat. */
  const flushPreviews = useCallback(() => {
    frame.current = 0;
    const next = { ...previewRef.current };
    let gap = false;
    for (const event of queued.current.splice(0)) {
      if (event.chat_id !== current.current) continue;
      const known = next[event.message_id];
      const seq = known?.seq ?? 0;
      if (event.seq <= seq) continue;
      if (event.seq > seq + 1) {
        gap = true;
        continue;
      }
      next[event.message_id] = {
        seq: event.seq,
        text: event.restarted || !known ? event.text : known.text + event.text,
        searching: event.searching,
        generating: event.generating,
        activity: known?.activity ?? [],
      };
    }
    showPreviews(next);
    if (gap && current.current) void load(current.current);
  }, [load, showPreviews]);

  const open = useCallback(
    (id: string | null, push = true) => {
      if (push) history.pushState(null, "", id ? `/c/${id}` : "/");
      select(id);
      if (id) void load(id);
    },
    [select, load],
  );

  const resync = useCallback(() => {
    api.chats().then(setChats, () => {});
    if (current.current) void load(current.current);
  }, [load]);

  useEffect(() => {
    api.chats().then(setChats, (error) => setNotice({ text: describe(error), tone: "error" }));
    api.models().then(({ models, default: fallback, default_effort }) => {
      setModels(models);
      setDefaultChoice(fallback ? { model: fallback, effort: default_effort } : null);
      // A remembered model this server no longer offers falls back to the default, and a
      // remembered effort the model does not take to the model's own.
      setNewChoice((chosen) => {
        const model = models.find((m) => m.id === chosen?.model);
        if (!chosen || !model) return null;
        const kept = chosen.effort !== null && model.efforts.includes(chosen.effort);
        return { model: model.id, effort: kept ? chosen.effort : model.effort };
      });
    }, () => {});
    api.voice().then(setVoice, () => {});
    open(idFromPath(), false);
    const onPop = () => open(idFromPath(), false);
    addEventListener("popstate", onPop);
    return () => removeEventListener("popstate", onPop);
  }, [open]);

  useEffect(() => {
    const source = new EventSource("/api/events");
    let dropped = false;
    source.onerror = () => {
      dropped = true;
    };
    // Events are not replayed, so after a reconnect the page reloads what it shows.
    source.onopen = () => {
      if (dropped) resync();
      dropped = false;
    };
    source.onmessage = (e) => {
      const event = JSON.parse(e.data) as ServerEvent;
      if (event.type === "message") {
        if (event.message.chat_id === current.current) {
          setMessages((list) => upsert(list, event.message));
          // The final reply replaces its preview, including a stopped reply's partial text.
          const { [event.message.id]: _done, ...rest } = previewRef.current;
          showPreviews(rest);
        }
      } else if (event.type === "admitted") {
        // A message this page sent is already shown, and its own send response reconciles it.
        const [user] = event.messages;
        if (event.chat_id === current.current) {
          setMessages((list) => (list.some((m) => m.id === `local:${user.id}`) ? list : event.messages.reduce(upsert, list)));
        }
        setChats((list) => bump(list, event.chat_id));
      } else if (event.type === "preview") {
        queued.current.push(event);
        if (!frame.current) frame.current = requestAnimationFrame(flushPreviews);
      } else if (event.type === "activity") {
        // Each event carries the whole list, so the latest one is all that matters.
        if (event.chat_id === current.current) {
          const known = previewRef.current[event.message_id];
          const empty = { seq: 0, text: "", searching: false, generating: false };
          showPreviews({ ...previewRef.current, [event.message_id]: { ...(known ?? empty), activity: event.activity } });
        }
      } else if (event.type === "title") {
        setChats((list) => list.map((c) => (c.id === event.chat_id ? { ...c, title: event.title } : c)));
      } else if (event.type === "archived") {
        setChats((list) => list.filter((c) => c.id !== event.chat_id));
        if (event.chat_id === current.current) void load(event.chat_id);
      } else {
        resync();
      }
    };
    return () => {
      source.close();
      cancelAnimationFrame(frame.current);
      frame.current = 0;
    };
  }, [resync, flushPreviews, showPreviews]);

  // In the iPhone app, voice mode is the app's: it says when it has ended, and is told when
  // this page ends it, by switching chats for one.
  useEffect(() => {
    const ended = (event: Event) => {
      setVoiceMode(false);
      const reason = (event as CustomEvent<string | null>).detail;
      if (reason) setNotice({ text: reason, tone: "error" });
    };
    addEventListener(VOICE_ENDED, ended);
    return () => removeEventListener(VOICE_ENDED, ended);
  }, []);
  useEffect(() => {
    const app = nativeApp;
    if (!voiceMode || !app) return;
    return () => app.postMessage({ voice: null });
  }, [voiceMode]);

  /** OpenedChat returns the open chat's ID, creating the chat when a new one is showing. */
  const openedChat = async (): Promise<string> => {
    if (current.current) return current.current;
    const chat = await api.create(newChoice?.model ?? null, newChoice?.effort ?? null);
    setOpenChat(chat);
    setChats((list) => [chat, ...list]);
    current.current = chat.id;
    setChatId(chat.id);
    history.pushState(null, "", `/c/${chat.id}`);
    return chat.id;
  };

  /** Send posts a message, creating the chat first if needed, and returns where its reply is.
   *  New files are uploaded first; files a retried message already sent are named again. */
  const send = async (
    text: string,
    spoken = false,
    { files = [], paths = [] }: { files?: File[]; paths?: string[] } = {},
  ): Promise<Reply | null> => {
    const requestId = crypto.randomUUID();
    const now = Date.now();
    const optimistic: Message[] = [
      { id: requestId, chat_id: chatId ?? "", role: "user", text, status: "complete", reason: null, created_at_ms: now, sources: [], images: [], activity: [], attachments: [] },
      { id: `local:${requestId}`, chat_id: chatId ?? "", role: "assistant", text: "", status: "pending", reason: null, created_at_ms: now, sources: [], images: [], activity: [], attachments: [] },
    ];
    const isOptimistic = (m: Message) => optimistic.some((o) => o.id === m.id);
    setNotice(null);
    setDraft("");
    setAttached([]);
    setMessages((list) => [...list, ...optimistic]);
    let id = current.current;
    try {
      id = await openedChat();
      const uploaded = [...paths];
      for (const file of files) {
        uploaded.push((await api.upload(id, file)).path);
      }
      const server = await api.send(id, requestId, text, spoken, uploaded);
      if (current.current === id) setMessages((list) => reconcile(server, list));
      setChats((list) => bump(list, id!));
      const reply = server[server.length - 1];
      return reply?.role === "assistant" ? { chatId: id, replyId: reply.id } : null;
    } catch (error) {
      if (current.current !== id) return null;
      setMessages((list) => list.filter((m) => !isOptimistic(m)));
      if (!spoken) setDraft(text);
      setAttached(files);
      setNotice({ text: describe(error), tone: "error" });
      return null;
    }
  };

  const sendSpoken = async (text: string) => {
    const reply = await send(text, true);
    if (reply) setVoiceReply(reply.replyId);
    return reply;
  };

  /** Pending reports whether a reply is unfinished; one not shown yet was only just sent. */
  const pending = (replyId: string) => {
    const reply = messagesRef.current.find((m) => m.id === replyId);
    return reply ? reply.status === "pending" : true;
  };

  const startVoiceMode = async () => {
    stopSpeaking();
    setNotice(null);
    setVoiceReply(null);
    if (nativeApp) {
      // The app talks in a chat that exists, so a new one is created before it starts.
      try {
        nativeApp.postMessage({ voice: await openedChat() });
      } catch (error) {
        return setNotice({ text: describe(error), tone: "error" });
      }
    }
    setVoiceMode(true);
  };

  const chooseModel = async (model: string, effort: string) => {
    const choice = { model, effort };
    if (!chatId) {
      setNewChoice(choice);
      remember(choice);
      return;
    }
    try {
      const chat = await api.switchModel(chatId, model, effort);
      remember(choice);
      if (current.current === chatId) setOpenChat(chat);
      setChats((list) => list.map((c) => (c.id === chat.id ? chat : c)));
    } catch (error) {
      setNotice({ text: describe(error), tone: "error" });
    }
  };

  const archive = async (id: string) => {
    try {
      await api.archive(id);
      setChats((list) => list.filter((c) => c.id !== id));
      if (current.current === id) open(null);
      setNotice({ text: `Chat archived. Continue it in a terminal with reagent --resume ${id}`, tone: "info" });
    } catch (error) {
      setNotice({ text: describe(error), tone: "error" });
    }
  };

  const retry = (reply: Message) => {
    const i = messages.indexOf(reply);
    const prompt = messages
      .slice(0, i)
      .reverse()
      .find((m) => m.role === "user");
    if (prompt) void send(prompt.text, false, { paths: prompt.attachments.map((a) => a.path) });
  };

  // Attachments build on the latest list, since a paste and a drop can land before a render.
  const attach = (incoming: File[]) => {
    setAttached((current) => {
      const { files, refused } = accept(current, incoming);
      if (refused) setNotice({ text: refused, tone: "error" });
      return files;
    });
  };

  const speak = (reply: Message) => {
    const again = playback.current !== null && speaking === reply.id;
    stopSpeaking();
    if (again || !chatId) return;
    const current = play(api.speechUrl(chatId, reply.id));
    playback.current = current;
    setSpeaking(reply.id);
    current.done
      .catch((error) => setNotice({ text: describe(error), tone: "error" }))
      .finally(() => {
        if (playback.current !== current) return;
        playback.current = null;
        setSpeaking(null);
      });
  };

  const stop = () => {
    if (chatId) api.cancel(chatId).catch((error) => setNotice({ text: describe(error), tone: "error" }));
  };

  const last = messages[messages.length - 1];
  const running = last?.role === "assistant" && last.status === "pending";
  const title = chats.find((c) => c.id === chatId)?.title ?? openChat?.title;
  const empty = !chatId && messages.length === 0;
  const archived = openChat?.archived_at_ms != null;
  const opened = openChat?.model ? { model: openChat.model, effort: openChat.effort } : null;
  const choice: Choice | null = chatId ? opened : (newChoice ?? defaultChoice);
  const folder = chatId && openChat?.workspace ? chatId : null;
  // A new chat will have a folder; an older one without a folder takes no files.
  const takesFiles = !archived && !voiceMode && (!chatId || !!openChat?.workspace);
  // The latest reply's work marks the files it changed and refreshes the panel as it goes.
  const reply = [...messages].reverse().find((m) => m.role === "assistant");
  const work = (reply?.status === "pending" ? previews[reply.id]?.activity : reply?.activity) ?? [];
  const refresh = `${reply?.id}:${reply?.status}:${work.map((a) => a.state).join()}`;
  const openFile = (path: string) => {
    setFile(path);
    setFilesOpen(true);
  };

  return (
    <div className="app">
      <Sidebar
        chats={chats}
        currentId={chatId}
        open={sidebarOpen}
        onSelect={(id) => open(id)}
        onNew={() => open(null)}
        onArchive={(id) => void archive(id)}
        onPersonalize={() => setPersonalizing(true)}
        onClose={() => setSidebarOpen(false)}
      />
      <main
        className={`main ${empty ? "empty" : ""} ${dropping ? "dropping" : ""}`}
        onDragOver={(e) => {
          if (!takesFiles || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
          setDropping(true);
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
        }}
        onDrop={(e) => {
          setDropping(false);
          if (!takesFiles || e.dataTransfer.files.length === 0) return;
          e.preventDefault();
          attach([...e.dataTransfer.files]);
        }}
      >
        <header className="topbar">
          <button className="icon-button mobile-only" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <MenuIcon />
          </button>
          <ModelPicker
            models={models}
            value={choice}
            disabled={running || archived || (chatId !== null && !openChat)}
            onChange={(model, effort) => void chooseModel(model, effort)}
          />
          <span className="topbar-title">{chatId ? (title ?? "New chat") : ""}</span>
          {folder && (
            <button
              className={`icon-button ${filesOpen ? "active" : ""}`}
              onClick={() => setFilesOpen(!filesOpen)}
              aria-label="Files"
              aria-pressed={filesOpen}
              title="Files"
            >
              <FolderIcon />
            </button>
          )}
          <button className="icon-button mobile-only" onClick={() => open(null)} aria-label="New chat">
            <NewChatIcon />
          </button>
        </header>
        {empty ? (
          <div className="welcome">
            <h1>What can I help with?</h1>
          </div>
        ) : (
          <Thread
            chatId={chatId}
            messages={messages}
            previews={previews}
            fileUrl={(path) => (openChat?.workspace && chatId ? api.fileUrl(chatId, path) : api.imageUrl(path))}
            onOpenFile={folder ? openFile : undefined}
            onRetry={archived ? undefined : retry}
            onSpeak={voice && !voiceMode ? speak : undefined}
            speaking={speaking}
          />
        )}
        <div className="composer-dock">
          {notice && (
            <div className={`notice ${notice.tone}`} role={notice.tone === "error" ? "alert" : "status"}>
              <span>{notice.text}</span>
              <button onClick={() => setNotice(null)} aria-label="Dismiss">
                ×
              </button>
            </div>
          )}
          {archived ? (
            <p className="archived-note">
              This chat is archived. Continue it in a terminal with <code>reagent --resume {chatId}</code>
            </p>
          ) : voiceMode && nativeApp ? (
            <div className="voice-native" />
          ) : voiceMode ? (
            <Suspense fallback={<p className="voice-status voice-loading">Starting voice mode…</p>}>
              <Voice
              send={sendSpoken}
              pending={pending}
              cancel={(id) => void api.cancel(id).catch(() => {})}
              searching={voiceReply !== null && previews[voiceReply]?.searching === true}
              onError={(text) => setNotice({ text, tone: "error" })}
              onClose={() => setVoiceMode(false)}
              />
            </Suspense>
          ) : (
            <Composer
              value={draft}
              running={running}
              focusKey={chatId ?? "new"}
              onChange={setDraft}
              onSend={(text) => void send(text, false, { files: attached })}
              files={takesFiles ? attached : null}
              onAttach={attach}
              onRemove={(file) => setAttached((list) => list.filter((f) => f !== file))}
              onStop={stop}
              voice={voice}
              onVoiceMode={() => void startVoiceMode()}
              onError={(text) => setNotice({ text, tone: "error" })}
            />
          )}
          <p className="footnote">Covalent runs on re:agent. Replies can be wrong, so check what matters.</p>
        </div>
      </main>
      {folder && filesOpen && (
        <Files
          chatId={folder}
          refresh={refresh}
          marked={touched(work)}
          selected={file}
          onSelect={setFile}
          onClose={() => setFilesOpen(false)}
        />
      )}
      {personalizing && <Personalization onClose={() => setPersonalizing(false)} />}
    </div>
  );
}
