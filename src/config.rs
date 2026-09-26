//! Client configuration, resolved from CLI flags, the environment and defaults.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};

/// Environment variables understood by the client.
pub mod var {
    /// Gateway base URL.
    pub const URL: &str = "IBKR_GATEWAY_URL";
    /// Bearer token presented on `/api/v1/*` requests.
    pub const TOKEN: &str = "IBKR_GATEWAY_TOKEN";
    /// PEM file with the CA that signed the gateway's server certificate.
    pub const CA_CERT: &str = "IBKR_GATEWAY_CA_CERT";
    /// PEM client certificate for mutual TLS.
    pub const CLIENT_CERT: &str = "IBKR_GATEWAY_CLIENT_CERT";
    /// PEM private key matching [`CLIENT_CERT`].
    pub const CLIENT_KEY: &str = "IBKR_GATEWAY_CLIENT_KEY";
    /// Request timeout in seconds.
    pub const TIMEOUT_SEC: &str = "IBKR_GATEWAY_TIMEOUT_SEC";
    /// Retries for rate-limited reads.
    pub const MAX_RETRIES: &str = "IBKR_GATEWAY_MAX_RETRIES";
    /// Accept invalid server certificates (development only).
    pub const TLS_SKIP_VERIFY: &str = "IBKR_GATEWAY_TLS_SKIP_VERIFY";
}

/// Default gateway URL, matching the gateway's own `BIND_ADDR` default.
pub const DEFAULT_URL: &str = "https://127.0.0.1:8080";
/// Default CA certificate location (dev certs, repo root).
pub const DEFAULT_CA_CERT: &str = ".certs/ca.pem";
/// Default client-certificate location (dev certs, repo root).
pub const DEFAULT_CLIENT_CERT: &str = ".certs/client.pem";
/// Default client-key location (dev certs, repo root).
pub const DEFAULT_CLIENT_KEY: &str = ".certs/client-key.pem";
/// Default request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Default retries for rate-limited reads (`0` disables retrying).
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Resolved client configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Gateway base URL, without a trailing slash.
    pub base_url: String,
    /// Bearer token, when configured.
    pub token: Option<String>,
    /// CA certificate used to verify the gateway's server certificate.
    pub ca_cert: Option<PathBuf>,
    /// Client certificate for mutual TLS.
    pub client_cert: Option<PathBuf>,
    /// Client private key for mutual TLS.
    pub client_key: Option<PathBuf>,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Retries for rate-limited (`429`) reads; `0` disables retrying.
    pub max_retries: u32,
    /// Whether TLS verification is disabled (development only).
    pub tls_skip_verify: bool,
}

/// Command-line values that override the environment.
///
/// `None` means "not supplied on the command line"; the corresponding
/// environment variable and then the built-in default apply.
#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    /// Override for [`var::URL`].
    pub base_url: Option<String>,
    /// Override for [`var::TOKEN`].
    pub token: Option<String>,
    /// Override for [`var::CA_CERT`].
    pub ca_cert: Option<PathBuf>,
    /// Override for [`var::CLIENT_CERT`].
    pub client_cert: Option<PathBuf>,
    /// Override for [`var::CLIENT_KEY`].
    pub client_key: Option<PathBuf>,
    /// Override for [`var::TIMEOUT_SEC`].
    pub timeout: Option<Duration>,
    /// Override for [`var::MAX_RETRIES`].
    pub max_retries: Option<u32>,
    /// Override for [`var::TLS_SKIP_VERIFY`].
    pub tls_skip_verify: Option<bool>,
}

impl Config {
    /// Resolve configuration: command line beats environment beats defaults.
    ///
    /// Certificate paths fall back to the repo-local dev certificates only when
    /// those files exist, so the client stays usable outside this repository.
    /// The client keypair is all-or-nothing: supplying one half without the
    /// other is an error rather than a silent mix with the defaults.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when the URL scheme is unsupported or only
    /// one half of the client keypair was configured.
    pub fn resolve(overrides: ConfigOverrides) -> Result<Self> {
        Self::resolve_with(overrides, |key| env::var(key).ok())
    }

