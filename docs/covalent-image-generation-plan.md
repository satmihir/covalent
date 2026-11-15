# Covalent — plan for image generation (milestone 9)

Written 2026-11-14 against re:agent `c902b960` (main) and Covalent `d28e2c2`.
It continues [the voice plan](covalent-voice-plan.md). Like that page, this is
a plan: when the code and this page disagree, the code is right.

**Status, 2026-11-14:** built, with the open points settled as planned. The
re:agent side merged as pull request 290; Covalent is pinned to re:agent
`c223d6be`, which also has the connection fix described below.
As built: the image paragraph of the prompt is added only for chats that can
make images; the "Creating image…" tile stays from the first image call until
the reply finishes, because the image arrives with the finished reply; and a
tall image's box narrows with the window's height.

Checked live on port 7421: an OpenAI chat (`gpt-6.1-sol`) and a Claude chat
(`claude-haiku-5-5`) each showed the tile, then an image and one short sentence
with no file name, in 37 s and 19 s. The image survived a restart. Stopping
during generation showed "Stopped" and saved nothing. Not checked: a refused
prompt, a reply stopped after its image was saved, and whether OpenAI bills a
stopped generation.

Found on the way, and not part of this milestone: a Claude chat's request
sometimes fails at once with "connection or response read failed" when it
follows an earlier one by some seconds. It reproduces without images (the
third of four plain messages 18 s apart failed), so it is a connection-reuse
problem in re:agent's transport. re:agent pull request 291 fixes it; the fix
has not been checked live from Covalent.

## Goal

Ask a chat for a picture and see it in the reply. You type (or say) a prompt,
the chat model decides it needs an image, calls a tool, and the image appears
in the thread with a short sentence beside it.

Not in this plan: images you supply (uploads, reference images, edits of an
earlier picture), choosing size or quality, more than one image backend, an
image gallery, and deleting images.

## The decision: re:agent's own tool

Images are made by re:agent's existing `generate_image` tool. The chat model
asks for the tool, re:agent runs it, and the result returns to the conversation
as an ordinary tool result. Covalent only shows what re:agent reports.

Two alternatives were considered and set aside:

| | re:agent's tool (chosen) | OpenAI's hosted tool in the chat request | Covalent calls OpenAI itself |
|---|---|---|---|
| Claude chats | Can draw | Cannot | Would need its own way to detect intent |
| Where the image lives | A file from the start | Base64 in history, resent every turn | A file |
| History, traces, limits, cancel | re:agent's, as for any tool | Partly | None |
| Shared with the CLI | Yes | No | No |

The user also plans more image backends inside re:agent later (Gemini, and a
local Flux.1 model for touch-ups). That work belongs behind the same tool. To
keep it cheap, nothing on Covalent's side and nothing in the tool a chat sees
names OpenAI: a prompt goes in, a file comes out, and re:agent chooses how.
This plan builds no backend switch.

## What re:agent has

`generate_image {prompt, output_path, reference_image_paths?}` calls OpenAI's
Images API with `OPENAI_API_KEY` (model `gpt-image-2.5-flare`), saves a PNG
that never overwrites a file, and returns a shrunken preview the model sees on
its next request. A Run's result already lists the created file in `effects`.

Three things keep a chat from using it:

| Where | What blocks a chat |
|---|---|
| `tools.rs` specs and execute, `tool_images.rs` | A tool-free Session declares no tools and refuses every call |
| `run_loop.rs` after a response | A tool-free response with any ToolCall ends the Run with `protocol_error` |
| `tools.rs` `ToolState::new` | A tool-free Session opens no folder, so there is nowhere to save |
| `catalog.rs` `supports_images` | Only OpenAI chat models are offered image tools |
| The tool's schema | The model must choose `output_path`, which means nothing in a chat |

### Probe result (2026-11-14)

One `reagent run` with `gpt-6.1-sol` on a ChatGPT plan and the API key for the
image, the tool's first run against the real service:

| | Result |
|---|---|
| Whole Run | 24.7 s, 2 Steps, 1 tool call |
| Deciding to call the tool | 4.9 s |
| Generating the image | 15.6 s |
| Writing the reply afterwards | 3.1 s |
| Image | 1536 × 1024 PNG, 1.4 MB, matching the prompt |
| Image cost (API key) | 1,372 output and 76 input tokens, about 4.2 cents |
| Preview shown to the model | 864 × 576, roughly 1,000 input tokens on the next request |

The model's reply linked to the saved file by name, which suits a terminal and
not a chat. Only the default size and quality were tried.

## Changes in re:agent

One pull request.

1. **A launch setting.** `SessionOptions.image_generation: bool`, allowed only
   with `tool_free`, saved in the checkpoint and reported in the snapshot, as
   `web_search` is. A Session with tools is unchanged: it already has the tool.
2. **The folder.** With the setting on, the Session's `workspace` is the
   folder images are saved in, and it must exist. This is the Session's only
   file authority: creating new PNG files in that one folder. The existing
   create-only, symlink-safe publication code is used as it is. Messages still
   carry the date, not a workspace snapshot.
