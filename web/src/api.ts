export type Chat = {
  id: string;
  title: string | null;
  created_at_ms: number;
  updated_at_ms: number;
};

export type MessageStatus = "pending" | "complete" | "failed" | "cancelled" | "interrupted";

export type Message = {
  id: string;
  chat_id: string;
  role: "user" | "assistant";
  text: string;
  status: MessageStatus;
  reason: string | null;
  created_at_ms: number;
};

export type ServerEvent =
  | { type: "message"; message: Message }
  | { type: "title"; chat_id: string; title: string }
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
  chat: (id: string) => request<{ chat: Chat; messages: Message[] }>("GET", `/api/chats/${id}`),
  create: () => request<{ chat: Chat }>("POST", "/api/chats").then((d) => d.chat),
  send: (id: string, requestId: string, text: string) =>
    request<{ messages: Message[] }>("POST", `/api/chats/${id}/messages`, {
      request_id: requestId,
      text,
    }).then((d) => d.messages),
  cancel: (id: string) => request<object>("POST", `/api/chats/${id}/cancel`),
};

/** Describe turns any thrown value into a sentence for a notice. */
export function describe(error: unknown): string {
  if (error instanceof ApiError && error.code === "blocked") {
    return "This chat can't continue after its last error. Start a new chat to keep going.";
  }
  if (error instanceof Error) return error.message;
  return "Something went wrong.";
}
