//! The Solid Edge provider: honest probe, no fake build (ADR-0008/0011).
//!
//! Solid Edge runs on Windows behind the ADR-0008 loopback bridge. This module
//! does not import a bridge distribution; it resolves the bridge URL and its
//! token file exactly as the Python provider does, then asks the authenticated
//! `/v1/health` route **through the accepted `kala-solidedge-client`**. On a
//! non-Windows host it refuses without reading a token or opening a socket.
//!
//! The bridge milestone that landed has no production native executor and
//! reports `supported: false`. A build is therefore refused with a
//! [`CadProviderUnavailableError`]; nothing is ever returned as a success. The
//! exact off-Windows refusal text is part of the ADR-0011 contract:
//! `Solid Edge build requires the Windows bridge (not reachable)`.
//!
//! # Intentional differences from the Python provider
//!
//! * The health document is parsed by the accepted client's protocol-v1
//!   `Health` model, so a document with a wrong protocol version, unknown
//!   fields, a missing required field, an over-long `se_version`, or more
//!   than 64 KiB is reported as "did not answer a valid health probe".
//!   Python reads any JSON object and looks up four fields; the real bridge
//!   document parses in both.
//! * A `localhost` URL is transported as `127.0.0.1` because the accepted
//!   client accepts literal loopback addresses only. The resolved URL used in
//!   diagnostic messages still says `localhost`, like Python's.
//! * A missing home directory makes the default token path unknown and the
//!   probe returns the token-file reason instead of Python's `RuntimeError`
//!   from `Path.home()`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use kala_contracts::ConstructionScript;
use kala_solidedge_client::{BridgeClient, Health};

use crate::base::{
    Availability, CadProvider, CadProviderBuildError, CadProviderConfigError, CadProviderError,
    CadProviderUnavailableError, ProviderBuild,
};

/// The loopback bridge's default address.
pub const DEFAULT_BRIDGE_URL: &str = "http://127.0.0.1:8797";
/// The environment variable that overrides the bridge address.
pub const BRIDGE_URL_ENV_VAR: &str = "KALA_SOLIDEDGE_BRIDGE_URL";
/// The environment variable that overrides the token file.
pub const TOKEN_FILE_ENV_VAR: &str = "KALA_SOLIDEDGE_TOKEN_FILE";
/// The token file's name.
pub const TOKEN_FILENAME: &str = "bridge.token";
/// The default token directory below `%LOCALAPPDATA%`.
pub const DATA_DIR_PARTS: [&str; 2] = ["Kala", "SolidEdgeBridge"];
/// The authenticated health route.
pub const HEALTH_PATH: &str = "/v1/health";
/// The default bounded probe timeout.
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 5.0;
/// Largest health document the Python probe reads.
pub const MAX_HEALTH_BYTES: usize = 64 * 1024;
/// Smallest accepted bearer token.
pub const MIN_TOKEN_LENGTH: usize = 16;
/// Largest accepted bearer token.
pub const MAX_TOKEN_LENGTH: usize = 256;
/// Default probe cache duration.
pub const DEFAULT_PROBE_CACHE_SECONDS: f64 = 5.0;
/// The exact refusal text an unavailable Solid Edge build reports.
pub const NOT_REACHABLE_MESSAGE: &str =
    "Solid Edge build requires the Windows bridge (not reachable)";
/// The fallback unavailable reason.
pub const UNAVAILABLE_REASON: &str =
    "Solid Edge requires Windows with the ADR-0008 loopback bridge; this host cannot run it";

/// Constructor options for [`SolidEdgeProvider`].
#[derive(Clone, Debug, Default)]
pub struct SolidEdgeProviderOptions {
    /// The host platform string. Defaults to Python's `sys.platform` spelling.
    pub platform: Option<String>,
    /// The environment snapshot used for URL, token and child-path lookups.
    pub environ: Option<BTreeMap<String, String>>,
    /// An explicit bridge URL.
    pub url: Option<String>,
    /// An explicit token file.
    pub token_file: Option<PathBuf>,
    /// The bounded health-probe timeout in seconds.
    pub timeout_seconds: Option<f64>,
    /// The probe cache duration in seconds.
    pub probe_cache_seconds: Option<f64>,
}

/// Probe the Windows bridge; never invent a build off Windows.
#[derive(Clone, Debug)]
pub struct SolidEdgeProvider {
    platform: String,
    environ: BTreeMap<String, String>,
    url: Option<String>,
    token_file: Option<PathBuf>,
    timeout_seconds: f64,
    probe_cache_seconds: f64,
    probe_cached: Option<Availability>,
    probe_at: Option<Instant>,
}

