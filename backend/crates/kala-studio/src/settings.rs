//! Portal-compatible Studio settings and credential store.
//!
//! The store is `studio-settings.json` inside the Studio data directory. The
//! on-disk format, provider order, atomic replacement and `0o600` mode match
//! the Python implementation byte for byte. Stored keys are overlaid on the
//! environment at request time and are never echoed in a response.
//!
//! Values use [`kala_json::JsonText`] code points rather than Rust strings:
//! Python's `json.loads` accepts an escaped lone surrogate (`"\ud800"`) in a
//! stored key, and `json.dumps(..., ensure_ascii=True)` writes it back
//! unchanged. A Rust `String` cannot hold it, so the store preserves the
//! exact code points and only converts to UTF-8 for the paths that can carry
//! one (a missing conversion means the source would raise `UnicodeEncodeError`
//! while sending a header, which the route reports as a sanitized 500).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use kala_json::{CanonicalValue, JsonText, Limits};

use crate::boundary::TOKEN_FILE_MODE;
use crate::pytext;
use crate::settings_json::SettingsDocument;

pub const STUDIO_SETTINGS_FILE_NAME: &str = "studio-settings.json";
pub const SUPPORTED_CREDENTIAL_PROVIDERS: &[&str] = &["openai", "openrouter"];
pub const PROVIDER_ENV_VARS: &[(&str, &str)] = &[
    ("openai", "OPENAI_API_KEY"),
    ("openrouter", "OPENROUTER_API_KEY"),
];
pub const MAX_API_KEY_LENGTH: usize = 256;

/// Validated stored provider keys with Python code-point fidelity.
///
/// Only `openai` and `openrouter` can appear, and every value has already
/// passed [`validate_api_key_text`].
pub type StoredKeys = BTreeMap<&'static str, JsonText>;

/// Parse limits for the reviewed fallback reader.
///
/// `json.loads` on a settings file has no byte, depth, string or collection
/// cap; only CPython's 4,300-digit integer conversion limit applies. The
/// reviewed parsers therefore run unbounded except for that integer limit.
/// They are only a safety net for the CPython-compatible projection in
/// [`crate::settings_json`], which is the primary reader.
const SETTINGS_PARSE_LIMITS: Limits = Limits {
    max_json_bytes: usize::MAX,
    max_string_length: usize::MAX,
    max_collection_items: usize::MAX,
    max_key_length: usize::MAX,
    max_json_depth: usize::MAX,
    max_integer_digits: 4_300,
};

/// A credential row for `GET /api/studio/settings`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialStatus {
    pub provider: &'static str,
    pub env_var: &'static str,
    pub configured: bool,
    pub source: CredentialSource,
}

/// Where the active key comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    None,
    Environment,
    Stored,
    /// The operator's own subscription CLI login.
    ///
    /// The CLI holds the credential in its own store; Kala never sees it,
    /// never stores it and never forwards it.
    Subscription,
}

impl CredentialSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Environment => "environment",
            Self::Stored => "stored",
            Self::Subscription => "subscription",
        }
    }
}

/// Python `str.isspace()` for one code point.
fn cp_is_space(point: u32) -> bool {
    char::from_u32(point).is_some_and(pytext::is_space)
}

/// Validate one stored credential with Python code-point semantics.
///
/// The submitted value is never included in an error message.
///
/// # Errors
///
/// Returns the Python `ValueError` text for an over-long key, a control
/// character or embedded whitespace.
pub fn validate_api_key_text(value: &JsonText) -> Result<Option<JsonText>, String> {
    let points = value.code_points();
    let start = points
        .iter()
        .position(|point| !cp_is_space(*point))
        .unwrap_or(points.len());
    let end = points
        .iter()
        .rposition(|point| !cp_is_space(*point))
        .map_or(start, |index| index + 1);
    let trimmed = &points[start..end];
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_API_KEY_LENGTH {
        return Err("API key exceeds maximum length".to_owned());
    }
    if trimmed.iter().any(|point| *point < 32 || *point == 127) {
        return Err("API key contains unsupported control characters".to_owned());
    }
    if trimmed.iter().any(|point| cp_is_space(*point)) {
        return Err("API key must not contain whitespace".to_owned());
    }
    match JsonText::from_code_points(trimmed.to_vec()) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Err("API key is not valid Unicode".to_owned()),
    }
}

