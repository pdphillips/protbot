#!/usr/bin/env bash
# Inject the Proton Pass agent token into the environment, then exec protbot.
# The token is never written to disk, never placed in argv, and never echoed.
set -euo pipefail

if [[ $# -ne 0 ]]; then
    echo "protbot: spawn takes no arguments" >&2
    exit 2
fi

if [[ -z "${PROTON_PASS_AGENT_TOKEN:-}" ]]; then
    name="${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}"
    if ! command -v pass >/dev/null 2>&1; then
        echo "protbot: pass is not installed and PROTON_PASS_AGENT_TOKEN is unset" >&2
        exit 1
    fi
    token="$(pass show "$name" | head -n 1 || true)"
    if [[ -z "${token}" ]]; then
        echo "protbot: empty token from pass name ${name}" >&2
        exit 1
    fi
    export PROTON_PASS_AGENT_TOKEN="${token}"
    unset token
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
