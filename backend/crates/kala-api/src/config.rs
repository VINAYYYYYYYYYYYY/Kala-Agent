//! The local configuration of the Kala agent.
//!
//! The Python source is a Pydantic model with `extra="forbid"`,
//! `frozen=True` and `validate_default=True`. This port keeps the same
//! defaults, the same environment allowlist, the same precedence and the same
//! validation messages, and it reports the failures with the same kinds, so a
//! caller can tell a rejected path from a rejected timeout without parsing
//! prose.
//!
//! The environment is the only source. The Python module has no file source
//! either: `Settings.from_env` reads the seven allowlisted variables and
//! leaves everything else, including every secret, untouched.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use url::Url;

use crate::pytext;

/// The environment variables the settings read, and the fields they fill.
pub const ENVIRONMENT_FIELDS: [(&str, &str); 7] = [
    ("KALA_AGENT_DATA_DIR", "data_dir"),
    ("KALA_AGENT_MODEL_PROVIDER", "model_provider"),
    ("KALA_AGENT_MODEL_ID", "model_id"),
    ("KALA_AGENT_MODEL_ENDPOINT", "model_endpoint"),
    (
        "KALA_AGENT_REQUEST_TIMEOUT_SECONDS",
        "request_timeout_seconds",
    ),
    ("KALA_AGENT_TOOL_TIMEOUT_SECONDS", "tool_timeout_seconds"),
    ("KALA_AGENT_WORKER_COUNT", "worker_count"),
];

/// The default model provider.
pub const DEFAULT_MODEL_PROVIDER: &str = "local";

/// The default model identifier.
pub const DEFAULT_MODEL_ID: &str = "local";

/// The default request timeout in seconds.
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = 120.0;

/// The default tool timeout in seconds.
pub const DEFAULT_TOOL_TIMEOUT_SECONDS: f64 = 300.0;

/// The default number of workers.
pub const DEFAULT_WORKER_COUNT: u8 = 4;

/// The smallest number of workers.
pub const MIN_WORKER_COUNT: u8 = 1;

/// The largest number of workers.
pub const MAX_WORKER_COUNT: u8 = 8;

/// The longest model provider name.
pub const MAX_MODEL_PROVIDER_LENGTH: usize = 64;

/// The longest model identifier.
pub const MAX_MODEL_ID_LENGTH: usize = 256;

/// The validation error kinds the Python source reports.
pub mod error_kind {
    /// A field validator refused the value.
    pub const VALUE_ERROR: &str = "value_error";
    /// The value was not a string.
    pub const STRING_TYPE: &str = "string_type";
    /// The string was shorter than the minimum.
    pub const STRING_TOO_SHORT: &str = "string_too_short";
    /// The string was longer than the maximum.
    pub const STRING_TOO_LONG: &str = "string_too_long";
    /// The number was smaller than the minimum.
    pub const GREATER_THAN_EQUAL: &str = "greater_than_equal";
    /// The number was larger than the maximum.
    pub const LESS_THAN_EQUAL: &str = "less_than_equal";
    /// The URL could not be parsed.
    pub const URL_PARSING: &str = "url_parsing";
    /// The URL did not use an HTTP scheme.
    pub const URL_SCHEME: &str = "url_scheme";
    /// The value was not a URL.
    pub const URL_TYPE: &str = "url_type";
    /// The input carried a field the model does not declare.
    pub const EXTRA_FORBIDDEN: &str = "extra_forbidden";
}

/// One rejected configuration value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsError {
    /// The machine readable kind of the failure.
    pub kind: String,
    /// The field the failure belongs to.
    pub location: String,
    /// The human readable message, in the Python source's wording.
    pub message: String,
}

impl SettingsError {
    /// Build a failure of a field validator.
    fn value_error(location: &str, reason: &str) -> Self {
        Self {
            kind: error_kind::VALUE_ERROR.to_owned(),
            location: location.to_owned(),
            message: format!("Value error, {reason}"),
        }
    }
}