/// Validate and normalise an API key submitted over HTTP; the empty string
/// clears it.
///
/// # Errors
///
/// Returns the Python `ValueError` text for an over-long key, a control
/// character or embedded whitespace.
pub fn validate_api_key(value: Option<&str>) -> Result<String, String> {
    let Some(value) = value else {
        return Ok(String::new());
    };
    let text = JsonText::new(value);
    match validate_api_key_text(&text)? {
        Some(cleaned) => cleaned
            .to_utf8()
            .map_err(|_| "API key cannot be encoded as UTF-8".to_owned()),
        None => Ok(String::new()),
    }
}

/// A stored-settings file whose bytes are not UTF-8.
///
/// The source `Path.read_text(encoding="utf-8")` raises
/// `UnicodeDecodeError` for these bytes: reads (health, providers, settings,
/// chat) become the sanitized 500, and a settings write reports the codec
/// message through the route's 422 `invalid_request` branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDecodeError {
    message: String,
}

impl SettingsDecodeError {
    /// The CPython `UnicodeDecodeError` text (without the outer quotes).
    #[must_use]
    pub fn python_message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for SettingsDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SettingsDecodeError {}

/// A stored-settings read that the source read would raise on.
///
/// Python's `read_stored_keys` catches `OSError` and `json.JSONDecodeError`
/// only. `Path.read_text(encoding="utf-8")` raises `UnicodeDecodeError` (a
/// `ValueError`), and `json.loads` raises `ValueError` for an integer literal
/// past the 4,300-digit conversion limit; both escape the store. Reads on the
/// affected routes become the sanitized 500, and a settings write reports the
/// message through the route's 422 `invalid_request` branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsReadError {
    /// `Path.read_text(encoding="utf-8")` raised `UnicodeDecodeError`.
    Decode(SettingsDecodeError),
    /// `json.loads` raised `ValueError` from CPython's integer conversion
    /// limit; the text is the exact CPython message.
    IntegerLimit(String),
}

impl SettingsReadError {
    /// The Python error message (`str(error)`).
    #[must_use]
    pub fn python_message(&self) -> &str {
        match self {
            Self::Decode(error) => error.python_message(),
            Self::IntegerLimit(message) => message,
        }
    }

    /// The Python exception class name the source logs for this failure.
    #[must_use]
    pub fn python_exception_name(&self) -> &'static str {
        match self {
            Self::Decode(_) => "UnicodeDecodeError",
            Self::IntegerLimit(_) => "ValueError",
        }
    }
}

impl std::fmt::Display for SettingsReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.python_message())
    }
}

impl std::error::Error for SettingsReadError {}

fn utf8_error_message(raw: &[u8], error: std::str::Utf8Error) -> String {
    let start = error.valid_up_to();
    let lead = raw.get(start).copied().unwrap_or(0);
    match error.error_len() {
        None => {
            // CPython names the whole truncated sequence.
            let tail = &raw[start..];
            if tail.len() <= 1 {
                format!(
                    "'utf-8' codec can't decode byte 0x{lead:02x} in position {start}: unexpected end of data"
                )
            } else {
                format!(
                    "'utf-8' codec can't decode bytes in position {start}-{}: unexpected end of data",
                    start + tail.len() - 1
                )
            }
        }
        Some(_) if (0xC2..=0xF4).contains(&lead) => format!(
            "'utf-8' codec can't decode byte 0x{lead:02x} in position {start}: invalid continuation byte"
        ),
        Some(_) => format!(
            "'utf-8' codec can't decode byte 0x{lead:02x} in position {start}: invalid start byte"
        ),
    }
}

/// Extract validated keys from a `serde_json` document.
///
/// Returns `None` when the source would not reach the provider loop at all
/// (the top level or `api_keys` is not an object), exactly like the Python
/// source.
fn keys_from_serde(value: &serde_json::Value) -> Option<StoredKeys> {
    let api_keys = value.get("api_keys")?.as_object()?;
    let mut result = StoredKeys::new();
    for provider in SUPPORTED_CREDENTIAL_PROVIDERS {
        let Some(candidate) = api_keys.get(*provider).and_then(serde_json::Value::as_str) else {
            continue;
        };
        if let Ok(Some(cleaned)) = validate_api_key_text(&JsonText::new(candidate)) {
            result.insert(*provider, cleaned);
        }
    }
    Some(result)
}

