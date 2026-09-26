//! Client for the IBKR gateway.
//!
//! The gateway (a separate service) owns the single connection to TWS / IB
//! Gateway and exposes it as an authenticated JSON API: REST for
//! request/response operations and a WebSocket endpoint for streaming. This
//! crate wraps that API for Rust consumers and backs the `ibkr` binary.
//!
//! Transport is mutual TLS plus a bearer token. [`Config`] resolves the
//! endpoint and credentials from CLI flags and environment variables;
//! [`Client`] performs the HTTP requests.
//!
//! # Status
//!
//! The crate is being built out in stages. Today [`Client::health`] is wired
//! end to end; the remaining endpoints in [`cli::Command`] are declared but
//! not implemented.

pub mod api;
pub mod cli;
pub mod client;
pub mod commands;
pub mod config;
pub mod error;
pub mod output;
pub mod types;

pub use client::Client;
pub use config::{Config, ConfigOverrides};
pub use error::{ApiError, Error, ErrorKind, Result};

use std::process::ExitCode;

use clap::Parser;

/// Parse the command line and run the requested command.
///
/// Returns the process exit code: success, or failure after printing the
/// error to stderr.
pub async fn run() -> ExitCode {
    let cli = cli::Cli::parse();
    match commands::execute(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {}", describe(&error));
            ExitCode::FAILURE
        }
    }
}

/// Render an error together with its cause chain.
///
/// `reqwest` reports every transport failure as "error sending request", with
/// the real reason — a timeout, a certificate rejection — one level down.
fn describe(error: &Error) -> String {
    use std::error::Error as _;
    use std::fmt::Write as _;

    let mut rendered = error.to_string();
    let mut cause = error.source();
    while let Some(cause_error) = cause {
        let _ = write!(rendered, "\n  caused by: {cause_error}");
        cause = cause_error.source();
    }
    rendered
}
