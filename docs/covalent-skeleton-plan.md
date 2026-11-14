# Covalent — skeleton development plan

Written 2026-11-14 against re:agent `fb27bedd` (main). This is a plan, not a
specification: when the code and this page disagree, the code is right.

**Status, 2026-11-14:** milestones 1 to 4 are built locally and nothing is
pushed. Where the build differs from the plan below, "As built" says how.

## Goal

A minimal web chat, in the shape of ChatGPT, whose backend is a Rust program
that embeds re:agent as a library through `reagent::session_api`.

The skeleton is done when, on this machine:

1. You open the page and see your earlier chats in a sidebar.
2. "New chat" gives an empty thread; typing a prompt and pressing Enter sends it.
3. The reply appears in the thread, rendered as Markdown.
4. The chat gets a short AI-written title in the sidebar.
5. You can keep going back and forth, reload the page, restart the server, and
   pick the chat up again.

## Not in the skeleton

- Tools of any kind (planned next, not here).
- Making replies sound like a chat product rather than a coding agent.
- Accounts, sign-in, or more than one user. Covalent binds to `127.0.0.1` only,
  which matches the Session API's "trusted-local" stance.
- Rename, delete, search, model picker, regenerate, edit-and-resend, attachments.
- Token-by-token streaming. The wire protocol reserves a place for it (see
  "Events"), and it is milestone 6, after the skeleton.

## Decisions already made

| Question | Decision |
|---|---|
| What is a chat? | One re:agent Session. The chat ID is the 16-hex Session ID. |
| Where do Sessions live? | re:agent's own store (`<user cache>/reagent/sessions`), shared with the CLI, marked with an origin label so the CLI can filter them out. |
| Who owns the transcript the user sees? | Covalent. re:agent's `read_entries` truncates content (4 KiB per field), and compaction rewrites model context, so Covalent stores each user message and each reply itself. re:agent stays the authority for model context. |
| How does Covalent depend on re:agent? | A git dependency pinned to a revision. While both repos change locally, an untracked `.cargo/config.toml` patches it to the sibling checkout. |
| Frontend stack | Vite, React, TypeScript. `react-markdown` with `remark-gfm` for replies. Plain CSS with custom properties. No router, state, or component library. |
| Nothing is pushed | All work stays local in both repos until asked. |

## Shape

```text
browser (web/)  --HTTP + SSE-->  covalent binary  --in process-->  reagent::session_api
                                      |                                  |
                                 covalent.db                  reagent/sessions/<id>
                              (chats, messages)                  (checkpoints)
```

One crate at the repository root, one binary. The toolchain is pinned to Rust
1.99.1, the same as re:agent.

| File | Owns |
|---|---|
| `src/main.rs` | Startup: data directory, store, listener on `127.0.0.1`. |
| `src/http.rs` | Routes, request and response types, the SSE endpoint, static files. |
| `src/chats.rs` | Live `SessionApi` handles keyed by chat ID: open, resume, send, cancel, close when idle. |
| `src/store.rs` | SQLite: the `chats` and `messages` tables. |
| `src/title.rs` | Title generation. |
| `web/` | The Vite app: `App.tsx`, `Sidebar.tsx`, `Thread.tsx`, `Composer.tsx`, `api.ts`, `styles.css`. |

Dependencies: `reagent`, `axum`, `tokio`, `serde`, `serde_json`, `rusqlite`
(bundled SQLite), `tower-http` (static files). The Session API is blocking, so
every call into it runs on a blocking thread, never on the async runtime.

Covalent's own data (the SQLite file and an empty stand-in workspace) lives in
the platform data directory, overridable with `COVALENT_DATA_DIR`. Credentials
are whatever re:agent already uses: provider keys in the environment or
`reagent login`.

## Data

```text
chats     id (Session ID), title (null until generated), created_at_ms, updated_at_ms
messages  id, chat_id, role (user | assistant), text, status, reason, created_at_ms
```

- A user message's `id` is the browser-generated request ID, which is also the
  `request_id` passed to `send_message`. A retried POST is therefore harmless
  in both the store and re:agent.
- An assistant message's `id` is the Run ID.
- `status` is `pending`, `complete`, `failed`, `cancelled`, or `interrupted`.
  At startup, any `pending` row becomes `interrupted`: re:agent never restarts
  work that was in flight, so Covalent does not pretend it will.

## HTTP

| Method and path | Does |
|---|---|
| `GET /api/chats` | Chats, newest first. |
| `POST /api/chats` | Opens a Session, returns the new chat. Called on the first send, not on "New chat". |
| `GET /api/chats/:id` | The chat, its messages, and the active Run if one is in flight. |
| `POST /api/chats/:id/messages` | `{request_id, text}`. Stores the user message, admits a Run, returns at once. |
| `POST /api/chats/:id/cancel` | Cancels the active Run. |
| `GET /api/chats/:id/events` | Server-sent events for this chat. |

As built: one `GET /api/events` stream carries every chat's events, each with
its `chat_id`, because the sidebar needs titles for chats that are not open.
The port is 7420.

### Events

