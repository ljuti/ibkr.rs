//! Historical market data.

use crate::client::Client;
use crate::error::Result;
use crate::types::{HistoricalDataRequest, HistoricalDataResponse};

impl Client {
    /// Fetch historical OHLCV bars (`POST /api/v1/market-data/historical`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; unparseable `bar_size` /
    /// `duration` / `what_to_show` values are gateway `invalid_input`.
    pub async fn historical(
        &self,
        request: &HistoricalDataRequest,
    ) -> Result<HistoricalDataResponse> {
        self.post("/api/v1/market-data/historical", request).await
    }
}
