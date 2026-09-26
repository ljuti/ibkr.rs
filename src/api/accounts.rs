//! Positions, account summary and profit-and-loss.

use crate::client::Client;
use crate::error::Result;
use crate::types::{AccountSummaryValue, PnLResponse, PositionResponse, SnapshotEnvelope};

impl Client {
    /// Bounded snapshot of open positions (`GET /api/v1/accounts/positions`).
    ///
    /// `limit` is clamped by the gateway (default 500, maximum 1000).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; a stalled drain is a gateway
    /// `timeout`.
    pub async fn positions(
        &self,
        limit: Option<usize>,
    ) -> Result<SnapshotEnvelope<PositionResponse>> {
        let query = limit.map_or_else(Vec::new, |limit| vec![("limit", limit.to_string())]);
        self.get_with("/api/v1/accounts/positions", &query).await
    }

    /// Account summary tag values (`GET /api/v1/accounts/summary`).
    ///
    /// `tags` is a comma-separated list; `None` requests the gateway's default
    /// set (all supported tags).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn account_summary(&self, tags: Option<&str>) -> Result<Vec<AccountSummaryValue>> {
        let query = tags.map_or_else(Vec::new, |tags| vec![("tags", tags.to_owned())]);
        self.get_with("/api/v1/accounts/summary", &query).await
    }

    /// Latest profit-and-loss snapshot (`GET /api/v1/accounts/pnl`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; an empty account id is a gateway
    /// `invalid_input`.
    pub async fn pnl(&self, account: &str) -> Result<PnLResponse> {
        self.get_with("/api/v1/accounts/pnl", &[("account", account.to_owned())])
            .await
    }
}
