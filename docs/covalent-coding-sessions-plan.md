# Covalent — plan for coding Sessions (milestones 17 to 20)

Written 2026-11-27 against re:agent `18df4e5c` (branch `add-astra-6-2`) and
Covalent `e9abc08`. It continues [the uploads plan](covalent-uploads-plan.md);
the milestone numbers continue from [the iPhone design](covalent-ios-design.md).
Like those pages, this is a plan: when the code and this page disagree, the
code is right.

**Status, 2026-11-27:** a design, agreed in conversation. Nothing is built.

## Goal

From any Covalent chat, typed or spoken, start coding work that re:agent does
on the machine Covalent runs on, ask what the work in progress is doing, talk
to it, and stop it. "Start a session in re-agent on Sol 6.2 high to fix the
flaky auth test and open a pull request." "What are my sessions doing?" "Ask
the auth one why it changed the retry count." "Stop it."

Covalent is the control plane. The chat model only controls; it never does the
coding itself. The coding is done by a separate re:agent Session in its own
process, with re:agent's own rules and tools.

Not in this plan: isolation (containers, a sandbox, a restricted user), a limit
on how many Sessions run, a default model, notifications when the app is
closed, deleting worktrees, merging pull requests, deploying, gRPC, and a page
in the web app that lists Sessions. Everything here is reached by talking to a
chat.

## Words

- A **chat Session** is the re:agent Session behind one Covalent chat, as today:
  in Covalent's process, no commands, its own folder.
- A **coding Session** is an ordinary re:agent Session whose workspace is a git
  worktree, running in a process of its own. Nothing about it is stored
  outside re:agent's session store: no job record, no registry. Say "re:agent
  session" aloud; "job" is not used, so it cannot be mixed up with anything
  else.
- Each instruction to a coding Session is a **Run**, as everywhere in re:agent.

## The decisions

| Decision | Set aside | Why |
|---|---|---|
| The tools are Covalent's, offered to a chat Session through a new host-tool hook in re:agent | Tools built into re:agent and switched on by an option, as `generate_image` is | Finding and cloning GitHub repos and keeping a worktree area is product policy, not harness. There are two tool families now (repos, Sessions) and memory will be a third. Changing a tool's wording becomes a Covalent deploy, not a re:agent pull request and a revision bump. |
| A coding Session runs in its own process, one per open Session | A thread in Covalent's process, as chats are | Deploying Covalent restarts it, and both crates abort on any panic (`Cargo.toml`, `panic = "abort"`), so a thread would die with every deploy and one bad Session would take every chat down. The likeliest work is on Covalent and re:agent themselves, ending in a deploy. |
| That process is the Covalent binary run again as `covalent reagent serve` | A separately installed `reagent` | re:agent's command line is a public function (`reagent::cli::main_os`). The same binary means no second install and no version mismatch with the pinned revision. |
| Covalent talks to the process over a Unix socket, one JSON request and one JSON reply per line | gRPC over the socket; no connection at all, reading files and sending signals | The Session API's records already serialize to and from JSON and its reads are long polls with cursors, so no pushed stream is needed. gRPC would add `tonic`, `prost` and a protobuf compiler, and a second definition of every record. Without a connection there is no `ask`, no steering and no streamed reply. |
| A Session with no process is read from its checkpoint on disk | Keeping every Session's process alive | A dormant Session costs nothing, and Sessions started in a terminal can be listed too. |
| One coding Session has one worktree and one branch, in an area Covalent owns | Working in existing checkouts | Sessions cannot collide, your own checkouts and the one the service runs from are never touched, and the branch name is a name you can say. |
| The model and effort are said in conversation; the chat asks when they are not | A default | Chosen by the user. Covalent's memory may hold a preference later. |
| A coding Session may open a pull request when its instructions say so | Commit only, ship from a terminal | Chosen by the user. `git_do` is on for a Session with commands and never pushes the default branch. |
| Every new chat gets the tools | One marked chat | Chosen by the user. |

## How it fits together

```text
phone or browser
      |
   Covalent ──── chat Session (in process, one per chat)
      |                | calls a host tool: repo_*, session_*
      |                v
      |          Covalent answers the call
      |            |              |
      |            | git, gh      | starts the process, then
      |            v              | JSON lines over a Unix socket
      |     repos/  worktrees/    v
      |                    covalent reagent serve   (one per open coding Session)
      |                           |
      |                    coding Session: re:agent's rules, exec, agents, git_do
      |                           |
      └── reads checkpoints ──> ~/.cache/reagent/sessions/<id>/
          of dormant Sessions       checkpoint.json, lock, socket
```

