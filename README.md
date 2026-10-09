# Protbot

Protbot is a side gate for an unattended Grok bot that has its own Proton account on a Duo plan, separate from the owner's vault. The account covers Mail, Pass, Drive, and Calendar. Protbot exists so the bot can read and, when allowed, send mail and store site credentials in its own encrypted Pass vault without Proton Mail Bridge, without a paid Bridge feature, and without plaintext secrets on disk. The owner sees the bot's vault by logging into the bot's account.

It is a stdio [MCP](https://modelcontextprotocol.io) server. Grok spawns it and talks JSON-RPC on stdin and stdout. The server does not speak Proton's APIs itself. It shells out to Proton's official CLIs: `proton-mail` for mail, `pass-cli` for Pass, and `proton-drive` for Drive. Calendar is read from ICS feed URLs.

Protbot is unofficial and is not affiliated with Proton AG.

## Setup

On the bot's machine, once:

1. Install `proton-mail` and `proton-drive` and sign each CLI in as the bot account. Those CLIs keep their session in the OS keyring. Protbot never sees the Proton password and does not write it down.
2. In the bot's Proton Pass account, create an agent token scoped to the vaults the bot should touch. This is the only credential Protbot holds.
3. Store that token in the local `pass` password-store, first line only:

   ```bash
   pass insert -m protbot/agent-token
   ```

4. Build the release binary on a machine that has Rust, then publish it. The bot's machine does not need a toolchain.

   ```bash
   scripts/release.sh
   gh release create v0.1.0 dist/protbot dist/protbot.sha256 dist/version
   ```

   `dist/protbot.sha256` is the SHA-256 checksum and is uploaded next to the binary. `dist/version` is the plain version string.

5. On the bot's machine, point the bootstrap at that release and run it:

   ```bash
   export PROTBOT_RELEASE_BASE=https://github.com/OWNER/protbot/releases/latest/download
   export PROTBOT_RECIPIENT_ALLOWLIST='owner@proton.me'
   scripts/bootstrap.sh
   ```

The bootstrap script downloads `version` first and skips the binary download when `~/.local/bin/protbot.version` already matches. Otherwise it downloads `protbot` and `protbot.sha256`, checks the checksum, installs the binary mode `0755`, writes `~/.config/protbot/settings.toml` from `settings.example.toml` if that file is absent, and registers the server. If `pass-cli` is missing, the script installs it with Proton's official one-liner:

```bash
curl -fsSL https://proton.me/download/pass-cli/install.sh | bash
```

Registration runs this command (`scripts/register.sh` is the same call). The token is not an argument and is not an environment entry in the Grok config:

```bash
grok mcp add protbot \
  -e PROTBOT_BIN="$HOME/.local/bin/protbot" \
  -e ALLOW_WRITES="${ALLOW_WRITES:-false}" \
  -e PROTBOT_RECIPIENT_ALLOWLIST="${PROTBOT_RECIPIENT_ALLOWLIST:-}" \
  -e PROTBOT_SETTINGS="${PROTBOT_SETTINGS:-$HOME/.config/protbot/settings.toml}" \
  -e PROTBOT_TOKEN_PASS_NAME="${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}" \
  -- scripts/spawn.sh
```

`scripts/spawn.sh` is what Grok executes. If `PROTON_PASS_AGENT_TOKEN` is already set, spawn leaves it alone. Otherwise it reads the token with `pass show` and exports it for the child process only. The binary takes no arguments, so the token cannot be smuggled through argv.

To revoke access, the owner kills the agent token from the bot's Pass settings in a browser, or with `pass-cli pat revoke` if they hold a copy of the token.

## Security model

- The agent token lives in the process environment (`PROTON_PASS_AGENT_TOKEN`). Protbot copies it to `PROTON_PASS_PERSONAL_ACCESS_TOKEN` only for `pass-cli` children. Mail, Drive, and calendar fetches do not receive it.
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

A background watcher calls `proton-mail messages list` once per poll. The first poll records the current message ids and does not notify, so a restart does not replay the inbox.

After that:

- Mail from `sender_allowlist`, and mail whose subject or one-line summary contains an `importance_keywords` entry, is pushed immediately. The push is an MCP `notifications/message` with sender, subject, and a one-line summary.
- All other new mail is held and emitted as one batch when the summary cadence elapses, and only if something arrived. A quiet hour sends nothing and costs the model nothing.
- Message ids are remembered (capped at 10,000). An id reported on the immediate path is not reported again in the batch.

`mail_notifications` returns the same notices if the host did not surface the push. Headers, folder listings, and calendar events are cached for 30 seconds. Any successful write clears that cache. Listing mail is one `proton-mail` call, not one call per message.

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

## CLI contract

Binaries default to `proton-mail`, `pass-cli`, `proton-drive`, and `curl`. Override with `PROTBOT_MAIL_BIN`, `PROTBOT_PASS_BIN`, `PROTBOT_DRIVE_BIN`, and `PROTBOT_CURL_BIN`.

Mail uses `--json` and a single list call: `messages list --folder <name> --page-size 200`, `messages read <id> --format text`, `messages send --to <addr> --subject <s> --body -` (body on stdin), `messages trash|delete <id>`, `messages move <id> --dest <folder>`, `folders list`, `whoami`.

Pass uses `vault list`, `item list`, `item view`, `item create login --from-template -`, `item delete`, and `item totp`, with `--output json`. The agent token and reason are environment variables.

Drive uses `filesystem list|info|upload|download|trash`. `--json` is passed for list and info.

Calendar feeds are fetched with `curl --config -` so the URL is on stdin, and only `https://` URLs are accepted.

## Tests

```bash
cargo test
```

Unit tests cover allowlist checks, settings clamps, cache expiry and invalidation, path traversal, mail-list parsing, token redaction, and the immediate-versus-batch dedupe. Integration tests drive the tools with a fake CLI and speak stdio JSON-RPC to the binary, including a hung CLI that must be killed. GitHub Actions runs `cargo test` on every push.

## License

Protbot is open source software under the [MIT License](LICENSE).

The creator is not responsible for any issues you might have with it. You use this software at your own risk.
