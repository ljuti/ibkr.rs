//! Rendering of gateway responses for the terminal.
//!
//! Two modes, chosen with `--output`:
//!
//! * `json` (default) prints the payload as delivered, complete and
//!   pipe-friendly.
//! * `table` renders a fixed column projection for humans. A table is lossy by
//!   design — it shows the columns chosen here and nothing else.
//!
//! Anything that is not the payload itself (envelope metadata, notes, headings)
//! goes to stderr, so stdout stays a clean stream of rows or JSON.

use std::io::Write;

use serde::Serialize;

use crate::cli::RenderMode;
use crate::client::Conditional;
use crate::config::Config;
use crate::error::Result;
use crate::types::{
    AccountSummaryValue, AckResponse, BracketOrderIdsResponse, CancelResponse, CashTransaction,
    ContractDetailsResponse, ExecutionResponse, FlexConfigResponse, FlexReportResponse, Health,
    HistoricalDataResponse, OrderIdResponse, OrderResponse, PnLResponse, PositionResponse,
    SnapshotEnvelope, SymbolSearchResponse, Trade,
};

/// Smallest width a column may be shrunk to before truncation stops.
const MIN_COLUMN_WIDTH: usize = 6;
/// Gap between columns.
const COLUMN_GAP: usize = 2;

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// A header row plus data rows, rendered as aligned text.
#[derive(Debug, Clone)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// Start a table with the given column headers.
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|header| (*header).to_owned()).collect(),
            rows: Vec::new(),
        }
    }

    /// Append a row. Extra cells beyond the header count are ignored.
    pub fn push(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    /// Whether the table has no data rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Render the table, shrinking columns to fit `available` when given.
    fn render(&self, available: Option<usize>) -> String {
        let widths = self.column_widths(available);

        let mut rendered = String::new();
        push_row(&mut rendered, &self.headers, &widths);
        for row in &self.rows {
            push_row(&mut rendered, row, &widths);
        }
        rendered
    }

    fn column_widths(&self, available: Option<usize>) -> Vec<usize> {
        let mut widths: Vec<usize> = self
            .headers
            .iter()
            .map(|header| header.chars().count())
            .collect();

        for row in &self.rows {
            for (index, cell) in row.iter().enumerate() {
                if index < widths.len() {
                    widths[index] = widths[index].max(cell.chars().count());
                }
            }
        }

        let Some(available) = available else {
            return widths;
        };

        let gaps = COLUMN_GAP * widths.len().saturating_sub(1);
        let mut total: usize = widths.iter().sum::<usize>() + gaps;
        // Take a character at a time from whichever column is currently widest,
        // so wide columns absorb the deficit instead of narrow ones collapsing.
        while total > available {
            let widest = widths
                .iter()
                .enumerate()
                .filter(|(_, width)| **width > MIN_COLUMN_WIDTH)
                .max_by_key(|(_, width)| **width)
                .map(|(index, _)| index);
            match widest {
                Some(index) => {
                    widths[index] -= 1;
                    total -= 1;
                }
                None => break,
            }
        }
        widths
    }
}

impl std::fmt::Display for Table {
    /// Render at natural width, ignoring the terminal.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.render(None))
    }
}

/// Append one padded, truncated row.
fn push_row(rendered: &mut String, row: &[String], widths: &[usize]) {
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            rendered.push_str(&" ".repeat(COLUMN_GAP));
        }
        let cell = row.get(index).map_or("", String::as_str);
        rendered.push_str(&fit(cell, *width));
    }
    // Trailing padding is noise in a terminal and in diffs.
    while rendered.ends_with(' ') {
        rendered.pop();
    }
    rendered.push('\n');
}

/// Truncate to `width` characters, marking elision, then pad to it.
fn fit(cell: &str, width: usize) -> String {
    let length = cell.chars().count();
    if length > width {
        let mut truncated: String = cell.chars().take(width.saturating_sub(1)).collect();
        truncated.push('…');
        return truncated;
    }
    let mut padded = cell.to_owned();
    padded.push_str(&" ".repeat(width - length));
    padded
}

/// Render `value` as JSON on stdout.
pub fn json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Build a two-column table from key/value pairs.
pub fn kv_table(rows: &[(&str, String)]) -> Table {
    let mut table = Table::new(&["field", "value"]);
    for (key, value) in rows {
        table.push(vec![(*key).to_owned(), value.clone()]);
    }
    table
}

/// Print a table on stdout, noting an empty result on stderr.
fn print_table(table: &Table, mode: RenderMode) -> Result<()> {
    if table.is_empty() {
        note("(no rows)");
    }
    print!("{}", table.render(mode.available_width()));
    std::io::stdout().flush()?;
    Ok(())
}

