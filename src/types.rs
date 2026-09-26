//! Wire types mirroring the gateway's JSON contract.
//!
//! Field names are `camelCase` on the wire and enum values use the gateway's
//! `SCREAMING_SNAKE_CASE` spellings. Timestamps stay as RFC 3339 strings: the
//! CLI prints them verbatim, and IB uses several date shapes that are not all
//! worth modelling.
//!
//! Request types reject values the gateway would reject (for example
//! [`OptionRight`] cannot express anything but `"C"`/`"P"`). Response types
//! stay permissive where the gateway owns the value, so an unfamiliar value
//! surfaces as a decoding error rather than a silently dropped field.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Common
// ---------------------------------------------------------------------------

/// Response of `GET /health`: gateway and broker connectivity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    /// `ok` when connected to TWS / IB Gateway, `degraded` otherwise.
    pub status: String,
    /// Whether the gateway currently holds a broker connection.
    pub connected: bool,
    /// TWS / IB Gateway protocol version.
    pub server_version: i32,
    /// API client id used by the gateway.
    pub client_id: i32,
    /// Next order id the gateway will hand out.
    pub next_order_id: i32,
    /// RFC 3339 timestamp of when the broker connection was established.
    pub connection_time: Option<String>,
    /// IANA name of the broker's time zone, for example `America/New_York`.
    pub time_zone: Option<String>,
}

/// Error envelope returned by the gateway for 4xx/5xx responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Human-readable message.
    pub error: String,
    /// Machine-readable category, decoded into [`crate::ErrorKind`].
    pub kind: String,
}

/// Envelope returned by bounded snapshot endpoints.
///
/// `truncated` is true when the gateway's item cap was reached, in which case
/// `count` is the number of items returned, not the broker total.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEnvelope<T> {
    /// The collected items.
    pub data: Vec<T>,
    /// Whether draining stopped at the item cap.
    pub truncated: bool,
    /// Number of items in `data`.
    pub count: usize,
}

/// Simple acknowledgement returned by the Flex config endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckResponse {
    /// Always `ok` on success.
    pub status: String,
    /// Report name, for query registration.
    pub report: Option<String>,
}

// ---------------------------------------------------------------------------
// Contracts
// ---------------------------------------------------------------------------

/// Option right: the wire values `"C"` and `"P"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionRight {
    /// Call.
    #[serde(rename = "C")]
    Call,
    /// Put.
    #[serde(rename = "P")]
    Put,
}

impl OptionRight {
    /// Wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Call => "C",
            Self::Put => "P",
        }
    }
}

/// Instrument specification sent to the gateway.
///
/// `sec_type` accepts IB wire codes (`STK`, `OPT`, `FUT`, `CASH`, `BOND`,
/// `FOP`, `IND`, `CRYPTO`, `CFD`, `BAG`, ...). Unset optional fields are
/// omitted rather than sent as `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractSpec {
    /// Symbol, for example `AAPL`.
    pub symbol: String,
    /// Security type; the gateway defaults to `STK`.
    pub sec_type: String,
    /// Exchange; the gateway defaults to `SMART`.
    pub exchange: String,
    /// Currency; the gateway defaults to `USD`.
    pub currency: String,
    /// IB contract id; `0` lets TWS resolve the contract.
    pub contract_id: i32,
    /// Option strike price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strike: Option<f64>,
    /// Option right.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<OptionRight>,
    /// `YYYYMMDD` (last trade date) or `YYYYMM` (contract month).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_trade_date_or_contract_month: Option<String>,
    /// Contract multiplier, for example `"100"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<String>,
    /// Primary exchange for ambiguous symbols.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_exchange: Option<String>,
    /// IB local symbol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_symbol: Option<String>,
}

impl ContractSpec {
    /// A stock on `SMART`/`USD`, the common case.
    pub fn stock(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            sec_type: "STK".to_owned(),
            exchange: "SMART".to_owned(),
            currency: "USD".to_owned(),
            contract_id: 0,
            strike: None,
            right: None,
            last_trade_date_or_contract_month: None,
            multiplier: None,
            primary_exchange: None,
            local_symbol: None,
        }
    }
}

/// Stable description of a resolved contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractSummary {
    /// IB contract id.
    pub contract_id: i32,
    /// Symbol.
    pub symbol: String,
    /// Security type.
    pub sec_type: String,
    /// Exchange.
    pub exchange: String,
    /// Currency.
    pub currency: String,
    /// Symbol within the primary exchange (OCC symbol for options).
    pub local_symbol: String,
    /// Option strike; absent for non-options.
    pub strike: Option<f64>,
    /// Option right; absent for non-options.
    pub right: Option<String>,
    /// ISO expiry (`YYYY-MM-DD`) or contract month (`YYYYMM`).
    pub last_trade_date_or_contract_month: Option<String>,
}

/// Body of `POST /api/v1/contracts/details`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractDetailsRequest {
    /// Instrument to resolve.
    pub contract: ContractSpec,
    /// Include expired contracts in the search.
    #[serde(default)]
    pub include_expired: bool,
}

