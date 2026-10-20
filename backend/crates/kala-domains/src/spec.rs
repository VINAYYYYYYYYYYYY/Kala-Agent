//! Shared declaration types: parameter specifications, catalog steps,
//! identifiers, digests, capabilities and canonical digests.

use kala_budget::compat::clamp_unit_interval as budget_clamp_unit_interval;
use kala_contracts::{ApprovalKind, Sha256Digest, Text};
use kala_json::{CanonicalValue, Limits, canonical_dumps, canonical_json_digest};

use crate::error::DomainError;

/// Rounded digits used by every bounded score.
pub const ROUNDING_DIGITS: i32 = 6;
/// A safety factor used when the surrogate stress is exactly zero.
pub const UNBOUNDED_SAFETY_FACTOR: f64 = 1.0e6;
/// The finding every domain result carries.
pub const NON_AUTHORITATIVE_FINDING: &str = "Surrogate reference arithmetic only; not a certified analysis and not a substitute for a validated CAD or CAE model.";
/// The Python evaluator identity scheme this crate deliberately replaces.
pub const LEGACY_PYTHON_IMPLEMENTATION_DIGEST_VERSION: &str = "evaluator-implementation-digest-v2";
/// The native Rust evaluator identity scheme.
pub const NATIVE_IMPLEMENTATION_DIGEST_VERSION: &str = "evaluator-implementation-digest-v3-rust";
/// Native evaluator identity language marker.
pub const NATIVE_IMPLEMENTATION_LANGUAGE: &str = "rust";
/// Maximum declared implementation name length.
pub const MAX_IMPLEMENTATION_NAME_LENGTH: usize = 256;

/// Validate and normalize a contract identifier.
pub fn require_identifier(value: &str, field_name: &str) -> Result<String, DomainError> {
    kala_contracts::Identifier::new(value)
        .map(|identifier| identifier.as_str().to_owned())
        .map_err(|_| DomainError::Parameter(format!("{field_name} must be a safe identifier")))
}

/// Validate and normalize a SHA-256 digest.
pub fn require_digest(value: &str, field_name: &str) -> Result<String, DomainError> {
    let normalized = Text::new(value)
        .python_strip()
        .to_utf8()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Sha256Digest::from_hex(&normalized)
        .map(|digest| digest.to_hex())
        .ok_or_else(|| DomainError::Parameter(format!("{field_name} must be a sha-256 digest")))
}

/// Validate and normalize a lowercase declaration name.
pub fn require_name(value: &str, field_name: &str) -> Result<String, DomainError> {
    let normalized = Text::new(value)
        .python_strip()
        .to_utf8()
        .unwrap_or_default();
    if !is_declaration_name(&normalized) {
        return Err(DomainError::Parameter(format!(
            "{field_name} must be a lowercase declaration name"
        )));
    }
    Ok(normalized)
}

/// Python `not value.strip()` for a Rust string.
pub(crate) fn is_python_blank(value: &str) -> bool {
    Text::new(value).python_strip().is_empty()
}

fn is_declaration_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes[1..].iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.' | b'-')
    })
}

/// Return a sorted, duplicate-free tuple of capability identifiers.
///
/// The Rust signature requires a collection of string-like values, so the
/// Python refusal of a bare string is a compile-time property here rather than
/// a runtime error.
pub fn require_capabilities<I, S>(values: I, field_name: &str) -> Result<Vec<String>, DomainError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut capabilities = std::collections::BTreeSet::new();
    for value in values {
        capabilities.insert(require_identifier(value.as_ref(), field_name)?);
    }
    Ok(capabilities.into_iter().collect())
}

/// Parse a declared checkpoint kind string.
pub fn parse_checkpoint_kind(value: &str) -> Result<ApprovalKind, DomainError> {
    ApprovalKind::parse(value).ok_or_else(|| {
        DomainError::Parameter("a checkpoint kind must be a known approval kind".to_owned())
    })
}

/// Python `round(min(1.0, max(0.0, value)), 6)`.
pub fn clamp_unit_interval(value: f64) -> Result<f64, DomainError> {
    budget_clamp_unit_interval(value)
        .map_err(|_| DomainError::Parameter("bounded terms must be finite numbers".to_owned()))
}

/// Canonical JSON bytes for a declaration.
pub fn canonical_json(payload: &CanonicalValue) -> Result<String, DomainError> {
    canonical_dumps(payload, &Limits::default()).map_err(|error| {
        DomainError::Package(format!("a declaration cannot be canonicalized: {error}"))
    })
}

