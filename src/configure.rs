//! First-run configuration: collect the settings and write them to disk.
//!
//! Every command resolves settings from flags, then the environment, then the
//! configuration file, then defaults (see [`crate::config`]). This module fills
//! that file in, so regular use needs no exported variables at all — and, at
//! the end, says what is *effective*, because an ambient variable still beats
//! the file and that is worth seeing while configuring rather than later.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::cli::RenderMode;
use crate::client::Client;
use crate::config::{Config, ConfigFile, ConfigOverrides, Source, is_truthy};
use crate::error::{Error, Result};
use crate::output;

/// What the wizard collected. `None` means "keep what is already there", which
/// is also what pressing enter at a prompt means.
#[derive(Debug, Default)]
struct Answers {
    url: Option<String>,
    token: Option<String>,
    ca_cert: Option<String>,
    client_cert: Option<String>,
    client_key: Option<String>,
    timeout: Option<String>,
    max_retries: Option<String>,
    tls_skip_verify: Option<String>,
    db: Option<String>,
}

impl Answers {
    /// Overlay the answers on the file already on disk.
    fn apply(self, file: &mut ConfigFile) {
        if let Some(value) = self.url {
            file.url = Some(value);
        }
        if let Some(value) = self.token {
            file.token = Some(value);
        }
        if let Some(value) = self.ca_cert {
            file.ca_cert = Some(PathBuf::from(value));
        }
        if let Some(value) = self.client_cert {
            file.client_cert = Some(PathBuf::from(value));
        }
        if let Some(value) = self.client_key {
            file.client_key = Some(PathBuf::from(value));
        }
        if let Some(value) = self.timeout.and_then(|value| value.parse::<u64>().ok()) {
            file.timeout = Some(value);
        }
        if let Some(value) = self.max_retries.and_then(|value| value.parse::<u32>().ok()) {
            file.max_retries = Some(value);
        }
        if let Some(value) = self.tls_skip_verify {
            file.tls_skip_verify = Some(is_truthy(&value));
        }
        if let Some(value) = self.db {
            file.db = Some(PathBuf::from(value));
        }
    }

    /// The answers that came from flags, in non-interactive mode.
    fn from_overrides(overrides: &ConfigOverrides) -> Self {
        Self {
            url: overrides.base_url.clone(),
            token: overrides.token.clone(),
            ca_cert: overrides.ca_cert.clone().map(|path| display(&path)),
            client_cert: overrides.client_cert.clone().map(|path| display(&path)),
            client_key: overrides.client_key.clone().map(|path| display(&path)),
            timeout: overrides.timeout.map(|value| value.as_secs().to_string()),
            max_retries: overrides.max_retries.map(|value| value.to_string()),
            tls_skip_verify: overrides.tls_skip_verify.map(|value| value.to_string()),
            db: overrides.db.clone().map(|path| display(&path)),
        }
    }

    fn is_empty(&self) -> bool {
        self.url.is_none()
            && self.token.is_none()
            && self.ca_cert.is_none()
            && self.client_cert.is_none()
            && self.client_key.is_none()
            && self.timeout.is_none()
            && self.max_retries.is_none()
            && self.tls_skip_verify.is_none()
            && self.db.is_none()
    }
}

/// Run `ibkr configure`.
///
/// # Errors
///
/// Returns [`Error::Invalid`] when there is nothing to write and no terminal to
/// prompt from, [`Error::Aborted`] when the user ends the prompts, and any
/// error from the optional connectivity check.
pub async fn run(
    overrides: ConfigOverrides,
    show: bool,
    verify: bool,
    mode: RenderMode,
) -> Result<()> {
    let config = Config::resolve(overrides.clone())?;

    if show {
        if config.config_path.exists() && ConfigFile::is_world_readable(&config.config_path)? {
            output::note(&format!(
                "warning: {} is readable by other users; it holds the bearer token",
                config.config_path.display()
            ));
        }
        return output::configure_show(&config, mode);
    }

    let mut file = config.file.clone();
    if interactive() {
        Answers::from_prompts(&config)?.apply(&mut file);
    } else {
        let answers = Answers::from_overrides(&overrides);
        if answers.is_empty() && config.file_is_empty() {
            return Err(Error::Invalid(
                "nothing to configure: pass the settings as flags (for example \
                 `ibkr configure --url https://host:8090 --token <token>`), or run in a \
                 terminal to be prompted"
                    .to_owned(),
            ));
        }
        answers.apply(&mut file);
    }

    file.save(&config.config_path)?;
    output::note(&format!(
        "wrote {} (mode 0600)",
        config.config_path.display()
    ));

    // What a command would use *now*. Ambient variables still win over the
    // file, so showing this here turns a shadowed setting into something the
    // user can see rather than a puzzling failure later.
    let effective = Config::resolve_layers(
        overrides,
        |key| std::env::var(key).ok(),
        file,
        config.config_path.clone(),
    )?;
    output::configure_show(&effective, mode)?;

    if verify || interactive() {
        check(&effective).await?;
    }
    Ok(())
}

