# Task runner for the ibkr CLI. Install: `cargo install just` or a distro package.
set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

# List recipes.
default:
    @just --list

# Format the workspace.
fmt:
    cargo fmt --all

# Check formatting without writing.
fmt-check:
    cargo fmt --all -- --check

# Lint with clippy, warnings as errors.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Type-check every target.
check:
    cargo check --all-targets --all-features

# Run the test suite (nextest when available).
test:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run --all-features
    else
        cargo test --all-features
    fi

# Build the CLI.
build:
    cargo build

# Remove build artefacts.
clean:
    cargo clean

# Run the CLI with this repo's .env loaded, e.g. `just run health` or
# `just run -- health`.
#
# The binary resolves configuration from flags, then the environment, then
# defaults — it never reads .env itself, and the devcontainer's own environment
# points at the compose gateway (`https://ibkr-gateway:8080`) with the dev
# certificates. Sourcing .env here is what makes `just run` talk to the gateway
# .env actually names.
#
# `just` forwards a `--` separator verbatim, so drop it before handing the
# arguments to cargo; otherwise the binary sees it as its own argument.
# The attribute is what makes `"$@"` available (and safe for arguments with
# spaces, e.g. SQL): without it a shebang recipe receives no positional
# arguments at all.
[positional-arguments]
run *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi
    if [ "${1:-}" = "--" ]; then shift; fi
    exec cargo run -- "$@"

# Report changes in the upstream contract artifacts (docs/watch/sources.txt).
# `just contract-watch --update` records the new hashes once triaged.
[positional-arguments]
contract-watch *args:
    #!/usr/bin/env bash
    set -euo pipefail
    exec ./scripts/contract-watch.sh "$@"

# Smoke a live gateway through the built binary (read-only; changes nothing).
# Needs a reachable, configured gateway: run it where the gateway is.
canary:
    cargo build --release
    IBKR=target/release/ibkr ./scripts/canary.sh

# Audit dependencies (licenses, advisories, bans).
deny:
    cargo deny check

# Report unused dependencies.
machete:
    cargo machete

# Everything CI runs.
ci: fmt-check lint test

# Generate development mTLS certificates into .certs/.
certs:
    ./scripts/gen-dev-certs.sh

# Start the gateway (compose profile `gateway`).
gateway-up:
    docker compose -f .devcontainer/docker-compose.devcontainer.yml --profile gateway up -d --build ibkr-gateway

# Start the gateway plus the headless IB Gateway broker (needs broker creds).
broker-up:
    docker compose -f .devcontainer/docker-compose.devcontainer.yml --profile gateway --profile broker up -d --build

# Follow gateway logs.
gateway-logs:
    docker compose -f .devcontainer/docker-compose.devcontainer.yml --profile gateway logs -f ibkr-gateway

# Stop the compose services started by gateway-up/broker-up.
gateway-down:
    docker compose -f .devcontainer/docker-compose.devcontainer.yml --profile gateway --profile broker down
