//! The model gateway error hierarchy.
//!
//! The Python module raises nine public exception classes:
//!
//! ```text
//! ModelError(RuntimeError)
//! ├── ModelRequestError(ModelError, ValueError)
//! │   └── ModelCapabilityError(ModelRequestError)
//! ├── StructuredOutputError(ModelError, ValueError)
//! ├── ModelProviderError(ModelError)
//! │   ├── ProviderUnavailableError(ModelProviderError)
//! │   └── ProviderSubstitutionError(ModelProviderError)
//! ├── EgressDeniedError(ModelError, PermissionError)
//! └── TranscriptError(ModelError)
//! ```
//!
//! Rust has no exception hierarchy, so the classes become the variants of one
//! enum. Every variant keeps the exact Python message text and
//! [`ModelError::python_class_name`] reports the class the Python
//! implementation would have raised, so a differential oracle can compare the
//! class name as well as the message.

use std::fmt;

use kala_budget::BudgetError;
use kala_kernel::policy::PolicyError;

/// Every failure raised by the model gateway layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// `ModelRequestError`: an invalid request, message, artifact or response
    /// field.
    Request(String),
    /// `ModelCapabilityError`: an unknown or undeclared model capability.
    Capability(String),
    /// `StructuredOutputError`: a model answer that does not satisfy the
    /// declared response schema.
    Structured(String),
    /// `ModelProviderError`: an unusable provider answer.
    Provider(String),
    /// `ProviderUnavailableError`: the provider could not be reached.
    Unavailable(String),
    /// `ProviderDegradedError`: the provider became unavailable and the run
    /// degrades to the deterministic path without substituting a provider.
    Degraded {
        /// The provider that was configured.
        provider_id: String,
        /// The model that was configured.
        model_id: String,
        /// The reason the provider is unavailable.
        reason: String,
        /// The rendered exception message.
        message: String,
    },
    /// `ProviderSubstitutionError`: the provider answered for a different
    /// provider or model than the one that was configured.
    Substitution(String),
    /// `EgressDeniedError`: the egress policy refused the provider call.
    EgressDenied(String),
    /// `TranscriptError`: the transcript could not be persisted.
    Transcript(String),
}

impl ModelError {
    /// Build a `ModelRequestError`.
    pub fn request(message: impl Into<String>) -> Self {
        Self::Request(message.into())
    }

    /// Build a `ModelCapabilityError`.
    pub fn capability(message: impl Into<String>) -> Self {
        Self::Capability(message.into())
    }

    /// Build a `StructuredOutputError`.
    pub fn structured(message: impl Into<String>) -> Self {
        Self::Structured(message.into())
    }

    /// Build a `ModelProviderError`.
    pub fn provider(message: impl Into<String>) -> Self {
        Self::Provider(message.into())
    }

