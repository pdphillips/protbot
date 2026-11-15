#!/usr/bin/env bash
# Read-only check: is the running protbot worth driving?
# Sends MCP ping. Does not call tools/call and does not write settings.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
load_state

reasons=()
note() {
  reasons+=("$1")
}

if ! session_up; then
  note "tmux session $TMUX_SESSION is not on $TMUX_SOCK"
fi

if [[ -z "${PROTBOT_PID:-}" ]] || ! kill -0 "$PROTBOT_PID" 2>/dev/null; then
  note "protbot pid ${PROTBOT_PID:-missing} is not running"
else
  cmd=$(cmdline_of "$PROTBOT_PID")
  cmd=${cmd%" "}
  if [[ "$cmd" != *"/debug/protbot" && "$cmd" != *"/release/protbot" ]]; then
    note "pid $PROTBOT_PID command is not the protbot binary"
  fi
fi

if [[ ! -f "${STDERR_LOG:-}" ]] || ! grep -q 'protbot ready' "$STDERR_LOG"; then
  note "stderr has no 'protbot ready' line"
fi

if [[ ! -f "${SETTINGS_PATH:-}" ]]; then
  note "settings file is missing"
elif ! python3 - "$SETTINGS_PATH" <<'PY'
import os, re, stat, sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
mode = stat.S_IMODE(os.stat(path).st_mode)
if mode != 0o600:
    raise SystemExit(f"mode {mode:o}")
match = re.search(r"(?m)^poll_interval_secs\s*=\s*(\d+)\s*$", text)
if not match:
    raise SystemExit("no poll_interval_secs")
value = int(match.group(1))
if not 30 <= value <= 86400:
    raise SystemExit(f"poll {value}")
if "http://" in text or "https://" in text:
    raise SystemExit("url in settings")
PY
then
  note "settings.toml is missing, the wrong mode, out of clamp, or contains a URL"
fi

if [[ -z "${IMAP_PORT:-}" ]]; then
  note "IMAP port is not recorded"
elif ! python3 -c "import socket,sys; s=socket.create_connection(('127.0.0.1', int(sys.argv[1])), 2); data=s.recv(128); s.close(); sys.exit(0 if b'OK' in data.upper() else 1)" "$IMAP_PORT"; then
  note "IMAP 127.0.0.1:$IMAP_PORT did not greet with OK"
fi

if [[ ${#reasons[@]} -eq 0 ]]; then
  ping_id=$(alloc_id)
  if ! ping_line=$(mcp_call "$ping_id" "{\"jsonrpc\":\"2.0\",\"id\":${ping_id},\"method\":\"ping\"}" 8); then
    note "ping got no response"
  elif ! printf '%s\n' "$ping_line" | python3 -c 'import json,sys; msg=json.loads(sys.stdin.read()); assert msg.get("result") == {}'; then
    note "ping did not return an empty result"
  fi
fi

if [[ ${#reasons[@]} -eq 0 ]]; then
  if ! assert_no_secrets "$STDERR_LOG" "$STDOUT_LOG" "$SETTINGS_PATH"; then
    note "stderr, stdout, or settings contains the token or bridge password"
  fi
fi

if [[ ${#reasons[@]} -eq 0 ]]; then
  printf 'protbot doctor: worth driving\n'
  printf 'pid=%s session=%s mail=%s imap=127.0.0.1:%s pass=%s drive=%s writes=%s settings=%s\n' \
    "$PROTBOT_PID" "$TMUX_SESSION" "$MAIL_BACKEND" "$IMAP_PORT" "$PASS_BACKEND" "$DRIVE_BACKEND" "$ALLOW_WRITES" "$SETTINGS_PATH"
  exit 0
fi

printf 'protbot doctor: not worth driving\n'
for reason in "${reasons[@]}"; do
  printf 'reason: %s\n' "$reason"
done
exit 1
