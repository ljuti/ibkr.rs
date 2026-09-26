//! Upserting one fetched Flex report into the store.

use std::collections::HashMap;

use rusqlite::{Connection, named_params};
use serde::Serialize;

use crate::error::Result;
use crate::types::{FlexReportResponse, Trade, flex_date};

/// What one report contributed to the store.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStats {
    /// Report name the payload was fetched under.
    pub report: String,
    /// Statement window start (the query's Period setting is authoritative).
    pub from_date: Option<String>,
    /// Statement window end.
    pub to_date: Option<String>,
    /// Executions upserted.
    pub executions: usize,
    /// Closed lots upserted.
    pub lots: usize,
    /// Cash transactions upserted.
    pub cash_transactions: usize,
    /// Rows deliberately left out: aggregates, or rows with no transaction id.
    pub skipped: usize,
    /// Cash movements that arrived with an event date.
    pub cash_dated: usize,
    /// `ETag` the payload arrived with.
    pub etag: Option<String>,
}

/// Which store table a Flex trade row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Execution,
    Lot,
    Aggregate,
}

/// Flex puts several levels of detail in one array. Executions and closed lots
/// are different records of the same economics, and the summary levels are
/// aggregates of both — so they must not land in the same table.
fn classify(trade: &Trade) -> Level {
    match trade.level_of_detail.as_deref().map(str::trim) {
        // Older gateways do not send the field; every row is an execution.
        None | Some("") => Level::Execution,
        Some(level) if level.eq_ignore_ascii_case("EXECUTION") => Level::Execution,
        Some(level)
            if level.eq_ignore_ascii_case("CLOSED_LOT") || level.eq_ignore_ascii_case("LOT") =>
        {
            Level::Lot
        }
        Some(_) => Level::Aggregate,
    }
}

const EXECUTION_INSERT: &str = "
    INSERT OR REPLACE INTO executions (
        transaction_id, exec_id, conid, symbol, description, asset_category, currency,
        trade_date, trade_time, settle_date, buy_sell, open_close, quantity, price, proceeds,
        cost, commission, taxes, net_cash, realized_pnl, mtm_pnl, fx_rate_to_base, multiplier,
        strike, expiry, put_call, ib_order_id, level_of_detail, report, synced_at
    ) VALUES (
        :transaction_id, :exec_id, :conid, :symbol, :description, :asset_category, :currency,
        :trade_date, :trade_time, :settle_date, :buy_sell, :open_close, :quantity, :price,
        :proceeds, :cost, :commission, :taxes, :net_cash, :realized_pnl, :mtm_pnl,
        :fx_rate_to_base, :multiplier, :strike, :expiry, :put_call, :ib_order_id,
        :level_of_detail, :report, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    )";

const LOT_INSERT: &str = "
    INSERT OR REPLACE INTO closed_lots (
        lot_key, open_transaction_id, close_time, close_date, conid, symbol, description,
        asset_category, currency, buy_sell, quantity, cost, realized_pnl, multiplier, open_time,
        open_date, report, synced_at
    ) VALUES (
        :lot_key, :open_transaction_id, :close_time, :close_date, :conid, :symbol, :description,
        :asset_category, :currency, :buy_sell, :quantity, :cost, :realized_pnl, :multiplier,
        :open_time, :open_date, :report, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    )";

const CASH_INSERT: &str = "
    INSERT OR REPLACE INTO cash_transactions (
        transaction_id, type, description, amount, currency, fx_rate_to_base, date,
        settle_date, ex_date, conid, symbol, report, synced_at
    ) VALUES (
        :transaction_id, :type, :description, :amount, :currency, :fx_rate_to_base, :date,
        :settle_date, :ex_date, :conid, :symbol, :report,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    )";

const SYNC_STATE_UPSERT: &str = "
    INSERT INTO sync_state (
        report, from_date, to_date, etag, executions, lots, cash_transactions, skipped, synced_at
    ) VALUES (
        :report, :from_date, :to_date, :etag, :executions, :lots, :cash_transactions, :skipped,
        strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
    )
    ON CONFLICT (report) DO UPDATE SET
        from_date = excluded.from_date,
        to_date = excluded.to_date,
        etag = excluded.etag,
        executions = excluded.executions,
        lots = excluded.lots,
        cash_transactions = excluded.cash_transactions,
        skipped = excluded.skipped,
        synced_at = excluded.synced_at";

