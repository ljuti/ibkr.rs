//! Command-line interface definition.
//!
//! The command tree mirrors the gateway's surface: REST groups (`contracts`,
//! `market-data`, `accounts`, `orders`, `flex`) plus `stream` for the WebSocket
//! endpoint. `stream` is declared but not implemented yet.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::config::ConfigOverrides;
use crate::types::{ContractSpec, OptionRight, OrderSide, OrderType, TimeInForce};

/// CLI client for the IBKR gateway.
#[derive(Debug, Parser)]
#[command(
    name = "ibkr",
    version,
    about = "CLI client for the IBKR gateway (REST + WebSocket)",
    long_about = "CLI client for the IBKR gateway.\n\n\
        The gateway owns the connection to TWS / IB Gateway; this client talks \
        to it over mutual TLS with a bearer token. Connection settings come \
        from flags, then environment variables, then defaults \
        (see .env.example).\n\n\
        Order placement, bracket placement and cancellation prompt for \
        confirmation; pass --yes to skip the prompt in scripts.",
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Connection settings, shared by every command.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Connection and output settings, accepted before or after the subcommand.
#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// Gateway base URL (env: `IBKR_GATEWAY_URL`).
    #[arg(short = 'u', long, value_name = "URL", global = true)]
    pub url: Option<String>,

    /// Bearer token (env: `IBKR_GATEWAY_TOKEN`).
    #[arg(short = 't', long, value_name = "TOKEN", global = true)]
    pub token: Option<String>,

    /// PEM file with the CA that signed the gateway certificate
    /// (env: `IBKR_GATEWAY_CA_CERT`).
    #[arg(long, value_name = "PATH", global = true)]
    pub ca_cert: Option<PathBuf>,

    /// PEM client certificate for mutual TLS (env: `IBKR_GATEWAY_CLIENT_CERT`).
    #[arg(long, value_name = "PATH", global = true)]
    pub client_cert: Option<PathBuf>,

    /// PEM client private key for mutual TLS (env: `IBKR_GATEWAY_CLIENT_KEY`).
    #[arg(long, value_name = "PATH", global = true)]
    pub client_key: Option<PathBuf>,

    /// Request timeout in seconds (env: `IBKR_GATEWAY_TIMEOUT_SEC`).
    #[arg(long, value_name = "SECS", global = true)]
    pub timeout: Option<u64>,

    /// Retries for rate-limited reads; 0 disables (env: `IBKR_GATEWAY_MAX_RETRIES`).
    ///
    /// Reads only: order placement and cancellation are never retried.
    #[arg(long, value_name = "N", global = true)]
    pub max_retries: Option<u32>,

    /// Accept invalid server certificates. Development only.
    #[arg(long, global = true)]
    pub tls_skip_verify: bool,

    /// Local store database file, used by `store` (env: `IBKR_STORE_DB`).
    #[arg(long, value_name = "PATH", global = true)]
    pub db: Option<PathBuf>,

    /// Configuration file read by every command (env: `IBKR_CONFIG`).
    #[arg(long, value_name = "PATH", global = true)]
    pub config: Option<PathBuf>,

    /// Output format.
    #[arg(short = 'o', long, value_enum, default_value_t = Output::Json, global = true)]
    pub output: Output,

    /// Do not shrink table columns to the terminal width.
    #[arg(long, global = true)]
    pub no_truncate: bool,
}

impl GlobalArgs {
    /// Build configuration overrides from the parsed command line.
    pub fn overrides(&self) -> ConfigOverrides {
        ConfigOverrides {
            base_url: self.url.clone(),
            token: self.token.clone(),
            ca_cert: self.ca_cert.clone(),
            client_cert: self.client_cert.clone(),
            client_key: self.client_key.clone(),
            timeout: self.timeout.map(Duration::from_secs),
            max_retries: self.max_retries,
            tls_skip_verify: self.tls_skip_verify.then_some(true),
            db: self.db.clone(),
            config: self.config.clone(),
        }
    }

    /// How responses should be rendered.
    pub fn render_mode(&self) -> RenderMode {
        RenderMode {
            output: self.output,
            truncate: !self.no_truncate,
        }
    }
}

