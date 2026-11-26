---
name: bump-reagent
description: Move Covalent to a newer re:agent commit. Use when a re:agent change has merged and Covalent should have it, such as a new model, new reasoning efforts or a fix.
---

# Move the re:agent pin

Covalent builds against one re:agent commit: the `rev` in `Cargo.toml`.
Everything re:agent merged since the old pin arrives with the new one, wanted or
not, so read what is coming before moving it.

## 1. Pick the commit

Use the commit at the head of re:agent's `main`:

```bash
git ls-remote https://github.com/satmihir/re-agent refs/heads/main
```

Never pin a branch commit. It goes away when the branch is squashed or deleted,
and Covalent stops building.

## 2. Read what comes with it

In a re:agent checkout, list the commits between the old `rev` and the new one
(`git log --oneline OLD..NEW`) and read its `docs/reagent-overview.md` for each
change that is not obviously internal. Ask of each one what it does to a chat
here. Every Covalent chat is opened by `chat_options` in `src/chats.rs`: web
search on, no commands, files in its own folder, and image generation when there
is an OpenAI key.

- **Catalog changes need no code.** The picker draws whatever
  `reagent::catalog::models()` lists: a new model is a new row or a new version
  on its family's row, and new efforts are new cells.
- **A new provider needs a decision.** If it cannot do what `chat_options` asks
  for, its models fail on the first reply. Keep it out with `searchable` in
  `src/chats.rs`, which already does this for Gemini, and say so in `README.md`.
- **Session API changes** show up as compile errors or failing tests.

## 3. Move it and test

Edit `rev`, then run `cargo fmt --check` and `cargo test`, which also updates
`Cargo.lock`. The picker test in `src/http.rs` names models and efforts; move it
to the new facts if they changed. If anything under `web/` changed, also run
`cd web && npm run typecheck && npm test && npm run build`.

The pull request says what the new pin brings in, what was kept out and why,
and what was not checked.

## 4. After the merge

Covalent runs from its owner's terminal, with that shell's keys. Give these
commands for the checkout it runs from, and do not restart it yourself:

```bash
git pull --ff-only
cargo build --release
```

Then they press Ctrl-C and start `target/release/covalent` again. If `web/`
changed, `cd web && npm run build` comes after the server build and right before
the restart: the page breaks while new page code is served by the old server.
