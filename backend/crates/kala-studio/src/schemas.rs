//! Request DTOs and Pydantic-compatible validation for the Studio routes.
//!
//! The Python surface uses Pydantic v2 with `extra="forbid"` and
//! `str_strip_whitespace=True` on request models. Plain string fields are
//! stripped before their constraints, enum fields are matched against the
//! raw JSON string, parameter keys are stripped, and every rejected request
//! becomes the flat `invalid_request` envelope with bounded detail strings.

use serde_json::{Map, Value};

use crate::errors::{
    FlatError, MAX_VALIDATION_DETAILS, VALIDATION_MESSAGE, safe_detail, truncate_chars,
};
use crate::json_input::{
    JsonInputError, NormalisedJson, decode_json_bytes, normalise_json, original_offset,
};

#[cfg(feature = "native-primitives")]
pub const MAX_PARAMETERS_ENTRIES: usize = 32;
#[cfg(feature = "native-primitives")]
pub const MAX_PARAMETER_KEY_LENGTH: usize = 64;
#[cfg(feature = "native-primitives")]
pub const MAX_PARAMETER_STRING_VALUE_LENGTH: usize = 128;
pub const MAX_PROMPT_LENGTH: usize = 4_096;
pub const MAX_MODEL_ID_LENGTH: usize = 256;
pub const MAX_CAD_PROVIDER_LENGTH: usize = 64;
pub const INTEGRATION_ID_PATTERN: &str = r"^[a-z0-9][a-z0-9._-]{0,63}$";
pub const MODEL_ID_PATTERN: &str = r"^[A-Za-z0-9._:/-]{1,256}$";
pub const CAD_PROVIDER_PATTERN: &str = r"^[a-z0-9][a-z0-9._-]{0,63}$";
const MAX_INTEGER_DIGITS: usize = 4_300;

/// The primitive kinds the Studio mesh generator supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimitiveKind {
    Box,
    Cylinder,
    Sphere,
}

impl PrimitiveKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Box => "box",
            Self::Cylinder => "cylinder",
            Self::Sphere => "sphere",
        }
    }

    #[cfg(feature = "native-primitives")]
    fn from_variant(value: &str) -> Option<Self> {
        match value {
            "box" => Some(Self::Box),
            "cylinder" => Some(Self::Cylinder),
            "sphere" => Some(Self::Sphere),
            _ => None,
        }
    }
}

/// The export formats.
#[cfg(feature = "native-primitives")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Stl,
    Step,
}

#[cfg(feature = "native-primitives")]
impl ExportFormat {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stl => "stl",
            Self::Step => "step",
        }
    }

    fn from_variant(value: &str) -> Option<Self> {
        match value {
            "stl" => Some(Self::Stl),
            "step" => Some(Self::Step),
            _ => None,
        }
    }
}

/// Chat providers understood by the route.
///
/// `local` is the offline deterministic helper, `openai` and `openrouter`
/// are the API-key routes, and `claude_code`, `codex_cli` and `opencode_cli`
/// are the subscription routes that run the operator's own, already
/// authenticated command-line tool as a sandboxed subprocess. `codex` is the
/// historical id of the Codex CLI route and stays an alias of `codex_cli`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderId {
    Local,
    OpenAi,
    OpenRouter,
    Codex,
    ClaudeCode,
    CodexCli,
    OpenCodeCli,
}

impl ProviderId {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::CodexCli => "codex_cli",
            Self::OpenCodeCli => "opencode_cli",
        }
    }

    fn from_variant(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "openai" => Some(Self::OpenAi),
            "openrouter" => Some(Self::OpenRouter),
            "codex" => Some(Self::Codex),
            "claude_code" => Some(Self::ClaudeCode),
            "codex_cli" => Some(Self::CodexCli),
            "opencode_cli" => Some(Self::OpenCodeCli),
            _ => None,
        }
    }
}

/// Providers whose API keys can be configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialProviderId {
    OpenAi,
    OpenRouter,
}

