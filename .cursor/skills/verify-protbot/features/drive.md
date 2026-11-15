# Drive

## Sub-features

- `drive_list` lists a folder. Argument: `path`, absolute. Child command, current proton-drive: `proton-drive filesystem list --json <path>`. Cached for 30 seconds.
- `drive_info` describes one path. Same path rule. Child command: `filesystem info --json <path>`. Cached for 30 seconds.
- `drive_download` copies a remote file into a directory. Arguments: `remote_path`, `dest_dir`. Both absolute. `dest_dir` must sit inside `PROTBOT_DOWNLOAD_DIR`. Child command: `filesystem download <remote> <dest>`.
- `drive_upload` uploads a local file. Arguments: `local_path`, `remote_path`. Requires `ALLOW_WRITES=true`. `local_path` must sit inside `PROTBOT_UPLOAD_DIR`. Child command: `filesystem upload <local> <remote>`.
- `drive_trash` trashes a path. Argument: `path`. Requires `ALLOW_WRITES=true`. Child command: `filesystem trash <path>`.

If `filesystem list --json <path>` or `filesystem info --json <path>` exits non-zero, protbot retries once as `--json filesystem <sub> <path>` and keeps the order that worked. Timeouts are not retried. Upload, download, and trash do not retry.

## How to get to it (user POV)

The host lists the bot's Drive at `/my-files`. That is `tools/call` named `drive_list` with `"path":"/my-files"`. In the stand-in, the folder contains a file named `notes.txt`. A path with `..` is not a folder the host can open. Protbot rejects it before it starts `proton-drive`.

Downloads land only under `.verify/data/download` for this launch (`PROTBOT_DOWNLOAD_DIR`). Uploads leave only from `.verify/data/upload`.

## Driving it with tmux

```bash
.cursor/skills/verify-protbot/scripts/drive.sh drive
```

Lines it types:

```json
{"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"drive_list","arguments":{"path":"/my-files"}}}
```

```json
{"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"drive_list","arguments":{"path":"/my-files/../secret"}}}
```

The upload call uses the absolute upload directory from this launch, `.verify/data/upload/note.txt`:

```json
{"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":"drive_upload","arguments":{"local_path":"/workspace/.verify/data/upload/note.txt","remote_path":"/my-files/note.txt"}}}
```

`drive.sh` builds `local_path` from `VERIFY_DIR` recorded in `.verify/state.env`, so a relocated `PROTBOT_VERIFY_DIR` still points at that launch's `data/upload/note.txt`. The file does not need to exist for the refusal below, because writes are checked first.

With the stand-in:

- The list text is `[{"name":"notes.txt","type":"file"}]`.
- `.verify/data/cli.log` contains `filesystem list --json /my-files`.
- The `..` call returns `path traversal rejected` and `isError` true. No new cli log line contains `..`.
- The upload call returns `writes are disabled; set ALLOW_WRITES=true`.
- Stderr contains `protbot tool=drive_list id=-`.

Evidence: `.verify/evidence/drive.json`, `drive-cli.log`, `drive-audit.txt`, `drive.txt`.

When `proton-drive` is on `PATH`, launch uses it. `drive.sh drive` then only checks that `drive_list` of `/my-files` returns JSON, and it still requires the `..` rejection. A live Drive listing will not be `notes.txt`.

A real upload needs `ALLOW_WRITES=true` and a file inside `PROTBOT_UPLOAD_DIR`. `drive.sh` will not upload while writes are on. Call `drive_upload` yourself. The stand-in prints `uploaded` and does not copy bytes. Live `proton-drive` performs the upload.

## Gotchas

- Paths must be absolute. `notes.txt` returns `path must be absolute`. A `..` component returns `path traversal rejected`. Backslash and NUL are rejected. `.` components are dropped.
- `drive_list` and `drive_info` are cached for 30 seconds under `drive:list:<path>` and `drive:info:<path>`. A successful write clears the whole cache.
- proton-drive 0.9.0 accepts `--json` only after the subcommand. Older builds want it before `filesystem`. The first success is remembered for the process.
- Download's destination has to be inside `PROTBOT_DOWNLOAD_DIR`. Upload's local file has to be inside `PROTBOT_UPLOAD_DIR`. This launch sets those to `.verify/data/download` and `.verify/data/upload`.
- The stand-in is used when `proton-drive` is not installed. Doctor prints `drive=stand-in`. That proves argument order and the traversal check. It does not prove a Proton Drive account.
- Drive children do not receive the Pass token. `run_cli` scrubs `PROTON_PASS_AGENT_TOKEN`, `PROTON_BRIDGE_PASSWORD`, and `PROTBOT_CALENDAR_ICS` before `exec`.
