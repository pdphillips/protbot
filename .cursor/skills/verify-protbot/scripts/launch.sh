#!/usr/bin/env bash
# Start protbot from an empty data directory and initialize the MCP session.
set -euo pipefail
umask 077

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
verify_paths

if [[ -S "$PROTBOT_TMUX_SOCK" ]]; then
  if tmux_cmd has-session -t "$PROTBOT_TMUX_SESSION" 2>/dev/null; then
    die "session $PROTBOT_TMUX_SESSION is already running; run scripts/cleanup.sh first"
  fi
  tmux_cmd kill-server >/dev/null 2>&1 || true
  rm -f "$PROTBOT_TMUX_SOCK"
fi

rm -rf "$PROTBOT_VERIFY_DIR/data"
mkdir -p \
  "$PROTBOT_VERIFY_DIR/data/runtime" \
  "$PROTBOT_VERIFY_DIR/data/upload" \
  "$PROTBOT_VERIFY_DIR/data/download" \
  "$PROTBOT_EVIDENCE_DIR"

cargo build --manifest-path "$PROTBOT_REPO_ROOT/Cargo.toml" --bin protbot
BIN=$PROTBOT_REPO_ROOT/target/debug/protbot
[[ -x "$BIN" ]] || die "missing $BIN"

DATA=$PROTBOT_VERIFY_DIR/data
SETTINGS_PATH=$DATA/settings.toml
STDERR_LOG=$DATA/stderr.log
STDOUT_LOG=$DATA/stdout.log
IMAP_LOG=$DATA/backend.log
CLI_LOG=$DATA/cli.log
PASS_DIR=$DATA/pass
SMTP_COPY=$DATA/smtp-last.bin
READY=$DATA/fixture.ready
: >"$STDERR_LOG"
: >"$STDOUT_LOG"
: >"$IMAP_LOG"
: >"$CLI_LOG"

if [[ ! -f "$SETTINGS_PATH" ]]; then
  :
else
  die "refusing to launch; $SETTINGS_PATH already exists"
fi

MAIL_BACKEND=${PROTBOT_VERIFY_MAIL:-fixture}
case "$MAIL_BACKEND" in
  fixture|bridge) ;;
  *) die "PROTBOT_VERIFY_MAIL must be fixture or bridge" ;;
esac

FIXTURE_PID=
STARTED_FIXTURE=0
if [[ "$MAIL_BACKEND" == "bridge" ]]; then
  [[ -n "${PROTON_PASS_AGENT_TOKEN:-}" ]] || die "PROTON_PASS_AGENT_TOKEN is unset. Store it in pass and export it, or leave PROTBOT_VERIFY_MAIL=fixture."
  [[ -n "${PROTON_BRIDGE_PASSWORD:-}" ]] || die "PROTON_BRIDGE_PASSWORD is unset. Store it in pass and export it, or leave PROTBOT_VERIFY_MAIL=fixture."
  [[ -n "${PROTBOT_BRIDGE_USER:-}" ]] || die "PROTBOT_BRIDGE_USER is unset. Set it to the mailbox address Bridge shows."
  IMAP_PORT=${PROTBOT_IMAP_PORT:-1143}
  SMTP_PORT=${PROTBOT_SMTP_PORT:-1025}
  python3 -c "import socket; s=socket.create_connection(('127.0.0.1', int('${IMAP_PORT}')), 2); data=s.recv(64); s.close(); assert b'OK' in data.upper()" \
    || die "nothing on 127.0.0.1:${IMAP_PORT} greeted as IMAP. Sign Proton Mail Bridge in, or use the fixture."