/// How responses are rendered on stdout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Output {
    /// Pretty-printed JSON.
    #[default]
    Json,
    /// Aligned columns for humans.
    Table,
}

/// Output selection plus the terminal-fitting decision.
#[derive(Debug, Clone, Copy)]
pub struct RenderMode {
    /// Selected format.
    pub output: Output,
    /// Whether tables shrink to the terminal width.
    pub truncate: bool,
}

impl RenderMode {
    /// Width available for a table, `None` when the width is unknown (piped
    /// output) or truncation is switched off.
    pub fn available_width(self) -> Option<usize> {
        if !self.truncate {
            return None;
        }
        // A pty with no window size reports 0 (or a nonsense width); shrinking
        // to that would render one character per column, so treat it as unknown.
        terminal_size::terminal_size()
            .map(|(width, _)| usize::from(width.0))
            .filter(|width| *width >= MIN_TABLE_WIDTH)
    }
}

/// Below this width the terminal is not a usable table surface.
const MIN_TABLE_WIDTH: usize = 40;

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Gateway and broker connectivity.
    Health,

    /// Write the settings every command resolves, so `.env` is not needed.
    ///
    /// With no flags and a terminal, prompts for each setting (showing what
    /// each one currently resolves to); otherwise takes whatever the global
    /// flags supplied. `--show` prints the effective settings, where each came
    /// from, and the file they would be written to.
    Configure {
        /// Print the effective settings and their sources; write nothing.
        #[arg(long)]
        show: bool,

        /// Check the gateway with the saved settings before finishing.
        #[arg(long)]
        verify: bool,
    },

    /// Contract resolution and symbol search.
    #[command(subcommand)]
    Contracts(ContractsCommand),

    /// Historical market data.
    #[command(name = "market-data", subcommand)]
    MarketData(MarketDataCommand),

    /// Portfolio, positions and account values.
    #[command(subcommand)]
    Accounts(AccountsCommand),

    /// Order management and executions.
    #[command(subcommand)]
    Orders(OrdersCommand),

    /// Flex reports and Flex credential configuration.
    #[command(subcommand)]
    Flex(FlexCommand),

    /// Local `SQLite` store of Flex statement data.
    #[command(subcommand)]
    Store(StoreCommand),

    /// WebSocket streams (market data, order updates, account values).
    #[command(subcommand)]
    Stream(StreamCommand),
}

impl Command {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Configure { .. } => "configure",
            Self::Contracts(command) => command.label(),
            Self::MarketData(command) => command.label(),
            Self::Accounts(command) => command.label(),
            Self::Orders(command) => command.label(),
            Self::Flex(command) => command.label(),
            Self::Store(command) => command.label(),
            Self::Stream(command) => command.label(),
        }
    }
}

/// Instrument specification flags, shared by every command that takes one.
#[derive(Debug, Clone, Args)]
pub struct ContractArgs {
    /// Symbol, for example `AAPL`.
    #[arg(short = 's', long, value_name = "SYMBOL")]
    pub symbol: String,

    /// Security type: `STK`, `OPT`, `FUT`, `CASH`, `BOND`, `FOP`, `IND`, `CRYPTO`, `CFD`, `BAG`.
    #[arg(long, default_value = "STK", value_name = "TYPE")]
    pub sec_type: String,

    /// Exchange.
    #[arg(long, default_value = "SMART", value_name = "EXCHANGE")]
    pub exchange: String,

    /// Currency.
    #[arg(long, default_value = "USD", value_name = "CURRENCY")]
    pub currency: String,

    /// IB contract id; 0 resolves the contract from the other fields.
    #[arg(long, default_value_t = 0, value_name = "ID")]
    pub contract_id: i32,

    /// Option strike price.
    #[arg(long, value_name = "PRICE")]
    pub strike: Option<f64>,

    /// Option right.
    #[arg(long, value_enum, ignore_case = true, value_name = "C|P")]
    pub right: Option<RightArg>,

    /// Expiry: `YYYYMMDD` (last trade date) or `YYYYMM` (contract month).
    #[arg(long, value_name = "DATE")]
    pub expiry: Option<String>,

