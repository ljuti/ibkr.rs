# Contributing

Thanks for looking. This is a CLI client for a separate service (the IBKR
gateway); gateway bugs belong in that repository, issues here are about the
client.

## Getting set up

Requirements: the pinned Rust toolchain (1.94.1, installed automatically via
`rust-toolchain.toml`), and optionally `just` and Docker for the gateway
recipes.

```bash
just build            # cargo build
ibkr configure        # point the client at a gateway: just build && ./target/debug/ibkr configure
just ci               # fmt-check + clippy + test — what CI runs
just deny             # licences, advisories, bans (needs cargo-deny)
```

Two checks need more than a checkout, so CI cannot run them:

- `just canary` — read-only smoke of a live gateway through the built binary.
  Run it where the gateway is reachable.
- `just contract-watch` — hashes the upstream artifacts in
  `docs/watch/sources.txt` and reports what moved. It also runs daily in CI.

## House rules

**No secrets, ever.** Not in a commit, not in a test fixture, not in a
screenshot. The repository is MIT-licensed and public: a token that lands in a
commit is a token to rotate. `.gitignore` covers `.env*` (except the example),
`.certs/`, `tmp/`, `*.key`/`*-key.pem`, key stores and the local store. Never
paste a statement export either — those carry account numbers, positions and
P&L, which is why the conformance vectors use synthetic accounts.

**Commits** follow the conventional style already in the log —
`feat(store):`, `fix(output):`, `docs:`, `chore:` — one concern per commit, and
the body explains why the change is right, not what the diff does.

**Tests** are required for behaviour: a new case should be one that would have
failed before the change. Prefer asserting what a consumer sees (rows, counts,
sums, error text) over that a function was called. For anything about the
gateway's wire contract, add a case to `conformance/vectors/` first — it is the
specification, and it is what keeps the client and the gateway from drifting.

**Style** is enforced: `cargo fmt --all`, and clippy with warnings denied
(`just lint`). Pedantic lints are on with a few deliberate exemptions in
`Cargo.toml`; if a lint is wrong about this code, say so in a comment rather
than silencing it globally.

## Where things live

| Path | What belongs there |
|---|---|
| `src/client.rs` | transport: mutual TLS, bearer auth, retry policy, error decoding |
| `src/api/` | typed endpoint methods on `Client`, one module per route area |
| `src/commands.rs` | dispatch: CLI → client calls, order confirmation |
| `src/cli.rs` | the clap command tree (mirrors the gateway's surface) |
| `src/output.rs` | JSON and table projections — has no network access |
| `src/config.rs` | flag → environment → config file → default resolution |
| `src/configure.rs` | the first-run wizard that writes the config file |
| `src/store/` | the local `SQLite` store: schema, ingest, queries |
| `conformance/` | the gateway payload contract as data |
| `scripts/` | dev certificates, the contract watch, the canary |

## Adding things

**An endpoint.** Add the typed call in the relevant `src/api/` module, the wire
types in `src/types.rs`, a CLI subcommand in `src/cli.rs`, dispatch in
`src/commands.rs`, and a projection in `src/output.rs` (or `json` passthrough
when a table adds nothing). Cover the decode with a test that uses the
documented payload shape.

**A store change.** Bump `SCHEMA_VERSION` in `src/store/schema.rs` and add the
step to `MIGRATIONS`; derived rows are dropped and re-synced rather than
translated, so say in the migration why that is safe. Add a conformance case if
the change is about how a payload lands in the store.

**A Flex rule.** Write the vector first, watch it fail, then make it pass. Those
rules were learned against real statements and are expensive to rediscover.
