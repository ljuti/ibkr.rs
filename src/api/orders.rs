//! Order placement, listing and cancellation.

use crate::client::Client;
use crate::error::Result;
use crate::types::{
    BracketOrderIdsResponse, BracketOrderRequest, CancelResponse, ExecutionFilter,
    ExecutionResponse, OrderIdResponse, OrderRequest, OrderResponse, SnapshotEnvelope,
};

impl Client {
    /// Place a single order (`POST /api/v1/orders`).
    ///
    /// `idempotency_key` is required by the gateway: repeating a key with the
    /// same body replays the stored result instead of placing a second order.
    /// The request is sent exactly once — see
    /// [`OrderRequest::validate`] for the checks worth running first.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; a key reused with a different
    /// body is an `idempotency_conflict`.
    pub async fn place_order(
        &self,
        request: &OrderRequest,
        idempotency_key: &str,
    ) -> Result<OrderIdResponse> {
        self.post_idempotent("/api/v1/orders", request, idempotency_key)
            .await
    }

    /// Place a bracket order — entry plus take-profit and stop-loss children
    /// (`POST /api/v1/orders/bracket`).
    ///
    /// Sent exactly once, with the same idempotency semantics as
    /// [`Client::place_order`].
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn place_bracket(
        &self,
        request: &BracketOrderRequest,
        idempotency_key: &str,
    ) -> Result<BracketOrderIdsResponse> {
        self.post_idempotent("/api/v1/orders/bracket", request, idempotency_key)
            .await
    }

    /// Currently open orders (`GET /api/v1/orders`); the gateway caps the
    /// snapshot at 100 items.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn open_orders(&self) -> Result<SnapshotEnvelope<OrderResponse>> {
        self.get("/api/v1/orders").await
    }

    /// Orders completed today (`GET /api/v1/orders/completed`).
    ///
    /// `api_only` restricts the result to orders placed through the API; the
    /// gateway also accepts `false` to include orders placed manually in TWS.
    /// Scope is today only — this is not historical backfill.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn completed_orders(
        &self,
        api_only: bool,
    ) -> Result<SnapshotEnvelope<OrderResponse>> {
        self.get_with(
            "/api/v1/orders/completed",
            &[("apiOnly", api_only.to_string())],
        )
        .await
    }

    /// Executions joined with their commission reports
    /// (`GET /api/v1/orders/executions`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; an unknown `side` is a gateway
    /// `invalid_input`.
    pub async fn executions(
        &self,
        filter: &ExecutionFilter,
    ) -> Result<SnapshotEnvelope<ExecutionResponse>> {
        let mut query = Vec::new();
        if let Some(account) = &filter.account {
            query.push(("account", account.clone()));
        }
        if let Some(symbol) = &filter.symbol {
            query.push(("symbol", symbol.clone()));
        }
        if let Some(side) = filter.side {
            query.push(("side", side.as_wire().to_owned()));
        }
        if let Some(days) = filter.last_n_days {
            query.push(("lastNDays", days.to_string()));
        }
        if let Some(limit) = filter.limit {
            query.push(("limit", limit.to_string()));
        }
        self.get_with("/api/v1/orders/executions", &query).await
    }

    /// Cancel an order by client order id (`DELETE /api/v1/orders/{orderId}`).
    ///
    /// Sent exactly once. The returned status is whatever the gateway observed
    /// while draining the broker's acknowledgement, so it may be `None`.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; a read-only gateway answers
    /// `forbidden`.
    pub async fn cancel_order(&self, order_id: i32) -> Result<CancelResponse> {
        self.delete(&format!("/api/v1/orders/{order_id}")).await
    }
}
