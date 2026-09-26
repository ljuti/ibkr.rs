//! Error types for gateway interaction.

use std::time::Duration;

/// Result alias for client operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Machine-readable failure category, mirroring the gateway's `kind` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The request was malformed or semantically invalid.
    InvalidInput,
    /// Missing or invalid bearer token.
    Unauthorized,
    /// Authenticated but not permitted (for example, the gateway is read-only).
    Forbidden,
    /// An idempotency key was reused with a different payload.
    IdempotencyConflict,
    /// A configuration value failed external validation.
    ValidationFailed,
    /// Pacing limit exceeded; retry after the `Retry-After` interval.
    RateLimited,
    /// The upstream TWS / IB Gateway request failed.
    Upstream,
    /// The gateway timed out waiting on an IB stream.
    Timeout,
    /// Unexpected gateway-side failure.
    Internal,
    /// A kind this client version does not recognise.
    Unknown(String),
}

impl ErrorKind {
    /// Parse the gateway's `kind` string.
    pub fn parse(kind: &str) -> Self {
        match kind {
            "invalid_input" => Self::InvalidInput,
            "unauthorized" => Self::Unauthorized,
            "forbidden" => Self::Forbidden,
            "idempotency_conflict" => Self::IdempotencyConflict,
            "validation_failed" => Self::ValidationFailed,
            "rate_limited" => Self::RateLimited,
            "upstream" => Self::Upstream,
            "timeout" => Self::Timeout,
            "internal" => Self::Internal,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput => formatter.write_str("invalid_input"),
            Self::Unauthorized => formatter.write_str("unauthorized"),
            Self::Forbidden => formatter.write_str("forbidden"),
            Self::IdempotencyConflict => formatter.write_str("idempotency_conflict"),
            Self::ValidationFailed => formatter.write_str("validation_failed"),
            Self::RateLimited => formatter.write_str("rate_limited"),
            Self::Upstream => formatter.write_str("upstream"),
            Self::Timeout => formatter.write_str("timeout"),
            Self::Internal => formatter.write_str("internal"),
            Self::Unknown(kind) if kind.is_empty() => formatter.write_str("unknown"),
            Self::Unknown(kind) => formatter.write_str(kind),
        }
    }
}

/// An error response returned by the gateway.
#[derive(Debug, Clone, thiserror::Error)]
#[error("gateway error {status} ({kind}): {message}")]
pub struct ApiError {
    /// HTTP status code.
    pub status: u16,
    /// Machine-readable failure category.
    pub kind: ErrorKind,
    /// Human-readable message from the gateway.
    pub message: String,
    /// Suggested retry delay, when the gateway sent `Retry-After`.
    pub retry_after: Option<Duration>,
}

/// Errors produced while talking to the gateway.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The client is misconfigured: bad URL, unreadable or invalid credentials.
    #[error("configuration error: {0}")]
    Config(String),

    /// Transport-level failure (DNS, TCP, TLS, timeout, body read).
    #[error("transport error: {0}")]
    Transport(#[from] reqwest::Error),

    /// The gateway answered with an error response.
    #[error("{0}")]
    Api(#[from] ApiError),

    /// A response body did not match the expected type.
    #[error("failed to decode response: {0}")]
    Decode(#[from] serde_json::Error),

    /// Reading a prompt from stdin, or writing it to stderr, failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// The user declined a confirmation prompt; nothing was sent.
    #[error("aborted: nothing was sent")]
    Aborted,

    /// The command's arguments are unusable, for example a limit order with no
    /// limit price. Caught locally instead of as a gateway `400`.
    #[error("invalid arguments: {0}")]
    Invalid(String),

    /// The command is part of the planned surface but not implemented yet.
    #[error("{command} is not implemented yet")]
    Unimplemented {
        /// Human-readable command name, for example `orders place`.
        command: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_kinds_map_onto_variants() {
        let cases = [
            ("invalid_input", ErrorKind::InvalidInput),
            ("unauthorized", ErrorKind::Unauthorized),
            ("forbidden", ErrorKind::Forbidden),
            ("idempotency_conflict", ErrorKind::IdempotencyConflict),
            ("validation_failed", ErrorKind::ValidationFailed),
            ("rate_limited", ErrorKind::RateLimited),
            ("upstream", ErrorKind::Upstream),
            ("timeout", ErrorKind::Timeout),
            ("internal", ErrorKind::Internal),
        ];
        for (wire, expected) in cases {
            assert_eq!(ErrorKind::parse(wire), expected, "kind {wire}");
        }
    }

    #[test]
    fn unrecognised_kinds_are_preserved() {
        // An older client must not silently drop a kind a newer gateway added.
        let kind = ErrorKind::parse("teapot");
        assert_eq!(kind, ErrorKind::Unknown("teapot".to_owned()));
        assert_eq!(kind.to_string(), "teapot");
        assert_eq!(ErrorKind::Unknown(String::new()).to_string(), "unknown");
    }

    #[test]
    fn api_error_display_carries_status_kind_and_message() {
        let error = ApiError {
            status: 429,
            kind: ErrorKind::RateLimited,
            message: "pacing limit exceeded".to_owned(),
            retry_after: Some(Duration::from_secs(7)),
        };
        assert_eq!(
            error.to_string(),
            "gateway error 429 (rate_limited): pacing limit exceeded"
        );
        // The retry hint survives conversion into the crate error type.
        assert!(matches!(Error::from(error), Error::Api(_)));
    }
}
