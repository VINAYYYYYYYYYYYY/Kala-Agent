//! The provider model catalog behind `GET /api/studio/providers/{id}/models`.
//!
//! The route answers one envelope for every known provider:
//!
//! ```json
//! {"provider":"openrouter","source":"openrouter",
//!  "fetched_at":"2026-10-20T12:00:00Z",
//!  "models":[{"id":"minimax/minimax-m3.1-flash:free",
//!             "name":"MiniMax M3.1 Flash (free)",
//!             "context_length":262144,
//!             "pricing":{"prompt":0.0,"completion":0.0},
//!             "free":true,"supports_tools":true}]}
//! ```
//!
//! `pricing.prompt` and `pricing.completion` are the provider's own numbers
//! in its own unit (OpenRouter reports USD per token); the backend neither
//! converts nor invents them. A provider failure is reported inside the same
//! envelope: HTTP 200, `models: []`, `source: "none"` and `error` set. Only
//! an unknown provider (404) and a missing remote-egress consent (403) are
//! route-level errors.
//!
//! # Sources
//!
//! * `openrouter`: `GET https://openrouter.ai/api/v1/models` with the stored
//!   key in `Authorization`. `free` is true when both prices are zero or the
//!   id ends with `:free`.
//! * `openai`: `GET {base}/v1/models`, where the base is
//!   [`OPENAI_DEFAULT_BASE_URL`] or the validated `OPENAI_BASE_URL`
//!   environment value (an OpenAI-compatible endpoint).
//! * `local`: one static row for the deterministic helper.
//! * `claude_code` / `codex_cli`: a documented static list of the CLI's model
//!   aliases, `source: "static"`.
//! * `opencode_cli`: `opencode models` when the CLI is installed (bounded
//!   argv, no shell, sanitized environment), otherwise the static fallback
//!   with an honest `error`.
//!
//! # Limits
//!
//! The transport timeout is [`CATALOG_REQUEST_TIMEOUT_SECONDS`], the remote
//! body is capped at [`MAX_CATALOG_RESPONSE_BYTES`], the listing at
//! [`MAX_MODELS`] rows, and a CLI listing at [`MAX_CLI_OUTPUT_BYTES`] and
//! [`CLI_MODEL_LIST_TIMEOUT_SECONDS`]. A successful remote listing is cached
//! in memory for [`MODEL_CACHE_TTL_SECONDS`] per provider and key
//! fingerprint (a SHA-256 digest, never the key). The key is never written
//! to a log, a cache entry, an error or a response.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::json::Json;
use crate::providers::ProviderFailure;
use crate::providers::remote::{
    GetRequest, HttpTransport, RemoteResponse, TransportError, credential,
};
use crate::pytext;
use crate::settings::StoredKeys;

/// The lifetime of one successful cached model listing.
pub const MODEL_CACHE_TTL_SECONDS: u64 = 600;
/// The bounded timeout of one remote model listing.
pub const CATALOG_REQUEST_TIMEOUT_SECONDS: u64 = 15;
/// The largest remote model-listing body the transport reads.
pub const MAX_CATALOG_RESPONSE_BYTES: usize = 4_000_000;
/// The largest number of rows one listing carries.
pub const MAX_MODELS: usize = 500;
/// The largest number of cached listings one service keeps.
pub const MAX_CACHE_ENTRIES: usize = 32;
/// The largest model name kept in a row; the id is never truncated.
pub const MAX_MODEL_NAME_LENGTH: usize = 200;
/// The largest configured OpenAI-compatible base URL.
pub const MAX_BASE_URL_LENGTH: usize = 2_048;
/// The default OpenAI API root.
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com";
/// The environment variable that selects an OpenAI-compatible API root.
pub const OPENAI_BASE_URL_ENV: &str = "OPENAI_BASE_URL";
/// The fixed OpenRouter model-listing endpoint.
pub const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
/// The bounded timeout of one CLI model listing.
pub const CLI_MODEL_LIST_TIMEOUT_SECONDS: u64 = 10;
/// The largest CLI model-listing output read.
pub const MAX_CLI_OUTPUT_BYTES: usize = 1_000_000;

const REFERER: &str = "https://kala-agent.local/studio";

/// The documented Claude Code model aliases.
///
/// Claude Code accepts these aliases (and full model names) on its model
/// flag; a subscription can change which model an alias resolves to, so the
/// catalog lists the aliases rather than pinning vendor snapshots.
const CLAUDE_CODE_MODELS: &[(&str, &str)] = &[
    ("default", "Claude Code default"),
    ("sonnet", "Claude Sonnet (alias)"),
    ("opus", "Claude Opus (alias)"),
    ("haiku", "Claude Haiku (alias)"),
];

/// The documented Codex CLI model aliases.
///
/// Codex passes any model id through; these are the documented ids at the
/// time of writing and the CLI's own default.
const CODEX_CLI_MODELS: &[(&str, &str)] = &[
    ("default", "Codex CLI default"),
    ("gpt-5-codex", "GPT-5 Codex"),
    ("gpt-5", "GPT-5"),
    ("o3", "o3"),
    ("o4-mini", "o4-mini"),
];

/// The only environment names a subscription CLI receives.
///
/// Nothing of Kala's own configuration (no stored key, no studio token, no
/// data directory) is inherited.
const CLI_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
];

/// Where one listing came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogSource {
    OpenRouter,
    OpenAi,
    Cli,
    Static,
    None,
}

impl CatalogSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenAi => "openai",
            Self::Cli => "cli",
            Self::Static => "static",
            Self::None => "none",
        }
    }
}

/// The prices of one model in the provider's own currency and unit.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPricing {
    pub prompt: f64,
    pub completion: f64,
}

/// One model a provider can use.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub context_length: Option<u64>,
    pub pricing: Option<ModelPricing>,
    pub free: bool,
    pub supports_tools: Option<bool>,
}