/// Extended contract details returned by TWS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractDetailsResponse {
    /// Resolved contract.
    pub contract: ContractSummary,
    /// Market name.
    pub market_name: String,
    /// Minimum price increment.
    pub min_tick: f64,
    /// Order types accepted for this contract.
    pub order_types: Vec<String>,
    /// Exchanges the contract trades on.
    pub valid_exchanges: Vec<String>,
    /// Long company/instrument name.
    pub long_name: String,
    /// Industry classification.
    pub industry: String,
    /// Category classification.
    pub category: String,
    /// Subcategory classification.
    pub subcategory: String,
    /// IANA time zone of the contract's market.
    pub time_zone_id: String,
    /// Trading hours spec.
    pub trading_hours: Vec<String>,
    /// Liquid hours spec.
    pub liquid_hours: Vec<String>,
    /// Underlying symbol.
    pub under_symbol: String,
    /// Underlying security type.
    pub under_security_type: String,
}

/// One result of `GET /api/v1/contracts/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolSearchResponse {
    /// Matching contract.
    pub contract: ContractSummary,
    /// Security types derivable from this contract.
    pub derivative_security_types: Vec<String>,
}

// ---------------------------------------------------------------------------
// Market data
// ---------------------------------------------------------------------------

/// One OHLCV bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BarResponse {
    /// Bar timestamp (RFC 3339; daily bars are UTC midnight).
    pub date: String,
    /// Open.
    pub open: f64,
    /// High.
    pub high: f64,
    /// Low.
    pub low: f64,
    /// Close.
    pub close: f64,
    /// Traded volume.
    pub volume: f64,
    /// Volume-weighted average price.
    pub wap: f64,
    /// Number of trades.
    pub count: i32,
}

/// Body of `POST /api/v1/market-data/historical`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDataRequest {
    /// Instrument to query.
    pub contract: ContractSpec,
    /// IB bar size wire string, for example `"1 min"`, `"1 hour"`, `"1 day"`.
    pub bar_size: String,
    /// Look-back window, for example `"1 D"`, `"30 M"`, `"1 Y"`.
    pub duration: String,
    /// Optional anchor end time (RFC 3339); defaults to now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ending: Option<String>,
    /// `TRADES` (default) | `MIDPOINT` | `BID` | `ASK` | `BID_ASK` | `AGG_TRADES`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub what_to_show: Option<String>,
    /// `RTH` (default) | `ALL`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trading_hours: Option<String>,
}

/// Response of a historical bar query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricalDataResponse {
    /// Requested window start (RFC 3339).
    pub start: String,
    /// Requested window end (RFC 3339).
    pub end: String,
    /// Bars, oldest first.
    pub bars: Vec<BarResponse>,
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

/// One open position.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionResponse {
    /// Account holding the position.
    pub account: String,
    /// Instrument.
    pub contract: ContractSummary,
    /// Signed quantity.
    pub position: f64,
    /// Average cost per unit.
    pub average_cost: f64,
}

/// One account-summary tag value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummaryValue {
    /// Account id.
    pub account: String,
    /// Tag name, for example `NetLiquidation`.
    pub tag: String,
    /// Value as delivered by IBKR (string).
    pub value: String,
    /// Currency the value is denominated in.
    pub currency: String,
}

/// Latest account profit-and-loss snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PnLResponse {
    /// Profit and loss since the session opened.
    pub daily_pnl: f64,
    /// Unrealized profit and loss.
    pub unrealized_pnl: Option<f64>,
    /// Realized profit and loss.
    pub realized_pnl: Option<f64>,
}

// ---------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------

/// Order side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSide {
    /// Buy.
    Buy,
    /// Sell.
    Sell,
}

impl OrderSide {
    /// Wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }
}

/// Order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderType {
    /// Market order.
    Market,
    /// Limit order.
    Limit,
    /// Stop order.
    Stop,
    /// Stop-limit order.
    StopLimit,
}

/// Time in force.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimeInForce {
    /// Day order (default).
    #[default]
    Day,
    /// Good till cancelled.
    Gtc,
    /// Immediate or cancel.
    Ioc,
}

/// Body of `POST /api/v1/orders`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRequest {
    /// Instrument to trade.
    pub contract: ContractSpec,
    /// Buy or sell.
    pub side: OrderSide,
    /// Quantity (must be positive).
    pub quantity: f64,
    /// Order type.
    pub order_type: OrderType,
    /// Required for `LIMIT` and `STOP_LIMIT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<f64>,
    /// Required for `STOP` and `STOP_LIMIT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<f64>,
    /// Time in force.
    #[serde(default)]
    pub tif: TimeInForce,
    /// Allow execution outside regular trading hours.
    #[serde(default)]
    pub outside_rth: bool,
    /// Hide the order from market depth (NASDAQ only).
    #[serde(default)]
    pub hidden: bool,
}