    /// Build a `ProviderUnavailableError`.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable(message.into())
    }

    /// Build a `ProviderDegradedError`.
    pub fn degraded(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let provider_id = provider_id.into();
        let model_id = model_id.into();
        let reason = reason.into();
        let message = format!(
            "provider {provider_id} model {model_id} is unavailable: {reason}; \
             no substitute provider is used"
        );
        Self::Degraded {
            provider_id,
            model_id,
            reason,
            message,
        }
    }

    /// The provider that was configured, for a `ProviderDegradedError`.
    #[must_use]
    pub fn degraded_provider_id(&self) -> Option<&str> {
        match self {
            Self::Degraded { provider_id, .. } => Some(provider_id),
            _ => None,
        }
    }

    /// The model that was configured, for a `ProviderDegradedError`.
    #[must_use]
    pub fn degraded_model_id(&self) -> Option<&str> {
        match self {
            Self::Degraded { model_id, .. } => Some(model_id),
            _ => None,
        }
    }

    /// The degradation reason, for a `ProviderDegradedError`.
    #[must_use]
    pub fn degraded_reason(&self) -> Option<&str> {
        match self {
            Self::Degraded { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// Build a `ProviderSubstitutionError`.
    pub fn substitution(message: impl Into<String>) -> Self {
        Self::Substitution(message.into())
    }

    /// Build an `EgressDeniedError`.
    pub fn egress_denied(message: impl Into<String>) -> Self {
        Self::EgressDenied(message.into())
    }

    /// Build a `TranscriptError`.
    pub fn transcript(message: impl Into<String>) -> Self {
        Self::Transcript(message.into())
    }

    /// The name of the Python exception class the port would have raised.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Request(_) => "ModelRequestError",
            Self::Capability(_) => "ModelCapabilityError",
            Self::Structured(_) => "StructuredOutputError",
            Self::Provider(_) => "ModelProviderError",
            Self::Unavailable(_) => "ProviderUnavailableError",
            Self::Degraded { .. } => "ProviderDegradedError",
            Self::Substitution(_) => "ProviderSubstitutionError",
            Self::EgressDenied(_) => "EgressDeniedError",
            Self::Transcript(_) => "TranscriptError",
        }
    }

    /// The exception message.
    pub fn message(&self) -> &str {
        match self {
            Self::Request(message)
            | Self::Capability(message)
            | Self::Structured(message)
            | Self::Provider(message)
            | Self::Unavailable(message)
            | Self::Substitution(message)
            | Self::EgressDenied(message)
            | Self::Transcript(message) => message,
            Self::Degraded { message, .. } => message,
        }
    }

    /// Whether the error is a `ModelRequestError` (and therefore also a
    /// `ModelCapabilityError` when the variant says so).
    pub fn is_request(&self) -> bool {
        matches!(self, Self::Request(_) | Self::Capability(_))
    }

    /// Whether the error is a `ModelProviderError`.
    pub fn is_provider(&self) -> bool {
        matches!(
            self,
            Self::Provider(_) | Self::Unavailable(_) | Self::Substitution(_)
        )
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Degraded {
                provider_id,
                model_id,
                reason,
                ..
            } => write!(
                formatter,
                "provider {provider_id} model {model_id} is unavailable: {reason}; \
                 no substitute provider is used"
            ),
            other => formatter.write_str(other.message()),
        }
    }
}

impl std::error::Error for ModelError {}

/// Every failure the gateway itself can raise.
///
/// The Python gateway lets three foreign exception families escape: the
/// budget errors raised by the ledger, the policy errors raised by the policy
/// engine and the model errors raised by the client. The Rust port keeps the
/// same three families so the error precedence is unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum GatewayError {
    /// A model error (`ModelError` and its subclasses).
    Model(ModelError),
    /// A budget error raised by the ledger (`BudgetExhaustedError` and
    /// friends).
    Budget(BudgetError),
    /// A policy error raised by the policy engine.
    Policy(PolicyError),
}

impl GatewayError {
    /// The name of the Python exception class the port would have raised.
    pub fn python_class_name(&self) -> &'static str {
        match self {
            Self::Model(error) => error.python_class_name(),
            Self::Budget(error) => error.contract_error_name(),
            Self::Policy(error) => error.python_class_name(),
        }
    }

    /// The exception message.
    pub fn message(&self) -> String {
        match self {
            Self::Model(error) => error.message().to_owned(),
            Self::Budget(error) => error.to_string(),
            Self::Policy(error) => error.message().to_owned(),
        }
    }

    /// The model error, when the failure came from the model layer.
    pub fn as_model(&self) -> Option<&ModelError> {
        match self {
            Self::Model(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for GatewayError {}

impl From<ModelError> for GatewayError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl From<BudgetError> for GatewayError {
    fn from(error: BudgetError) -> Self {
        Self::Budget(error)
    }
}

impl From<PolicyError> for GatewayError {
    fn from(error: PolicyError) -> Self {
        Self::Policy(error)
    }
}

/// The Python `ModelRequestError` class.
///
/// Rust has no exception hierarchy: the model errors are the variants of
/// [`ModelError`]. The alias keeps the Python name available to a caller
/// that migrates from the Python module.
pub type ModelRequestError = ModelError;

/// The Python `ModelCapabilityError` class.
pub type ModelCapabilityError = ModelError;

/// The Python `StructuredOutputError` class.
pub type StructuredOutputError = ModelError;

/// The Python `ModelProviderError` class.
pub type ModelProviderError = ModelError;

/// The Python `ProviderUnavailableError` class.
pub type ProviderUnavailableError = ModelError;

/// The Python `ProviderDegradedError` class.
pub type ProviderDegradedError = ModelError;

/// The Python `ProviderSubstitutionError` class.
pub type ProviderSubstitutionError = ModelError;

/// The Python `EgressDeniedError` class.
pub type EgressDeniedError = ModelError;

/// The Python `TranscriptError` class.
pub type TranscriptError = ModelError;
