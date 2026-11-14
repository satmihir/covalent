import { memo, useLayoutEffect, useRef } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Message } from "./api";

type Props = {
  chatId: string | null;
  messages: Message[];
  onRetry: (message: Message) => void;
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

function Status({ message, onRetry }: { message: Message; onRetry: (m: Message) => void }) {
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
          <button onClick={() => onRetry(message)}>Retry</button>
        </div>
      );
    default:
      return null;
  }
}

export function Thread({ chatId, messages, onRetry }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const stuck = useRef(true);

  useLayoutEffect(() => {
    stuck.current = true;
  }, [chatId]);

  // Follow new content only while the reader is already at the bottom.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && stuck.current) el.scrollTop = el.scrollHeight;
  }, [messages, chatId]);

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
              {m.text && <Reply text={m.text} />}
              <Status message={m} onRetry={onRetry} />
            </div>
          ),
        )}
      </div>
    </div>
  );
}