impl ModelEntry {
    fn static_entry(id: &str, name: &str, free: bool) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            context_length: None,
            pricing: None,
            free,
            supports_tools: None,
        }
    }

    fn to_json(&self) -> Json {
        let mut entries: Vec<(&'static str, Json)> = vec![
            ("id", Json::String(self.id.clone())),
            ("name", Json::String(self.name.clone())),
        ];
        if let Some(context_length) = self.context_length {
            entries.push(("context_length", Json::UInt(context_length)));
        }
        if let Some(pricing) = &self.pricing {
            entries.push((
                "pricing",
                Json::object(vec![
                    ("prompt", Json::Float(pricing.prompt)),
                    ("completion", Json::Float(pricing.completion)),
                ]),
            ));
        }
        entries.push(("free", Json::Bool(self.free)));
        if let Some(supports_tools) = self.supports_tools {
            entries.push(("supports_tools", Json::Bool(supports_tools)));
        }
        Json::object(entries)
    }
}

/// One provider's model-catalog envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogListing {
    pub provider: String,
    pub source: CatalogSource,
    pub fetched_at: String,
    pub models: Vec<ModelEntry>,
    pub error: Option<String>,
}

impl CatalogListing {
    /// Render the exact wire shape of the route.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut entries = vec![
            ("provider", Json::String(self.provider.clone())),
            ("source", Json::String(self.source.as_str().to_owned())),
            ("fetched_at", Json::String(self.fetched_at.clone())),
            (
                "models",
                Json::Array(self.models.iter().map(ModelEntry::to_json).collect()),
            ),
        ];
        if let Some(error) = &self.error {
            entries.push(("error", Json::String(error.clone())));
        }
        Json::object(entries)
    }
}

/// The only failures that are not carried inside the envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogFailure {
    /// The provider id is not a catalog provider.
    UnknownProvider,
    /// A remote listing was requested without `allow_remote=true`.
    ConsentRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CacheKey {
    provider: &'static str,
    fingerprint: String,
}

#[derive(Clone, Debug)]
struct CachedListing {
    fetched_at_epoch: u64,
    listing: CatalogListing,
}

/// The in-memory model-listing cache.
///
/// One cache belongs to one Studio service. The clock is a seam so a test
/// can prove expiry without waiting ten minutes.
pub struct ModelCatalog {
    ttl_seconds: u64,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
    entries: Mutex<HashMap<CacheKey, CachedListing>>,
}

impl std::fmt::Debug for ModelCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        formatter
            .debug_struct("ModelCatalog")
            .field("ttl_seconds", &self.ttl_seconds)
            .field("entries", &entries.len())
            .finish()
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelCatalog {
    /// A cache with the product TTL and the wall clock.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(
            MODEL_CACHE_TTL_SECONDS,
            Arc::new(current_epoch_seconds as fn() -> u64),
        )
    }

    /// A cache with an explicit TTL and clock (the test seam).
    #[must_use]
    pub fn with_clock(ttl_seconds: u64, now: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        Self {
            ttl_seconds,
            now,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The configured lifetime of one cached listing.
    #[must_use]
    pub fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }

    /// Drop every cached listing.
    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn now_epoch(&self) -> u64 {
        (self.now)()
    }

    fn lookup(&self, key: &CacheKey) -> Option<CatalogListing> {
        let now = self.now_epoch();
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cached = entries.get(key)?;
        if now.saturating_sub(cached.fetched_at_epoch) < self.ttl_seconds {
            Some(cached.listing.clone())
        } else {
            None
        }
    }

    fn store(&self, key: CacheKey, fetched_at_epoch: u64, listing: &CatalogListing) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Expired rows are dropped on every insert, so a long-running service
        // that sees many keys cannot grow the cache without bound.
        entries.retain(|_, cached| {
            fetched_at_epoch.saturating_sub(cached.fetched_at_epoch) < self.ttl_seconds
        });
        if entries.len() >= MAX_CACHE_ENTRIES && !entries.contains_key(&key) {
            let oldest = entries
                .iter()
                .min_by_key(|(_, cached)| cached.fetched_at_epoch)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(
            key,
            CachedListing {
                fetched_at_epoch,
                listing: listing.clone(),
            },
        );
    }
}

/// Return the model catalog of one provider.
///
/// `allow_remote` is the operator's per-request remote-egress consent; the
/// API-key routes and the OpenCode CLI listing refuse without it. The local
/// helper and the static CLI lists open no connection and need no consent.
///
/// # Errors
///
/// Returns [`CatalogFailure::UnknownProvider`] for an id that is not a
/// catalog provider and [`CatalogFailure::ConsentRequired`] when a remote
/// listing is requested without consent. Provider, transport and parse
/// failures are reported inside the returned envelope.
pub async fn list_models(
    catalog: &ModelCatalog,
    provider_id: &str,
    allow_remote: bool,
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    transport: &dyn HttpTransport,
) -> Result<CatalogListing, CatalogFailure> {
    let provider = canonical_provider(provider_id).ok_or(CatalogFailure::UnknownProvider)?;
    match provider {
        "local" => Ok(static_listing(
            provider,
            catalog.now_epoch(),
            &[(
                crate::providers::local::DEFAULT_LOCAL_MODEL,
                "Local Deterministic Helper",
            )],
            true,
        )),
        "claude_code" => Ok(static_listing(
            provider,
            catalog.now_epoch(),
            CLAUDE_CODE_MODELS,
            false,
        )),
        "codex_cli" => Ok(static_listing(
            provider,
            catalog.now_epoch(),
            CODEX_CLI_MODELS,
            false,
        )),
        "opencode_cli" => {
            if !allow_remote {
                return Err(CatalogFailure::ConsentRequired);
            }
            Ok(opencode_listing(catalog, environ).await)
        }
        "openrouter" => {
            if !allow_remote {
                return Err(CatalogFailure::ConsentRequired);
            }
            Ok(openrouter_listing(catalog, environ, stored, transport).await)
        }
        "openai" => {
            if !allow_remote {
                return Err(CatalogFailure::ConsentRequired);
            }
            Ok(openai_listing(catalog, environ, stored, transport).await)
        }
        _ => Err(CatalogFailure::UnknownProvider),
    }
}

