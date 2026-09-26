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

# Run the CLI, e.g. `just run -- health`.
run *args:
    cargo run -- {{args}}

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