| Event | Carries | When |
|---|---|---|
| `run` | Run ID, state (`queued`, `running`, `terminal`) | A Run changes state. |
| `delta` | Run ID, text | Reserved. Not sent until milestone 6. |
| `message` | A stored message | A reply, failure, or cancellation is recorded. |
| `title` | The title | Title generation finishes. |

There is no replay. A client that connects or reconnects fetches
`GET /api/chats/:id` first and then listens; that is enough to resynchronize.

As built: there is no `run` event. The send response already contains the
pending reply, and `message` replaces it when the Run ends. A `lagged` event
tells a client that fell behind to reload.

In the skeleton each Run is observed with `wait_run` on a blocking thread. When
re:agent can publish reply text as it arrives, that thread becomes a
`read_events` loop and starts sending `delta`; the browser side does not change.

## Titles

When a chat's first message is admitted, Covalent opens a second, throwaway
Session with checkpointing off and a small model, asks it for a title of at
most six words for that message, cleans the answer (one line, no quotes,
bounded length), stores it, sends `title`, and closes the Session. It runs
alongside the first reply. If it fails, the title falls back to the start of
the message. Until then the sidebar shows "New chat".

## Interface

- Left sidebar: a "New chat" button and the chat list by recency. Collapses on
  narrow screens.
- Center column, about 720 px wide: the thread. User messages in a soft bubble
  on the right, replies as plain Markdown on the left, no avatars.
- Composer pinned to the bottom: a text area that grows with its content.
  Enter sends, Shift+Enter inserts a newline. While a Run is active the send
  button becomes Stop.
- A quiet "thinking" indicator while a reply is pending; a short inline notice
  with Retry when a Run fails or is interrupted.
- System font stack, one accent color, light and dark from the system setting.
- Each chat has its own URL (`/c/<id>`), handled with the History API.

## What re:agent lacks today

1. **An origin label.** Nothing distinguishes a Covalent Session from a CLI
   one, so they would appear in the CLI's session picker and `--continue`.
2. **Tool-free conversations.** `SessionOptions` always offers at least the
   read tools and attaches a workspace snapshot to each message. An internal
   `tool_free` setting exists for BTW questions, but it also turns off Run
   cancellation binding and resumability, and it is not saved in checkpoints.
3. **Reply text as it arrives.** Streamed text goes to a private terminal
   region. The event feed carries state hints only.

Until (2) lands, Covalent uses a stand-in: `read_only: true`, `agents: false`,
and an empty directory as the workspace. The model can see read tools that have
nothing to read. This is a known wart of the skeleton, removed in milestone 5.

## Known constraints to design around

- One opening of a Session retains at most 256 command receipts and never
  evicts them. On `resource_exhausted` Covalent closes and resumes the Session
  to get a fresh opening. As built, idle handles are not closed: they live
  until the process exits, which also keeps the CLI from resuming an open chat.
- Some Run outcomes block a Session until reset or compaction. The skeleton
  shows the reason and offers a new chat; it does not expose reset or compact.
- re:agent's default model and effort are tuned for coding work and may feel
  slow in chat. The skeleton uses the defaults unless `COVALENT_MODEL` is set.

## Milestones

One at a time. Each ends with a report of what ran and what did not, then stops.

| # | Repo | Milestone | Done when |
|---|---|---|---|
| 1 | re:agent | Origin label: an optional short label in `SessionOptions`, saved in the checkpoint, kept across resume; the CLI's listing skips labelled Sessions. | `make check` passes; a labelled Session is absent from the picker and `--continue`; old checkpoints behave as before. |
| 2 | Covalent | Backend: store, live handles, the HTTP routes above, non-streaming. | Offline tests pass using re:agent's `scripted` option; one live chat over `curl` survives a server restart. |
| 3 | Covalent | Frontend. | The five points under "Goal", except the title, work in a real browser. |
| 4 | Covalent | Titles. | A new chat's title appears in the sidebar without a reload. |
| 5 | Both | Tool-free conversation Sessions in re:agent; Covalent drops the stand-in. | A chat request carries no tool declarations and no workspace snapshot, and can still be cancelled and resumed. |
| 6 | Both | Reply text as it arrives, starting with a short design note in re:agent. | Text appears in the thread as the model writes it. |

Milestones 1 to 4 are the skeleton. Milestones 5 and 6 each start with their
own short design, because both touch re:agent's loop and checkpoint format.

## Checking

- Covalent's tests run offline with no key, using a scripted Session. Scripted
  Sessions do not checkpoint, so restart-and-resume is checked live.
- The frontend is checked in a real browser against the running binary, not
  only with unit tests.
- Changes in re:agent follow its `AGENTS.md`, including `make check`.

## Open questions

- The label's name (`origin` is the working name) and whether
  `reagent --resume <id>` should still open a labelled Session when asked for
  it by ID. Resuming a Covalent chat from the CLI would leave Covalent's stored
  transcript behind the model's context.
- Which small model writes titles when the provider is OpenAI versus Anthropic.
  As built, titles use re:agent's default model unless `COVALENT_TITLE_MODEL`
  is set; on a ChatGPT plan that took about as long as the reply.
- Whether Covalent adopts re:agent's `AGENTS.md` conventions as they are or
  gets a shorter file of its own.
