#!/usr/bin/env bash
# Register protbot with the bot by writing its connector file.
# The command is scripts/spawn.sh. Secrets are not written. grok mcp add is not called.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="${PROTBOT_BIN:-${HOME}/.local/bin/protbot}"
spawn="${PROTBOT_SPAWN:-${root}/scripts/spawn.sh}"
settings="${PROTBOT_SETTINGS:-${XDG_CONFIG_HOME:-${HOME}/.config}/protbot/settings.toml}"
dest="${PROTBOT_CONNECTOR_FILE:-${XDG_CONFIG_HOME:-${HOME}/.config}/protbot/connector.json}"
user="${PROTBOT_BRIDGE_USER:?set PROTBOT_BRIDGE_USER to the mailbox address Bridge shows}"

if [[ "${user}" != *@* ]]; then
    echo "protbot: PROTBOT_BRIDGE_USER must be the mailbox address Bridge shows" >&2
    exit 1
fi

mkdir -p "$(dirname "${dest}")"
umask 077
cat > "${dest}.tmp" <<EOF
{
  "name": "protbot",
  "command": "${spawn}",
  "args": [],
  "env": {
    "PROTBOT_BIN": "${bin}",
    "ALLOW_WRITES": "${ALLOW_WRITES:-false}",
    "PROTBOT_RECIPIENT_ALLOWLIST": "${PROTBOT_RECIPIENT_ALLOWLIST:-}",
    "PROTBOT_SETTINGS": "${settings}",
    "PROTBOT_TOKEN_PASS_NAME": "${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}",
    "PROTBOT_BRIDGE_PASS_NAME": "${PROTBOT_BRIDGE_PASS_NAME:-protbot/bridge-password}",
    "PROTBOT_BRIDGE_USER": "${user}",
    "PROTBOT_BRIDGE_HOST": "127.0.0.1",
    "PROTBOT_IMAP_PORT": "${PROTBOT_IMAP_PORT:-1143}",
    "PROTBOT_SMTP_PORT": "${PROTBOT_SMTP_PORT:-1025}"
  }
}
EOF
install -m 0600 "${dest}.tmp" "${dest}"
rm -f "${dest}.tmp"
echo "protbot: connector command is ${spawn}"
echo "protbot: wrote ${dest}"
echo "protbot: tell the bot to add a custom stdio MCP server that runs ${spawn}"