/// Write a note to stderr, keeping stdout machine-readable.
pub fn note(message: &str) {
    eprintln!("{message}");
}

/// Format a number without scientific notation for whole values.
pub fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Format an optional number, `-` when absent.
fn opt_number(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_owned(), number)
}

/// Format optional text, `-` when absent or empty.
fn opt_text(value: Option<&str>) -> String {
    match value {
        Some(text) if !text.is_empty() => text.to_owned(),
        _ => "-".to_owned(),
    }
}

/// Format a boolean as `yes`/`no`.
fn boolean(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

/// Render the broker connectivity snapshot.
pub fn health(health: &Health, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(health);
    }
    let rows = [
        ("status", health.status.clone()),
        ("connected", boolean(health.connected)),
        ("serverVersion", health.server_version.to_string()),
        ("clientId", health.client_id.to_string()),
        ("nextOrderId", health.next_order_id.to_string()),
        (
            "connectionTime",
            opt_text(health.connection_time.as_deref()),
        ),
        ("timeZone", opt_text(health.time_zone.as_deref())),
    ];
    print_table(&kv_table(&rows), mode)
}

// ---------------------------------------------------------------------------
// Contracts
// ---------------------------------------------------------------------------

/// Render contract details.
pub fn contract_details(details: &[ContractDetailsResponse], mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(&details);
    }
    let mut table = Table::new(&[
        "contractId",
        "symbol",
        "secType",
        "exchange",
        "currency",
        "longName",
        "marketName",
        "minTick",
    ]);
    for detail in details {
        table.push(vec![
            detail.contract.contract_id.to_string(),
            detail.contract.symbol.clone(),
            detail.contract.sec_type.clone(),
            detail.contract.exchange.clone(),
            detail.contract.currency.clone(),
            detail.long_name.clone(),
            detail.market_name.clone(),
            number(detail.min_tick),
        ]);
    }
    print_table(&table, mode)
}

/// Render symbol search results.
pub fn symbol_search(results: &[SymbolSearchResponse], mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(&results);
    }
    // Matching-symbols results carry only id, symbol, security type and
    // currency — IBKR's `reqMatchingSymbols` has no exchange or local symbol,
    // so those columns would always be empty here. They stay in the JSON
    // payload; only the human projection drops them.
    let mut table = Table::new(&["contractId", "symbol", "secType", "currency"]);
    for result in results {
        table.push(vec![
            result.contract.contract_id.to_string(),
            result.contract.symbol.clone(),
            result.contract.sec_type.clone(),
            result.contract.currency.clone(),
        ]);
    }
    print_table(&table, mode)
}

// ---------------------------------------------------------------------------
// Market data
// ---------------------------------------------------------------------------

/// Render a historical bar response.
pub fn historical(response: &HistoricalDataResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(response);
    }
    note(&format!(
        "{} bars from {} to {}",
        response.bars.len(),
        response.start,
        response.end
    ));
    let mut table = Table::new(&[
        "date", "open", "high", "low", "close", "volume", "wap", "count",
    ]);
    for bar in &response.bars {
        table.push(vec![
            bar.date.clone(),
            number(bar.open),
            number(bar.high),
            number(bar.low),
            number(bar.close),
            number(bar.volume),
            number(bar.wap),
            bar.count.to_string(),
        ]);
    }
    print_table(&table, mode)
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// Render a positions snapshot.
pub fn positions(envelope: &SnapshotEnvelope<PositionResponse>, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(envelope);
    }
    envelope_note(envelope.count, "positions", envelope.truncated);
    // Option identity matters here: one underlying can hold several contracts
    // that differ only by expiry/right/strike. Non-options render `-`.
    let mut table = Table::new(&[
        "account",
        "symbol",
        "secType",
        "expiry",
        "right",
        "strike",
        "position",
        "averageCost",
        "currency",
    ]);
    for position in &envelope.data {
        table.push(vec![
            position.account.clone(),
            position.contract.symbol.clone(),
            position.contract.sec_type.clone(),
            opt_text(
                position
                    .contract
                    .last_trade_date_or_contract_month
                    .as_deref(),
            ),
            opt_text(position.contract.right.as_deref()),
            opt_number(position.contract.strike),
            number(position.position),
            number(position.average_cost),
            position.contract.currency.clone(),
        ]);
    }
    print_table(&table, mode)
}

/// Render account summary values.
pub fn account_summary(values: &[AccountSummaryValue], mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(&values);
    }
    let mut table = Table::new(&["account", "tag", "value", "currency"]);
    for value in values {
        table.push(vec![
            value.account.clone(),
            value.tag.clone(),
            value.value.clone(),
            value.currency.clone(),
        ]);
    }
    print_table(&table, mode)
}

