#!/usr/bin/env bash
# Build the release binary and write a SHA-256 checksum next to it.
# Upload dist/protbot, dist/protbot.sha256, and dist/version to the GitHub release.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${root}"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-0}"
cargo build --release --locked
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
mkdir -p dist
cp -f target/release/protbot dist/protbot
chmod 0755 dist/protbot
printf '%s\n' "${version}" > dist/version
(
    cd dist
    sha256sum protbot > protbot.sha256
)
echo "dist/protbot"
echo "dist/protbot.sha256"
echo "dist/version"
echo "gh release create v${version} dist/protbot dist/protbot.sha256 dist/version"
