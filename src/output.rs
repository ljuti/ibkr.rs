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
use crate::error::Result;
use crate::types::{
    AccountSummaryValue, AckResponse, BracketOrderIdsResponse, CancelResponse,
    ContractDetailsResponse, ExecutionResponse, FlexConfigResponse, FlexReportResponse, Health,
    HistoricalDataResponse, OrderIdResponse, OrderResponse, PnLResponse, PositionResponse,
    SnapshotEnvelope, SymbolSearchResponse,
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
    let mut table = Table::new(&[
        "contractId",
        "symbol",
        "secType",
        "exchange",
        "currency",
        "localSymbol",
    ]);
    for result in results {
        table.push(vec![
            result.contract.contract_id.to_string(),
            result.contract.symbol.clone(),
            result.contract.sec_type.clone(),
            result.contract.exchange.clone(),
            result.contract.currency.clone(),
            result.contract.local_symbol.clone(),
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
    let mut table = Table::new(&[
        "account",
        "symbol",
        "secType",
        "position",
        "averageCost",
        "currency",
    ]);
    for position in &envelope.data {
        table.push(vec![
            position.account.clone(),
            position.contract.symbol.clone(),
            position.contract.sec_type.clone(),
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
            print_table(&trades, mode)?;

            note(&format!(
                "cash transactions ({} rows)",
                value.cash_transactions.len()
            ));
            let mut cash = Table::new(&[
                "date",
                "transactionType",
                "amount",
                "currency",
                "description",
            ]);
            for transaction in &value.cash_transactions {
                cash.push(vec![
                    opt_text(transaction.date.as_deref()),
                    opt_text(transaction.transaction_type.as_deref()),
                    opt_number(transaction.amount),
                    opt_text(transaction.currency.as_deref()),
                    opt_text(transaction.description.as_deref()),
                ]);
            }
            print_table(&cash, mode)
        }
    }
}

/// Note the size of a bounded snapshot.
fn envelope_note(count: usize, label: &str, truncated: bool) {
    note(&format!("{label}: {count} (truncated: {truncated})"));
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
}
