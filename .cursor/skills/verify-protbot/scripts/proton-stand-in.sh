#!/bin/sh
# Stand-in for pass-cli and proton-drive when those binaries are not installed.
# Logs argv and the Pass agent reason. Never logs or accepts a token on argv.
# Pass login templates arrive on stdin and are stored under PROTBOT_VERIFY_PASS_DIR.
set -eu

log=${PROTBOT_VERIFY_CLI_LOG:-}
pass_dir=${PROTBOT_VERIFY_PASS_DIR:-}

for arg in "$@"; do
  case "$arg" in
    *pst_*|*PROTON_PASS_*|*PROTON_BRIDGE_PASSWORD*)
      echo "protbot stand-in: refusing a secret on argv" >&2
      exit 3
      ;;
  esac
done

if [ "${1:-}" = "--json" ]; then
  shift
fi

if [ -n "$log" ]; then
  printf '%s\n' "$*" >>"$log"
  if [ -n "${PROTON_PASS_AGENT_REASON:-}" ]; then
    case "$PROTON_PASS_AGENT_REASON" in
      *pst_*|*PROTON_PASS_*|*PROTON_BRIDGE_PASSWORD*)
        echo "protbot stand-in: refusing to log a secret reason" >&2
        exit 3
        ;;
    esac
    printf 'reason=%s\n' "$PROTON_PASS_AGENT_REASON" >>"$log"
  fi
fi

require_pass() {
  if [ -z "${PROTON_PASS_PERSONAL_ACCESS_TOKEN:-}" ] || [ -z "${PROTON_PASS_AGENT_REASON:-}" ]; then
    echo "missing pass env" >&2
    exit 2
  fi
}

cmd=${1:-}
case "$cmd" in
  info|login)
    require_pass
    printf '%s\n' '{}'
    ;;
  vault)
    require_pass
    printf '%s\n' '{"vaults":[{"id":"v1","name":"Bot"}]}'
    ;;
  item)
    require_pass
    sub=${2:-}
    case "$sub" in
      list)
        printf '%s\n' '{"items":[{"id":"item1","title":"Example"}]}'
        ;;
      view)
        printf '%s\n' '{"item":{"id":"item1","title":"Example","username":"bot"}}'
        ;;
      totp)
        printf '%s\n' '{"code":"424242"}'
        ;;
      create)
        if [ -n "$pass_dir" ]; then
          mkdir -p "$pass_dir"
          cat >"$pass_dir/last-create.json"
          chmod 600 "$pass_dir/last-create.json"
        else
          cat >/dev/null
        fi
        printf '%s\n' '{"ok":true}'
        ;;
      delete)
        printf '%s\n' '{"ok":true}'
        ;;
      *)
        echo "unmocked item command" >&2
        exit 1
        ;;
    esac
    ;;
  filesystem)
    action=${2:-}
    dest=
    for arg in "$@"; do
      dest=$arg
    done
    case "$action" in
      list)
        printf '%s\n' '[{"name":"notes.txt","type":"file"}]'
        ;;
      info)
        printf '%s\n' '{"name":"notes.txt","type":"file"}'
        ;;
      upload)
        printf '%s\n' 'uploaded'
        ;;
      download)
        if [ -d "$dest" ]; then
          printf '%s\n' 'fixture' >"$dest/notes.txt"
        fi
        printf '%s\n' 'downloaded'
        ;;
      trash)
        printf '%s\n' 'trashed'
        ;;
      *)
        echo "unmocked filesystem command" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "unmocked" >&2
    exit 1
    ;;
esac
