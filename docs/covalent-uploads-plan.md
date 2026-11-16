# Covalent — plan for uploads (milestone 12)

Written 2026-11-14 against re:agent `8799b9a1` (main) and Covalent `6ccafac`.
It continues [the workspace plan](covalent-workspace-plan.md). Like that page,
this is a plan: when the code and this page disagree, the code is right.

**Status, 2026-11-14:** milestone 12 is built, with the open points settled as
planned. The re:agent side merged as pull request 293; Covalent is pinned to
re:agent `c1a47cd2`.
As built:

- In re:agent, Claude sees images only on the offered models (Haiku, Sonnet
  and Opus 5.5). Any other Claude model gets no pixels and no error, as before,
  and a model-switch handoff says pixels are not supplied only to such a model.
- Context GC now retires older previews for any provider, so tool-free Claude
  chats from milestone 9 also see their images and keep working past eight.
- A pasted picture called `image.png` is renamed to `Pasted <date> <time>.png`.
- Attachment thumbnails in the thread load eagerly; with lazy loading, one
  never loaded in the in-app browser.

Checked live on port 7421:

- A Claude chat (`claude-haiku-5-5`) described an uploaded picture after
  `view_image` (7 s). It edited it into a sunset version with the original as
  the reference (24 s): the first live run of re:agent's reference editing.
  It said plainly that it cannot read an uploaded PDF.
- An OpenAI chat (`gpt-6.1-sol`) described an uploaded picture (10 s), made a
  winter version from it (34 s), and added a correct total row to an uploaded
  CSV (17 s).
- A second upload named `boat.png` was saved as `boat-2.png`.
- In the page, a pasted picture and a dropped text file both attached, showed
  as chips, uploaded on send, and appeared in the bubble after a reload. The
  model answered about both, and clicking an attachment opened it in the panel.
- An older chat showed no attach button.

Found and fixed while checking:

- A paste and a drop in quick succession kept only the second file.
- A `view_image` row read "Used view_image"; it now reads "Looked at".

Not checked: the paperclip's file picker (a browser under automation cannot
open it), what an image costs in Claude input tokens, HEIC photos, and
switching a chat with images between providers.

## Goal

Give a chat your own files. Attach files or images to a message, work on them
with the chat, and download the results. With the folder, the file tools and
the files panel already there, this completes the loop: bring a file in,
iterate on it, take it out.

Not in this plan: reading PDF and Office files, uploading folders, removing or
renaming files from the page, uploads in voice mode, and uploads in chats
created before folders existed.

## How it works

1. **Attach.** Three ways: a paperclip button in the composer, dragging files
   onto the chat, or pasting into the composer (a screenshot, or files copied
   in Finder). Each attachment is a chip above the message box, with a
   thumbnail for images, and can be removed before sending.
2. **Send.** The browser uploads each file into the chat's folder, then sends
   the message naming them. The user's bubble shows the attachments, and
   clicking one opens it in the files panel.
3. **The model is told in the message's note**, the channel voice mode uses:
   which files the user attached, with their kind and size. Nothing is put
   into the model's context automatically. It reads or looks at a file with
   its tools when it needs to, so a large file costs nothing until opened.
4. **Iterate.** Text files are edited in place. Images are edited with
   `generate_image` and reference images, and each edit is a new file, so the
   original is kept.
5. **Download** from the files panel, as today.

## The decisions

| Choice | Decision | Why |
|---|---|---|
| How the model learns of an upload | The message's note, then its own tools | No new Session API; a file costs tokens only when the model opens it |
| Claude and images | Fix in re:agent first: Claude models can see images | Otherwise a Claude chat edits an uploaded photo blind and cannot describe it |
| When files upload | At send | Simpler, and removing a chip leaves no stray file |
| Where they go | The folder's top level, under their own names | The model and the user see the same names |
| Same name twice | Keep both: `photo.jpg`, `photo-2.jpg` | Nothing the user or the model made is ever replaced by an upload |
| Limits | 20 MiB per file, 10 files per message | 20 MiB is re:agent's own cap for an image it will open |
| PDF and Office files | Stored and downloadable, not readable | Reading them needs a new re:agent tool; commands could, and are off |

## What re:agent has

- The file tools read and edit UTF-8 text up to 1 MiB.
- `view_image {path}` shows the model a PNG, JPEG or still WebP as a shrunken
  preview (at most 2,048 pixels and 512 KiB). `generate_image` edits or
  composes from one to four reference images and saves a new PNG. Editing with
  references has never been run against the real service.
- At most eight previews are sent per request; older ones are retired.
- A message may carry a host note of at most 1 KiB.
- An empty message is accepted.

Two things are in the way:

| Where | What is in the way |
|---|---|
| `anthropic.rs`, `catalog.rs` `supports_images` | The Anthropic encoder sends no pixels, and `view_image` is offered to OpenAI models only. A Claude chat is only told that an image exists. |
| `session_api.rs` `MAX_NOTE_BYTES` | 1 KiB cannot hold the names of ten attachments beside the spoken-message note. |

## Changes in re:agent

One pull request.

1. **Claude sees images.** The Anthropic encoder sends each active preview as
   an image block: inside the tool result for `view_image` and
   `generate_image`, and beside the text for a user Entry that carries images.
   A preview whose pixels are missing becomes the same "pixels unavailable"
   text the OpenAI encoder writes.
2. **The model gate.** `supports_images` accepts the offered Claude models, so
   a Session with tools offers them `view_image`, and `generate_image` without
   needing `image_generation`. The OpenAI encoder's own check is unchanged.
3. **Limits stay shared.** The eight-preview limit, context GC's retirement of
   older previews, and the request size check apply to Anthropic requests as
   they do to OpenAI ones. The image-token estimate gains an Anthropic rate if
   the existing one does not fit.
