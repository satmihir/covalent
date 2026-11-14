import type { Chat } from "./api";
import { Logo, NewChatIcon } from "./icons";

type Props = {
  chats: Chat[];
  currentId: string | null;
  open: boolean;
  onSelect: (id: string) => void;
  onNew: () => void;
  onClose: () => void;
};

const DAY = 24 * 60 * 60 * 1000;

/** Group places a chat under a recency heading, measured from local midnight. */
function group(ms: number, midnight: number): string {
  if (ms >= midnight) return "Today";
  if (ms >= midnight - DAY) return "Yesterday";
  if (ms >= midnight - 7 * DAY) return "Previous 7 days";
  if (ms >= midnight - 30 * DAY) return "Previous 30 days";
  return "Older";
}

export function Sidebar({ chats, currentId, open, onSelect, onNew, onClose }: Props) {
  const midnight = new Date().setHours(0, 0, 0, 0);
  const groups: { label: string; chats: Chat[] }[] = [];
  for (const chat of chats) {
    const label = group(chat.updated_at_ms, midnight);
    const last = groups[groups.length - 1];
    if (last?.label === label) last.chats.push(chat);
    else groups.push({ label, chats: [chat] });
  }

  return (
    <>
      <div className={`scrim ${open ? "visible" : ""}`} onClick={onClose} />
      <aside className={`sidebar ${open ? "open" : ""}`}>
        <div className="sidebar-head">
          <a
            className="brand"
            href="/"
            onClick={(e) => {
              e.preventDefault();
              onNew();
            }}
          >
            <Logo />
            <span>Covalent</span>
          </a>
          <button className="icon-button" onClick={onNew} title="New chat" aria-label="New chat">
            <NewChatIcon />
          </button>
        </div>
        <nav className="chat-list">
          {groups.map((g) => (
            <section key={g.label}>
              <h2>{g.label}</h2>
              {g.chats.map((chat) => (
                <a
                  key={chat.id}
                  href={`/c/${chat.id}`}
                  className={`chat-link ${chat.id === currentId ? "active" : ""}`}
                  onClick={(e) => {
                    e.preventDefault();
                    onSelect(chat.id);
                  }}
                >
                  {chat.title ?? <span className="untitled">New chat</span>}
                </a>
              ))}
            </section>
          ))}
          {chats.length === 0 && <p className="chat-list-empty">Your chats will appear here.</p>}
        </nav>
      </aside>
    </>
  );
}