impl OrderRequest {
    /// Check the price fields the gateway requires per order type.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] naming the missing price, so a bad
    /// request fails locally instead of as a gateway 400.
    pub fn validate(&self) -> crate::Result<()> {
        if self.quantity <= 0.0 {
            return Err(crate::Error::Invalid(
                "quantity must be positive".to_owned(),
            ));
        }
        let needs_limit = matches!(self.order_type, OrderType::Limit | OrderType::StopLimit);
        let needs_stop = matches!(self.order_type, OrderType::Stop | OrderType::StopLimit);
        if needs_limit && self.limit_price.is_none() {
            return Err(crate::Error::Invalid(format!(
                "limitPrice is required for {} orders",
                self.order_type
            )));
        }
        if needs_stop && self.stop_price.is_none() {
            return Err(crate::Error::Invalid(format!(
                "stopPrice is required for {} orders",
                self.order_type
            )));
        }
        Ok(())
    }
}

impl std::fmt::Display for OrderType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Market => "MARKET",
            Self::Limit => "LIMIT",
            Self::Stop => "STOP",
            Self::StopLimit => "STOP_LIMIT",
        })
    }
}

/// Body of `POST /api/v1/orders/bracket`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BracketOrderRequest {
    /// Instrument to trade.
    pub contract: ContractSpec,
    /// Buy or sell.
    pub side: OrderSide,
    /// Quantity (must be positive).
    pub quantity: f64,
    /// Entry limit price; omit for a market entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_limit: Option<f64>,
    /// Take-profit limit price.
    pub take_profit: f64,
    /// Stop-loss trigger price.
    pub stop_loss: f64,
}