3. **A chat-shaped declaration.** The Session declares exactly one tool,
   `generate_image {prompt}`. re:agent names the file (`image-<id>.png`); the
   model never supplies a path. The description says the image is shown to the
   user and says nothing about keys, paths or the backend. Reference images are
   not offered.
4. **The guards.** The tool-free refusals in `tools.rs`, `tool_images.rs` and
   the run loop let this one call through and still refuse everything else.
5. **Any chat model.** In such a Session the tool is offered to Anthropic
   models too. Claude gets the result as text (made, size, shown to the user)
   without the pixels, which is how re:agent already encodes image results for
   Anthropic. OpenAI models keep the preview.
6. **Progress.** The reply preview reports that an image is being generated,
   the way it reports `searching`, so a host can show it during the 15 seconds.
7. **Unchanged on purpose.** The image model, the limits, single-attempt
   behavior, and the rule that an uncertain submission stops the Run.
8. **Tests, offline**, against a fake Responses endpoint and a fake Images
   endpoint on one loopback base:
   - the request declares only the chat tool;
   - a call saves a PNG in the folder and the Run's effects name it;
   - the result continues to a reply in a second Step;
   - any other tool call is still a `protocol_error`;
   - the setting and folder survive resume;
   - an Anthropic Session is offered the tool and gets a text-only result;
   - a missing key returns the tool's error without a request;
   - a ninth image in one chat does not fail the request (older previews retire).
9. `make check`, and `make tty-check` only if the terminal display changes.

## Changes in Covalent

- **Turning it on.** New chats open with `image_generation` when
  `OPENAI_API_KEY` is set, the same condition as voice. Chats created earlier
  keep their saved setting. At most four tool calls per reply
  (`max_tool_calls`), which bounds what one message can cost.
- **Where images live.** One folder, `images/` in Covalent's data folder,
  given to every chat Session. One folder for all chats, because a Session
  needs its folder before the chat has an ID. File names are re:agent's unique
  IDs.
- **Recording.** When a Run ends, Covalent reads the created files from the
  result's effects, reads each PNG's width and height, and stores them with the
  message in a new `images` column, as it stores sources.
- **Serving.** `GET /api/images/{name}` serves a file only if some message
  records that name.
- **While it works.** The preview event carries the "generating" flag, and the
  reply shows a "Creating image…" tile.
- **When it is done.** The image sits above the reply's text at its recorded
  proportions, so the page does not jump. Clicking opens it full size; a small
  button downloads it.
- **Prompt.** The chat prompt says the model can create images, to do so only
  when asked for one, and to answer with a short sentence without naming files
  or repeating the prompt.
- **Voice.** No change: a spoken request shows the image in the thread and
  reads the short reply.
- **Docs.** A README section covering the key, billing, and what is sent.
- **Tests.** The fake provider gains a tool-call reply and an Images route.
  A route test covers the generating preview, the stored image, serving it, and
  refusing an unrecorded name.

## Done when

- "Draw a red sailboat" in an OpenAI chat and in a Claude chat each show an
  image and one short sentence, with no file name in the text.
- The "Creating image…" tile shows while it works, and Stop ends the wait.
- A reload shows the same image; so does a restart of Covalent.
- A chat that asks for no picture makes no image call.
- Without `OPENAI_API_KEY`, chats behave as they do today.
- `make check` passes in re:agent; Covalent's tests pass offline.

## Cost and privacy

About four cents per image at the default size and quality, billed to the API
key. The chat model's own tokens stay on the plan or the Anthropic key. The
image prompt goes to OpenAI even when the chat model is Claude. Images are
stored only on this machine.

## To verify while building

- A refused prompt: the tool's error should reach the model, which explains.
- Stopping during generation: OpenAI may still bill an image that is never
  saved.
- A reply stopped after its image was saved: whether re:agent keeps the tool
  result in the conversation, and so whether Covalent should show the image.

## Order and size

The re:agent pull request first, then the Covalent change once it merges, built
meanwhile against the sibling checkout through the local patch.

Rough size, tests included: two to three hundred lines in re:agent, three to
four hundred of Rust and about a hundred and fifty of TypeScript in Covalent.

## Open points

1. **Claude chats draw without seeing the result.** Planned. The alternative
   is OpenAI chats only until re:agent can show Claude an image.
2. **One images folder for all chats**, as planned, or a folder per chat, which
   needs the folder to be settable after the Session opens.
3. **The cap of four tool calls per reply.** A lower cap saves money on a
   runaway reply; a higher one allows "give me six variations".
4. **Size and quality** stay automatic. A later step can let the model ask for
   a shape (square, wide, tall).
5. **OpenAI chat models keep seeing their images** in later turns, about 1,000
   input tokens each for the eight most recent. Turning that off saves tokens
   and loses "what is in the picture you made".
6. **A stopped reply's finished image** is shown, since it exists and was paid
   for, if re:agent keeps it in the conversation.