/// Extract validated keys from a codepoint-fidelity canonical document.
fn keys_from_canonical(value: &CanonicalValue) -> Option<StoredKeys> {
    let CanonicalValue::Object(root) = value else {
        return None;
    };
    let CanonicalValue::Object(api_keys) = root.get(&JsonText::new("api_keys"))? else {
        return None;
    };
    let mut result = StoredKeys::new();
    for provider in SUPPORTED_CREDENTIAL_PROVIDERS {
        let Some(CanonicalValue::Text(candidate)) = api_keys.get(&JsonText::new(provider)) else {
            continue;
        };
        if let Ok(Some(cleaned)) = validate_api_key_text(candidate) {
            result.insert(*provider, cleaned);
        }
    }
    Some(result)
}

/// Extract validated keys from a codepoint-fidelity projection.
fn keys_from_candidates(candidates: BTreeMap<&'static str, JsonText>) -> StoredKeys {
    let mut result = StoredKeys::new();
    for (provider, candidate) in candidates {
        if let Ok(Some(cleaned)) = validate_api_key_text(&candidate) {
            result.insert(provider, cleaned);
        }
    }
    result
}

/// The reviewed parsers as a safety net for documents the projection rejects.
///
/// `serde_json` cannot parse escaped lone surrogates and the strict
/// `kala-json` parser rejects duplicate keys, so this path only runs when the
/// projection itself reported malformed input. A document no parser accepts
/// is `{}`, exactly like the source's `except json.JSONDecodeError`.
fn parse_settings_keys_reviewed_fallback(text: &str) -> StoredKeys {
    let Ok(normalised) = crate::json_input::normalise_python_values(text) else {
        return StoredKeys::new();
    };
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&normalised) {
        return keys_from_serde(&value).unwrap_or_default();
    }
    match kala_json::parse_strict(&normalised, &SETTINGS_PARSE_LIMITS) {
        Ok(value) => keys_from_canonical(&value).unwrap_or_default(),
        Err(_) => StoredKeys::new(),
    }
}

/// Parse a settings document with Python `json.loads` semantics.
///
/// [`crate::settings_json`] walks the document with CPython's acceptance
/// rules: duplicate keys are last-wins, escaped lone surrogates survive as
/// code points, `NaN`/`Infinity` are values, and an integer literal past
/// CPython's 4,300-digit conversion limit is reported in scan order with the
/// exact source `ValueError` message. A malformed document still falls back
/// to the reviewed parsers so an accepted document is never dropped by a
/// projection bug.
///
/// # Errors
///
/// Returns the CPython `ValueError` text when the document contains an
/// integer literal the source cannot convert.
fn parse_settings_keys(text: &str) -> Result<StoredKeys, String> {
    match crate::settings_json::parse_settings_document(text) {
        SettingsDocument::Parsed(candidates) => Ok(keys_from_candidates(candidates)),
        SettingsDocument::IntegerLimit(message) => Err(message),
        SettingsDocument::Malformed => Ok(parse_settings_keys_reviewed_fallback(text)),
    }
}

/// Read validated stored keys, surfacing the source's read failures.
///
/// Missing, unreadable and malformed-JSON files are an empty map, exactly as
/// in the Python source; only the failures the source does not catch — a
/// UTF-8 decoding error and the integer conversion limit — are reported.
///
/// # Errors
///
/// Returns [`SettingsReadError::Decode`] when the file is not valid UTF-8 and
/// [`SettingsReadError::IntegerLimit`] when it holds an integer literal past
/// CPython's 4,300-digit conversion limit.
pub fn read_stored_keys_checked(data_dir: Option<&Path>) -> Result<StoredKeys, SettingsReadError> {
    let Some(data_dir) = data_dir else {
        return Ok(StoredKeys::new());
    };
    let path = data_dir.join(STUDIO_SETTINGS_FILE_NAME);
    // The source `Path.read_text` has no size cap: a valid stored key must
    // survive legal trailing JSON whitespace beyond any arbitrary threshold.
    let Ok(metadata) = fs::metadata(&path) else {
        return Ok(StoredKeys::new());
    };
    if !metadata.is_file() {
        return Ok(StoredKeys::new());
    }
    let Ok(raw) = fs::read(&path) else {
        return Ok(StoredKeys::new());
    };
    let text = match std::str::from_utf8(&raw) {
        Ok(text) => text,
        Err(error) => {
            return Err(SettingsReadError::Decode(SettingsDecodeError {
                message: utf8_error_message(&raw, error),
            }));
        }
    };
    parse_settings_keys(text).map_err(SettingsReadError::IntegerLimit)
}

