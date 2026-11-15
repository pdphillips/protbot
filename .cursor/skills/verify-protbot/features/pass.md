# Pass

## Sub-features

- `pass_vaults` lists vaults the agent token can see. No arguments. The child command is `pass-cli vault list --output json`.
- `pass_items` lists items in one vault. Argument: `vault`. The child command is `item list --vault-name <vault> --output json`.
- `pass_item_view` views one item. Arguments: `vault`, and exactly one of `item_id` or `item_title`, plus optional `field`. The child command is `item view --vault-name <vault> --output json` with `--item-id` or `--item-title`.
- `pass_totp` reads a TOTP code. Arguments: `vault`, and exactly one of `item_id` or `item_title`. The child command is `item totp` with the same selectors.
- `pass_item_create_login` creates a login. Arguments: `vault`, `title`, `password`, optional `username`, `email`, `url`. Requires `ALLOW_WRITES=true`. The password goes to `item create login --from-template -` on stdin, not argv.
- `pass_item_delete` deletes by `share_id` and `item_id`. Requires `ALLOW_WRITES=true`. The child command is `item delete --share-id <share> --item-id <item>`.

Before the first Pass tool, protbot runs `info --output json`. If that exits non-zero (and did not time out) it runs `login`, then retries later calls. Each call sets `PROTON_PASS_PERSONAL_ACCESS_TOKEN` from `PROTON_PASS_AGENT_TOKEN` and sets `PROTON_PASS_AGENT_REASON` to `protbot <tool>` or `protbot <tool> item <id>`. The token is removed from the child environment's other secret variables first. `PROTON_PASS_SESSION_DIR` is `$XDG_RUNTIME_DIR/protbot-<pid>`.

## How to get to it (user POV)

The host asks which vaults the bot's Proton Pass token can see. That is `tools/call` named `pass_vaults` with `{}`. In the stand-in, the vault is `Bot` with id `v1`. Opening that vault is `pass_items` with `"vault":"Bot"`. The stand-in item is id `item1`, title `Example`.

Creating a login is a separate, writes-on step. The host passes the password in the tool arguments. Protbot forwards it on the child's stdin. The host should not see the token in stderr or in the cli log.

## Driving it with tmux

```bash
.cursor/skills/verify-protbot/scripts/drive.sh pass
```

Lines it types (ids are allocated; samples start at 21):

```json
{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"pass_vaults","arguments":{}}}
```

```json
{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"pass_items","arguments":{"vault":"Bot"}}}
```

```json
{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"pass_item_create_login","arguments":{"vault":"Bot","title":"Verify login","password":"fixture-secret"}}}
```

With the stand-in (`pass=stand-in` from launch or doctor):

- `pass_vaults` text is `{"vaults":[{"id":"v1","name":"Bot"}]}`.
- `pass_items` text is `{"items":[{"id":"item1","title":"Example"}]}`.
- `.verify/data/cli.log` contains `info --output json`, `reason=protbot pass_session`, `vault list --output json`, `reason=protbot pass_vaults`, and `item list --vault-name Bot --output json`.
- The create call returns `writes are disabled; set ALLOW_WRITES=true`. `.verify/data/pass/last-create.json` must not exist, and `fixture-secret` must not appear in `cli.log`.
- Stderr contains `protbot tool=pass_vaults id=-`.

Evidence files: `.verify/evidence/pass.json`, `pass-cli.log`, `pass-audit.txt`, `pass.txt`.

When `pass-cli` is on `PATH`, launch uses it and `drive.sh pass` only checks that `pass_vaults` returns JSON. It does not require vault id `v1`. A live token can list real vault names. Do not copy item secrets into the PR.

A writes-on create is not what `drive.sh` does. Launch with `ALLOW_WRITES=true` and call `pass_item_create_login` yourself. The stand-in writes the stdin template to `.verify/data/pass/last-create.json` mode `0600`. Confirm the title is in that file and the cli log line is `item create login --vault-name Bot --from-template -` with no password on that line.

## Gotchas

- Provide `item_id` or `item_title`, not both and not neither. `pass_item_view` may also take `field`.
- Labels reject empty strings, control characters, `..`, and lengths over 200. Item ids reject `/`, `\`, and `..`.
- `email`, when set, must be an address. `url`, when set, must be `https://` (`validate_ics_url`).
- The password must be non-empty, at most 1024 bytes, and must not contain NUL.
- The stand-in exits 3 if any argument contains `pst_`, `PROTON_PASS_`, or `PROTON_BRIDGE_PASSWORD`. That is the same rule as the unit fixture: the token stays in the environment.
- `pass-cli` is skipped when it is not installed. Doctor prints `pass=stand-in` in that case. A stand-in success proves protbot's argv, reason string, and JSON handling. It does not prove a Proton Pass account.
- Session files live under the runtime dir and are removed when the process drops `Config`. A `SIGKILL` skips that drop. `cleanup.sh` sends `C-d` first so the drop can run.