impl BracketOrderRequest {
    /// Check the quantity and the profit/stop ordering.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Invalid`] when the quantity is not positive or
    /// the target/stop are on the wrong side of the entry.
    pub fn validate(&self) -> crate::Result<()> {
        if self.quantity <= 0.0 {
            return Err(crate::Error::Invalid(
                "quantity must be positive".to_owned(),
            ));
        }
        match (self.side, self.entry_limit) {
            // Bracket targets must bracket the entry: profit above and stop
            // below for a long, mirrored for a short. Getting this wrong
            // produces orders TWS rejects or, worse, triggers immediately.
            (OrderSide::Buy, Some(entry)) => {
                if self.take_profit <= entry {
                    return Err(crate::Error::Invalid(format!(
                        "takeProfit ({}) must be above the entry ({entry}) for a BUY",
                        self.take_profit
                    )));
                }
                if self.stop_loss >= entry {
                    return Err(crate::Error::Invalid(format!(
                        "stopLoss ({}) must be below the entry ({entry}) for a BUY",
                        self.stop_loss
                    )));
                }
            }
            (OrderSide::Sell, Some(entry)) => {
                if self.take_profit >= entry {
                    return Err(crate::Error::Invalid(format!(
                        "takeProfit ({}) must be below the entry ({entry}) for a SELL",
                        self.take_profit
                    )));
                }
                if self.stop_loss <= entry {
                    return Err(crate::Error::Invalid(format!(
                        "stopLoss ({}) must be above the entry ({entry}) for a SELL",
                        self.stop_loss
                    )));
                }
            }
            (_, None) => {
                if (self.take_profit - self.stop_loss).abs() <= f64::EPSILON {
                    return Err(crate::Error::Invalid(
                        "takeProfit and stopLoss must differ".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Response of `POST /api/v1/orders`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderIdResponse {
    /// Client order id, for cancellation and stream correlation.
    pub order_id: i32,
}

/// Response of `POST /api/v1/orders/bracket`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BracketOrderIdsResponse {
    /// Parent (entry) order id.
    pub parent: i32,
    /// Take-profit child order id.
    pub take_profit: i32,
    /// Stop-loss child order id.
    pub stop_loss: i32,
}

/// One open or completed order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResponse {
    /// Client order id.
    pub order_id: i32,
    /// Client id the order belongs to.
    pub client_id: i32,
    /// Broker permanent id.
    pub perm_id: i64,
    /// Symbol.
    pub symbol: String,
    /// Security type.
    pub sec_type: String,
    /// Action (`BUY` / `SELL`).
    pub action: String,
    /// Order quantity.
    pub total_quantity: f64,
    /// Order type.
    pub order_type: String,
    /// Limit price, when set.
    pub limit_price: Option<f64>,
    /// Stop/aux trigger price, when set.
    pub aux_price: Option<f64>,
    /// Time in force as reported by IB.
    pub tif: String,
    /// IB order status, for example `Submitted`.
    pub status: String,
    /// Filled quantity.
    pub filled: Option<f64>,
    /// Remaining quantity.
    pub remaining: Option<f64>,
    /// Average fill price.
    pub average_fill_price: Option<f64>,
    /// Last fill price.
    pub last_fill_price: Option<f64>,
    /// Parent order id for bracket children.
    pub parent_id: Option<i32>,
}

/// One execution, joined with its commission report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResponse {
    /// Client order id.
    pub order_id: i32,
    /// Client id.
    pub client_id: i32,
    /// IB execution id (join key with commission reports).
    pub execution_id: String,
    /// Execution time as reported by IB.
    pub time: String,
    /// Account.
    pub account: String,
    /// Execution venue.
    pub exchange: String,
    /// Side as reported by IB (`Bought` / `Sold`).
    pub side: String,
    /// Executed quantity.
    pub shares: f64,
    /// Execution price.
    pub price: f64,
    /// Broker permanent id.
    pub perm_id: i64,
    /// Commission, when its report arrived.
    pub commission: Option<f64>,
    /// Commission currency.
    pub commission_currency: Option<String>,
    /// Realized profit and loss, when reported.
    pub realized_pnl: Option<f64>,
}

/// Query filter for `GET /api/v1/orders/executions`.
#[derive(Debug, Clone, Default)]
pub struct ExecutionFilter {
    /// Account id.
    pub account: Option<String>,
    /// Symbol.
    pub symbol: Option<String>,
    /// `BUY` or `SELL`.
    pub side: Option<OrderSide>,
    /// Only executions from the last N days (`0` = no filter).
    pub last_n_days: Option<i32>,
    /// Maximum rows to return (gateway default 500, max 1000).
    pub limit: Option<usize>,
}

/// Response of `DELETE /api/v1/orders/{orderId}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelResponse {
    /// Cancelled order id.
    pub order_id: i32,
    /// Last status observed while draining the cancel acknowledgement.
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Flex reports
// ---------------------------------------------------------------------------

/// One executed trade from a Flex statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    /// IB transaction id (unique per trade).
    pub transaction_id: Option<String>,
    /// Account id.
    pub account_id: Option<String>,
    /// IB contract id.
    pub conid: Option<String>,
    /// Symbol.
    pub symbol: Option<String>,
    /// Instrument description.
    pub description: Option<String>,
    /// Asset category.
    pub asset_category: Option<String>,
    /// `B` or `S`.
    pub buy_sell: Option<String>,
    /// Trade date.
    pub trade_date: Option<String>,
    /// Settlement date.
    pub settle_date: Option<String>,
    /// Quantity.
    pub quantity: Option<f64>,
    /// Price.
    pub price: Option<f64>,
    /// Proceeds.
    pub proceeds: Option<f64>,
    /// Cost basis.
    pub cost: Option<f64>,
    /// Commission.
    pub commission: Option<f64>,
    /// Realized FIFO profit and loss.
    pub fifo_pnl_realized: Option<f64>,
    /// Currency.
    pub currency: Option<String>,
    /// Exchange.
    pub exchange: Option<String>,
    /// Underlying symbol.
    pub underlying_symbol: Option<String>,
    /// Underlying contract id.
    pub underlying_conid: Option<String>,
    /// Contract multiplier (100 for standard equity options).
    pub multiplier: Option<f64>,
    /// Option strike.
    pub strike: Option<f64>,
    /// Option expiry (`YYYY-MM-DD`).
    pub expiry: Option<String>,
    /// Option right, `C` or `P`.
    pub put_call: Option<String>,
    /// `O` for an opening trade, `C` for a closing one, `C;O` when a single
    /// execution closes and reopens the position.
    pub open_close: Option<String>,
    /// When the position this row closes was opened. Flex emits `date;time`
    /// (or `date time`); the tables show the date part.
    pub open_date_time: Option<String>,
    /// Execution time within `trade_date`, when the statement carries it.
    pub trade_time: Option<String>,
    /// Which projection this row came from: `EXECUTION`, `CLOSED_LOT`, `ORDER`,
    /// `SYMBOL_SUMMARY`, ...
    pub level_of_detail: Option<String>,
    /// Taxes on the trade.
    pub taxes: Option<f64>,
    /// Proceeds plus tax plus commission.
    pub net_cash: Option<f64>,
    /// Mark-to-market profit and loss (commissions excluded).
    pub mtm_pnl: Option<f64>,
    /// FX rate from the trade currency to the account's base currency.
    pub fx_rate_to_base: Option<f64>,
    /// IB order id.
    pub ib_order_id: Option<String>,
    /// IB execution id.
    pub exec_id: Option<String>,
}

/// One cash transaction (dividend, fee, deposit, ...) from a Flex statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashTransaction {
    /// IB transaction id.
    pub transaction_id: Option<String>,
    /// Account id.
    pub account_id: Option<String>,
    /// Type, for example `Deposits`, `Dividends`, `WithholdingTax`.
    pub transaction_type: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Amount.
    pub amount: Option<f64>,
    /// Currency.
    pub currency: Option<String>,
    /// FX rate to the account's base currency.
    pub fx_rate_to_base: Option<f64>,
    /// Date.
    pub date: Option<String>,
    /// Settlement date.
    pub settle_date: Option<String>,
    /// Ex-dividend date.
    pub ex_date: Option<String>,
    /// IB contract id.
    pub conid: Option<String>,
    /// Symbol.
    pub symbol: Option<String>,
    /// Which projection this row came from: `DETAIL` for the individual cash
    /// movements, `SUMMARY` for per-report-date aggregates of the same money.
    /// Summing both double counts; a `SUMMARY` row carries no transaction id.
    pub level_of_detail: Option<String>,
}

impl CashTransaction {
    /// Whether this row is an individual movement rather than an aggregate of
    /// the same money. The gateway returns both levels, so anything that adds
    /// up cash must ask first.
    pub(crate) fn is_detail(&self) -> bool {
        match self.level_of_detail.as_deref().map(str::trim) {
            Some(level) if !level.is_empty() => level.eq_ignore_ascii_case("DETAIL"),
            // Gateways that predate `levelOfDetail` on cash: the summary rows
            // are the ones without a transaction id.
            _ => self
                .transaction_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty()),
        }
    }
}

/// Normalize a Flex date-time to an ISO date, `None` when it carries none.
///
/// Flex separates date and time inconsistently and emits compact dates —
/// `20260608;155631`, `2025-01-15;100000`, `2025-01-15 10:00:00`,
/// `2025-01-15T10:00:00`. Only the date survives, normalized so it lines up
/// with the fields the gateway parses into dates.
pub(crate) fn flex_date(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    let date = text.split([' ', ',', ';', 'T']).next().unwrap_or(text);
    if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
        Some(format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8]))
    } else if date.is_empty() {
        None
    } else {
        Some(date.to_owned())
    }
}

