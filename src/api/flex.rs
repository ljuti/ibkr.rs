//! Flex reports and Flex credential configuration.

use crate::client::{Client, Conditional};
use crate::error::Result;
use crate::types::{
    AckResponse, FlexConfigResponse, FlexReportResponse, SetQueryRequest, SetTokenRequest,
};

impl Client {
    /// Registered Flex reports and credential status
    /// (`GET /api/v1/config/flex`). Never includes the token value.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error.
    pub async fn flex_config(&self) -> Result<FlexConfigResponse> {
        self.get("/api/v1/config/flex").await
    }

    /// Register or rotate a report's Flex query id
    /// (`PUT /api/v1/config/flex/queries/{reportName}`), validated live
    /// against the Flex service.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; a rejected query id is a
    /// `validation_failed`.
    pub async fn set_flex_query(&self, report_name: &str, query_id: &str) -> Result<AckResponse> {
        self.put(
            &format!(
                "/api/v1/config/flex/queries/{}",
                encode_segment(report_name)
            ),
            &SetQueryRequest {
                query_id: query_id.to_owned(),
            },
        )
        .await
    }

    /// Register or rotate the Flex web-service token
    /// (`PUT /api/v1/config/flex/token`).
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; a rejected token is a
    /// `validation_failed`.
    pub async fn set_flex_token(&self, token: &str) -> Result<AckResponse> {
        self.put(
            "/api/v1/config/flex/token",
            &SetTokenRequest {
                token: token.to_owned(),
            },
        )
        .await
    }

    /// Fetch a report's trades and cash transactions
    /// (`GET /api/v1/accounts/reports/{reportName}`).
    ///
    /// The statement window is fixed by the query's Period setting; the Flex
    /// service accepts no date-range override. Pass the `etag` from a previous
    /// response to get [`Conditional::NotModified`] instead of a re-fetch, or
    /// `refresh` to bypass the gateway cache.
    ///
    /// # Errors
    ///
    /// Returns a transport or gateway error; an unknown report name is a
    /// gateway `invalid_input`.
    pub async fn flex_report(
        &self,
        report_name: &str,
        refresh: bool,
        etag: Option<&str>,
    ) -> Result<Conditional<FlexReportResponse>> {
        let path = format!("/api/v1/accounts/reports/{}", encode_segment(report_name));
        let query = if refresh {
            vec![("refresh", "true".to_owned())]
        } else {
            Vec::new()
        };
        self.get_conditional(&path, &query, etag).await
    }
}

/// Percent-encode a value for use as a single URL path segment.
///
/// Report names come from configuration, but a name with a slash or space must
/// not be able to reshape the request path.
fn encode_segment(value: &str) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            other => {
                // Writing into a String cannot fail.
                let _ = write!(encoded, "%{other:02X}");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_segments_are_encoded() {
        assert_eq!(encode_segment("transactions_30d"), "transactions_30d");
        assert_eq!(encode_segment("a/b"), "a%2Fb");
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(encode_segment("tax.2026"), "tax.2026");
        assert_eq!(encode_segment("é"), "%C3%A9");
    }
}
