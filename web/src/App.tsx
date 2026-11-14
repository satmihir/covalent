import { useCallback, useEffect, useRef, useState } from "react";
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
import { Composer } from "./Composer";
import { MenuIcon, NewChatIcon } from "./icons";
import { ModelPicker } from "./ModelPicker";
import { Sidebar } from "./Sidebar";
import { Thread } from "./Thread";

type Notice = { text: string; tone: "error" | "info" };

const MODEL_KEY = "covalent.model";

/** Remembered reads this viewer's last model choice for new chats; storage may be unavailable. */
function remembered(): string | null {
  try {
    return localStorage.getItem(MODEL_KEY);
  } catch {
    return null;
  }
}

function remember(model: string) {
  try {
    localStorage.setItem(MODEL_KEY, model);
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
  const [notice, setNotice] = useState<Notice | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  // openChat is the open chat as last read, including an archived one the sidebar omits.
  const [openChat, setOpenChat] = useState<Chat | null>(null);
  const [models, setModels] = useState<Model[]>([]);
  const [defaultModel, setDefaultModel] = useState<string | null>(null);
  const [newModel, setNewModel] = useState<string | null>(remembered);
  const current = useRef(chatId);
  // Previews change many times a second; the ref is applied at once and shown once per frame.
  const previewRef = useRef<Record<string, Preview>>({});
  const [previews, setPreviews] = useState<Record<string, Preview>>({});
  const queued = useRef<PreviewEvent[]>([]);
  const frame = useRef(0);

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
    },
    [showPreviews],
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
    api.models().then(({ models, default: fallback }) => {
      setModels(models);
      setDefaultModel(fallback);
      // A remembered model this server no longer offers falls back to the default.
      setNewModel((chosen) => (chosen && models.some((m) => m.id === chosen) ? chosen : null));
    }, () => {});
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
      } else if (event.type === "preview") {
        queued.current.push(event);
        if (!frame.current) frame.current = requestAnimationFrame(flushPreviews);
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

  const send = async (text: string) => {
    const requestId = crypto.randomUUID();
    const now = Date.now();
    const optimistic: Message[] = [
      { id: requestId, chat_id: chatId ?? "", role: "user", text, status: "complete", reason: null, created_at_ms: now },
      { id: `local:${requestId}`, chat_id: chatId ?? "", role: "assistant", text: "", status: "pending", reason: null, created_at_ms: now },
    ];
    const isOptimistic = (m: Message) => optimistic.some((o) => o.id === m.id);
    setNotice(null);
    setDraft("");
    setMessages((list) => [...list, ...optimistic]);
    let id = current.current;
    try {
      if (!id) {
        const chat = await api.create(newModel);
        setOpenChat(chat);
        setChats((list) => [chat, ...list]);
        id = chat.id;
        current.current = id;
        setChatId(id);
        history.pushState(null, "", `/c/${id}`);
      }
      const server = await api.send(id, requestId, text);
      if (current.current === id) setMessages((list) => reconcile(server, list));
      setChats((list) => bump(list, id!));
    } catch (error) {
      if (current.current !== id) return;
      setMessages((list) => list.filter((m) => !isOptimistic(m)));
      setDraft(text);
      setNotice({ text: describe(error), tone: "error" });
    }
  };

  const chooseModel = async (model: string) => {
    if (!chatId) {
      setNewModel(model);
      remember(model);
      return;
    }
    try {
      const chat = await api.switchModel(chatId, model);
      remember(model);
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
    if (prompt) void send(prompt.text);
  };

  const stop = () => {
    if (chatId) api.cancel(chatId).catch((error) => setNotice({ text: describe(error), tone: "error" }));
  };

  const last = messages[messages.length - 1];
  const running = last?.role === "assistant" && last.status === "pending";
  const title = chats.find((c) => c.id === chatId)?.title ?? openChat?.title;
  const empty = !chatId && messages.length === 0;
  const archived = openChat?.archived_at_ms != null;
  const model = chatId ? (openChat?.model ?? null) : (newModel ?? defaultModel);

  return (
    <div className="app">
      <Sidebar
        chats={chats}
        currentId={chatId}
        open={sidebarOpen}
        onSelect={(id) => open(id)}
        onNew={() => open(null)}
        onArchive={(id) => void archive(id)}
        onClose={() => setSidebarOpen(false)}
      />
      <main className={`main ${empty ? "empty" : ""}`}>
        <header className="topbar">
          <button className="icon-button mobile-only" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <MenuIcon />
          </button>
          <ModelPicker
            models={models}
            value={model}
            disabled={running || archived || (chatId !== null && !openChat)}
            onChange={(m) => void chooseModel(m)}
          />
          <span className="topbar-title">{chatId ? (title ?? "New chat") : ""}</span>
          <button className="icon-button mobile-only" onClick={() => open(null)} aria-label="New chat">
            <NewChatIcon />
          </button>
        </header>
        {empty ? (
          <div className="welcome">
            <h1>What can I help with?</h1>
          </div>
        ) : (
          <Thread chatId={chatId} messages={messages} previews={previews} onRetry={archived ? undefined : retry} />
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
          ) : (
            <Composer
              value={draft}
              running={running}
              focusKey={chatId ?? "new"}
              onChange={setDraft}
              onSend={send}
              onStop={stop}
            />
          )}
          <p className="footnote">Covalent runs on re:agent. Replies can be wrong, so check what matters.</p>
        </div>
      </main>
    </div>
  );
}