/// One open position from a Flex statement's `OpenPositions` section.
///
/// A snapshot at the report date, not a windowed history: a position history
/// comes from syncing several report windows.
///
/// Every attribute is optional on the wire and maps to an `Option`, so an
/// absent attribute is absent here rather than a guessed default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlexPosition {
    /// IB contract id.
    pub conid: Option<String>,
    /// Symbol.
    pub symbol: Option<String>,
    /// Instrument description.
    pub description: Option<String>,
    /// Wire spelling (`STK`, `OPT`, `FUT`, ...).
    pub asset_category: Option<String>,
    /// Currency the values are stated in.
    pub currency: Option<String>,
    /// Signed quantity: negative is short.
    pub quantity: Option<f64>,
    /// Position value in `currency`.
    pub position_value: Option<f64>,
    /// Mark price per unit.
    pub mark_price: Option<f64>,
    /// Cost basis per unit.
    pub cost_basis_price: Option<f64>,
    /// Unrealized P/L, FIFO.
    pub fifo_pnl_unrealized: Option<f64>,
    /// Contract multiplier.
    pub multiplier: Option<f64>,
    /// Option strike.
    pub strike: Option<f64>,
    /// Option expiry (`YYYY-MM-DD`).
    pub expiry: Option<String>,
    /// Option right, `C` or `P`.
    pub put_call: Option<String>,
    /// Underlying contract id.
    pub underlying_conid: Option<String>,
    /// Underlying symbol.
    pub underlying_symbol: Option<String>,
    /// When the position was opened, verbatim (`20260807;120536`).
    pub open_date_time: Option<String>,
    /// The opening execution's transaction id — joins the position to the trade that opened it.
    pub originating_transaction_id: Option<String>,
    /// `Long`/`Short`, as the statement reports it.
    pub side: Option<String>,
    /// Row label when the section is multi-level.
    pub level_of_detail: Option<String>,
    /// Snapshot date (`YYYY-MM-DD`).
    pub report_date: Option<String>,
}

/// One currency's balances and movements from a Flex statement's `CashReport`
/// section.
///
/// The `BASE_SUMMARY` row is the account's base currency and arrives as its own
/// entry, never folded into the currencies.
///
/// Every attribute is optional on the wire and maps to an `Option`, so an
/// absent attribute is absent here rather than a guessed default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CashReportCurrency {
    /// Currency code, or `BASE_SUMMARY` for the base-currency row.
    pub currency: Option<String>,
    /// Window start this report covers.
    pub from_date: Option<String>,
    /// Window end.
    pub to_date: Option<String>,
    /// Level of detail.
    pub level_of_detail: Option<String>,
    /// Cash at the window start.
    pub starting_cash: Option<f64>,
    /// Cash at the window end.
    pub ending_cash: Option<f64>,
    /// Settled cash at the window end.
    pub ending_settled_cash: Option<f64>,
    /// Commissions paid.
    pub commissions: Option<f64>,
    /// Deposits.
    pub deposits: Option<f64>,
    /// Withdrawals.
    pub withdrawals: Option<f64>,
    /// Net deposits and withdrawals.
    pub deposit_withdrawals: Option<f64>,
    /// Dividends received.
    pub dividends: Option<f64>,
    /// Broker interest.
    pub broker_interest: Option<f64>,
    /// Bond interest.
    pub bond_interest: Option<f64>,
    /// Withholding tax.
    pub withholding_tax: Option<f64>,
    /// Other fees.
    pub other_fees: Option<f64>,
    /// Client fees.
    pub client_fees: Option<f64>,
    /// Broker fees.
    pub broker_fees: Option<f64>,
    /// Net trade sales.
    pub net_trades_sales: Option<f64>,
    /// Net trade purchases.
    pub net_trades_purchases: Option<f64>,
    /// Account transfers.
    pub account_transfers: Option<f64>,
    /// Internal transfers.
    pub internal_transfers: Option<f64>,
    /// External transfers.
    pub external_transfers: Option<f64>,
    /// FX translation P/L.
    pub fx_translation_pnl: Option<f64>,
    /// Realized forex P/L.
    pub realized_forex_pnl: Option<f64>,
    /// Cash settling mark-to-market.
    pub cash_settling_mtm: Option<f64>,
    /// Linking adjustments.
    pub linking_adjustments: Option<f64>,
    /// Transaction tax.
    pub transaction_tax: Option<f64>,
    /// Payments in lieu.
    pub payment_in_lieu: Option<f64>,
    /// Billable sales tax.
    pub billable_sales_tax: Option<f64>,
    /// Other income.
    pub other_income: Option<f64>,
}