impl CredentialProviderId {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
        }
    }

    fn from_variant(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::OpenAi),
            "openrouter" => Some(Self::OpenRouter),
            _ => None,
        }
    }
}

/// A validated preview request.
#[derive(Debug)]
#[cfg(feature = "native-primitives")]
pub struct PreviewRequest {
    pub kind: PrimitiveKind,
    pub parameters: Option<Value>,
}

/// A validated export request.
#[derive(Debug)]
#[cfg(feature = "native-primitives")]
pub struct ExportRequest {
    pub kind: PrimitiveKind,
    pub parameters: Option<Value>,
    pub format: ExportFormat,
}

/// A validated handoff request.
#[derive(Debug)]
#[cfg(feature = "native-primitives")]
pub struct HandoffRequest {
    pub integration_id: String,
    pub kind: PrimitiveKind,
    pub parameters: Option<Value>,
    pub format: ExportFormat,
}

/// A validated chat request.
#[derive(Debug)]
pub struct ChatRequest {
    pub provider: ProviderId,
    pub model: Option<String>,
    pub prompt: String,
    pub allow_remote: bool,
    /// The CAD provider the operator selected for this turn.
    ///
    /// ADR-0011 makes the choice explicit: `opencascade` is never selected
    /// automatically, so a turn without a provider asks which one to use
    /// instead of building a design.
    pub cad_provider: Option<String>,
}

/// A validated settings update.
#[derive(Debug)]
pub struct UpdateProviderKeyRequest {
    pub provider: CredentialProviderId,
    pub api_key: Option<String>,
}

/// A parsed body that could not become a request model.
#[derive(Debug)]
pub enum RequestParseError {
    /// Empty body or JSON `null`: Pydantic reports a missing body.
    Missing,
    /// A non-object top level: `model_attributes_type`.
    ModelAttributes,
    /// Malformed JSON: `body.<offset>: json_invalid: JSON decode error`.
    InvalidJson(usize),
    /// `json.loads` raised a non-JSON error (invalid UTF-8 or the integer
    /// digit limit); FastAPI answers 400 `request_rejected`.
    Rejected,
}

