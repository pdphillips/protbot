#!/usr/bin/env bash
# Install a prebuilt protbot binary, Pass CLI, and Proton Mail Bridge if needed.
# Check the pass entries and the Bridge session, then register the connector.
# Does not compile Rust, does not call grok mcp add, and does not write secrets.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base="${PROTBOT_RELEASE_BASE:?set PROTBOT_RELEASE_BASE to the GitHub release download URL}"
: "${PROTBOT_BRIDGE_USER:?set PROTBOT_BRIDGE_USER to the mailbox address Bridge shows}"
dest="${PROTBOT_BIN:-${HOME}/.local/bin/protbot}"
settings="${PROTBOT_SETTINGS:-${XDG_CONFIG_HOME:-${HOME}/.config}/protbot/settings.toml}"
token_name="${PROTBOT_TOKEN_PASS_NAME:-protbot/agent-token}"
bridge_name="${PROTBOT_BRIDGE_PASS_NAME:-protbot/bridge-password}"
imap_port="${PROTBOT_IMAP_PORT:-1143}"

mkdir -p "$(dirname "${dest}")"

if ! command -v pass-cli >/dev/null 2>&1; then
    curl -fsSL https://proton.me/download/pass-cli/install.sh | bash
fi

if ! command -v protonmail-bridge >/dev/null 2>&1 && ! command -v proton-bridge >/dev/null 2>&1; then
    if ! command -v apt-get >/dev/null 2>&1; then
        echo "protbot: Proton Mail Bridge is not installed. Install the official package from https://proton.me/mail/bridge and sign in on this machine. A free account cannot sign into Bridge." >&2
        exit 1
    fi
    deb_url="${PROTBOT_BRIDGE_DEB:-https://proton.me/download/bridge/protonmail-bridge_3.27.0-1_amd64.deb}"
    tmpdeb="$(mktemp --suffix=.deb)"
    curl -fsSL "${deb_url}" -o "${tmpdeb}"
    if [[ "$(id -u)" -eq 0 ]]; then
        apt-get install -y "${tmpdeb}"
    else
        sudo apt-get install -y "${tmpdeb}"
    fi
    rm -f "${tmpdeb}"
fi

pass_ready() {
    local name="$1"
    local value
    if ! command -v pass >/dev/null 2>&1; then
        echo "protbot: pass is not installed. A restored Grok Bot computer does not keep the password store. Protbot will not prompt." >&2
        exit 1
    fi
    value="$(PASSWORD_STORE_GPG_OPTS="--batch --pinentry-mode error" pass show "$name" </dev/null 2>/dev/null | head -n 1 || true)"
    if [[ -z "${value}" ]]; then
        echo "protbot: pass entry ${name} is missing. Sign Proton Mail Bridge in on this machine with the bot account password and 2FA, then store ${token_name} and ${bridge_name}. Protbot will not prompt and will not invent a password." >&2
        exit 1
    fi
    unset value
}

pass_ready "${token_name}"
pass_ready "${bridge_name}"

if ! timeout 3 bash -c "echo >/dev/tcp/127.0.0.1/${imap_port}" 2>/dev/null; then
    echo "protbot: Proton Mail Bridge is not listening on 127.0.0.1:${imap_port}. Sign Bridge in on this machine. A restored Grok Bot computer wipes that session. Protbot stopped and will not prompt." >&2
    exit 1
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT
curl -fsSL "${base}/version" -o "${tmpdir}/version"
if [[ -f "${dest}.version" ]] && cmp -s "${tmpdir}/version" "${dest}.version"; then
    echo "protbot already at $(tr -d '[:space:]' < "${dest}.version")"
else
    curl -fsSL "${base}/protbot" -o "${tmpdir}/protbot"
    curl -fsSL "${base}/protbot.sha256" -o "${tmpdir}/protbot.sha256"
    (cd "${tmpdir}" && sha256sum -c protbot.sha256)
    install -m 0755 "${tmpdir}/protbot" "${dest}"
    install -m 0644 "${tmpdir}/version" "${dest}.version"
fi

if [[ ! -f "${settings}" ]]; then
    mkdir -p "$(dirname "${settings}")"
    install -m 0600 "${root}/settings.example.toml" "${settings}"
fi

install -m 0755 "${root}/scripts/spawn.sh" "$(dirname "${dest}")/protbot-spawn"
PROTBOT_BIN="${dest}" PROTBOT_SPAWN="$(dirname "${dest}")/protbot-spawn" PROTBOT_SETTINGS="${settings}" "${root}/scripts/register.sh"