impl Answers {
    /// Prompt for every setting, defaulting to what it resolves to today.
    fn from_prompts(config: &Config) -> Result<Self> {
        output::note("press enter to keep what is shown; ctrl-c aborts");
        Ok(Self {
            url: ask(&Prompt::new("gateway url", &config.base_url, config, "url"))?,
            token: ask_secret(&Prompt::new(
                "bearer token",
                if config.token.is_some() {
                    "(set)"
                } else {
                    "unset"
                },
                config,
                "token",
            ))?,
            ca_cert: ask(&Prompt::new(
                "ca certificate",
                &display_opt(config.ca_cert.as_ref()),
                config,
                "ca-cert",
            ))?,
            client_cert: ask(&Prompt::new(
                "client certificate",
                &display_opt(config.client_cert.as_ref()),
                config,
                "client-cert",
            ))?,
            client_key: ask(&Prompt::new(
                "client key",
                &display_opt(config.client_key.as_ref()),
                config,
                "client-key",
            ))?,
            timeout: ask(&Prompt::new(
                "timeout seconds",
                &config.timeout.as_secs().to_string(),
                config,
                "timeout",
            ))?,
            max_retries: ask(&Prompt::new(
                "max retries",
                &config.max_retries.to_string(),
                config,
                "max-retries",
            ))?,
            tls_skip_verify: ask(&Prompt::new(
                "skip tls verification",
                &config.tls_skip_verify.to_string(),
                config,
                "tls-skip-verify",
            ))?,
            db: ask(&Prompt::new(
                "store database",
                &display(&config.db),
                config,
                "db",
            ))?,
        })
    }
}

/// One question: its name, the value it would keep, and where that comes from.
struct Prompt<'a> {
    label: &'a str,
    current: &'a str,
    source: Source,
}

impl<'a> Prompt<'a> {
    fn new(label: &'a str, current: &'a str, config: &Config, key: &str) -> Self {
        Self {
            label,
            current,
            source: config.source(key),
        }
    }

    /// `label [current] (from env): `
    fn render(&self) -> String {
        let current = match self.current.trim() {
            "" => "unset",
            value => value,
        };
        match self.source {
            Source::Default => format!("{} [{current}]: ", self.label),
            source => format!("{} [{current}] (from {}): ", self.label, source.label()),
        }
    }
}

/// Whether the process can talk to a user.
fn interactive() -> bool {
    io::stdin().is_terminal()
}

/// Prompt for a value; an empty answer keeps whatever is already configured.
fn ask(prompt: &Prompt<'_>) -> Result<Option<String>> {
    eprint!("{}", prompt.render());
    io::stderr().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer)? == 0 {
        return Err(Error::Aborted);
    }
    Ok(non_empty(answer.trim()))
}

/// Prompt for a secret without echoing it, when the terminal allows it.
///
/// The token must not be left on screen, so `stty` turns echo off around the
/// read; a terminal that refuses to is not worth failing the configuration
/// over, so the prompt falls back to echoing with a warning.
fn ask_secret(prompt: &Prompt<'_>) -> Result<Option<String>> {
    let hidden = config_echo(false).is_ok();
    if !hidden {
        output::note(&format!(
            "warning: cannot disable terminal echo, `{}` will be visible",
            prompt.label
        ));
    }
    let result = ask_hidden(prompt, hidden);
    if hidden {
        let _ = config_echo(true);
    }
    result
}

/// As [`ask`], aware that the typed newline was not echoed.
fn ask_hidden(prompt: &Prompt<'_>, hidden: bool) -> Result<Option<String>> {
    eprint!("{}", prompt.render());
    io::stderr().flush()?;
    let mut answer = String::new();
    let read = io::stdin().read_line(&mut answer);
    if hidden {
        eprintln!();
    }
    if read? == 0 {
        return Err(Error::Aborted);
    }
    Ok(non_empty(answer.trim()))
}

/// Turn terminal echo off (`false`) or back on (`true`).
fn config_echo(on: bool) -> io::Result<()> {
    let status = std::process::Command::new("stty")
        .arg(if on { "echo" } else { "-echo" })
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("stty failed"))
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn display_opt(path: Option<&PathBuf>) -> String {
    path.map_or_else(String::new, |path| path.display().to_string())
}

impl Config {
    /// Which layer supplied a setting, by [`ConfigFile`] field name.
    fn source(&self, key: &str) -> Source {
        self.sources.get(key).copied().unwrap_or(Source::Default)
    }

    /// Whether the configuration file holds nothing yet.
    fn file_is_empty(&self) -> bool {
        self.file.url.is_none() && self.file.token.is_none() && self.file.db.is_none()
    }
}

/// Report whether the settings actually reach the gateway.
async fn check(config: &Config) -> Result<()> {
    output::note("checking the connection…");
    let client = Client::new(config)?;
    match client.health().await {
        Ok(health) => {
            output::note(&format!(
                "connected: {} (server version {}, client id {})",
                health.status, health.server_version, health.client_id
            ));
            Ok(())
        }
        Err(error) => {
            output::note(&format!("connection check failed: {error}"));
            Err(error)
        }
    }
}
