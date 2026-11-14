# Covalent — plan for milestones 5 and 6

Written 2026-11-14 against re:agent `c1757cca` (main) and Covalent `cb88a52`.
It continues [the skeleton plan](covalent-skeleton-plan.md). Like that page,
this is a plan: when the code and this page disagree, the code is right.

**Status, 2026-11-14:** milestone 5 is done: re:agent
[#285](https://github.com/satmihir/re-agent/pull/285) merged as `fbeb3972`, and
Covalent pins it. The same one-sentence question
now costs 179 input tokens with no tools and 588 bytes of Covalent's prompt.
Open points 1, 2 and 5 were settled as recommended.

Milestone 5 makes a chat a real chat Session: no tools, no workspace, and a
prompt Covalent writes. Milestone 6 makes the reply appear as the model writes
it. They do not depend on each other. Each is one re:agent pull request,
followed by one Covalent change that bumps the pinned revision.

## Where things stand

Measured from the trace of a one-sentence question sent through Covalent today:

| | Today (stand-in) |
|---|---|
| Tool declarations in the request | 8 |
| Instructions | 3,468 bytes of coding-agent rules |
| Per-message extra | a workspace snapshot (path, date, Git probes) |
| Input tokens | 2,061 |
| Reply | arrives whole, after the model finishes |

## Milestone 5 — tool-free conversation Sessions

### What re:agent has

An internal `tool_free` setting, used only by BTW questions (`ask`). It mixes
two things that a chat needs separated:

| Where | What `tool_free` does there | Kind |
|---|---|---|
| `tools.rs` specs, execute, `ToolState::new`; `tool_images.rs` | No tool declarations, no execution, no opened workspace | no tools |
| `run_loop.rs` user Entry | No workspace snapshot on the message | no tools |
| `run_loop.rs` after a response | A response with ToolCalls ends the Run with `protocol_error` | no tools |
| `run_loop.rs` `Session::new` | Skips binding the Session's cancellation token | BTW only |
| `run_loop.rs` finish | Forces `resumable: false` | BTW only |
| `provider.rs` | Counts unreported attempts for an isolated question | BTW only |

It is not saved in checkpoints, because a BTW Session is never saved.

Instructions are pinned per Session, but every re-pin (compaction, Reset, a
fresh model switch) renders them again from the embedded coding rules. A prompt
set from outside would be lost at the first compaction.

### Changes in re:agent

1. **Two new launch settings.** `SessionOptions.tool_free: bool` and
   `SessionOptions.instructions: Option<String>`. Instructions are allowed only
   with `tool_free`, are bounded (32 KiB, the same cap as `AGENTS.md`), and
   replace the embedded rules and the runtime section entirely. The host owns
   the prompt; re:agent adds nothing to it.
2. **Split the BTW-only behavior** from the no-tools behavior, following the
   table above. A tool-free conversation binds cancellation, can be resumable,
   and uses ordinary usage accounting. BTW keeps today's behavior exactly.
3. **Checkpoint.** Two optional fields, `tool_free` and `instructions`, absent
   from every existing checkpoint. Resume restores both. Every re-pin keeps the
   host's instructions.
4. **The date.** A tool-free message carries only the date and time zone, in
   place of the workspace snapshot. Instructions stay free of anything that
   changes per request, so the provider can still cache them.
5. **What a tool-free Session refuses or turns off.** Agents and `git_do` are
   off. The human-shell control and turning plan mode on are rejected, since
   both are about a workspace. `open_session` neither opens the workspace nor
   reads `AGENTS.md`. Compaction, model switch, Reset, `ask`, and steering work
   as they do now.
6. **Wording.** The `protocol_error` and `permission_denied` messages stop
   saying "BTW" when the Session is a conversation.

Left alone on purpose: the compaction prompt still asks for repository state
and commits. It works for a chat but is not written for one; that belongs with
the later work on chat quality.

### Changes in Covalent

- Chats open with `tool_free: true` and a short chat prompt kept in
  `src/prompt.txt`. This is the first prompt Covalent owns; tuning it is the
  "make it feel like a chat" work that follows, not part of this milestone.
- Title Sessions also become tool-free, with their own two-line instructions,
  so a title request is a few hundred tokens.
- The stand-in goes away: no `read_only`, no empty workspace directory.
- Chats created before this keep the settings saved in their checkpoints. They
  stay read-only Sessions with tools; nothing migrates them.

### Done when

- A chat request carries no tool declarations, no workspace record, and
  Covalent's prompt; the same one-sentence question costs a few hundred input
  tokens, not 2,061.
- Stop still cancels, and a chat still resumes after a restart with its prompt
  and tool-free setting intact, including after a compaction.
- A BTW question behaves exactly as before.
- `make check` passes in re:agent; Covalent's tests pass offline.

## Milestone 6 — reply text as it arrives

### What re:agent has

Every live request already streams. `transport/stream.rs` has a `Preview` that
parses both providers' stream events, separates visible text from reasoning and
tool arguments, redacts credentials, and feeds the terminal region a bounded
tail of complete lines. It exists per HTTP attempt and is cleared when the
attempt ends, because a retried attempt starts its text over.

The Session API exposes none of it. The event feed is 256 records of hints,
with one content-bearing exception (`ShellOutput`, 1 KiB chunks, gaps allowed
when a reader falls behind).

### The design choice: a read, not events

**Recommended: a blocking read.** The owner keeps the active conversation Run's
preview text in the shared state, and callers read it by offset:

```rust
read_preview(&self, run: &RunRef, cursor: Option<PreviewCursor>, wait_ms: u64,
             cancellation: Option<&AtomicBool>) -> Result<PreviewPage, ApiError>
```

A page carries the Step and attempt it belongs to, the bytes after the cursor,
the next cursor, and whether the Run is terminal. It waits the way
`read_events` waits. A new attempt or Step starts a new preview at offset zero,
and the page says so, so the caller replaces what it showed.

The alternative is a second content-bearing event, shaped like `ShellOutput`.
It is less code in re:agent, but text arrives in hundreds of small pieces per
reply and would crowd the 256-record feed, a slow reader would see gaps, and a
caller that joins mid-reply would get nothing until the end. The read has none
of those problems and matches how the API already serves content
(`read_entries`), with events left as hints.

### Rules for preview text

- **Speculative.** It is what the model has written so far in the current
  attempt. The accepted reply in `RunResult` is the only authoritative text,
  and may differ: an attempt can be retried, refused, or discarded.
- **Visible text only.** Text and refusal deltas. Never reasoning, tool
  arguments, or provider-native items.
- **Redacted like the terminal preview.** No credential reaches a caller, even
  one split across deltas. To keep token-level smoothness, only a tail that
  could be the start of a credential is held back, not whole lines.
- **Bounded.** The buffer stops growing at a fixed cap (1 MiB) and says it was
  cut; the final reply still arrives whole.
- **Conversation Runs of the root Session only.** Not BTW questions, not
  agents, not scripted Sessions (they have no stream).
- **Gone at the end.** The buffer is dropped when the Run is terminal. Nothing
  about previews is saved in checkpoints or changes history.

### Changes in re:agent

1. A preview buffer for the active Run in the Session API's shared state, with
   its Step and attempt counters.
2. The stream parser gets a second destination beside the terminal region. The
   run loop hands it over per Run, the way it hands over the cancellation token.
3. `read_preview` and its page and cursor types.
4. Tests against the local fake provider the steering tests already use:
   deltas arrive in order, a retried attempt resets, a credential split across
   deltas never appears, reasoning never appears, the cap holds, and a
   cancelled Run ends the read.
5. `make tty-check`, since the terminal preview's parser is touched.

### Changes in Covalent

- **Backend.** The thread that waits for each Run loops on `read_preview`
  instead of `wait_run`, sends a `preview` event (message ID, attempt, offset,
  text), and keeps the text so far in memory. `GET /api/chats/:id` returns a
  pending reply with that text, so a page loaded mid-reply catches up and then
  splices later events by offset.
- **Frontend.** A pending reply renders its preview as Markdown, updated at
  most once per animation frame. The thinking dots show until the first text.
  The final `message` event replaces the preview.
- **Stopping.** A stopped reply shows "Stopped" and drops the partial text.
  re:agent does not keep a cancelled reply in the conversation, so showing it
  would display text the model does not remember.
- **Tests.** A small fake streaming provider in Covalent's tests, reached
  through `REAGENT_OPENAI_BASE_URL`, because scripted Sessions do not stream.

### Done when

- Text appears in the thread within a moment of the model writing it, in both
  an OpenAI and an Anthropic chat.
- Reloading the page mid-reply shows the text so far and keeps going.
- A retried attempt replaces the partial text and never duplicates it.
- The stored message equals `RunResult.reply`, never the preview.
- `make check` and `make tty-check` pass in re:agent; Covalent's tests pass
  offline.

## Order and size

Milestone 5 first. It is the smaller change, it fixes what every turn costs,
and it settles the checkpoint fields before milestone 6 adds anything. Each
milestone is: agree the open points below, the re:agent pull request, then the
Covalent change once it merges. While a re:agent change is unmerged, Covalent
builds against the sibling checkout through the local patch.

Rough size, tests included: a few hundred lines in re:agent for each
milestone, and about a hundred in Covalent for milestone 5 and two to three
hundred for milestone 6.

## Open points

1. **Prompt ownership.** The plan has the host's instructions replace
   re:agent's rules entirely. The alternative is to keep a short fixed re:agent
   preamble in front of them.
2. **How the date reaches the model.** Per message, as planned, or left to the
   host to put in its instructions at the cost of a prompt that changes daily.
3. **Read or events** for preview text, as argued above.
4. **Partial text on Stop.** Dropped, as planned, or kept on screen and marked
   as not part of the conversation. Keeping it properly would need re:agent to
   accept a partial reply on cancellation, which is a feature of its own.
5. **Names.** `tool_free` and `instructions` on `SessionOptions`, and "preview"
   for the streamed text, matching the word `stream.rs` already uses.