/// One configuration value, as it arrives from a caller or an environment.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    /// A string value.
    Text(String),
    /// An integer value.
    Integer(i64),
    /// A floating point value.
    Float(f64),
    /// A boolean value.
    Boolean(bool),
    /// An explicit null.
    Null,
}

/// The validated local configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The local data directory. It is always absolute and resolved.
    pub data_dir: PathBuf,
    /// The model provider name, normalized to lower case.
    pub model_provider: String,
    /// The model identifier.
    pub model_id: String,
    /// The declared provider endpoint, normalized, if there is one.
    pub model_endpoint: Option<String>,
    /// The request timeout in seconds.
    pub request_timeout_seconds: f64,
    /// The tool timeout in seconds.
    pub tool_timeout_seconds: f64,
    /// The number of workers.
    pub worker_count: u8,
}

impl Settings {
    /// The directory the artifacts live in.
    #[must_use]
    pub fn artifact_dir(&self) -> PathBuf {
        self.data_dir.join("artifacts")
    }

    /// The path of the event journal.
    #[must_use]
    pub fn journal_path(&self) -> PathBuf {
        self.data_dir.join("journal.sqlite3")
    }

    /// The path of the application database.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("kala.sqlite3")
    }

    /// The directory the transcripts live in.
    #[must_use]
    pub fn transcript_dir(&self) -> PathBuf {
        self.data_dir.join("transcripts")
    }
}