/// `SQLite` stores integers as `i64`; row counts never approach the limit.
fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Upsert a report's rows, returning what landed.
///
/// # Errors
///
/// Returns [`Error::Store`](crate::error::Error::Store) when a statement fails;
/// the whole upsert runs in one transaction.
pub(super) fn upsert(
    connection: &mut Connection,
    report: &str,
    etag: Option<&str>,
    payload: &FlexReportResponse,
) -> Result<SyncStats> {
    let mut stats = SyncStats {
        report: report.to_owned(),
        from_date: payload.from_date.clone(),
        to_date: payload.to_date.clone(),
        executions: 0,
        lots: 0,
        cash_transactions: 0,
        skipped: 0,
        cash_dated: 0,
        etag: etag.map(str::to_owned),
    };

    let transaction = connection.transaction()?;
    // A sync replaces whatever that report contributed before: rows a
    // regenerated statement no longer contains would otherwise linger and be
    // counted twice. Rows another report also contributed stay until their own
    // report is synced.
    for table in ["executions", "closed_lots", "cash_transactions"] {
        transaction.execute(&format!("DELETE FROM {table} WHERE report = ?1"), [report])?;
    }
    insert_trades(&transaction, report, payload, &mut stats)?;
    insert_lots(&transaction, report, &payload.lots, &mut stats)?;
    insert_cash(&transaction, report, payload, &mut stats)?;
    record_sync_state(&transaction, &stats)?;
    transaction.commit()?;
    Ok(stats)
}

/// Closed-lot rows, which the gateway returns in their own array precisely so
/// that they are not summed with the executions they belong to.
fn insert_lots(
    transaction: &rusqlite::Transaction<'_>,
    report: &str,
    lots: &[Trade],
    stats: &mut SyncStats,
) -> Result<()> {
    let mut lot_insert = transaction.prepare(LOT_INSERT)?;
    let mut seen: HashMap<String, u32> = HashMap::new();
    for trade in lots {
        let Some(open_transaction_id) = trade
            .transaction_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            stats.skipped += 1;
            continue;
        };
        let content = lot_content(trade);
        let ordinal = seen.entry(content.clone()).or_insert(0);
        *ordinal += 1;
        lot_insert.execute(named_params! {
            ":lot_key": format!("{content}#{ordinal}"),
            ":open_transaction_id": open_transaction_id,
            ":close_time": trade.trade_time.as_deref().unwrap_or(""),
            ":close_date": as_date(trade.trade_date.as_deref()),
            ":conid": trade.conid,
            ":symbol": trade.symbol,
            ":description": trade.description,
            ":asset_category": trade.asset_category,
            ":currency": trade.currency,
            ":buy_sell": trade.buy_sell,
            ":quantity": trade.quantity,
            ":cost": trade.cost,
            ":realized_pnl": trade.fifo_pnl_realized,
            ":multiplier": trade.multiplier,
            ":open_time": trade.open_date_time,
            ":open_date": as_date(trade.open_date_time.as_deref()),
            ":report": report,
        })?;
        stats.lots += 1;
    }
    Ok(())
}

/// A lot's content, which together with its ordinal forms `lot_key`.
///
/// Two lots can be byte-identical — each belongs to a different closing
/// execution — so content alone would collapse them and lose the second one's
/// realized P/L.
fn lot_content(trade: &Trade) -> String {
    format!(
        "{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        trade.transaction_id.as_deref().unwrap_or(""),
        trade.trade_time.as_deref().unwrap_or(""),
        trade.conid.as_deref().unwrap_or(""),
        trade.quantity,
        trade.cost,
        trade.fifo_pnl_realized,
        trade.open_date_time.as_deref(),
        trade.buy_sell.as_deref(),
        trade.trade_date.as_deref(),
    )
}