else
  export PROTON_PASS_AGENT_TOKEN="${PROTON_PASS_AGENT_TOKEN:-pst_verify_fixture::NOTREAL}"
  export PROTON_BRIDGE_PASSWORD="${PROTON_BRIDGE_PASSWORD:-bridge-fixture}"
  export PROTBOT_BRIDGE_USER="${PROTBOT_BRIDGE_USER:-bot@proton.me}"
  python3 "$PROTBOT_SKILL_DIR/imap-fixture.py" \
    --imap-port 0 \
    --smtp-port 0 \
    --ready-file "$READY" \
    --log "$IMAP_LOG" \
    --smtp-copy "$SMTP_COPY" &
  FIXTURE_PID=$!
  STARTED_FIXTURE=1
  PROTBOT_PID=
  PASS_BACKEND=
  DRIVE_BACKEND=
  ALLOW_WRITES=${ALLOW_WRITES:-false}
  BRIDGE_USER=$PROTBOT_BRIDGE_USER
  IMAP_PORT=
  SMTP_PORT=
  write_state
  trap 'status=$?; if [[ $status -ne 0 ]]; then "'"$PROTBOT_SKILL_DIR"'/cleanup.sh" || true; fi; exit $status' EXIT
  ready=0
  for _ in $(seq 1 50); do
    if [[ -s "$READY" ]]; then
      ready=1
      break
    fi
    if ! kill -0 "$FIXTURE_PID" 2>/dev/null; then
      die "imap fixture exited before it was ready"
    fi
    sleep 0.1
  done
  [[ "$ready" == 1 ]] || die "imap fixture did not write $READY"
  # shellcheck disable=SC1090
  source "$READY"
  IMAP_PORT=$imap_port
  SMTP_PORT=$smtp_port
fi

BRIDGE_USER=$PROTBOT_BRIDGE_USER

if [[ -n "${PROTBOT_PASS_BIN:-}" ]]; then
  PASS_BIN=$PROTBOT_PASS_BIN
  PASS_BACKEND=custom
elif command -v pass-cli >/dev/null 2>&1; then
  PASS_BIN=$(command -v pass-cli)
  PASS_BACKEND=pass-cli
else
  PASS_BIN=$PROTBOT_SKILL_DIR/proton-stand-in.sh
  PASS_BACKEND=stand-in
fi
if [[ -n "${PROTBOT_DRIVE_BIN:-}" ]]; then
  DRIVE_BIN=$PROTBOT_DRIVE_BIN
  DRIVE_BACKEND=custom
elif command -v proton-drive >/dev/null 2>&1; then
  DRIVE_BIN=$(command -v proton-drive)
  DRIVE_BACKEND=proton-drive
else
  DRIVE_BIN=$PROTBOT_SKILL_DIR/proton-stand-in.sh
  DRIVE_BACKEND=stand-in
fi
[[ -x "$PASS_BIN" ]] || die "pass binary is not executable: $PASS_BIN"
[[ -x "$DRIVE_BIN" ]] || die "drive binary is not executable: $DRIVE_BIN"

ALLOW_WRITES=${ALLOW_WRITES:-false}

export PROTBOT_BIN=$BIN
export PROTBOT_STDERR_LOG=$STDERR_LOG
export PROTBOT_BRIDGE_HOST=127.0.0.1
export PROTBOT_IMAP_PORT=$IMAP_PORT
export PROTBOT_SMTP_PORT=$SMTP_PORT
export PROTBOT_SETTINGS=$SETTINGS_PATH
export XDG_RUNTIME_DIR=$DATA/runtime
export PROTBOT_UPLOAD_DIR=$DATA/upload
export PROTBOT_DOWNLOAD_DIR=$DATA/download
export PROTBOT_PASS_BIN=$PASS_BIN
export PROTBOT_DRIVE_BIN=$DRIVE_BIN
export PROTBOT_VERIFY_CLI_LOG=$CLI_LOG
export PROTBOT_VERIFY_PASS_DIR=$PASS_DIR
export ALLOW_WRITES
unset PROTBOT_DISABLE_WATCHER || true
unset PROTBOT_BRIDGE_PROBE || true
unset PROTBOT_CALENDAR_ICS || true