    /// Contract multiplier, for example `100`.
    #[arg(long, value_name = "MULTIPLIER")]
    pub multiplier: Option<String>,

    /// Primary exchange, for symbols listed on several.
    #[arg(long, value_name = "EXCHANGE")]
    pub primary_exchange: Option<String>,

    /// IB local symbol.
    #[arg(long, value_name = "SYMBOL")]
    pub local_symbol: Option<String>,
}

impl ContractArgs {
    /// Build the wire specification.
    pub fn to_spec(&self) -> ContractSpec {
        ContractSpec {
            symbol: self.symbol.clone(),
            sec_type: self.sec_type.clone(),
            exchange: self.exchange.clone(),
            currency: self.currency.clone(),
            contract_id: self.contract_id,
            strike: self.strike,
            right: self.right.map(OptionRight::from),
            last_trade_date_or_contract_month: self.expiry.clone(),
            multiplier: self.multiplier.clone(),
            primary_exchange: self.primary_exchange.clone(),
            local_symbol: self.local_symbol.clone(),
        }
    }

    /// One-line description of the instrument, for confirmation prompts.
    pub fn describe(&self) -> String {
        use std::fmt::Write as _;

        let mut description = format!("{} ({})", self.symbol, self.sec_type);
        if self.sec_type != "STK" {
            let _ = write!(description, " on {}", self.exchange);
        }
        if let Some(right) = self.right {
            let _ = write!(description, " {}", right.as_wire());
        }
        if let Some(strike) = self.strike {
            let _ = write!(description, " strike {strike}");
        }
        if let Some(expiry) = &self.expiry {
            let _ = write!(description, " expiry {expiry}");
        }
        description
    }
}

/// Option right on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RightArg {
    /// Call.
    #[value(name = "C")]
    Call,
    /// Put.
    #[value(name = "P")]
    Put,
}

impl RightArg {
    /// Wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Call => "C",
            Self::Put => "P",
        }
    }
}

impl From<RightArg> for OptionRight {
    fn from(right: RightArg) -> Self {
        match right {
            RightArg::Call => Self::Call,
            RightArg::Put => Self::Put,
        }
    }
}

/// Order side on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SideArg {
    /// Buy.
    #[value(name = "BUY")]
    Buy,
    /// Sell.
    #[value(name = "SELL")]
    Sell,
}

impl From<SideArg> for OrderSide {
    fn from(side: SideArg) -> Self {
        match side {
            SideArg::Buy => Self::Buy,
            SideArg::Sell => Self::Sell,
        }
    }
}

/// Order type on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OrderTypeArg {
    /// Market order.
    #[value(name = "MARKET")]
    Market,
    /// Limit order.
    #[value(name = "LIMIT")]
    Limit,
    /// Stop order.
    #[value(name = "STOP")]
    Stop,
    /// Stop-limit order.
    #[value(name = "STOP_LIMIT")]
    StopLimit,
}

impl From<OrderTypeArg> for OrderType {
    fn from(order_type: OrderTypeArg) -> Self {
        match order_type {
            OrderTypeArg::Market => Self::Market,
            OrderTypeArg::Limit => Self::Limit,
            OrderTypeArg::Stop => Self::Stop,
            OrderTypeArg::StopLimit => Self::StopLimit,
        }
    }
}

/// Time in force on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TifArg {
    /// Day order.
    #[value(name = "DAY")]
    Day,
    /// Good till cancelled.
    #[value(name = "GTC")]
    Gtc,
    /// Immediate or cancel.
    #[value(name = "IOC")]
    Ioc,
}

impl From<TifArg> for TimeInForce {
    fn from(tif: TifArg) -> Self {
        match tif {
            TifArg::Day => Self::Day,
            TifArg::Gtc => Self::Gtc,
            TifArg::Ioc => Self::Ioc,
        }
    }
}

/// Bar content selector on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WhatToShowArg {
    /// Executed trades.
    #[value(name = "TRADES")]
    Trades,
    /// Midpoint of bid and ask.
    #[value(name = "MIDPOINT")]
    Midpoint,
    /// Bid prices.
    #[value(name = "BID")]
    Bid,
    /// Ask prices.
    #[value(name = "ASK")]
    Ask,
    /// Both bid and ask.
    #[value(name = "BID_ASK")]
    BidAsk,
    /// Aggregated trades (crypto).
    #[value(name = "AGG_TRADES")]
    AggTrades,
}

