//! `SQLite` schema for the local trade store.
//!
//! The DDL is idempotent and additive: every statement uses `IF NOT EXISTS`,
//! so applying it to an existing database is a no-op. A version is recorded in
//! `PRAGMA user_version`; when a column or view changes, bump
//! [`SCHEMA_VERSION`] and add the change to the DDL (the store is derived data
//! — a rebuild from Flex statements is always possible — so migrations stay
//! additive rather than surgical).
//!
//! Column conventions:
//!
//! * dates are ISO `YYYY-MM-DD`, parsed out of the statement's compact
//!   `YYYYMMDD` form on the way in;
//! * money and quantities are `REAL` in the trade's own `currency`, with
//!   `fx_rate_to_base` kept alongside so BASE aggregation is possible;
//! * `realized_pnl` is the broker's figure for a closing execution
//!   (`fifoPnlRealized`), not something recomputed here.

/// Schema version recorded in `PRAGMA user_version`.
pub const SCHEMA_VERSION: i32 = 2;

/// Steps that bring an older store forward, applied in order before the DDL.
///
/// Derived rows are dropped rather than migrated, and `sync_state` is cleared
/// with them so the next `ibkr store sync` re-fetches every report: rebuilding
/// from the gateway is cheaper and less error-prone than translating a table
/// whose contents Flex cannot be asked to reproduce.
pub(super) const MIGRATIONS: &[(i32, &str)] = &[(
    2,
    "DROP TABLE IF EXISTS closed_lots; DELETE FROM sync_state;",
)];

/// Applied on every open; every statement is idempotent.
pub(super) const DDL: &str = r"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

