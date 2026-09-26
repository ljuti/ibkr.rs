//! Endpoint methods on [`crate::Client`], grouped by the gateway's route areas.
//!
//! Each module adds an `impl Client` block for one area, so a call reads as
//! `client.positions(None).await` rather than a bare path. Paths and query
//! parameter names match the gateway contract exactly.

pub mod accounts;
pub mod contracts;
pub mod flex;
pub mod market_data;
pub mod orders;
