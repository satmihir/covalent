import { useEffect, useState } from "react";
import { api, describe, type File, type Listing } from "./api";
import { decode, kind, size } from "./fileView";
import { BackIcon, CloseIcon, DownloadIcon } from "./icons";

type Props = {
  chatId: string;
  /** refresh changes whenever a reply may have changed the folder. */
  refresh: string;
  /** marked lists the files the latest reply created or changed. */
  marked: string[];
  /** selected is the file being viewed, or null for the list. */
  selected: string | null;
  onSelect: (path: string | null) => void;
  onClose: () => void;
};

/** Files lists a chat's folder beside the thread and shows one file at a time. */
export function Files({ chatId, refresh, marked, selected, onSelect, onClose }: Props) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    api.files(chatId).then(
      (next) => {
        if (!live) return;
        setListing(next);
        setError(null);
      },
      (failure) => live && setError(describe(failure)),
    );
    return () => {
      live = false;
    };
  }, [chatId, refresh]);

  const file = selected ? listing?.files.find((f) => f.path === selected) : undefined;
  const name = selected?.split("/").pop() ?? "Files";
  return (
    <aside className="files" aria-label="Files">
      <div className="files-head">
        {selected && (
          <button className="icon-button" onClick={() => onSelect(null)} aria-label="Back to files" title="Back">
            <BackIcon />
          </button>
        )}
        <span className="files-title" title={selected ?? undefined}>
          {name}
        </span>
        {file && (
          <a className="icon-button" href={api.fileUrl(chatId, file.path)} download={name} aria-label="Download" title="Download">
            <DownloadIcon />
          </a>
        )}
        <button className="icon-button" onClick={onClose} aria-label="Close files" title="Close">
          <CloseIcon />
        </button>
      </div>
      {error ? (
        <p className="files-note">{error}</p>
      ) : !listing ? null : selected ? (
        file ? (
          <Viewer chatId={chatId} file={file} />
        ) : (
          <p className="files-note">This file is no longer in the folder.</p>
        )
      ) : listing.files.length === 0 ? (
        <p className="files-note">No files yet. Ask for one and it will appear here.</p>
      ) : (
        <ul className="file-list">
          {listing.files.map((f) => (
            <li key={f.path}>
              <button onClick={() => onSelect(f.path)}>
                <span className="file-path">{f.path}</span>
                {marked.includes(f.path) && <span className="file-mark" title="Changed by the latest reply" />}
                <span className="file-size">{size(f.size)}</span>
              </button>
            </li>
          ))}
          {listing.more && <li className="files-note">Only the first {listing.files.length} files are listed.</li>}
        </ul>
      )}
    </aside>
  );
}

/** Viewer shows a picture as itself, a small text file as plain text, and anything else as a
 *  name and size to download. It reloads when the file changes. */
function Viewer({ chatId, file }: { chatId: string; file: File }) {
  const url = `${api.fileUrl(chatId, file.path)}?v=${file.modified_at_ms}`;
  const shown = kind(file.path, file.size);
  const [text, setText] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    if (shown !== "text") return;
    let live = true;
    setText(undefined);
    fetch(url)
      .then((response) => (response.ok ? response.arrayBuffer() : Promise.reject()))
      .then((bytes) => live && setText(decode(bytes)))
      .catch(() => live && setText(null));
    return () => {
      live = false;
    };
  }, [url, shown]);

  if (shown === "picture") {
    return (
      <div className="file-view picture">
        <img src={url} alt={file.path} />
      </div>
    );
  }
  if (shown === "text" && text === undefined) return null;
  if (shown === "text" && text !== null) {
    return <pre className="file-view text">{text}</pre>;
  }
  return (
    <div className="file-view other">
      <p>
        {shown === "text" ? "This file is not text." : "This file is too large to show here."} {size(file.size)}.
      </p>
      <a className="file-download" href={url} download={file.path.split("/").pop()}>
        <DownloadIcon /> Download
      </a>
    </div>
  );
}