/// Read validated stored keys; missing, unreadable or malformed files are an
/// empty map, and a failure the source raises is reported as empty for
/// callers that cannot surface it. Server routes use
/// [`read_stored_keys_checked`].
#[must_use]
pub fn read_stored_keys(data_dir: Option<&Path>) -> StoredKeys {
    read_stored_keys_checked(data_dir).unwrap_or_default()
}

/// Store or clear one provider key and return the resulting validated map.
///
/// # Errors
///
/// Returns the sanitized validation error for an unsupported provider or an
/// invalid key, or the I/O error from the atomic writer.
pub fn write_stored_key(
    data_dir: &Path,
    provider: &str,
    api_key: Option<&str>,
) -> Result<StoredKeys, WriteKeyError> {
    if !SUPPORTED_CREDENTIAL_PROVIDERS.contains(&provider) {
        return Err(WriteKeyError::Validation(format!(
            "unsupported provider for API key settings: '{provider}'"
        )));
    }
    let cleaned = validate_api_key(api_key).map_err(WriteKeyError::Validation)?;
    let mut current = read_stored_keys_checked(Some(data_dir))
        .map_err(|error| WriteKeyError::Validation(error.python_message().to_owned()))?;
    let provider_static = SUPPORTED_CREDENTIAL_PROVIDERS
        .iter()
        .find(|candidate| **candidate == provider)
        .copied()
        .ok_or_else(|| {
            WriteKeyError::Validation(format!(
                "unsupported provider for API key settings: '{provider}'"
            ))
        })?;
    if cleaned.is_empty() {
        current.remove(provider_static);
    } else {
        current.insert(provider_static, JsonText::new(&cleaned));
    }
    let text = render_settings(&current);
    atomic_write_text(
        &data_dir.join(STUDIO_SETTINGS_FILE_NAME),
        &text,
        TOKEN_FILE_MODE,
    )
    .map_err(WriteKeyError::Io)?;
    Ok(current)
}

/// Why a settings write failed.
#[derive(Debug)]
pub enum WriteKeyError {
    Validation(String),
    Io(io::Error),
}

impl std::fmt::Display for WriteKeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for WriteKeyError {}

/// Render `json.dumps({"api_keys": keys}, indent=2, sort_keys=True) + "\n"`.
///
/// The two provider names are inserted in sorted order and every value uses
/// CPython's `ensure_ascii=True` escaping, including escaped lone surrogates
/// that Rust strings cannot hold.
fn render_settings(keys: &StoredKeys) -> String {
    let present: Vec<&'static str> = SUPPORTED_CREDENTIAL_PROVIDERS
        .iter()
        .filter(|provider| keys.contains_key(**provider))
        .copied()
        .collect();
    let mut out = String::from("{\n  \"api_keys\": {");
    if present.is_empty() {
        out.push_str("}\n}\n");
        return out;
    }
    for provider in &present {
        let Some(value) = keys.get(*provider) else {
            continue;
        };
        out.push_str("\n    \"");
        out.push_str(provider);
        out.push_str("\": ");
        encode_code_points(value.code_points(), &mut out);
        out.push(',');
    }
    out.pop();
    out.push_str("\n  }\n}\n");
    out
}