/// Parse an HTTP body the way FastAPI does for an `application/json` route.
///
/// # Errors
///
/// Returns [`RequestParseError`] instead of panicking on malformed input.
pub fn parse_json_body(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<Map<String, Value>, RequestParseError> {
    let json_media = content_type.is_some_and(is_json_media_type);
    if body.is_empty() {
        return Err(RequestParseError::Missing);
    }
    if !json_media {
        return Err(RequestParseError::ModelAttributes);
    }
    let text = match decode_json_bytes(body) {
        Ok(text) => text,
        Err(JsonInputError::Encoding) => return Err(RequestParseError::Rejected),
        Err(JsonInputError::Surrogate(offset)) => {
            // CPython's `surrogatepass` decodes the value and Pydantic then
            // rejects the lone surrogate. The native parser cannot put it in
            // a Rust string, but the source status is the same validation
            // 422; report the parser position for the detail.
            return Err(RequestParseError::InvalidJson(offset));
        }
        Err(JsonInputError::Syntax(offset)) => {
            return Err(RequestParseError::InvalidJson(offset));
        }
    };
    let normalised = match normalise_json(&text) {
        Ok(normalised) => normalised,
        Err(offset) => return Err(RequestParseError::InvalidJson(offset)),
    };
    let value: Value = match serde_json::from_slice(&normalised.bytes) {
        Ok(value) => value,
        Err(error) => {
            // CPython's `json.decoder` reports the position of its own first
            // error; the scanner reproduces that for the malformed classes
            // where serde's position differs (partial literals, invalid
            // escapes, malformed numbers, unterminated strings). A document
            // the scanner accepts (a lone surrogate escape) keeps the
            // serde-derived fallback.
            if let Some(offset) = crate::json_diagnostics::python_json_error_offset(&text) {
                return Err(RequestParseError::InvalidJson(offset));
            }
            return Err(RequestParseError::InvalidJson(error_offset(
                &normalised,
                &error,
            )));
        }
    };
    if has_overwide_integer(&value) {
        return Err(RequestParseError::Rejected);
    }
    match value {
        Value::Null => Err(RequestParseError::Missing),
        Value::Object(map) => Ok(map),
        _ => Err(RequestParseError::ModelAttributes),
    }
}

fn is_json_media_type(content_type: &str) -> bool {
    let media = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || media.ends_with("+json")
}

/// `serde_json` reports 1-based line/column in the normalised document;
/// Pydantic reports a 0-based character offset in the original decoded
/// document.
fn error_offset(normalised: &NormalisedJson, error: &serde_json::Error) -> usize {
    let message = error.to_string();
    if message.contains("EOF") {
        return normalised.original_chars;
    }
    // serde_json columns are 1-based byte positions in the current line.
    let body = &normalised.bytes;
    let line = error.line().max(1);
    let mut line_start = 0usize;
    for _ in 1..line {
        match body[line_start..].iter().position(|byte| *byte == b'\n') {
            Some(offset) => line_start += offset + 1,
            None => break,
        }
    }
    let mut index = (line_start + error.column().saturating_sub(1)).min(body.len());
    if message.contains("trailing comma") && index > line_start {
        // CPython points at the comma; serde points at the closing bracket.
        let mut cursor = index.saturating_sub(1);
        while cursor > line_start && body[cursor].is_ascii_whitespace() {
            cursor -= 1;
        }
        if body[cursor] == b',' {
            index = cursor;
        }
    }
    original_offset(normalised, index)
}

pub(crate) fn has_overwide_integer(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            let text = number.to_string();
            if text.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
                return false;
            }
            text.trim_start_matches('-').len() > MAX_INTEGER_DIGITS
        }
        Value::Array(values) => values.iter().any(has_overwide_integer),
        Value::Object(map) => map.values().any(has_overwide_integer),
        _ => false,
    }
}

struct Errors {
    details: Vec<String>,
}

impl Errors {
    fn new() -> Self {
        Self {
            details: Vec::new(),
        }
    }

    fn push(&mut self, location: String, kind: &str, message: &str) {
        if self.details.len() >= MAX_VALIDATION_DETAILS {
            return;
        }
        let message = safe_detail(message).unwrap_or("value rejected by validator");
        let message = truncate_chars(message, 96);
        self.details.push(format!("{location}: {kind}: {message}"));
    }

    fn finish(self) -> Option<Vec<String>> {
        if self.details.is_empty() {
            None
        } else {
            Some(self.details)
        }
    }
}

fn strip(value: &str) -> String {
    value.trim_matches(char::is_whitespace).to_owned()
}

fn enum_message(variants: &[&str]) -> String {
    let quoted: Vec<String> = variants
        .iter()
        .map(|variant| format!("'{variant}'"))
        .collect();
    match quoted.as_slice() {
        [one] => format!("Input should be {one}"),
        [first, second] => format!("Input should be {first} or {second}"),
        _ => {
            let (last, rest) = quoted.split_last().expect("non-empty variants");
            format!("Input should be {} or {last}", rest.join(", "))
        }
    }
}

fn required_enum<T: Copy>(
    map: &Map<String, Value>,
    name: &str,
    variants: &[&str],
    parse: impl Fn(&str) -> Option<T>,
    errors: &mut Errors,
) -> Option<T> {
    let Some(value) = map.get(name) else {
        errors.push(format!("body.{name}"), "missing", "Field required");
        return None;
    };
    if let Value::String(text) = value {
        if let Some(parsed) = parse(text) {
            return Some(parsed);
        }
    }
    errors.push(format!("body.{name}"), "enum", &enum_message(variants));
    None
}

