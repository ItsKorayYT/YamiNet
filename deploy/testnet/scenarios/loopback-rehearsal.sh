#!/usr/bin/env bash
# Single-machine loopback rehearsal for the public-testnet kit.
#
# This starts a bootstrap node, a relay and an outbound-only website host on
# 127.0.0.1, then checks the same signed DHT / pinned TLS / relay / YNP paths.
# It is a simulation. It does not prove VPS, home-router, firewall or
# independent-client behaviour. No firewall rule or router setting changes.
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../../.." && pwd)
BIN_DIR=${YAMINET_BIN_DIR:-"$REPO_ROOT/target/release"}
EXE=""
case "$(uname -s 2>/dev/null || echo unknown)" in
    MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
esac
if [ ! -x "$BIN_DIR/yami-node$EXE" ] || [ ! -x "$BIN_DIR/yami-relay$EXE" ] || [ ! -x "$BIN_DIR/yami-testnet$EXE" ]; then
    BIN_DIR="$REPO_ROOT/target/debug"
fi
for binary in yami-node yami-relay yami-testnet; do
    [ -x "$BIN_DIR/$binary$EXE" ] || {
        printf 'error: %s not found; build first: cargo build -p yami-node -p yami-relay-cli -p yami-testnet\n' "$binary" >&2
        exit 1
    }
done

PROFILE=$(mktemp -d "${TMPDIR:-/tmp}/yaminet-rehearsal.XXXXXX")
if [ -n "$EXE" ]; then
    # Git Bash's native children need a Windows-absolute path, while its own
    # utilities accept the mixed C:/ form.
    PROFILE=$(cygpath -m "$PROFILE")
fi
LOGS="$PROFILE/logs"
PIDS=()

cleanup() {
    local pid
    for pid in "${PIDS[@]:-}"; do
        kill -TERM "$pid" 2>/dev/null || true
    done
    for pid in "${PIDS[@]:-}"; do
        wait "$pid" 2>/dev/null || true
    done
    if [ "${YAMINET_KEEP_PROFILE:-0}" = "1" ]; then
        printf 'kept profile: %s\n' "$PROFILE"
    else
        rm -rf "$PROFILE"
    fi
}
trap cleanup EXIT

read_env() {
    sed -n "s/^$1='\(.*\)'$/\1/p" "$PROFILE/addresses.env" | head -n 1
}

printf '== YamiNet testnet loopback rehearsal (SIMULATION) ==\n'
"$BIN_DIR/yami-testnet$EXE" init \
    --profile "$PROFILE" \
    --relay --site --client \
    --binary-dir "$BIN_DIR"
mkdir -p "$LOGS"

"$BIN_DIR/yami-node$EXE" --config "$PROFILE/bootstrap/bootstrap.conf" \
    >"$LOGS/bootstrap.log" 2>&1 &
PIDS+=("$!")
sleep 0.5

"$BIN_DIR/yami-relay$EXE" --config "$PROFILE/relay/relay.conf" \
    >"$LOGS/relay.log" 2>&1 &
PIDS+=("$!")
sleep 0.5

"$BIN_DIR/yami-node$EXE" --config "$PROFILE/host/host.conf" \
    >"$LOGS/host.log" 2>&1 &
PIDS+=("$!")

HOST_URL=$(read_env HOST_URL)
[ -n "$HOST_URL" ] || {
    printf 'error: profile did not contain a HOST_URL\n' >&2
    exit 1
}
printf 'Website address: %s\n' "$HOST_URL"
printf 'Waiting for signed publication and relay registration...\n'

REPORT="$LOGS/preflight.txt"
PASS=0
for attempt in $(seq 1 20); do
    if "$BIN_DIR/yami-testnet$EXE" preflight \
        --config "$PROFILE/client/client.conf" \
        --url "$HOST_URL" \
        --asset cover \
        --route relay \
        --timeout 10 \
        >"$REPORT" 2>&1
    then
        PASS=1
        break
    fi
    sleep 1
done
if [ "$PASS" != "1" ]; then
    printf 'error: loopback preflight did not pass; logs follow\n' >&2
    tail -n 20 "$LOGS/bootstrap.log" "$LOGS/relay.log" "$LOGS/host.log" >&2 || true
fi
cat "$REPORT"

printf '\n== Expected direct-route failure (relay-only host) ==\n'
if "$BIN_DIR/yami-testnet$EXE" preflight \
    --config "$PROFILE/client/client.conf" \
    --url "$HOST_URL" \
    --route direct \
    --timeout 10 \
    >"$LOGS/direct.txt" 2>&1
then
    printf 'error: direct route unexpectedly passed for a relay-only host\n' >&2
    exit 1
fi
grep -E '^\[(FAIL|NOT RUN)\].*(Direct|route|Relay)' "$LOGS/direct.txt" || true
printf 'Direct route failed as expected; no plaintext fallback occurred.\n'

if [ "$PASS" != "1" ]; then
    exit 1
fi
printf '\nRehearsal PASS (loopback simulation only; independent machines NOT VERIFIED).\n'
