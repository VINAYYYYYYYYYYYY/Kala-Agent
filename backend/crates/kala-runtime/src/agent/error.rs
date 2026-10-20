//! The agent error hierarchy.
//!
//! The Python module raises five public exception classes of its own and lets
//! four foreign families escape:
//!
//! ```text
//! AgentError(RuntimeError)
//! ├── AgentPlanError(AgentError, ValueError)
//! ├── AgentPolicyError(AgentError)
//! ├── AgentMergeError(AgentError)
//! └── PlannerFailure(AgentError)
//! ```
//!
//! The foreign families are the tool protocol errors (`JsonEnvelopeError`,
//! `SchemaValidationError`), the tool registry errors, the tool executor
//! errors, the budget errors of the ledger, the policy errors of the kernel
//! and the model errors of the gateway.
//!
//! Rust has no exception hierarchy, so every class becomes a variant of one
//! enum. Each variant keeps the exact Python message text and
//! [`AgentError::python_class_name`] reports the class the Python
//! implementation would have raised, so a differential oracle can compare the
//! class name as well as the message.

use std::fmt;

use kala_budget::BudgetError;
use kala_kernel::policy::PolicyError;
use kala_tools::executor::ToolExecutorError;
use kala_tools::protocol::ToolProtocolError;
use kala_tools::registry::ToolRegistryError;

use crate::gateway::ModelError;

/// Every failure raised by the agent runtime.
#[derive(Clone, Debug, PartialEq)]
pub enum AgentError {
    /// `AgentError`: the base class of the agent errors.
    Base(String),
    /// `AgentPlanError`: an invalid plan, task, catalog or objective.
    Plan(String),
    /// `AgentPolicyError`: a required policy decision that cannot be made.
    Policy(String),
    /// `AgentMergeError`: a merged result that cannot be assembled.
    Merge(String),
    /// `PlannerFailure`: a planner that cannot produce a plan.
    PlannerFailure(String),
    /// `JsonEnvelopeError`: a malformed or oversized JSON document.
    JsonEnvelope(String),
    /// `ValidationError`: a contract model that does not validate.
    Validation(String),
    /// A model gateway failure (`ModelError` and its subclasses).
    Model(ModelError),
    /// A budget failure (`BudgetError` and its subclasses).
    Budget(BudgetError),
    /// A policy engine failure (`PolicyError` and its subclasses).
    PolicyError(PolicyError),
    /// A tool registry failure (`ToolRegistryError` and its subclasses).
    Registry(ToolRegistryError),
    /// A tool executor failure (`ToolExecutorError` and its subclasses).
    Executor(ToolExecutorError),
    /// A tool protocol failure (`ToolProtocolError` and its subclasses).
    Protocol(ToolProtocolError),
}

impl AgentError {
    /// Build the base `AgentError`.
    pub fn base(message: impl Into<String>) -> Self {
        Self::Base(message.into())
    }

    /// Build an `AgentPlanError`.
    pub fn plan(message: impl Into<String>) -> Self {
        Self::Plan(message.into())
    }

    /// Build an `AgentPolicyError`.
    pub fn policy(message: impl Into<String>) -> Self {
        Self::Policy(message.into())
    }

    /// Build an `AgentMergeError`.
    pub fn merge(message: impl Into<String>) -> Self {
        Self::Merge(message.into())
    }

    /// Build a `PlannerFailure`.
    pub fn planner_failure(message: impl Into<String>) -> Self {
        Self::PlannerFailure(message.into())
    }

    /// The name of the Python exception class the port would have raised.
    #[must_use]
    pub fn python_class_name(&self) -> &'static str {
        match self {
            Self::Base(_) => "AgentError",
            Self::Plan(_) => "AgentPlanError",
            Self::Policy(_) => "AgentPolicyError",
            Self::Merge(_) => "AgentMergeError",
            Self::PlannerFailure(_) => "PlannerFailure",
            Self::JsonEnvelope(_) => "JsonEnvelopeError",
            Self::Validation(_) => "ValidationError",
            Self::Model(error) => error.python_class_name(),
            Self::Budget(error) => error.contract_error_name(),
            Self::PolicyError(error) => error.python_class_name(),
            Self::Registry(error) => error.python_class_name(),
            Self::Executor(error) => error.python_class_name(),
            Self::Protocol(error) => error.python_class_name(),
        }
    }

    /// The exception message.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Base(message)
            | Self::Plan(message)
            | Self::Policy(message)
            | Self::Merge(message)
            | Self::PlannerFailure(message)
            | Self::JsonEnvelope(message)
            | Self::Validation(message) => message.clone(),
            Self::Model(error) => error.message().to_owned(),
            Self::Budget(error) => error.to_string(),
            Self::PolicyError(error) => error.message().to_owned(),
            Self::Registry(error) => error.to_string(),
            Self::Executor(error) => error.message().to_owned(),
            Self::Protocol(error) => error.to_string(),
        }
    }

    /// Whether the failure is one of the four errors that never degrade to
    /// the deterministic planner.
    ///
    /// The Python module keeps the tuple `NEVER_DEGRADED_ERRORS`:
    /// `AgentPolicyError`, `BudgetExhaustedError`, `EgressDeniedError` and
    /// `ModelCapabilityError`. A budget exhaustion is matched on the *variant*
    /// and not on the class name, because the Python class name of a budget
    /// exhaustion is the dimension specific subclass
    /// (`ToolCallBudgetExhaustedError` and friends), all of which derive from
    /// `BudgetExhaustedError`.
    #[must_use]
    pub fn is_never_degraded(&self) -> bool {
        matches!(
            self,
            Self::Policy(_)
                | Self::Budget(BudgetError::Exhausted(_))
                | Self::Model(ModelError::EgressDenied(_))
                | Self::Model(ModelError::Capability(_))
        )
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for AgentError {}

/// The four errors that never degrade to the deterministic planner.
///
/// The Python module publishes the tuple
/// `NEVER_DEGRADED_ERRORS = (AgentPolicyError, BudgetExhaustedError,
/// EgressDeniedError, ModelCapabilityError)`. The Rust port publishes the
/// class names, so a caller can assert the same membership, and
/// [`AgentError::is_never_degraded`] decides the membership at runtime.
pub const NEVER_DEGRADED_ERRORS: [&str; 4] = [
    "AgentPolicyError",
    "BudgetExhaustedError",
    "EgressDeniedError",
    "ModelCapabilityError",
];

/// The Python `AgentError` class.
///
/// Rust has no exception hierarchy: the agent errors are the variants of
/// [`AgentError`]. The alias keeps the Python name available to a caller that
/// migrates from the Python module.
pub type AgentErrorBase = AgentError;

/// The Python `AgentPlanError` class.
pub type AgentPlanError = AgentError;

/// The Python `AgentPolicyError` class.
pub type AgentPolicyError = AgentError;

/// The Python `AgentMergeError` class.
pub type AgentMergeError = AgentError;

/// The Python `PlannerFailure` class.
pub type PlannerFailure = AgentError;