/// Render a profit-and-loss snapshot.
pub fn pnl(pnl: &PnLResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(pnl);
    }
    let rows = [
        ("dailyPnl", number(pnl.daily_pnl)),
        ("unrealizedPnl", opt_number(pnl.unrealized_pnl)),
        ("realizedPnl", opt_number(pnl.realized_pnl)),
    ];
    print_table(&kv_table(&rows), mode)
}

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

/// Render an order snapshot (`label` names the set, e.g. `open orders`).
pub fn orders(
    envelope: &SnapshotEnvelope<OrderResponse>,
    label: &str,
    mode: RenderMode,
) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(envelope);
    }
    envelope_note(envelope.count, label, envelope.truncated);
    let mut table = Table::new(&[
        "orderId",
        "symbol",
        "action",
        "totalQuantity",
        "orderType",
        "limitPrice",
        "status",
        "filled",
        "remaining",
    ]);
    for order in &envelope.data {
        table.push(vec![
            order.order_id.to_string(),
            order.symbol.clone(),
            order.action.clone(),
            number(order.total_quantity),
            order.order_type.clone(),
            order
                .limit_price
                .map_or_else(|| opt_number(order.aux_price), number),
            order.status.clone(),
            opt_number(order.filled),
            opt_number(order.remaining),
        ]);
    }
    print_table(&table, mode)
}

/// Render an executions snapshot.
pub fn executions(envelope: &SnapshotEnvelope<ExecutionResponse>, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(envelope);
    }
    envelope_note(envelope.count, "executions", envelope.truncated);
    let mut table = Table::new(&[
        "time",
        "side",
        "shares",
        "price",
        "commission",
        "currency",
        "executionId",
    ]);
    for execution in &envelope.data {
        table.push(vec![
            execution.time.clone(),
            execution.side.clone(),
            number(execution.shares),
            number(execution.price),
            opt_number(execution.commission),
            opt_text(execution.commission_currency.as_deref()),
            execution.execution_id.clone(),
        ]);
    }
    print_table(&table, mode)
}

/// Render the acknowledgement of a placed order.
pub fn order_placed(response: &OrderIdResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(response);
    }
    print_table(
        &kv_table(&[("orderId", response.order_id.to_string())]),
        mode,
    )
}

/// Render the acknowledgement of a placed bracket order.
pub fn bracket_placed(response: &BracketOrderIdsResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(response);
    }
    let rows = [
        ("parent", response.parent.to_string()),
        ("takeProfit", response.take_profit.to_string()),
        ("stopLoss", response.stop_loss.to_string()),
    ];
    print_table(&kv_table(&rows), mode)
}

/// Render the acknowledgement of a cancellation.
pub fn cancelled(response: &CancelResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(response);
    }
    let rows = [
        ("orderId", response.order_id.to_string()),
        ("status", opt_text(response.status.as_deref())),
    ];
    print_table(&kv_table(&rows), mode)
}

// ---------------------------------------------------------------------------
// Flex
// ---------------------------------------------------------------------------

/// Render the Flex configuration status.
pub fn flex_config(config: &FlexConfigResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(config);
    }
    let mut table = Table::new(&["report", "queryConfigured", "tokenConfigured"]);
    for report in &config.reports {
        table.push(vec![
            report.name.clone(),
            boolean(report.query_configured),
            boolean(report.token_configured),
        ]);
    }
    print_table(&table, mode)
}

/// Render a Flex configuration acknowledgement.
pub fn flex_ack(response: &AckResponse, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(response);
    }
    let rows = [
        ("status", response.status.clone()),
        ("report", opt_text(response.report.as_deref())),
    ];
    print_table(&kv_table(&rows), mode)
}

/// Render a Flex report fetch, including the `304` outcome.
pub fn flex_report(outcome: &Conditional<FlexReportResponse>, mode: RenderMode) -> Result<()> {
    match outcome {
        Conditional::NotModified { etag } => {
            note(&format!(
                "not modified (etag {})",
                opt_text(etag.as_deref())
            ));
            Ok(())
        }
        Conditional::Fresh { value, etag } => {
            note(&format!("etag {}", opt_text(etag.as_deref())));
            if mode.output == crate::cli::Output::Json {
                return json(value);
            }

            note(&format!(
                "trades ({} rows, account {} from {} to {})",
                value.trades.len(),
                opt_text(value.account_id.as_deref()),
                opt_text(value.from_date.as_deref()),
                opt_text(value.to_date.as_deref())
            ));
            print_table(&trades_table(value), mode)?;

            let (from_lots, closed) = round_trips(value);
            note(&round_trip_note(from_lots, closed.len()));
            print_table(&round_trips_table(&closed), mode)?;

            let (detail, summaries) = cash_detail_rows(value);
            if summaries == 0 {
                note(&format!("cash transactions ({} rows)", detail.len()));
            } else {
                note(&format!(
                    "cash transactions ({} rows, {summaries} summary rows omitted)",
                    detail.len()
                ));
            }
            print_table(&cash_table(&detail), mode)
        }
    }
}