impl WhatToShowArg {
    /// Wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Trades => "TRADES",
            Self::Midpoint => "MIDPOINT",
            Self::Bid => "BID",
            Self::Ask => "ASK",
            Self::BidAsk => "BID_ASK",
            Self::AggTrades => "AGG_TRADES",
        }
    }
}

/// Trading-hours selector on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TradingHoursArg {
    /// Regular trading hours only.
    #[value(name = "RTH")]
    Rth,
    /// All hours, including extended sessions.
    #[value(name = "ALL")]
    All,
}

impl TradingHoursArg {
    /// Wire spelling.
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Rth => "RTH",
            Self::All => "ALL",
        }
    }
}

/// Contract commands.
#[derive(Debug, Subcommand)]
pub enum ContractsCommand {
    /// Resolve an instrument into full contract details.
    Details {
        /// Instrument to resolve.
        #[command(flatten)]
        contract: ContractArgs,

        /// Include expired contracts in the search.
        #[arg(long)]
        include_expired: bool,
    },

    /// Search for instruments matching a symbol pattern.
    Search {
        /// Pattern to match against symbol names.
        pattern: String,
    },
}

impl ContractsCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Details { .. } => "contracts details",
            Self::Search { .. } => "contracts search",
        }
    }
}

/// Historical market-data commands.
#[derive(Debug, Subcommand)]
pub enum MarketDataCommand {
    /// Fetch historical OHLCV bars.
    Historical {
        /// Instrument to query.
        #[command(flatten)]
        contract: ContractArgs,

        /// IB bar size, for example `"1 min"`, `"1 hour"`, `"1 day"`.
        #[arg(long, value_name = "SIZE")]
        bar_size: String,

        /// Look-back window, for example `"1 D"`, `"30 M"`, `"1 Y"`.
        #[arg(long, value_name = "DURATION")]
        duration: String,

        /// Anchor end time (RFC 3339); defaults to now.
        #[arg(long, value_name = "RFC3339")]
        ending: Option<String>,

        /// Bar content.
        #[arg(long, value_enum, ignore_case = true, value_name = "WHAT")]
        what_to_show: Option<WhatToShowArg>,

        /// Trading hours.
        #[arg(long, value_enum, ignore_case = true, value_name = "HOURS")]
        trading_hours: Option<TradingHoursArg>,
    },
}

impl MarketDataCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Historical { .. } => "market-data historical",
        }
    }
}

/// Account commands.
#[derive(Debug, Subcommand)]
pub enum AccountsCommand {
    /// Snapshot of open positions.
    Positions {
        /// Maximum rows to return (gateway default 500, maximum 1000).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },

    /// Account summary tags (net liquidation, buying power, margins, ...).
    Summary {
        /// Comma-separated tags; defaults to the gateway's full set.
        #[arg(long, value_name = "TAG,TAG")]
        tags: Option<String>,
    },

    /// Latest profit-and-loss snapshot for an account.
    Pnl {
        /// Account id, for example `DU1234567`.
        account: String,
    },
}

impl AccountsCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Positions { .. } => "accounts positions",
            Self::Summary { .. } => "accounts summary",
            Self::Pnl { .. } => "accounts pnl",
        }
    }
}

