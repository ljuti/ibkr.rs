//! Client configuration, resolved from CLI flags, the environment and defaults.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

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
    /// Local store database file used by `ibkr store`.
    pub const STORE_DB: &str = "IBKR_STORE_DB";
    /// Configuration file holding settings written by `ibkr configure`.
    pub const CONFIG: &str = "IBKR_CONFIG";
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
/// Default local store database (relative to the working directory).
pub const DEFAULT_STORE_DB: &str = "ibkr.db";
/// Default configuration file, under the user's config directory
/// (`$XDG_CONFIG_HOME/ibkr/config.toml`, `~/.config/ibkr/config.toml`).
pub const DEFAULT_CONFIG_FILE: &str = "ibkr/config.toml";

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
    /// `SQLite` file backing `ibkr store`.
    pub db: PathBuf,
    /// Configuration file these settings were resolved with.
    pub config_path: PathBuf,
    /// What that file contained (empty when it does not exist).
    pub file: ConfigFile,
    /// Which layer supplied each setting, keyed as in [`ConfigFile`].
    pub sources: BTreeMap<&'static str, Source>,
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
    /// Override for [`var::STORE_DB`].
    pub db: Option<PathBuf>,
    /// Override for [`var::CONFIG`]: which file to read and write.
    pub config: Option<PathBuf>,
}

/// Settings that live in the configuration file.
///
/// Field names mirror the command-line flags, so a key reads the same however
/// it is set: `--ca-cert`, `IBKR_GATEWAY_CA_CERT` or `ca-cert = "…"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ConfigFile {
    /// Gateway base URL.
    pub url: Option<String>,
    /// Bearer token presented on `/api/v1/*` requests.
    pub token: Option<String>,
    /// PEM file with the CA that signed the gateway's server certificate.
    pub ca_cert: Option<PathBuf>,
    /// PEM client certificate for mutual TLS.
    pub client_cert: Option<PathBuf>,
    /// PEM private key matching `client-cert`.
    pub client_key: Option<PathBuf>,
    /// Request timeout in seconds.
    pub timeout: Option<u64>,
    /// Retries for rate-limited reads.
    pub max_retries: Option<u32>,
    /// Accept invalid server certificates (development only).
    pub tls_skip_verify: Option<bool>,
    /// Local store database used by `ibkr store`.
    pub db: Option<PathBuf>,
}

impl ConfigFile {
    /// Read the file, or an empty configuration when it does not exist yet.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when the file exists but cannot be read or
    /// parsed, and [`Error::Io`] for a read failure that is not a missing file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        toml::from_str(&text).map_err(|error| Error::Config(format!("{}: {error}", path.display())))
    }

    /// Write the file, readable only by its owner: it holds a bearer token.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`] when the settings cannot be serialized and
    /// [`Error::Io`] when the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|error| Error::Config(format!("cannot serialize configuration: {error}")))?;
        let text = format!(
            "# Written by `ibkr configure`. Settings resolve as:\n\
             #   flags > environment > this file > defaults.\n{body}"
        );
        write_private(path, &text)?;
        Ok(())
    }

    /// Whether the file has a mode anyone else can read.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the metadata cannot be read.
    pub fn is_world_readable(path: &Path) -> Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(path)?.permissions().mode();
            Ok(mode & 0o077 != 0)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(false)
        }
    }
}

/// Write a file only its owner can read.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::os::unix::fs::PermissionsExt as _;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        // An existing file keeps its old mode through `open`, so set it too.
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, contents)
    }
}

/// Where a resolved setting came from, for `configure --show`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Source {
    /// A command-line flag.
    Flag,
    /// An environment variable.
    Environment,
    /// The configuration file.
    File,
    /// A built-in default (or nothing at all, for settings that may be unset).
    #[default]
    Default,
}

impl Source {
    /// Human-readable name used in diagnostics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Environment => "env",
            Self::File => "file",
            Self::Default => "default",
        }
    }
}