fn required_string(
    map: &Map<String, Value>,
    name: &str,
    minimum: usize,
    maximum: usize,
    pattern: Option<&str>,
    errors: &mut Errors,
) -> Option<String> {
    let Some(value) = map.get(name) else {
        errors.push(format!("body.{name}"), "missing", "Field required");
        return None;
    };
    let Value::String(raw) = value else {
        errors.push(
            format!("body.{name}"),
            "string_type",
            "Input should be a valid string",
        );
        return None;
    };
    let text = strip(raw);
    let length = text.chars().count();
    if length < minimum {
        errors.push(
            format!("body.{name}"),
            "string_too_short",
            &format!(
                "String should have at least {minimum} character{}",
                if minimum == 1 { "" } else { "s" }
            ),
        );
        return None;
    }
    if length > maximum {
        errors.push(
            format!("body.{name}"),
            "string_too_long",
            &format!("String should have at most {maximum} characters"),
        );
        return None;
    }
    if let Some(pattern) = pattern {
        if !matches_pattern(&text, pattern) {
            errors.push(
                format!("body.{name}"),
                "string_pattern_mismatch",
                &format!("String should match pattern '{pattern}'"),
            );
            return None;
        }
    }
    Some(text)
}

fn optional_string(
    map: &Map<String, Value>,
    name: &str,
    maximum: usize,
    pattern: Option<&str>,
    errors: &mut Errors,
) -> Option<String> {
    match map.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            let text = strip(raw);
            if text.chars().count() > maximum {
                errors.push(
                    format!("body.{name}"),
                    "string_too_long",
                    &format!("String should have at most {maximum} characters"),
                );
                return None;
            }
            if let Some(pattern) = pattern {
                if !matches_pattern(&text, pattern) {
                    errors.push(
                        format!("body.{name}"),
                        "string_pattern_mismatch",
                        &format!("String should match pattern '{pattern}'"),
                    );
                    return None;
                }
            }
            Some(text)
        }
        Some(_) => {
            errors.push(
                format!("body.{name}"),
                "string_type",
                "Input should be a valid string",
            );
            None
        }
    }
}

fn matches_pattern(value: &str, pattern: &str) -> bool {
    // Only the fixed Studio patterns are needed here; all of them are simple
    // ASCII classes and anchors.
    if pattern == INTEGRATION_ID_PATTERN || pattern == CAD_PROVIDER_PATTERN {
        let mut chars = value.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return false;
        }
        if value.chars().count() > 64 {
            return false;
        }
        return chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        });
    }
    if pattern == MODEL_ID_PATTERN {
        return !value.is_empty()
            && value.chars().count() <= 256
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._:/-".contains(character));
    }
    false
}

/// The closed ADR-0011 set of CAD providers.
pub const CAD_PROVIDER_IDS: &[&str] = &["opencascade", "freecad", "solidedge"];

/// The optional CAD provider of a chat turn.
///
/// The set is closed, like the source's `CadProviderId` enum: any other
/// value is an enum refusal, not a pattern refusal.
fn optional_cad_provider(
    map: &Map<String, Value>,
    name: &str,
    errors: &mut Errors,
) -> Option<String> {
    match map.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            let text = strip(raw);
            if CAD_PROVIDER_IDS.contains(&text.as_str()) {
                Some(text)
            } else {
                errors.push(
                    format!("body.{name}"),
                    "enum",
                    &enum_message(CAD_PROVIDER_IDS),
                );
                None
            }
        }
        Some(_) => {
            errors.push(
                format!("body.{name}"),
                "enum",
                &enum_message(CAD_PROVIDER_IDS),
            );
            None
        }
    }
}