/// The canonical catalog provider id, or `None`.
///
/// `codex` is the historical alias of `codex_cli`.
#[must_use]
pub fn canonical_provider(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "local" => Some("local"),
        "openai" => Some("openai"),
        "openrouter" => Some("openrouter"),
        "claude_code" => Some("claude_code"),
        "codex_cli" | "codex" => Some("codex_cli"),
        "opencode_cli" => Some("opencode_cli"),
        _ => None,
    }
}

/// Whether a query string carries `name=true`.
///
/// The flag is deliberately exact: at least one member must be the literal
/// `true`. A missing, empty or differently cased value does not consent, and
/// a second `false` member cannot revoke an explicit `true`.
#[must_use]
pub fn query_flag_true(query: Option<&str>, name: &str) -> bool {
    let Some(query) = query else {
        return false;
    };
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        percent_decode(key) == name && percent_decode(value) == "true"
    })
}

/// Decode one query component like a browser form value.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (hex_value(bytes[index + 1]), hex_value(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        decoded.push((high << 4) | low);
                        index += 3;
                    }
                    _ => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Render an aware UTC timestamp (`%Y-%m-%dT%H:%M:%SZ`).
#[must_use]
pub fn format_timestamp(epoch_seconds: u64) -> String {
    let days = i64::try_from(epoch_seconds / 86_400).unwrap_or(i64::MAX);
    let seconds = epoch_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60
    )
}

/// Howard Hinnant's `civil_from_days`, for days since 1970-01-01.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    (
        year + i64::from(month <= 2),
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn static_listing(
    provider: &'static str,
    fetched_at_epoch: u64,
    rows: &[(&str, &str)],
    free: bool,
) -> CatalogListing {
    CatalogListing {
        provider: provider.to_owned(),
        source: CatalogSource::Static,
        fetched_at: format_timestamp(fetched_at_epoch),
        models: rows
            .iter()
            .map(|(id, name)| ModelEntry::static_entry(id, name, free))
            .collect(),
        error: None,
    }
}

async fn openrouter_listing(
    catalog: &ModelCatalog,
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    transport: &dyn HttpTransport,
) -> CatalogListing {
    let key = match credential("openrouter", environ, stored) {
        Ok(key) => key,
        Err(failure) => return error_listing(catalog, "openrouter", &failure),
    };
    let fingerprint = fingerprint("openrouter", &key, None);
    let key_hash = CacheKey {
        provider: "openrouter",
        fingerprint,
    };
    if let Some(hit) = catalog.lookup(&key_hash) {
        return hit;
    }
    let fetched_at = catalog.now_epoch();
    let request = GetRequest {
        url: OPENROUTER_MODELS_URL.to_owned(),
        headers: remote_headers(&key, true),
        timeout: Duration::from_secs(CATALOG_REQUEST_TIMEOUT_SECONDS),
        max_bytes: MAX_CATALOG_RESPONSE_BYTES,
    };
    let response = match transport.get(request).await {
        Ok(response) => response,
        Err(error) => {
            return error_listing(catalog, "openrouter", &transport_failure(error));
        }
    };
    let body = match checked_body(response) {
        Ok(body) => body,
        Err(failure) => return error_listing(catalog, "openrouter", &failure),
    };
    let models = match parse_model_list(&body, openrouter_entry) {
        Ok(models) => models,
        Err(failure) => return error_listing(catalog, "openrouter", &failure),
    };
    let listing = CatalogListing {
        provider: "openrouter".to_owned(),
        source: CatalogSource::OpenRouter,
        fetched_at: format_timestamp(fetched_at),
        models,
        error: None,
    };
    catalog.store(key_hash, fetched_at, &listing);
    listing
}

async fn openai_listing(
    catalog: &ModelCatalog,
    environ: &BTreeMap<String, String>,
    stored: &StoredKeys,
    transport: &dyn HttpTransport,
) -> CatalogListing {
    let key = match credential("openai", environ, stored) {
        Ok(key) => key,
        Err(failure) => return error_listing(catalog, "openai", &failure),
    };
    let base = match openai_base_url(environ) {
        Ok(base) => base,
        Err(failure) => return error_listing(catalog, "openai", &failure),
    };
    let fingerprint = fingerprint("openai", &key, Some(&base));
    let key_hash = CacheKey {
        provider: "openai",
        fingerprint,
    };
    if let Some(hit) = catalog.lookup(&key_hash) {
        return hit;
    }
    let fetched_at = catalog.now_epoch();
    let request = GetRequest {
        url: format!("{base}/v1/models"),
        headers: remote_headers(&key, false),
        timeout: Duration::from_secs(CATALOG_REQUEST_TIMEOUT_SECONDS),
        max_bytes: MAX_CATALOG_RESPONSE_BYTES,
    };
    let response = match transport.get(request).await {
        Ok(response) => response,
        Err(error) => return error_listing(catalog, "openai", &transport_failure(error)),
    };
    let body = match checked_body(response) {
        Ok(body) => body,
        Err(failure) => return error_listing(catalog, "openai", &failure),
    };
    let models = match parse_model_list(&body, openai_entry) {
        Ok(models) => models,
        Err(failure) => return error_listing(catalog, "openai", &failure),
    };
    let listing = CatalogListing {
        provider: "openai".to_owned(),
        source: CatalogSource::OpenAi,
        fetched_at: format_timestamp(fetched_at),
        models,
        error: None,
    };
    catalog.store(key_hash, fetched_at, &listing);
    listing
}

