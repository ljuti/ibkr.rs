//! Command dispatch: maps the parsed CLI onto client calls.

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use uuid::Uuid;

use crate::cli::{
    AccountsCommand, Cli, Command, ContractArgs, ContractsCommand, FlexCommand, MarketDataCommand,
    OrdersCommand, RenderMode,
};
use crate::client::{Client, RetryAttempt, RetryObserver};
use crate::config::Config;
use crate::error::{Error, Result};
use crate::output;
use crate::types::{
    BracketOrderRequest, ContractDetailsRequest, ExecutionFilter, HistoricalDataRequest,
    OrderRequest, TimeInForce,
};

/// Execute the parsed command.
///
/// # Errors
///
/// Returns a configuration, transport, or gateway error; order commands also
/// return [`Error::Aborted`] when the user declines the confirmation prompt.
pub async fn execute(cli: Cli) -> Result<()> {
    let config = Config::resolve(cli.global.overrides())?;
    let client = Client::new(&config)?.with_retry_observer(retry_reporter());
    let mode = cli.global.render_mode();

    match cli.command {
        Command::Health => output::health(&client.health().await?, mode),
        Command::Contracts(command) => contracts(&client, command, mode).await,
        Command::MarketData(command) => market_data(&client, command, mode).await,
        Command::Accounts(command) => accounts(&client, command, mode).await,
        Command::Orders(command) => orders(&client, command, mode).await,
        Command::Flex(command) => flex(&client, command, mode).await,
        Command::Stream(command) => Err(Error::Unimplemented {
            command: command.label(),
        }),
    }
}

/// Report retries on stderr, keeping stdout parseable.
fn retry_reporter() -> RetryObserver {
    Arc::new(|retry: RetryAttempt| {
        output::note(&format!(
            "rate limited (HTTP {}); retrying in {:.1}s (retry {}/{})",
            retry.status,
            retry.delay.as_secs_f64(),
            retry.attempt,
            retry.max_retries
        ));
    })
}

async fn contracts(client: &Client, command: ContractsCommand, mode: RenderMode) -> Result<()> {
    match command {
        ContractsCommand::Details {
            contract,
            include_expired,
        } => {
            let request = ContractDetailsRequest {
                contract: contract.to_spec(),
                include_expired,
            };
            output::contract_details(&client.contract_details(&request).await?, mode)
        }
        ContractsCommand::Search { pattern } => {
            output::symbol_search(&client.symbol_search(&pattern).await?, mode)
        }
    }
}

async fn market_data(client: &Client, command: MarketDataCommand, mode: RenderMode) -> Result<()> {
    match command {
        MarketDataCommand::Historical {
            contract,
            bar_size,
            duration,
            ending,
            what_to_show,
            trading_hours,
        } => {
            let request = HistoricalDataRequest {
                contract: contract.to_spec(),
                bar_size,
                duration,
                ending,
                what_to_show: what_to_show.map(|what| what.as_wire().to_owned()),
                trading_hours: trading_hours.map(|hours| hours.as_wire().to_owned()),
            };
            output::historical(&client.historical(&request).await?, mode)
        }
    }
}

async fn accounts(client: &Client, command: AccountsCommand, mode: RenderMode) -> Result<()> {
    match command {
        AccountsCommand::Positions { limit } => {
            output::positions(&client.positions(limit).await?, mode)
        }
        AccountsCommand::Summary { tags } => {
            output::account_summary(&client.account_summary(tags.as_deref()).await?, mode)
        }
        AccountsCommand::Pnl { account } => output::pnl(&client.pnl(&account).await?, mode),
    }
}

async fn orders(client: &Client, command: OrdersCommand, mode: RenderMode) -> Result<()> {
    match command {
        OrdersCommand::Place {
            contract,
            side,
            quantity,
            order_type,
            limit_price,
            stop_price,
            tif,
            outside_rth,
            hidden,
            idempotency_key,
            yes,
        } => {
            let request = OrderRequest {
                contract: contract.to_spec(),
                side: side.into(),
                quantity,
                order_type: order_type.into(),
                limit_price,
                stop_price,
                tif: tif.into(),
                outside_rth,
                hidden,
            };
            request.validate()?;

            let key = idempotency_key.unwrap_or_else(new_idempotency_key);
            confirm(
                "place order",
                &order_summary(&contract, &request, &key),
                yes,
            )?;

            output::order_placed(&client.place_order(&request, &key).await?, mode)
        }

        OrdersCommand::Bracket {
            contract,
            side,
            quantity,
            take_profit,
            stop_loss,
            entry_limit,
            idempotency_key,
            yes,
        } => {
            let request = BracketOrderRequest {
                contract: contract.to_spec(),
                side: side.into(),
                quantity,
                entry_limit,
                take_profit,
                stop_loss,
            };
            request.validate()?;

            let key = idempotency_key.unwrap_or_else(new_idempotency_key);
            confirm(
                "place bracket order",
                &bracket_summary(&contract, &request, &key),
                yes,
            )?;

            output::bracket_placed(&client.place_bracket(&request, &key).await?, mode)
        }

        OrdersCommand::List => output::orders(&client.open_orders().await?, "open orders", mode),

        OrdersCommand::Completed { include_manual } => output::orders(
            &client.completed_orders(!include_manual).await?,
            "completed orders",
            mode,
        ),

        OrdersCommand::Executions {
            account,
            symbol,
            side,
            last_n_days,
            limit,
        } => {
            let filter = ExecutionFilter {
                account,
                symbol,
                side: side.map(Into::into),
                last_n_days,
                limit,
            };
            output::executions(&client.executions(&filter).await?, mode)
        }

        OrdersCommand::Cancel { order_id, yes } => {
            let summary = output::kv_table(&[("orderId", order_id.to_string())]);
            confirm("cancel order", &summary.to_string(), yes)?;
            output::cancelled(&client.cancel_order(order_id).await?, mode)
        }
    }
}

