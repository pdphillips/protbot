#!/usr/bin/env bash
# Stop the protbot pane and the fixture this skill started.
# Does not delete .verify/evidence or .verify/data.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
verify_paths

if [[ ! -f "$PROTBOT_STATE_FILE" ]]; then
  if [[ -S "$PROTBOT_TMUX_SOCK" ]] && tmux_cmd has-session -t "$PROTBOT_TMUX_SESSION" 2>/dev/null; then
    die "session is up but $PROTBOT_STATE_FILE is missing; not guessing which pids to kill"
  fi
  printf 'protbot verify: nothing to stop\n'
  exit 0
fi

load_state

if session_up; then
  tmux_cmd send-keys -t "$TMUX_SESSION" C-d >/dev/null 2>&1 || true
  if [[ -n "${PROTBOT_PID:-}" ]]; then
    for _ in $(seq 1 30); do
      kill -0 "$PROTBOT_PID" 2>/dev/null || break
      sleep 0.1
    done
  fi
  tmux_cmd kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
fi
if [[ -S "$TMUX_SOCK" ]]; then
  tmux_cmd kill-server >/dev/null 2>&1 || true
  rm -f "$TMUX_SOCK"
fi

if [[ -n "${PROTBOT_PID:-}" ]] && kill -0 "$PROTBOT_PID" 2>/dev/null; then
  cmd=$(cmdline_of "$PROTBOT_PID")
  if [[ "$cmd" == *"/protbot" ]]; then
    kill "$PROTBOT_PID" 2>/dev/null || true
    sleep 0.3
    kill -9 "$PROTBOT_PID" 2>/dev/null || true
  else
    die "refusing to kill pid $PROTBOT_PID; command does not look like protbot"
  fi
fi

if [[ "${STARTED_FIXTURE:-0}" == "1" && -n "${FIXTURE_PID:-}" ]] && kill -0 "$FIXTURE_PID" 2>/dev/null; then
  cmd=$(cmdline_of "$FIXTURE_PID")
  if [[ "$cmd" == *imap-fixture.py* ]]; then
    kill "$FIXTURE_PID" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$FIXTURE_PID" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "$FIXTURE_PID" 2>/dev/null; then
      kill -9 "$FIXTURE_PID" 2>/dev/null || true
    fi
  else
    die "refusing to kill pid $FIXTURE_PID; command is not imap-fixture.py"
  fi
fi

if [[ -n "${PROTBOT_PID:-}" ]] && kill -0 "$PROTBOT_PID" 2>/dev/null; then
  die "protbot pid $PROTBOT_PID is still running"
fi
if [[ "${STARTED_FIXTURE:-0}" == "1" && -n "${FIXTURE_PID:-}" ]] && kill -0 "$FIXTURE_PID" 2>/dev/null; then
  die "fixture pid $FIXTURE_PID is still running"
fi

printf 'protbot verify: stopped\n'
printf 'protbot verify: evidence kept at %s\n' "${EVIDENCE_DIR:-$PROTBOT_EVIDENCE_DIR}"
