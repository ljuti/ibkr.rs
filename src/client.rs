//! HTTP transport for the gateway.
//!
//! This module owns credentials, retry policy, and error decoding. The
//! endpoint methods themselves live in [`crate::api`] as further `impl Client`
//! blocks, mirroring the gateway's route layout.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{IF_NONE_MATCH, RETRY_AFTER};
use reqwest::{Certificate, Client as HttpClient, Identity, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::Config;
use crate::error::{ApiError, Error, ErrorKind, Result};
use crate::types::{ApiErrorBody, Health};

/// Longest `Retry-After` the client is willing to wait out.
///
/// Pacing windows can be minutes long; sleeping that out inside a request would
/// look like a hang, so a longer hint is surfaced to the caller instead.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// A rate-limited read that is about to be retried.
#[derive(Debug, Clone, Copy)]
pub struct RetryAttempt {
    /// Which attempt just failed (1-based).
    pub attempt: u32,
    /// Retries configured for this client.
    pub max_retries: u32,
    /// How long the client will wait before re-sending.
    pub delay: Duration,
    /// HTTP status that triggered the retry.
    pub status: u16,
}

/// Observer notified before each retry.
///
/// The library writes no output itself, so callers decide how to surface a
/// retry (the CLI prints to stderr, keeping stdout parseable).
pub type RetryObserver = Arc<dyn Fn(RetryAttempt) + Send + Sync>;

/// Outcome of a conditional read.
#[derive(Debug, Clone)]
pub enum Conditional<T> {
    /// The gateway returned the resource.
    Fresh {
        /// Decoded payload.
        value: T,
        /// Entity tag to send next time as `If-None-Match`.
        etag: Option<String>,
    },
    /// The caller's cached copy is still current (HTTP 304).
    NotModified {
        /// Entity tag, repeated for convenience.
        etag: Option<String>,
    },
}

/// Authenticated client for the gateway REST API.
///
/// The client presents the configured client certificate (mutual TLS) and
/// attaches the bearer token to every request. `/health` is the one endpoint
/// the gateway serves without a token, so it is also the connectivity probe.
///
/// Reads (`GET`) are retried when the gateway answers `429 Too Many Requests`,
/// waiting out its `Retry-After` hint. Mutations (`POST`, `PUT`, `DELETE`) are
/// sent exactly once: a retry could duplicate an order.
pub struct Client {
    http: HttpClient,
    base_url: String,
    token: Option<String>,
    max_retries: u32,
    on_retry: Option<RetryObserver>,
}

impl Client {
    /// Build a client from resolved [`Config`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when a configured certificate cannot be read
    /// or parsed, and [`Error::Transport`] when the HTTP client cannot be
    /// constructed.
    pub fn new(config: &Config) -> Result<Self> {
        let http = build_http_client(config)?;
        Ok(Self {
            http,
            base_url: config.base_url.clone(),
            token: config.token.clone(),
            max_retries: config.max_retries,
            on_retry: None,
        })
    }

    /// Report retries (for example to a log) as they happen.
    #[must_use]
    pub fn with_retry_observer(mut self, observer: RetryObserver) -> Self {
        self.on_retry = Some(observer);
        self
    }

    /// Gateway base URL this client talks to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Whether a bearer token is configured.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Retries configured for rate-limited reads.
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Gateway and broker connectivity (`GET /health`).
    ///
    /// # Errors
    ///
    /// Returns a transport or decode error. `/health` needs mTLS but no token,
    /// so it never reports `unauthorized`.
    pub async fn health(&self) -> Result<Health> {
        self.get("/health").await
    }

    /// Perform a `GET` and decode the JSON response.
    ///
    /// `path` is relative to the base URL and must start with `/`. Retried on
    /// `429` up to the configured retry budget.
    ///
    /// # Errors
    ///
    /// Returns a transport, gateway, or decode error.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send_read(self.http.get(self.url(path))).await
    }

    /// Perform a `GET` with query parameters and decode the JSON response.
    ///
    /// # Errors
    ///
    /// Same as [`Client::get`].
    pub async fn get_with<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        self.send_read(self.http.get(self.url(path)).query(query))
            .await
    }

    /// Perform a `GET` carrying `If-None-Match`.
    ///
    /// # Errors
    ///
    /// Same as [`Client::get`], except a `304` is reported as
    /// [`Conditional::NotModified`] rather than an error.
    pub async fn get_conditional<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        etag: Option<&str>,
    ) -> Result<Conditional<T>> {
        let mut request = self.http.get(self.url(path)).query(query);
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }

        let response = self.read_response(request).await?;
        let status = response.status();
        let response_etag = header(&response, "etag");

        if status == StatusCode::NOT_MODIFIED {
            return Ok(Conditional::NotModified {
                etag: response_etag,
            });
        }
        if status.is_success() {
            return Ok(Conditional::Fresh {
                value: response.json::<T>().await?,
                etag: response_etag,
            });
        }
        Err(Error::Api(decode_api_error(response, status).await))
    }

    /// Perform a `POST` with a JSON body and decode the JSON response.
    ///
    /// Sent exactly once — never retried.
    ///
    /// # Errors
    ///
    /// Returns a transport, gateway, or decode error.
    pub async fn post<B, T>(&self, path: &str, body: &B) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        self.send_write(self.http.post(self.url(path)).json(body))
            .await
    }

    /// Perform a `POST` carrying an `Idempotency-Key`.
    ///
    /// The gateway requires the header on order placement: replaying the same
    /// key with the same body returns the original result instead of placing a
    /// second order. Sent exactly once — never retried.
    ///
    /// # Errors
    ///
    /// Returns a transport, gateway, or decode error.
    pub async fn post_idempotent<B, T>(&self, path: &str, body: &B, key: &str) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        self.send_write(
            self.http
                .post(self.url(path))
                .header("Idempotency-Key", key)
                .json(body),
        )
        .await
    }

    /// Perform a `PUT` with a JSON body and decode the JSON response.
    ///
    /// Sent exactly once — never retried.
    ///
    /// # Errors
    ///
    /// Returns a transport, gateway, or decode error.
    pub async fn put<B, T>(&self, path: &str, body: &B) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        self.send_write(self.http.put(self.url(path)).json(body))
            .await
    }

    /// Perform a `DELETE` and decode the JSON response.
    ///
    /// Sent exactly once — never retried.
    ///
    /// # Errors
    ///
    /// Returns a transport, gateway, or decode error.
    pub async fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.send_write(self.http.delete(self.url(path))).await
    }

    /// Absolute URL for a path relative to the base URL.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Attach credentials and send, returning the response whatever its status.
    async fn execute(&self, request: RequestBuilder) -> Result<Response> {
        let request = match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        };
        Ok(request.send().await?)
    }

    /// Send a read request, retrying pacing rejections.
    ///
    /// Returns the response for any non-`429` status, including `304`, so
    /// conditional callers can inspect it.
    async fn read_response(&self, request: RequestBuilder) -> Result<Response> {
        let mut request = request;
        let mut retries_left = self.max_retries;

        loop {
            // Keep a copy in reserve: the retry must carry the same method,
            // URL, query, headers and body as the attempt that was rejected.
            let retry_copy = if retries_left > 0 {
                request.try_clone()
            } else {
                None
            };

            let response = self.execute(request).await?;
            let status = response.status();
            if status != StatusCode::TOO_MANY_REQUESTS {
                return Ok(response);
            }

            let error = decode_api_error(response, status).await;
            let (Some(delay), Some(next)) = (retry_delay(&error), retry_copy) else {
                return Err(Error::Api(error));
            };

            if let Some(observer) = &self.on_retry {
                observer(RetryAttempt {
                    attempt: self.max_retries - retries_left + 1,
                    max_retries: self.max_retries,
                    delay,
                    status: error.status,
                });
            }

            tokio::time::sleep(delay).await;
            retries_left -= 1;
            request = next;
        }
    }

    /// Send a read request and decode it.
    async fn send_read<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
        let response = self.read_response(request).await?;
        let status = response.status();
        if status.is_success() {
            Ok(response.json::<T>().await?)
        } else {
            Err(Error::Api(decode_api_error(response, status).await))
        }
    }

    /// Send a mutation exactly once and decode it.
    async fn send_write<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
        let response = self.execute(request).await?;
        let status = response.status();
        if status.is_success() {
            Ok(response.json::<T>().await?)
        } else {
            Err(Error::Api(decode_api_error(response, status).await))
        }
    }
}