4. **A longer note.** `MAX_NOTE_BYTES` becomes 4 KiB.
5. **Unchanged on purpose.** The tool-free chat form of `generate_image`,
   which never showed pixels to Claude and is used only by older chats, the
   preview sizes, and single-attempt behavior.
6. **Tests, offline**, against the fake provider:
   - an Anthropic Session with tools is offered `view_image`;
   - a viewed image reaches the next request as an image block in the tool
     result, and a generated one does too;
   - a ninth image retires the oldest preview on an Anthropic request;
   - a missing preview becomes text, not a failed request;
   - a 4 KiB note is accepted and one byte more is refused.
7. `make check`. No terminal display changes, so no `make tty-check`.
8. Docs: the overview's Images paragraph and tool table, the image design
   page, and the Session API's note size.

## Changes in Covalent — milestone 12

- **Upload route.** `PUT /api/chats/{id}/files/{name}` with the file as the
  request body, at most 20 MiB. No multipart, so no new dependency feature.
  The server:
  - takes only the last part of the name, and refuses an empty name, `.`,
    `..`, a name that starts with a dot (re:agent's tools skip `.env` and
    `.git`), control characters, and names over 255 bytes;
  - creates the file only if it does not exist, and otherwise tries `-2`,
    `-3`, and so on before the extension;
  - answers with the path it used and the size.
- **Sending.** The send body gains `attachments`, a list of paths. Covalent
  checks each is a file in the chat's folder, records them with the user's
  message in a new `messages.attachments` column, and writes the note: each
  file's path, kind (image, text or other) and size. A spoken message keeps
  its own note first.
- **A message with files and no text** is allowed. Its title comes from the
  file names.
- **Composer.** A paperclip button, drop onto the chat, and paste. Attachment
  chips with a remove button; image chips show a thumbnail from the local
  file. Send uploads the files in order, then posts the message. If an upload
  fails, nothing is sent and the notice says which file and why.
- **Thread.** The user's bubble lists its attachments under the text: pictures
  as small thumbnails, other files as names. Clicking one opens it in the
  files panel.
- **Prompt.** A paragraph for workspace chats:
  - the user can attach files, which are in the folder under the names the
    note gives;
  - look at an image with `view_image` before describing or editing it;
  - edit an image by generating a new one with the original as a reference,
    under a new name, and say which file is the result;
  - say plainly when a file is a kind it cannot read.
- **Retry.** Retrying a failed reply sends the same text and the same
  attachments, which are already in the folder.
- **Docs.** The README's Files section gains uploads: how to attach, the
  limits, what each kind of file allows, and that uploads are never deleted.
- **Tests.**
  - A route test: an upload lands in the folder under its name; a second with
    the same name gets `-2`; refused names and an oversized body are refused;
    a send with attachments stores them and the request the fake provider
    sees carries the note; an attachment outside the folder is refused.
  - Node tests for the logic that is not layout: reading files from a paste
    and a drop, the limits, and the note's kind for a file name.

## Done when

- Dropping a CSV on a chat and asking for a change edits it in place, and the
  panel shows the new text.
- Pasting a screenshot and asking "what is this?" gets a description, in an
  OpenAI chat and in a Claude chat.
- Uploading a photo and asking for an edit produces a new image file, with
  the original kept, in both kinds of chat.
- Uploading the same name twice keeps both files.
- A PDF uploads, shows in the panel as a download, and the model says it
  cannot read it.
- A reload shows the attachments on the user's message.
- An older chat shows no attach button.
- `make check` passes in re:agent; Covalent's tests pass offline.

## Cost and privacy

- An uploaded file goes to the model's provider only when the model opens it:
  text as input tokens, an image as a shrunken preview. Each of the eight
  most recent previews is resent with later requests, roughly 1,000 input
  tokens each on OpenAI.
- Looking at an image is one extra model request, a few seconds, because the
  model calls `view_image` first.
- An image edit costs about what a generation does, on the OpenAI API key,
  and sends the reference image to OpenAI even in a Claude chat.
- Uploads stay on this machine in the chat's folder and are never deleted.

## To verify while building

- Image editing with a reference against the real service, which re:agent has
  never run: that it works, what it costs, and how long it takes.
- Whether Claude models accept image blocks inside tool results at re:agent's
  preview size, and what an image costs in input tokens there.
- Switching a chat with images between OpenAI and Claude models.
- What the model does with a HEIC photo or an animated GIF, which `view_image`
  does not accept.
- That a pasted screenshot's default name (`image.png`) is tolerable once
  several are in one chat.

## Order and size

The re:agent pull request first. Covalent's milestone 12 is built meanwhile
against the sibling checkout through the local patch and lands once the pull
request merges. Only the longer note is strictly needed by Covalent; Claude
seeing images is what makes uploads worth having in Claude chats.

Rough size, tests included:

| | Lines |
|---|---|
| re:agent pull request | 200 to 300 |
| Covalent milestone 12 | about 250 of Rust, 400 of TypeScript |

## Open points

1. **Images attach as files, not as pixels in the message.** Planned. The
   model must call `view_image`, one extra request. The alternative is a
   Session API change that sends a message with its images already in view,
   which is faster for "what is this screenshot?" and is more to build.
2. **Uploads at send**, as planned, or as soon as a file is attached, which
   shows progress earlier and can leave stray files.
3. **The limits**: 20 MiB per file and 10 files per message.
4. **Dot files are refused.** The alternative stores them where the model
   cannot see them.
5. **PDF and Office files** stay unreadable. A later step can add a re:agent
   tool that extracts their text.
6. **Removing a file from the page** is not planned; the model can delete one
   when asked.
