#!/usr/bin/env bash
#
# Read-only smoke of a live gateway, through the built binary.
#
# This is the check that unit tests cannot be: it exercises the real gateway,
# the real certificates and the real client together, and it is safe to run
# against a funded account because nothing here places or cancels anything.
# Point it at a gateway that is configured and reachable — a cron on the host
# that runs the gateway, a self-hosted runner, or your own devcontainer — and
# treat a failure as "the client and the gateway have diverged".
#
# Usage:  IBKR=target/release/ibkr scripts/canary.sh
#         just canary

set -uo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

BIN=${IBKR:-target/release/ibkr}
if [ ! -x "$BIN" ]; then
    echo "no binary at $BIN — build it first (just canary does), or set IBKR" >&2
    exit 2
fi

failures=0
failed() {
    echo "FAILED: $1"
    shift
    for line in "$@"; do
        echo "  $line"
    done
    failures=$((failures + 1))
}
ok() { echo "ok: $1"; }

echo "canary: $BIN"
"$BIN" --version 2>/dev/null | head -1 | sed 's/^/  /'

# 1. Transport: certificates, bearer token, DNS, the broker session behind it.
if ! health=$("$BIN" health -o table 2>&1); then
    failed "health" "$health"
elif printf '%s' "$health" | grep -q 'connected *yes'; then
    ok "gateway reachable, broker connected"
else
    failed "health reports no broker connection" "$health"
fi

# 2. What configuration is in play — the answer to most "why is it doing that".
if show=$("$BIN" configure --show -o table 2>&1); then
    ok "configure --show: $(printf '%s' "$show" | awk '/^url /{print $2}') (from $(printf '%s' "$show" | awk '/^url /{print $3}'))"
else
    failed "configure --show" "$show"
fi

# 2. The broker session behind the gateway. A session can answer /health while
#    its upstream is gone — IBKR 10159 ("Error sending message to a CCP") — and
#    then every request that has to reach the broker hangs or fails while the
#    cheap ones keep working. Ask for something that must reach it.
if search=$("$BIN" contracts search AAPL -o json 2>&1); then
    ok "broker answers a symbol lookup"
elif printf '%s' "$search" | grep -q '10159'; then
    failed "broker upstream degraded (IBKR 10159) — the gateway's broker session needs a reconnect" \
        "$search" \
        "the gateway ships a watchdog for this: check its timer on the gateway host"
elif printf '%s' "$search" | grep -qE 'unauthorized|401'; then
    failed "the gateway rejected the bearer token" "$search"
else
    failed "contracts search" "$search"
fi

# 3. A typed round trip: request encoding, response decoding.
#    This one resolves a contract through the broker, so it is the check that
#    hangs first when the session above is degraded.
if details=$("$BIN" contracts details -s AAPL -o json 2>&1); then
    if printf '%s' "$details" | grep -q '"contractId"'; then
        ok "contracts details decodes"
    else
        failed "contracts details returned no contract" "$details"
    fi
else
    failed "contracts details" "$details"
fi

# 4. A snapshot endpoint: the envelope the store also consumes.
if positions=$("$BIN" accounts positions -o json 2>&1); then
    if printf '%s' "$positions" | grep -q '"count"'; then
        ok "accounts positions decodes"
    else
        failed "accounts positions returned no envelope" "$positions"
    fi
else
    failed "accounts positions" "$positions"
fi

# 5. Flex: the report registry, and — when a report is registered — a full
#    sync into a throwaway store plus a query over what landed.
if config=$("$BIN" flex config 2>&1); then
    report=$(printf '%s' "$config" | grep -o '"name": "[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -z "$report" ]; then
        echo "ok: flex config reachable (no reports registered, sync skipped)"
    else
        store=$(mktemp -d)/canary.db
        if sync=$("$BIN" store sync --report "$report" --db "$store" -o table 2>&1); then
            ok "store sync of '$report'"
        else
            failed "store sync of '$report'" "$sync"
        fi
        if status=$("$BIN" store status --db "$store" -o table 2>&1); then
            ok "store status: $(printf '%s' "$status" | grep '^store holds' || echo 'empty store')"
        else
            failed "store status" "$status"
        fi
        rm -rf "$(dirname "$store")"
    fi
else
    failed "flex config" "$config"
fi

echo
if [ "$failures" -eq 0 ]; then
    echo "canary passed"
    exit 0
fi
echo "canary failed: $failures check(s)"
exit 1