/// Build the underlying HTTP client: timeout, user agent, and mutual TLS.
fn build_http_client(config: &Config) -> Result<HttpClient> {
    let mut builder = HttpClient::builder()
        .timeout(config.timeout)
        .user_agent(concat!("ibkr/", env!("CARGO_PKG_VERSION")));

    if config.is_https() {
        if let Some(path) = &config.ca_cert {
            let pem = read_pem(path, "CA certificate")?;
            builder =
                builder.add_root_certificate(Certificate::from_pem(&pem).map_err(|error| {
                    Error::Config(format!(
                        "invalid CA certificate {}: {error}",
                        path.display()
                    ))
                })?);
        }

        if let (Some(cert_path), Some(key_path)) = (&config.client_cert, &config.client_key) {
            // `Identity::from_pem` wants certificate and key in one buffer.
            let mut pem = read_pem(cert_path, "client certificate")?;
            pem.push(b'\n');
            pem.extend_from_slice(&read_pem(key_path, "client key")?);
            builder = builder.identity(Identity::from_pem(&pem).map_err(|error| {
                Error::Config(format!(
                    "invalid client identity ({} + {}): {error}",
                    cert_path.display(),
                    key_path.display()
                ))
            })?);
        }

        if config.tls_skip_verify {
            builder = builder
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true);
        }
    }

    builder.build().map_err(Error::Transport)
}