fn bool_field(map: &Map<String, Value>, name: &str, default: bool, errors: &mut Errors) -> bool {
    let Some(value) = map.get(name) else {
        return default;
    };
    match value {
        Value::Bool(value) => *value,
        Value::Number(number) => {
            let text = number.to_string();
            if let Ok(integer) = text.parse::<i64>() {
                match integer {
                    0 => return false,
                    1 => return true,
                    _ => {}
                }
            } else if let Ok(value) = text.parse::<f64>() {
                if value == 0.0 {
                    return false;
                }
                if value == 1.0 {
                    return true;
                }
            }
            errors.push(
                format!("body.{name}"),
                "bool_parsing",
                "Input should be a valid boolean, unable to interpret input",
            );
            default
        }
        Value::String(text) => match text.to_ascii_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => false,
            "1" | "on" | "t" | "true" | "y" | "yes" => true,
            _ => {
                errors.push(
                    format!("body.{name}"),
                    "bool_parsing",
                    "Input should be a valid boolean, unable to interpret input",
                );
                default
            }
        },
        _ => {
            errors.push(
                format!("body.{name}"),
                "bool_parsing",
                "Input should be a valid boolean, unable to interpret input",
            );
            default
        }
    }
}

#[cfg(feature = "native-primitives")]
fn parameters_field(map: &Map<String, Value>, name: &str, errors: &mut Errors) -> Option<Value> {
    let value = map.get(name)?;
    if value.is_null() {
        return None;
    }
    let Value::Object(object) = value else {
        errors.push(
            format!("body.{name}"),
            "dict_type",
            "Input should be a valid dictionary",
        );
        return None;
    };
    if object.len() > MAX_PARAMETERS_ENTRIES {
        errors.push(
            format!("body.{name}"),
            "value_error",
            &format!(
                "Value error, parameters cannot declare more than {MAX_PARAMETERS_ENTRIES} entries"
            ),
        );
        return None;
    }
    let mut clean = Map::new();
    for (key, entry) in object {
        let key = strip(key);
        if key.is_empty() || key.chars().count() > MAX_PARAMETER_KEY_LENGTH {
            errors.push(
                format!("body.{name}"),
                "value_error",
                "Value error, parameter names must be nonempty and bounded",
            );
            return None;
        }
        match entry {
            Value::Bool(_) => {
                errors.push(
                    format!("body.{name}"),
                    "value_error",
                    "Value error, parameter values must not be booleans",
                );
                return None;
            }
            Value::Number(_) => {}
            Value::String(text) => {
                if text.is_empty() || text.chars().count() > MAX_PARAMETER_STRING_VALUE_LENGTH {
                    errors.push(
                        format!("body.{name}"),
                        "value_error",
                        "Value error, string parameter values are bounded",
                    );
                    return None;
                }
            }
            _ => {
                errors.push(
                    format!("body.{name}"),
                    "value_error",
                    "Value error, parameter values must be numbers or bounded strings",
                );
                return None;
            }
        }
        clean.insert(key, entry.clone());
    }
    Some(Value::Object(clean))
}

fn report_extras(map: &Map<String, Value>, known: &[&str], errors: &mut Errors) {
    for key in map.keys() {
        if !known.contains(&key.as_str()) {
            errors.push(
                format!("body.{key}"),
                "extra_forbidden",
                "Extra inputs are not permitted",
            );
        }
    }
}

fn invalid(details: Vec<String>) -> FlatError {
    FlatError::new(422, "invalid_request", VALIDATION_MESSAGE, details)
}

#[cfg(feature = "native-primitives")]
/// Validate a preview request.
///
/// # Errors
///
/// Returns the bounded 422 envelope details.
pub fn validate_preview(map: &Map<String, Value>) -> Result<PreviewRequest, FlatError> {
    let mut errors = Errors::new();
    let kind = required_enum(
        map,
        "kind",
        &["box", "cylinder", "sphere"],
        PrimitiveKind::from_variant,
        &mut errors,
    );
    let parameters = parameters_field(map, "parameters", &mut errors);
    report_extras(map, &["kind", "parameters"], &mut errors);
    match (kind, errors.finish()) {
        (Some(kind), None) => Ok(PreviewRequest { kind, parameters }),
        (_, Some(details)) => Err(invalid(details)),
        (None, None) => Err(invalid(vec![])),
    }
}

