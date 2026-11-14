import { useCallback, useEffect, useRef, useState } from "react";
import { api, describe, type Chat, type Message, type ServerEvent } from "./api";
import { Composer } from "./Composer";
import { MenuIcon, NewChatIcon } from "./icons";
import { Sidebar } from "./Sidebar";
import { Thread } from "./Thread";

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
  const [notice, setNotice] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const current = useRef(chatId);

  const select = useCallback((id: string | null) => {
    current.current = id;
    setChatId(id);
    setMessages([]);
    setNotice(null);
    setSidebarOpen(false);
  }, []);

  const load = useCallback(async (id: string) => {
    try {
      const { messages } = await api.chat(id);
      if (current.current === id) setMessages(messages);
    } catch (error) {
      if (current.current === id) setNotice(describe(error));
    }
  }, []);

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
    api.chats().then(setChats, (error) => setNotice(describe(error)));
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
        }
      } else if (event.type === "title") {
        setChats((list) => list.map((c) => (c.id === event.chat_id ? { ...c, title: event.title } : c)));
      } else {
        resync();
      }
    };
    return () => source.close();
  }, [resync]);

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
        const chat = await api.create();
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
      setNotice(describe(error));
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
    if (chatId) api.cancel(chatId).catch((error) => setNotice(describe(error)));
  };

  const last = messages[messages.length - 1];
  const running = last?.role === "assistant" && last.status === "pending";
  const title = chats.find((c) => c.id === chatId)?.title;
  const empty = !chatId && messages.length === 0;

  return (
    <div className="app">
      <Sidebar
        chats={chats}
        currentId={chatId}
        open={sidebarOpen}
        onSelect={(id) => open(id)}
        onNew={() => open(null)}
        onClose={() => setSidebarOpen(false)}
      />
      <main className={`main ${empty ? "empty" : ""}`}>
        <header className="topbar">
          <button className="icon-button mobile-only" onClick={() => setSidebarOpen(true)} aria-label="Open sidebar">
            <MenuIcon />
          </button>
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
          <Thread chatId={chatId} messages={messages} onRetry={retry} />
        )}
        <div className="composer-dock">
          {notice && (
            <div className="notice" role="alert">
              <span>{notice}</span>
              <button onClick={() => setNotice(null)} aria-label="Dismiss">
                ×
              </button>
            </div>
          )}
          <Composer
            value={draft}
            running={running}
            focusKey={chatId ?? "new"}
            onChange={setDraft}
            onSend={send}
            onStop={stop}
          />
          <p className="footnote">Covalent runs on re:agent. Replies can be wrong, so check what matters.</p>
        </div>
      </main>
    </div>
  );
}
