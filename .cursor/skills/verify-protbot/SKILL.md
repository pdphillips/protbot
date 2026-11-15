---
name: verify-protbot
description: Drive Protbot, the stdio MCP server for one Proton account (Mail through Proton Mail Bridge, Pass, Drive, and Calendar). Use when a change needs proof on a running server: launch from an empty data directory, decide the instance is worth driving, speak JSON-RPC over its tmux PTY the way an MCP host does, and keep the evidence.
---

# Verify Protbot

Protbot is a stdio MCP server. The user is the bot that spawns it and sends JSON-RPC on stdin. There is no web UI, no database, and no migration command. Mail is IMAP and SMTP to `127.0.0.1` only. Pass and Drive are child processes (`pass-cli`, `proton-drive`). Calendar is `curl` fetching `https://` ICS feeds.

Do not treat `cargo test` as proof that a user-facing change works. `cargo test` is the regression suite. This skill runs `target/debug/protbot` and talks to it.

Run every command from the repo root. Read the matching file in `features/` before driving a feature.

## Launch

There is no database. The data directory is empty when it has no `settings.toml`. On startup `SettingsStore::open` writes clamped defaults into that path (mode `0600`). That write is the seed. Values match `settings.example.toml`: `poll_interval_secs = 300`, `summary_cadence_secs = 3600`, `settings_reload_secs = 180`, and three empty arrays.

```bash
.cursor/skills/verify-protbot/scripts/launch.sh
```

What that command does:

1. Refuses when tmux session `protbot-verify` is already on socket `.verify/tmux.sock`.
2. Deletes `.verify/data` and recreates it with empty `runtime/`, `upload/`, and `download/` directories. It does not delete `.verify/evidence`.
3. Builds the current tree: `cargo build --manifest-path Cargo.toml --bin protbot`.
4. Mail backend, default `fixture`. Starts `.cursor/skills/verify-protbot/scripts/imap-fixture.py` on `127.0.0.1` with ephemeral IMAP and SMTP ports. The fixture speaks the Bridge subset implemented in `src/bridge.rs` (greeting, `CAPABILITY` without `STARTTLS`, `LOGIN`, `SELECT`, `UID FETCH`, `LIST`, `LOGOUT`, and SMTP without `STARTTLS`). It seeds one unread message, uid `7`, from `owner@proton.me`, subject `Verify mailbox`, body `hello from the loopback mailbox`, in `INBOX`, plus a `Sent` folder. Set `PROTBOT_VERIFY_MAIL=bridge` only when Proton Mail Bridge is already signed in on `127.0.0.1:1143`. That mode does not invent a password: `PROTON_PASS_AGENT_TOKEN`, `PROTON_BRIDGE_PASSWORD`, and `PROTBOT_BRIDGE_USER` must already be exported. The default fixture mode never connects to a real Bridge, even if port `1143` is open.
5. In fixture mode, when `PROTON_PASS_AGENT_TOKEN` or `PROTON_BRIDGE_PASSWORD` is unset, the script exports `pst_verify_fixture::NOTREAL` and `bridge-fixture` for this process only. It does not write them to a file or pass them as arguments. `PROTBOT_BRIDGE_USER` defaults to `bot@proton.me`.
6. Pass and Drive binaries: `pass-cli` and `proton-drive` when they are on `PATH`, otherwise `.cursor/skills/verify-protbot/scripts/proton-stand-in.sh`. Override with `PROTBOT_PASS_BIN` and `PROTBOT_DRIVE_BIN`.
7. Starts tmux session `protbot-verify` (2000 columns) running `.cursor/skills/verify-protbot/scripts/pane.sh`. The pane turns off echo and execs `target/debug/protbot`. Stdin and stdout stay on the PTY. Stderr goes to `.verify/data/stderr.log`. `pipe-pane` copies pane output to `.verify/data/stdout.log` through `.cursor/skills/verify-protbot/scripts/capture.py`.
8. The pane environment is the tmux server environment captured when `new-session` starts the server (not argv). A bare `start-server` exits on this tmux because `exit-empty` is on:

   - `PROTBOT_SETTINGS=.verify/data/settings.toml` (the file does not exist yet)
   - `XDG_RUNTIME_DIR=.verify/data/runtime`
   - `PROTBOT_UPLOAD_DIR=.verify/data/upload`
   - `PROTBOT_DOWNLOAD_DIR=.verify/data/download`
   - `PROTBOT_BRIDGE_HOST=127.0.0.1`
   - `PROTBOT_IMAP_PORT` and `PROTBOT_SMTP_PORT` from the fixture, or `1143` and `1025` in bridge mode
   - `ALLOW_WRITES` passed through, or `false` when unset
   - `PROTBOT_DISABLE_WATCHER`, `PROTBOT_BRIDGE_PROBE`, and `PROTBOT_CALENDAR_ICS` are unset, so the inbox watcher runs and the Bridge probe runs