/// Order commands.
#[derive(Debug, Subcommand)]
pub enum OrdersCommand {
    /// Place a single order.
    Place {
        /// Instrument to trade.
        #[command(flatten)]
        contract: ContractArgs,

        /// Buy or sell.
        #[arg(long, value_enum, ignore_case = true, value_name = "SIDE")]
        side: SideArg,

        /// Quantity.
        #[arg(long, value_name = "QTY")]
        quantity: f64,

        /// Order type.
        #[arg(long = "type", value_enum, ignore_case = true, value_name = "TYPE")]
        order_type: OrderTypeArg,

        /// Limit price; required for `LIMIT` and `STOP_LIMIT`.
        #[arg(long, value_name = "PRICE")]
        limit_price: Option<f64>,

        /// Stop price; required for `STOP` and `STOP_LIMIT`.
        #[arg(long, value_name = "PRICE")]
        stop_price: Option<f64>,

        /// Time in force.
        #[arg(long, value_enum, ignore_case = true, default_value_t = TifArg::Day, value_name = "TIF")]
        tif: TifArg,

        /// Allow execution outside regular trading hours.
        #[arg(long)]
        outside_rth: bool,

        /// Hide the order from market depth (NASDAQ only).
        #[arg(long)]
        hidden: bool,

        /// Idempotency key; generated when omitted.
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Place a bracket order (entry + take-profit + stop-loss).
    Bracket {
        /// Instrument to trade.
        #[command(flatten)]
        contract: ContractArgs,

        /// Buy or sell.
        #[arg(long, value_enum, ignore_case = true, value_name = "SIDE")]
        side: SideArg,

        /// Quantity.
        #[arg(long, value_name = "QTY")]
        quantity: f64,

        /// Take-profit limit price.
        #[arg(long, value_name = "PRICE")]
        take_profit: f64,

        /// Stop-loss trigger price.
        #[arg(long, value_name = "PRICE")]
        stop_loss: f64,

        /// Entry limit price; omit for a market entry.
        #[arg(long, value_name = "PRICE")]
        entry_limit: Option<f64>,

        /// Idempotency key; generated when omitted.
        #[arg(long, value_name = "KEY")]
        idempotency_key: Option<String>,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// List currently open orders.
    List,

    /// List orders completed today.
    Completed {
        /// Also include orders placed manually in TWS.
        #[arg(long)]
        include_manual: bool,
    },

    /// List executions, joined with their commissions.
    Executions {
        /// Account id.
        #[arg(long, value_name = "ACCOUNT")]
        account: Option<String>,

        /// Symbol.
        #[arg(long, value_name = "SYMBOL")]
        symbol: Option<String>,

        /// Restrict to one side.
        #[arg(long, value_enum, ignore_case = true, value_name = "SIDE")]
        side: Option<SideArg>,

        /// Only executions from the last N days.
        #[arg(long, value_name = "N")]
        last_n_days: Option<i32>,

        /// Maximum rows to return (gateway default 500, maximum 1000).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },

    /// Cancel an order by client order id.
    Cancel {
        /// Client order id returned when the order was placed.
        order_id: i32,

        /// Skip the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

impl OrdersCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Place { .. } => "orders place",
            Self::Bracket { .. } => "orders bracket",
            Self::List => "orders list",
            Self::Completed { .. } => "orders completed",
            Self::Executions { .. } => "orders executions",
            Self::Cancel { .. } => "orders cancel",
        }
    }

    /// Whether this command changes broker state.
    pub fn is_mutation(&self) -> bool {
        matches!(
            self,
            Self::Place { .. } | Self::Bracket { .. } | Self::Cancel { .. }
        )
    }
}

/// Flex report and configuration commands.
#[derive(Debug, Subcommand)]
pub enum FlexCommand {
    /// Show the registered Flex reports and credential status.
    Config,

    /// Register or rotate a report's Flex query id.
    SetQuery {
        /// Report name the query is registered under.
        report_name: String,

        /// Flex query id.
        query_id: String,
    },

    /// Register or rotate the Flex web-service token.
    SetToken {
        /// Flex web-service token.
        ///
        /// Deliberately not named `token`: a field with that name shares clap's
        /// arg id with the global `--token` and silently becomes the bearer
        /// credential instead.
        #[arg(value_name = "TOKEN")]
        flex_token: String,
    },

    /// Fetch a report (trades and cash transactions).
    Report {
        /// Registered report name.
        report_name: String,

        /// Bypass the gateway cache and force a fresh fetch.
        #[arg(long)]
        refresh: bool,

        /// Entity tag from a previous response, sent as `If-None-Match`.
        #[arg(long, value_name = "ETAG")]
        etag: Option<String>,
    },
}

impl FlexCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Config => "flex config",
            Self::SetQuery { .. } => "flex set-query",
            Self::SetToken { .. } => "flex set-token",
            Self::Report { .. } => "flex report",
        }
    }
}