/// Every execution in the report.
fn trades_table(value: &FlexReportResponse) -> Table {
    let mut trades = Table::new(&[
        "tradeDate",
        "symbol",
        "buySell",
        "quantity",
        "price",
        "proceeds",
        "commission",
        "currency",
    ]);
    for trade in &value.trades {
        trades.push(vec![
            opt_text(trade.trade_date.as_deref()),
            opt_text(trade.symbol.as_deref()),
            opt_text(trade.buy_sell.as_deref()),
            opt_number(trade.quantity),
            opt_number(trade.price),
            opt_number(trade.proceeds),
            opt_number(trade.commission),
            opt_text(trade.currency.as_deref()),
        ]);
    }
    trades
}

/// Rows for the round-trip projection, and whether they came from lots.
///
/// Closed lots are the exact pairing — opening execution, close, matched
/// quantity, cost basis, realized P/L. Without them, closing executions are the
/// best available source: they carry the realized figure but rarely an open
/// date.
fn round_trips(value: &FlexReportResponse) -> (bool, Vec<&Trade>) {
    if value.lots.is_empty() {
        (
            false,
            value
                .trades
                .iter()
                .filter(|trade| closes_position(trade))
                .collect(),
        )
    } else {
        (true, value.lots.iter().collect())
    }
}

/// Heading for the round-trip table, naming the remedy when there are no lots.
///
/// A report is only as capable as the Flex query behind it, and a missing
/// section is a configuration choice the reader can fix.
fn round_trip_note(from_lots: bool, rows: usize) -> String {
    if from_lots {
        format!("closed lots ({rows} rows)")
    } else {
        format!(
            "closed trades ({rows} rows; no closed-lot rows in this report — add the Closed Lots \
             level to the Flex query for exact pairing)"
        )
    }
}

fn round_trips_table(closed: &[&Trade]) -> Table {
    let mut table = Table::new(&[
        "closed", "opened", "symbol", "expiry", "right", "strike", "quantity", "cost", "pnl",
        "currency",
    ]);
    for trade in closed {
        table.push(vec![
            opt_text(trade.trade_date.as_deref()),
            date_part(trade.open_date_time.as_deref()),
            opt_text(trade.symbol.as_deref()),
            opt_text(trade.expiry.as_deref()),
            enum_text(trade.put_call.as_deref()),
            opt_number(trade.strike),
            opt_number(trade.quantity),
            opt_number(trade.cost),
            opt_number(trade.fifo_pnl_realized),
            opt_text(trade.currency.as_deref()),
        ]);
    }
    table
}

/// Cash movements, and how many summary rows were left out.
///
/// The gateway returns both levels; a summary row restates the same money per
/// report date, so printing both invites a double count.
fn cash_detail_rows(value: &FlexReportResponse) -> (Vec<&CashTransaction>, usize) {
    let detail: Vec<&CashTransaction> = value
        .cash_transactions
        .iter()
        .filter(|transaction| transaction.is_detail())
        .collect();
    let summaries = value.cash_transactions.len() - detail.len();
    (detail, summaries)
}

fn cash_table(detail: &[&CashTransaction]) -> Table {
    let mut cash = Table::new(&[
        "date",
        "transactionType",
        "amount",
        "currency",
        "description",
    ]);
    for transaction in detail {
        cash.push(vec![
            opt_text(transaction.date.as_deref()),
            opt_text(transaction.transaction_type.as_deref()),
            opt_number(transaction.amount),
            opt_text(transaction.currency.as_deref()),
            opt_text(transaction.description.as_deref()),
        ]);
    }
    cash
}