async fn opencode_listing(
    catalog: &ModelCatalog,
    environ: &BTreeMap<String, String>,
) -> CatalogListing {
    let key_hash = CacheKey {
        provider: "opencode_cli",
        fingerprint: "opencode-models".to_owned(),
    };
    if let Some(hit) = catalog.lookup(&key_hash) {
        return hit;
    }
    let fetched_at = catalog.now_epoch();
    match run_opencode_models(environ).await {
        Ok(ids) if !ids.is_empty() => {
            let listing = CatalogListing {
                provider: "opencode_cli".to_owned(),
                source: CatalogSource::Cli,
                fetched_at: format_timestamp(fetched_at),
                models: ids
                    .iter()
                    .map(|id| ModelEntry::static_entry(id, id, id.ends_with(":free")))
                    .collect(),
                error: None,
            };
            catalog.store(key_hash, fetched_at, &listing);
            listing
        }
        Ok(_) => CatalogListing {
            provider: "opencode_cli".to_owned(),
            source: CatalogSource::Static,
            fetched_at: format_timestamp(fetched_at),
            models: Vec::new(),
            error: Some(
                "the opencode CLI reported no models; pass a provider/model id explicitly"
                    .to_owned(),
            ),
        },
        Err(reason) => CatalogListing {
            provider: "opencode_cli".to_owned(),
            source: CatalogSource::Static,
            fetched_at: format_timestamp(fetched_at),
            models: Vec::new(),
            error: Some(format!(
                "the opencode CLI model list is unavailable: {reason}"
            )),
        },
    }
}

fn remote_headers(key: &str, openrouter: bool) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Authorization".to_owned(), format!("Bearer {key}")),
        ("Accept".to_owned(), "application/json".to_owned()),
        ("User-Agent".to_owned(), "kala-studio/0.1".to_owned()),
    ];
    if openrouter {
        headers.push(("HTTP-Referer".to_owned(), REFERER.to_owned()));
        headers.push(("X-Title".to_owned(), "kala-studio".to_owned()));
    }
    headers
}

/// A cache fingerprint: a SHA-256 digest of the provider, the key and the
/// optional base URL. The key itself is never stored.
fn fingerprint(provider: &str, key: &str, base: Option<&str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update([0]);
    hasher.update(key.as_bytes());
    hasher.update([0]);
    if let Some(base) = base {
        hasher.update(base.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// The validated OpenAI-compatible API root.
///
/// The default is the public OpenAI API. `OPENAI_BASE_URL` may select an
/// OpenAI-compatible server; it must be HTTPS, except for a loopback host
/// (a local model server), and may not carry credentials, a query or a
/// fragment. A trailing `/v1` is accepted and normalised away because the
/// route appends `/v1/models` itself.
fn openai_base_url(environ: &BTreeMap<String, String>) -> Result<String, ProviderFailure> {
    let raw = environ
        .get(OPENAI_BASE_URL_ENV)
        .map(|value| pytext::trim(value))
        .unwrap_or("");
    if raw.is_empty() {
        return Ok(OPENAI_DEFAULT_BASE_URL.to_owned());
    }
    let invalid = |message: &str| ProviderFailure::Configuration {
        code: "invalid_base_url",
        message: message.to_owned(),
    };
    if raw.chars().count() > MAX_BASE_URL_LENGTH {
        return Err(invalid("OPENAI_BASE_URL is longer than the bounded length"));
    }
    let parsed = reqwest::Url::parse(raw)
        .map_err(|_| invalid("OPENAI_BASE_URL is not a valid absolute URL"))?;
    let loopback_http = parsed.scheme() == "http" && is_loopback_host(parsed.host_str());
    if parsed.scheme() != "https" && !loopback_http {
        return Err(invalid(
            "OPENAI_BASE_URL must use https (http is allowed for a loopback host only)",
        ));
    }
    if parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid(
            "OPENAI_BASE_URL must not carry credentials, a query or a fragment",
        ));
    }
    let mut base = raw.trim_end_matches('/').to_owned();
    if base.ends_with("/v1") {
        base.truncate(base.len() - 3);
    }
    Ok(base)
}

fn is_loopback_host(host: Option<&str>) -> bool {
    match host {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

fn transport_failure(error: TransportError) -> ProviderFailure {
    match error {
        TransportError::Timeout => ProviderFailure::Timeout(
            "the provider model listing exceeded its bounded timeout".to_owned(),
        ),
        TransportError::Network(class) => {
            ProviderFailure::Network(format!("the provider network call failed: {class}"))
        }
        TransportError::TooLarge(message) | TransportError::Internal(message) => {
            ProviderFailure::Internal(message)
        }
    }
}

fn checked_body(response: RemoteResponse) -> Result<Vec<u8>, ProviderFailure> {
    if (300..400).contains(&response.status) {
        return Err(ProviderFailure::Network(
            "redirects to a different origin are refused".to_owned(),
        ));
    }
    if !(200..300).contains(&response.status) {
        // The reason phrase is server-controlled and could reflect anything,
        // including the credential the provider just received. The status
        // and our own explanation are returned; the phrase never is.
        let message = if response.status == 401 || response.status == 403 {
            format!(
                "the provider rejected the stored credential (HTTP {})",
                response.status
            )
        } else if response.status == 429 {
            format!(
                "the provider rate-limited the model listing (HTTP {})",
                response.status
            )
        } else {
            format!("the provider returned HTTP {}", response.status)
        };
        return Err(ProviderFailure::Protocol(message));
    }
    if response
        .content_length
        .is_some_and(|value| value > MAX_CATALOG_RESPONSE_BYTES as u64)
        || response.body.len() > MAX_CATALOG_RESPONSE_BYTES
    {
        return Err(ProviderFailure::Internal(
            "the provider response exceeded the bounded size".to_owned(),
        ));
    }
    Ok(response.body)
}

fn parse_model_list(
    body: &[u8],
    mapper: fn(&Value) -> Option<ModelEntry>,
) -> Result<Vec<ModelEntry>, ProviderFailure> {
    let payload: Value = serde_json::from_slice(body).map_err(|_| {
        ProviderFailure::Protocol("the provider response was not valid JSON".to_owned())
    })?;
    let rows = match &payload {
        Value::Object(object) => match object.get("data") {
            Some(Value::Array(rows)) => rows.as_slice(),
            _ => {
                return Err(ProviderFailure::Protocol(
                    "the provider response did not declare a model list".to_owned(),
                ));
            }
        },
        Value::Array(rows) => rows.as_slice(),
        _ => {
            return Err(ProviderFailure::Protocol(
                "the provider response was not a JSON object".to_owned(),
            ));
        }
    };
    let mut models = Vec::new();
    for row in rows {
        if let Some(entry) = mapper(row) {
            models.push(entry);
            if models.len() >= MAX_MODELS {
                break;
            }
        }
    }
    Ok(models)
}

fn openrouter_entry(value: &Value) -> Option<ModelEntry> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?;
    if !valid_model_id(id) {
        return None;
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(bounded_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned());
    let context_length = object
        .get("context_length")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0);
    let pricing = object.get("pricing").and_then(openrouter_pricing);
    let free = pricing
        .as_ref()
        .is_some_and(|pricing| pricing.prompt == 0.0 && pricing.completion == 0.0)
        || id.ends_with(":free");
    let supports_tools = object.get("supported_parameters").map(|parameters| {
        parameters
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some("tools")))
    });
    Some(ModelEntry {
        id: id.to_owned(),
        name,
        context_length,
        pricing,
        free,
        supports_tools,
    })
}

