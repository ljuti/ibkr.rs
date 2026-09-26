//! Contract resolution and symbol search.

use crate::client::Client;
use crate::error::Result;
use crate::types::{ContractDetailsRequest, ContractDetailsResponse, SymbolSearchResponse};

impl Client {
    /// Resolve an instrument specification into full contract details
    /// (`POST /api/v1/contracts/details`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; an unusable spec is a gateway
    /// `invalid_input`.
    pub async fn contract_details(
        &self,
        request: &ContractDetailsRequest,
    ) -> Result<Vec<ContractDetailsResponse>> {
        self.post("/api/v1/contracts/details", request).await
    }

    /// Search for instruments matching a symbol pattern
    /// (`GET /api/v1/contracts/search`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn symbol_search(&self, pattern: &str) -> Result<Vec<SymbolSearchResponse>> {
        self.get_with(
            "/api/v1/contracts/search",
            &[("pattern", pattern.to_owned())],
        )
        .await
    }
}