/// Executions and closed lots, split by level of detail.
fn insert_trades(
    transaction: &rusqlite::Transaction<'_>,
    report: &str,
    payload: &FlexReportResponse,
    stats: &mut SyncStats,
) -> Result<()> {
    let mut execution_insert = transaction.prepare(EXECUTION_INSERT)?;
    let mut lot_insert = transaction.prepare(LOT_INSERT)?;
    for trade in &payload.trades {
        // The transaction id is the store's identity for a row; without it
        // there is nothing to dedupe on, so the row is counted, not guessed.
        let Some(transaction_id) = trade
            .transaction_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            stats.skipped += 1;
            continue;
        };
        match classify(trade) {
            Level::Execution => {
                execution_insert.execute(named_params! {
                    ":transaction_id": transaction_id,
                    ":exec_id": trade.exec_id,
                    ":conid": trade.conid,
                    ":symbol": trade.symbol,
                    ":description": trade.description,
                    ":asset_category": trade.asset_category,
                    ":currency": trade.currency,
                    ":trade_date": as_date(trade.trade_date.as_deref()),
                    ":trade_time": trade.trade_time,
                    ":settle_date": as_date(trade.settle_date.as_deref()),
                    ":buy_sell": trade.buy_sell,
                    ":open_close": trade.open_close,
                    ":quantity": trade.quantity,
                    ":price": trade.price,
                    ":proceeds": trade.proceeds,
                    ":cost": trade.cost,
                    ":commission": trade.commission,
                    ":taxes": trade.taxes,
                    ":net_cash": trade.net_cash,
                    ":realized_pnl": trade.fifo_pnl_realized,
                    ":mtm_pnl": trade.mtm_pnl,
                    ":fx_rate_to_base": trade.fx_rate_to_base,
                    ":multiplier": trade.multiplier,
                    ":strike": trade.strike,
                    ":expiry": as_date(trade.expiry.as_deref()),
                    ":put_call": trade.put_call,
                    ":ib_order_id": trade.ib_order_id,
                    ":level_of_detail": trade.level_of_detail,
                    ":report": report,
                })?;
                stats.executions += 1;
            }
            Level::Lot => {
                // Flex keys a lot by the *opening* execution and carries the
                // close (and the realized result) on the row.
                lot_insert.execute(named_params! {
                    ":open_transaction_id": transaction_id,
                    ":close_time": trade.trade_time.as_deref().unwrap_or(""),
                    ":close_date": as_date(trade.trade_date.as_deref()),
                    ":conid": trade.conid,
                    ":symbol": trade.symbol,
                    ":description": trade.description,
                    ":asset_category": trade.asset_category,
                    ":currency": trade.currency,
                    ":buy_sell": trade.buy_sell,
                    ":quantity": trade.quantity,
                    ":cost": trade.cost,
                    ":realized_pnl": trade.fifo_pnl_realized,
                    ":multiplier": trade.multiplier,
                    ":open_time": trade.open_date_time,
                    ":open_date": as_date(trade.open_date_time.as_deref()),
                    ":report": report,
                })?;
                stats.lots += 1;
            }
            // ORDER, SYMBOL_SUMMARY, ASSET_SUMMARY: aggregates of the same
            // executions, deliberately not stored.
            Level::Aggregate => stats.skipped += 1,
        }
    }
    Ok(())
}

/// Cash rows are multi-level too: `DETAIL` rows are the movements, and
/// `SUMMARY` rows aggregate the same money per report date. Only detail rows
/// are stored (see [`CashTransaction::is_detail`]).
fn insert_cash(
    transaction: &rusqlite::Transaction<'_>,
    report: &str,
    payload: &FlexReportResponse,
    stats: &mut SyncStats,
) -> Result<()> {
    let mut cash_insert = transaction.prepare(CASH_INSERT)?;
    for cash in &payload.cash_transactions {
        let identified = cash
            .transaction_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        // A summary row, or a detail row with nothing to dedupe on: counted,
        // not guessed at.
        let Some(transaction_id) = identified.filter(|_| cash.is_detail()) else {
            stats.skipped += 1;
            continue;
        };
        if cash
            .date
            .as_deref()
            .is_some_and(|date| !date.trim().is_empty())
        {
            stats.cash_dated += 1;
        }
        cash_insert.execute(named_params! {
            ":transaction_id": transaction_id,
            ":type": cash.transaction_type,
            ":description": cash.description,
            ":amount": cash.amount,
            ":currency": cash.currency,
            ":fx_rate_to_base": cash.fx_rate_to_base,
            ":date": as_date(cash.date.as_deref()),
            ":settle_date": as_date(cash.settle_date.as_deref()),
            ":ex_date": as_date(cash.ex_date.as_deref()),
            ":conid": cash.conid,
            ":symbol": cash.symbol,
            ":report": report,
        })?;
        stats.cash_transactions += 1;
    }
    Ok(())
}