/// Note the size of a bounded snapshot.
fn envelope_note(count: usize, label: &str, truncated: bool) {
    note(&format!("{label}: {count} (truncated: {truncated})"));
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Effective settings, where each came from, and the file they live in.
///
/// Shows what a command would *use* — flags beat environment beats file beats
/// defaults — so an ambient variable that shadows the file is visible rather
/// than surfacing later as a puzzling failure.
pub fn configure_show(config: &Config, mode: RenderMode) -> Result<()> {
    let settings: [(&str, String, &str); 9] = [
        ("url", config.base_url.clone(), "url"),
        ("token", masked(config.token.as_deref()), "token"),
        (
            "ca-cert",
            config
                .ca_cert
                .as_ref()
                .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
            "ca-cert",
        ),
        (
            "client-cert",
            config
                .client_cert
                .as_ref()
                .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
            "client-cert",
        ),
        (
            "client-key",
            config
                .client_key
                .as_ref()
                .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
            "client-key",
        ),
        (
            "timeout",
            format!("{}s", config.timeout.as_secs()),
            "timeout",
        ),
        ("max-retries", config.max_retries.to_string(), "max-retries"),
        (
            "tls-skip-verify",
            boolean(config.tls_skip_verify),
            "tls-skip-verify",
        ),
        ("db", config.db.display().to_string(), "db"),
    ];

    if mode.output == crate::cli::Output::Json {
        let mut object = serde_json::Map::new();
        object.insert(
            "configFile".to_owned(),
            serde_json::json!(config.config_path.display().to_string()),
        );
        for (name, value, key) in settings {
            object.insert(
                name.to_owned(),
                serde_json::json!({
                    "value": value,
                    "source": config.sources.get(key).copied().unwrap_or_default().label(),
                }),
            );
        }
        return json(&serde_json::Value::Object(object));
    }

    let mut table = Table::new(&["setting", "value", "source"]);
    for (name, value, key) in settings {
        let source = config
            .sources
            .get(key)
            .copied()
            .unwrap_or(crate::config::Source::Default);
        table.push(vec![name.to_owned(), value, source.label().to_owned()]);
    }
    print_table(&table, mode)?;
    note(&format!("config file: {}", config.config_path.display()));
    Ok(())
}

/// A secret rendered for display: enough to tell two of them apart, never the
/// value itself.
fn masked(secret: Option<&str>) -> String {
    match secret.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.chars().count() <= 4 => "●●●●".to_owned(),
        Some(value) => {
            let head: String = value.chars().take(4).collect();
            format!("{head}●●●●")
        }
        None => "-".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Local store
// ---------------------------------------------------------------------------

/// Render the outcome of `store sync`.
pub fn store_sync(outcomes: &[crate::store::SyncOutcome], mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        let values: Vec<serde_json::Value> = outcomes.iter().map(sync_outcome_json).collect();
        return json(&values);
    }
    let mut table = Table::new(&[
        "report",
        "status",
        "from",
        "to",
        "executions",
        "lots",
        "cash",
        "skipped",
        "etag",
    ]);
    for outcome in outcomes {
        match outcome {
            crate::store::SyncOutcome::Synced(stats) => table.push(vec![
                stats.report.clone(),
                "synced".to_owned(),
                opt_text(stats.from_date.as_deref()),
                opt_text(stats.to_date.as_deref()),
                stats.executions.to_string(),
                stats.lots.to_string(),
                stats.cash_transactions.to_string(),
                stats.skipped.to_string(),
                opt_text(stats.etag.as_deref()),
            ]),
            crate::store::SyncOutcome::NotModified { report, etag } => table.push(vec![
                report.clone(),
                "not modified".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                opt_text(etag.as_deref()),
            ]),
            // The sweep kept going: this report's failure is one row, and the
            // command's exit code still reports it.
            crate::store::SyncOutcome::Failed { report, message } => table.push(vec![
                report.clone(),
                "failed".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                "-".to_owned(),
                message.clone(),
            ]),
        }
    }
    print_table(&table, mode)
}

/// One sync outcome as JSON: `status` first, then the fields that apply.
fn sync_outcome_json(outcome: &crate::store::SyncOutcome) -> serde_json::Value {
    match outcome {
        crate::store::SyncOutcome::Synced(stats) => {
            let mut object = serde_json::Map::new();
            object.insert("status".to_owned(), serde_json::json!("synced"));
            if let serde_json::Value::Object(fields) =
                serde_json::to_value(stats).unwrap_or_default()
            {
                object.extend(fields);
            }
            serde_json::Value::Object(object)
        }
        crate::store::SyncOutcome::NotModified { report, etag } => serde_json::json!({
            "status": "notModified",
            "report": report,
            "etag": etag,
        }),
        crate::store::SyncOutcome::Failed { report, message } => serde_json::json!({
            "status": "failed",
            "report": report,
            "error": message,
        }),
    }
}

/// Render what the store holds and what that is enough to answer.
pub fn store_status(status: &crate::store::StoreStatus, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        return json(status);
    }
    let mut table = Table::new(&[
        "report",
        "from",
        "to",
        "executions",
        "lots",
        "cash",
        "skipped",
        "synced at",
    ]);
    for report in &status.reports {
        table.push(vec![
            report.report.clone(),
            opt_text(report.from_date.as_deref()),
            opt_text(report.to_date.as_deref()),
            report.executions.to_string(),
            report.lots.to_string(),
            report.cash_transactions.to_string(),
            report.skipped.to_string(),
            report.synced_at.clone(),
        ]);
    }
    print_table(&table, mode)?;
    note(&format!(
        "store holds {} executions, {} closed lots, {} cash movements ({} dated)",
        status.executions, status.lots, status.cash_transactions, status.cash_dated
    ));
    for line in capability_notes(&status.capabilities) {
        note(&line);
    }
    Ok(())
}