A chat turn that starts work returns as soon as the coding Session has
accepted its first message. The work then goes on with no chat turn open, and
survives Covalent restarting.

## What exists today

In re:agent:

- The Session API (`rust/src/session_api.rs`) already has every operation this
  needs, in process: `send_message`, `ask` (a question answered from the
  Session's context, without tools and without interrupting its work),
  `steer`, `cancel_run`, `get_snapshot`, `read_entries`, `read_events`,
  `read_preview` and workspace consent. Its design page expects a transport
  later (it mentions a gRPC adapter); there is none yet.
- A Session opened through the API with commands gets `git_do` in recipe mode
  (`open_session`), which includes `ship_pr` and `follow_up_pr`.
- Every Session's folder holds a `lock` file that its owner holds with `flock`
  (`rust/src/checkpoint.rs`), released by the kernel when the process dies. So
  "a live process owns this Session" can be asked from outside without a
  process ID that could be stale.
- The checkpoint is saved before each model request, after each accepted
  response, before each tool call and when the Run ends, so a finished step is
  on disk before the next one starts. It holds a `phase` (`idle`, `model`,
  `accepted`, `tool`, `terminal`), the call in flight, a `blocked` status,
  usage, the launch workspace and a preview of the first message. `checkpoint/listing.rs` already reads all of them
  without locking, for the resume picker, but it is private to the crate and
  skips Sessions that carry an origin label.
- A Session whose process died mid-Run resumes without replaying anything and
  accepts a new message (`checkpoint::recover`).
- Tools come only from `tools::specs` and are run by `tools::execute`. A host
  cannot add one. Workspace consent (`session_api/consent.rs`) is the existing
  pattern for "the Run waits while the host answers".

In Covalent:

- A chat Session is opened with `agents: false` and `exec: false`
  (`Launch::chat_options`), and `follow` already long-polls a Run's events to
  show its tool calls.
- A restart marks pending replies `interrupted` (`store.rs`).
- The service unit is described in `.agents/skills/deploy/SKILL.md`. By
  default systemd ends a service's child processes when it restarts.

## The tools

Names say "session", never "job". `session` in an argument is a Session's name,
`repo/name` when two repos share a name, or its ID.

| Tool | Arguments | Does | Returns |
|---|---|---|---|
| `repo_find` | `query` | Lists clones in the area and your GitHub repos that match, through `gh` | Up to 20 rows: `owner/name`, private or public, cloned or not |
| `repo_clone` | `repo` (`owner/name`) | Clones into the area if it is not there | The clone's default branch |
| `session_start` | `repo`, `name`, `task`, `model`, `effort` | Fetches, creates the worktree and the branch `name` from the default branch, starts the process, sends `task` as the first message | Name, model, effort, `working` |
| `session_list` | none | Reads every saved Session that is not a Covalent chat, including ones started in a terminal | Up to 20 rows: name, repo, state, age, one line of what it last did |
| `session_status` | `session` | Reads the Session, and git in its worktree | State, model, steps and tokens, the last reply (bounded), the last five tool calls; from git: files changed, commits ahead, pushed or not, the pull request if there is one |
| `session_read` | `session`, `cursor` (optional) | Pages back through the Session's visible Entries, newest first | One page and the cursor for the next |
| `session_send` | `session`, `message` | Starts the process if there is none, then sends the message: a new Run, or queued behind the Run in progress | `working` |
| `session_ask` | `session`, `question` | Starts the process if there is none, then `ask` | The answer |
| `session_stop` | `session` | Cancels the Run in progress | `stopped`, and whether a command's effects are unknown |

Rules the tool descriptions and the prompt paragraph carry:

- Before `session_start`, the chat repeats the repo, the name, the model and
  effort and the task, and waits for a yes. A spoken turn can be transcribed
  wrongly, and a garbled task is the expensive mistake.
- `model` and `effort` are required. If the user did not say them, the chat
  asks; it never picks.
- `name` is lowercase letters, digits and hyphens, short enough to say. It is
  the branch and the worktree's folder.
- To redirect work in progress: `session_stop`, then `session_send`.
- The change, push and pull-request facts in `session_status` come from git and
  `gh`, not from what the Session says it did.

### States

| Said | How it is known |
|---|---|
| working | A process owns the Session and a Run is live (its snapshot, over the socket; before the socket exists, the lock is held and the phase is mid-Run) |
| waiting | The last Run completed or was refused. Its reply may be a result or a question; the chat reads it and says which |
| stopped | The last Run was cancelled |
| failed | The checkpoint's `blocked` names the status |
| interrupted | No process owns it, but the phase is mid-Run: the process died |

There is no "finished". `completed` means the model replied, not that the task
succeeded. Whether the work is done is the user's call.

## Changes in re:agent

Three pieces, each useful to any host, none naming Covalent.

**1. Host tools.** `SessionOptions.host_tools` lists tools the host answers:
name, description, input schema, and whether the tool only reads. They are
declared to the model after re:agent's own, saved in the checkpoint like
`instructions`, and so survive resume. In the run loop a call to one goes
where a built-in call goes (after `recall` and the agent tools), is saved as in
flight first like any call, and then waits for the host, in the shape workspace
consent has: the host registers for an opening with a timeout, reads the
pending call, and answers it once with an outcome. With no host registered the
call fails at once, which is what a Covalent chat resumed in a terminal needs.
re:agent checks the arguments against the schema before publishing the call,
refuses a non-reading host tool in plan mode, and never offers host tools to
agents or BTW questions. A name that collides with a built-in tool is refused
at open.

Files: `session_api/types.rs` (the option), a new `session_api/host_tools.rs`
(register, pending, answer; about the size of `consent.rs`), `tools.rs`
(declaring them), `run_loop.rs` (the dispatch branch), `checkpoint.rs` (the
saved field), `docs/reagent-session-api.md` and `docs/reagent-overview.md`.

**2. Reading saved Sessions.** Two public functions beside `resume_session`:
one lists saved Sessions (ID, origin, launch and active workspace, model,
effort, modified time, first-message preview, whether a live process owns it,
phase, `blocked`, usage), and one pages through a saved Session's Entries using
the same visible-only projection `read_entries` uses (no reasoning, no
provider-native items, bounded pages). Neither takes the lock or changes a
file. The checkpoint also gains the last Run's status and reason, so "stopped"
does not depend on a best-effort trace.

Files: `checkpoint/listing.rs`, `session_api/observations.rs`,
`checkpoint.rs`, the two docs.

**3. `reagent serve` and the socket.** `reagent serve [flags]` opens a Session
(or `--resume ID` resumes one) through the Session API, listens on `socket` in
the Session's private folder, and prints one line naming the Session ID and the
socket when it is ready. It serves until the Session has had no live Run and no
request for an idle period, then closes the Session and removes the socket.

The wire is one JSON object per line each way:

```text
→ {"method":"ask","params":{"request_id":"q1","question":"What is left to do?"}}
← {"ok":{ ...CommandInfo... }}
→ {"method":"wait_run","params":{"run":{ ...RunRef... }}}
← {"ok":{ ...RunInfo... }}
← {"error":{"code":"busy","message":"..."}}
```

Requests are answered in order on a connection; a call that waits holds its
connection, and a client opens another for a second call. Results are the
Session API's existing records, unchanged. Errors are `ApiError`'s code and
message. First methods: `get_snapshot`, `send_message`, `ask`, `steer`,
`get_run`, `wait_run`, `cancel_run`, `read_entries`, `read_events`,
`read_preview`. Later: workspace consent. Never: the human shell control,
which is for a person at a keyboard.

The client lives in re:agent beside the server, so the request shapes are
defined once: a host calls `connect(session_id)` and gets the same method
names. The socket is as trusted as the checkpoint next to it: the folder is
private to the user, and there is no other authorization.

Files: a new `session_socket.rs` (requests, server, client), `cli.rs` (the
`serve` command), the two docs. No new dependency: blocking `std` sockets and a
thread per connection.

## Changes in Covalent

- **`main.rs`**: when the first argument is `reagent`, run re:agent's command
  line with the rest and exit. `covalent reagent serve …` is then a coding
  Session's process.
- **`tools.rs`** (new): the host tools' declarations, and the function that
  answers one call.
- **`coding.rs`** (new): start, list, status, read, send, ask and stop, over
  re:agent's saved-Session functions and socket client. Starting a process
  detaches it from Covalent and waits for its ready line.
- **`repos.rs`** (new): find, clone and worktree, running `git` and `gh` with
  an argument list and a time limit.
- **`chats.rs`**: `chat_options` adds the host tools and a `CODING` paragraph
  to the instructions, as `IMAGES` is added now; opening or resuming a chat
  registers Covalent as its host; `follow` answers a pending call when its
  event arrives.
- **`activity.rs`**: a row for a `session_*` call names its Session.
- **The area**: `repos/<owner>/<name>/` for clones and
  `worktrees/<repo>/<name>/` for worktrees, in Covalent's data folder beside
  `workspaces/`. Nothing outside it is cloned into or branched from.
- **`README.md`** and **the deploy skill**: what coding Sessions can do, and
  `KillMode=process` in the service unit so a restart leaves them running.

Existing chats keep the settings they were opened with, as happened with files
and web search: only new chats get the tools.

Credentials reach a coding Session the way they reach chats, through the
environment Covalent starts with: OpenAI models use the ChatGPT plan first and
the API key second, Claude models use `ANTHROPIC_API_KEY`.

## Authority, cost and privacy

- **This changes what Covalent exposes.** A chat cannot run commands today, and
  Covalent has no login. With these tools, every device on the tailnet can
  start work that runs commands as the user on that machine.
- **Nothing confines a coding Session.** Its file tools stay in its worktree,
  but `exec` does not, and commands get `HOME`, so git and `gh` credentials on
  disk are reachable. A coding Session is as trusted as `reagent` typed in a
  terminal, without the user watching. Isolation is set aside on purpose.
- **Both repos are public**, so a push or a pull request publishes. A Session
  does either only when its instructions say so, and never merges.
- **Spend is not capped.** A Run that hits a step cap ends `limit_exceeded`,
  which blocks its Session until reset, so a cap is a kill switch and not a
  budget. Spend is governed by the model and effort the user names, and
  `session_status` reports tokens used.
- **A permission request expires.** Workspace consent waits at most five
  minutes; until it is offered over the socket, a coding Session that asks for
  another folder is refused at once.

## Done when

- **Milestone 17.** With two re:agent Sessions on the machine, one working in a
  terminal and one idle, a new chat asked "what are my sessions doing?" names
  both with the right state, and a follow-up question about one is answered
  from its Entries. A Covalent chat resumed in a terminal fails a host tool
  call at once and carries on.
- **Milestone 18.** `reagent serve` started by hand answers JSON lines typed
  into its socket: a message, a question asked while the message's Run works,
  a stop. It exits by itself when idle, and a second `serve` on the same
  Session is refused.
- **Milestone 19.** By voice from a phone: find a repo that is not cloned,
  start a Session in it after the read-back, hear its status, ask it a
  question, stop it, send it on, and get a pull request. Covalent is restarted
  in the middle and the Session keeps working.

## To verify while building

- That `KillMode=process` behaves as expected on the machine, and what systemd
  logs about the processes left behind.
- The lock probe: trying the lock from outside can make an owner that is
  starting at that instant fail to claim it. Decide whether that window
  matters before the socket replaces the probe for live Sessions.
- Where a dormant Session's last Run status comes from, and that adding it to
  the checkpoint keeps old checkpoints readable.
- Socket path length. Unix socket paths are limited to about 104 bytes on
  macOS; a Session folder under a temporary HOME can be longer, so tests need
  a short root.
- What `serve` does on SIGINT and SIGTERM, and that a killed process leaves a
  socket file the next one can replace.
- The size of Entry pages on the wire: byte fields serialize as arrays of
  numbers.
- Starting the Covalent binary from Covalent's tests, which are unit tests
  today and do not get the binary's path from Cargo.
- That a plan refusing a model, which fails on the first reply, is reported
  by `session_start` or the first `session_status` in words that make sense
  aloud.
- The shapes of `gh` output for listing repos and finding a branch's pull
  request.

## Order and size

| Milestone | Where | What |
|---|---|---|
| 17 | re:agent, then Covalent | Host tools and reading saved Sessions; `session_list`, `session_status`, `session_read` from disk. Useful alone, spends nothing, grants nothing |
| 18 | re:agent | `reagent serve`, the socket, the client |
| 19 | Covalent | The `reagent` argument, the repo tools, start, send, ask and stop, the service unit |
| 20 | Both | Later, in any order: a notice in the chat when a Run ends, a Session's reply read aloud as it is written, permission requests answered by voice |

Milestone 17's re:agent half is the largest single piece: host tools are about
the size of workspace consent. Milestone 19 is mostly Covalent glue.

## Open points

- How long `serve` waits idle before it exits.
- Whether the branch is the plain name or carries a prefix.
- Whether `reagent chat` in a terminal should also listen on its Session's
  socket, so a chat could ask a Session you are sitting at. Not planned.
- Whether `session_send` should offer steering a Run in progress, now that the
  socket makes `steer` available, or stay with stop-then-send.
- Whether Gemini models may be named for a coding Session. Chats exclude them
  only because of web search, which coding Sessions do not use.