/// Flex report envelope: trades and cash transactions in one response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlexReportResponse {
    /// Account the statement covers.
    pub account_id: Option<String>,
    /// Statement window start (authoritative; set by the query's Period).
    pub from_date: Option<String>,
    /// Statement window end.
    pub to_date: Option<String>,
    /// Executed trades.
    pub trades: Vec<Trade>,
    /// Closed-lot rows (`levelOfDetail` `CLOSED_LOT`), when the query asks for
    /// lot-level detail. Deliberately separate from [`Self::trades`]: a lot and
    /// its execution both carry `fifoPnlRealized`, so summing the two arrays
    /// double counts. Absent on gateways that do not map them yet.
    #[serde(default)]
    pub lots: Vec<Trade>,
    /// Open positions, from the statement's `OpenPositions` section (empty when
    /// the query does not request it).
    #[serde(default)]
    pub positions: Vec<FlexPosition>,
    /// Per-currency balances and movements, from the statement's `CashReport`
    /// section (empty when the query does not request it).
    #[serde(default)]
    pub cash_report: Vec<CashReportCurrency>,
    /// Cash transactions.
    pub cash_transactions: Vec<CashTransaction>,
}

/// Body of `PUT /api/v1/config/flex/queries/{reportName}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetQueryRequest {
    /// Flex query id.
    pub query_id: String,
}

/// Body of `PUT /api/v1/config/flex/token`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTokenRequest {
    /// Flex web-service token.
    pub token: String,
}

/// One registered Flex report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportStatus {
    /// Report name.
    pub name: String,
    /// Whether a query id is registered.
    pub query_configured: bool,
    /// Whether a token is available.
    pub token_configured: bool,
}