9. Waits until stderr contains the line `protbot ready`. The process prints that only after the IMAP probe logs in and logs out. If the probe fails, the process exits and stderr says Bridge is not signed in. That is not a usable server.
10. Checks the seed file is mode `0600` and contains the six default lines above.
11. Sends this initialize line, then `notifications/initialized`:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"verify-protbot","version":"0"}}}
```

Usable means all of these are true: stderr has `protbot ready`, `.verify/data/settings.toml` has the seed above, and the initialize response on stdout contains `"serverInfo":{"name":"protbot","version":"0.1.0"}`. The script prints `protbot verify: ready` and the backends it bound.

Teardown is `cleanup.sh` below. A failed launch runs it before exiting.

Signed-in Bridge, Pass, and Drive are optional. This checkout does not ship a Proton session. Fixture mode is the launch that runs from a fresh tree.

## Doctor

One read-only check. It does not call `tools/call` and it does not change `settings.toml`.

```bash
.cursor/skills/verify-protbot/scripts/doctor.sh
```

Worth driving only when every line below holds:

- tmux session `protbot-verify` is on `.verify/tmux.sock` and the recorded pid's command is `target/debug/protbot`
- `.verify/data/stderr.log` contains `protbot ready`
- `settings.toml` is mode `0600`, `poll_interval_secs` is inside `30..=86400`, and the file contains no `http://` or `https://`
- a TCP connection to the recorded IMAP port receives a greeting containing `OK` (the connection is closed without `LOGIN`)
- MCP `ping` returns `{"result":{}}`
- stderr, stdout, and `settings.toml` do not contain the live token or bridge password

Exit 0 prints `protbot doctor: worth driving` plus pid, backends, and the settings path. Anything else prints `protbot doctor: not worth driving` and a reason, and exits 1. Do not drive a failing doctor.

## Drive

The harness is tmux. The pane is the server's stdin. `drive.sh` types one JSON object and Enter, then reads the new line from `.verify/data/stdout.log` whose `"id"` matches and which contains `"result"` or `"error"`.

```bash
.cursor/skills/verify-protbot/scripts/drive.sh mail
.cursor/skills/verify-protbot/scripts/drive.sh pass
.cursor/skills/verify-protbot/scripts/drive.sh drive
.cursor/skills/verify-protbot/scripts/drive.sh calendar
.cursor/skills/verify-protbot/scripts/drive.sh settings
```

Hand-typing the same call after `doctor.sh` has passed:

```bash
tmux -S .verify/tmux.sock send-keys -t protbot-verify -l '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"mail_list","arguments":{"folder":"inbox","unread_only":false}}}'
tmux -S .verify/tmux.sock send-keys -t protbot-verify Enter
```

The response is one JSON line in `.verify/data/stdout.log`. Tool text is `result.content[0].text`. `result.isError` is true when the tool refused.

`drive.sh` arguments and the side effects it checks are in `features/`. It does not send mail, create a Pass login, or upload a file when `ALLOW_WRITES` is on. Those writes need a separate launch with `ALLOW_WRITES=true` and, for send, `PROTBOT_RECIPIENT_ALLOWLIST`. The read-only launch proves the write gate by calling the write tool and requiring `writes are disabled; set ALLOW_WRITES=true`, with no SMTP `DATA` and no Pass template file.

There is no Playwright, Cypress, or HTTP app in this repo. Do not add a browser. Do not point `PROTBOT_BRIDGE_HOST` at anything but loopback; the binary rejects other hosts before it connects.

## Evidence

Write evidence under `.verify/evidence/`. `drive.sh` already writes these files:

