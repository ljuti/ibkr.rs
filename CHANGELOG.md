# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims
for [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

First public release (crate version `0.1.0`).

### Added

- **Gateway client** for the whole REST surface: `health`, `contracts details`,
  `contracts search`, `market-data historical`, `accounts positions`,
  `accounts summary`, `accounts pnl`, `orders place`, `orders bracket`,
  `orders list`, `orders completed`, `orders executions`, `orders cancel`,
  `flex config`, `flex set-query`, `flex set-token`, `flex report`.
- **`configure`**: prompts for the gateway URL, bearer token and certificate
  paths (token typed without echo), writes
  `$XDG_CONFIG_HOME/ibkr/config.toml` with mode `0600`, and checks the
  connection. `configure --show` prints the effective settings, the layer each
  value came from and the file in use. Resolution is
  flag → environment → config file → default, so an exported `.env` is optional.
- **Read-only mode**: `--read-only` / `IBKR_READ_ONLY` / `read-only` in the
  config file refuse `orders place`, `orders bracket` and `orders cancel` before
  validation and before anything is sent.
- **`store`**: Flex statements in a local `SQLite` file —
  `store sync` (idempotent, overlap-safe, ETag-cached, fail-soft across
  reports), `store query` (read-only SQL), `store schema`, and `store status`
  (what is synced, and which analyses the stored rows support).
- **`store import`**: store a payload written by `flex report -o json` (from a
  file or stdin) without a gateway — archived windows, payloads from another
  machine, or one kept from a bug report.
- **Analysis views**: `pnl_by_symbol`, `pnl_by_month`, `pnl_by_asset_class`,
  `trade_stats`, `cash_by_type`, `open_positions`, `round_trips` — with the
  broker's own realized P/L, and closed-lot rows for exact round trips.
- **Conformance vectors** (`conformance/vectors/`): the gateway's Flex payload
  contract as data, run by `cargo test --test conformance`. Cross-language by
  design — the gateway's tests or a port can reproduce the same cases.
- **`scripts/contract-watch.sh`** (`just contract-watch`): hashes the upstream
  artifacts in `docs/watch/sources.txt` and reports what moved, with a daily
  workflow that records hashes and opens an issue.
- **`scripts/canary.sh`** (`just canary`): read-only smoke of a live gateway —
  transport, broker session, a typed round trip, a snapshot, and a Flex sync
  into a throwaway store. Diagnoses a degraded broker session (IBKR 10159) even
  when `/health` reports connected.
- **Docs**: `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, and the MIT license.

### Changed

- Tables for positions carry option identity (`expiry`, `right`, `strike`), and
  the Flex round-trip view renders closed lots — `closed`, `opened`, cost basis
  and realized P/L — when the query provides them.

### Fixed

- Flex enum attributes that arrive blank (or unknown) are treated as absent
  rather than as the `Unknown` sentinel.
- Cash transactions print the movements (`DETAIL`) and report how many summary
  rows were omitted, instead of inviting a double count.
- Positions and search projections no longer carry columns the gateway always
  leaves empty.

### Known limitations

- `stream *` commands are declared in `--help` but not implemented; they exit
  with `error: stream <name> is not implemented yet`.
- `open_positions` is derived from the synced window: it is a reconciliation
  aid, not an authority (`accounts positions` is).
- Round trips require the Flex query to include the Closed Lots level; the
  report falls back to closing executions and says so.
