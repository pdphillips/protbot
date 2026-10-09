# Protbot

Protbot is a side gate for an unattended Grok bot that has its own Proton account on a Duo plan, separate from the owner's vault. The account covers Mail, Pass, Drive, and Calendar. Protbot exists so the bot can read and, when allowed, send mail and store site credentials in its own encrypted Pass vault. Mail goes through the official Proton Mail Bridge on that computer. There is no unofficial mail CLI, no Proton account password on disk, and no plaintext secrets in the repo. The owner sees the bot's vault by logging into the bot's account.

It is a stdio [MCP](https://modelcontextprotocol.io) server. The bot spawns `scripts/spawn.sh`, which injects secrets from the local `pass` store and execs the binary. Protbot talks JSON-RPC on stdin and stdout. Mail is IMAP and SMTP to Bridge on `127.0.0.1` only (IMAP `1143`, SMTP `1025`). Pass uses `pass-cli` with a scoped agent token. Drive uses the official `proton-drive` CLI. Calendar is a read-only ICS link.

Bridge is mail only, and it requires the bot's paid Proton account. A free account cannot sign into it. The user signs Bridge in once on the bot's screen, with that account's password and 2FA. Bridge keeps the session. Protbot never stores the account password and never prompts for it.

Protbot is unofficial and is not affiliated with Proton AG.

## Setup

On the bot's machine, once:

1. Install `proton-drive` and sign it in as the bot account. That CLI keeps its session in the OS keyring. Protbot never sees the Proton password and does not write it down.
2. Sign Proton Mail Bridge in once on this screen as the bot account (password and 2FA). In Bridge, copy the mailbox address and the Bridge IMAP password. That password is not the account password.
3. In the bot's Proton Pass account, create an agent token scoped to the vaults the bot should touch.
4. Store both secrets in the local `pass` password-store, first line only:

   ```bash
   pass insert -m protbot/agent-token
   pass insert -m protbot/bridge-password
   ```

5. Build the release binary on a machine that has Rust, then publish it. The bot's machine does not need a toolchain.

   ```bash
   scripts/release.sh
   gh release create v0.1.0 dist/protbot dist/protbot.sha256 dist/version
   ```

   `dist/protbot.sha256` is the SHA-256 checksum and is uploaded next to the binary. `dist/version` is the plain version string.

6. On the bot's machine, point the bootstrap at that release and run it:

   ```bash
   export PROTBOT_RELEASE_BASE=https://github.com/pdphillips/protbot/releases/latest/download
   export PROTBOT_RECIPIENT_ALLOWLIST='owner@proton.me'
   export PROTBOT_BRIDGE_USER='bot@proton.me'
   scripts/bootstrap.sh
   ```

The bootstrap script downloads `version` first and skips the binary download when `~/.local/bin/protbot.version` already matches. Otherwise it downloads `protbot` and `protbot.sha256`, checks the checksum, and installs the binary mode `0755`. It writes `~/.config/protbot/settings.toml` from `settings.example.toml` if that file is absent. If `pass-cli` is missing, the script installs it with Proton's official one-liner. If Proton Mail Bridge is missing and `apt-get` is available, it installs Proton's official deb. It then checks `protbot/agent-token` and `protbot/bridge-password` without printing them, and it checks that Bridge is listening on `127.0.0.1:1143`. A missing pass entry or a missing Bridge session stops the script. A restored Grok Bot computer wipes both. The script does not prompt and does not invent a password.

Registration does not call `grok mcp add`. It installs `protbot-spawn` and writes `~/.config/protbot/connector.json`. The connector command is that spawn script. Tell the bot to add a custom stdio MCP server that runs it. The JSON holds the mailbox address and ports, not the Bridge password and not the agent token.

```bash
curl -fsSL https://proton.me/download/pass-cli/install.sh | bash
```

`scripts/spawn.sh` is what the bot executes. If `PROTON_PASS_AGENT_TOKEN` or `PROTON_BRIDGE_PASSWORD` is already set, spawn leaves that value alone. Otherwise it reads `protbot/agent-token` and `protbot/bridge-password` with `pass show`, with GnuPG set to `--batch --pinentry-mode error` and stdin closed, and exports them for the child process only. A missing entry exits with a message. Spawn does not prompt. The binary takes no arguments, so the secrets cannot be smuggled through argv.

To revoke access, sign Bridge out or change the bot account password, and kill the agent token from the bot's Pass settings in a browser, or with `pass-cli pat revoke` if you hold a copy of the token.

## Security model

- This runs on a Grok Bot cloud computer that the operator can read, shared by every bot on that account. A passphrase-less `pass` store is a convenience for the unattended restart, not a defense against the host. The real controls are the bot's separate account, signing Bridge out or changing that account password, and revoking the Pass agent token from a browser.
- The agent token lives in the process environment (`PROTON_PASS_AGENT_TOKEN`). Protbot copies it to `PROTON_PASS_PERSONAL_ACCESS_TOKEN` only for `pass-cli` children. The Bridge IMAP password lives in `PROTON_BRIDGE_PASSWORD` and is used only for the loopback IMAP and SMTP sessions. Drive and calendar fetches receive neither secret.
- Bridge hosts other than `127.0.0.1`, `localhost`, and `::1` are rejected before a connection is opened. STARTTLS is used when Bridge offers it. The Bridge certificate is accepted only after that loopback check, because Bridge generates it locally.
- Every `pass-cli` invocation sets `PROTON_PASS_AGENT_REASON` to a short reason of the form `protbot <tool> item <id>`. Pass stores that reason in the agent audit log.
- The token is never written to the repo, the binary, a `.env` file, a log line, or argv. Login item passwords go to `pass-cli` on stdin (`--from-template -`), not on the command line.
- Stderr records `protbot tool=<name> id=<id>` and nothing else about the call. Message bodies, subjects, tokens, and feed URLs are not logged. Tool results still return mail and Pass contents to the model, because that is the point of the tool.
- Pass session files, if `pass-cli` creates them, go under `$XDG_RUNTIME_DIR/protbot-<pid>` (or `/dev/shm`) mode `0700`, and that directory is removed when the process exits.
- The server is read-only unless `ALLOW_WRITES=true`. Sends, trash, delete, move, Pass create, Pass delete, Drive upload, and Drive trash all refuse without it.
- `mail_send` checks every To, Cc, and Bcc address against `PROTBOT_RECIPIENT_ALLOWLIST`. Entries are exact addresses or `@domain`. An empty allowlist rejects every send.
- Drive paths must be absolute. Any `..` segment is rejected before a CLI is started. Uploads must sit inside `PROTBOT_UPLOAD_DIR`. Downloads must sit inside `PROTBOT_DOWNLOAD_DIR`.
- Tool arguments are deserialized with serde and unknown fields are rejected.
- Each CLI call runs under `tokio::time::timeout`. On timeout the process group is killed, so a hung CLI does not freeze the server.
- Direct dependencies are pinned with `=` versions. `Cargo.lock` pins the rest. `rust-toolchain.toml` pins Rust `1.99.0`.

## Mail checking

A background watcher lists the Bridge inbox once per poll. The first poll records the current message ids and does not notify, so a restart does not replay the inbox.

After that:

- Mail from `sender_allowlist`, and mail whose subject or one-line summary contains an `importance_keywords` entry, is pushed immediately. The push is an MCP `notifications/message` with sender, subject, and a one-line summary.
- All other new mail is held and emitted as one batch when the summary cadence elapses, and only if something arrived. A quiet hour sends nothing and costs the model nothing.
- Message ids are remembered (capped at 10,000). An id reported on the immediate path is not reported again in the batch.

`mail_notifications` returns the same notices if the host did not surface the push. Headers, folder listings, and calendar events are cached for 30 seconds. Any successful write clears that cache. Listing mail is one IMAP fetch for the mailbox, not one round trip per message.

## Settings

`settings.toml` is read at startup and again every `settings_reload_secs`. Edit the file in place, or call the `settings_update` tool. Either way the new values apply without a restart. Clamp ranges:

| Key | Default | Clamp |
| --- | --- | --- |
| `poll_interval_secs` | 300 | 30–86400 |
| `summary_cadence_secs` | 3600 | 300–604800 |
| `settings_reload_secs` | 180 | 30–3600 |

`sender_allowlist` holds exact addresses or `@domain` entries (up to 64). `importance_keywords` holds up to 32 strings, each truncated to 64 characters. `calendar_feeds` holds Pass item refs of the form `vault/item`, not URLs. The URL is read from that item at runtime. `PROTBOT_CALENDAR_ICS` may also hold comma-separated `https://` feed URLs in the environment. Feed URLs are not written into the settings file and are not logged.

A value below the floor is raised when the file is read, so a tool cannot set the poll interval to a few seconds and burn Proton rate limits. `settings.example.toml` is the template the bootstrap installs.

## Tools

`mail_list`, `mail_read`, `mail_folders`, `mail_notifications`, `pass_vaults`, `pass_items`, `pass_item_view`, `pass_totp`, `drive_list`, `drive_info`, `drive_download`, `calendar_events`, `settings_get`, `settings_update`, `whoami`.

Writes: `mail_send`, `mail_trash`, `mail_delete`, `mail_move`, `pass_item_create_login`, `pass_item_delete`, `drive_upload`, `drive_trash`.

## Mail and CLI contract

Bridge defaults are host `127.0.0.1`, IMAP `1143`, SMTP `1025`. Override the ports with `PROTBOT_IMAP_PORT` and `PROTBOT_SMTP_PORT`. `PROTBOT_BRIDGE_USER` is the mailbox address Bridge shows. The Bridge password comes from spawn, not from the settings file.

Pass, Drive, and calendar still shell out. Binaries default to `pass-cli`, `proton-drive`, and `curl`. Override with `PROTBOT_PASS_BIN`, `PROTBOT_DRIVE_BIN`, and `PROTBOT_CURL_BIN`.

Pass uses `vault list`, `item list`, `item view`, `item create login --from-template -`, `item delete`, and `item totp`, with `--output json`. The agent token and reason are environment variables.

Drive uses `filesystem list|info|upload|download|trash`. `--json` is passed for list and info.

Calendar feeds are fetched with `curl --config -` so the URL is on stdin, and only `https://` URLs are accepted.

## Tests

```bash
cargo test
```

Unit tests cover allowlist checks, settings clamps, cache expiry and invalidation, path traversal, mail-list parsing, token redaction, the immediate-versus-batch dedupe, and rejection of a non-loopback Bridge host. Integration tests drive Pass and Drive with a fake CLI, speak IMAP to a loopback fixture, and speak stdio JSON-RPC to the binary, including a hung CLI that must be killed. A missing `pass` entry exits instead of prompting. GitHub Actions runs `cargo test` on every push.

## License

Protbot is open source software under the [MIT License](LICENSE).

The creator is not responsible for any issues you might have with it. You use this software at your own risk.
