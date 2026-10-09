#!/usr/bin/env bash
# Register protbot with Grok. The agent token is not part of this command.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="${PROTBOT_BIN:-${HOME}/.local/bin/protbot}"
spawn="${PROTBOT_SPAWN:-${root}/scripts/spawn.sh}"
settings="${PROTBOT_SETTINGS:-${XDG_CONFIG_HOME:-${HOME}/.config}/protbot/settings.toml}"

if ! command -v grok >/dev/null 2>&1; then
    echo "protbot: grok is not on PATH" >&2
    echo "grok mcp add protbot -e PROTBOT_BIN=${bin} -e ALLOW_WRITES=\${ALLOW_WRITES:-false} -e PROTBOT_RECIPIENT_ALLOWLIST=\${PROTBOT_RECIPIENT_ALLOWLIST:-} -e PROTBOT_SETTINGS=${settings} -e PROTBOT_TOKEN_PASS_NAME=\${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token} -- ${spawn}" >&2
    exit 1
fi

exec grok mcp add protbot \
    -e "PROTBOT_BIN=${bin}" \
    -e "ALLOW_WRITES=${ALLOW_WRITES:-false}" \
    -e "PROTBOT_RECIPIENT_ALLOWLIST=${PROTBOT_RECIPIENT_ALLOWLIST:-}" \
    -e "PROTBOT_SETTINGS=${settings}" \
    -e "PROTBOT_TOKEN_PASS_NAME=${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}" \
    -- "${spawn}"
