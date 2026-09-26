#!/usr/bin/env bash
#
# Watch upstream contract artifacts for changes.
#
# The client decodes what the gateway publishes. When that contract moves — a
# field renamed, an enum spelling changed, a route added — the client is the
# first thing to notice, usually in production. This script notices instead: it
# fetches each watched source, hashes the body, and compares against the
# recorded hash. A change means the upstream published something, and a human
# (or an agent) decides whether it touches what we decode.
#
# Usage:
#   scripts/contract-watch.sh            # report changes; exit 1 if any
#   scripts/contract-watch.sh --update   # report changes and record the new hashes
#
# Only stable machine artifacts belong in docs/watch/sources.txt: raw JSON,
# OpenAPI documents, raw changelogs. Rendered HTML changes on every request and
# would make this cry wolf daily.
#
# Deliberately plain text (no jq, no python): the sources and hashes files are
# reviewable in a diff, and the script runs anywhere curl and sha256sum do.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

SOURCES=docs/watch/sources.txt
HASHES=docs/watch/hashes.txt
TIMEOUT=30
update=0
if [ "${1:-}" = "--update" ]; then
    update=1
elif [ -n "${1:-}" ]; then
    echo "usage: ${0##*/} [--update]" >&2
    exit 2
fi

if [ ! -f "$SOURCES" ]; then
    echo "no $SOURCES to read" >&2
    exit 2
fi
touch "$HASHES"

recorded() {
    # Last hash recorded for a source, or nothing.
    awk -v name="$1" '$2 == name { hash = $1 } END { print hash }' "$HASHES"
}

record() {
    # Replace this source's line in place, keeping the file sorted by name.
    grep -v -E "[[:space:]]$1\$" "$HASHES" > "$HASHES.tmp" 2>/dev/null || true
    printf '%s %s\n' "$2" "$1" >> "$HASHES.tmp"
    sort -k2 -o "$HASHES" "$HASHES.tmp"
    rm -f "$HASHES.tmp"
}

checked=0
changed=0
unreachable=0

while read -r name url; do
    case "$name" in '' | '#'*) continue ;; esac
    checked=$((checked + 1))

    if ! body=$(curl -fsSL --max-time "$TIMEOUT" "$url" 2>/dev/null | tr -s '[:space:]' ' '); then
        echo "unreachable: $name ($url)"
        unreachable=$((unreachable + 1))
        continue
    fi

    hash=$(printf '%s' "$body" | sha256sum | cut -c1-16)
    previous=$(recorded "$name")

    if [ "$previous" = "$hash" ]; then
        continue
    fi

    if [ -z "$previous" ]; then
        echo "first observation: $name ($url)"
    else
        echo "changed: $name ($url)"
        changed=$((changed + 1))
    fi
    if [ "$update" -eq 1 ]; then
        record "$name" "$hash"
    fi
done < "$SOURCES"

echo "$checked sources checked: $changed changed, $unreachable unreachable"
# Unreachable is reported but not fatal: a network blip is not a contract change.
if [ "$changed" -gt 0 ]; then
    [ "$update" -eq 1 ] && exit 0
    echo "run ${0##*/} --update to record the new hashes once triaged" >&2
    exit 1
fi