/// SHA-256 of the canonical JSON form of a declaration.
pub fn canonical_digest(payload: &CanonicalValue) -> Result<String, DomainError> {
    canonical_json_digest(payload)
        .map(|digest| digest.to_hex())
        .map_err(|error| {
            DomainError::Package(format!("a declaration cannot be canonicalized: {error}"))
        })
}

/// A declared parameter default value.
#[derive(Clone, Debug, PartialEq)]
pub enum ParameterDefault {
    /// A float default, serialized as a JSON float (`2.0` stays `2.0`).
    Float(f64),
    /// An integer default.
    Integer(i64),
    /// A string default such as a material or enum key.
    Text(String),
}

impl ParameterDefault {
    /// The Python `as_dict()`/`model_dump(mode="json")` value.
    pub fn to_canonical_value(&self) -> CanonicalValue {
        match self {
            Self::Float(value) => CanonicalValue::Float(*value),
            Self::Integer(value) => CanonicalValue::Integer((*value).into()),
            Self::Text(value) => CanonicalValue::text(value.as_str()),
        }
    }
}

/// A declared parameter with its unit, range, and default value.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterSpec {
    name: String,
    unit: String,
    minimum: f64,
    maximum: f64,
    default: ParameterDefault,
    description: String,
}

impl ParameterSpec {
    /// Build a parameter declaration.
    pub fn new(
        name: &str,
        unit: &str,
        minimum: f64,
        maximum: f64,
        default: ParameterDefault,
        description: &str,
    ) -> Result<Self, DomainError> {
        let name = require_name(name, "parameter name")?;
        if is_python_blank(unit) {
            return Err(DomainError::Parameter(
                "parameter units cannot be empty".to_owned(),
            ));
        }
        if is_python_blank(description) {
            return Err(DomainError::Parameter(
                "parameter descriptions cannot be empty".to_owned(),
            ));
        }
        if !minimum.is_finite() || !maximum.is_finite() {
            return Err(DomainError::Parameter(
                "parameter ranges must be finite".to_owned(),
            ));
        }
        if maximum < minimum {
            return Err(DomainError::Parameter(
                "a parameter maximum must not be below its minimum".to_owned(),
            ));
        }
        Ok(Self {
            name,
            unit: unit.to_owned(),
            minimum,
            maximum,
            default,
            description: description.to_owned(),
        })
    }

    /// The declaration name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The declared unit text.
    pub fn unit(&self) -> &str {
        &self.unit
    }
    /// The inclusive minimum.
    pub fn minimum(&self) -> f64 {
        self.minimum
    }
    /// The inclusive maximum.
    pub fn maximum(&self) -> f64 {
        self.maximum
    }
    /// The declared default.
    pub fn default_value(&self) -> &ParameterDefault {
        &self.default
    }
    /// The declared description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The Python `ParameterSpec.as_dict()` shape.
    pub fn as_canonical_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("default", self.default.to_canonical_value()),
            (
                "description",
                CanonicalValue::text(self.description.as_str()),
            ),
            ("maximum", CanonicalValue::Float(self.maximum)),
            ("minimum", CanonicalValue::Float(self.minimum)),
            ("name", CanonicalValue::text(self.name.as_str())),
            ("unit", CanonicalValue::text(self.unit.as_str())),
        ])
    }
}

/// A stable digest over a package parameter declaration table.
pub fn parameter_schema_digest(specs: &[ParameterSpec]) -> Result<String, DomainError> {
    let mut ordered: Vec<&ParameterSpec> = specs.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    let payload = CanonicalValue::Array(
        ordered
            .iter()
            .map(|spec| spec.as_canonical_value())
            .collect(),
    );
    canonical_digest(&payload)
}

/// A named workflow step offered by a domain package.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogStep {
    step_id: String,
    name: String,
    description: String,
    input_names: Vec<String>,
    metric_names: Vec<String>,
    required_capabilities: Vec<String>,
    estimated_duration_seconds: f64,
    checkpoint_kind: Option<ApprovalKind>,
    max_attempts: u32,
}

