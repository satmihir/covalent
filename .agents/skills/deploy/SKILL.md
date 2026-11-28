---
name: deploy
description: Put what is on Covalent's `main` into the running server. Use after a change is pushed or merged, or when asked to update, restart or check the running Covalent.
---

# Update the running Covalent

Covalent runs on its owner's desktop, a Linux machine, as the systemd user
service `covalent`. It runs from a checkout of `main` at `~/src/covalent`. Its
keys and `COVALENT_HOSTS` are in that checkout's `.env`, which git ignores, and
its chats are in `~/.covalent`. The machine's SSH address is kept out of this
repository: take it from your notes on the project, or ask.

Each chat also has a re:agent Session, saved under `~/.cache/reagent/sessions`
in a folder named by the chat's ID. A Session records its chat folder's full
path and a checksum of itself, so a backup needs both places, and a move to
another home directory needs those paths rewritten and the checksums redone.

## 1. Push first

The desktop builds what GitHub has. Never copy source files to it or edit them
there: the next pull would fail or quietly lose the change.

## 2. Pull, build, restart

```bash
ssh HOST 'cd ~/src/covalent && git pull --ff-only && ~/.cargo/bin/cargo build --release --locked && (cd web && npm ci && npm run build) && systemctl --user restart covalent'
```

The page is built last, right before the restart: new page code served by the
old server can come up blank. A restart cuts off any reply being written, which
the chat then shows as interrupted, so say when you are about to restart.

## 3. Check it came back

```bash
ssh HOST 'systemctl --user is-active covalent; journalctl --user -u covalent -n 5 --no-pager -o cat; curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:7420/api/models'
```

Expect `active`, a line saying it is serving, and `200`. Say which commit is
now running (`git -C ~/src/covalent log --oneline -1`).

## What is set up there

The unit is `~/.config/systemd/user/covalent.service`:

```ini
[Unit]
Description=Covalent chat server
After=network-online.target

[Service]
WorkingDirectory=%h/src/covalent
# The keys and COVALENT_HOSTS; git ignores this file.
EnvironmentFile=%h/src/covalent/.env
ExecStart=%h/src/covalent/target/release/covalent
Restart=on-failure

[Install]
WantedBy=default.target
```

Lingering is on for the owner's user (`loginctl enable-linger`), so the service
starts at boot without anyone logging in. Tailscale publishes it to the owner's
own devices with `tailscale serve --bg 7420`; never use `tailscale funnel`.

The iPhone app is not part of this: it is built and installed from a Mac.
