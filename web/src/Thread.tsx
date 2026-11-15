import { memo, useLayoutEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { Action, Image, Message, Preview, Source } from "./api";
import { CheckIcon, ChevronIcon, CloseIcon, DownloadIcon, GlobeIcon, ImageIcon, SpeakerIcon, StopIcon } from "./icons";

type Props = {
  chatId: string | null;
  messages: Message[];
  previews: Record<string, Preview>;
  /** fileUrl is where one of the chat's files, or an older chat's picture, is served. */
  fileUrl: (path: string) => string;
  /** onOpenFile shows one of the chat's files in the files panel; older chats have none. */
  onOpenFile?: (path: string) => void;
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

/** Images shows a reply's pictures at their own proportions, so loading does not move the page. */
function Images({ images, fileUrl }: { images: Image[]; fileUrl: (path: string) => string }) {
  return (
    <div className="images">
      {images.map((image) => {
        const url = fileUrl(image.name);
        return (
          <figure
            key={image.name}
            className="image"
            style={{
              aspectRatio: `${image.width} / ${image.height}`,
              // A tall image is limited by the window's height, so its box narrows with it.
              maxWidth: `min(480px, calc(70vh * ${image.width / image.height}))`,
            }}
          >
            <a href={url} target="_blank" rel="noreferrer" aria-label="Open image at full size">
              <img src={url} width={image.width} height={image.height} alt="Generated image" loading="lazy" />
            </a>
            <a className="image-download" href={url} download={image.name.split("/").pop()} aria-label="Download image" title="Download">
              <DownloadIcon />
            </a>
          </figure>
        );
      })}
    </div>
  );
}

/** VERBS names what each tool does while it works, once done, and in "couldn't …". */
const VERBS: Record<string, [string, string, string]> = {
  write_file: ["Writing", "Wrote", "write"],
  edit_file: ["Editing", "Edited", "edit"],
  delete_file: ["Deleting", "Deleted", "delete"],
  read_file: ["Reading", "Read", "read"],
  list_files: ["Listing", "Listed", "list"],
  search_text: ["Searching for", "Searched for", "search for"],
  generate_image: ["Creating", "Created", "create"],
};

/** Label says what one tool call did, in the words the thread shows. */
function label(action: Action): string {
  const [working, done, base] = VERBS[action.tool] ?? [`Using ${action.tool}`, `Used ${action.tool}`, `use ${action.tool}`];
  const listing = action.tool === "list_files" && (!action.target || action.target === ".");
  const target = listing ? "the folder" : action.tool === "search_text" ? `“${action.target ?? ""}”` : (action.target ?? "");
  switch (action.state) {
    case "working":
      return `${working} ${target}…`;
    case "failed":
      return `Couldn't ${base} ${target}`;
    case "stopped":
      return `${working} ${target} (stopped)`;
    default:
      return `${action.tool === "write_file" && action.operation === "create" ? "Created" : done} ${target}`;
  }
}

/** Opens says whether a finished call left a file worth opening from its row. */
function opens(action: Action): boolean {
  return action.state === "done" && !!action.target && ["write_file", "edit_file", "generate_image"].includes(action.tool);
}

function ActionRow({ action, onOpenFile }: { action: Action; onOpenFile?: (path: string) => void }) {
  const text = label(action);
  const target = action.target;
  return (
    <li className={`action ${action.state}`} title={action.error ?? undefined}>
      <span className="action-mark" aria-hidden>
        {action.state === "done" ? <CheckIcon /> : action.state === "failed" ? <CloseIcon /> : <span className="action-spinner" />}
      </span>
      {onOpenFile && opens(action) && target ? (
        <button className="action-open" onClick={() => onOpenFile(target)}>
          {text}
        </button>
      ) : (
        <span>{text}</span>
      )}
    </li>
  );
}

type ActivityProps = { actions: Action[]; live: boolean; onOpenFile?: (path: string) => void };

/** Activity lists a reply's tool calls: all of them while it works, folded to one line after. */
function Activity({ actions, live, onOpenFile }: ActivityProps) {
  const [open, setOpen] = useState(false);
  const failed = actions.filter((a) => a.state === "failed").length;
  const summary =
    actions.length === 1 ? label(actions[0]) : `${actions.length} actions${failed ? `, ${failed} failed` : ""}`;
  const rows = (
    <ul className="actions">
      {actions.map((action) => (
        <ActionRow key={action.call_id} action={action} onOpenFile={onOpenFile} />
      ))}
    </ul>
  );
  if (live) return <div className="activity">{rows}</div>;
  return (
    <div className="activity">
      <button className={`activity-toggle ${open ? "open" : ""}`} onClick={() => setOpen(!open)} aria-expanded={open}>
        <span>{summary}</span>
        <ChevronIcon />
      </button>
      {open && rows}
    </div>
  );
}

function Generating() {
  return (
    <div className="image-tile" role="status">
      <ImageIcon />
      <span>Creating image…</span>
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

export function Thread({ chatId, messages, previews, fileUrl, onOpenFile, onRetry, onSpeak, speaking }: Props) {
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
              {m.status === "pending" &&
              previews[m.id] &&
              (previews[m.id].text || previews[m.id].searching || previews[m.id].generating || previews[m.id].activity.length > 0) ? (
                <>
                  {previews[m.id].activity.length > 0 && <Activity actions={previews[m.id].activity} live onOpenFile={onOpenFile} />}
                  {previews[m.id].generating && <Generating />}
                  {previews[m.id].text && <Reply text={previews[m.id].text} />}
                  {previews[m.id].searching && <Searching />}
                </>
              ) : (
                <>
                  {m.activity.length > 0 && <Activity actions={m.activity} live={false} onOpenFile={onOpenFile} />}
                  {m.images.length > 0 && <Images images={m.images} fileUrl={fileUrl} />}
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
