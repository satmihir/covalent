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
| `COVALENT_MODEL`, `COVALENT_EFFORT` | `gpt-6-sol` with OpenAI credentials, else re:agent's default; the model's own effort | Model and reasoning effort for new chats whose model was not picked. |
| `COVALENT_TITLE_MODEL`, `COVALENT_TITLE_EFFORT` | re:agent's defaults | Model and effort for writing titles. |
| `COVALENT_HOSTS` | none | Extra host names to answer to, separated by commas, such as a Tailscale name. |

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

**Voice mode** is the button in place of Send while the message box is empty.
It listens, sends what you say when you pause, and reads the reply aloud as it
is written; the thread shows the same conversation as text. What you say
streams to OpenAI while you say it (`gpt-live-transcribe`, through Covalent's
`/api/listen` WebSocket), so its words are ready a moment after you pause; a
turn that could not stream is sent whole, as the microphone button does. A
spoken message asks the model for a short reply without Markdown, and to search
the web only for recent or local facts, since every search adds seconds before
you hear anything; a typed message in the same chat still gets formatting and
searches as usual. Talk over a reply, or tap the circle, to stop
it: a reply still being written is cancelled. Ending voice mode lets a reply in
progress finish as text. Pauses are found in the browser by a speech detector
(Silero, through `@ricky0123/vad-web` and ONNX Runtime), whose files Covalent
serves itself under `/vad/`; voice mode loads them the first time it opens.
Use headphones if the reply's sound makes it stop itself.

While voice mode is open it keeps the screen on, where the browser allows it,
so a phone does not lock mid-conversation. If the microphone is taken away (the
phone locks anyway, another app comes to the front, or a call comes in), voice
mode pauses and says so; tap the circle to start it again.

Voice sends recordings, and the text of each reply read aloud, to OpenAI, also
in chats whose model is Claude. It is billed to the API key, apart from any
ChatGPT plan: roughly half a cent per minute dictated and three cents per minute
read aloud; voice mode's streaming transcription is billed by OpenAI as well.
Covalent stores no audio.

The key is also one re:agent can use for chat. While your ChatGPT login is
valid, chats still use the plan, but if the login lapses re:agent falls back to
the key without saying so. To make that an error instead, start Covalent with
`REAGENT_OPENAI_AUTH=chatgpt`.

## Using it from a phone

Covalent listens only on this machine's loopback address, and a phone's
browser allows the microphone only on HTTPS. Tailscale gives both without
opening Covalent to anyone else:

1. Install Tailscale on the Mac and the phone, signed in to the same account.
2. On the Mac, publish Covalent to your own devices:

   ```bash
   tailscale serve --bg 7420
   ```

   It prints an address like `https://mac.tail1234.ts.net`.
3. Start Covalent with that name allowed:

   ```bash
   COVALENT_HOSTS=mac.tail1234.ts.net cargo run --release
   ```

4. Open the address in Safari on the phone.

Covalent has no login. Every device on your tailnet can then read and use
every chat and its files. Never publish it with `tailscale funnel`, which puts
it on the public internet.

### The iPhone app (a proof, for now)

`ios/` holds a small native app that does voice mode only, and keeps talking
with the phone locked or another app in front, which Safari cannot. It is the
first step of [the iPhone design](docs/covalent-ios-design.md).

1. Open `ios/Covalent.xcodeproj` in Xcode.
2. Under the Covalent target's Signing & Capabilities, choose your team. If
   Xcode says the bundle identifier is taken, change `local.covalent.app` to
   something of your own.
3. Connect the iPhone, choose it as the destination, and run.
4. Enter Covalent's Tailscale address, leave the chat empty for a new one, and
   tap Start talking.

With a free Apple ID the app stops opening after 7 days; run it from Xcode
again. The parts that need no phone are a Swift package with tests:

```bash
cd ios/CovalentVoice && swift test
```

## Files

Each new chat has a folder of its own, `workspaces/<name>/` in Covalent's data
folder, and its model can list, read, search, create, edit and delete files
there with re:agent's file tools. Ask for a file ("write a packing list as
Markdown", "add sunscreen to it") and the reply lists what the model did, each
call as one row while it works, folded to one line afterwards.

The folder button in a chat's header opens the files panel beside the thread.
It lists the chat's files (the first 500, in path order), marks the ones the
latest reply created or changed, and refreshes as a reply works. Clicking a
file shows it: pictures as pictures, text files up to 1 MiB as plain text, and
anything else as a name and size to download. Every file has a download
button, and a row in a reply's activity opens its file in the panel. Files are
never shown as web pages: HTML appears as text, and a file fetched directly is
a download, so a file the model wrote can never run as part of Covalent.

To bring your own files, click the paperclip, drop files onto the chat, or
paste (a screenshot, or files copied in Finder). They upload when you send, up
to 10 per message and 20 MB each, into the chat's folder under their own names;
a name already taken gets a number (`photo-2.jpg`), so an upload never replaces
a file. The model is told which files came with the message and opens them
with its tools: it reads and edits text files, looks at pictures, and edits a
picture by making a new one from it, keeping the original. It cannot read PDF
or Office files; those can still be stored and downloaded.

A chat cannot run programs or commands, and its file tools stay inside its own
folder. One reply can make at most 40 tool calls in 30 model requests. Web
search stays on, so a chat that has read a file could put its text into a
search query. Folders are never deleted, including when a chat is archived.
Chats created before folders existed keep working as they did, without files.

Covalent answers only requests addressed to this machine (`127.0.0.1`,
`localhost` or `[::1]`) and refuses a request a browser sends from another
site.

## Images

With `OPENAI_API_KEY` set where Covalent starts, new chats can make images. Ask
for a picture and the chat model calls re:agent's `generate_image` tool; a
"Creating image…" tile shows while it works, usually 15 to 25 seconds, and the
image then sits above a short reply. Click it to open it at full size, or use
the download button. It works in OpenAI and Claude chats alike; an OpenAI model
also sees a small copy of what it made, a Claude model is only told it was made.

Images are saved as PNG files in the chat's folder and are never deleted. Each
one costs about four cents on the API key, apart from any ChatGPT plan. The
image prompt goes to OpenAI, also in chats whose model is Claude. Pictures made
before chats had folders stay in `images/` in the data folder.

## Models and archiving

The model picker offers the catalog models whose provider has credentials
here. A new chat uses the model picked before its first message, or GPT-6 Sol
when none was: it starts answering seconds sooner than 6.1 Sol, which voice
mode feels most. The page remembers the last model you picked; switching an
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
cd web && npm run typecheck && npm test
```

The Rust tests run offline with scripted re:agent Sessions, fake providers and
a temporary HOME. `npm test` runs voice mode's turn-taking rules with Node's
built-in test runner; check the rest of the frontend in a browser against the
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
