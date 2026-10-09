#!/usr/bin/env bash
# Install a prebuilt protbot binary, the Pass CLI if needed, settings, and Grok registration.
# Does not compile Rust and does not write the agent token anywhere.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
base="${PROTBOT_RELEASE_BASE:?set PROTBOT_RELEASE_BASE to the GitHub release download URL}"
dest="${PROTBOT_BIN:-${HOME}/.local/bin/protbot}"
settings="${PROTBOT_SETTINGS:-${XDG_CONFIG_HOME:-${HOME}/.config}/protbot/settings.toml}"

mkdir -p "$(dirname "${dest}")"

if ! command -v pass-cli >/dev/null 2>&1; then
    curl -fsSL https://proton.me/download/pass-cli/install.sh | bash
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

PROTBOT_BIN="${dest}" PROTBOT_SETTINGS="${settings}" "${root}/scripts/register.sh"