async fn flex(client: &Client, command: FlexCommand, mode: RenderMode) -> Result<()> {
    match command {
        FlexCommand::Config => output::flex_config(&client.flex_config().await?, mode),

        FlexCommand::SetQuery {
            report_name,
            query_id,
        } => output::flex_ack(&client.set_flex_query(&report_name, &query_id).await?, mode),

        FlexCommand::SetToken { flex_token } => {
            output::flex_ack(&client.set_flex_token(&flex_token).await?, mode)
        }

        FlexCommand::Report {
            report_name,
            refresh,
            etag,
        } => output::flex_report(
            &client
                .flex_report(&report_name, refresh, etag.as_deref())
                .await?,
            mode,
        ),
    }
}

/// Ask before changing broker state.
///
/// The summary goes to stderr, so stdout stays a clean payload stream. Without
/// `--yes` a non-interactive stdin is refused rather than assumed to consent:
/// an unattended `orders place` should never be a surprise.
fn confirm(action: &str, summary: &str, assume_yes: bool) -> Result<()> {
    output::note(&format!("about to {action}:"));
    eprint!("{summary}");

    if assume_yes {
        output::note("--yes given: sending without confirmation");
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(Error::Invalid(
            "refusing to change broker state without confirmation: stdin is not a terminal \
             (pass --yes to confirm non-interactively)"
                .to_owned(),
        ));
    }

    eprint!("confirm? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(Error::Aborted)
    }
}

/// Generate an idempotency key, telling the user what it was.
///
/// Reusing the printed key with `--idempotency-key` replays the original result
/// instead of placing another order, which is what makes a retry after a
/// timeout safe.
fn new_idempotency_key() -> String {
    let key = Uuid::new_v4().to_string();
    output::note(&format!(
        "idempotency key {key} (pass --idempotency-key {key} to replay instead of re-placing)"
    ));
    key
}

/// Human-readable summary of a single order, for the confirmation prompt.
fn order_summary(contract: &ContractArgs, request: &OrderRequest, key: &str) -> String {
    let mut rows = vec![
        ("instrument", contract.describe()),
        ("side", request.side.as_wire().to_owned()),
        ("quantity", output::number(request.quantity)),
        ("type", request.order_type.to_string()),
    ];
    if let Some(limit) = request.limit_price {
        rows.push(("limit price", output::number(limit)));
    }
    if let Some(stop) = request.stop_price {
        rows.push(("stop price", output::number(stop)));
    }
    rows.push(("tif", tif_wire(request.tif).to_owned()));
    rows.push(("outside rth", yes_no(request.outside_rth)));
    rows.push(("hidden", yes_no(request.hidden)));
    rows.push(("idempotency", key.to_owned()));
    output::kv_table(&rows).to_string()
}

/// Human-readable summary of a bracket order, for the confirmation prompt.
fn bracket_summary(contract: &ContractArgs, request: &BracketOrderRequest, key: &str) -> String {
    let entry = request.entry_limit.map_or_else(
        || "MARKET".to_owned(),
        |limit| format!("LIMIT {}", output::number(limit)),
    );
    let rows = [
        ("instrument", contract.describe()),
        ("side", request.side.as_wire().to_owned()),
        ("quantity", output::number(request.quantity)),
        ("entry", entry),
        ("take profit", output::number(request.take_profit)),
        ("stop loss", output::number(request.stop_loss)),
        ("idempotency", key.to_owned()),
    ];
    output::kv_table(&rows).to_string()
}

/// Wire spelling of a time-in-force value.
fn tif_wire(tif: TimeInForce) -> &'static str {
    match tif {
        TimeInForce::Day => "DAY",
        TimeInForce::Gtc => "GTC",
        TimeInForce::Ioc => "IOC",
    }
}

fn yes_no(value: bool) -> String {
    if value {
        "yes".to_owned()
    } else {
        "no".to_owned()
    }
}
