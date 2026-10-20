//! Domain failures and parameter-validation failures.
//!
//! The Python source has two different failure families and this crate keeps
//! them apart for the same reason:
//!
//! * [`DomainError`] maps the four `kala_agent.domains` exception classes
//!   (`DomainError`, `DomainParameterError`, `DomainPackageError`,
//!   `DomainStepError`, `DomainCapabilityError`).
//! * [`ParameterError`] maps a Pydantic `ValidationError` raised by one of the
//!   parameter models. Its message vocabulary follows the Pydantic messages
//!   the application summary exposes (`"<field>: <message>"`).

use std::fmt;

/// One of the four declarative-domain exception classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainError {
    /// `DomainParameterError`: a parameter or context value is outside its
    /// declared contract.
    Parameter(String),
    /// `DomainPackageError`: a package declaration is internally inconsistent.
    Package(String),
    /// `DomainStepError`: a catalog step is not declared by a package.
    Step(String),
    /// `DomainCapabilityError`: declared capabilities were not granted exactly.
    Capability(String),
}

impl DomainError {
    /// The Python exception class name this failure maps to.
    pub const fn class_name(&self) -> &'static str {
        match self {
            Self::Parameter(_) => "DomainParameterError",
            Self::Package(_) => "DomainPackageError",
            Self::Step(_) => "DomainStepError",
            Self::Capability(_) => "DomainCapabilityError",
        }
    }

    /// The exact message text carried by the Python exception.
    pub fn message(&self) -> &str {
        match self {
            Self::Parameter(message)
            | Self::Package(message)
            | Self::Step(message)
            | Self::Capability(message) => message,
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for DomainError {}

/// The validation class of one parameter failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterErrorKind {
    /// The model input was not a JSON object, or the document was not JSON.
    ModelType,
    /// The supplied JSON document could not be parsed.
    JsonInvalid,
    /// A field has the wrong JSON type.
    Type,
    /// A numeric string could not be parsed as an integer.
    IntegerParsing,
    /// A float had a fractional part where an integer is required.
    IntegerFromFloat,
    /// A numeric string could not be parsed as a float.
    FloatParsing,
    /// A numeric input was not finite where a finite number is required.
    FiniteNumber,
    /// A value was below an inclusive bound.
    GreaterThanEqual,
    /// A value was above an inclusive bound.
    LessThanEqual,
    /// An enum value was not declared.
    Enum,
    /// An object key is not a declared field.
    ExtraForbidden,
    /// A model-level (cross-field) validator failed.
    Value,
}

impl ParameterErrorKind {
    /// The Pydantic error type name where one exists.
    pub const fn pydantic_type(self) -> &'static str {
        match self {
            Self::ModelType => "model_type",
            Self::JsonInvalid => "json_invalid",
            Self::Type => "type_error",
            Self::IntegerParsing => "int_parsing",
            Self::IntegerFromFloat => "int_from_float",
            Self::FloatParsing => "float_parsing",
            Self::FiniteNumber => "finite_number",
            Self::GreaterThanEqual => "greater_than_equal",
            Self::LessThanEqual => "less_than_equal",
            Self::Enum => "enum",
            Self::ExtraForbidden => "extra_forbidden",
            Self::Value => "value_error",
        }
    }
}

/// A parameter-model validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParameterError {
    field: Option<String>,
    kind: ParameterErrorKind,
    message: String,
}

impl ParameterError {
    /// Build a field-level failure.
    pub fn field(
        field: impl Into<String>,
        kind: ParameterErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            field: Some(field.into()),
            kind,
            message: message.into(),
        }
    }

    /// Build a model-level failure. Pydantic prefixes the text with
    /// `"Value error, "`; `Application._summarize_validation` then presents it
    /// under `parameters`.
    pub fn model(message: impl Into<String>) -> Self {
        Self {
            field: None,
            kind: ParameterErrorKind::Value,
            message: format!("Value error, {}", message.into()),
        }
    }

    /// Build a document/root-level failure with no field location.
    pub fn root(kind: ParameterErrorKind, message: impl Into<String>) -> Self {
        Self {
            field: None,
            kind,
            message: message.into(),
        }
    }

    /// Build a JSON parse failure using Pydantic's `json_invalid` message
    /// prefix.
    pub fn invalid_json(detail: impl Into<String>) -> Self {
        Self::root(
            ParameterErrorKind::JsonInvalid,
            format!("Invalid JSON: {}", detail.into()),
        )
    }

    /// The failing field, or `None` for a model-level failure.
    pub fn field_name(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// The failure class.
    pub const fn kind(&self) -> ParameterErrorKind {
        self.kind
    }

    /// The Pydantic-compatible message text (without the `loc` prefix).
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The application-compatible `"<location>: <message>"` summary.
    pub fn summary(&self) -> String {
        let location = self.field.as_deref().unwrap_or("parameters");
        format!("{location}: {}", self.message)
    }
}

impl fmt::Display for ParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.summary())
    }
}

impl std::error::Error for ParameterError {}

/// A parameters-or-domain failure from the untyped registry dispatch surface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainCallError {
    /// Parameter validation failed before any evaluator ran.
    Parameters(ParameterError),
    /// A domain contract check failed.
    Domain(DomainError),
}

impl From<ParameterError> for DomainCallError {
    fn from(error: ParameterError) -> Self {
        Self::Parameters(error)
    }
}

impl From<DomainError> for DomainCallError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl fmt::Display for DomainCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameters(error) => fmt::Display::fmt(error, formatter),
            Self::Domain(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for DomainCallError {}