impl CatalogStep {
    /// Build a catalog step descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        step_id: &str,
        name: &str,
        description: &str,
        input_names: &[&str],
        metric_names: &[&str],
        required_capabilities: &[&str],
        estimated_duration_seconds: f64,
        checkpoint_kind: Option<ApprovalKind>,
        max_attempts: u32,
    ) -> Result<Self, DomainError> {
        let step_id = require_identifier(step_id, "catalog step_id")?;
        if is_python_blank(name) || is_python_blank(description) {
            return Err(DomainError::Parameter(
                "catalog steps need a name and a description".to_owned(),
            ));
        }
        for value in input_names {
            require_name(value, "catalog step input name")?;
        }
        for value in metric_names {
            require_name(value, "catalog step metric name")?;
        }
        if max_attempts < 1 {
            return Err(DomainError::Parameter(
                "catalog steps must allow at least one attempt".to_owned(),
            ));
        }
        if !estimated_duration_seconds.is_finite() {
            return Err(DomainError::Parameter(
                "catalog step duration must be finite".to_owned(),
            ));
        }
        if estimated_duration_seconds < 0.0 {
            return Err(DomainError::Parameter(
                "catalog step duration must be finite and nonnegative".to_owned(),
            ));
        }
        Ok(Self {
            step_id,
            name: name.to_owned(),
            description: description.to_owned(),
            input_names: input_names
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            metric_names: metric_names
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            required_capabilities: require_capabilities(
                required_capabilities.iter().copied(),
                "catalog step capability",
            )?,
            estimated_duration_seconds,
            checkpoint_kind,
            max_attempts,
        })
    }

    /// The catalog step identifier.
    pub fn step_id(&self) -> &str {
        &self.step_id
    }
    /// The human-readable name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The description.
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Declared parameter input names.
    pub fn input_names(&self) -> &[String] {
        &self.input_names
    }
    /// Declared metric names.
    pub fn metric_names(&self) -> &[String] {
        &self.metric_names
    }
    /// Declared required capabilities.
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }
    /// Declared estimated duration.
    pub fn estimated_duration_seconds(&self) -> f64 {
        self.estimated_duration_seconds
    }
    /// The declared checkpoint kind.
    pub fn checkpoint_kind(&self) -> Option<ApprovalKind> {
        self.checkpoint_kind
    }
    /// The maximum attempts.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Return a copy with a different declared capability set.
    pub fn with_required_capabilities(
        &self,
        required_capabilities: &[&str],
    ) -> Result<Self, DomainError> {
        let mut copy = self.clone();
        copy.required_capabilities = require_capabilities(
            required_capabilities.iter().copied(),
            "catalog step capability",
        )?;
        Ok(copy)
    }

    /// Return a copy with a different checkpoint kind.
    pub fn with_checkpoint_kind(&self, checkpoint_kind: Option<ApprovalKind>) -> Self {
        let mut copy = self.clone();
        copy.checkpoint_kind = checkpoint_kind;
        copy
    }

    /// The Python `CatalogStep.as_dict()` shape.
    pub fn as_canonical_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "checkpoint_kind",
                self.checkpoint_kind.map_or(CanonicalValue::Null, |kind| {
                    CanonicalValue::text(kind.as_str())
                }),
            ),
            (
                "description",
                CanonicalValue::text(self.description.as_str()),
            ),
            (
                "estimated_duration_seconds",
                CanonicalValue::Float(self.estimated_duration_seconds),
            ),
            (
                "input_names",
                CanonicalValue::Array(
                    self.input_names
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_str()))
                        .collect(),
                ),
            ),
            (
                "max_attempts",
                CanonicalValue::Integer(self.max_attempts.into()),
            ),
            (
                "metric_names",
                CanonicalValue::Array(
                    self.metric_names
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_str()))
                        .collect(),
                ),
            ),
            ("name", CanonicalValue::text(self.name.as_str())),
            (
                "required_capabilities",
                CanonicalValue::Array(
                    self.required_capabilities
                        .iter()
                        .map(|value| CanonicalValue::text(value.as_str()))
                        .collect(),
                ),
            ),
            ("step_id", CanonicalValue::text(self.step_id.as_str())),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_names_match_python_rules() {
        assert_eq!(
            require_name("  plate.length ", "name").unwrap(),
            "plate.length"
        );
        assert_eq!(require_name("a", "name").unwrap(), "a");
        assert!(require_name("Not A Name", "name").is_err());
        assert!(require_name("", "name").is_err());
        assert!(require_name(&format!("a{}", "b".repeat(64)), "name").is_err());
    }

    #[test]
    fn digests_are_normalized_and_local_identifier_errors_match() {
        let digest = " A".to_owned() + &"0".repeat(63) + " ";
        assert_eq!(
            require_digest(&digest, "d").unwrap(),
            "a".to_owned() + &"0".repeat(63)
        );
        assert_eq!(
            require_identifier("not a subject", "subject_id")
                .unwrap_err()
                .message(),
            "subject_id must be a safe identifier"
        );
    }
}
