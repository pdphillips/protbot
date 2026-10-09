#!/usr/bin/env bash
# Read the Pass agent token and the Bridge IMAP password from the local pass
# store, export them for this process only, then exec protbot.
# Neither value is written to disk, placed in argv, or echoed.
set -euo pipefail

if [[ $# -ne 0 ]]; then
    echo "protbot: spawn takes no arguments" >&2
    exit 2
fi

read_pass() {
    local name="$1"
    local value
    if ! command -v pass >/dev/null 2>&1; then
        echo "protbot: pass is not installed. Store ${name} and start again. Protbot will not prompt." >&2
        exit 1
    fi
    # pinentry-mode=error and a closed stdin: a missing entry or a locked key fails.
    # This process does not ask for a passphrase.
    value="$(PASSWORD_STORE_GPG_OPTS="--batch --pinentry-mode error" pass show "$name" </dev/null 2>/dev/null | head -n 1 || true)"
    if [[ -z "${value}" ]]; then
        echo "protbot: pass entry ${name} is missing. Sign Proton Mail Bridge in on this machine, store protbot/agent-token and protbot/bridge-password, then start again. Protbot will not prompt." >&2
        exit 1
    fi
    printf '%s' "${value}"
}

if [[ -z "${PROTON_PASS_AGENT_TOKEN:-}" ]]; then
    token="$(read_pass "${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}")"
    export PROTON_PASS_AGENT_TOKEN="${token}"
    unset token
fi

if [[ -z "${PROTON_BRIDGE_PASSWORD:-}" ]]; then
    bridge_password="$(read_pass "${PROTBOT_BRIDGE_PASS_NAME:-protbot/bridge-password}")"
    export PROTON_BRIDGE_PASSWORD="${bridge_password}"
    unset bridge_password
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -n "${PROTBOT_BIN:-}" && -x "${PROTBOT_BIN}" ]]; then
    bin="${PROTBOT_BIN}"
elif [[ -x "${HOME}/.local/bin/protbot" ]]; then
    bin="${HOME}/.local/bin/protbot"
elif [[ -x "${root}/target/release/protbot" ]]; then
    bin="${root}/target/release/protbot"
else
    echo "protbot: binary not found; set PROTBOT_BIN" >&2
    exit 1
fi

exec "${bin}"