impl SolidEdgeProvider {
    /// Validate the options and build a provider.
    pub fn new(options: SolidEdgeProviderOptions) -> Result<Self, CadProviderConfigError> {
        let timeout_seconds = options.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        if timeout_seconds <= 0.0 {
            return Err(CadProviderConfigError::new(
                "the bridge probe timeout must be positive",
            ));
        }
        let probe_cache_seconds = options
            .probe_cache_seconds
            .unwrap_or(DEFAULT_PROBE_CACHE_SECONDS);
        if probe_cache_seconds < 0.0 {
            return Err(CadProviderConfigError::new(
                "the probe cache duration must be non-negative",
            ));
        }
        Ok(Self {
            platform: options.platform.unwrap_or_else(default_platform),
            environ: options
                .environ
                .unwrap_or_else(crate::base::snapshot_environment),
            url: options.url,
            token_file: options.token_file,
            timeout_seconds,
            probe_cache_seconds,
            probe_cached: None,
            probe_at: None,
        })
    }

    /// A provider with the Python defaults.
    pub fn with_defaults() -> Result<Self, CadProviderConfigError> {
        Self::new(SolidEdgeProviderOptions::default())
    }

    /// The platform string this provider reasons about.
    pub fn platform(&self) -> &str {
        &self.platform
    }

    /// The bounded health-probe timeout in seconds.
    pub const fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds
    }

    /// Return whether the bridge is reachable and reports a build capability.
    pub fn probe(&mut self) -> Availability {
        let now = Instant::now();
        if let (Some(cached), Some(at)) = (&self.probe_cached, self.probe_at) {
            if now.duration_since(at).as_secs_f64() < self.probe_cache_seconds {
                return cached.clone();
            }
        }
        let availability = self.probe_uncached();
        self.probe_cached = Some(availability.clone());
        self.probe_at = Some(now);
        availability
    }

    fn probe_uncached(&self) -> Availability {
        if self.platform != "win32" {
            return Availability::new(
                false,
                None::<String>,
                Some(format!(
                    "{NOT_REACHABLE_MESSAGE}; this host is {}, so the ADR-0008 bridge \
                     cannot run here",
                    self.platform
                )),
            );
        }
        let Some(bridge_url) = self.resolve_url() else {
            return Availability::new(
                false,
                None::<String>,
                Some("the Solid Edge bridge URL is not a loopback HTTP URL".to_owned()),
            );
        };
        let Some(token) = self.read_token() else {
            return Availability::new(
                false,
                None::<String>,
                Some(
                    "the Solid Edge bridge token file was not found; run the bridge \
                     token command on this Windows host"
                        .to_owned(),
                ),
            );
        };
        let Some(health) = self.fetch_health(&bridge_url, &token) else {
            return Availability::new(
                false,
                None::<String>,
                Some(format!(
                    "the Solid Edge bridge at {bridge_url} did not answer a valid health probe"
                )),
            );
        };
        let available = health.available && health.supported;
        let reason = if health.detail.trim().is_empty() {
            None
        } else {
            Some(health.detail.clone())
        };
        Availability::new(
            available,
            health.se_version.clone(),
            if available {
                None
            } else {
                Some(
                    reason.unwrap_or_else(|| {
                        "the bridge reports no native build capability".to_owned()
                    }),
                )
            },
        )
    }

    fn resolve_url(&self) -> Option<String> {
        let raw = self
            .url
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.environ
                    .get(BRIDGE_URL_ENV_VAR)
                    .map(String::as_str)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or(DEFAULT_BRIDGE_URL);
        parse_loopback_url(raw)
    }

    fn read_token(&self) -> Option<String> {
        let path = self.token_file_path()?;
        let bytes = std::fs::read(path).ok()?;
        if !bytes.is_ascii() {
            return None;
        }
        let text = String::from_utf8(bytes).ok()?;
        let token = text.trim();
        if !(MIN_TOKEN_LENGTH..=MAX_TOKEN_LENGTH).contains(&token.len()) {
            return None;
        }
        Some(token.to_owned())
    }

    fn token_file_path(&self) -> Option<PathBuf> {
        if let Some(path) = &self.token_file {
            return Some(path.clone());
        }
        if let Some(value) = self
            .environ
            .get(TOKEN_FILE_ENV_VAR)
            .filter(|value| !value.is_empty())
        {
            return Some(PathBuf::from(value));
        }
        let base = match self
            .environ
            .get("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
        {
            Some(value) => PathBuf::from(value),
            None => home_directory(&self.environ)?.join("AppData").join("Local"),
        };
        Some(
            base.join(DATA_DIR_PARTS[0])
                .join(DATA_DIR_PARTS[1])
                .join(TOKEN_FILENAME),
        )
    }

    fn fetch_health(&self, bridge_url: &str, token: &str) -> Option<Health> {
        let transport = transport_url(bridge_url);
        let client = BridgeClient::new(
            &transport,
            token,
            self.timeout_seconds,
            self.timeout_seconds,
        )
        .ok()?;
        client.health().ok()
    }

    /// Refuse honestly: this milestone has no verified native build path.
    pub fn build(
        &mut self,
        _script: &ConstructionScript,
    ) -> Result<ProviderBuild, CadProviderError> {
        let availability = self.probe();
        if !availability.is_available() {
            return Err(CadProviderUnavailableError::new(
                self.name(),
                NOT_REACHABLE_MESSAGE,
                availability
                    .reason()
                    .map(str::to_owned)
                    .or_else(|| Some(UNAVAILABLE_REASON.to_owned())),
                Vec::new(),
            )
            .into());
        }
        Err(CadProviderBuildError::with_op(
            "the Solid Edge bridge is reachable but this Studio worker has no verified \
             native build executor; no Solid Edge build was attempted",
            "bridge.build",
        )
        .into())
    }
}

