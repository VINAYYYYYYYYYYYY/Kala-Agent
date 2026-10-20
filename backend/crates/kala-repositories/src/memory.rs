//! The validated, immutable memory record stored by [`crate::MemoryRepository`].
//!
//! This is a native counterpart of
//! `kala_agent.persistence.repositories.MemoryRecord`. It is not part of the
//! accepted `kala-contracts` crate because the Python source defines it inside
//! the repository module. It deliberately implements the accepted
//! [`ContractModel`] trait so model JSON enters and leaves through the same
//! Pydantic-compatible parser/validator as the other stored models:
//!
//! * `extra="forbid"` and immutable values,
//! * `Identifier` normalization for `memory_id`, `source` and
//!   `provenance_id`,
//! * Pydantic `NonEmptyText` trimming, 1..=16,384 code-point content and
//!   surrogate refusal,
//! * canonical JSON object metadata in the unbounded persistence domain,
//! * the `expires_at > created_at` cross-field invariant compared as absolute
//!   instants.

use kala_contracts::{
    CanonicalJsonObject, ContractError, ContractIssue, ContractModel, Identifier, Text, Timestamp,
    WireValue,
};
use kala_json::{CanonicalObject, CanonicalValue, JsonText, Limits};

/// The `MemoryRecord` model name used in validation errors.
pub const MEMORY_RECORD: &str = "MemoryRecord";

const MAX_CONTENT_LENGTH: usize = 16_384;

/// One validated, immutable durable memory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRecord {
    memory_id: Identifier,
    content: Text,
    source: Identifier,
    provenance_id: Option<Identifier>,
    metadata_json: CanonicalJsonObject,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
}

impl MemoryRecord {
    /// Build and validate a memory record.
    ///
    /// `content` is trimmed with Pydantic's `strip_whitespace=True` semantics
    /// and must contain 1..=16,384 non-surrogate code points. An expiry must be
    /// strictly later than `created_at` as an absolute instant.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_id: &str,
        content: impl Into<Text>,
        source: &str,
        provenance_id: Option<&str>,
        metadata_json: CanonicalJsonObject,
        created_at: Timestamp,
        expires_at: Option<Timestamp>,
    ) -> Result<Self, ContractError> {
        let memory_id = Identifier::new(memory_id)?;
        let content = validated_content(content.into())?;
        let source = Identifier::new(source)?;
        let provenance_id = provenance_id.map(Identifier::new).transpose()?;
        if let Some(expires_at) = &expires_at {
            if expires_at <= &created_at {
                return Err(ContractError::at(
                    MEMORY_RECORD,
                    "expires_at",
                    "expires_at must be later than created_at",
                ));
            }
        }
        Ok(Self {
            memory_id,
            content,
            source,
            provenance_id,
            metadata_json,
            created_at,
            expires_at,
        })
    }

    /// The normalized memory identifier.
    pub fn memory_id(&self) -> &str {
        self.memory_id.as_str()
    }

    /// The trimmed, validated content.
    pub fn content(&self) -> &Text {
        &self.content
    }

    /// The normalized source identifier.
    pub fn source(&self) -> &str {
        self.source.as_str()
    }

    /// The optional normalized provenance identifier.
    pub fn provenance_id(&self) -> Option<&str> {
        self.provenance_id.as_ref().map(Identifier::as_str)
    }

    /// The canonical JSON metadata object.
    pub fn metadata_json(&self) -> &CanonicalJsonObject {
        &self.metadata_json
    }

    /// The creation instant.
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// The optional expiry instant.
    pub fn expires_at(&self) -> Option<&Timestamp> {
        self.expires_at.as_ref()
    }
}