/// Read a PEM file, reporting the failing path.
fn read_pem(path: &Path, what: &str) -> Result<Vec<u8>> {
    fs::read(path)
        .map_err(|error| Error::Config(format!("cannot read {what} {}: {error}", path.display())))
}

/// A response header as an owned string, when present and printable.
fn header(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Delay before re-sending a read, or `None` when the failure is not
/// retryable.
///
/// Only pacing failures are retried, and only when the gateway said how long to
/// wait. A hint longer than [`MAX_RETRY_AFTER`] is not worth sleeping out
/// inside a CLI request, so the error reaches the caller instead.
fn retry_delay(error: &ApiError) -> Option<Duration> {
    if error.kind != ErrorKind::RateLimited {
        return None;
    }
    let delay = error.retry_after?;
    (delay <= MAX_RETRY_AFTER).then_some(delay)
}

/// Turn an error response into an [`ApiError`], preferring the gateway's
/// structured body and falling back to the raw body or the status code.
async fn decode_api_error(response: Response, status: StatusCode) -> ApiError {
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs);

    let body = response.text().await.unwrap_or_default();
    let status_code = status.as_u16();

    if let Ok(parsed) = serde_json::from_str::<ApiErrorBody>(&body) {
        return ApiError {
            status: status_code,
            kind: ErrorKind::parse(&parsed.kind),
            message: parsed.error,
            retry_after,
        };
    }

    // No structured body: keep whatever the gateway sent, else the status.
    let trimmed = body.trim();
    let message = if trimmed.is_empty() {
        format!("HTTP {status}")
    } else {
        trimmed.to_owned()
    };
    ApiError {
        status: status_code,
        kind: ErrorKind::Unknown(String::new()),
        message,
        retry_after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_error(kind: ErrorKind, retry_after: Option<Duration>) -> ApiError {
        ApiError {
            status: 429,
            kind,
            message: "pacing limit exceeded".to_owned(),
            retry_after,
        }
    }

    #[test]
    fn rate_limited_reads_are_retried_after_the_advertised_delay() {
        let delay = retry_delay(&api_error(
            ErrorKind::RateLimited,
            Some(Duration::from_secs(2)),
        ));
        assert_eq!(delay, Some(Duration::from_secs(2)));
    }

    #[test]
    fn a_rate_limit_without_a_hint_is_surfaced() {
        // The gateway always sends Retry-After; without it there is nothing
        // safe to sleep on, so the caller decides.
        let delay = retry_delay(&api_error(ErrorKind::RateLimited, None));
        assert_eq!(delay, None);
    }

    #[test]
    fn an_oversized_retry_hint_is_surfaced_instead_of_slept_out() {
        let capped = retry_delay(&api_error(
            ErrorKind::RateLimited,
            Some(MAX_RETRY_AFTER + Duration::from_secs(1)),
        ));
        assert_eq!(capped, None, "hint beyond the cap must not be waited out");

        let at_cap = retry_delay(&api_error(ErrorKind::RateLimited, Some(MAX_RETRY_AFTER)));
        assert_eq!(at_cap, Some(MAX_RETRY_AFTER));
    }

    #[test]
    fn only_pacing_failures_are_retryable() {
        // Retrying an IB-side failure or a gateway timeout would just repeat
        // the same upstream fault.
        for wire in ["upstream", "timeout", "invalid_input", "internal"] {
            let delay = retry_delay(&api_error(
                ErrorKind::parse(wire),
                Some(Duration::from_secs(1)),
            ));
            assert_eq!(delay, None, "{wire} must not be retried");
        }
    }
}