/// Build the settings from an explicit set of values.
///
/// The keys are the field names of the Python model. A key the model does not
/// declare is refused, exactly as `extra="forbid"` refuses it.
///
/// # Errors
///
/// Returns every rejected field, in the order the Python model declares its
/// fields, with the extra keys last.
pub fn settings_from_values(
    values: &BTreeMap<String, SettingValue>,
) -> Result<Settings, Vec<SettingsError>> {
    let mut errors: Vec<SettingsError> = Vec::new();

    let data_dir = match values.get("data_dir") {
        Some(value) => match validate_data_dir(value) {
            Ok(path) => Some(path),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(default_data_dir()),
    };
    let model_provider = match values.get("model_provider") {
        Some(value) => match validate_model_provider(value) {
            Ok(name) => Some(name),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(DEFAULT_MODEL_PROVIDER.to_owned()),
    };
    let model_id = match values.get("model_id") {
        Some(value) => match validate_model_id(value) {
            Ok(identifier) => Some(identifier),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(DEFAULT_MODEL_ID.to_owned()),
    };
    let model_endpoint = match values.get("model_endpoint") {
        Some(value) => match validate_model_endpoint(value) {
            Ok(endpoint) => Some(endpoint),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(None),
    };
    let request_timeout_seconds = match values.get("request_timeout_seconds") {
        Some(value) => match validate_timeout(value, "request_timeout_seconds") {
            Ok(timeout) => Some(timeout),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(DEFAULT_REQUEST_TIMEOUT_SECONDS),
    };
    let tool_timeout_seconds = match values.get("tool_timeout_seconds") {
        Some(value) => match validate_timeout(value, "tool_timeout_seconds") {
            Ok(timeout) => Some(timeout),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(DEFAULT_TOOL_TIMEOUT_SECONDS),
    };
    let worker_count = match values.get("worker_count") {
        Some(value) => match validate_worker_count(value) {
            Ok(count) => Some(count),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => Some(DEFAULT_WORKER_COUNT),
    };

    for key in values.keys() {
        if !ENVIRONMENT_FIELDS
            .iter()
            .any(|(_, field)| *field == key.as_str())
        {
            errors.push(SettingsError {
                kind: error_kind::EXTRA_FORBIDDEN.to_owned(),
                location: key.clone(),
                message: "Extra inputs are not permitted".to_owned(),
            });
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Settings {
        data_dir: data_dir.unwrap_or_default(),
        model_provider: model_provider.unwrap_or_default(),
        model_id: model_id.unwrap_or_default(),
        model_endpoint: model_endpoint.unwrap_or_default(),
        request_timeout_seconds: request_timeout_seconds.unwrap_or_default(),
        tool_timeout_seconds: tool_timeout_seconds.unwrap_or_default(),
        worker_count: worker_count.unwrap_or_default(),
    })
}

/// Build the settings from the environment.
///
/// Only the allowlisted variables are read; every other variable, including
/// every secret, is left untouched and never retained.
///
/// # Errors
///
/// Returns every rejected field, in the order the Python model declares its
/// fields.
pub fn settings_from_env(
    environ: &BTreeMap<String, String>,
) -> Result<Settings, Vec<SettingsError>> {
    let mut values: BTreeMap<String, SettingValue> = BTreeMap::new();
    for (environment_name, field_name) in ENVIRONMENT_FIELDS {
        if let Some(value) = environ.get(environment_name) {
            values.insert(field_name.to_owned(), SettingValue::Text(value.clone()));
        }
    }
    settings_from_values(&values)
}

/// The default local data directory.
fn default_data_dir() -> PathBuf {
    home_directory()
        .map(|home| home.join(".local").join("share").join("kala-agent"))
        .unwrap_or_else(|| PathBuf::from(".local").join("share").join("kala-agent"))
}

/// Validate the local data directory.
fn validate_data_dir(value: &SettingValue) -> Result<PathBuf, SettingsError> {
    let candidate = match value {
        SettingValue::Text(text) => PathBuf::from(text),
        _ => {
            return Err(SettingsError::value_error(
                "data_dir",
                "data_dir must be a path",
            ));
        }
    };
    if candidate.to_string_lossy().contains('\u{0}') {
        return Err(SettingsError::value_error(
            "data_dir",
            "data_dir contains an invalid character",
        ));
    }
    let expanded = match expand_user(&candidate) {
        Some(expanded) => expanded,
        None => {
            return Err(SettingsError::value_error(
                "data_dir",
                "data_dir cannot be resolved",
            ));
        }
    };
    if !expanded.is_absolute() {
        return Err(SettingsError::value_error(
            "data_dir",
            "data_dir must be absolute",
        ));
    }
    let resolved = match resolve_path(&expanded) {
        Ok(resolved) => resolved,
        Err(_) => {
            return Err(SettingsError::value_error(
                "data_dir",
                "data_dir cannot be resolved",
            ));
        }
    };
    if resolved.parent().is_none() {
        return Err(SettingsError::value_error(
            "data_dir",
            "data_dir cannot be a filesystem root",
        ));
    }
    if resolved.exists() && !resolved.is_dir() {
        return Err(SettingsError::value_error(
            "data_dir",
            "data_dir must identify a directory",
        ));
    }
    Ok(resolved)
}

/// Validate the model provider name.
fn validate_model_provider(value: &SettingValue) -> Result<String, SettingsError> {
    let text = match value {
        SettingValue::Text(text) => text,
        _ => {
            return Err(SettingsError {
                kind: error_kind::STRING_TYPE.to_owned(),
                location: "model_provider".to_owned(),
                message: "Input should be a valid string".to_owned(),
            });
        }
    };
    let stripped = strip_pydantic(text);
    if let Some(error) = check_length(&stripped, 1, MAX_MODEL_PROVIDER_LENGTH, "model_provider") {
        return Err(error);
    }
    let normalized = pytext::lower(pytext::trim(&stripped));
    if !is_provider_name(&normalized) {
        return Err(SettingsError::value_error(
            "model_provider",
            "model_provider contains unsupported characters",
        ));
    }
    Ok(normalized)
}

/// Validate the model identifier.
fn validate_model_id(value: &SettingValue) -> Result<String, SettingsError> {
    let text = match value {
        SettingValue::Text(text) => text,
        _ => {
            return Err(SettingsError {
                kind: error_kind::STRING_TYPE.to_owned(),
                location: "model_id".to_owned(),
                message: "Input should be a valid string".to_owned(),
            });
        }
    };
    let stripped = strip_pydantic(text);
    if let Some(error) = check_length(&stripped, 1, MAX_MODEL_ID_LENGTH, "model_id") {
        return Err(error);
    }
    Ok(stripped)
}

/// Validate the declared provider endpoint.
fn validate_model_endpoint(value: &SettingValue) -> Result<Option<String>, SettingsError> {
    let text = match value {
        SettingValue::Null => return Ok(None),
        SettingValue::Text(text) => text,
        _ => {
            return Err(SettingsError {
                kind: error_kind::URL_TYPE.to_owned(),
                location: "model_endpoint".to_owned(),
                message: "URL input should be a string or URL".to_owned(),
            });
        }
    };
    let parsed = match Url::parse(text) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Err(SettingsError {
                kind: error_kind::URL_PARSING.to_owned(),
                location: "model_endpoint".to_owned(),
                message: format!("Input should be a valid URL, {}", url_error_reason(error)),
            });
        }
    };
    let scheme = parsed.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(SettingsError {
            kind: error_kind::URL_SCHEME.to_owned(),
            location: "model_endpoint".to_owned(),
            message: "URL scheme should be 'http' or 'https'".to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(SettingsError::value_error(
            "model_endpoint",
            "model_endpoint cannot contain credentials",
        ));
    }
    let has_query = parsed.query().is_some_and(|query| !query.is_empty());
    let has_fragment = parsed
        .fragment()
        .is_some_and(|fragment| !fragment.is_empty());
    if has_query || has_fragment {
        return Err(SettingsError::value_error(
            "model_endpoint",
            "model_endpoint cannot contain query parameters or fragments",
        ));
    }
    Ok(Some(parsed.to_string()))
}

/// Validate a timeout in seconds.
fn validate_timeout(value: &SettingValue, location: &str) -> Result<f64, SettingsError> {
    let parsed = match value {
        SettingValue::Boolean(_) => {
            return Err(SettingsError::value_error(
                location,
                "timeouts must be numeric",
            ));
        }
        SettingValue::Text(text) => match python_float(text) {
            Some(parsed) => parsed,
            None => {
                return Err(SettingsError::value_error(
                    location,
                    "timeouts must be numeric",
                ));
            }
        },
        SettingValue::Integer(number) => *number as f64,
        SettingValue::Float(number) => *number,
        SettingValue::Null => {
            return Err(SettingsError::value_error(
                location,
                "timeouts must be numeric",
            ));
        }
    };
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(SettingsError::value_error(
            location,
            "timeouts must be finite and nonnegative",
        ));
    }
    Ok(parsed)
}

/// Validate the number of workers.
fn validate_worker_count(value: &SettingValue) -> Result<u8, SettingsError> {
    let count = match value {
        SettingValue::Boolean(_) => {
            return Err(SettingsError::value_error(
                "worker_count",
                "worker_count must be an integer",
            ));
        }
        SettingValue::Integer(number) => *number,
        SettingValue::Text(text) => {
            let candidate = pytext::trim(text);
            if candidate.is_empty() || !candidate.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(SettingsError::value_error(
                    "worker_count",
                    "worker_count must be an integer",
                ));
            }
            match candidate.parse::<i64>() {
                Ok(number) => number,
                Err(_) => {
                    return Err(SettingsError::value_error(
                        "worker_count",
                        "worker_count must be an integer",
                    ));
                }
            }
        }
        _ => {
            return Err(SettingsError::value_error(
                "worker_count",
                "worker_count must be an integer",
            ));
        }
    };
    if count < i64::from(MIN_WORKER_COUNT) {
        return Err(SettingsError {
            kind: error_kind::GREATER_THAN_EQUAL.to_owned(),
            location: "worker_count".to_owned(),
            message: format!("Input should be greater than or equal to {MIN_WORKER_COUNT}"),
        });
    }
    if count > i64::from(MAX_WORKER_COUNT) {
        return Err(SettingsError {
            kind: error_kind::LESS_THAN_EQUAL.to_owned(),
            location: "worker_count".to_owned(),
            message: format!("Input should be less than or equal to {MAX_WORKER_COUNT}"),
        });
    }
    Ok(count as u8)
}

/// Strip a string the way Pydantic's `strip_whitespace` does.
fn strip_pydantic(value: &str) -> String {
    value.trim().to_owned()
}

/// Check a stripped string against the length bounds.
fn check_length(
    value: &str,
    minimum: usize,
    maximum: usize,
    location: &str,
) -> Option<SettingsError> {
    let length = value.chars().count();
    if length < minimum {
        return Some(SettingsError {
            kind: error_kind::STRING_TOO_SHORT.to_owned(),
            location: location.to_owned(),
            message: if minimum == 1 {
                "String should have at least 1 character".to_owned()
            } else {
                format!("String should have at least {minimum} characters")
            },
        });
    }
    if length > maximum {
        return Some(SettingsError {
            kind: error_kind::STRING_TOO_LONG.to_owned(),
            location: location.to_owned(),
            message: format!("String should have at most {maximum} characters"),
        });
    }
    None
}

/// Whether a normalized provider name matches the accepted pattern.
fn is_provider_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
}

/// The reason a URL could not be parsed, in the Python source's wording.
fn url_error_reason(error: url::ParseError) -> &'static str {
    match error {
        url::ParseError::EmptyHost => "empty host",
        url::ParseError::IdnaError => "invalid international domain name",
        url::ParseError::InvalidPort => "invalid port number",
        url::ParseError::InvalidIpv4Address => "invalid IPv4 address",
        url::ParseError::InvalidIpv6Address => "invalid IPv6 address",
        url::ParseError::InvalidDomainCharacter => "invalid domain character",
        url::ParseError::RelativeUrlWithoutBase => "relative URL without a base",
        url::ParseError::RelativeUrlWithCannotBeABaseBase => {
            "relative URL with a cannot-be-a-base base"
        }
        url::ParseError::SetHostOnCannotBeABaseUrl => {
            "a cannot-be-a-base URL doesn't have a host to set"
        }
        url::ParseError::Overflow => "URLs more than 4GB are not supported",
        _ => "invalid URL",
    }
}

/// Parse a string the way CPython's `float()` does.
fn python_float(text: &str) -> Option<f64> {
    let trimmed = pytext::trim(text);
    let cleaned = remove_python_underscores(trimmed)?;
    cleaned.parse::<f64>().ok()
}

/// Remove the underscores CPython's `float()` accepts between digits.
fn remove_python_underscores(text: &str) -> Option<String> {
    if !text.contains('_') {
        return Some(text.to_owned());
    }
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (index, character) in characters.iter().enumerate() {
        if *character == '_' {
            let before = index
                .checked_sub(1)
                .and_then(|position| characters.get(position));
            let after = characters.get(index + 1);
            match (before, after) {
                (Some(before), Some(after))
                    if before.is_ascii_digit() && after.is_ascii_digit() => {}
                _ => return None,
            }
            continue;
        }
        out.push(*character);
    }
    Some(out)
}

/// The per-user home directory on the current platform.
fn home_directory() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    match (
        std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty()),
        std::env::var_os("HOMEPATH").filter(|value| !value.is_empty()),
    ) {
        (Some(drive), Some(path)) => {
            let mut home = PathBuf::from(drive);
            home.push(path);
            Some(home)
        }
        _ => None,
    }
}

/// Expand a leading `~` the way `Path.expanduser` does.
fn expand_user(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" {
        return home_directory();
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return home_directory().map(|home| home.join(rest));
    }
    Some(path.to_path_buf())
}