/// Response of `GET /api/v1/config/flex`. Never contains the token value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlexConfigResponse {
    /// Registered reports.
    pub reports: Vec<ReportStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Compare floats within a tolerance: exact equality is not meaningful for
    /// values that survived a JSON round trip.
    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn assert_opt_close(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("value present");
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn health_decodes_the_documented_document() {
        // Verbatim from API.md §1.1.
        let payload = json!({
            "status": "ok",
            "connected": true,
            "serverVersion": 213,
            "clientId": 100,
            "nextOrderId": 42,
            "connectionTime": "2026-08-15T13:00:00Z",
            "timeZone": "America/New_York"
        });
        let health: Health = serde_json::from_value(payload).unwrap();
        assert_eq!(health.status, "ok");
        assert!(health.connected);
        assert_eq!(health.server_version, 213);
        assert_eq!(health.client_id, 100);
        assert_eq!(health.next_order_id, 42);
        assert_eq!(
            health.connection_time.as_deref(),
            Some("2026-08-15T13:00:00Z")
        );
        assert_eq!(health.time_zone.as_deref(), Some("America/New_York"));
    }

    #[test]
    fn health_decodes_a_disconnected_gateway() {
        // A gateway that has not reached the broker omits both optional fields.
        let payload = json!({
            "status": "degraded",
            "connected": false,
            "serverVersion": 0,
            "clientId": 100,
            "nextOrderId": -1
        });
        let health: Health = serde_json::from_value(payload).unwrap();
        assert_eq!(health.status, "degraded");
        assert!(!health.connected);
        assert!(health.connection_time.is_none());
        assert!(health.time_zone.is_none());
    }

    #[test]
    fn wire_field_names_are_camel_case() {
        let health = Health {
            status: "ok".to_owned(),
            connected: true,
            server_version: 213,
            client_id: 100,
            next_order_id: 42,
            connection_time: None,
            time_zone: None,
        };
        let value = serde_json::to_value(&health).unwrap();
        assert!(value.get("serverVersion").is_some(), "{value}");
        assert!(value.get("nextOrderId").is_some(), "{value}");
        assert!(value.get("server_version").is_none(), "{value}");
    }

    #[test]
    fn snapshot_envelope_decodes_items_and_metadata() {
        // Verbatim from API.md §1.5 (one position row).
        let payload = json!({
            "data": [
                {
                    "account": "DU1234567",
                    "contract": {
                        "contractId": 265_598,
                        "symbol": "AAPL",
                        "secType": "STK",
                        "exchange": "SMART",
                        "currency": "USD",
                        "localSymbol": "AAPL"
                    },
                    "position": 100.0,
                    "averageCost": 182.43
                }
            ],
            "truncated": false,
            "count": 1
        });
        let envelope: SnapshotEnvelope<PositionResponse> = serde_json::from_value(payload).unwrap();
        assert_eq!(envelope.count, 1);
        assert!(!envelope.truncated);
        assert_eq!(envelope.data[0].account, "DU1234567");
        assert_eq!(envelope.data[0].contract.symbol, "AAPL");
        assert_close(envelope.data[0].average_cost, 182.43);
    }

    #[test]
    fn api_error_body_decodes() {
        let payload = json!({"error": "missing or invalid bearer token", "kind": "unauthorized"});
        let body: ApiErrorBody = serde_json::from_value(payload).unwrap();
        assert_eq!(body.error, "missing or invalid bearer token");
        assert_eq!(body.kind, "unauthorized");
    }

    #[test]
    fn enums_serialize_to_the_gateway_spellings() {
        assert_eq!(serde_json::to_value(OrderSide::Buy).unwrap(), json!("BUY"));
        assert_eq!(
            serde_json::to_value(OrderSide::Sell).unwrap(),
            json!("SELL")
        );
        assert_eq!(
            serde_json::to_value(OrderType::StopLimit).unwrap(),
            json!("STOP_LIMIT")
        );
        assert_eq!(
            serde_json::to_value(TimeInForce::Gtc).unwrap(),
            json!("GTC")
        );
        assert_eq!(
            serde_json::from_value::<TimeInForce>(json!("IOC")).unwrap(),
            TimeInForce::Ioc
        );
    }

    #[test]
    fn option_right_round_trips_as_c_and_p() {
        assert_eq!(serde_json::to_value(OptionRight::Call).unwrap(), json!("C"));
        assert_eq!(serde_json::to_value(OptionRight::Put).unwrap(), json!("P"));
        assert_eq!(
            serde_json::from_value::<OptionRight>(json!("P")).unwrap(),
            OptionRight::Put
        );
        assert!(serde_json::from_value::<OptionRight>(json!("X")).is_err());
        assert_eq!(OptionRight::Call.as_wire(), "C");
    }

    #[test]
    fn contract_spec_omits_unset_fields() {
        let spec = ContractSpec::stock("AAPL");
        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            value,
            json!({
                "symbol": "AAPL",
                "secType": "STK",
                "exchange": "SMART",
                "currency": "USD",
                "contractId": 0
            }),
            "unset optionals must be omitted, not sent as null"
        );
    }

    #[test]
    fn contract_spec_carries_option_identity() {
        let mut spec = ContractSpec::stock("HL");
        spec.sec_type = "OPT".to_owned();
        spec.strike = Some(25.0);
        spec.right = Some(OptionRight::Call);
        spec.last_trade_date_or_contract_month = Some("20260821".to_owned());

        let value = serde_json::to_value(&spec).unwrap();
        assert_eq!(value["secType"], "OPT");
        assert_eq!(value["strike"], 25.0);
        assert_eq!(value["right"], "C");
        assert_eq!(value["lastTradeDateOrContractMonth"], "20260821");
    }

    #[test]
    fn historical_request_matches_the_documented_body() {
        let request = HistoricalDataRequest {
            contract: ContractSpec::stock("AAPL"),
            bar_size: "1 hour".to_owned(),
            duration: "30 D".to_owned(),
            ending: Some("2026-08-15T20:00:00Z".to_owned()),
            what_to_show: Some("TRADES".to_owned()),
            trading_hours: Some("RTH".to_owned()),
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["barSize"], "1 hour");
        assert_eq!(value["duration"], "30 D");
        assert_eq!(value["ending"], "2026-08-15T20:00:00Z");
        assert_eq!(value["contract"]["symbol"], "AAPL");

        // Optional fields default to absent, not null.
        let minimal = HistoricalDataRequest {
            contract: ContractSpec::stock("AAPL"),
            bar_size: "1 day".to_owned(),
            duration: "1 Y".to_owned(),
            ending: None,
            what_to_show: None,
            trading_hours: None,
        };
        let value = serde_json::to_value(&minimal).unwrap();
        assert!(value.get("ending").is_none(), "{value}");
        assert!(value.get("whatToShow").is_none(), "{value}");
    }

    #[test]
    fn order_request_serializes_prices_and_flags() {
        let request = OrderRequest {
            contract: ContractSpec::stock("AAPL"),
            side: OrderSide::Buy,
            quantity: 100.0,
            order_type: OrderType::Limit,
            limit_price: Some(210.0),
            stop_price: None,
            tif: TimeInForce::Day,
            outside_rth: true,
            hidden: false,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["side"], "BUY");
        assert_eq!(value["orderType"], "LIMIT");
        assert_eq!(value["limitPrice"], 210.0);
        assert_eq!(value["tif"], "DAY");
        assert_eq!(value["outsideRth"], true);
        assert!(value.get("stopPrice").is_none(), "{value}");
    }

    #[test]
    fn order_validation_matches_the_gateway_rules() {
        let mut request = OrderRequest {
            contract: ContractSpec::stock("AAPL"),
            side: OrderSide::Buy,
            quantity: 100.0,
            order_type: OrderType::StopLimit,
            limit_price: None,
            stop_price: None,
            tif: TimeInForce::Day,
            outside_rth: false,
            hidden: false,
        };
        assert!(request.validate().is_err(), "both prices are required");

        request.limit_price = Some(210.0);
        assert!(request.validate().is_err(), "stop price still missing");

        request.stop_price = Some(205.0);
        assert!(request.validate().is_ok());

        request.quantity = 0.0;
        assert!(request.validate().is_err(), "quantity must be positive");
    }

    #[test]
    fn bracket_validation_keeps_targets_around_the_entry() {
        let bracket = |side: OrderSide, entry: Option<f64>, profit: f64, stop: f64| {
            BracketOrderRequest {
                contract: ContractSpec::stock("AAPL"),
                side,
                quantity: 100.0,
                entry_limit: entry,
                take_profit: profit,
                stop_loss: stop,
            }
            .validate()
        };

        assert!(bracket(OrderSide::Buy, Some(210.0), 225.0, 198.0).is_ok());
        assert!(bracket(OrderSide::Sell, Some(210.0), 198.0, 225.0).is_ok());

        // Inverted targets would trigger immediately.
        assert!(bracket(OrderSide::Buy, Some(210.0), 205.0, 198.0).is_err());
        assert!(bracket(OrderSide::Buy, Some(210.0), 225.0, 215.0).is_err());
        assert!(bracket(OrderSide::Sell, Some(210.0), 215.0, 225.0).is_err());
        assert!(bracket(OrderSide::Sell, Some(210.0), 198.0, 205.0).is_err());

        // A market entry has no price to bracket, but the targets must differ.
        assert!(bracket(OrderSide::Buy, None, 225.0, 198.0).is_ok());
        assert!(bracket(OrderSide::Buy, None, 200.0, 200.0).is_err());
    }

    #[test]
    fn executions_and_cancel_decode() {
        // Verbatim from API.md §1.11.
        let execution: ExecutionResponse = serde_json::from_value(json!({
            "orderId": 42,
            "clientId": 100,
            "executionId": "0000f43a.66a12345.01.01",
            "time": "20260815  13:45:02",
            "account": "DU1234567",
            "exchange": "NASDAQ",
            "side": "Bought",
            "shares": 100,
            "price": 209.95,
            "permId": 123_456_789,
            "commission": 0.35,
            "commissionCurrency": "USD",
            "realizedPnl": null
        }))
        .unwrap();
        assert_eq!(execution.side, "Bought");
        assert_close(execution.shares, 100.0);
        assert_opt_close(execution.commission, 0.35);
        assert!(execution.realized_pnl.is_none());

        let cancel: CancelResponse =
            serde_json::from_value(json!({"orderId": 42, "status": "Cancelled"})).unwrap();
        assert_eq!(cancel.order_id, 42);
        assert_eq!(cancel.status.as_deref(), Some("Cancelled"));
    }

    #[test]
    fn flex_report_decodes_both_sections() {
        let report: FlexReportResponse = serde_json::from_value(json!({
            "accountId": "DU1234567",
            "fromDate": "2026-08-01",
            "toDate": "2026-08-31",
            "trades": [
                {
                    "transactionId": "1",
                    "symbol": "AAPL",
                    "buySell": "B",
                    "quantity": 100.0,
                    "price": 209.95,
                    "commission": 0.35,
                    "fifoPnlRealized": null
                }
            ],
            "cashTransactions": [
                {
                    "transactionId": "2",
                    "transactionType": "Dividends",
                    "amount": 12.5,
                    "currency": "USD"
                }
            ]
        }))
        .unwrap();
        assert_eq!(report.account_id.as_deref(), Some("DU1234567"));
        assert_eq!(report.trades.len(), 1);
        assert_eq!(report.trades[0].buy_sell.as_deref(), Some("B"));
        assert!(report.trades[0].fifo_pnl_realized.is_none());
        assert_eq!(
            report.cash_transactions[0].transaction_type.as_deref(),
            Some("Dividends")
        );
    }

    #[test]
    fn flex_config_decodes_report_statuses() {
        let config: FlexConfigResponse = serde_json::from_value(json!({
            "reports": [
                {"name": "transactions_30d", "queryConfigured": true, "tokenConfigured": false}
            ]
        }))
        .unwrap();
        assert_eq!(config.reports.len(), 1);
        assert!(config.reports[0].query_configured);
        assert!(!config.reports[0].token_configured);
    }

    #[test]
    fn contract_details_decode() {
        // Verbatim shape from API.md §1.2.
        let details: ContractDetailsResponse = serde_json::from_value(json!({
            "contract": {
                "contractId": 265_598,
                "symbol": "AAPL",
                "secType": "STK",
                "exchange": "SMART",
                "currency": "USD",
                "localSymbol": "AAPL"
            },
            "marketName": "AAPL",
            "minTick": 0.01,
            "orderTypes": ["MARKET", "LIMIT"],
            "validExchanges": ["NYSE", "NASDAQ"],
            "longName": "APPLE INC",
            "industry": "Technology",
            "category": "Computer Manufacturing",
            "subcategory": "Computers",
            "timeZoneId": "America/New_York",
            "tradingHours": ["20260815:0930-1600"],
            "liquidHours": ["20260815:0930-1600"],
            "underSymbol": "",
            "underSecurityType": ""
        }))
        .unwrap();
        assert_eq!(details.long_name, "APPLE INC");
        assert_close(details.min_tick, 0.01);
        assert_eq!(details.order_types, ["MARKET", "LIMIT"]);
        assert_eq!(details.time_zone_id, "America/New_York");
    }
}