PROTBOT_PID=
write_state
printf '1\n' >"$PROTBOT_VERIFY_DIR/next-id"

trap 'status=$?; if [[ $status -ne 0 ]]; then "'"$PROTBOT_SKILL_DIR"'/cleanup.sh" || true; fi; exit $status' EXIT

# new-session is the first tmux command, so the server inherits this environment
# (token and bridge password included). They are not written to disk.
# A bare start-server exits immediately because this tmux has exit-empty on.
tmux_cmd new-session -d -s "$PROTBOT_TMUX_SESSION" -x 2000 -y 40 -- "$PROTBOT_SKILL_DIR/pane.sh"
tmux_cmd set-option -t "$PROTBOT_TMUX_SESSION" remain-on-exit on
tmux_cmd set-option -t "$PROTBOT_TMUX_SESSION" history-limit 5000
tmux_cmd pipe-pane -t "$PROTBOT_TMUX_SESSION" -o "python3 -u '$PROTBOT_SKILL_DIR/capture.py' '$STDOUT_LOG'"

found=0
for _ in $(seq 1 50); do
  if PROTBOT_PID=$(find_protbot_pid); then
    found=1
    break
  fi
  sleep 0.1
done
[[ "$found" == 1 ]] || die "protbot did not replace the pane. stderr: $(cat "$STDERR_LOG")"
write_state

ready=0
for _ in $(seq 1 150); do
  if grep -q 'protbot ready' "$STDERR_LOG"; then
    ready=1
    break
  fi
  if ! kill -0 "$PROTBOT_PID" 2>/dev/null; then
    die "protbot exited before it was ready. stderr: $(cat "$STDERR_LOG")"
  fi
  sleep 0.1
done
[[ "$ready" == 1 ]] || die "timed out waiting for 'protbot ready'. stderr: $(cat "$STDERR_LOG")"

python3 - "$SETTINGS_PATH" <<'PY'
import os, stat, sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
mode = stat.S_IMODE(os.stat(path).st_mode)
if mode != 0o600:
    raise SystemExit(f"settings mode is {mode:o}, expected 600")
for needle in (
    "poll_interval_secs = 300",
    "summary_cadence_secs = 3600",
    "settings_reload_secs = 180",
    "sender_allowlist = []",
    "importance_keywords = []",
    "calendar_feeds = []",
):
    if needle not in text:
        raise SystemExit(f"seed missing {needle!r}:\n{text}")
PY

load_state
init_id=$(alloc_id)
init_line=$(mcp_call "$init_id" "{\"jsonrpc\":\"2.0\",\"id\":${init_id},\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"verify-protbot\",\"version\":\"0\"}}}")
printf '%s\n' "$init_line" | python3 -c '
import json, sys
msg = json.loads(sys.stdin.read())
info = msg["result"]["serverInfo"]
if info.get("name") != "protbot" or info.get("version") != "0.1.0":
    raise SystemExit(f"unexpected serverInfo: {info}")
'
tmux_send_line '{"jsonrpc":"2.0","method":"notifications/initialized"}'
assert_no_secrets "$STDERR_LOG" "$STDOUT_LOG" "$IMAP_LOG" "$CLI_LOG" "$SETTINGS_PATH"

printf 'protbot verify: ready\n'
printf 'protbot verify: mail=%s imap=127.0.0.1:%s smtp=127.0.0.1:%s pass=%s drive=%s writes=%s\n' \
  "$MAIL_BACKEND" "$IMAP_PORT" "$SMTP_PORT" "$PASS_BACKEND" "$DRIVE_BACKEND" "$ALLOW_WRITES"
printf 'protbot verify: settings=%s\n' "$SETTINGS_PATH"
printf 'protbot verify: evidence=%s\n' "$EVIDENCE_DIR"
printf 'protbot verify: next: .cursor/skills/verify-protbot/scripts/doctor.sh\n'