fn openai_entry(value: &Value) -> Option<ModelEntry> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?;
    if !valid_model_id(id) {
        return None;
    }
    let name = object
        .get("name")
        .or_else(|| object.get("display_name"))
        .and_then(Value::as_str)
        .map(bounded_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_owned());
    Some(ModelEntry {
        id: id.to_owned(),
        name,
        context_length: object
            .get("context_length")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0),
        pricing: None,
        free: id.ends_with(":free"),
        supports_tools: None,
    })
}

fn openrouter_pricing(value: &Value) -> Option<ModelPricing> {
    let object = value.as_object()?;
    Some(ModelPricing {
        prompt: price_number(object.get("prompt")?)?,
        completion: price_number(object.get("completion")?)?,
    })
}

fn price_number(value: &Value) -> Option<f64> {
    let parsed = match value {
        Value::String(text) => text.parse::<f64>().ok()?,
        Value::Number(number) => number.as_f64()?,
        _ => return None,
    };
    if parsed.is_finite() && parsed >= 0.0 {
        Some(parsed)
    } else {
        None
    }
}

fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().count() <= crate::schemas::MAX_MODEL_ID_LENGTH
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '/' | '-')
        })
}

fn bounded_name(name: &str) -> String {
    // A provider may echo arbitrary text; drop control characters so a name
    // cannot smuggle terminal escapes into an operator's terminal or a log.
    let cleaned: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_MODEL_NAME_LENGTH)
        .collect();
    cleaned.trim().to_owned()
}

fn error_listing(
    catalog: &ModelCatalog,
    provider: &str,
    failure: &ProviderFailure,
) -> CatalogListing {
    let message = match failure {
        ProviderFailure::Internal(_) => "the provider model list could not be fetched".to_owned(),
        ProviderFailure::Unavailable(message)
        | ProviderFailure::Timeout(message)
        | ProviderFailure::Network(message)
        | ProviderFailure::Protocol(message)
        | ProviderFailure::Subprocess(message) => message.clone(),
        ProviderFailure::Configuration { message, .. } => message.clone(),
    };
    CatalogListing {
        provider: provider.to_owned(),
        source: CatalogSource::None,
        fetched_at: format_timestamp(catalog.now_epoch()),
        models: Vec::new(),
        error: Some(message),
    }
}

