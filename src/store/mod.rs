//! Local `SQLite` store for Flex statement data.
//!
//! Flex statements are the only complete transaction history the gateway can
//! produce (see `ljuti/ibkr-gateway#3`), but they are paged by query window and
//! served one report at a time. This module keeps them in a single `SQLite` file
//! so trade performance can be queried with SQL: `ibkr store sync` fetches
//! reports and upserts them, `ibkr store query` reads the result, and
//! [`schema`](self) documents the views that answer the usual questions.
//!
//! Ingest is idempotent and overlap-safe: rows are keyed by their IB
//! transaction id, so syncing a 30-day and a 365-day report that cover the same
//! execution stores it once. Rows are split by `levelOfDetail` — executions,
//! closed lots, and aggregates are kept apart, because Flex puts several levels
//! of detail in one array and summing across them would double count.
//!
//! The store is derived data. It can always be rebuilt from the gateway, and
//! the schema is applied additively on open (see [`schema::SCHEMA_VERSION`]).

mod schema;
mod sync;

pub use schema::SCHEMA_VERSION;
pub use sync::SyncStats;

/// Outcome of syncing one report.
#[derive(Debug, Clone)]
pub enum SyncOutcome {
    /// Fetched and upserted.
    Synced(SyncStats),
    /// The gateway answered `304`: the stored rows already match the statement.
    NotModified {
        /// Report name.
        report: String,
        /// `ETag` the gateway confirmed.
        etag: Option<String>,
    },
    /// This report failed; the sweep continued with the others.
    Failed {
        /// Report name.
        report: String,
        /// Rendered failure, as the command would print it.
        message: String,
    },
}

/// What a synced report can support, derived from what it delivered.
///
/// Flex statements only carry what the query asked for: a query without the
/// Closed Lots level has no exact round trips, and one without Cash
/// Transactions has no cash movements. Saying so where the sync finishes beats
/// a reader discovering empty views later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCapabilities {
    /// Closed-lot rows arrived, so `round_trips` pairs exact lots.
    pub round_trips: bool,
    /// Cash movements (`DETAIL` rows) arrived.
    pub cash_movements: bool,
    /// Cash rows carry an event date.
    pub cash_dates: bool,
}

impl SyncCapabilities {
    /// Capabilities of one synced report, and why any are missing.
    pub(super) fn of(stats: &SyncStats) -> (Self, Option<String>) {
        let capabilities = Self {
            round_trips: stats.lots > 0,
            cash_movements: stats.cash_transactions > 0,
            cash_dates: stats.cash_dated > 0,
        };
        let mut missing = Vec::new();
        if stats.lots == 0 && stats.executions > 0 {
            missing.push(
                "no closed-lot rows: add the Closed Lots level to the Flex query for exact round trips",
            );
        }
        if stats.cash_transactions == 0 && stats.skipped > 0 {
            missing.push("no cash movements: add the Cash Transactions section to the Flex query");
        }
        let hint = (!missing.is_empty()).then(|| missing.join("; "));
        (capabilities, hint)
    }
}

use std::fs;
use std::path::Path;

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::error::{Error, Result};
use crate::types::FlexReportResponse;

/// Rows of a query result, cells left in their `SQLite` types.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Column names, in select order.
    pub columns: Vec<String>,
    /// One entry per row, aligned with [`Self::columns`].
    pub rows: Vec<Vec<Value>>,
}

/// One report's recorded sync state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportStatus {
    /// Report name.
    pub report: String,
    /// Statement window start.
    pub from_date: Option<String>,
    /// Statement window end.
    pub to_date: Option<String>,
    /// `ETag` from the last fetch.
    pub etag: Option<String>,
    /// Executions the report contributed.
    pub executions: i64,
    /// Closed lots it contributed.
    pub lots: i64,
    /// Cash movements it contributed.
    pub cash_transactions: i64,
    /// Rows it contributed that the store skips (aggregates, cash summaries).
    pub skipped: i64,
    /// When it was last synced.
    pub synced_at: String,
}