/// CPython `json.dumps(..., ensure_ascii=True)` string escaping for code
/// points, including lone surrogates.
fn encode_code_points(points: &[u32], out: &mut String) {
    out.push('"');
    for point in points {
        match *point {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x08 => out.push_str("\\b"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0C => out.push_str("\\f"),
            0x0D => out.push_str("\\r"),
            0x20..=0x7E => {
                out.push(char::from_u32(*point).unwrap_or('\u{fffd}'));
            }
            0x00..=0xFFFF => {
                out.push_str(&format!("\\u{point:04x}"));
            }
            _ => {
                let value = point - 0x1_0000;
                let high = 0xD800 + (value >> 10);
                let low = 0xDC00 + (value & 0x3FF);
                out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
            }
        }
    }
    out.push('"');
}

/// Merge stored keys over the base environment.
///
/// A stored value that contains a lone surrogate cannot be represented in a
/// Rust `String`; it is omitted here, and callers that report configured
/// state or dispatch a provider use the [`StoredKeys`] map directly so the
/// source semantics are preserved.
#[must_use]
pub fn effective_environ(
    base: &BTreeMap<String, String>,
    stored: &StoredKeys,
) -> BTreeMap<String, String> {
    let mut merged = base.clone();
    for (provider, env_name) in PROVIDER_ENV_VARS {
        if let Some(key) = stored.get(provider) {
            if let Ok(text) = key.to_utf8() {
                if !text.is_empty() {
                    merged.insert((*env_name).to_owned(), text);
                }
            }
        }
    }
    merged
}

/// Whether one provider has a non-empty stored key.
#[must_use]
pub fn stored_key_configured(stored: &StoredKeys, provider: &str) -> bool {
    stored
        .get(provider)
        .is_some_and(|value| !value.code_points().is_empty())
}

/// The subscription providers whose credential is the operator's own CLI
/// login.
///
/// The rows come after the API-key rows so the stored-key indices of the
/// settings route do not move.
pub const SUBSCRIPTION_CREDENTIAL_PROVIDERS: &[&str] =
    &["codex", "claude_code", "codex_cli", "opencode_cli"];

/// Return non-secret credential status rows in provider order.
///
/// The API-key rows report where the key comes from. The subscription rows
/// report the CLI's own probe: the row is `configured` when the CLI is
/// installed and signed in, and its source is `subscription` because the
/// credential lives in the CLI's own store, never in Kala's.
#[must_use]
pub fn describe_provider_credentials(
    base: &BTreeMap<String, String>,
    stored: &StoredKeys,
    subscription_probes: &[crate::providers::CliProbe],
) -> Vec<CredentialStatus> {
    let mut rows = Vec::with_capacity(SUPPORTED_CREDENTIAL_PROVIDERS.len());
    for provider in SUPPORTED_CREDENTIAL_PROVIDERS {
        let env_var = PROVIDER_ENV_VARS
            .iter()
            .find(|(candidate, _)| candidate == provider)
            .map(|(_, env_var)| *env_var)
            .unwrap_or("");
        let stored_key = stored.get(*provider);
        let env_key = base
            .get(env_var)
            .map(|value| pytext::trim(value))
            .unwrap_or("");
        let (configured, source) =
            if stored_key.is_some_and(|value| !value.code_points().is_empty()) {
                (true, CredentialSource::Stored)
            } else if !env_key.is_empty() {
                (true, CredentialSource::Environment)
            } else {
                (false, CredentialSource::None)
            };
        rows.push(CredentialStatus {
            provider,
            env_var,
            configured,
            source,
        });
    }
    for provider in SUBSCRIPTION_CREDENTIAL_PROVIDERS {
        let probe = subscription_probes.iter().find(|probe| {
            probe.id == *provider
                || (*provider == "codex" && probe.id == crate::providers::codex_cli::PROVIDER_ID)
        });
        rows.push(CredentialStatus {
            provider,
            env_var: "",
            configured: probe.is_some_and(crate::providers::CliProbe::usable),
            source: CredentialSource::Subscription,
        });
    }
    rows
}

/// Write text through a temporary file in the destination directory and an
/// atomic replacement, tightening the mode on POSIX.
///
/// On Windows the `0o600` guarantee is an explicit, inheritance-free DACL that
/// names the owner, `LocalSystem` and `Administrators` only
/// ([`kala_process::restrict_file_to_current_user`]); it is installed on the
/// temporary file before the content is written, and the atomic replacement
/// carries it to the destination. Both platforms treat the restriction as part
/// of the write: a secret that cannot be restricted is reported rather than
/// left readable by an inherited entry.
///
/// # Errors
///
/// Returns the underlying I/O error and removes the temporary file on
/// failure.
pub fn atomic_write_text(path: &Path, content: &str, mode: u32) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))?;
    #[cfg(windows)]
    {
        // Restrict the temporary file *before* the secret is written into it,
        // so it is never readable through the directory's inherited entries.
        // The replacement keeps the DACL, so the destination inherits it.
        kala_process::restrict_file_to_current_user(temporary.path())?;
    }
    temporary.write_all(content.as_bytes())?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Python's `_tighten_file_mode` treats a chmod failure as advisory.
        let _ = temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }
    temporary.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    Ok(())
}