/// Remember what the report contributed, and its `ETag`.
fn record_sync_state(transaction: &rusqlite::Transaction<'_>, stats: &SyncStats) -> Result<()> {
    transaction.execute(
        SYNC_STATE_UPSERT,
        named_params! {
            ":report": stats.report,
            ":from_date": stats.from_date,
            ":to_date": stats.to_date,
            ":etag": stats.etag,
            ":executions": count(stats.executions),
            ":lots": count(stats.lots),
            ":cash_transactions": count(stats.cash_transactions),
            ":skipped": count(stats.skipped),
        },
    )?;
    Ok(())
}

/// Normalize a statement date to ISO, keeping `None` as `None`.
fn as_date(value: Option<&str>) -> Option<String> {
    value.and_then(flex_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::store::Store;
    use crate::types::{CashTransaction, FlexReportResponse, Trade};

    fn trade(transaction_id: &str, level: Option<&str>) -> Trade {
        Trade {
            transaction_id: Some(transaction_id.to_owned()),
            account_id: Some("U1".to_owned()),
            conid: Some("265598".to_owned()),
            symbol: Some("AAPL".to_owned()),
            description: Some("APPLE INC".to_owned()),
            asset_category: Some("STK".to_owned()),
            buy_sell: Some("BUY".to_owned()),
            trade_date: Some("2026-03-02".to_owned()),
            settle_date: Some("20260304".to_owned()),
            quantity: Some(100.0),
            price: Some(210.5),
            proceeds: Some(-21050.0),
            cost: Some(21050.35),
            commission: Some(-0.35),
            fifo_pnl_realized: Some(0.0),
            currency: Some("USD".to_owned()),
            exchange: Some("SMART".to_owned()),
            underlying_symbol: None,
            underlying_conid: None,
            multiplier: Some(1.0),
            strike: None,
            expiry: None,
            put_call: None,
            open_close: Some("O".to_owned()),
            open_date_time: Some("20260302;100000".to_owned()),
            trade_time: Some("20260302;100000".to_owned()),
            level_of_detail: level.map(str::to_owned),
            taxes: Some(0.0),
            net_cash: Some(-21050.35),
            mtm_pnl: Some(-12.5),
            fx_rate_to_base: Some(1.25),
            ib_order_id: Some("42".to_owned()),
            exec_id: Some("exec-1".to_owned()),
        }
    }

    fn empty_payload(trades: Vec<Trade>, cash: Vec<CashTransaction>) -> FlexReportResponse {
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

    fn cash(transaction_id: &str) -> CashTransaction {
        CashTransaction {
            transaction_id: Some(transaction_id.to_owned()),
            account_id: Some("U1".to_owned()),
            transaction_type: Some("Dividends".to_owned()),
            description: Some("APPLE INC CASH DIVIDEND".to_owned()),
            amount: Some(42.5),
            currency: Some("USD".to_owned()),
            fx_rate_to_base: Some(1.25),
            date: Some("20260315".to_owned()),
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
            std::env::temp_dir().join(format!("ibkr-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("trades.db")
    }

    /// Remove the store and everything `SQLite` kept beside it.
    fn cleanup(path: &Path) {
        if let Some(directory) = path.parent() {
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn compact_dates_are_normalized_on_the_way_in() {
        let path = temp_store("dates");
        let mut store = Store::open(&path).unwrap();
        store
            .upsert_report(
                "r",
                None,
                &empty_payload(vec![trade("t1", Some("EXECUTION"))], vec![cash("c1")]),
            )
            .unwrap();
        let row = store
            .query("SELECT trade_date, settle_date, trade_time FROM executions")
            .unwrap();
        assert_eq!(
            row.rows[0][0],
            rusqlite::types::Value::Text("2026-03-02".to_owned())
        );
        assert_eq!(
            row.rows[0][1],
            rusqlite::types::Value::Text("2026-03-04".to_owned())
        );
        // Times stay verbatim: they are not part of the ISO normalization.
        assert_eq!(
            row.rows[0][2],
            rusqlite::types::Value::Text("20260302;100000".to_owned())
        );
        let cash_date = store.query("SELECT date FROM cash_transactions").unwrap();
        assert_eq!(
            cash_date.rows[0][0],
            rusqlite::types::Value::Text("2026-03-15".to_owned())
        );
        cleanup(&path);
    }

    #[test]
    fn a_lot_row_is_stored_under_its_opening_transaction() {
        let path = temp_store("lot");
        let mut store = Store::open(&path).unwrap();
        let mut lot = trade("20260608;155631-open", Some("CLOSED_LOT"));
        lot.open_date_time = Some("20260608;155631".to_owned());
        lot.trade_date = Some("20260702".to_owned());
        lot.trade_time = Some("20260702;133744".to_owned());
        lot.fifo_pnl_realized = Some(38.275);
        store
            .upsert_report("r", None, &empty_payload(vec![lot], Vec::new()))
            .unwrap();

        let trip = store
            .query("SELECT open_date, close_date, realized_pnl FROM round_trips")
            .unwrap();
        assert_eq!(
            trip.rows[0][0],
            rusqlite::types::Value::Text("2026-06-08".to_owned())
        );
        assert_eq!(
            trip.rows[0][1],
            rusqlite::types::Value::Text("2026-07-02".to_owned())
        );
        assert_eq!(trip.rows[0][2], rusqlite::types::Value::Real(38.275));
        let executions = store.query("SELECT COUNT(*) FROM executions").unwrap();
        assert_eq!(executions.rows[0][0], rusqlite::types::Value::Integer(0));
        cleanup(&path);
    }

    #[test]
    fn resyncing_replaces_instead_of_duplicating() {
        let path = temp_store("replace");
        let mut store = Store::open(&path).unwrap();
        let first = empty_payload(vec![trade("t1", Some("EXECUTION"))], Vec::new());
        store.upsert_report("r", Some("\"a\""), &first).unwrap();

        // The statement is regenerated with a corrected quantity for the same
        // execution: the row is replaced, not appended.
        let mut corrected = trade("t1", Some("EXECUTION"));
        corrected.quantity = Some(150.0);
        let mut payload = empty_payload(vec![corrected], Vec::new());
        payload.from_date = Some("2026-04-01".to_owned());
        let stats = store.upsert_report("r", Some("\"b\""), &payload).unwrap();
        assert_eq!(stats.executions, 1);

        let rows = store
            .query("SELECT COUNT(*), MAX(quantity) FROM executions")
            .unwrap();
        assert_eq!(rows.rows[0][0], rusqlite::types::Value::Integer(1));
        assert_eq!(rows.rows[0][1], rusqlite::types::Value::Real(150.0));
        let sync_row = store
            .query("SELECT from_date, etag FROM sync_state WHERE report = 'r'")
            .unwrap();
        assert_eq!(
            sync_row.rows[0][0],
            rusqlite::types::Value::Text("2026-04-01".to_owned())
        );
        assert_eq!(
            sync_row.rows[0][1],
            rusqlite::types::Value::Text("\"b\"".to_owned())
        );
        cleanup(&path);
    }

    #[test]
    fn classify_keeps_aggregates_out() {
        assert_eq!(classify(&trade("t", None)), Level::Execution);
        assert_eq!(classify(&trade("t", Some("EXECUTION"))), Level::Execution);
        assert_eq!(classify(&trade("t", Some("CLOSED_LOT"))), Level::Lot);
        assert_eq!(classify(&trade("t", Some("LOT"))), Level::Lot);
        assert_eq!(
            classify(&trade("t", Some("SYMBOL_SUMMARY"))),
            Level::Aggregate
        );
        assert_eq!(
            classify(&trade("t", Some("ASSET_SUMMARY"))),
            Level::Aggregate
        );
        assert_eq!(classify(&trade("t", Some("ORDER"))), Level::Aggregate);
    }

    #[test]
    fn lots_arriving_in_their_own_array_become_round_trips() {
        let path = temp_store("lots-array");
        let mut store = Store::open(&path).unwrap();
        // The gateway's shape: executions in `trades`, closed lots in `lots`,
        // the same money recorded in both — only the lots may be summed.
        let mut execution = trade("close-exec", Some("EXECUTION"));
        execution.open_close = Some("C".to_owned());
        execution.fifo_pnl_realized = Some(92.825);
        let mut lot = trade("open-exec", Some("CLOSED_LOT"));
        lot.open_date_time = Some("20260807;120536".to_owned());
        lot.trade_time = Some("20260828;123310".to_owned());
        lot.trade_date = Some("2026-08-28".to_owned());
        lot.quantity = Some(500.0);
        lot.cost = Some(155.85);
        lot.fifo_pnl_realized = Some(92.825);

        let stats = store
            .upsert_report(
                "r",
                None,
                &payload_with_lots(vec![execution], vec![lot], Vec::new()),
            )
            .unwrap();
        assert_eq!(stats.executions, 1);
        assert_eq!(stats.lots, 1);

        let trips = store
            .query("SELECT open_transaction_id, open_date, close_date, quantity, cost, realized_pnl FROM round_trips")
            .unwrap();
        assert_eq!(
            trips.rows[0][0],
            rusqlite::types::Value::Text("open-exec".to_owned())
        );
        assert_eq!(
            trips.rows[0][1],
            rusqlite::types::Value::Text("2026-08-07".to_owned())
        );
        assert_eq!(
            trips.rows[0][2],
            rusqlite::types::Value::Text("2026-08-28".to_owned())
        );
        assert_eq!(trips.rows[0][4], rusqlite::types::Value::Real(155.85));
        assert_eq!(trips.rows[0][5], rusqlite::types::Value::Real(92.825));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn identical_lots_belong_to_different_closes_and_are_all_kept() {
        let path = temp_store("identical-lots");
        let mut store = Store::open(&path).unwrap();
        // Real shape: one order fills in five executions, and two of them close
        // the same quantity of the same opened lot — byte-identical lot rows.
        let mut first = trade("open-exec", Some("CLOSED_LOT"));
        first.trade_date = Some("2026-07-06".to_owned());
        first.trade_time = Some("20260706;145105".to_owned());
        first.quantity = Some(500.0);
        first.cost = Some(238.7625);
        first.fifo_pnl_realized = Some(22.375);
        let twin = first.clone();

        let stats = store
            .upsert_report(
                "r",
                None,
                &payload_with_lots(Vec::new(), vec![first, twin], Vec::new()),
            )
            .unwrap();
        assert_eq!(
            stats.lots, 2,
            "an identical twin is a second lot, not a duplicate key"
        );

        let rows = store
            .query("SELECT COUNT(*), ROUND(SUM(realized_pnl), 4) FROM round_trips")
            .unwrap();
        assert_eq!(rows.rows[0][0], rusqlite::types::Value::Integer(2));
        assert_eq!(rows.rows[0][1], rusqlite::types::Value::Real(44.75));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn resyncing_a_report_replaces_its_lots_instead_of_accumulating() {
        let path = temp_store("replace-lots");
        let mut store = Store::open(&path).unwrap();
        let mut lot = trade("open-exec", Some("CLOSED_LOT"));
        lot.trade_time = Some("20260706;145105".to_owned());
        lot.quantity = Some(100.0);
        store
            .upsert_report(
                "r",
                None,
                &payload_with_lots(Vec::new(), vec![lot.clone()], Vec::new()),
            )
            .unwrap();
        assert_eq!(
            store
                .query("SELECT COUNT(*) FROM round_trips")
                .unwrap()
                .rows[0][0],
            rusqlite::types::Value::Integer(1)
        );

        // The statement is regenerated with a different close: the old row is
        // gone rather than lingering beside the new one.
        let mut corrected = lot.clone();
        corrected.trade_time = Some("20260707;093000".to_owned());
        corrected.trade_date = Some("2026-07-07".to_owned());
        store
            .upsert_report(
                "r",
                None,
                &payload_with_lots(Vec::new(), vec![corrected], Vec::new()),
            )
            .unwrap();
        let rows = store
            .query("SELECT COUNT(*), MAX(close_date) FROM round_trips")
            .unwrap();
        assert_eq!(rows.rows[0][0], rusqlite::types::Value::Integer(1));
        assert_eq!(
            rows.rows[0][1],
            rusqlite::types::Value::Text("2026-07-07".to_owned())
        );

        // Another report's lots are untouched by this report's re-sync.
        store
            .upsert_report(
                "other",
                None,
                &payload_with_lots(Vec::new(), vec![lot], Vec::new()),
            )
            .unwrap();
        store
            .upsert_report(
                "r",
                None,
                &payload_with_lots(Vec::new(), Vec::new(), Vec::new()),
            )
            .unwrap();
        assert_eq!(
            store
                .query("SELECT COUNT(*) FROM round_trips")
                .unwrap()
                .rows[0][0],
            rusqlite::types::Value::Integer(1),
            "the other report's lot must survive"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn cash_summaries_are_not_stored_alongside_their_details() {
        let path = temp_store("cash-levels");
        let mut store = Store::open(&path).unwrap();

        let detail = cash("c1");
        let mut summary = cash("s1"); // per-report-date aggregate of the same money
        summary.level_of_detail = Some("SUMMARY".to_owned());
        summary.transaction_id = None;
        let mut unidentified = cash("c2");
        unidentified.transaction_id = None; // detail with nothing to dedupe on

        let stats = store
            .upsert_report(
                "r",
                None,
                &empty_payload(Vec::new(), vec![detail, summary, unidentified]),
            )
            .unwrap();
        assert_eq!(stats.cash_transactions, 1);
        assert_eq!(stats.skipped, 2);

        // One row stored, so the cash view sums each movement exactly once.
        let rows = store
            .query("SELECT COUNT(*), SUM(amount) FROM cash_transactions")
            .unwrap();
        assert_eq!(rows.rows[0][0], rusqlite::types::Value::Integer(1));
        assert_eq!(rows.rows[0][1], rusqlite::types::Value::Real(42.5));
        cleanup(&path);
    }

    #[test]
    fn cash_detail_rule_prefers_the_level_over_the_transaction_id() {
        let mut summary_with_id = cash("s1");
        summary_with_id.level_of_detail = Some("SUMMARY".to_owned());
        assert!(
            !summary_with_id.is_detail(),
            "a labelled summary is not a movement"
        );
        let mut detail_without_id = cash("c2");
        detail_without_id.transaction_id = None;
        assert!(
            detail_without_id.is_detail(),
            "a labelled detail stays a movement"
        );
        let mut unlabelled = cash("c3");
        unlabelled.level_of_detail = None;
        assert!(unlabelled.is_detail());
        unlabelled.transaction_id = None;
        assert!(!unlabelled.is_detail(), "no level, no id: nothing to store");
    }

    #[test]
    fn cash_without_a_level_falls_back_to_the_transaction_id() {
        let path = temp_store("cash-fallback");
        let mut store = Store::open(&path).unwrap();
        // A gateway that predates `levelOfDetail` on cash: an id means detail.
        let identified = cash("c1");
        let anonymous = CashTransaction {
            transaction_id: None,
            ..cash("ignored")
        };
        let stats = store
            .upsert_report(
                "r",
                None,
                &empty_payload(Vec::new(), vec![identified, anonymous]),
            )
            .unwrap();
        assert_eq!(stats.cash_transactions, 1);
        assert_eq!(stats.skipped, 1);
        cleanup(&path);
    }

    #[test]
    fn as_date_leaves_absent_values_absent() {
        assert_eq!(as_date(Some("20260702")), Some("2026-07-02".to_owned()));
        assert_eq!(as_date(Some("2026-07-02")), Some("2026-07-02".to_owned()));
        assert_eq!(as_date(Some("")), None);
        assert_eq!(as_date(None), None);
    }

    #[test]
    fn the_store_lives_outside_the_statement_dir() {
        // Guards the test helper itself: a temp path, never the repo default.
        let path = temp_store("sanity");
        assert!(!path.starts_with(Path::new("/workspaces/ibkr.rs")));
        cleanup(&path);
    }
}
