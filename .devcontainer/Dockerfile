# Development image for the ibkr CLI.
#
# Base image supplies rustup; the exact toolchain from rust-toolchain.toml is
# then installed so the container is hermetic — no toolchain download on first
# run inside it.
ARG RUST_VERSION=1.94
FROM rust:${RUST_VERSION}-bookworm

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# Exact compiler pinned by rust-toolchain.toml (keep the two in step).
ARG TOOLCHAIN=1.94.1

# Base tooling: shell quality-of-life, git, openssl (dev certificates), jq.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        fd-find \
        fzf \
        git \
        jq \
        less \
        openssl \
        pkg-config \
        ripgrep \
        sudo \
    && ln -s /usr/bin/fdfind /usr/bin/fd \
    && rm -rf /var/lib/apt/lists/*

# rustup-installed toolchains are not on the login PATH, and VS Code terminals
# run login shells. Restore them for interactive use, plus ~/.local/bin where
# the OMP installer (post-create.sh) places the agent binary.
ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:/usr/local/rustup/bin:/home/vscode/.local/bin:$PATH
RUN printf 'export PATH="/usr/local/cargo/bin:/usr/local/rustup/bin:$HOME/.local/bin:$PATH"\n' \
        > /etc/profile.d/dev-path.sh

# Pinned toolchain with the components the project lints with, then the
# developer tooling, installed as root so it lands in the root-owned
# CARGO_HOME.
RUN rustup toolchain install "${TOOLCHAIN}" --profile minimal \
        --component clippy,rustfmt,rust-src,llvm-tools-preview \
    && rustup default "${TOOLCHAIN}"
RUN curl -LsSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | sh \
    && cargo binstall --no-confirm cargo-nextest cargo-deny cargo-machete just

# Non-root developer user (devcontainer convention). `vscode` owns the
# toolchain directories so `cargo fetch`/`cargo install` work without sudo.
ARG USERNAME=vscode
RUN useradd --create-home --shell /bin/bash "$USERNAME" \
    && echo "$USERNAME ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/$USERNAME" \
    && chmod 0440 "/etc/sudoers.d/$USERNAME" \
    && chown -R "$USERNAME:$USERNAME" "$CARGO_HOME" "$RUSTUP_HOME"

USER $USERNAME
WORKDIR /workspaces/ibkr.rs