impl ContractModel for MemoryRecord {
    const NAME: &'static str = MEMORY_RECORD;

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = match value {
            CanonicalValue::Object(map) => map,
            _ => {
                return Err(ContractError::new(
                    MEMORY_RECORD,
                    "Input should be a valid dictionary or object to extract fields from",
                ));
            }
        };
        let mut issues = Vec::new();
        let memory_id = required(map, "memory_id", &mut issues, as_identifier);
        let content = required(map, "content", &mut issues, as_nonempty_text);
        let source = required(map, "source", &mut issues, as_identifier);
        let provenance_id = optional(map, "provenance_id", &mut issues, as_identifier);
        let metadata_json = defaulted(
            map,
            "metadata_json",
            &mut issues,
            CanonicalJsonObject::empty,
            as_canonical_object,
        );
        let created_at = required(map, "created_at", &mut issues, as_timestamp);
        let expires_at = optional(map, "expires_at", &mut issues, as_timestamp);
        reject_extra(
            map,
            &[
                "memory_id",
                "content",
                "source",
                "provenance_id",
                "metadata_json",
                "created_at",
                "expires_at",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            if let (Some(created_at), Some(expires_at)) = (&created_at, &expires_at) {
                if expires_at <= created_at {
                    issues.push(ContractIssue::new(
                        "expires_at",
                        "expires_at must be later than created_at",
                    ));
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(MEMORY_RECORD, issues));
        }
        Ok(Self {
            memory_id: memory_id.expect("validated"),
            content: content.expect("validated"),
            source: source.expect("validated"),
            provenance_id,
            metadata_json,
            created_at: created_at.expect("validated"),
            expires_at,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("memory_id", text(self.memory_id.as_str())),
            ("content", WireValue::Text(self.content.clone())),
            ("source", text(self.source.as_str())),
            (
                "provenance_id",
                self.provenance_id
                    .as_ref()
                    .map_or(WireValue::Null, |identifier| text(identifier.as_str())),
            ),
            ("metadata_json", text(self.metadata_json.as_str())),
            ("created_at", timestamp(&self.created_at)),
            (
                "expires_at",
                self.expires_at.as_ref().map_or(WireValue::Null, timestamp),
            ),
        ])
    }
}

fn text(value: &str) -> WireValue {
    WireValue::Text(Text::new(value))
}

fn timestamp(value: &Timestamp) -> WireValue {
    WireValue::Text(Text::new(&value.to_wire_string()))
}

fn key(field: &str) -> JsonText {
    JsonText::new(field)
}

fn push(issues: &mut Vec<ContractIssue>, field: &str, message: impl Into<String>) {
    issues.push(ContractIssue::new(field, message));
}

fn required<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
        None => {
            push(issues, field, "Field required");
            None
        }
    }
}

fn optional<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
    }
}

fn defaulted<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    default: impl FnOnce() -> T,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> T {
    match map.get(&key(field)) {
        None => default(),
        Some(value) => match parse(value) {
            Ok(value) => value,
            Err(message) => {
                push(issues, field, message);
                default()
            }
        },
    }
}

fn reject_extra(map: &CanonicalObject, allowed: &[&str], issues: &mut Vec<ContractIssue>) {
    for name in map.keys() {
        let rendered = name.to_string_lossy();
        if !allowed.contains(&rendered.as_str()) {
            push(issues, &rendered, "Extra inputs are not permitted");
        }
    }
}

fn as_text(value: &CanonicalValue) -> Option<Text> {
    value
        .as_text()
        .map(|text| Text::from_json_text(text.clone()))
}

fn as_identifier(value: &CanonicalValue) -> Result<Identifier, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    let stripped = text.python_strip();
    let utf8 = stripped
        .to_utf8()
        .map_err(|_| "identifier must be nonempty and contain only safe characters".to_owned())?;
    Identifier::new(&utf8).map_err(|error| error.message().to_owned())
}

fn as_nonempty_text(value: &CanonicalValue) -> Result<Text, String> {
    let text = as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
    if text.has_surrogates() {
        return Err(
            "Input should be a valid string, unable to parse raw data as a unicode string"
                .to_owned(),
        );
    }
    let stripped = text.pydantic_strip();
    if stripped.is_empty() {
        return Err("String should have at least 1 character".to_owned());
    }
    if stripped.len() > MAX_CONTENT_LENGTH {
        return Err("String should have at most 16384 characters".to_owned());
    }
    Ok(stripped)
}

fn as_canonical_object(value: &CanonicalValue) -> Result<CanonicalJsonObject, String> {
    CanonicalJsonObject::from_value(value).map_err(|error| error.message().to_owned())
}

fn as_timestamp(value: &CanonicalValue) -> Result<Timestamp, String> {
    Timestamp::from_value(value).map_err(|error| error.message().to_owned())
}

fn validated_content(content: Text) -> Result<Text, ContractError> {
    if content.has_surrogates() {
        return Err(ContractError::at(
            MEMORY_RECORD,
            "content",
            "Input should be a valid string, unable to parse raw data as a unicode string",
        ));
    }
    let stripped = content.pydantic_strip();
    if stripped.is_empty() {
        return Err(ContractError::at(
            MEMORY_RECORD,
            "content",
            "String should have at least 1 character",
        ));
    }
    if stripped.len() > MAX_CONTENT_LENGTH {
        return Err(ContractError::at(
            MEMORY_RECORD,
            "content",
            "String should have at most 16384 characters",
        ));
    }
    Ok(stripped)
}

/// The persistence-domain JSON limits used by the metadata object.
///
/// This is exposed for tests and consumers that want to state explicitly that
/// memory metadata is not subject to the bounded tool-protocol defaults.
pub const fn memory_metadata_limits() -> Limits {
    kala_contracts::model_json_limits()
}
