export type Chat = {
  id: string;
  title: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  /** model is null for chats created before Covalent recorded it. */
  model: string | null;
  archived_at_ms: number | null;
};

export type Model = { id: string; provider: string; note: string };

export type MessageStatus = "pending" | "complete" | "failed" | "cancelled" | "interrupted";

export type Source = { url: string; title: string };

/** Image is a picture a reply made, served at `/api/images/{name}`. */
export type Image = { name: string; width: number; height: number };

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
};

/** Preview is a pending reply's text so far, numbered by its latest preview event. */
export type Preview = { seq: number; text: string; searching: boolean; generating: boolean };

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
  /** send posts a message; a spoken one asks for a reply meant to be heard. */
  send: (id: string, requestId: string, text: string, spoken = false) =>
    request<{ messages: Message[] }>("POST", `/api/chats/${id}/messages`, {
      request_id: requestId,
      text,
      spoken,
    }).then((d) => d.messages),
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
  imageUrl: (name: string) => `/api/images/${name}`,
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