-- One row per Flex execution (level of detail EXECUTION).
CREATE TABLE IF NOT EXISTS executions (
    transaction_id   TEXT PRIMARY KEY,
    exec_id          TEXT,
    conid            TEXT,
    symbol           TEXT,
    description      TEXT,
    asset_category   TEXT,
    currency         TEXT,
    trade_date       TEXT,
    trade_time       TEXT,
    settle_date      TEXT,
    buy_sell         TEXT,
    open_close       TEXT,
    quantity         REAL,
    price            REAL,
    proceeds         REAL,
    cost             REAL,
    commission       REAL,
    taxes            REAL,
    net_cash         REAL,
    realized_pnl     REAL,
    mtm_pnl          REAL,
    fx_rate_to_base  REAL,
    multiplier       REAL,
    strike           REAL,
    expiry           TEXT,
    put_call         TEXT,
    ib_order_id      TEXT,
    level_of_detail  TEXT,
    report           TEXT NOT NULL,
    synced_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS executions_conid ON executions (conid);
CREATE INDEX IF NOT EXISTS executions_trade_date ON executions (trade_date);

-- One row per closed lot (level of detail CLOSED_LOT). Flex keys a lot by the
-- *opening* execution and carries the close on the row, but that pair is not
-- unique: one close can split into several lots, and two lots can be
-- byte-identical while belonging to different closing executions. `lot_key` is
-- the row's own content plus its occurrence among identical rows in the
-- payload, which keeps every delivered lot and still converges when two
-- overlapping report windows describe the same lot.
CREATE TABLE IF NOT EXISTS closed_lots (
    lot_key             TEXT PRIMARY KEY,
    open_transaction_id TEXT NOT NULL,
    close_time          TEXT NOT NULL,
    close_date          TEXT,
    conid               TEXT,
    symbol              TEXT,
    description         TEXT,
    asset_category      TEXT,
    currency            TEXT,
    buy_sell            TEXT,
    quantity            REAL,
    cost                REAL,
    realized_pnl        REAL,
    multiplier          REAL,
    open_time           TEXT,
    open_date           TEXT,
    report              TEXT NOT NULL,
    synced_at           TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS closed_lots_close_date ON closed_lots (close_date);
CREATE INDEX IF NOT EXISTS closed_lots_open_tx ON closed_lots (open_transaction_id);

-- One row per cash transaction (dividends, fees, interest, transfers).
CREATE TABLE IF NOT EXISTS cash_transactions (
    transaction_id  TEXT PRIMARY KEY,
    type            TEXT,
    description     TEXT,
    amount          REAL,
    currency        TEXT,
    fx_rate_to_base REAL,
    date            TEXT,
    settle_date     TEXT,
    ex_date         TEXT,
    conid           TEXT,
    symbol          TEXT,
    report          TEXT NOT NULL,
    synced_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS cash_transactions_type ON cash_transactions (type);

-- What each report contributed, and its ETag for conditional fetches.
CREATE TABLE IF NOT EXISTS sync_state (
    report            TEXT PRIMARY KEY,
    from_date         TEXT,
    to_date           TEXT,
    etag              TEXT,
    executions        INTEGER NOT NULL DEFAULT 0,
    lots              INTEGER NOT NULL DEFAULT 0,
    cash_transactions INTEGER NOT NULL DEFAULT 0,
    skipped           INTEGER NOT NULL DEFAULT 0,
    synced_at         TEXT NOT NULL
);

-- Broker-reported realized P/L, per symbol and currency.
CREATE VIEW IF NOT EXISTS pnl_by_symbol AS
    SELECT symbol, currency, COUNT(*) AS closes, ROUND(SUM(realized_pnl), 2) AS realized_pnl
    FROM executions WHERE open_close = 'C'
    GROUP BY symbol, currency ORDER BY realized_pnl DESC;

CREATE VIEW IF NOT EXISTS pnl_by_month AS
    SELECT substr(trade_date, 1, 7) AS month, COUNT(*) AS closes,
           ROUND(SUM(realized_pnl), 2) AS realized_pnl
    FROM executions WHERE open_close = 'C'
    GROUP BY month ORDER BY month;

CREATE VIEW IF NOT EXISTS pnl_by_asset_class AS
    SELECT asset_category, COUNT(*) AS closes, ROUND(SUM(realized_pnl), 2) AS realized_pnl
    FROM executions WHERE open_close = 'C'
    GROUP BY asset_category ORDER BY realized_pnl DESC;

-- Win/loss shape and expectancy over closing executions.
CREATE VIEW IF NOT EXISTS trade_stats AS
    SELECT COUNT(*) AS closes,
           SUM(realized_pnl > 0) AS wins,
           SUM(realized_pnl < 0) AS losses,
           SUM(realized_pnl = 0) AS flat,
           ROUND(SUM(realized_pnl), 2) AS realized_pnl,
           ROUND(AVG(CASE WHEN realized_pnl > 0 THEN realized_pnl END), 2) AS average_win,
           ROUND(AVG(CASE WHEN realized_pnl < 0 THEN realized_pnl END), 2) AS average_loss
    FROM executions WHERE open_close = 'C';

-- Cash flow by type; dates are only as good as the gateway's (see issue #7).
CREATE VIEW IF NOT EXISTS cash_by_type AS
    SELECT type, currency, COUNT(*) AS entries, ROUND(SUM(amount), 2) AS amount
    FROM cash_transactions GROUP BY type, currency ORDER BY amount;

-- Net position per contract derived from executions. Only trustworthy when
-- `first_indicator` is `O`: a contract whose earliest synced row is a *close*
-- was already open when the window started, so the running sum is a fragment
-- of its history and can look open when the broker holds nothing.
CREATE VIEW IF NOT EXISTS open_positions AS
    SELECT conid, symbol, currency, ROUND(SUM(quantity), 6) AS position,
           MIN(trade_date) AS first_seen,
           (SELECT earliest.open_close FROM executions AS earliest
             WHERE earliest.conid = positions.conid
             ORDER BY earliest.trade_date, COALESCE(earliest.trade_time, ''),
                      earliest.transaction_id
             LIMIT 1) AS first_indicator
    FROM executions AS positions
    GROUP BY conid HAVING ABS(SUM(quantity)) > 1e-9;

-- Exact round trips: opening execution, close, matched quantity, cost basis and
-- realized P/L. Empty until the gateway returns CLOSED_LOT rows.
CREATE VIEW IF NOT EXISTS round_trips AS
    SELECT open_transaction_id, open_date, close_date, conid, symbol, currency,
           quantity, cost, realized_pnl
    FROM closed_lots;
";