/// The directory name the settings fallback carries under the per-user base.
const SETTINGS_DIRECTORY_NAME: &str = "kala-studio";

/// Return the per-user directory used by the POST settings route when the
/// server has no configured directory.
///
/// The shared temporary directory is never used: another local principal can
/// create `<temp>/kala-studio` before first use, so a secret written through
/// that path would land in a directory this user does not control. The
/// per-user base is `%LOCALAPPDATA%` (or `%USERPROFILE%\AppData\Local`) on
/// Windows and `$XDG_STATE_HOME` (or `$HOME/.local/state`) everywhere else.
///
/// # Errors
///
/// Returns a message naming what is missing when no per-user base directory
/// can be resolved, so the route reports it instead of writing to a shared
/// location.
pub fn fallback_settings_dir() -> Result<PathBuf, String> {
    Ok(per_user_settings_base()?.join(SETTINGS_DIRECTORY_NAME))
}

/// Resolve the absolute per-user base directory for the settings fallback.
fn per_user_settings_base() -> Result<PathBuf, String> {
    #[cfg(windows)]
    const CANDIDATES: &[(&str, &str)] = &[("LOCALAPPDATA", ""), ("USERPROFILE", "AppData\\Local")];
    #[cfg(not(windows))]
    const CANDIDATES: &[(&str, &str)] = &[("XDG_STATE_HOME", ""), ("HOME", ".local/state")];
    for (name, suffix) in CANDIDATES {
        if let Some(base) = per_user_directory(name, suffix) {
            return Ok(base);
        }
    }
    Err(format!(
        "no per-user settings directory is configured; set {} or pass --data-dir",
        CANDIDATES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(" or ")
    ))
}

