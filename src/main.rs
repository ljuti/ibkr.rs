//! Binary entry point for the `ibkr` CLI.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    ibkr::run().await
}