/// Local trade store: Flex statements, queryable with SQL.
#[derive(Debug, Subcommand)]
pub enum StoreCommand {
    /// Fetch Flex reports and upsert them into the store.
    ///
    /// Rows are keyed by their IB transaction id, so re-syncing, re-fetching a
    /// corrected statement, or syncing overlapping report windows all leave one
    /// row per execution.
    Sync {
        /// Registered report name; repeat to sync several.
        #[arg(
            short = 'r',
            long = "report",
            value_name = "NAME",
            required = true,
            action = clap::ArgAction::Append
        )]
        reports: Vec<String>,

        /// Ignore stored `ETag`s and fetch every report afresh.
        #[arg(long)]
        refresh: bool,
    },

    /// Run a read-only SQL query against the store.
    Query {
        /// SQL statement, for example `SELECT * FROM pnl_by_symbol LIMIT 10`.
        sql: String,
    },

    /// List the store's tables, views and columns.
    Schema,
}

impl StoreCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Sync { .. } => "store sync",
            Self::Query { .. } => "store query",
            Self::Schema => "store schema",
        }
    }
}

/// Streaming commands. Declared, not implemented.
#[derive(Debug, Subcommand)]
pub enum StreamCommand {
    /// Five-second real-time bars.
    Bars,

    /// Raw market-data ticks, optionally with IB generic ticks.
    MarketData,

    /// Tick-by-tick trades, bid/ask or midpoint.
    TickByTick,

    /// Order lifecycle updates.
    Orders,

    /// Live account values (per-currency cash balances and related metrics).
    AccountValues,
}

