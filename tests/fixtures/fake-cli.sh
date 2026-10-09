#!/bin/sh
# Test double for pass-cli, proton-drive, and curl. Mail uses Bridge, not this script.
set -eu

if [ "${PROTBOT_FAKE_SLEEP:-}" = "1" ]; then
    sleep 30 &
    if [ -n "${PROTBOT_FAKE_PID:-}" ]; then
        echo "$!" > "$PROTBOT_FAKE_PID"
    fi
    wait "$!"
    exit 0
fi

first=${1:-}
if [ "$first" = "--json" ] || [ "$first" = "--config" ]; then
    if [ -n "${PROTON_PASS_AGENT_TOKEN:-}" ] || [ -n "${PROTON_PASS_PERSONAL_ACCESS_TOKEN:-}" ]; then
        echo "secret env leaked" >&2
        exit 9
    fi
fi

case "$first" in
    info|login|item|vault)
        if [ -z "${PROTON_PASS_AGENT_REASON:-}" ] || [ -z "${PROTON_PASS_PERSONAL_ACCESS_TOKEN:-}" ]; then
            echo "missing pass env" >&2
            exit 2
        fi
        for arg in "$@"; do
            case "$arg" in
                *pst_*|*PROTON_PASS_AGENT_TOKEN*|*PROTON_PASS_PERSONAL_ACCESS_TOKEN*)
                    echo "token in argv" >&2
                    exit 3
                    ;;
            esac
        done
        ;;
esac

if [ -n "${PROTBOT_FAKE_LOG:-}" ]; then
    printf '%s\n' "$*" >> "$PROTBOT_FAKE_LOG"
fi

if [ "$first" = "--json" ]; then
    shift
fi
set -- ${1:-} ${2:-}

if [ "$1" = "messages" ] && [ "$2" = "list" ]; then
    printf '%s\n' '{"messages":[{"id":"m1","from":"owner@proton.me","subject":"Hi","snippet":"one line"}]}'
    exit 0
fi
if [ "$1" = "vault" ] && [ "$2" = "list" ]; then
    printf '%s\n' '{"vaults":[{"id":"v1","name":"Bot"}]}'
    exit 0
fi
if [ "$1" = "info" ] || [ "$1" = "login" ]; then
    printf '%s\n' '{}'
    exit 0
fi
if [ "$first" = "--config" ]; then
    printf '%s\n' 'BEGIN:VCALENDAR'
    exit 0
fi

echo "unmocked" >&2
exit 1