    /// Resolve against a custom environment lookup.
    ///
    /// This is the seam the tests use: it keeps resolution a pure function of
    /// its inputs instead of mutating process-global environment state.
    ///
    /// # Errors
    ///
    /// Same as [`Config::resolve`].
    pub fn resolve_with<F>(overrides: ConfigOverrides, lookup: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let base_url = overrides
            .base_url
            .or_else(|| env_string(&lookup, var::URL))
            .unwrap_or_else(|| DEFAULT_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();

        let token = overrides.token.or_else(|| env_string(&lookup, var::TOKEN));

        let timeout = overrides
            .timeout
            .or_else(|| {
                env_string(&lookup, var::TIMEOUT_SEC)
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(Duration::from_secs)
            })
            .unwrap_or(DEFAULT_TIMEOUT);

        // The client keypair is all-or-nothing per source: when either half is
        // configured explicitly, the other half must be too. Filling the
        // missing half from the repo defaults would silently pair a supplied
        // certificate with an unrelated local key.
        let explicit_cert = overrides
            .client_cert
            .or_else(|| env_string(&lookup, var::CLIENT_CERT).map(PathBuf::from));
        let explicit_key = overrides
            .client_key
            .or_else(|| env_string(&lookup, var::CLIENT_KEY).map(PathBuf::from));
        let (client_cert, client_key) = if explicit_cert.is_some() || explicit_key.is_some() {
            (explicit_cert, explicit_key)
        } else {
            (
                existing_default(DEFAULT_CLIENT_CERT),
                existing_default(DEFAULT_CLIENT_KEY),
            )
        };

        let config = Self {
            base_url,
            token,
            ca_cert: path_override(overrides.ca_cert, var::CA_CERT, DEFAULT_CA_CERT, &lookup),
            client_cert,
            client_key,
            timeout,
            max_retries: overrides
                .max_retries
                .or_else(|| {
                    env_string(&lookup, var::MAX_RETRIES)
                        .and_then(|value| value.parse::<u32>().ok())
                })
                .unwrap_or(DEFAULT_MAX_RETRIES),
            tls_skip_verify: overrides
                .tls_skip_verify
                .or_else(|| {
                    env_string(&lookup, var::TLS_SKIP_VERIFY).map(|value| is_truthy(&value))
                })
                .unwrap_or(false),
        };
        config.validate()?;
        Ok(config)
    }

    /// Whether the configured endpoint uses TLS.
    pub fn is_https(&self) -> bool {
        self.base_url.starts_with("https://")
    }

    fn validate(&self) -> Result<()> {
        if !(self.base_url.starts_with("http://") || self.is_https()) {
            return Err(Error::Config(format!(
                "base URL must start with http:// or https:// (got {})",
                self.base_url
            )));
        }
        if self.client_cert.is_some() != self.client_key.is_some() {
            return Err(Error::Config(
                "client certificate and client key must be configured together".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Read a variable through `lookup`, treating empty values as unset.
fn env_string<F>(lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key).filter(|value| !value.trim().is_empty())
}

/// Resolve a certificate path: explicit value, else environment variable, else
/// the default path when that file exists on disk.
fn path_override<F>(
    explicit: Option<PathBuf>,
    key: &str,
    default: &str,
    lookup: &F,
) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if explicit.is_some() {
        return explicit;
    }
    if let Some(value) = env_string(lookup, key) {
        return Some(PathBuf::from(value));
    }
    existing_default(default)
}

/// The default path, when that file exists on disk.
fn existing_default(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    candidate.exists().then(|| candidate.to_path_buf())
}

/// Interpret a boolean-ish environment value.
fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build an environment lookup from `(key, value)` pairs.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    #[test]
    fn defaults_apply_when_nothing_is_configured() {
        let config = Config::resolve_with(ConfigOverrides::default(), env(&[])).unwrap();
        assert_eq!(config.base_url, DEFAULT_URL);
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
        assert_eq!(config.max_retries, DEFAULT_MAX_RETRIES);
        assert!(config.token.is_none());
        assert!(!config.tls_skip_verify);
        assert!(config.is_https());
    }

    #[test]
    fn environment_beats_defaults() {
        let lookup = env(&[
            (var::URL, "https://gateway.internal:8443"),
            (var::TOKEN, "token-from-env"),
            (var::TIMEOUT_SEC, "5"),
            (var::MAX_RETRIES, "7"),
            (var::TLS_SKIP_VERIFY, "yes"),
        ]);
        let config = Config::resolve_with(ConfigOverrides::default(), lookup).unwrap();
        assert_eq!(config.base_url, "https://gateway.internal:8443");
        assert_eq!(config.token.as_deref(), Some("token-from-env"));
        assert_eq!(config.timeout, Duration::from_secs(5));
        assert_eq!(config.max_retries, 7);
        assert!(config.tls_skip_verify);
    }

    #[test]
    fn flags_beat_environment() {
        let lookup = env(&[
            (var::URL, "https://from-env:8080"),
            (var::TOKEN, "token-from-env"),
            (var::TIMEOUT_SEC, "5"),
        ]);
        let overrides = ConfigOverrides {
            base_url: Some("https://from-flag:9443".to_owned()),
            token: Some("token-from-flag".to_owned()),
            timeout: Some(Duration::from_secs(30)),
            max_retries: Some(0),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(overrides, lookup).unwrap();
        assert_eq!(config.base_url, "https://from-flag:9443");
        assert_eq!(config.token.as_deref(), Some("token-from-flag"));
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.max_retries, 0, "0 must survive as 'retrying off'");
    }

    #[test]
    fn trailing_slash_is_trimmed() {
        let overrides = ConfigOverrides {
            base_url: Some("https://gateway:8080/".to_owned()),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(overrides, env(&[])).unwrap();
        assert_eq!(config.base_url, "https://gateway:8080");
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        let overrides = ConfigOverrides {
            base_url: Some("ftp://gateway:8080".to_owned()),
            ..ConfigOverrides::default()
        };
        let error = Config::resolve_with(overrides, env(&[])).unwrap_err();
        assert!(matches!(error, Error::Config(_)), "got {error:?}");
    }

    #[test]
    fn client_keypair_is_all_or_nothing() {
        // Regression: an explicit half must not be completed from the repo
        // defaults, which would pair a supplied certificate with a local key.
        for overrides in [
            ConfigOverrides {
                client_cert: Some(PathBuf::from("client.pem")),
                ..ConfigOverrides::default()
            },
            ConfigOverrides {
                client_key: Some(PathBuf::from("client-key.pem")),
                ..ConfigOverrides::default()
            },
        ] {
            let error = Config::resolve_with(overrides, env(&[])).unwrap_err();
            assert!(matches!(error, Error::Config(_)), "got {error:?}");
        }

        // Both halves supplied together resolve unchanged.
        let overrides = ConfigOverrides {
            client_cert: Some(PathBuf::from("client.pem")),
            client_key: Some(PathBuf::from("client-key.pem")),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(overrides, env(&[])).unwrap();
        assert_eq!(config.client_cert, Some(PathBuf::from("client.pem")));
        assert_eq!(config.client_key, Some(PathBuf::from("client-key.pem")));
    }

    #[test]
    fn empty_environment_values_count_as_unset() {
        let lookup = env(&[(var::TOKEN, "   "), (var::TIMEOUT_SEC, "")]);
        let config = Config::resolve_with(ConfigOverrides::default(), lookup).unwrap();
        assert!(config.token.is_none());
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn path_override_prefers_explicit_then_env_then_existing_default() {
        let lookup = env(&[(var::CA_CERT, "from-env.pem")]);

        // Explicit value wins over the environment.
        let explicit = path_override(
            Some(PathBuf::from("explicit.pem")),
            var::CA_CERT,
            "Cargo.toml",
            &lookup,
        );
        assert_eq!(explicit, Some(PathBuf::from("explicit.pem")));

        // Environment wins over the default.
        let from_env = path_override(None, var::CA_CERT, "Cargo.toml", &lookup);
        assert_eq!(from_env, Some(PathBuf::from("from-env.pem")));
    }

    #[test]
    fn default_path_applies_only_when_the_file_exists() {
        // Tests run with the crate root as the working directory.
        let none = path_override(None, var::CA_CERT, "missing-file.pem", &env(&[]));
        assert_eq!(none, None);

        let existing = path_override(None, var::CA_CERT, "Cargo.toml", &env(&[]));
        assert_eq!(existing, Some(PathBuf::from("Cargo.toml")));
    }

    #[test]
    fn truthy_spellings() {
        for value in ["1", "true", "TRUE", "Yes", "on", " on "] {
            assert!(is_truthy(value), "{value} should be truthy");
        }
        for value in ["0", "false", "no", "off", ""] {
            assert!(!is_truthy(value), "{value} should not be truthy");
        }
    }
}