/// What the store holds, and what that is enough to answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatus {
    /// Every report that has been synced.
    pub reports: Vec<ReportStatus>,
    /// Execution rows in the store (all reports combined).
    pub executions: i64,
    /// Closed-lot rows.
    pub lots: i64,
    /// Cash movement rows.
    pub cash_transactions: i64,
    /// Cash rows carrying an event date.
    pub cash_dated: i64,
    /// Which analyses the stored data supports.
    pub capabilities: SyncCapabilities,
}

/// A table or view in the store.
#[derive(Debug, Clone)]
pub struct SchemaEntry {
    /// Table or view name.
    pub name: String,
    /// `table` or `view`.
    pub kind: String,
    /// Column names, in declaration order.
    pub columns: Vec<String>,
}

/// Handle on the store database file.
#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Open the store, creating the file and schema when absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the file is not a usable database, and
    /// [`Error::Io`] when its directory cannot be created.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        let version: i32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > schema::SCHEMA_VERSION {
            return Err(Error::Store(format!(
                "{} was written by a newer ibkr (schema {version}, this build knows {})",
                path.display(),
                schema::SCHEMA_VERSION
            )));
        }
        // A fresh file has nothing to carry forward; an existing one is stepped
        // up before the current DDL recreates whatever the migration dropped.
        if version > 0 && version < schema::SCHEMA_VERSION {
            for (_, sql) in schema::MIGRATIONS
                .iter()
                .filter(|(target, _)| *target > version)
            {
                connection.execute_batch(sql)?;
            }
        }
        connection.execute_batch(schema::DDL)?;
        if version != schema::SCHEMA_VERSION {
            connection
                .execute_batch(&format!("PRAGMA user_version = {}", schema::SCHEMA_VERSION))?;
        }
        Ok(Self { connection })
    }

    /// Open an existing store read-only, so a query cannot modify anything.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] when the file does not exist — an empty store
    /// answers no questions, and creating one behind a `SELECT` would only hide
    /// the missing sync.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(Error::Invalid(format!(
                "no store at {} — run `ibkr store sync` first",
                path.display()
            )));
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { connection })
    }

    /// Upsert one fetched report.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the write fails; the insert runs in a
    /// single transaction, so a failure leaves the store untouched.
    pub fn upsert_report(
        &mut self,
        report: &str,
        etag: Option<&str>,
        payload: &FlexReportResponse,
    ) -> Result<SyncStats> {
        sync::upsert(&mut self.connection, report, etag, payload)
    }

    /// `ETag` recorded for `report`, if it has ever been synced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the read fails.
    pub fn last_etag(&self, report: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension as _;

        let etag = self
            .connection
            .query_row(
                "SELECT etag FROM sync_state WHERE report = ?1",
                [report],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(etag)
    }

    /// Sync state per report, plus what the stored rows can answer.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the reads fail.
    pub fn status(&self) -> Result<StoreStatus> {
        let mut statement = self.connection.prepare(
            "SELECT report, from_date, to_date, etag, executions, lots, cash_transactions, \
                    skipped, synced_at \
             FROM sync_state ORDER BY report",
        )?;
        let reports: Vec<ReportStatus> = statement
            .query_map([], |row| {
                Ok(ReportStatus {
                    report: row.get(0)?,
                    from_date: row.get(1)?,
                    to_date: row.get(2)?,
                    etag: row.get(3)?,
                    executions: row.get(4)?,
                    lots: row.get(5)?,
                    cash_transactions: row.get(6)?,
                    skipped: row.get(7)?,
                    synced_at: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);

        let counts = self.connection.query_row(
            "SELECT (SELECT COUNT(*) FROM executions), (SELECT COUNT(*) FROM closed_lots), \
                    (SELECT COUNT(*) FROM cash_transactions), \
                    (SELECT COUNT(*) FROM cash_transactions WHERE date IS NOT NULL)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let (executions, lots, cash_transactions, cash_dated) = counts;
        Ok(StoreStatus {
            reports,
            executions,
            lots,
            cash_transactions,
            cash_dated,
            capabilities: SyncCapabilities {
                round_trips: lots > 0,
                cash_movements: cash_transactions > 0,
                cash_dates: cash_dated > 0,
            },
        })
    }

    /// Run a read-only SQL statement and return its rows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when `SQLite` rejects the statement.
    pub fn query(&self, sql: &str) -> Result<QueryResult> {
        let mut statement = self.connection.prepare(sql)?;
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let width = columns.len();
        let mut rows = Vec::new();
        let mut cursor = statement.query([])?;
        while let Some(row) = cursor.next()? {
            let mut values = Vec::with_capacity(width);
            for index in 0..width {
                values.push(row.get::<_, Value>(index)?);
            }
            rows.push(values);
        }
        Ok(QueryResult { columns, rows })
    }

    /// Tables and views, with their columns.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Store`] when the catalogue cannot be read.
    pub fn schema(&self) -> Result<Vec<SchemaEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT name, type FROM sqlite_master \
             WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' \
             ORDER BY type, name",
        )?;
        let objects: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        drop(statement);

        let mut entries = Vec::with_capacity(objects.len());
        for (name, kind) in objects {
            let quoted = name.replace('"', "\"\"");
            let mut columns_statement = self
                .connection
                .prepare(&format!("PRAGMA table_info(\"{quoted}\")"))?;
            let columns: Vec<String> = columns_statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<_>>()?;
            entries.push(SchemaEntry {
                name,
                kind,
                columns,
            });
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CashTransaction, FlexReportResponse, Trade};

    fn trade(transaction_id: &str) -> Trade {
        Trade {
            transaction_id: Some(transaction_id.to_owned()),
            account_id: Some("U1".to_owned()),
            conid: Some("265598".to_owned()),
            symbol: Some("AAPL".to_owned()),
            description: Some("APPLE INC".to_owned()),
            asset_category: Some("STK".to_owned()),
            buy_sell: Some("BUY".to_owned()),
            trade_date: Some("2026-03-02".to_owned()),
            settle_date: Some("2026-03-04".to_owned()),
            quantity: Some(100.0),
            price: Some(210.5),
            proceeds: Some(-21050.0),
            cost: Some(21050.35),
            commission: Some(-0.35),
            fifo_pnl_realized: Some(0.0),
            currency: Some("USD".to_owned()),
            exchange: None,
            underlying_symbol: None,
            underlying_conid: None,
            multiplier: Some(1.0),
            strike: None,
            expiry: None,
            put_call: None,
            open_close: Some("O".to_owned()),
            open_date_time: None,
            trade_time: Some("20260302;100000".to_owned()),
            level_of_detail: Some("EXECUTION".to_owned()),
            taxes: Some(0.0),
            net_cash: Some(-21050.35),
            mtm_pnl: None,
            fx_rate_to_base: Some(1.25),
            ib_order_id: None,
            exec_id: Some(format!("exec-{transaction_id}")),
        }
    }

    fn payload(trades: Vec<Trade>, cash: Vec<CashTransaction>) -> FlexReportResponse {
        payload_with_lots(trades, Vec::new(), cash)
    }

    fn payload_with_lots(
        trades: Vec<Trade>,
        lots: Vec<Trade>,
        cash: Vec<CashTransaction>,
    ) -> FlexReportResponse {
        FlexReportResponse {
            account_id: Some("U1".to_owned()),
            from_date: Some("2026-03-01".to_owned()),
            to_date: Some("2026-03-31".to_owned()),
            trades,
            lots,
            cash_transactions: cash,
        }
    }

    fn cash(transaction_id: &str, amount: f64) -> CashTransaction {
        CashTransaction {
            transaction_id: Some(transaction_id.to_owned()),
            account_id: Some("U1".to_owned()),
            transaction_type: Some("Dividends".to_owned()),
            description: Some("APPLE INC CASH DIVIDEND".to_owned()),
            amount: Some(amount),
            currency: Some("USD".to_owned()),
            fx_rate_to_base: Some(1.25),
            date: Some("2026-03-15".to_owned()),
            settle_date: Some("2026-03-15".to_owned()),
            ex_date: None,
            conid: Some("265598".to_owned()),
            symbol: Some("AAPL".to_owned()),
            level_of_detail: Some("DETAIL".to_owned()),
        }
    }

    /// A store in its own temporary directory, so the WAL and SHM sidecars
    /// disappear with it instead of lingering next to an unlinked file.
    fn temp_store(name: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("ibkr-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory.join("trades.db")
    }

    /// Remove the store and everything `SQLite` kept beside it.
    fn cleanup(path: &Path) {
        if let Some(directory) = path.parent() {
            let _ = fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn sync_is_idempotent_and_dedupes_overlapping_reports() {
        let path = temp_store("idempotent");
        let mut store = Store::open(&path).unwrap();
        let report = payload(vec![trade("t1")], Vec::new());

        let first = store
            .upsert_report("transactions_30d", Some("\"e1\""), &report)
            .unwrap();
        assert_eq!(first.executions, 1);
        assert_eq!(first.skipped, 0);
        assert_eq!(
            store.last_etag("transactions_30d").unwrap().as_deref(),
            Some("\"e1\"")
        );

        // Same rows again — a re-sync, and the same execution inside a second,
        // wider report: one row either way.
        store
            .upsert_report("transactions_30d", Some("\"e1\""), &report)
            .unwrap();
        let second = store
            .upsert_report("last_365_days", Some("\"e2\""), &report)
            .unwrap();
        assert_eq!(second.executions, 1);

        let count = store.query("SELECT COUNT(*) FROM executions").unwrap();
        assert_eq!(count.rows[0][0], Value::Integer(1));
        let reports = store.query("SELECT COUNT(*) FROM sync_state").unwrap();
        assert_eq!(reports.rows[0][0], Value::Integer(2));
        drop(store);
        cleanup(&path);
        assert!(!path.exists(), "the store file must be gone");
    }

    #[test]
    fn levels_of_detail_are_kept_apart() {
        let path = temp_store("levels");
        let mut store = Store::open(&path).unwrap();
        let mut execution = trade("e1");
        let mut lot = trade("open-1");
        lot.level_of_detail = Some("CLOSED_LOT".to_owned());
        lot.open_date_time = Some("20260101;093000".to_owned());
        lot.trade_time = Some("20260302;100000".to_owned());
        let mut aggregate = trade("agg-1");
        aggregate.level_of_detail = Some("SYMBOL_SUMMARY".to_owned());
        execution.level_of_detail = None; // older gateways omit the field

        let stats = store
            .upsert_report(
                "r",
                None,
                &payload(vec![execution, lot, aggregate], Vec::new()),
            )
            .unwrap();
        assert_eq!(stats.executions, 1);
        assert_eq!(stats.lots, 1);
        assert_eq!(stats.skipped, 1);

        // The aggregate must not be summed anywhere: one execution, one lot.
        let executions = store.query("SELECT COUNT(*) FROM executions").unwrap();
        assert_eq!(executions.rows[0][0], Value::Integer(1));
        let lots = store.query("SELECT COUNT(*) FROM round_trips").unwrap();
        assert_eq!(lots.rows[0][0], Value::Integer(1));
        cleanup(&path);
    }

    #[test]
    fn a_row_without_a_transaction_id_is_skipped_not_guessed() {
        let path = temp_store("no-id");
        let mut store = Store::open(&path).unwrap();
        let mut anonymous = trade("ignored");
        anonymous.transaction_id = None;
        let stats = store
            .upsert_report("r", None, &payload(vec![anonymous], Vec::new()))
            .unwrap();
        assert_eq!(stats.executions, 0);
        assert_eq!(stats.skipped, 1);
        cleanup(&path);
    }

    #[test]
    fn views_answer_the_analysis_questions() {
        let path = temp_store("views");
        let mut store = Store::open(&path).unwrap();
        // Open 100 AAPL, close 100 at +500 realized (broker figure).
        let mut open = trade("o1");
        open.open_close = Some("O".to_owned());
        let mut close = trade("c1");
        close.open_close = Some("C".to_owned());
        close.buy_sell = Some("SELL".to_owned());
        close.quantity = Some(-100.0);
        close.price = Some(215.5);
        close.fifo_pnl_realized = Some(500.0);
        let mut dividend = cash("div-1", 42.5);
        dividend.transaction_type = Some("Dividends".to_owned());
        dividend.amount = Some(42.5);

        store
            .upsert_report("r", None, &payload(vec![open, close], vec![dividend]))
            .unwrap();

        let by_symbol = store
            .query("SELECT symbol, realized_pnl FROM pnl_by_symbol")
            .unwrap();
        assert_eq!(by_symbol.rows[0][1], Value::Real(500.0));
        let stats = store
            .query("SELECT closes, wins, losses, realized_pnl FROM trade_stats")
            .unwrap();
        assert_eq!(stats.rows[0][0], Value::Integer(1));
        assert_eq!(stats.rows[0][1], Value::Integer(1));
        assert_eq!(stats.rows[0][3], Value::Real(500.0));
        let cash = store
            .query("SELECT type, amount FROM cash_by_type")
            .unwrap();
        assert_eq!(cash.rows[0][0], Value::Text("Dividends".to_owned()));
        assert_eq!(cash.rows[0][1], Value::Real(42.5));
        // The round trip is closed, so nothing is left open.
        let open_positions = store.query("SELECT COUNT(*) FROM open_positions").unwrap();
        assert_eq!(open_positions.rows[0][0], Value::Integer(0));
        cleanup(&path);
    }

    #[test]
    fn schema_is_listed_and_reapplied_idempotently() {
        let path = temp_store("schema");
        let store = Store::open(&path).unwrap();
        let entries = store.schema().unwrap();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        for expected in [
            "executions",
            "closed_lots",
            "cash_transactions",
            "sync_state",
            "pnl_by_symbol",
            "pnl_by_month",
            "pnl_by_asset_class",
            "trade_stats",
            "cash_by_type",
            "open_positions",
            "round_trips",
        ] {
            assert!(names.contains(&expected), "missing {expected} in {names:?}");
        }
        let executions = entries
            .iter()
            .find(|entry| entry.name == "executions")
            .unwrap();
        assert!(executions.columns.contains(&"realized_pnl".to_owned()));
        assert!(executions.columns.contains(&"open_close".to_owned()));
        drop(store);

        // Reopening applies the same DDL without error and keeps the version.
        let reopened = Store::open(&path).unwrap();
        let version = reopened.query("PRAGMA user_version").unwrap();
        assert_eq!(
            version.rows[0][0],
            Value::Integer(i64::from(SCHEMA_VERSION))
        );
        cleanup(&path);
    }

    #[test]
    fn an_older_store_is_stepped_up_and_asks_to_be_resynced() {
        let path = temp_store("migration");
        {
            let mut store = Store::open(&path).unwrap();
            store
                .upsert_report("r", Some("\"e\""), &payload(vec![trade("t1")], Vec::new()))
                .unwrap();
            // As if the file had been written by the previous schema, whose
            // closed-lot key could not represent every delivered lot.
            store.query("PRAGMA user_version = 1").unwrap();
        }

        let store = Store::open(&path).unwrap();
        let version = store.query("PRAGMA user_version").unwrap();
        assert_eq!(
            version.rows[0][0],
            Value::Integer(i64::from(SCHEMA_VERSION))
        );
        // Derived lots are dropped, and the reports that produced them are
        // marked for re-fetch rather than left answering 304.
        assert_eq!(
            store
                .query("SELECT COUNT(*) FROM round_trips")
                .unwrap()
                .rows[0][0],
            Value::Integer(0)
        );
        assert_eq!(
            store.query("SELECT COUNT(*) FROM sync_state").unwrap().rows[0][0],
            Value::Integer(0),
            "a migration must force a re-sync instead of trusting stale ETags"
        );
        // Executions survive: their key never changed.
        assert_eq!(
            store.query("SELECT COUNT(*) FROM executions").unwrap().rows[0][0],
            Value::Integer(1)
        );
        drop(store);
        cleanup(&path);
    }

    #[test]
    fn status_reports_what_the_stored_rows_can_answer() {
        let path = temp_store("status");
        let mut store = Store::open(&path).unwrap();

        // A report with executions only: nothing pairs, no cash.
        let mut execution = trade("e1");
        execution.open_close = Some("C".to_owned());
        store
            .upsert_report("bare", None, &payload(vec![execution], Vec::new()))
            .unwrap();
        let status = store.status().unwrap();
        assert_eq!(status.reports.len(), 1);
        assert_eq!(status.reports[0].report, "bare");
        assert_eq!(status.executions, 1);
        assert_eq!(status.lots, 0);
        assert!(!status.capabilities.round_trips);
        assert!(!status.capabilities.cash_movements);

        // Now one with lots and a dated cash movement.
        let mut lot = trade("open-1");
        lot.level_of_detail = Some("CLOSED_LOT".to_owned());
        lot.open_date_time = Some("20260807;120536".to_owned());
        lot.trade_time = Some("20260828;123310".to_owned());
        lot.trade_date = Some("2026-08-28".to_owned());
        store
            .upsert_report(
                "full",
                None,
                &payload_with_lots(Vec::new(), vec![lot], vec![cash("c1", 42.5)]),
            )
            .unwrap();
        let status = store.status().unwrap();
        assert_eq!(status.reports.len(), 2);
        assert_eq!(status.lots, 1);
        assert_eq!(status.cash_transactions, 1);
        assert_eq!(status.cash_dated, 1);
        assert!(status.capabilities.round_trips);
        assert!(status.capabilities.cash_movements);
        assert!(status.capabilities.cash_dates);

        // An undated cash row is stored but not usable as a time series.
        let mut undated = cash("c2", 1.0);
        undated.date = None;
        store
            .upsert_report(
                "undated",
                None,
                &payload_with_lots(Vec::new(), Vec::new(), vec![undated]),
            )
            .unwrap();
        let status = store.status().unwrap();
        assert_eq!(status.cash_transactions, 2);
        assert_eq!(status.cash_dated, 1);
        assert!(status.capabilities.cash_dates);
        drop(store);
        cleanup(&path);
    }

    #[test]
    fn capabilities_of_a_report_name_the_missing_sections() {
        let mut stats = SyncStats {
            report: "r".to_owned(),
            from_date: None,
            to_date: None,
            executions: 10,
            lots: 0,
            cash_transactions: 0,
            skipped: 4,
            cash_dated: 0,
            etag: None,
        };
        let (capabilities, hint) = SyncCapabilities::of(&stats);
        assert!(!capabilities.round_trips);
        let hint = hint.expect("a report without lots or cash has something to say");
        assert!(hint.contains("Closed Lots"), "{hint}");
        assert!(hint.contains("Cash Transactions"), "{hint}");

        stats.lots = 3;
        stats.cash_transactions = 2;
        stats.cash_dated = 2;
        let (capabilities, hint) = SyncCapabilities::of(&stats);
        assert_eq!(
            capabilities,
            SyncCapabilities {
                round_trips: true,
                cash_movements: true,
                cash_dates: true
            }
        );
        assert!(hint.is_none(), "{hint:?}");
    }

    #[test]
    fn a_read_only_store_refuses_to_write() {
        let path = temp_store("readonly");
        drop(Store::open(&path).unwrap());
        let store = Store::open_read_only(&path).unwrap();
        assert!(store.query("SELECT COUNT(*) FROM executions").is_ok());
        let write = store.query("INSERT INTO sync_state (report, synced_at) VALUES ('x', 'y')");
        assert!(write.is_err(), "a read-only handle must not write");

        let missing = Store::open_read_only(Path::new("/nonexistent/ibkr-store.db"));
        assert!(matches!(missing, Err(Error::Invalid(_))));
        cleanup(&path);
    }

    #[test]
    fn query_keeps_sqlite_types() {
        let path = temp_store("types");
        let store = Store::open(&path).unwrap();
        let result = store
            .query("SELECT 1 AS n, 1.5 AS f, 'x' AS t, NULL AS z")
            .unwrap();
        assert_eq!(result.columns, vec!["n", "f", "t", "z"]);
        assert_eq!(
            result.rows[0],
            vec![
                Value::Integer(1),
                Value::Real(1.5),
                Value::Text("x".to_owned()),
                Value::Null
            ]
        );
        cleanup(&path);
    }
}
