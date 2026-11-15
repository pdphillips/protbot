#!/usr/bin/env bash
# Shared helpers for the protbot verification skill. Source this file.
# Do not execute it. It does not start or stop the server.

if [[ -z "${PROTBOT_VERIFY_LIB_LOADED:-}" ]]; then
  PROTBOT_VERIFY_LIB_LOADED=1

  PROTBOT_SKILL_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
  PROTBOT_REPO_ROOT=$(cd "$PROTBOT_SKILL_DIR/../../../.." && pwd)

  verify_paths() {
    PROTBOT_VERIFY_DIR=${PROTBOT_VERIFY_DIR:-$PROTBOT_REPO_ROOT/.verify}
    case "$PROTBOT_VERIFY_DIR" in
      /*) ;;
      *) PROTBOT_VERIFY_DIR=$PROTBOT_REPO_ROOT/$PROTBOT_VERIFY_DIR ;;
    esac
    PROTBOT_STATE_FILE=$PROTBOT_VERIFY_DIR/state.env
    PROTBOT_EVIDENCE_DIR=$PROTBOT_VERIFY_DIR/evidence
    PROTBOT_TMUX_SOCK=$PROTBOT_VERIFY_DIR/tmux.sock
    PROTBOT_TMUX_SESSION=${PROTBOT_TMUX_SESSION:-protbot-verify}
    PROTBOT_TMUX_BIN=${PROTBOT_TMUX_BIN:-tmux}
  }

  die() {
    printf 'protbot verify: %s\n' "$*" >&2
    exit 1
  }

  load_state() {
    verify_paths
    [[ -f "$PROTBOT_STATE_FILE" ]] || die "no state at $PROTBOT_STATE_FILE; run launch.sh first"
    # shellcheck disable=SC1090
    source "$PROTBOT_STATE_FILE"
    [[ -n "${TMUX_SOCK:-}" && -n "${TMUX_SESSION:-}" && -n "${STDOUT_LOG:-}" ]] || die "state file is incomplete"
  }

  write_state() {
    verify_paths
    mkdir -p "$PROTBOT_VERIFY_DIR"
    umask 077
    cat >"$PROTBOT_STATE_FILE" <<EOF
VERIFY_DIR=$(printf '%q' "$PROTBOT_VERIFY_DIR")
REPO_ROOT=$(printf '%q' "$PROTBOT_REPO_ROOT")
TMUX_SOCK=$(printf '%q' "$PROTBOT_TMUX_SOCK")
TMUX_SESSION=$(printf '%q' "$PROTBOT_TMUX_SESSION")
TMUX_BIN=$(printf '%q' "$PROTBOT_TMUX_BIN")
PROTBOT_PID=$(printf '%q' "${PROTBOT_PID:-}")
FIXTURE_PID=$(printf '%q' "${FIXTURE_PID:-}")
STARTED_FIXTURE=$(printf '%q' "${STARTED_FIXTURE:-0}")
IMAP_PORT=$(printf '%q' "${IMAP_PORT:-}")
SMTP_PORT=$(printf '%q' "${SMTP_PORT:-}")
MAIL_BACKEND=$(printf '%q' "${MAIL_BACKEND:-}")
PASS_BACKEND=$(printf '%q' "${PASS_BACKEND:-}")
DRIVE_BACKEND=$(printf '%q' "${DRIVE_BACKEND:-}")
ALLOW_WRITES=$(printf '%q' "${ALLOW_WRITES:-false}")
BRIDGE_USER=$(printf '%q' "${BRIDGE_USER:-}")
SETTINGS_PATH=$(printf '%q' "${SETTINGS_PATH:-}")
STDERR_LOG=$(printf '%q' "${STDERR_LOG:-}")
STDOUT_LOG=$(printf '%q' "${STDOUT_LOG:-}")
IMAP_LOG=$(printf '%q' "${IMAP_LOG:-}")
CLI_LOG=$(printf '%q' "${CLI_LOG:-}")
EVIDENCE_DIR=$(printf '%q' "$PROTBOT_EVIDENCE_DIR")
SESSION_PARENT=$(printf '%q' "${SESSION_PARENT:-}")
EOF
    chmod 600 "$PROTBOT_STATE_FILE"
  }

  tmux_cmd() {
    "${TMUX_BIN:-${PROTBOT_TMUX_BIN:-tmux}}" -S "${TMUX_SOCK:-$PROTBOT_TMUX_SOCK}" "$@"
  }

  session_up() {
    [[ -S "${TMUX_SOCK:-$PROTBOT_TMUX_SOCK}" ]] || return 1
    tmux_cmd has-session -t "${TMUX_SESSION:-$PROTBOT_TMUX_SESSION}" 2>/dev/null
  }

  alloc_id() {
    local file="${VERIFY_DIR:-$PROTBOT_VERIFY_DIR}/next-id"
    local id
    [[ -f "$file" ]] || printf '1\n' >"$file"
    id=$(tr -d '[:space:]' <"$file")
    [[ "$id" =~ ^[0-9]+$ ]] || die "bad next-id"
    printf '%s\n' "$((id + 1))" >"$file"
    printf '%s\n' "$id"
  }

  tmux_send_line() {
    local payload=$1
    tmux_cmd send-keys -t "$TMUX_SESSION" -l -- "$payload"
    sleep 0.2
    tmux_cmd send-keys -t "$TMUX_SESSION" Enter
  }

  # Print the JSON-RPC response line for this id. Reads only bytes appended
  # to the stdout log after the call, so earlier echoes cannot satisfy it.
  mcp_call() {
    local id=$1
    local payload=$2
    local timeout=${3:-20}
    local start deadline line
    [[ -f "$STDOUT_LOG" ]] || : >"$STDOUT_LOG"
    start=$(wc -c <"$STDOUT_LOG" | tr -d '[:space:]')
    tmux_send_line "$payload"
    deadline=$((SECONDS + timeout))
    while (( SECONDS < deadline )); do
      line=$(tail -c +$((start + 1)) "$STDOUT_LOG" 2>/dev/null | tr -d '\r' | grep -F "\"id\":${id}," | grep -E '"result"|"error"' | tail -n 1 || true)
      if [[ -n "$line" ]]; then
        printf '%s\n' "$line"
        return 0
      fi
      sleep 0.1
    done
    printf 'protbot verify: no JSON-RPC response for id %s\n' "$id" >&2
    printf 'protbot verify: stdout log appended:\n' >&2
    tail -c +$((start + 1)) "$STDOUT_LOG" >&2 || true
    printf 'protbot verify: pane:\n' >&2
    tmux_cmd capture-pane -t "$TMUX_SESSION" -p -J -S -40 >&2 || true
    return 1
  }

  # Print the tool text. Exit 1 when the response is an error or isError.
  tool_text() {
    local line=$1
    printf '%s\n' "$line" | python3 -c '
import json, sys
raw = sys.stdin.read().strip()
msg = json.loads(raw)
if "error" in msg:
    sys.stderr.write("jsonrpc error: " + json.dumps(msg["error"]) + "\n")
    sys.exit(1)
result = msg["result"]
if result.get("isError"):
    text = result["content"][0]["text"]
    sys.stderr.write("tool error: " + text + "\n")
    sys.exit(1)
sys.stdout.write(result["content"][0]["text"])
'
  }

  tool_error_text() {
    local line=$1
    printf '%s\n' "$line" | python3 -c '
import json, sys
msg = json.loads(sys.stdin.read().strip())
if "error" in msg:
    sys.stdout.write(msg["error"].get("message", ""))
    sys.exit(0)
result = msg["result"]
if not result.get("isError"):
    sys.stderr.write("expected isError\n")
    sys.exit(1)
sys.stdout.write(result["content"][0]["text"])
'
  }

  call_tool() {
    local name=$1
    local args=$2
    local id
    id=$(alloc_id)
    mcp_call "$id" "{\"jsonrpc\":\"2.0\",\"id\":${id},\"method\":\"tools/call\",\"params\":{\"name\":\"${name}\",\"arguments\":${args}}}"
  }

  # Fail if any file contains the live token or bridge password.
  # Reads those values from the server process environment and does not print them.
  assert_no_secrets() {
    local pid=${PROTBOT_PID:-}
    [[ -n "$pid" && -r "/proc/${pid}/environ" ]] || die "cannot read server environ to scan evidence"
    python3 - "$pid" "$@" <<'PY'
import sys
pid = sys.argv[1]
paths = sys.argv[2:]
env = open(f"/proc/{pid}/environ", "rb").read().split(b"\0")
secrets = []
for item in env:
    for key in (b"PROTON_PASS_AGENT_TOKEN=", b"PROTON_BRIDGE_PASSWORD="):
        if item.startswith(key):
            value = item[len(key):]
            if len(value) >= 4:
                secrets.append(value)
if not secrets:
    sys.stderr.write("protbot verify: server environ has no token or bridge password\n")
    sys.exit(1)
for path in paths:
    blob = open(path, "rb").read()
    for secret in secrets:
        if secret in blob:
            sys.stderr.write(f"protbot verify: credential leaked into {path}\n")
            sys.exit(1)
PY
  }

  cmdline_of() {
    local pid=$1
    tr '\0' ' ' <"/proc/${pid}/cmdline" 2>/dev/null || true
  }

  # tmux's pane pid is often a shell. The protbot binary is that pid or a child.
  # Match target/debug/protbot or target/release/protbot, not the verify-protbot path.
  find_protbot_pid() {
    local session=${TMUX_SESSION:-$PROTBOT_TMUX_SESSION}
    local root pid cmd child
    root=$(tmux_cmd list-panes -t "$session" -F '#{pane_pid}' 2>/dev/null || true)
    [[ -n "$root" ]] || return 1
    local -a layer=("$root")
    local -a next=()
    local round
    for round in 1 2 3 4; do
      next=()
      for pid in "${layer[@]}"; do
        [[ -n "$pid" ]] || continue
        cmd=$(cmdline_of "$pid")
        if [[ "$cmd" == *"/debug/protbot"* || "$cmd" == *"/release/protbot"* ]]; then
          printf '%s\n' "$pid"
          return 0
        fi
        while read -r child; do
          child=${child//[[:space:]]/}
          [[ -n "$child" ]] && next+=("$child")
        done < <(ps -o pid= --ppid "$pid" 2>/dev/null || true)
      done
      [[ ${#next[@]} -gt 0 ]] || return 1
      layer=("${next[@]}")
    done
    return 1
  }
fi