/// Configuration file the client reads and `ibkr configure` writes.
///
/// `--config` beats `IBKR_CONFIG`, which beats the XDG default. Nothing is
/// created until `configure` runs: a missing file is simply empty settings.
pub fn config_path<F>(overrides: &ConfigOverrides, lookup: &F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(path) = &overrides.config {
        return path.clone();
    }
    if let Some(path) = env_string(lookup, var::CONFIG) {
        return PathBuf::from(path);
    }
    let base = env_string(lookup, "XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env_string(lookup, "HOME").map(|home| PathBuf::from(home).join(".config")));
    match base {
        Some(directory) => directory.join(DEFAULT_CONFIG_FILE),
        None => PathBuf::from(DEFAULT_CONFIG_FILE),
    }
}

/// Resolve one setting from the layers that can supply it.
fn setting<T>(flag: Option<T>, environment: Option<T>, file: Option<T>, default: T) -> (T, Source) {
    if let Some(value) = flag {
        return (value, Source::Flag);
    }
    if let Some(value) = environment {
        return (value, Source::Environment);
    }
    if let Some(value) = file {
        return (value, Source::File);
    }
    (default, Source::Default)
}

/// Resolve one optional setting (a token or a certificate path).
fn optional_setting<T>(
    flag: Option<T>,
    environment: Option<T>,
    file: Option<T>,
) -> (Option<T>, Source) {
    if let Some(value) = flag {
        return (Some(value), Source::Flag);
    }
    if let Some(value) = environment {
        return (Some(value), Source::Environment);
    }
    if let Some(value) = file {
        return (Some(value), Source::File);
    }
    (None, Source::Default)
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
        let config_path = config_path(&overrides, &lookup);
        let file = ConfigFile::load(&config_path)?;
        Self::resolve_layers(overrides, lookup, file, config_path)
    }

    /// Resolve from an already-read configuration file.
    ///
    /// Split out so the wizard can resolve without re-reading the file it is
    /// about to write.
    pub(crate) fn resolve_layers<F>(
        overrides: ConfigOverrides,
        lookup: F,
        file: ConfigFile,
        config_path: PathBuf,
    ) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut sources = BTreeMap::new();
        let record =
            |sources: &mut BTreeMap<&'static str, Source>, key: &'static str, source: Source| {
                sources.insert(key, source);
            };

        let (base_url, source) = setting(
            overrides.base_url,
            env_string(&lookup, var::URL),
            file.url.clone(),
            DEFAULT_URL.to_owned(),
        );
        record(&mut sources, "url", source);

        let (token, source) = optional_setting(
            overrides.token,
            env_string(&lookup, var::TOKEN),
            file.token.clone(),
        );
        record(&mut sources, "token", source);

        let (timeout_secs, source) = setting(
            overrides.timeout.map(|value| value.as_secs()),
            env_string(&lookup, var::TIMEOUT_SEC).and_then(|value| value.parse::<u64>().ok()),
            file.timeout,
            DEFAULT_TIMEOUT.as_secs(),
        );
        record(&mut sources, "timeout", source);

        let (max_retries, source) = setting(
            overrides.max_retries,
            env_string(&lookup, var::MAX_RETRIES).and_then(|value| value.parse::<u32>().ok()),
            file.max_retries,
            DEFAULT_MAX_RETRIES,
        );
        record(&mut sources, "max-retries", source);

        let (tls_skip_verify, source) = setting(
            overrides.tls_skip_verify,
            env_string(&lookup, var::TLS_SKIP_VERIFY).map(|value| is_truthy(&value)),
            file.tls_skip_verify,
            false,
        );
        record(&mut sources, "tls-skip-verify", source);

        let (db, source) = setting(
            overrides.db,
            env_string(&lookup, var::STORE_DB).map(PathBuf::from),
            file.db.clone(),
            PathBuf::from(DEFAULT_STORE_DB),
        );
        record(&mut sources, "db", source);

        // The client keypair is all-or-nothing per source: when either half is
        // configured anywhere but the defaults, the other must be too. Filling
        // the missing half from the repo defaults would silently pair a
        // supplied certificate with an unrelated local key.
        let flag_cert = overrides.client_cert;
        let env_cert = env_string(&lookup, var::CLIENT_CERT).map(PathBuf::from);
        let flag_key = overrides.client_key;
        let env_key = env_string(&lookup, var::CLIENT_KEY).map(PathBuf::from);
        let configured = flag_cert.is_some()
            || env_cert.is_some()
            || file.client_cert.is_some()
            || flag_key.is_some()
            || env_key.is_some()
            || file.client_key.is_some();
        let (client_cert, cert_source) = if configured {
            optional_setting(flag_cert, env_cert, file.client_cert.clone())
        } else {
            (existing_default(DEFAULT_CLIENT_CERT), Source::Default)
        };
        let (client_key, key_source) = if configured {
            optional_setting(flag_key, env_key, file.client_key.clone())
        } else {
            (existing_default(DEFAULT_CLIENT_KEY), Source::Default)
        };
        record(&mut sources, "client-cert", cert_source);
        record(&mut sources, "client-key", key_source);

        let (ca_cert, source) = optional_setting(
            overrides.ca_cert,
            env_string(&lookup, var::CA_CERT).map(PathBuf::from),
            file.ca_cert.clone(),
        );
        let ca_cert = ca_cert.or_else(|| existing_default(DEFAULT_CA_CERT));
        record(&mut sources, "ca-cert", source);

        let config = Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            token,
            ca_cert,
            client_cert,
            client_key,
            timeout: Duration::from_secs(timeout_secs),
            max_retries,
            tls_skip_verify,
            db,
            config_path,
            file,
            sources,
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

/// The default path, when that file exists on disk.
fn existing_default(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    candidate.exists().then(|| candidate.to_path_buf())
}

/// Interpret a boolean-ish environment value.
pub fn is_truthy(value: &str) -> bool {
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
        assert_eq!(config.db, PathBuf::from(DEFAULT_STORE_DB));
    }

    #[test]
    fn store_path_follows_flag_then_environment_then_default() {
        // Unlike the certificates, the store file need not exist yet: it is
        // created on the first sync, so no existence check may filter it out.
        let lookup = env(&[(var::STORE_DB, "from-env/trades.db")]);
        let config = Config::resolve_with(ConfigOverrides::default(), lookup).unwrap();
        assert_eq!(config.db, PathBuf::from("from-env/trades.db"));

        let lookup = env(&[(var::STORE_DB, "from-env/trades.db")]);
        let overrides = ConfigOverrides {
            db: Some(PathBuf::from("from-flag/trades.db")),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(overrides, lookup).unwrap();
        assert_eq!(config.db, PathBuf::from("from-flag/trades.db"));
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
    fn settings_resolve_flag_then_environment_then_file_then_default() {
        let path = temp_config("layers");
        let file = ConfigFile {
            url: Some("https://from-file:1".to_owned()),
            timeout: Some(11),
            ..ConfigFile::default()
        };
        file.save(&path).unwrap();

        // File beats defaults.
        let lookup = env(&[]);
        let overrides = ConfigOverrides {
            config: Some(path.clone()),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(overrides.clone(), lookup).unwrap();
        assert_eq!(config.base_url, "https://from-file:1");
        assert_eq!(config.timeout, Duration::from_secs(11));
        assert_eq!(config.sources.get("url"), Some(&Source::File));

        // Environment beats file.
        let lookup = env(&[(var::URL, "https://from-env:2"), (var::TIMEOUT_SEC, "22")]);
        let config = Config::resolve_with(overrides.clone(), lookup).unwrap();
        assert_eq!(config.base_url, "https://from-env:2");
        assert_eq!(config.timeout, Duration::from_secs(22));
        assert_eq!(config.sources.get("url"), Some(&Source::Environment));

        // Flags beat environment.
        let lookup = env(&[(var::URL, "https://from-env:2")]);
        let flags = ConfigOverrides {
            config: Some(path.clone()),
            base_url: Some("https://from-flag:3".to_owned()),
            ..ConfigOverrides::default()
        };
        let config = Config::resolve_with(flags, lookup).unwrap();
        assert_eq!(config.base_url, "https://from-flag:3");
        assert_eq!(config.sources.get("url"), Some(&Source::Flag));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn a_missing_config_file_is_empty_settings_not_an_error() {
        let path = temp_config("absent");
        let config = Config::resolve_with(
            ConfigOverrides {
                config: Some(path.clone()),
                ..ConfigOverrides::default()
            },
            env(&[]),
        )
        .unwrap();
        assert_eq!(config.base_url, DEFAULT_URL);
        assert_eq!(config.config_path, path);
        assert!(config.file.url.is_none());
    }

    #[test]
    fn a_malformed_config_file_says_which_file_it_is() {
        let path = temp_config("malformed");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "url = https://not-quoted\n").unwrap();
        let error = Config::resolve_with(
            ConfigOverrides {
                config: Some(path.clone()),
                ..ConfigOverrides::default()
            },
            env(&[]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn an_unknown_config_key_is_rejected_rather_than_ignored() {
        let path = temp_config("unknown-key");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "url = \"https://host\"\ntypo = 1\n").unwrap();
        let error = Config::resolve_with(
            ConfigOverrides {
                config: Some(path.clone()),
                ..ConfigOverrides::default()
            },
            env(&[]),
        )
        .unwrap_err();
        assert!(error.to_string().contains("typo"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn the_config_file_is_written_for_the_owner_only() {
        let path = temp_config("private");
        let file = ConfigFile {
            token: Some("secret-token".to_owned()),
            ..ConfigFile::default()
        };
        file.save(&path).unwrap();

        let written = ConfigFile::load(&path).unwrap();
        assert_eq!(written.token.as_deref(), Some("secret-token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "a file holding a token must be private"
            );
        }
        assert!(!ConfigFile::is_world_readable(&path).unwrap());
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn config_path_follows_flag_then_environment_then_xdg() {
        let lookup = env(&[
            ("XDG_CONFIG_HOME", "/xdg/config"),
            ("HOME", "/home/someone"),
        ]);
        let overrides = ConfigOverrides::default();
        assert_eq!(
            config_path(&overrides, &lookup),
            PathBuf::from("/xdg/config").join(DEFAULT_CONFIG_FILE)
        );

        let lookup = env(&[("HOME", "/home/someone")]);
        assert_eq!(
            config_path(&overrides, &lookup),
            PathBuf::from("/home/someone/.config").join(DEFAULT_CONFIG_FILE)
        );

        let lookup = env(&[
            (var::CONFIG, "/custom/config.toml"),
            ("XDG_CONFIG_HOME", "/xdg/config"),
        ]);
        assert_eq!(
            config_path(&overrides, &lookup),
            PathBuf::from("/custom/config.toml")
        );

        let lookup = env(&[(var::CONFIG, "/custom/config.toml")]);
        let flags = ConfigOverrides {
            config: Some(PathBuf::from("/flag/config.toml")),
            ..ConfigOverrides::default()
        };
        assert_eq!(
            config_path(&flags, &lookup),
            PathBuf::from("/flag/config.toml")
        );
    }

    /// A config file path inside its own temporary directory.
    fn temp_config(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("ibkr-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory.join("config.toml")
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
