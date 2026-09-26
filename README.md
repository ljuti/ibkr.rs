# ibkr.rs

CLI client for the [IBKR gateway](https://github.com/ljuti/ibkr-gateway) — the service that owns the
single connection to TWS / IB Gateway and exposes it as an authenticated JSON
API (REST for request/response operations, WebSocket for streaming).

The gateway requires **mutual TLS** (a client certificate signed by its pinned
CA) **and** a bearer token on every `/api/v1/*` request. This client handles
both: it presents the client certificate from `.certs/` and attaches the token
to every call.

## Status

| Area | Commands | State |
|------|----------|-------|
| Setup | `configure`, `configure --show` | implemented |
| Health | `health` | implemented |
| Contracts | `contracts details`, `contracts search` | implemented |
| Market data | `market-data historical` | implemented |
| Accounts | `accounts positions`, `accounts summary`, `accounts pnl` | implemented |
| Orders | `orders place`, `bracket`, `list`, `completed`, `executions`, `cancel` | implemented |
| Flex | `flex config`, `set-query`, `set-token`, `report` | implemented |
| Store | `store sync`, `store import`, `store query`, `store schema`, `store status` | implemented |
| Streaming | `stream bars`, `market-data`, `tick-by-tick`, `orders`, `account-values` | not implemented |

The REST surface is complete: every route the gateway exposes has a typed
client method and a CLI command. Streaming is next; those commands are declared
in `--help` and exit with `error: <command> is not implemented yet`.

### Endpoints

| Command | Route |
|---------|-------|
| `health` | `GET /health` |
| `contracts details` | `POST /api/v1/contracts/details` |
| `contracts search <PATTERN>` | `GET /api/v1/contracts/search` |
| `market-data historical` | `POST /api/v1/market-data/historical` |
| `accounts positions` | `GET /api/v1/accounts/positions` |
| `accounts summary` | `GET /api/v1/accounts/summary` |
| `accounts pnl <ACCOUNT>` | `GET /api/v1/accounts/pnl` |
| `orders place` | `POST /api/v1/orders` |
| `orders bracket` | `POST /api/v1/orders/bracket` |
| `orders list` | `GET /api/v1/orders` |
| `orders completed` | `GET /api/v1/orders/completed` |
| `orders executions` | `GET /api/v1/orders/executions` |
| `orders cancel <ORDER_ID>` | `DELETE /api/v1/orders/{orderId}` |
| `flex config` | `GET /api/v1/config/flex` |
| `flex set-query <NAME> <ID>` | `PUT /api/v1/config/flex/queries/{reportName}` |
| `flex set-token <TOKEN>` | `PUT /api/v1/config/flex/token` |
| `flex report <NAME>` | `GET /api/v1/accounts/reports/{reportName}` |

## Layout

```
src/
  main.rs      # binary entry point (thin: delegates to lib::run)
  lib.rs       # run() -> ExitCode, error reporting with cause chains
  cli.rs       # clap command tree (mirrors the gateway's surface)
  commands.rs  # dispatch: CLI -> client calls, order confirmations
  client.rs    # transport: mutual TLS, bearer auth, retry policy, error decoding
  api/         # endpoint methods on Client, one module per route area
  config.rs    # flags + environment + defaults resolution
  error.rs     # Error / ApiError / ErrorKind
  output.rs    # JSON and table rendering, column projections
  store/       # local SQLite store of Flex statements (schema, sync, query)
  types.rs     # wire types (camelCase, timestamps as RFC 3339 strings)
conformance/
  vectors/           # the gateway payload contract as data (see conformance/README.md)
docs/
  watch/             # upstream contract artifacts to watch (scripts/contract-watch.sh)
scripts/
  gen-dev-certs.sh   # dev CA + server + client certificates
  contract-watch.sh  # report changes in the watched upstream artifacts
  canary.sh          # read-only smoke of a live gateway
.devcontainer/       # Rust dev environment (compose form)
deny.toml            # cargo-deny: licenses, advisories, bans, sources
justfile             # task runner
rust-toolchain.toml  # pinned toolchain (1.94.1), shared with CI
```

## Quick start

Run `ibkr configure` once. It prompts for the gateway URL, the bearer token and
the certificate paths (the token is typed without echo), writes them to
`~/.config/ibkr/config.toml` with mode `0600`, and checks the connection before
finishing. Every later command uses that file, so nothing needs exporting:

```bash
ibkr configure                    # prompt for each setting
ibkr configure --show             # what is effective, and where each value came from
ibkr health
```

Non-interactive setups pass the values as flags
(`ibkr configure --url https://host:8090 --token … --ca-cert …/ca.pem …`); the
environment variables below still work and still win over the file, which is
what CI and throwaway shells usually want. The gateway must be running first —
the two ways to get one:

**A. Gateway in the devcontainer stack** (needs
[ljuti/ibkr-gateway](https://github.com/ljuti/ibkr-gateway) checked out at
`../ibkr-gateway`):

```bash
cp .env.example .env
./scripts/gen-dev-certs.sh     # generates .certs/ (ca, server, client)
just gateway-up                # builds + starts the gateway from ../ibkr-gateway
```

`just gateway-up` runs the gateway alone; it stays in reconnect backoff until a
broker appears, so start it only if you want the HTTP API surface. To add the
headless IB Gateway (real credentials required in `../ibkr-gateway/.env`):

```bash
just broker-up                 # gateway + headless IB Gateway, first boot ~2-3 min
```

**B. Gateway already running elsewhere** (for example on the host, or in the
gateway repo's own devcontainer):

```bash
export IBKR_GATEWAY_URL=https://127.0.0.1:8080
export IBKR_GATEWAY_TOKEN=<token>
export IBKR_GATEWAY_CA_CERT=/path/to/ca.pem
export IBKR_GATEWAY_CLIENT_CERT=/path/to/client.pem
export IBKR_GATEWAY_CLIENT_KEY=/path/to/client-key.pem
```

Then:

```bash
cargo run -- health
cargo run -- health -o table
```

`health` is the one endpoint the gateway serves without a bearer token, so it
isolates transport problems: a TLS error means the certificates are wrong, and
any other failure is the network. A 401 on an `/api/v1/*` request means the
token is wrong.

## Configuration

Precedence is flag → environment → default. Certificate defaults
(`.certs/ca.pem`, `.certs/client.pem`, `.certs/client-key.pem`) apply only when
those files exist, so the client works outside this repository without extra
config.

The binary does **not** read `.env` — something has to put those values in its
environment. A devcontainer shell already has `IBKR_GATEWAY_URL` etc. set, by
the compose file, to the *compose* gateway (`https://ibkr-gateway:8080`) and the
dev certificates; that shadows `.env`. Either export the file first
(`set -a; . ./.env; set +a`), pass the flags (each has an env equivalent in the
table below), point the shell at it once (`.env` in `~/.bashrc`), or use
`just run <command>`, which sources `.env` for you.

| Flag | Environment variable | Default |
|------|----------------------|---------|
| `-u, --url` | `IBKR_GATEWAY_URL` | `https://127.0.0.1:8080` |
| `-t, --token` | `IBKR_GATEWAY_TOKEN` | — |
| `--ca-cert` | `IBKR_GATEWAY_CA_CERT` | `.certs/ca.pem` |
| `--client-cert` | `IBKR_GATEWAY_CLIENT_CERT` | `.certs/client.pem` |
| `--client-key` | `IBKR_GATEWAY_CLIENT_KEY` | `.certs/client-key.pem` |
| `--timeout` | `IBKR_GATEWAY_TIMEOUT_SEC` | `60` |
| `--max-retries` | `IBKR_GATEWAY_MAX_RETRIES` | `3` |
| `--tls-skip-verify` | `IBKR_GATEWAY_TLS_SKIP_VERIFY` | `false` |
| `--db` | `IBKR_STORE_DB` | `ibkr.db` |
| `--config` | `IBKR_CONFIG` | `$XDG_CONFIG_HOME/ibkr/config.toml` |
| `--read-only` | `IBKR_READ_ONLY` | `false` |
| `-o, --output` | — | `json` |

Errors are mapped onto the gateway's machine-readable `kind` values in
`src/error.rs`; `ErrorKind::RateLimited` carries the gateway's `Retry-After`.

### Order confirmation

`orders place`, `orders bracket` and `orders cancel` print the resolved intent
to stderr and ask before sending. `--yes` (or `-y`) skips the prompt; without it
a non-interactive stdin is **refused** rather than treated as consent, so an
unattended script cannot place an order by accident:

```bash
ibkr orders place -s AAPL --side BUY --quantity 100 --type LIMIT --limit-price 210
# about to place order:
# field        value
# instrument   AAPL (STK)
# side         BUY
# ...
# confirm? [y/N]

ibkr orders place ... --yes          # scripts: explicit, no prompt
```

Arguments are validated locally first (a `LIMIT` without `--limit-price`, a
non-positive quantity, or bracket targets on the wrong side of the entry all
fail before anything is sent).

Read-only mode refuses `orders place`, `orders bracket` and `orders cancel`
before validation and before anything is sent: turn it on with `--read-only`,
`IBKR_READ_ONLY`, or `read-only = true` in the config file (`ibkr configure
--read-only` writes that). Reads are untouched, so an agent or a cron job can be
given a store and a report feed without a way to reach the order book. The
gateway's own `GATEWAY_READ_ONLY` stays the authority; this is a guard rail that
fails earlier and says why.

Order placement sends an `Idempotency-Key`: either `--idempotency-key` or a
generated UUID, which is printed to stderr. Re-using that key with the same
body replays the original result instead of placing a second order — that is
what makes a retry after a timeout safe.

### Retries

The gateway enforces IBKR pacing itself and answers `429` with a `Retry-After`
hint. Read requests (`GET`) are retried against that hint, up to
`--max-retries`; each retry is reported on **stderr** so stdout stays
parseable. A hint longer than 60 seconds is not slept out — the error reaches
you instead of the command appearing to hang.

Mutations (`POST`, `PUT`, `DELETE`) are sent **exactly once**, whatever the
status: retrying an order could duplicate it. `--max-retries 0` disables
retrying reads too.

### Output modes

`-o json` (default) prints the gateway's payload as-is — complete and
pipe-friendly. `-o table` renders a fixed column projection for humans:

| Command | Columns |
|---------|---------|
| `accounts positions` | account, symbol, secType, expiry, right, strike, position, averageCost, currency |
| `accounts summary` | account, tag, value, currency |
| `orders list` / `completed` | orderId, symbol, action, totalQuantity, orderType, limitPrice, status, filled, remaining |
| `orders executions` | time, side, shares, price, commission, currency, executionId |
| `contracts details` | contractId, symbol, secType, exchange, currency, longName, marketName, minTick |
| `contracts search` | contractId, symbol, secType, currency |
| `market-data historical` | date, open, high, low, close, volume, wap, count |
| `flex report` | trades, closed lots (or closed trades), and cash transactions |

A table is lossy by design — it shows those columns and nothing else. Anything
that is not a row (envelope `count`/`truncated`, headings, notes, retry
warnings, confirmation prompts) goes to **stderr**, so `stdout` stays a clean
stream of rows or JSON. Columns shrink to fit the terminal, marking elision
with `…`; `--no-truncate` keeps natural width.

### Flex reports

The Flex Web Service accepts no date-range override, so each report's window is
fixed by its query's Period setting — the envelope's `fromDate`/`toDate` are
authoritative. Responses carry an `ETag`; the CLI prints it to stderr, and
`--etag <TAG>` sends it back as `If-None-Match`:

```bash
ibkr flex report transactions_30d              # -> etag "abc123"
ibkr flex report transactions_30d --etag '"abc123"'   # -> not modified
ibkr flex report transactions_30d --refresh    # bypass the gateway cache
```

The `table` mode prints three projections: every execution, the round trips,
and the cash side. The round-trip table — `closed`, `opened`, `symbol`,
`expiry`, `right`, `strike`, `quantity`, `cost`, `pnl`, `currency` — comes from
the statement's **closed lots** when the query asks for lot-level detail: Flex
keys a lot by its *opening* execution and carries the close, the matched
quantity, the cost basis and the realized P/L on the row, which is the exact
answer to "when was this opened, when was it closed, what did it realize?".
Without lots it falls back to closing executions, where `opened` is `-`
whenever IBKR leaves `openDateTime` empty at execution level (observed on
every execution of a real 365-day statement).

`-o json` prints the payload as delivered, which for cash means **both**
levels: filter on `levelOfDetail` (`DETAIL` is the movements, `SUMMARY`
restates the same money per report date) or on `transactionId` being present
before summing. The `table` mode shows the movements and reports how many
summary rows it left out.

`trades` and `lots` are separate arrays in the payload and stay separate
everywhere: a lot and its execution both carry `fifoPnlRealized`, so summing
them double counts
([ibkr-gateway#6](https://github.com/ljuti/ibkr-gateway/issues/6)). Rows are
selected by the statement's open/close indicator (`C`, or `C;O` for a
close-and-reopen); an absent or blank indicator falls back to a non-zero
realized P/L, and blank enum attributes arrive as `null`
([ibkr-gateway#5](https://github.com/ljuti/ibkr-gateway/issues/5)).

### Local trade store

Flex statements are the only complete transaction history the gateway can
produce, but they arrive one report (one window) at a time. `store` keeps them
in a single SQLite file so performance can be asked as SQL:

```bash
ibkr store sync --report last_365_days --report transactions_30d   # fetch + upsert
ibkr store import report.json                                     # store a saved payload, no gateway
ibkr store status                                                 # what is synced, what it can answer
ibkr store schema                                                 # tables, views, columns
ibkr store query "SELECT * FROM trade_stats" -o table
ibkr store query "SELECT * FROM pnl_by_month"
```

`store import` takes what `flex report -o json` writes — a file, or `-` for a
pipe — so a statement can be analysed without a gateway at all: an archived
window that has rolled off, a report fetched on another machine, or a payload
kept from a bug report. It replaces that report name's rows, exactly as a sync
does, and the file's name is the report name unless `--report` says otherwise.

One report failing does not cost the others their sync: a sweep reports each
report's outcome, keeps the ones that worked, and still exits non-zero. `store
status` then says what the stored rows are enough to answer — a report without
the Closed Lots level has no exact round trips, and one without Cash
Transactions has no cash movements, so the fix (a section to add, and
`--refresh` to re-fetch) is named where the sync finishes rather than left to be
discovered as an empty view.

`store sync` also names a missing section when it reads one back, and
`flex report -o table` does the same in its round-trip heading: the data is only
as capable as the Flex query behind it, and that is a configuration choice the
reader can act on.

`sync` is idempotent and overlap-safe: rows are keyed by their IB transaction
id, so re-syncing, re-fetching a corrected statement, or syncing overlapping
reports (the 30-day and 365-day windows share executions) all leave one row per
execution. Each report's ETag is stored, so a re-sync the gateway answers with
`304` ingests nothing. Executions and closed lots are kept in separate tables
(`round_trips` reads the lots) because Flex mixes levels of detail and summing
across them double counts — `ORDER`/`SYMBOL_SUMMARY`/`ASSET_SUMMARY` rows are
counted and dropped. Cash is partitioned the same way: only `DETAIL` rows are
stored, since `SUMMARY` rows aggregate the same money per report date.

Views answer the usual questions with the broker's own numbers:

| View | Answers |
|------|---------|
| `pnl_by_symbol`, `pnl_by_month`, `pnl_by_asset_class` | realized P/L per close, grouped |
| `trade_stats` | closes, wins, losses, flat, realized P/L, average win/loss |
| `cash_by_type` | dividends, withholding, fees, interest, transfers |
| `open_positions` | net position per contract, derived from executions |
| `round_trips` | opening execution ↔ close, quantity, cost basis, realized P/L (from the statement's closed lots) |

`realized_pnl` is the broker's figure for a closing execution, not something
recomputed here: pairing executions ourselves reproduces the broker's open
dates for ~88% of closes but not its P/L, because opens before the window,
securities transfers and specific-lot consumption are not visible in the
statement's execution rows.

`open_positions` inherits those limits and says so: `first_indicator` is the
open/close flag of the contract's earliest synced row, and a contract whose
window starts with a *close* was already open — its running sum is a fragment
of history and can look open when the broker holds nothing. Measured against
`accounts positions` for one 365-day window: 71 of 80 contracts with
`first_indicator = 'O'` agreed exactly, against 71 of 115 unfiltered; two live
positions never appear (transferred in, or opened before the window). Treat the
view as a reconciliation aid — `accounts positions` is the authority.

Cash rows carry the statement's event date, so `cash_by_type` totals per type
and grouping by month is one query away.

The file is ordinary SQLite: `sqlite3`, DuckDB, pandas and Metabase can all
read it. `store query` opens it read-only, so a stray statement cannot damage
what a sync took minutes to fetch.

## Development

Contributions are welcome — [CONTRIBUTING.md](CONTRIBUTING.md) has the house
rules and where things live. [SECURITY.md](SECURITY.md) covers how to report a
vulnerability, and is also the honest list of what the client does and does not
protect (the token is plaintext on disk; `--token` is visible in `ps`).

Host requirements: Rust (the pinned toolchain installs via rustup) and, for the
gateway/container recipes, Docker. `just` is optional — the recipes are thin
wrappers over `cargo`.

```bash
cargo build
cargo run -- health
just ci            # fmt-check + clippy + test
just deny          # license/advisory/ban check (needs cargo-deny)
```

Cargo aliases are defined in `.cargo/config.toml`: `cargo c` (check),
`cargo t` (test), `cargo lint` (clippy, warnings as errors).

`just canary` builds the release binary and smokes a live gateway with it
(read-only: health, a contract lookup, a positions snapshot, and a Flex sync into
a throwaway store). It is the check unit tests cannot be, and it belongs where
the gateway is reachable — a cron on that host, or your own shell.

`just contract-watch` hashes the upstream artifacts in `docs/watch/sources.txt`
(the gateway's OpenAPI document and its two contract specs) and reports what
moved, so schema drift surfaces before it reaches a release. `--update` records
the new hashes once triaged; the same check runs daily in CI and opens an issue
when something changes.

`just run health` loads `.env` before running the binary, so it talks to the
gateway `.env` names rather than the devcontainer's compose default. Plain
`target/debug/ibkr …` uses the ambient environment: source `.env` first if you
want the same thing.

### Devcontainer

`.devcontainer/` is a compose-form devcontainer: the workspace container builds
from `rust:1.94-bookworm` with clippy, rustfmt, `cargo-nextest`, `cargo-deny`
and `cargo-machete` preinstalled. `post-create.sh` loads `.env`, warms the
dependency cache, generates the dev certificates, and installs the
[OMP](https://omp.sh) coding agent (so the container has an agent available;
`~/.local/bin` is on PATH for login and non-login shells).

The gateway services are profile-gated, so opening the container is fast and
does not require the sibling repo to build:

| Profile | Service | Purpose |
|---------|---------|---------|
| `gateway` | `ibkr-gateway` | the gateway, built from `../ibkr-gateway` |
| `broker` | `ib-gateway` | headless IB Gateway (Xvfb + automated login/2FA) |

Inside the container the gateway is reached as `https://ibkr-gateway:8080`
(compose service name) using `/workspaces/ibkr.rs/.certs`. Note that the
devcontainer mounts the host Docker socket, which is root-equivalent on the
host — do not expose the container.

## Test coverage

`cargo test` covers:

* **Configuration** — flag/environment/default precedence, the all-or-nothing
  client keypair rule, certificate path resolution, truthiness parsing.
* **Transport** — the retry decision (pacing-only, missing hint, over-cap hint,
  exactly-at-cap) and error-kind mapping/rendering.
* **Wire types** — every documented payload shape (`/health`, contract details,
  symbol search, historical bars, positions, summary, PnL, orders, executions,
  cancel, Flex reports and config), enum spellings (`BUY`, `STOP_LIMIT`, `GTC`,
  `C`/`P`), camelCase field names, and omission of unset optionals.
* **Request validation** — order price requirements, bracket targets around the
  entry, positive quantities.
* **CLI surface** — parsed command forms, global flags before and after the
  subcommand, mutation classification, instrument descriptions, and a
  regression test for a positional silently shadowing the global `--token`.
* **Store** — schema application and versioning (including stepping an older
  file up and forcing the re-sync that rebuilds it), idempotent/overlapping
  syncs, level-of-detail partitioning (executions vs lots vs aggregates, cash
  detail vs summary), lot keys for rows Flex cannot key itself, ETag state,
  read-only queries, and the analysis views.
* **Rendering** — column alignment, terminal-width shrinking with elision,
  number formatting, absent-value handling, Flex close detection, enum-sentinel
  handling and the date-time projection.

**Conformance vectors** (`conformance/vectors/*.json`, run by
`cargo test --test conformance`) pin the gateway's Flex payload contract as
data: what a conforming consumer must decode, store and answer. They are the
executable form of what a statement actually does — blank enum attributes
arriving as `null`, compact dates, `C;O` closes, lot rows arriving separately,
identical lots belonging to different closes, cash `DETAIL` versus `SUMMARY`,
zero-price assignment closes, overlapping report windows. See
[`conformance/README.md`](conformance/README.md).

Endpoint behaviour is otherwise verified against a running gateway; there is no
in-repo gateway fixture yet.

## License

MIT — see [LICENSE](LICENSE).