/// Resolve a path the way `Path.resolve(strict=False)` does.
///
/// The existing prefix is resolved through the filesystem, so a symlink is
/// followed, and the rest of the path is appended and normalized.
fn resolve_path(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut prefix = absolute.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(resolved) = fs::canonicalize(&prefix) {
            let mut out = resolved;
            for component in rest.iter().rev() {
                out.push(component);
            }
            return Ok(normalize_components(&out));
        }
        let Some(name) = prefix.file_name().map(std::ffi::OsString::from) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the path cannot be resolved",
            ));
        };
        let Some(parent) = prefix.parent().map(Path::to_path_buf) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the path cannot be resolved",
            ));
        };
        rest.push(name);
        prefix = parent;
    }
}

/// Remove the `.` and `..` components of a resolved path.
fn normalize_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, SettingValue)]) -> BTreeMap<String, SettingValue> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect()
    }

    fn text(value: &str) -> SettingValue {
        SettingValue::Text(value.to_owned())
    }

    #[test]
    fn the_defaults_are_the_python_defaults() {
        let settings = settings_from_values(&BTreeMap::new()).unwrap();
        assert_eq!(settings.model_provider, "local");
        assert_eq!(settings.model_id, "local");
        assert_eq!(settings.model_endpoint, None);
        assert_eq!(settings.request_timeout_seconds, 120.0);
        assert_eq!(settings.tool_timeout_seconds, 300.0);
        assert_eq!(settings.worker_count, 4);
    }

    #[test]
    fn the_environment_is_the_only_source() {
        let mut environ = BTreeMap::new();
        environ.insert("KALA_AGENT_WORKER_COUNT".to_owned(), "2".to_owned());
        environ.insert("OPENAI_API_KEY".to_owned(), "secret".to_owned());
        let settings = settings_from_env(&environ).unwrap();
        assert_eq!(settings.worker_count, 2);
        assert!(!format!("{settings:?}").contains("secret"));
    }

    #[test]
    fn the_provider_is_normalized() {
        let settings =
            settings_from_values(&values(&[("model_provider", text("  LOCAL  "))])).unwrap();
        assert_eq!(settings.model_provider, "local");
    }

    #[test]
    fn the_provider_pattern_is_enforced() {
        let errors = settings_from_values(&values(&[("model_provider", text("a b"))])).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].kind, error_kind::VALUE_ERROR);
        assert_eq!(
            errors[0].message,
            "Value error, model_provider contains unsupported characters"
        );
    }

    #[test]
    fn the_string_bounds_are_enforced() {
        let errors = settings_from_values(&values(&[("model_provider", text("  "))])).unwrap_err();
        assert_eq!(errors[0].kind, error_kind::STRING_TOO_SHORT);
        assert_eq!(errors[0].message, "String should have at least 1 character");
        let errors = settings_from_values(&values(&[("model_provider", text(&"a".repeat(65)))]))
            .unwrap_err();
        assert_eq!(errors[0].kind, error_kind::STRING_TOO_LONG);
        assert_eq!(
            errors[0].message,
            "String should have at most 64 characters"
        );
        let errors =
            settings_from_values(&values(&[("model_id", text(&"a".repeat(257)))])).unwrap_err();
        assert_eq!(
            errors[0].message,
            "String should have at most 256 characters"
        );
    }

    #[test]
    fn the_worker_count_is_bounded() {
        let errors = settings_from_values(&values(&[("worker_count", SettingValue::Integer(9))]))
            .unwrap_err();
        assert_eq!(errors[0].kind, error_kind::LESS_THAN_EQUAL);
        assert_eq!(errors[0].message, "Input should be less than or equal to 8");
        let errors = settings_from_values(&values(&[("worker_count", SettingValue::Integer(0))]))
            .unwrap_err();
        assert_eq!(errors[0].kind, error_kind::GREATER_THAN_EQUAL);
        assert_eq!(
            errors[0].message,
            "Input should be greater than or equal to 1"
        );
        let settings = settings_from_values(&values(&[("worker_count", text(" 4 "))])).unwrap();
        assert_eq!(settings.worker_count, 4);
        let errors =
            settings_from_values(&values(&[("worker_count", SettingValue::Boolean(true))]))
                .unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, worker_count must be an integer"
        );
    }

    #[test]
    fn the_timeouts_are_finite_and_nonnegative() {
        let errors = settings_from_values(&values(&[(
            "request_timeout_seconds",
            SettingValue::Float(f64::INFINITY),
        )]))
        .unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, timeouts must be finite and nonnegative"
        );
        let errors = settings_from_values(&values(&[(
            "request_timeout_seconds",
            SettingValue::Boolean(false),
        )]))
        .unwrap_err();
        assert_eq!(errors[0].message, "Value error, timeouts must be numeric");
        let settings =
            settings_from_values(&values(&[("request_timeout_seconds", text("12.5"))])).unwrap();
        assert_eq!(settings.request_timeout_seconds, 12.5);
        let settings =
            settings_from_values(&values(&[("request_timeout_seconds", text("1_000"))])).unwrap();
        assert_eq!(settings.request_timeout_seconds, 1000.0);
    }

    #[test]
    fn the_endpoint_is_validated_and_normalized() {
        let settings =
            settings_from_values(&values(&[("model_endpoint", text("http://example.com"))]))
                .unwrap();
        assert_eq!(
            settings.model_endpoint.as_deref(),
            Some("http://example.com/")
        );
        let errors = settings_from_values(&values(&[(
            "model_endpoint",
            text("https://user:password@example.com/v1"),
        )]))
        .unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, model_endpoint cannot contain credentials"
        );
        let errors = settings_from_values(&values(&[(
            "model_endpoint",
            text("https://example.com/v1?api_key=secret"),
        )]))
        .unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, model_endpoint cannot contain query parameters or fragments"
        );
        let errors =
            settings_from_values(&values(&[("model_endpoint", text("/relative"))])).unwrap_err();
        assert_eq!(errors[0].kind, error_kind::URL_PARSING);
        assert_eq!(
            errors[0].message,
            "Input should be a valid URL, relative URL without a base"
        );
        let errors =
            settings_from_values(&values(&[("model_endpoint", text("ftp://example.com/v1"))]))
                .unwrap_err();
        assert_eq!(errors[0].kind, error_kind::URL_SCHEME);
        let errors = settings_from_values(&values(&[("model_endpoint", SettingValue::Integer(5))]))
            .unwrap_err();
        assert_eq!(errors[0].kind, error_kind::URL_TYPE);
    }

    #[test]
    fn extra_fields_are_refused() {
        let errors =
            settings_from_values(&values(&[("nope", SettingValue::Integer(1))])).unwrap_err();
        assert_eq!(errors[0].kind, error_kind::EXTRA_FORBIDDEN);
        assert_eq!(errors[0].location, "nope");
        assert_eq!(errors[0].message, "Extra inputs are not permitted");
    }

    #[test]
    fn the_data_dir_must_be_absolute_and_resolvable() {
        let errors =
            settings_from_values(&values(&[("data_dir", text("relative/path"))])).unwrap_err();
        assert_eq!(errors[0].message, "Value error, data_dir must be absolute");
        let errors = settings_from_values(&values(&[("data_dir", text("/"))])).unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, data_dir cannot be a filesystem root"
        );
        let errors = settings_from_values(&values(&[("data_dir", text("/tmp/a\0b"))])).unwrap_err();
        assert_eq!(
            errors[0].message,
            "Value error, data_dir contains an invalid character"
        );
        let errors =
            settings_from_values(&values(&[("data_dir", SettingValue::Integer(5))])).unwrap_err();
        assert_eq!(errors[0].message, "Value error, data_dir must be a path");
    }
}
