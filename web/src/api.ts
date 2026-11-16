export type Chat = {
  id: string;
  title: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  /** model is null for chats created before Covalent recorded it. */
  model: string | null;
  archived_at_ms: number | null;
  /** workspace names the chat's folder; chats made before folders have none. */
  workspace: string | null;
};

export type Model = { id: string; provider: string; note: string };

export type MessageStatus = "pending" | "complete" | "failed" | "cancelled" | "interrupted";

export type Source = { url: string; title: string };

/** Image is a picture a reply made: a path in the chat's folder, or for an older chat a name
 *  served at `/api/images/{name}`. */
export type Image = { name: string; width: number; height: number };

/** Action is one tool call a reply made. */
export type Action = {
  call_id: string;
  tool: string;
  /** target is the path or search text the call names. */
  target: string | null;
  state: "working" | "done" | "failed" | "stopped";
  /** operation is what a file tool reported doing, such as "create" or "update". */
  operation: string | null;
  error: string | null;
};

/** Attachment is a file the user sent with a message, saved in the chat's folder. */
export type Attachment = { path: string; size: number };

/** FolderFile is one file in a chat's folder. */
export type FolderFile = { path: string; size: number; modified_at_ms: number };

/** Listing is a chat's files in path order; more says some were left out. */
export type Listing = { files: FolderFile[]; more: boolean };

export type Message = {
  id: string;
  chat_id: string;
  role: "user" | "assistant";
  text: string;
  status: MessageStatus;
  reason: string | null;
  created_at_ms: number;
  /** sources lists the pages a reply cites from web search. */
  sources: Source[];
  /** images lists the pictures a reply made. */
  images: Image[];
  /** activity lists the tool calls a reply made. */
  activity: Action[];
  /** attachments lists the files the user sent with a message. */
  attachments: Attachment[];
};

/** Preview is a pending reply's text so far, numbered by its latest preview event. */
export type Preview = {
  seq: number;
  text: string;
  searching: boolean;
  generating: boolean;
  activity: Action[];
};

export type PreviewEvent = {
  type: "preview";
  chat_id: string;
  message_id: string;
  seq: number;
  restarted: boolean;
  text: string;
  searching: boolean;
  generating: boolean;
};

export type ServerEvent =
  | { type: "message"; message: Message }
  | { type: "title"; chat_id: string; title: string }
  | PreviewEvent
  | { type: "activity"; chat_id: string; message_id: string; activity: Action[] }
  | { type: "archived"; chat_id: string }
  | { type: "lagged" };

export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method,
    headers: body === undefined ? undefined : { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new ApiError(data.error ?? "http", data.message ?? `Request failed (${response.status})`);
  }
  return data as T;
}

export const api = {
  chats: () => request<{ chats: Chat[] }>("GET", "/api/chats").then((d) => d.chats),
  chat: (id: string) =>
    request<{ chat: Chat; messages: Message[]; previews: Record<string, Preview> }>("GET", `/api/chats/${id}`),
  models: () => request<{ models: Model[]; default: string | null }>("GET", "/api/models"),
  create: (model: string | null) =>
    request<{ chat: Chat }>("POST", "/api/chats", { model }).then((d) => d.chat),
  switchModel: (id: string, model: string) =>
    request<{ chat: Chat }>("POST", `/api/chats/${id}/model`, { model }).then((d) => d.chat),
  archive: (id: string) => request<object>("POST", `/api/chats/${id}/archive`),
  /** send posts a message, naming files already uploaded; a spoken one asks for a reply meant
   *  to be heard. */
  send: (id: string, requestId: string, text: string, spoken = false, attachments: string[] = []) =>
    request<{ messages: Message[] }>("POST", `/api/chats/${id}/messages`, {
      request_id: requestId,
      text,
      spoken,
      attachments,
    }).then((d) => d.messages),
  /** upload saves a file in the chat's folder and returns the path it was given, which may be
   *  numbered to keep an existing file. */
  upload: async (id: string, file: File): Promise<Attachment> => {
    const response = await fetch(`/api/chats/${id}/files/${encodeURIComponent(file.name)}`, {
      method: "PUT",
      headers: { "content-type": file.type || "application/octet-stream" },
      body: file,
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      const message = response.status === 413 ? `${file.name} is over the 20 MB limit.` : (data.message ?? `Upload failed (${response.status})`);
      throw new ApiError(data.error ?? "http", message);
    }
    return data as Attachment;
  },
  cancel: (id: string) => request<object>("POST", `/api/chats/${id}/cancel`),
  /** voice reports whether the server has what dictation and reading aloud need. */
  voice: () => request<{ available: boolean }>("GET", "/api/voice").then((d) => d.available),
  transcribe: async (recording: Blob) => {
    const response = await fetch("/api/transcribe", {
      method: "POST",
      headers: { "content-type": recording.type },
      body: recording,
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      throw new ApiError(data.error ?? "http", data.message ?? `Transcription failed (${response.status})`);
    }
    return data.text as string;
  },
  files: (id: string) => request<Listing>("GET", `/api/chats/${id}/files`),
  imageUrl: (name: string) => `/api/images/${name}`,
  fileUrl: (chatId: string, path: string) =>
    `/api/chats/${chatId}/files/${path.split("/").map(encodeURIComponent).join("/")}`,
  speechUrl: (chatId: string, messageId: string) => `/api/chats/${chatId}/messages/${messageId}/speech`,
};

/** Describe turns any thrown value into a sentence for a notice. */
export function describe(error: unknown): string {
  if (error instanceof ApiError && error.code === "blocked") {
    return "This chat can't continue after its last error. Start a new chat to keep going.";
  }
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}