#[cfg(feature = "native-primitives")]
/// Validate an export request.
///
/// # Errors
///
/// Returns the bounded 422 envelope details.
pub fn validate_export(map: &Map<String, Value>) -> Result<ExportRequest, FlatError> {
    let mut errors = Errors::new();
    let kind = required_enum(
        map,
        "kind",
        &["box", "cylinder", "sphere"],
        PrimitiveKind::from_variant,
        &mut errors,
    );
    let parameters = parameters_field(map, "parameters", &mut errors);
    let format = required_enum(
        map,
        "format",
        &["stl", "step"],
        ExportFormat::from_variant,
        &mut errors,
    );
    report_extras(map, &["kind", "parameters", "format"], &mut errors);
    match (kind, format, errors.finish()) {
        (Some(kind), Some(format), None) => Ok(ExportRequest {
            kind,
            parameters,
            format,
        }),
        (_, _, Some(details)) => Err(invalid(details)),
        _ => Err(invalid(vec![])),
    }
}

#[cfg(feature = "native-primitives")]
/// Validate a handoff request.
///
/// # Errors
///
/// Returns the bounded 422 envelope details.
pub fn validate_handoff(map: &Map<String, Value>) -> Result<HandoffRequest, FlatError> {
    let mut errors = Errors::new();
    let integration_id = required_string(
        map,
        "integration_id",
        1,
        64,
        Some(INTEGRATION_ID_PATTERN),
        &mut errors,
    );
    let kind = required_enum(
        map,
        "kind",
        &["box", "cylinder", "sphere"],
        PrimitiveKind::from_variant,
        &mut errors,
    );
    let parameters = parameters_field(map, "parameters", &mut errors);
    let format = required_enum(
        map,
        "format",
        &["stl", "step"],
        ExportFormat::from_variant,
        &mut errors,
    );
    report_extras(
        map,
        &["integration_id", "kind", "parameters", "format"],
        &mut errors,
    );
    match (integration_id, kind, format, errors.finish()) {
        (Some(integration_id), Some(kind), Some(format), None) => Ok(HandoffRequest {
            integration_id,
            kind,
            parameters,
            format,
        }),
        (_, _, _, Some(details)) => Err(invalid(details)),
        _ => Err(invalid(vec![])),
    }
}

/// Validate a chat request.
///
/// # Errors
///
/// Returns the bounded 422 envelope details.
pub fn validate_chat(map: &Map<String, Value>) -> Result<ChatRequest, FlatError> {
    let mut errors = Errors::new();
    let provider = required_enum(
        map,
        "provider",
        &[
            "local",
            "openai",
            "openrouter",
            "codex",
            "claude_code",
            "codex_cli",
            "opencode_cli",
        ],
        ProviderId::from_variant,
        &mut errors,
    );
    let model = optional_string(
        map,
        "model",
        MAX_MODEL_ID_LENGTH,
        Some(MODEL_ID_PATTERN),
        &mut errors,
    );
    let prompt = required_string(map, "prompt", 1, MAX_PROMPT_LENGTH, None, &mut errors);
    let allow_remote = bool_field(map, "allow_remote", false, &mut errors);
    let cad_provider = optional_cad_provider(map, "cad_provider", &mut errors);
    report_extras(
        map,
        &[
            "provider",
            "model",
            "prompt",
            "allow_remote",
            "cad_provider",
        ],
        &mut errors,
    );
    match (provider, prompt, errors.finish()) {
        (Some(provider), Some(prompt), None) => Ok(ChatRequest {
            provider,
            model,
            prompt,
            allow_remote,
            cad_provider,
        }),
        (_, _, Some(details)) => Err(invalid(details)),
        _ => Err(invalid(vec![])),
    }
}

