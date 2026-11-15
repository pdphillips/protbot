#!/usr/bin/env bash
# tmux pane command. stdin and stdout stay on the pane PTY. stderr is a file.
# Echo is off so the stdout capture is server output, not the JSON we typed.
set -euo pipefail

if [[ -z "${PROTBOT_BIN:-}" || -z "${PROTBOT_STDERR_LOG:-}" ]]; then
  printf 'protbot verify: pane is missing PROTBOT_BIN or PROTBOT_STDERR_LOG\n' >&2
  exit 1
fi

stty -echo
exec "$PROTBOT_BIN" 2>>"$PROTBOT_STDERR_LOG"