impl StreamCommand {
    /// Human-readable name used in diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Bars => "stream bars",
            Self::MarketData => "stream market-data",
            Self::TickByTick => "stream tick-by-tick",
            Self::Orders => "stream orders",
            Self::AccountValues => "stream account-values",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap_or_else(|error| panic!("{args:?}: {error}"))
    }

    /// Compare floats within a tolerance (clippy rejects `==` on floats).
    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn assert_opt_close(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("value present");
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    #[test]
    fn command_tree_is_well_formed() {
        // Catches duplicate command names, bad arity and clap misconfiguration.
        Cli::command().debug_assert();
    }

    #[test]
    fn health_uses_default_output() {
        let cli = parse(&["ibkr", "health"]);
        assert!(matches!(cli.command, Command::Health));
        assert_eq!(cli.global.output, Output::Json);
        assert!(cli.global.render_mode().truncate);
    }

    #[test]
    fn global_flags_are_accepted_before_and_after_the_subcommand() {
        let before = parse(&["ibkr", "--url", "https://gw:8080", "health"]);
        assert_eq!(before.global.url.as_deref(), Some("https://gw:8080"));

        let after = parse(&["ibkr", "health", "-o", "table"]);
        assert_eq!(after.global.output, Output::Table);
    }

    #[test]
    fn nested_commands_carry_their_positional_arguments() {
        let cli = parse(&["ibkr", "contracts", "search", "AAPL"]);
        match cli.command {
            Command::Contracts(ContractsCommand::Search { pattern }) => assert_eq!(pattern, "AAPL"),
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = parse(&["ibkr", "orders", "cancel", "42"]);
        match cli.command {
            Command::Orders(OrdersCommand::Cancel { order_id, yes }) => {
                assert_eq!(order_id, 42);
                assert!(!yes);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn market_data_group_is_kebab_cased_on_the_command_line() {
        let cli = parse(&[
            "ibkr",
            "market-data",
            "historical",
            "--symbol",
            "AAPL",
            "--bar-size",
            "1 hour",
            "--duration",
            "30 D",
        ]);
        match cli.command {
            Command::MarketData(MarketDataCommand::Historical {
                contract,
                bar_size,
                duration,
                what_to_show,
                ..
            }) => {
                assert_eq!(contract.symbol, "AAPL");
                assert_eq!(bar_size, "1 hour");
                assert_eq!(duration, "30 D");
                assert!(what_to_show.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn contract_flags_build_the_wire_specification() {
        let cli = parse(&[
            "ibkr",
            "contracts",
            "details",
            "--symbol",
            "HL",
            "--sec-type",
            "OPT",
            "--strike",
            "25",
            "--right",
            "c",
            "--expiry",
            "20260821",
            "--multiplier",
            "100",
            "--include-expired",
        ]);
        match cli.command {
            Command::Contracts(ContractsCommand::Details {
                contract,
                include_expired,
            }) => {
                assert!(include_expired);
                let spec = contract.to_spec();
                assert_eq!(spec.symbol, "HL");
                assert_eq!(spec.sec_type, "OPT");
                assert_eq!(spec.exchange, "SMART");
                assert_eq!(spec.currency, "USD");
                assert_eq!(spec.contract_id, 0);
                assert_opt_close(spec.strike, 25.0);
                assert_eq!(spec.right, Some(OptionRight::Call));
                assert_eq!(
                    spec.last_trade_date_or_contract_month.as_deref(),
                    Some("20260821")
                );
                assert_eq!(spec.multiplier.as_deref(), Some("100"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn order_flags_map_onto_wire_values() {
        let cli = parse(&[
            "ibkr",
            "orders",
            "place",
            "--symbol",
            "AAPL",
            "--side",
            "buy",
            "--quantity",
            "100",
            "--type",
            "stop_limit",
            "--limit-price",
            "210",
            "--stop-price",
            "205",
            "--tif",
            "gtc",
            "--outside-rth",
        ]);
        match cli.command {
            Command::Orders(OrdersCommand::Place {
                contract,
                side,
                quantity,
                order_type,
                limit_price,
                stop_price,
                tif,
                outside_rth,
                hidden,
                yes,
                ..
            }) => {
                assert_eq!(contract.to_spec().symbol, "AAPL");
                assert_eq!(OrderSide::from(side), OrderSide::Buy);
                assert_close(quantity, 100.0);
                assert_eq!(OrderType::from(order_type), OrderType::StopLimit);
                assert_opt_close(limit_price, 210.0);
                assert_opt_close(stop_price, 205.0);
                assert_eq!(TimeInForce::from(tif), TimeInForce::Gtc);
                assert!(outside_rth);
                assert!(!hidden);
                assert!(!yes, "confirmation is on unless --yes is passed");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn only_order_commands_are_mutations() {
        for args in [
            vec![
                "ibkr",
                "orders",
                "place",
                "--symbol",
                "AAPL",
                "--side",
                "BUY",
                "--quantity",
                "1",
                "--type",
                "MARKET",
            ],
            vec![
                "ibkr",
                "orders",
                "bracket",
                "--symbol",
                "AAPL",
                "--side",
                "BUY",
                "--quantity",
                "1",
                "--take-profit",
                "2",
                "--stop-loss",
                "1",
            ],
            vec!["ibkr", "orders", "cancel", "1"],
        ] {
            let cli = parse(&args);
            match cli.command {
                Command::Orders(command) => assert!(command.is_mutation(), "{args:?}"),
                other => panic!("unexpected command: {other:?}"),
            }
        }

        for args in [
            vec!["ibkr", "orders", "list"],
            vec!["ibkr", "accounts", "positions"],
            vec!["ibkr", "flex", "config"],
        ] {
            let cli = parse(&args);
            let mutating = match cli.command {
                Command::Orders(command) => command.is_mutation(),
                _ => false,
            };
            assert!(!mutating, "{args:?}");
        }
    }

    #[test]
    fn flex_token_positional_does_not_shadow_the_global_token() {
        // Regression: a subcommand field named `token` shares clap's arg id
        // with the global --token, so `ibkr flex set-token X` sent
        // `Authorization: Bearer X` and never set the Flex token.
        let cli = parse(&[
            "ibkr",
            "--token",
            "bearer",
            "flex",
            "set-token",
            "flex-secret",
        ]);
        assert_eq!(
            cli.global.token.as_deref(),
            Some("bearer"),
            "the global bearer token must stay what --token said"
        );
        match cli.command {
            Command::Flex(FlexCommand::SetToken { flex_token }) => {
                assert_eq!(flex_token, "flex-secret");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn subcommand_positionals_never_leak_into_the_global_token() {
        let cli = parse(&["ibkr", "flex", "set-token", "flex-secret"]);
        assert!(
            cli.global.token.is_none(),
            "a positional must not become the bearer token"
        );
    }

    #[test]
    fn store_commands_parse_with_repeated_reports() {
        let cli = parse(&[
            "ibkr",
            "store",
            "sync",
            "--report",
            "transactions_30d",
            "-r",
            "last_365_days",
        ]);
        match cli.command {
            Command::Store(StoreCommand::Sync { reports, refresh }) => {
                assert_eq!(reports, vec!["transactions_30d", "last_365_days"]);
                assert!(!refresh);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = parse(&[
            "ibkr",
            "--db",
            "data/trades.db",
            "store",
            "query",
            "SELECT 1",
        ]);
        match cli.command {
            Command::Store(StoreCommand::Query { sql }) => assert_eq!(sql, "SELECT 1"),
            other => panic!("unexpected command: {other:?}"),
        }
        assert_eq!(
            cli.global.overrides().db.as_deref(),
            Some(std::path::Path::new("data/trades.db"))
        );

        let cli = parse(&["ibkr", "store", "schema"]);
        assert!(matches!(cli.command, Command::Store(StoreCommand::Schema)));

        // `--report` is what selects a lot; an empty sync is a mistake, not a
        // no-op that silently reports success.
        assert!(Cli::try_parse_from(["ibkr", "store", "sync"]).is_err());
    }

    #[test]
    fn overrides_only_carry_explicitly_supplied_flags() {
        let cli = parse(&["ibkr", "health"]);
        let overrides = cli.global.overrides();
        assert!(overrides.base_url.is_none());
        assert!(overrides.token.is_none());
        assert!(overrides.timeout.is_none());
        assert!(overrides.max_retries.is_none());
        assert!(overrides.tls_skip_verify.is_none());

        let cli = parse(&[
            "ibkr",
            "--url",
            "https://gw:8080",
            "--timeout",
            "15",
            "--max-retries",
            "0",
            "--tls-skip-verify",
            "health",
        ]);
        let overrides = cli.global.overrides();
        assert_eq!(overrides.base_url.as_deref(), Some("https://gw:8080"));
        assert_eq!(overrides.timeout, Some(Duration::from_secs(15)));
        assert_eq!(overrides.max_retries, Some(0));
        assert_eq!(overrides.tls_skip_verify, Some(true));
    }

    #[test]
    fn top_level_flag_without_a_subcommand_is_an_error() {
        // Guard against a bare `ibkr` silently succeeding.
        assert!(Cli::try_parse_from(["ibkr"]).is_err());
    }

    #[test]
    fn unknown_commands_are_rejected() {
        assert!(Cli::try_parse_from(["ibkr", "nonsense"]).is_err());
        assert!(Cli::try_parse_from(["ibkr", "orders", "nonsense"]).is_err());
    }

    #[test]
    fn labels_identify_each_leaf() {
        let cli = parse(&[
            "ibkr",
            "orders",
            "place",
            "--symbol",
            "AAPL",
            "--side",
            "BUY",
            "--quantity",
            "1",
            "--type",
            "MARKET",
        ]);
        assert_eq!(cli.command.label(), "orders place");

        let cli = parse(&["ibkr", "stream", "account-values"]);
        assert_eq!(cli.command.label(), "stream account-values");

        let cli = parse(&["ibkr", "flex", "report", "transactions_30d"]);
        assert_eq!(cli.command.label(), "flex report");
    }

    #[test]
    fn instrument_descriptions_name_the_relevant_identity() {
        let stock = parse(&["ibkr", "contracts", "details", "--symbol", "AAPL"]);
        match stock.command {
            Command::Contracts(ContractsCommand::Details { contract, .. }) => {
                assert_eq!(contract.describe(), "AAPL (STK)");
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let option = parse(&[
            "ibkr",
            "contracts",
            "details",
            "--symbol",
            "HL",
            "--sec-type",
            "OPT",
            "--strike",
            "25",
            "--right",
            "C",
            "--expiry",
            "20260821",
        ]);
        match option.command {
            Command::Contracts(ContractsCommand::Details { contract, .. }) => {
                assert_eq!(
                    contract.describe(),
                    "HL (OPT) on SMART C strike 25 expiry 20260821"
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
