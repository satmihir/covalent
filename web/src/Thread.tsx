import { memo, useLayoutEffect, useRef } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Message, Preview, Source } from "./api";
import { GlobeIcon, SpeakerIcon, StopIcon } from "./icons";

type Props = {
  chatId: string | null;
  messages: Message[];
  previews: Record<string, Preview>;
  /** onRetry is absent when the chat cannot take messages, as when it is archived. */
  onRetry?: (message: Message) => void;
  /** onSpeak reads a finished reply aloud, or stops it; it is absent without voice. */
  onSpeak?: (message: Message) => void;
  /** speaking is the ID of the reply being read aloud. */
  speaking: string | null;
};

const Reply = memo(function Reply({ text }: { text: string }) {
  return (
    <div className="markdown">
      <Markdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer" />,
        }}
      >
        {text}
      </Markdown>
    </div>
  );
});

/** Link drops OpenAI's tracking parameter; the cited page is the same. */
function link(url: string): { href: string; host: string } {
  try {
    const parsed = new URL(url);
    parsed.searchParams.delete("utm_source");
    return { href: parsed.toString(), host: parsed.hostname.replace(/^www\./, "") };
  } catch {
    return { href: url, host: url };
  }
}

function Sources({ sources }: { sources: Source[] }) {
  return (
    <div className="sources" aria-label="Sources">
      {sources.map((source, i) => {
        const { href, host } = link(source.url);
        return (
          <a key={source.url} className="source" href={href} target="_blank" rel="noreferrer" title={source.title || href}>
            <span className="source-number">{i + 1}</span>
            {host}
          </a>
        );
      })}
    </div>
  );
}

function Searching() {
  return (
    <div className="searching" role="status">
      <GlobeIcon />
      <span>Searching the web…</span>
    </div>
  );
}

function Status({ message, onRetry }: { message: Message; onRetry?: (m: Message) => void }) {
  switch (message.status) {
    case "pending":
      return (
        <div className="thinking" aria-label="Thinking">
          <span />
          <span />
          <span />
        </div>
      );
    case "cancelled":
      return <p className="status-line">Stopped</p>;
    case "failed":
    case "interrupted":
      return (
        <div className="reply-error">
          <span>{message.reason ?? "This reply didn't finish."}</span>
          {onRetry && <button onClick={() => onRetry(message)}>Retry</button>}
        </div>
      );
    default:
      return null;
  }
}

function ReadAloud({ active, onClick }: { active: boolean; onClick: () => void }) {
  const label = active ? "Stop reading" : "Read aloud";
  return (
    <div className="reply-actions">
      <button className={`action-button ${active ? "active" : ""}`} onClick={onClick} aria-label={label} title={label}>
        {active ? <StopIcon /> : <SpeakerIcon />}
      </button>
    </div>
  );
}

export function Thread({ chatId, messages, previews, onRetry, onSpeak, speaking }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const stuck = useRef(true);

  useLayoutEffect(() => {
    stuck.current = true;
  }, [chatId]);

  // Follow new content only while the reader is already at the bottom.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stuck.current) el.scrollTop = el.scrollHeight;
  }, [messages, previews, chatId]);

  return (
    <div
      className="thread"
      ref={scroller}
      onScroll={(e) => {
        const el = e.currentTarget;
        stuck.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
      }}
    >
      <div className="thread-inner">
        {messages.map((m) =>
          m.role === "user" ? (
            <div key={m.id} className="row user">
              <div className="bubble">{m.text}</div>
            </div>
          ) : (
            <div key={m.id} className="row assistant">
              {m.status === "pending" && previews[m.id] && (previews[m.id].text || previews[m.id].searching) ? (
                <>
                  {previews[m.id].text && <Reply text={previews[m.id].text} />}
                  {previews[m.id].searching && <Searching />}
                </>
              ) : (
                <>
                  {m.text && <Reply text={m.text} />}
                  {m.status === "complete" && m.sources.length > 0 && <Sources sources={m.sources} />}
                  {onSpeak && m.status === "complete" && m.text && (
                    <ReadAloud active={speaking === m.id} onClick={() => onSpeak(m)} />
                  )}
                  <Status message={m} onRetry={onRetry} />
                </>
              )}
            </div>
          ),
        )}
      </div>
    </div>
  );
}