| Drive | Files | What proves the user path |
| --- | --- | --- |
| `mail` | `mail.json`, `mail-backend.log`, `mail-audit.txt`, `mail.txt` | `mail_list` returns uid `inbox:7` from `owner@proton.me` with subject `Verify mailbox`. `mail_read` of `inbox:7` returns `hello from the loopback mailbox`. The fixture log contains `BODY.PEEK[]`. Stderr contains `protbot tool=mail_read id=inbox7` (the colon is stripped). `mail_notifications` is `[]` because the watcher baselines the inbox and does not replay it. `mail_send` is an error and the fixture log has no `SMTP DATA`. |
| `pass` | `pass.json`, `pass-cli.log`, `pass-audit.txt`, `pass.txt` | Stand-in mode: vault `v1` / `Bot`, item `item1`, cli log lines `info --output json`, `reason=protbot pass_session`, `vault list --output json`, `reason=protbot pass_vaults`, `item list --vault-name Bot --output json`. The create call is refused and `data/pass/last-create.json` does not appear. The password is not in the cli log. |
| `drive` | `drive.json`, `drive-cli.log`, `drive-audit.txt`, `drive.txt` | Stand-in mode: `drive_list` of `/my-files` returns `notes.txt` and the cli log contains `filesystem list --json /my-files`. `/my-files/../secret` returns `path traversal rejected` and does not spawn the cli. Upload is refused while writes are off. |
| `calendar` | `calendar.json`, `calendar-audit.txt`, `calendar.txt` | With no `PROTBOT_CALENDAR_ICS` and `calendar_feeds = []`, the text is `{"failed_feeds":0,"events":[]}`. |
| `settings` | `settings.json`, `settings.toml`, `settings.txt` | `whoami` returns the mailbox and `"bridge":"127.0.0.1"`. `settings_update` with `poll_interval_secs` `1` and keyword `otp` is clamped to `30`. The on-disk file contains `poll_interval_secs = 30` and `otp`. |

Also keep doctor stdout. Copy it yourself:

```bash
.cursor/skills/verify-protbot/scripts/doctor.sh | tee .verify/evidence/doctor.txt
```

Every evidence file is scanned for the live `PROTON_PASS_AGENT_TOKEN` and `PROTON_BRIDGE_PASSWORD`. A hit deletes the proof: the script exits 1 and names the file, not the secret. Do not paste those values into the evidence directory, the PR, or stdout.

On a signed-in Bridge (`PROTBOT_VERIFY_MAIL=bridge`), `drive.sh mail` only checks that `mail_list` returns JSON. It does not look for the fixture subject. Do not run the fixture assertions against a real mailbox.

## Cleanup

```bash
.cursor/skills/verify-protbot/scripts/cleanup.sh
```

This stops only what launch started:

1. Sends `C-d` to the pane so protbot sees EOF, returns from `mcp::serve`, and drops `Config`, which deletes `$XDG_RUNTIME_DIR/protbot-<pid>`.
2. If that pid is still alive and its command line contains `/protbot`, sends `SIGTERM` then `SIGKILL`. Any other command is left alone and the script exits 1.
3. If launch started the fixture, kills that pid only when its command line contains `imap-fixture.py`.
4. Kills the tmux server on `.verify/tmux.sock` only, then removes that socket.

It does not delete `.verify/evidence` or `.verify/data`, and it does not signal processes by name. A second run with no state file prints `protbot verify: nothing to stop`.

`launch.sh` is what starts from an empty data directory. It removes `.verify/data` only when the session is down.

## Helpers

| Script | Invocation |
| --- | --- |
| `scripts/launch.sh` | `.cursor/skills/verify-protbot/scripts/launch.sh` |
| `scripts/doctor.sh` | `.cursor/skills/verify-protbot/scripts/doctor.sh` |
| `scripts/drive.sh` | `.cursor/skills/verify-protbot/scripts/drive.sh mail` |
| `scripts/cleanup.sh` | `.cursor/skills/verify-protbot/scripts/cleanup.sh` |
| `scripts/pane.sh` | Not run by hand. The tmux session's command is this path. |
| `scripts/capture.py` | Not run by hand. `pipe-pane` runs `python3 -u .cursor/skills/verify-protbot/scripts/capture.py .verify/data/stdout.log`. |
| `scripts/imap-fixture.py` | Started by `launch.sh`. Manual form, with `PROTON_BRIDGE_PASSWORD` already exported and not placed in argv: `python3 .cursor/skills/verify-protbot/scripts/imap-fixture.py --imap-port 0 --smtp-port 0 --ready-file .verify/data/fixture.ready --log .verify/data/backend.log --smtp-copy .verify/data/smtp-last.bin` |
| `scripts/proton-stand-in.sh` | Not run by hand. Launch sets `PROTBOT_PASS_BIN` and `PROTBOT_DRIVE_BIN` to this path when `pass-cli` or `proton-drive` is absent. |
| `scripts/lib.sh` | Sourced by the shell scripts. Do not execute it. |

`PROTBOT_VERIFY_DIR` relocates `.verify` (default is `.verify` under the repo root). `PROTBOT_TMUX_BIN` selects the tmux binary (default `tmux`).