/// What the stored data can and cannot answer, one line per gap.
pub fn capability_notes(capabilities: &crate::store::SyncCapabilities) -> Vec<String> {
    let mut lines = Vec::new();
    if capabilities.round_trips {
        lines.push("round trips: exact — closed lots are stored".to_owned());
    } else {
        lines.push(
            "round trips: unavailable — no closed lots stored; add the Closed Lots level to the \
             Flex query and re-sync with `store sync --refresh`"
                .to_owned(),
        );
    }
    if capabilities.cash_movements {
        lines.push(if capabilities.cash_dates {
            "cash movements: stored with dates".to_owned()
        } else {
            "cash movements: stored, but undated — grouping by month will not work".to_owned()
        });
    } else {
        lines.push(
            "cash movements: none stored — add the Cash Transactions section to the Flex query"
                .to_owned(),
        );
    }
    lines
}

/// Render rows returned by `store query`.
pub fn store_query(result: &crate::store::QueryResult, mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        let rows: Vec<serde_json::Value> = result
            .rows
            .iter()
            .map(|row| {
                let mut object = serde_json::Map::with_capacity(result.columns.len());
                for (column, cell) in result.columns.iter().zip(row) {
                    object.insert(column.clone(), cell_json(cell));
                }
                serde_json::Value::Object(object)
            })
            .collect();
        return json(&rows);
    }
    let headers: Vec<&str> = result.columns.iter().map(String::as_str).collect();
    let mut table = Table::new(&headers);
    for row in &result.rows {
        table.push(row.iter().map(cell_text).collect());
    }
    print_table(&table, mode)
}

/// Render the store's tables and views.
pub fn store_schema(entries: &[crate::store::SchemaEntry], mode: RenderMode) -> Result<()> {
    if mode.output == crate::cli::Output::Json {
        let values: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "kind": entry.kind,
                    "columns": entry.columns,
                })
            })
            .collect();
        return json(&values);
    }
    let mut table = Table::new(&["name", "kind", "columns"]);
    for entry in entries {
        table.push(vec![
            entry.name.clone(),
            entry.kind.clone(),
            entry.columns.join(", "),
        ]);
    }
    print_table(&table, mode)
}

/// A query cell as JSON, keeping `SQLite`'s types.
fn cell_json(cell: &rusqlite::types::Value) -> serde_json::Value {
    use rusqlite::types::Value;
    match cell {
        Value::Null => serde_json::Value::Null,
        Value::Integer(number) => serde_json::json!(number),
        Value::Real(number) => serde_json::Number::from_f64(*number)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Text(text) => serde_json::json!(text),
        // Blobs have no place in a trade table; the size is enough to spot one.
        Value::Blob(bytes) => serde_json::json!(format!("<{} bytes>", bytes.len())),
    }
}

/// A query cell for a table column: `-` for SQL `NULL`.
fn cell_text(cell: &rusqlite::types::Value) -> String {
    use rusqlite::types::Value;
    match cell {
        Value::Null => "-".to_owned(),
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => number(*value),
        Value::Text(text) => text.clone(),
        Value::Blob(bytes) => format!("<{} bytes>", bytes.len()),
    }
}

/// A Flex enum value, or `None` when it carries no information.
///
/// IBKR emits attributes that do not apply to an instrument as empty strings,
/// and the gateway passes them through `ib-flex`, whose `#[serde(other)]`
/// variant serializes as the sentinel `Unknown`. Neither is data: `putCall` on
/// a stock, or an open/close indicator the statement left blank.
fn enum_value(value: Option<&str>) -> Option<&str> {
    match value.map(str::trim) {
        Some(text) if !text.is_empty() && text != "Unknown" => Some(text),
        _ => None,
    }
}

/// A Flex enum rendered for a table column, `-` when it carries no value.
fn enum_text(value: Option<&str>) -> String {
    enum_value(value).map_or_else(|| "-".to_owned(), str::to_owned)
}