/// Read one per-user base directory from the process environment.
///
/// A value that is not absolute (or is a filesystem root) is not a per-user
/// location and is refused, like the source's own settings validator.
fn per_user_directory(name: &str, suffix: &str) -> Option<PathBuf> {
    let raw = std::env::var_os(name)?;
    if raw.is_empty() {
        return None;
    }
    let mut base = PathBuf::from(raw);
    if !suffix.is_empty() {
        base = base.join(suffix);
    }
    (base.is_absolute() && base.parent().is_some()).then_some(base)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn utf8(keys: &StoredKeys, provider: &str) -> Option<String> {
        keys.get(provider).and_then(|value| value.to_utf8().ok())
    }

    #[test]
    fn key_validation_matches_python_messages() {
        assert_eq!(validate_api_key(None).unwrap(), "");
        assert_eq!(validate_api_key(Some("  ")).unwrap(), "");
        assert_eq!(validate_api_key(Some("  sk-x  ")).unwrap(), "sk-x");
        assert_eq!(
            validate_api_key(Some("has space")).unwrap_err(),
            "API key must not contain whitespace"
        );
        assert_eq!(
            validate_api_key(Some(&"x".repeat(257))).unwrap_err(),
            "API key exceeds maximum length"
        );
    }

    #[test]
    fn settings_file_round_trips_python_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
        write_stored_key(directory.path(), "openai", Some("sk-x")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"api_keys\": {\n    \"openai\": \"sk-x\"\n  }\n}\n"
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            utf8(&read_stored_keys(Some(directory.path())), "openai").as_deref(),
            Some("sk-x")
        );
        write_stored_key(directory.path(), "openai", None).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "{\n  \"api_keys\": {}\n}\n"
        );
    }

    #[test]
    fn the_settings_fallback_is_a_per_user_directory() {
        // REVIEW-8 probe: the shared temporary directory must never be the
        // place a stored provider key lands.
        let fallback = fallback_settings_dir().expect("a per-user base is configured");
        assert!(fallback.is_absolute(), "{fallback:?}");
        assert!(fallback.ends_with("kala-studio"), "{fallback:?}");
        assert_ne!(fallback, std::env::temp_dir().join("kala-studio"));
        assert!(
            !fallback.starts_with(std::env::temp_dir()),
            "{fallback:?} must not live under the shared temporary directory"
        );
    }

    #[test]
    fn decode_error_messages_match_cpython() {
        let cases: [(&[u8], &str); 5] = [
            (
                b"\xff",
                "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte",
            ),
            (
                b"\xe2\x28",
                "'utf-8' codec can't decode byte 0xe2 in position 0: invalid continuation byte",
            ),
            (
                b"a\xe2",
                "'utf-8' codec can't decode byte 0xe2 in position 1: unexpected end of data",
            ),
            (
                b"\xf0\x9f",
                "'utf-8' codec can't decode bytes in position 0-1: unexpected end of data",
            ),
            (
                b"\xf5\x80\x80\x80",
                "'utf-8' codec can't decode byte 0xf5 in position 0: invalid start byte",
            ),
        ];
        for (raw, expected) in cases {
            let error = std::str::from_utf8(raw).unwrap_err();
            assert_eq!(utf8_error_message(raw, error), expected);
        }
    }

    #[test]
    fn non_finite_unrelated_values_do_not_discard_valid_keys() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join(STUDIO_SETTINGS_FILE_NAME),
            r#"{"api_keys":{"openai":"sk-x","other":NaN}}"#,
        )
        .unwrap();
        let keys = read_stored_keys(Some(directory.path()));
        assert_eq!(utf8(&keys, "openai").as_deref(), Some("sk-x"));
    }

    #[test]
    fn escaped_lone_surrogates_are_preserved_byte_for_byte() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
        fs::write(
            &path,
            br#"{"api_keys":{"openai":"sk-\ud800","openrouter":"sk-or"}}"#,
        )
        .unwrap();
        let keys = read_stored_keys(Some(directory.path()));
        let openai = keys.get("openai").expect("the surrogate key is stored");
        assert!(openai.has_surrogates());
        assert_eq!(openai.code_points(), &[0x73, 0x6B, 0x2D, 0xD800]);
        assert_eq!(utf8(&keys, "openrouter").as_deref(), Some("sk-or"));
        // Writing the other provider rewrites the document exactly like
        // `json.dumps(..., ensure_ascii=True)` and keeps the surrogate intact.
        write_stored_key(directory.path(), "openrouter", Some("sk-new")).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"{\n  \"api_keys\": {\n    \"openai\": \"sk-\\ud800\",\n    \"openrouter\": \"sk-new\"\n  }\n}\n"
        );
    }

    #[test]
    fn escaped_surrogate_in_a_key_breaks_utf8_conversion_only() {
        let points: Vec<u32> = vec![0xD800];
        let text = JsonText::from_code_points(points).unwrap();
        assert!(text.to_utf8().is_err());
        // Validation accepts the surrogate code point, like CPython's
        // `str.strip`/`len`/`ord` checks: it is neither a control character
        // nor whitespace.
        let cleaned = validate_api_key_text(&text).unwrap().expect("non-empty");
        assert_eq!(cleaned.code_points(), &[0xD800]);
    }

    #[test]
    fn escapes_non_ascii_like_cpython() {
        let directory = tempfile::tempdir().unwrap();
        write_stored_key(directory.path(), "openai", Some("clé")).unwrap();
        let text = fs::read_to_string(directory.path().join(STUDIO_SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains("cl\\u00e9"));
    }

    #[test]
    fn duplicate_and_non_object_documents_match_python() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
        fs::write(
            &path,
            r#"{"api_keys":{"openai":"first","openai":"second"}}"#,
        )
        .unwrap();
        assert_eq!(
            utf8(&read_stored_keys(Some(directory.path())), "openai").as_deref(),
            Some("second")
        );
        fs::write(&path, r#"[{"api_keys":{"openai":"x"}}]"#).unwrap();
        assert!(read_stored_keys(Some(directory.path())).is_empty());
        fs::write(&path, r#"{"api_keys":[]}"#).unwrap();
        assert!(read_stored_keys(Some(directory.path())).is_empty());
    }

    #[test]
    fn duplicate_and_surrogate_settings_survive_and_rewrite_like_python() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
        // REVIEW-3 probe: duplicate `api_keys` members at the root, a duplicate
        // provider key, an escaped surrogate in an ignored field and an actual
        // surrogate escape in the retained provider value. CPython keeps the
        // last member/value and preserves the surrogate code point.
        fs::write(
            &path,
            br#"{"api_keys":{},"api_keys":{"openai":"first","openai":"synthetic-review-key","openrouter":"sk-\ud800"},"unused":"\ud800"}"#,
        )
        .unwrap();
        let keys = read_stored_keys(Some(directory.path()));
        assert_eq!(
            utf8(&keys, "openai").as_deref(),
            Some("synthetic-review-key")
        );
        let openrouter = keys.get("openrouter").expect("surrogate key retained");
        assert!(openrouter.has_surrogates());
        assert_eq!(openrouter.code_points(), &[0x73, 0x6B, 0x2D, 0xD800]);

        // An unrelated provider write rewrites the canonical document without
        // losing the duplicated provider and keeps the surrogate escape.
        write_stored_key(directory.path(), "openrouter", Some("sk-or-new")).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"{\n  \"api_keys\": {\n    \"openai\": \"synthetic-review-key\",\n    \"openrouter\": \"sk-or-new\"\n  }\n}\n"
        );

        // A clear removes only the requested provider.
        fs::write(
            &path,
            br#"{"api_keys":{},"api_keys":{"openai":"synthetic-review-key","openrouter":"sk-\ud800"},"unused":"\ud800"}"#,
        )
        .unwrap();
        write_stored_key(directory.path(), "openai", None).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"{\n  \"api_keys\": {\n    \"openrouter\": \"sk-\\ud800\"\n  }\n}\n"
        );
    }

    #[test]
    fn integer_limited_settings_match_the_source_failure_class() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(STUDIO_SETTINGS_FILE_NAME);
        let digits = "7".repeat(4_301);
        let payload = format!(r#"{{"api_keys":{{"openai":"sk-x"}},"unused":{digits}}}"#);
        fs::write(&path, &payload).unwrap();

        let error = read_stored_keys_checked(Some(directory.path())).unwrap_err();
        assert_eq!(error.python_exception_name(), "ValueError");
        assert_eq!(
            error.python_message(),
            "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit"
        );

        // A settings write refuses with the source message and leaves the
        // original bytes untouched (the read fails before the atomic write).
        match write_stored_key(directory.path(), "openrouter", Some("sk-y")) {
            Err(WriteKeyError::Validation(message)) => assert_eq!(
                message,
                "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits; use sys.set_int_max_str_digits() to increase the limit"
            ),
            other => panic!("expected the source ValueError class, got {other:?}"),
        }
        assert_eq!(fs::read(&path).unwrap(), payload.as_bytes());

        // CPython's boundary is inclusive: 4,300 digits is a legal literal.
        let at_limit = format!(
            r#"{{"api_keys":{{"openai":"sk-x"}},"unused":{}}}"#,
            "7".repeat(4_300)
        );
        fs::write(&path, at_limit).unwrap();
        assert_eq!(
            utf8(&read_stored_keys(Some(directory.path())), "openai").as_deref(),
            Some("sk-x")
        );
    }

    #[test]
    fn deep_valid_documents_are_read_like_python() {
        // More nesting than serde's 128-level syntactic limit but well within
        // CPython's and the settings contract's accepted domain.
        let depth = 400;
        let mut document = String::from(r#"{"api_keys":{"openai":"sk-deep"},"other":"#);
        document.push_str(&"[".repeat(depth));
        document.push_str(&"]".repeat(depth));
        document.push('}');
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(STUDIO_SETTINGS_FILE_NAME), document).unwrap();
        assert_eq!(
            utf8(&read_stored_keys(Some(directory.path())), "openai").as_deref(),
            Some("sk-deep")
        );
    }
}
