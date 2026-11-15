# Settings

## Sub-features

- `whoami` reports the mailbox signed in to Bridge. No arguments. It opens IMAP, logs in, logs out, and returns `{"address":"<PROTBOT_BRIDGE_USER>","bridge":"127.0.0.1"}`. It does not list messages.
- `settings_get` returns the clamped settings, `writes_enabled`, `recipient_allowlist`, `pass_token_present`, and `bridge_password_present`. No arguments. It reloads the file only when `settings_reload_secs` has elapsed since the last check.
- `settings_update` patches `settings.toml` and returns the clamped settings. Any subset of `poll_interval_secs`, `summary_cadence_secs`, `settings_reload_secs`, `sender_allowlist`, `importance_keywords`, and `calendar_feeds`. Unknown fields are rejected.

Clamps, from `src/settings.rs`:

| Key | Floor | Ceiling | Seed |
| --- | --- | --- | --- |
| `poll_interval_secs` | 30 | 86400 | 300 |
| `summary_cadence_secs` | 300 | 604800 | 3600 |
| `settings_reload_secs` | 30 | 3600 | 180 |

`sender_allowlist` holds up to 64 addresses or `@domain` entries. `importance_keywords` holds up to 32 strings, each trimmed and cut to 64 characters. `calendar_feeds` holds up to 16 `vault/item` refs.

## How to get to it (user POV)

The host asks which mailbox this server is. That is `tools/call` named `whoami` with `{}`. On the fixture the address is `bot@proton.me` and the bridge host is `127.0.0.1`.

The host then reads the poll interval with `settings_get`. The fresh file says 300 seconds. Changing it is `settings_update`. A request for a 1 second poll is stored as 30, because the floor is 30. Adding the keyword `otp` makes later inbox mail whose subject or summary contains `otp` notify immediately instead of waiting for the hourly batch.

## Driving it with tmux

```bash
.cursor/skills/verify-protbot/scripts/drive.sh settings
```

Lines it types:

```json
{"jsonrpc":"2.0","id":51,"method":"tools/call","params":{"name":"whoami","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":52,"method":"tools/call","params":{"name":"settings_get","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":53,"method":"tools/call","params":{"name":"settings_update","arguments":{"poll_interval_secs":1,"importance_keywords":["otp"]}}}
```

Checks:

- `whoami` text has `"bridge":"127.0.0.1"` and, in fixture mode, `"address":"bot@proton.me"`.
- `settings_get` is `isError` false. `writes_enabled` is false on the default launch. `pass_token_present` and `bridge_password_present` are true. The token value is not in the text.
- `settings_update` text has `"poll_interval_secs":30` and `"importance_keywords":["otp"]`.
- `.verify/data/settings.toml` contains the lines `poll_interval_secs = 30` and the keyword `otp`, and it stays mode `0600`.
- The backend log gains another `IMAP LOGIN ok` from `whoami`, not a `FETCH`, unless the watcher polls at the same time.

Evidence: `.verify/evidence/settings.json`, `settings.toml`, `settings.txt`. The copy of the settings file is the side effect. Cleanup does not delete it.

`doctor.sh` still passes after this drive. It accepts any poll interval inside `30..=86400`. It does not require the seed value 300. The seed check belongs to `launch.sh`.

## Gotchas

- A value below the floor is raised when the file is read and when a patch is applied. `poll_interval_secs = 1` becomes 30. The tool cannot set a few-second poll.
- Hand-editing `settings.toml` is not visible until `settings_reload_secs` has passed (180 seconds at the seed, 30 after you lower it). `settings_update` applies in memory immediately and writes the file with a `settings.toml.tmp` rename.
- The file must not contain secrets or feed URLs. Doctor fails a settings file that contains `http://` or `https://`.
- `sender_allowlist` entries are normalized to lowercase. `Owner@Proton.Me` is stored as `owner@proton.me`.
- `importance_keywords` matching is case-insensitive against subject plus summary. The watcher uses the in-memory settings on its next poll, so a keyword written now does not classify mail that was already baselined.
- `whoami` proves the IMAP login, not the Pass token. `settings_get`'s `pass_token_present` flag is the token check, and it is a boolean.
- `ALLOW_WRITES` is not in the settings file. It is fixed for the life of the process. Changing it means cleanup and a new launch.