impl CadProvider for SolidEdgeProvider {
    fn name(&self) -> &'static str {
        "solidedge"
    }

    fn display_name(&self) -> &'static str {
        "Solid Edge"
    }

    fn measured_by(&self) -> &'static str {
        "Solid Edge"
    }

    fn mode(&self) -> &'static str {
        "bridge"
    }

    fn probe(&mut self) -> Availability {
        SolidEdgeProvider::probe(self)
    }

    fn build(&mut self, script: &ConstructionScript) -> Result<ProviderBuild, CadProviderError> {
        SolidEdgeProvider::build(self, script)
    }
}

fn default_platform() -> String {
    if cfg!(target_os = "windows") {
        "win32".to_owned()
    } else if cfg!(target_os = "macos") {
        "darwin".to_owned()
    } else {
        std::env::consts::OS.to_owned()
    }
}

fn home_directory(environ: &BTreeMap<String, String>) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    environ
        .get(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Normalize a loopback bridge URL exactly like the Python provider.
///
/// Only `http`, no user information, no path (except `/`), no query and no
/// fragment, a hostname of `127.0.0.1`, `localhost` or `::1`, and an explicit
/// port in `1..=65535`.
fn parse_loopback_url(raw: &str) -> Option<String> {
    // CPython's `urlsplit` removes leading C0 controls and spaces and removes
    // tab, CR and LF anywhere in the URL before parsing. Trailing spaces are
    // deliberately kept: they become part of the port and make it invalid.
    let trimmed = raw.trim_start_matches(|character: char| character <= '\u{20}');
    let cleaned: String = trimmed
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect();
    let raw = cleaned.as_str();
    let (scheme, rest) = split_scheme(raw)?;
    if scheme != "http" {
        return None;
    }
    let after = rest.strip_prefix("//")?;
    let netloc_end = after.find(['/', '?', '#']).unwrap_or(after.len());
    let netloc = &after[..netloc_end];
    let remainder = &after[netloc_end..];
    let (before_fragment, fragment) = split_once_or_empty(remainder, '#');
    if !fragment.is_empty() {
        return None;
    }
    let (path, query) = split_once_or_empty(before_fragment, '?');
    if !query.is_empty() {
        return None;
    }
    if !(path.is_empty() || path == "/") {
        return None;
    }
    let (userinfo, hostport) = match netloc.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, netloc),
    };
    if let Some(userinfo) = userinfo {
        let (user, password) = match userinfo.split_once(':') {
            Some((user, password)) => (user, Some(password)),
            None => (userinfo, None),
        };
        if !user.is_empty() || password.is_some_and(|value| !value.is_empty()) {
            return None;
        }
    }
    let (host, port) = if let Some(stripped) = hostport.strip_prefix('[') {
        let end = stripped.find(']')?;
        let host = &stripped[..end];
        let after_bracket = &stripped[end + 1..];
        let port = match after_bracket.strip_prefix(':') {
            Some(port) => port,
            None if after_bracket.is_empty() => "",
            None => return None,
        };
        (host.to_ascii_lowercase(), port)
    } else {
        match hostport.split_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), port),
            None => (hostport.to_ascii_lowercase(), ""),
        }
    };
    if port.is_empty() || !port.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let port: u32 = port.parse().ok()?;
    if !(1..=65_535).contains(&port) {
        return None;
    }
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return None;
    }
    Some(format!("http://{host}:{port}"))
}

fn split_scheme(raw: &str) -> Option<(String, &str)> {
    let (scheme, rest) = raw.split_once(':')?;
    let mut characters = scheme.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
    {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), rest))
}

fn split_once_or_empty(value: &str, separator: char) -> (&str, &str) {
    match value.split_once(separator) {
        Some((left, right)) => (left, right),
        None => (value, ""),
    }
}

/// The accepted client accepts literal loopback addresses, not `localhost`.
fn transport_url(resolved: &str) -> String {
    if let Some(rest) = resolved.strip_prefix("http://") {
        if let Some((host, port)) = rest.rsplit_once(':') {
            if host == "localhost" {
                return format!("http://127.0.0.1:{port}");
            }
        }
    }
    resolved.to_owned()
}