/// Validate a settings update.
///
/// # Errors
///
/// Returns the bounded 422 envelope details.
pub fn validate_settings(map: &Map<String, Value>) -> Result<UpdateProviderKeyRequest, FlatError> {
    let mut errors = Errors::new();
    let provider = required_enum(
        map,
        "provider",
        &["openai", "openrouter"],
        CredentialProviderId::from_variant,
        &mut errors,
    );
    let api_key = match map.get("api_key") {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => {
            if raw.chars().count() > 256 {
                errors.push(
                    "body.api_key".to_owned(),
                    "string_too_long",
                    "String should have at most 256 characters",
                );
                None
            } else {
                Some(raw.clone())
            }
        }
        Some(_) => {
            errors.push(
                "body.api_key".to_owned(),
                "string_type",
                "Input should be a valid string",
            );
            None
        }
    };
    report_extras(map, &["provider", "api_key"], &mut errors);
    match (provider, errors.finish()) {
        (Some(provider), None) => Ok(UpdateProviderKeyRequest { provider, api_key }),
        (_, Some(details)) => Err(invalid(details)),
        _ => Err(invalid(vec![])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(text: &str) -> Map<String, Value> {
        match serde_json::from_str(text).unwrap() {
            Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[cfg(feature = "native-primitives")]
    #[test]
    fn enum_and_extra_errors_match_pydantic() {
        let error = validate_preview(&map(r#"{"kind":"torus","nope":1}"#)).unwrap_err();
        assert_eq!(
            error.details,
            vec![
                "body.kind: enum: Input should be 'box', 'cylinder' or 'sphere'".to_owned(),
                "body.nope: extra_forbidden: Extra inputs are not permitted".to_owned(),
            ]
        );
    }

    #[cfg(feature = "native-primitives")]
    #[test]
    fn parameter_keys_are_stripped_and_validated() {
        let request =
            validate_preview(&map(r#"{"kind":"box","parameters":{" size_x ":2}}"#)).unwrap();
        assert_eq!(
            request.parameters.unwrap()["size_x"],
            Value::Number(2.into())
        );
        let error =
            validate_preview(&map(r#"{"kind":"box","parameters":{"size_x":true}}"#)).unwrap_err();
        assert_eq!(
            error.details,
            vec![
                "body.parameters: value_error: Value error, parameter values must not be booleans"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn chat_prompt_is_stripped_and_boolean_aliases_work() {
        let request = validate_chat(&map(
            r#"{"provider":"local","prompt":" x ","allow_remote":"yes"}"#,
        ))
        .unwrap();
        assert_eq!(request.prompt, "x");
        assert!(request.allow_remote);
    }

    #[test]
    fn boolean_numbers_follow_pydantic() {
        for (value, expected) in [
            ("1.0", true),
            ("0.0", false),
            ("-0.0", false),
            ("1e0", true),
        ] {
            let request = validate_chat(&map(&format!(
                r#"{{"provider":"local","prompt":"x","allow_remote":{value}}}"#
            )))
            .unwrap();
            assert_eq!(request.allow_remote, expected, "value {value}");
        }
        assert!(
            validate_chat(&map(
                r#"{"provider":"local","prompt":"x","allow_remote":2.0}"#
            ))
            .is_err()
        );
    }

    #[cfg(feature = "native-primitives")]
    #[test]
    fn nonstandard_constants_preserve_the_finite_error_branch() {
        let map = parse_json_body(
            br#"{"kind":"box","parameters":{"size_z":NaN}}"#,
            Some("application/json"),
        )
        .unwrap();
        let request = validate_preview(&map).unwrap();
        let geometry = kala_geometry::build_primitive_json("box", request.parameters.as_ref());
        assert_eq!(
            geometry.unwrap_err().message(),
            "size_z must be a finite number"
        );
    }

    #[test]
    fn json_parse_errors_carry_the_pydantic_offset() {
        match parse_json_body(b"{", Some("application/json")) {
            Err(RequestParseError::InvalidJson(offset)) => assert_eq!(offset, 1),
            other => panic!("unexpected {other:?}"),
        }
        match parse_json_body(b"{\"kind\":\"box\"} trailing", Some("application/json")) {
            Err(RequestParseError::InvalidJson(offset)) => assert_eq!(offset, 15),
            other => panic!("unexpected {other:?}"),
        }
    }
}
