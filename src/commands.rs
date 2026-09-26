//! Command dispatch: maps the parsed CLI onto client calls.

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use uuid::Uuid;

use crate::cli::{
    AccountsCommand, Cli, Command, ContractArgs, ContractsCommand, FlexCommand, MarketDataCommand,
    OrdersCommand, RenderMode, StoreCommand,
};
use crate::client::{Client, Conditional, RetryAttempt, RetryObserver};
use crate::config::Config;
use crate::configure;
use crate::error::{Error, Result};
use crate::output;
use crate::store::{Store, SyncOutcome};
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
        Command::Configure { show, verify } => {
            configure::run(cli.global.overrides(), show, verify, mode).await
        }
        Command::Contracts(command) => contracts(&client, command, mode).await,
        Command::MarketData(command) => market_data(&client, command, mode).await,
        Command::Accounts(command) => accounts(&client, command, mode).await,
        Command::Orders(command) => orders(&client, &config, command, mode).await,
        Command::Flex(command) => flex(&client, command, mode).await,
        Command::Store(command) => store(&client, &config, command, mode).await,
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

async fn orders(
    client: &Client,
    config: &Config,
    command: OrdersCommand,
    mode: RenderMode,
) -> Result<()> {
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
            // Read-only first: nothing below this line runs, so no validation
            // prompt and no request can happen.
            config.ensure_writable("orders place")?;
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
            config.ensure_writable("orders bracket")?;
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
            config.ensure_writable("orders cancel")?;
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

/// Sync Flex reports into the local store, or query what is already there.
async fn store(
    client: &Client,
    config: &Config,
    command: StoreCommand,
    mode: RenderMode,
) -> Result<()> {
    match command {
        StoreCommand::Sync { reports, refresh } => {
            let mut store = Store::open(&config.db)?;
            let mut outcomes = Vec::with_capacity(reports.len());
            let mut failures: Vec<Error> = Vec::new();
            for report in &reports {
                output::note(&format!("syncing {report}"));
                // One report failing must not cost the others their sync: a
                // rate-limited 30-day window should not stop the yearly one.
                match sync_one(&mut store, client, report, refresh).await {
                    Ok(outcome) => {
                        if let SyncOutcome::Synced(stats) = &outcome {
                            let (_, hint) = crate::store::SyncCapabilities::of(stats);
                            if let Some(hint) = hint {
                                output::note(&format!("{report}: {hint}"));
                            }
                        }
                        outcomes.push(outcome);
                    }
                    Err(error) => {
                        outcomes.push(SyncOutcome::Failed {
                            report: report.clone(),
                            message: error.to_string(),
                        });
                        failures.push(error);
                    }
                }
            }
            output::store_sync(&outcomes, mode)?;
            match failures.len() {
                0 => Ok(()),
                // A single report keeps its own error, retry hints and all.
                1 if reports.len() == 1 => Err(failures.pop().expect("one failure")),
                failed => Err(Error::Partial {
                    failed,
                    total: reports.len(),
                    detail: failures
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; "),
                }),
            }
        }

        StoreCommand::Query { sql } => {
            let store = Store::open_read_only(&config.db)?;
            output::store_query(&store.query(&sql)?, mode)
        }

        StoreCommand::Schema => {
            let store = Store::open_read_only(&config.db)?;
            output::store_schema(&store.schema()?, mode)
        }

        StoreCommand::Status => {
            let store = Store::open_read_only(&config.db)?;
            output::store_status(&store.status()?, mode)
        }
    }
}

/// Fetch one report and store it, or report why it was not stored.
async fn sync_one(
    store: &mut Store,
    client: &Client,
    report: &str,
    refresh: bool,
) -> Result<SyncOutcome> {
    // Statements are immutable per (query, window), so a stored ETag turns a
    // re-sync into a 304 and skips the ingest.
    let etag = if refresh {
        None
    } else {
        store.last_etag(report)?
    };
    Ok(
        match client.flex_report(report, refresh, etag.as_deref()).await? {
            Conditional::NotModified { etag } => SyncOutcome::NotModified {
                report: report.to_owned(),
                etag,
            },
            Conditional::Fresh { value, etag } => {
                SyncOutcome::Synced(store.upsert_report(report, etag.as_deref(), &value)?)
            }
        },
    )
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