/// Discover `opencode` on the configured PATH.
fn discover_opencode(environ: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = environ.get("PATH")?;
    for directory in std::env::split_paths(path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in ["opencode", "opencode.exe"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Run `opencode models --standalone` with a sanitized environment and
/// bounded output.
async fn run_opencode_models(environ: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let Some(program) = discover_opencode(environ) else {
        return Err("the opencode CLI is not installed on PATH".to_owned());
    };
    let work = tempfile::tempdir()
        .map_err(|_| "a private working directory could not be created".to_owned())?;
    let mut command = tokio::process::Command::new(program);
    command
        .arg("models")
        .arg("--standalone")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(work.path())
        .kill_on_drop(true);
    command.env_clear();
    for name in CLI_ENV_ALLOWLIST {
        if let Some(value) = environ.get(*name) {
            command.env(name, value);
        }
    }
    // Keep a non-TTY listing free of ANSI escapes; `opencode models` honors
    // the standard variable.
    command.env("NO_COLOR", "1");
    let mut child = command
        .spawn()
        .map_err(|_| "the opencode CLI could not be started".to_owned())?;
    let Some(stdout) = child.stdout.take() else {
        return Err("the opencode CLI output could not be read".to_owned());
    };
    let read = async move {
        let mut limited = stdout.take((MAX_CLI_OUTPUT_BYTES + 1) as u64);
        let mut buffer = Vec::new();
        limited
            .read_to_end(&mut buffer)
            .await
            .map_err(|_| "the opencode CLI output could not be read".to_owned())?;
        Ok::<Vec<u8>, String>(buffer)
    };
    let buffer =
        match tokio::time::timeout(Duration::from_secs(CLI_MODEL_LIST_TIMEOUT_SECONDS), read).await
        {
            Ok(Ok(buffer)) => buffer,
            Ok(Err(message)) => {
                let _ = child.kill().await;
                return Err(message);
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err("the opencode CLI exceeded its bounded timeout".to_owned());
            }
        };
    if buffer.len() > MAX_CLI_OUTPUT_BYTES {
        let _ = child.kill().await;
        return Err("the opencode CLI output exceeded the bounded size".to_owned());
    }
    let status = match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => status,
        _ => {
            let _ = child.kill().await;
            return Err("the opencode CLI did not exit".to_owned());
        }
    };
    if !status.success() {
        return Err(format!(
            "the opencode CLI exited with status {}",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(parse_opencode_models(&String::from_utf8_lossy(&buffer)))
}

/// Parse `opencode models` output: one `provider/model` token per line.
fn parse_opencode_models(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        let token = first.trim_matches(|character: char| {
            matches!(character, '`' | '*' | ',' | '|' | '"' | '\'')
        });
        if token.eq_ignore_ascii_case("id")
            || token.eq_ignore_ascii_case("model")
            || token.eq_ignore_ascii_case("models")
        {
            continue;
        }
        if !valid_model_id(token) {
            continue;
        }
        if seen.insert(token.to_owned()) {
            models.push(token.to_owned());
            if models.len() >= MAX_MODELS {
                break;
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU64, Ordering};

    type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[derive(Default)]
    struct MockTransport {
        requests: Mutex<Vec<GetRequest>>,
        responses: Mutex<VecDeque<Result<RemoteResponse, TransportError>>>,
    }

    impl MockTransport {
        fn push(&self, response: Result<RemoteResponse, TransportError>) {
            self.responses.lock().unwrap().push_back(response);
        }

        fn push_json(&self, body: &[u8]) {
            self.push(Ok(RemoteResponse {
                status: 200,
                reason: Some("OK".to_owned()),
                content_length: Some(body.len() as u64),
                body: body.to_vec(),
            }));
        }

        fn requests(&self) -> Vec<GetRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl HttpTransport for MockTransport {
        fn post<'a>(
            &'a self,
            _request: crate::providers::remote::RemoteRequest,
        ) -> BoxFuture<'a, Result<RemoteResponse, TransportError>> {
            Box::pin(async {
                Err(TransportError::Network(
                    "the catalog must not POST".to_owned(),
                ))
            })
        }

        fn get<'a>(
            &'a self,
            request: GetRequest,
        ) -> BoxFuture<'a, Result<RemoteResponse, TransportError>> {
            self.requests.lock().unwrap().push(request);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(TransportError::Network("no mock response".to_owned())));
            Box::pin(async move { response })
        }
    }

    fn environment() -> BTreeMap<String, String> {
        let mut environ = BTreeMap::new();
        environ.insert(
            "OPENROUTER_API_KEY".to_owned(),
            "sk-or-synthetic-catalog-key-0001".to_owned(),
        );
        environ.insert(
            "OPENAI_API_KEY".to_owned(),
            "sk-synthetic-catalog-key-0002".to_owned(),
        );
        environ
    }

    fn catalog() -> ModelCatalog {
        ModelCatalog::new()
    }

    const OPENROUTER_BODY: &[u8] = br#"{
        "data": [
            {"id":"minimax/minimax-m3.1-flash:free","name":"MiniMax M3.1 Flash (free)",
             "context_length":262144,"pricing":{"prompt":"0","completion":"0"},
             "supported_parameters":["tools","temperature"]},
            {"id":"anthropic/claude-sonnet-4.5","name":"Claude Sonnet 4.5",
             "context_length":200000,"pricing":{"prompt":"0.000003","completion":"0.000015"},
             "supported_parameters":["max_tokens"]},
            {"id":"odd/model","name":"","pricing":{"prompt":"-1","completion":"-1"}},
            {"id":"bad id","name":"not addressable"},
            {"id":"also/bad?","name":"not addressable"}
        ]
    }"#;

    #[tokio::test]
    async fn openrouter_mapping_and_free_detection() {
        let mock = MockTransport::default();
        mock.push_json(OPENROUTER_BODY);
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.provider, "openrouter");
        assert_eq!(listing.source, CatalogSource::OpenRouter);
        assert_eq!(listing.error, None);
        assert_eq!(listing.models.len(), 3);

        let free = &listing.models[0];
        assert_eq!(free.id, "minimax/minimax-m3.1-flash:free");
        assert_eq!(free.context_length, Some(262_144));
        assert_eq!(
            free.pricing,
            Some(ModelPricing {
                prompt: 0.0,
                completion: 0.0
            })
        );
        assert!(free.free);
        assert_eq!(free.supports_tools, Some(true));

        let paid = &listing.models[1];
        assert!(!paid.free);
        assert_eq!(
            paid.pricing,
            Some(ModelPricing {
                prompt: 0.000003,
                completion: 0.000015
            })
        );
        assert_eq!(paid.supports_tools, Some(false));

        // A negative ("variable") price is not reported as a number, the
        // name falls back to the id, and an unusable id is dropped.
        let odd = &listing.models[2];
        assert_eq!(odd.id, "odd/model");
        assert_eq!(odd.name, "odd/model");
        assert_eq!(odd.pricing, None);
        assert!(!odd.free);

        let requests = mock.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url, OPENROUTER_MODELS_URL);
        assert_eq!(requests[0].timeout, Duration::from_secs(15));
        assert_eq!(requests[0].max_bytes, MAX_CATALOG_RESPONSE_BYTES);
        assert_eq!(
            requests[0].headers[0],
            (
                "Authorization".to_owned(),
                "Bearer sk-or-synthetic-catalog-key-0001".to_owned()
            )
        );
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(name, value)| name == "HTTP-Referer" && value == REFERER)
        );
        let rendered = String::from_utf8(listing.to_json().to_bytes().unwrap()).unwrap();
        assert!(!rendered.contains("sk-or-synthetic-catalog-key-0001"));
        assert!(rendered.contains("\"free\":true"));
        assert!(rendered.contains("\"source\":\"openrouter\""));
    }

    #[tokio::test]
    async fn openai_compatible_base_url_and_mapping() {
        let mut environ = environment();
        environ.insert(
            "OPENAI_BASE_URL".to_owned(),
            "http://127.0.0.1:8080/llm/v1/".to_owned(),
        );
        let mock = MockTransport::default();
        mock.push_json(
            br#"{"object":"list","data":[
                {"id":"gpt-4o-mini","object":"model","owned_by":"openai"},
                {"id":"local/model:free","name":"Local Free"}
            ]}"#,
        );
        let listing = list_models(
            &catalog(),
            "openai",
            true,
            &environ,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.source, CatalogSource::OpenAi);
        assert_eq!(listing.models.len(), 2);
        assert_eq!(listing.models[0].id, "gpt-4o-mini");
        assert_eq!(listing.models[0].name, "gpt-4o-mini");
        assert!(!listing.models[0].free);
        assert!(listing.models[1].free);
        assert_eq!(
            mock.requests()[0].url,
            "http://127.0.0.1:8080/llm/v1/models"
        );
        assert!(
            !mock.requests()[0]
                .headers
                .iter()
                .any(|(name, _)| name == "HTTP-Referer")
        );
    }

    #[test]
    fn base_url_rules_are_https_or_loopback() {
        let mut environ = BTreeMap::new();
        assert_eq!(openai_base_url(&environ).unwrap(), OPENAI_DEFAULT_BASE_URL);
        environ.insert(
            "OPENAI_BASE_URL".to_owned(),
            "https://llm.example/v1".to_owned(),
        );
        assert_eq!(openai_base_url(&environ).unwrap(), "https://llm.example");
        environ.insert(
            "OPENAI_BASE_URL".to_owned(),
            "http://localhost:11434/v1".to_owned(),
        );
        assert_eq!(openai_base_url(&environ).unwrap(), "http://localhost:11434");
        environ.insert(
            "OPENAI_BASE_URL".to_owned(),
            "http://[::1]:11434".to_owned(),
        );
        assert_eq!(openai_base_url(&environ).unwrap(), "http://[::1]:11434");
        for rejected in [
            "http://llm.example",
            "ftp://llm.example",
            "https://user:secret@llm.example",
            "https://llm.example/?key=1",
            "https://llm.example/#frag",
            "not a url",
        ] {
            environ.insert("OPENAI_BASE_URL".to_owned(), rejected.to_owned());
            assert!(
                openai_base_url(&environ).is_err(),
                "{rejected} must be refused"
            );
        }
    }

    #[tokio::test]
    async fn cache_is_per_provider_and_key_fingerprint() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let ticking = Arc::clone(&clock);
        let cache = ModelCatalog::with_clock(
            MODEL_CACHE_TTL_SECONDS,
            Arc::new(move || ticking.load(Ordering::SeqCst)),
        );
        let mock = MockTransport::default();
        mock.push_json(OPENROUTER_BODY);
        mock.push_json(OPENROUTER_BODY);
        mock.push_json(OPENROUTER_BODY);

        let first = list_models(
            &cache,
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        // A hit returns the original timestamp and opens no connection.
        clock.store(1_500, Ordering::SeqCst);
        let second = list_models(
            &cache,
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(mock.requests().len(), 1);

        // A different key is a different cache entry.
        let mut other = environment();
        other.insert(
            "OPENROUTER_API_KEY".to_owned(),
            "sk-or-synthetic-catalog-key-9999".to_owned(),
        );
        list_models(
            &cache,
            "openrouter",
            true,
            &other,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(mock.requests().len(), 2);

        // The TTL is exclusive: one second before it lapses is a hit, at it
        // is a fresh fetch.
        clock.store(1_500 + MODEL_CACHE_TTL_SECONDS - 1, Ordering::SeqCst);
        list_models(
            &cache,
            "openrouter",
            true,
            &other,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(mock.requests().len(), 2);
        clock.store(1_500 + MODEL_CACHE_TTL_SECONDS, Ordering::SeqCst);
        list_models(
            &cache,
            "openrouter",
            true,
            &other,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(mock.requests().len(), 3);
    }

    #[test]
    fn cache_rows_are_pruned_and_bounded() {
        let clock = Arc::new(AtomicU64::new(1_000));
        let ticking = Arc::clone(&clock);
        let cache = ModelCatalog::with_clock(
            MODEL_CACHE_TTL_SECONDS,
            Arc::new(move || ticking.load(Ordering::SeqCst)),
        );
        let listing = CatalogListing {
            provider: "openai".to_owned(),
            source: CatalogSource::OpenAi,
            fetched_at: format_timestamp(1_000),
            models: Vec::new(),
            error: None,
        };
        let key = |index: usize| CacheKey {
            provider: "openai",
            fingerprint: format!("fingerprint-{index}"),
        };
        for index in 0..(MAX_CACHE_ENTRIES + 5) {
            cache.store(key(index), 1_000 + index as u64, &listing);
        }
        {
            let entries = cache.entries.lock().unwrap();
            assert_eq!(entries.len(), MAX_CACHE_ENTRIES);
            assert!(!entries.contains_key(&key(0)));
            assert!(entries.contains_key(&key(MAX_CACHE_ENTRIES + 4)));
        }
        // Once the TTL has passed, the next insert drops every stale row.
        let later = 3_000 + MODEL_CACHE_TTL_SECONDS;
        clock.store(later, Ordering::SeqCst);
        cache.store(key(99), later, &listing);
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remote_failures_are_reported_in_the_envelope() {
        let mock = MockTransport::default();
        mock.push(Ok(RemoteResponse {
            status: 401,
            reason: Some("Unauthorized".to_owned()),
            content_length: Some(2),
            body: b"{}".to_vec(),
        }));
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.source, CatalogSource::None);
        assert!(listing.models.is_empty());
        let error = listing.error.unwrap();
        assert!(error.contains("rejected the stored credential"), "{error}");
        assert!(!error.contains("sk-or-synthetic"));

        mock.push(Err(TransportError::Timeout));
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert!(listing.error.unwrap().contains("bounded timeout"));

        mock.push_json(b"{not json");
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert!(listing.error.unwrap().contains("not valid JSON"));

        mock.push_json(br#"{"object":"list"}"#);
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert!(
            listing
                .error
                .unwrap()
                .contains("did not declare a model list")
        );

        // The internal transport failure is reported generically.
        mock.push(Err(TransportError::TooLarge("secret internals".to_owned())));
        let listing = list_models(
            &catalog(),
            "openrouter",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(
            listing.error.as_deref(),
            Some("the provider model list could not be fetched")
        );
    }

    #[tokio::test]
    async fn consent_is_required_before_a_remote_call() {
        let mock = MockTransport::default();
        assert_eq!(
            list_models(
                &catalog(),
                "openrouter",
                false,
                &environment(),
                &StoredKeys::new(),
                &mock
            )
            .await
            .unwrap_err(),
            CatalogFailure::ConsentRequired
        );
        assert_eq!(
            list_models(
                &catalog(),
                "openai",
                false,
                &environment(),
                &StoredKeys::new(),
                &mock
            )
            .await
            .unwrap_err(),
            CatalogFailure::ConsentRequired
        );
        assert_eq!(
            list_models(
                &catalog(),
                "opencode_cli",
                false,
                &environment(),
                &StoredKeys::new(),
                &mock
            )
            .await
            .unwrap_err(),
            CatalogFailure::ConsentRequired
        );
        assert!(mock.requests().is_empty());

        // The local helper and the static CLI lists egress nothing.
        let local = list_models(
            &catalog(),
            "local",
            false,
            &environ_empty(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(local.models.len(), 1);
        assert_eq!(
            local.models[0].id,
            crate::providers::local::DEFAULT_LOCAL_MODEL
        );
        assert!(local.models[0].free);
        let claude = list_models(
            &catalog(),
            "claude_code",
            false,
            &environ_empty(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(claude.source, CatalogSource::Static);
        assert!(claude.models.iter().any(|model| model.id == "sonnet"));
        let codex = list_models(
            &catalog(),
            "codex",
            false,
            &environ_empty(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(codex.provider, "codex_cli");
        assert!(codex.models.iter().any(|model| model.id == "gpt-5-codex"));
    }

    fn environ_empty() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[tokio::test]
    async fn unknown_providers_are_refused() {
        let mock = MockTransport::default();
        for provider in ["freecad", "cad", "nope", ""] {
            assert_eq!(
                list_models(
                    &catalog(),
                    provider,
                    true,
                    &environment(),
                    &StoredKeys::new(),
                    &mock
                )
                .await
                .unwrap_err(),
                CatalogFailure::UnknownProvider,
                "provider {provider}"
            );
        }
        assert!(mock.requests().is_empty());
    }

    #[tokio::test]
    async fn listing_is_bounded() {
        let mut rows = Vec::new();
        for index in 0..(MAX_MODELS + 20) {
            rows.push(format!(
                r#"{{"id":"provider/model-{index}","name":"{}"}}"#,
                "n".repeat(500)
            ));
        }
        let body = format!(r#"{{"data":[{}]}}"#, rows.join(","));
        let mock = MockTransport::default();
        mock.push_json(body.as_bytes());
        let listing = list_models(
            &catalog(),
            "openai",
            true,
            &environment(),
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.models.len(), MAX_MODELS);
        assert_eq!(
            listing.models[0].name.chars().count(),
            MAX_MODEL_NAME_LENGTH
        );
    }

    #[test]
    fn query_consent_parsing_is_exact() {
        assert!(query_flag_true(Some("allow_remote=true"), "allow_remote"));
        assert!(query_flag_true(
            Some("x=1&allow_remote=true&y=2"),
            "allow_remote"
        ));
        assert!(query_flag_true(Some("allow_remote=%74rue"), "allow_remote"));
        assert!(!query_flag_true(Some("allow_remote=false"), "allow_remote"));
        assert!(!query_flag_true(Some("allow_remote=TRUE"), "allow_remote"));
        assert!(!query_flag_true(Some("allow_remote=1"), "allow_remote"));
        assert!(!query_flag_true(Some("allow_remote"), "allow_remote"));
        assert!(!query_flag_true(Some("other=true"), "allow_remote"));
        assert!(!query_flag_true(None, "allow_remote"));
    }

    #[test]
    fn timestamps_are_rfc3339_utc() {
        assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_timestamp(1_772_000_000), "2026-02-25T06:13:20Z");
    }

    #[cfg(unix)]
    fn write_fake_opencode(directory: &std::path::Path, script: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("opencode");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn opencode_cli_listing_is_dynamic_when_available() {
        let fake = tempfile::tempdir().unwrap();
        write_fake_opencode(
            fake.path(),
            "#!/bin/sh\nif [ -n \"$KALA_STUDIO_TOKEN\" ]; then printf 'leaked/secret\\n'; else printf 'openai/gpt-4o\\nanthropic/claude-sonnet-4.5\\nopenai/gpt-4o\\n'; fi\n",
        );
        let work = tempfile::tempdir().unwrap();
        let mut environ = BTreeMap::new();
        environ.insert(
            "PATH".to_owned(),
            fake.path().to_string_lossy().into_owned(),
        );
        environ.insert(
            "HOME".to_owned(),
            work.path().to_string_lossy().into_owned(),
        );
        environ.insert(
            "KALA_STUDIO_TOKEN".to_owned(),
            "must-not-be-passed".to_owned(),
        );
        let mock = MockTransport::default();
        let listing = list_models(
            &catalog(),
            "opencode_cli",
            true,
            &environ,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.source, CatalogSource::Cli);
        assert_eq!(
            listing
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["openai/gpt-4o", "anthropic/claude-sonnet-4.5"]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn opencode_cli_falls_back_to_an_honest_static_error() {
        let empty = tempfile::tempdir().unwrap();
        let mut environ = BTreeMap::new();
        environ.insert(
            "PATH".to_owned(),
            empty.path().to_string_lossy().into_owned(),
        );
        let mock = MockTransport::default();
        let listing = list_models(
            &catalog(),
            "opencode_cli",
            true,
            &environ,
            &StoredKeys::new(),
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(listing.source, CatalogSource::Static);
        assert!(listing.models.is_empty());
        assert!(listing.error.unwrap().contains("not installed on PATH"));
    }
}
