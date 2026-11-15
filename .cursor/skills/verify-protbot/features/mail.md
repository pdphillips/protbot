# Mail

## Sub-features

- `mail_list` lists headers for a folder. Arguments: `folder` (default `inbox`), `unread_only` (default false). Cached for 30 seconds.
- `mail_read` reads one message. Argument: `id`, shaped `folder:uid` (the fixture message is `inbox:7`).
- `mail_folders` lists mailbox names. No arguments. Cached for 30 seconds.
- `mail_notifications` returns watcher notices. Arguments: `clear` (default false), `limit` (default 20, clamped to `1..=100`).
- `mail_send` sends a message. Arguments: `to`, `subject`, `body`, optional `cc` and `bcc`. Requires `ALLOW_WRITES=true` and `PROTBOT_RECIPIENT_ALLOWLIST`.
- `mail_trash`, `mail_delete`, and `mail_move` change a message. `mail_move` also takes `dest`. All three require `ALLOW_WRITES=true`.

## How to get to it (user POV)

The host already has protbot on stdin after `initialize` and `notifications/initialized`. Listing the inbox is a `tools/call` named `mail_list` with `"folder":"inbox"`. The mailbox is `PROTBOT_BRIDGE_USER` (`bot@proton.me` in the fixture). The message the owner sent the bot is the seeded unread mail: from `owner@proton.me`, subject `Verify mailbox`. Opening it is `mail_read` with `"id":"inbox:7"`. The body the host should see is `hello from the loopback mailbox`.

A restart must not replay the inbox. The watcher records current ids on its first poll and emits nothing. `mail_notifications` on a fresh server is `[]`.

## Driving it with tmux

```bash
.cursor/skills/verify-protbot/scripts/drive.sh mail
```

That types these lines into session `protbot-verify` on socket `.verify/tmux.sock`. Ids increase from `.verify/next-id`; the samples use `11` and up.

```json
{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"mail_list","arguments":{"folder":"inbox","unread_only":false}}}
```

```json
{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"mail_folders","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"mail_notifications","arguments":{"limit":20}}}
```

```json
{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"mail_read","arguments":{"id":"inbox:7"}}}
```

```json
{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"mail_send","arguments":{"to":["owner@proton.me"],"subject":"verify","body":"nope"}}}
```

Read the matching `"id"` line from `.verify/data/stdout.log`. `result.isError` is false for the reads. The `mail_list` text is a JSON array containing `{"id":"inbox:7","from":"owner@proton.me","subject":"Verify mailbox","summary":"Verify mailbox","unread":true}`. The `mail_folders` text contains `INBOX` and `Sent`. The `mail_read` text contains `hello from the loopback mailbox`. The `mail_send` text is `writes are disabled; set ALLOW_WRITES=true` and `isError` is true.

Side effects to check, not just the JSON:

- `.verify/data/backend.log` gains `IMAP UID FETCH 7 (BODY.PEEK[])` from `mail_read`. Header polls log `UID FETCH 1:*` instead, and that line does not contain `BODY.PEEK[]`.
- `.verify/data/stderr.log` gains `protbot tool=mail_list id=-` and `protbot tool=mail_read id=inbox7`. The audit sanitizer drops the colon in `inbox:7`.
- `.verify/data/backend.log` does not gain `SMTP DATA`. The refused send never opens SMTP.

`drive.sh` copies those into `.verify/evidence/mail.json`, `mail-backend.log`, `mail-audit.txt`, and `mail.txt`.

To exercise a real send, stop the server, launch again with `ALLOW_WRITES=true` and `PROTBOT_RECIPIENT_ALLOWLIST=owner@proton.me`, and send the `mail_send` call yourself. Do not use `drive.sh mail` for that: it refuses to send while writes are on. On the fixture, success is `{"ok":true}` in the tool text and `SMTP DATA-END` in `backend.log`, and `smtp-last.bin` holds the message. Against `PROTBOT_VERIFY_MAIL=bridge` that call delivers to the real allowlisted recipient.

## Gotchas

- The watcher polls `inbox` immediately, then waits `poll_interval_secs` (minimum 30). A `mail_list` within 30 seconds can be the cache entry from that poll. `mail_read` is not cached. Judge a live IMAP read by `BODY.PEEK[]` in the backend log, not by a second `mail_list`.
- `mail_notifications` stays `[]` until a uid appears that was not in the baseline. The fixture never adds a second uid, so a quiet run stays empty. That is the restart behavior, not a stuck watcher.
- Folder names allow ASCII letters, digits, space, `_`, `.`, `/`, and `-`, up to 128 characters, and reject `..`. Ids allow the same idea plus `:`, `@`, `+`, and `=`, and reject `/` and `..`.
- `unread` is the absence of the IMAP `\Seen` flag. The fixture header fetch sends `FLAGS ()`.
- Writes are off unless `ALLOW_WRITES` is `1`, `true`, or `yes`. An empty recipient allowlist rejects every send even when writes are on. Allowlist entries are exact addresses or `@domain`.
- Subjects reject CR, LF, and NUL, and bodies over 200000 bytes. Unknown JSON fields are rejected (`additionalProperties` is false).
- The Bridge host must be `127.0.0.1`, `localhost`, or `::1`. The fixture advertises no `STARTTLS`, so the client stays on the plaintext socket. A real Bridge that advertises `STARTTLS` is upgraded inside `src/bridge.rs`.
- Stderr is only `protbot tool=<name> id=<id>`. Message bodies and the bridge password must not appear there or in the evidence files.