/// Whether a Flex trade row closes (part of) a position.
///
/// The statement's open/close indicator is authoritative: `C` closes, `C;O`
/// closes and reopens in one execution. A gateway that predates the indicator
/// only reports the realized amount, so a non-zero realized P/L stands in for
/// it there.
fn closes_position(trade: &Trade) -> bool {
    match enum_value(trade.open_close.as_deref()) {
        Some(indicator) => indicator.contains('C'),
        None => trade.fifo_pnl_realized.is_some_and(|pnl| pnl != 0.0),
    }
}

/// The date part of a Flex date-time, `-` when absent.
///
/// Statements encode the two inconsistently (`20260608;155631`,
/// `2025-01-15;100000`, `2025-01-15 10:00:00`); the tables show the date,
/// normalized so `opened` and `closed` read the same way.
fn date_part(value: Option<&str>) -> String {
    value
        .and_then(crate::types::flex_date)
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(table: &Table, available: Option<usize>) -> String {
        table.render(available)
    }

    #[test]
    fn columns_align_and_pad() {
        let mut table = Table::new(&["symbol", "qty"]);
        table.push(vec!["AAPL".to_owned(), "100".to_owned()]);
        table.push(vec!["MSFT".to_owned(), "5".to_owned()]);

        let rendered = render(&table, None);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "symbol  qty");
        assert_eq!(lines[1], "AAPL    100");
        // Trailing padding is trimmed, so rows have no trailing whitespace.
        assert_eq!(lines[2], "MSFT    5");
    }

    #[test]
    fn wide_tables_shrink_the_widest_column_and_mark_elision() {
        let mut table = Table::new(&["symbol", "description"]);
        table.push(vec![
            "AAPL".to_owned(),
            "APPLE INC — a very long instrument description".to_owned(),
        ]);

        let rendered = render(&table, Some(30));
        for line in rendered.lines() {
            assert!(line.chars().count() <= 30, "line too wide: {line:?}");
        }
        assert!(rendered.contains('…'), "elision must be marked: {rendered}");
        // The narrow column is untouched; the wide one absorbs the deficit.
        assert!(rendered.lines().next().unwrap().starts_with("symbol  "));
    }

    #[test]
    fn narrow_columns_are_not_shrunk_below_the_floor() {
        let mut table = Table::new(&["a", "b"]);
        table.push(vec!["aaaaaa".to_owned(), "bbbbbb".to_owned()]);
        let rendered = render(&table, Some(4));
        // Floor is 6 characters plus the 2-character gap: nothing collapses.
        assert_eq!(rendered.lines().next().unwrap(), "a       b");
    }

    #[test]
    fn optional_values_render_as_dash() {
        assert_eq!(opt_number(None), "-");
        assert_eq!(opt_text(None), "-");
        assert_eq!(opt_text(Some("")), "-");
        assert_eq!(opt_text(Some("NASDAQ")), "NASDAQ");
    }

    #[test]
    fn whole_numbers_lose_the_decimal_point_and_never_go_scientific() {
        assert_eq!(number(100.0), "100");
        assert_eq!(number(182.43), "182.43");
        assert_eq!(number(1_200_000.0), "1200000");
        assert_eq!(number(-75.25), "-75.25");
    }

    fn trade(open_close: Option<&str>, realized: Option<f64>) -> Trade {
        Trade {
            transaction_id: None,
            account_id: None,
            conid: None,
            symbol: None,
            description: None,
            asset_category: None,
            buy_sell: None,
            trade_date: None,
            settle_date: None,
            quantity: None,
            price: None,
            proceeds: None,
            cost: None,
            commission: None,
            fifo_pnl_realized: realized,
            currency: None,
            exchange: None,
            underlying_symbol: None,
            underlying_conid: None,
            multiplier: None,
            strike: None,
            expiry: None,
            put_call: None,
            open_close: open_close.map(str::to_owned),
            open_date_time: None,
            trade_time: None,
            level_of_detail: None,
            taxes: None,
            net_cash: None,
            mtm_pnl: None,
            fx_rate_to_base: None,
            ib_order_id: None,
            exec_id: None,
        }
    }

    #[test]
    fn closed_rows_are_the_ones_with_a_closing_indicator() {
        // The indicator decides, including the close-and-reopen spelling.
        assert!(closes_position(&trade(Some("C"), Some(0.0))));
        assert!(closes_position(&trade(Some("C;O"), Some(120.0))));
        assert!(!closes_position(&trade(Some("O"), Some(0.0))));
        assert!(!closes_position(&trade(Some("O"), Some(120.0))));
        // Blank attributes reach the client as ib-flex's `Unknown` sentinel
        // (IBKR emits `openCloseIndicator=""`), which is absence, not a value.
        assert!(closes_position(&trade(Some("Unknown"), Some(120.0))));
        assert!(!closes_position(&trade(Some("Unknown"), Some(0.0))));
        assert!(!closes_position(&trade(Some(""), Some(0.0))));
        // Without the indicator, only a non-zero realized amount is evidence
        // of a close — openings report 0.00, not nothing.
        assert!(closes_position(&trade(None, Some(0.01))));
        assert!(!closes_position(&trade(None, Some(0.0))));
        assert!(!closes_position(&trade(None, None)));
    }

    #[test]
    fn the_round_trip_heading_names_the_fix_when_lots_are_missing() {
        assert_eq!(round_trip_note(true, 12), "closed lots (12 rows)");
        let fallback = round_trip_note(false, 12);
        assert!(fallback.starts_with("closed trades (12 rows"));
        assert!(fallback.contains("Closed Lots"), "{fallback}");
        assert!(fallback.contains("exact pairing"), "{fallback}");
    }

    #[test]
    fn a_failed_sync_is_rendered_as_a_row_and_as_json() {
        use crate::store::SyncOutcome;

        let outcomes = [
            SyncOutcome::Failed {
                report: "transactions_30d".to_owned(),
                message: "rate limited (HTTP 429)".to_owned(),
            },
            SyncOutcome::NotModified {
                report: "last_365_days".to_owned(),
                etag: Some("\"abc\"".to_owned()),
            },
        ];
        let json = sync_outcome_json(&outcomes[0]);
        assert_eq!(json["status"], "failed");
        assert_eq!(json["report"], "transactions_30d");
        assert_eq!(json["error"], "rate limited (HTTP 429)");

        // The table keeps one row per report, failure included, so a sweep
        // never hides the part that did not work.
        let rows: Vec<Vec<String>> = outcomes
            .iter()
            .map(|outcome| match outcome {
                SyncOutcome::Failed { report, message } => {
                    vec![report.clone(), "failed".to_owned(), message.clone()]
                }
                SyncOutcome::NotModified { report, .. } => {
                    vec![report.clone(), "not modified".to_owned(), "-".to_owned()]
                }
                SyncOutcome::Synced(stats) => {
                    vec![stats.report.clone(), "synced".to_owned(), "-".to_owned()]
                }
            })
            .collect();
        assert_eq!(rows[0][1], "failed");
        assert_eq!(rows[1][1], "not modified");
    }

    #[test]
    fn capability_notes_say_what_is_missing_and_how_to_get_it() {
        let none = crate::store::SyncCapabilities {
            round_trips: false,
            cash_movements: false,
            cash_dates: false,
        };
        let lines = capability_notes(&none);
        assert!(
            lines.iter().any(|line| line.contains("Closed Lots")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("Cash Transactions")),
            "{lines:?}"
        );

        let all = crate::store::SyncCapabilities {
            round_trips: true,
            cash_movements: true,
            cash_dates: true,
        };
        let lines = capability_notes(&all);
        assert!(lines.iter().any(|line| line.contains("exact")), "{lines:?}");
        assert!(
            lines.iter().any(|line| line.contains("with dates")),
            "{lines:?}"
        );

        let undated = crate::store::SyncCapabilities {
            round_trips: true,
            cash_movements: true,
            cash_dates: false,
        };
        assert!(
            capability_notes(&undated)
                .iter()
                .any(|line| line.contains("undated"))
        );
    }

    #[test]
    fn enum_columns_render_absence_not_the_unknown_sentinel() {
        assert_eq!(enum_text(Some("C")), "C");
        assert_eq!(enum_text(Some("C;O")), "C;O");
        assert_eq!(enum_text(Some("Unknown")), "-");
        assert_eq!(enum_text(Some("")), "-");
        assert_eq!(enum_text(Some("  ")), "-");
        assert_eq!(enum_text(None), "-");
    }

    #[test]
    fn flex_date_times_lose_their_time_part() {
        assert_eq!(date_part(Some("2025-01-15;100000")), "2025-01-15");
        assert_eq!(date_part(Some("2025-01-15 10:00:00")), "2025-01-15");
        assert_eq!(date_part(Some("2025-01-15T10:00:00")), "2025-01-15");
        assert_eq!(date_part(Some("2025-01-15")), "2025-01-15");
        // Statements emit the compact form too; it is normalized to ISO so it
        // lines up with the dates the gateway parses.
        assert_eq!(date_part(Some("20260608;155631")), "2026-06-08");
        assert_eq!(date_part(Some("20260608")), "2026-06-08");
        assert_eq!(date_part(Some(" 20260608;155631 ")), "2026-06-08");
        assert_eq!(date_part(Some("")), "-");
        assert_eq!(date_part(Some("   ")), "-");
        assert_eq!(date_part(None), "-");
    }
}
