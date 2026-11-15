# Covalent

A minimal web chat built on [re:agent](https://github.com/satmihir/re-agent). The
backend is one Rust binary that embeds re:agent as a library through its Session
API; the frontend is a small React app it serves. Each chat is a re:agent
Session. Covalent keeps the displayed transcript and titles in its own SQLite
file.

It is single-user and listens on `127.0.0.1` only.

## Running it

You need Rust (the toolchain is pinned in `rust-toolchain.toml`), Node 20 or
newer, and re:agent credentials: `reagent login` for a ChatGPT plan, or
`OPENAI_API_KEY` / `ANTHROPIC_API_KEY` in the environment.

```bash
cd web && npm install && npm run build && cd ..
cargo run --release
```

Then open <http://127.0.0.1:7420>.

For frontend work, keep the binary running and start Vite, which proxies `/api`
to it and reloads on save:

```bash
cd web && npm run dev
```

## Settings

| Variable | Default | Meaning |
|---|---|---|
| `COVALENT_PORT` | `7420` | Loopback port to listen on. |
| `COVALENT_DATA_DIR` | `~/.covalent` | Where the chat store lives. |
| `COVALENT_WEB_DIR` | `web/dist` | The built frontend. |
| `COVALENT_MODEL`, `COVALENT_EFFORT` | re:agent's defaults | Model and reasoning effort for chats. |
| `COVALENT_TITLE_MODEL`, `COVALENT_TITLE_EFFORT` | re:agent's defaults | Model and effort for writing titles. |

re:agent's own settings, such as `REAGENT_OPENAI_BASE_URL`, apply as usual.
Sessions are saved in re:agent's session store with the origin label
`covalent`, which keeps them out of the CLI's session picker and `--continue`.

## Web search

New chats can search the web through the provider's hosted search (re:agent's
`web_search` option). While a search runs the reply shows "Searching the web…";
a finished reply lists the pages it cites under the text. Title requests never
search. Chats created before search was added keep their saved setting.

## Voice

With `OPENAI_API_KEY` set where Covalent starts, the composer gets a microphone
button and every finished reply a speaker button. The microphone records one
turn and puts its words in the composer to edit and send (OpenAI
`gpt-transcribe`). The speaker reads the reply aloud as plain sentences: no
Markdown, no URLs, and code blocks and tables replaced by a short spoken note
(OpenAI `gpt-realtime-2.1-mini`, voice `marin`). Without the key the buttons do
not appear.

Voice sends recordings, and the text of each reply read aloud, to OpenAI, also
in chats whose model is Claude. It is billed to the API key, apart from any
ChatGPT plan: roughly half a cent per minute dictated and three cents per minute
read aloud. Covalent stores no audio.

The key is also one re:agent can use for chat. While your ChatGPT login is
valid, chats still use the plan, but if the login lapses re:agent falls back to
the key without saying so. To make that an error instead, start Covalent with
`REAGENT_OPENAI_AUTH=chatgpt`.

## Models and archiving

The model picker offers the catalog models whose provider has credentials
here. A new chat uses the model picked before its first message; switching an
existing chat carries its history over to the new model. A ChatGPT plan does
not serve every catalog model; a model it refuses fails on the first reply.

Archiving a chat removes it from Covalent and releases its Session, so it can
be continued in a terminal with `reagent --resume ID` (shown when you archive).
Covalent does not reopen archived chats.

## Checking changes

```bash
cargo fmt --check && cargo test
```

```bash
cd web && npm run typecheck
```

The Rust tests run offline with scripted re:agent Sessions and a temporary
HOME. The frontend has no automated tests; check it in a browser against the
running binary.

## Developing against a local re:agent

`Cargo.toml` pins re:agent to a pushed revision. To build against a sibling
checkout instead, add an untracked `.cargo/config.toml`:

```toml
[patch."https://github.com/satmihir/re-agent"]
reagent = { path = "../re-agent/rust" }
```

The plan this was built from is in
[`docs/covalent-skeleton-plan.md`](docs/covalent-skeleton-plan.md).
