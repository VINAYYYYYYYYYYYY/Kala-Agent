//! The outcome of one agent step and the result of one agent run.
//!
//! The two types mirror the Python dataclasses `AgentStepOutcome` and
//! `AgentRunResult`, including the order in which every field is validated
//! and the exact shape of the `to_payload()` documents.

use kala_contracts::{ToolResult, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

use super::error::AgentError;
use super::value::{
    MAX_ERROR_TEXT, bounded_identifier, bounded_reasons, bounded_text, freeze_object,
    identifier_tuple, thaw_object,
};

/// The outcome of one step of a run.
///
/// The outcome records whether the step was dispatched, what the tool
/// answered, and — when the step was refused — the error code and the
/// reason.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentStepOutcome {
    step_id: String,
    call_id: String,
    status: ToolRunStatus,
    dispatched: bool,
    tool_id: String,
    result: Option<CanonicalValue>,
    error_code: Option<String>,
    error_message: Option<String>,
    artifact_ids: Vec<String>,
    duration_ms: f64,
    policy_reason: Option<String>,
}

impl AgentStepOutcome {
    /// Build an outcome from every field at once.
    ///
    /// The fields are validated in the order of the Python `__post_init__`:
    /// `step_id`, `call_id`, `tool_id`, the status, the dispatch flag, the
    /// result, `artifact_ids`, `error_message` and `policy_reason`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        step_id: &str,
        call_id: &str,
        status: ToolRunStatus,
        dispatched: bool,
        tool_id: &str,
        result: Option<&CanonicalValue>,
        error_code: Option<&str>,
        error_message: Option<&str>,
        artifact_ids: &[String],
        duration_ms: f64,
        policy_reason: Option<&str>,
    ) -> Result<Self, AgentError> {
        let mut outcome = Self {
            step_id: step_id.to_owned(),
            call_id: call_id.to_owned(),
            status,
            dispatched,
            tool_id: tool_id.to_owned(),
            result: result.cloned(),
            error_code: error_code.map(str::to_owned),
            error_message: error_message.map(str::to_owned),
            artifact_ids: artifact_ids.to_vec(),
            duration_ms,
            policy_reason: policy_reason.map(str::to_owned),
        };
        outcome.validate()?;
        Ok(outcome)
    }

    /// Validate every field, in the order of the Python `__post_init__`.
    fn validate(&mut self) -> Result<(), AgentError> {
        self.step_id = bounded_identifier(&self.step_id, "step_id")?;
        self.call_id = bounded_identifier(&self.call_id, "call_id")?;
        self.tool_id = bounded_identifier(&self.tool_id, "tool_id")?;
        if let Some(result) = &self.result {
            if !matches!(result, CanonicalValue::Object(_)) {
                return Err(AgentError::merge(
                    "the outcome result must be a JSON object",
                ));
            }
        }
        self.artifact_ids = identifier_tuple(&self.artifact_ids, "artifact_ids")?;
        if let Some(message) = &self.error_message {
            self.error_message = Some(bounded_text(message, "error_message", MAX_ERROR_TEXT)?);
        }
        if let Some(reason) = &self.policy_reason {
            self.policy_reason = Some(bounded_text(reason, "policy_reason", MAX_ERROR_TEXT)?);
        }
        Ok(())
    }

    /// Build an outcome from the answer of the tool executor.
    ///
    /// The Python implementation decodes the result payload of the tool
    /// answer and refuses a payload that is not a JSON object. The Rust
    /// contract already carries a parsed JSON object, so the decode cannot
    /// fail; the refusal is kept for the shapes the contract admits.
    pub fn from_tool_result(
        step_id: &str,
        result: &ToolResult,
        tool_id: &str,
    ) -> Result<Self, AgentError> {
        let payload = match result.result_json() {
            None => None,
            Some(object) => {
                let value = object
                    .to_value()
                    .map_err(|error| AgentError::Validation(error.message().to_owned()))?;
                if !matches!(value, CanonicalValue::Object(_)) {
                    return Err(AgentError::merge(
                        "a tool result payload must be a JSON object",
                    ));
                }
                Some(value)
            }
        };
        let artifact_ids: Vec<String> = result
            .artifacts()
            .iter()
            .map(|artifact| artifact.artifact_id().to_owned())
            .collect();
        Self::from_parts(
            step_id,
            result.call_id(),
            result.status(),
            true,
            tool_id,
            payload.as_ref(),
            result.error_code(),
            result.error_message().and_then(|message| message.as_str()),
            &artifact_ids,
            result.duration_ms().unwrap_or(0.0),
            None,
        )
    }

    /// The step identifier.
    #[must_use]
    pub fn step_id(&self) -> &str {
        &self.step_id
    }

    /// The call identifier.
    #[must_use]
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The status of the step.
    #[must_use]
    pub fn status(&self) -> ToolRunStatus {
        self.status
    }

    /// Whether the step was dispatched to a tool.
    #[must_use]
    pub fn dispatched(&self) -> bool {
        self.dispatched
    }

    /// The tool identifier.
    #[must_use]
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// The result payload of the step.
    #[must_use]
    pub fn result(&self) -> Option<&CanonicalValue> {
        self.result.as_ref()
    }

    /// The error code of the step.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    /// The error message of the step.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// The artifact identifiers the step reported.
    #[must_use]
    pub fn artifact_ids(&self) -> &[String] {
        &self.artifact_ids
    }

    /// The duration of the step, in milliseconds.
    #[must_use]
    pub fn duration_ms(&self) -> f64 {
        self.duration_ms
    }

    /// The policy reason of a refusal.
    #[must_use]
    pub fn policy_reason(&self) -> Option<&str> {
        self.policy_reason.as_deref()
    }

    /// Whether the step succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.status == ToolRunStatus::Succeeded && self.result.is_some()
    }

    /// Whether the step was denied.
    #[must_use]
    pub fn denied(&self) -> bool {
        self.status == ToolRunStatus::Denied
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "artifact_ids",
                CanonicalValue::array(
                    self.artifact_ids
                        .iter()
                        .map(|value| CanonicalValue::text(value.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            ("call_id", CanonicalValue::text(self.call_id.clone())),
            ("dispatched", CanonicalValue::Bool(self.dispatched)),
            ("duration_ms", CanonicalValue::Float(self.duration_ms)),
            (
                "error_code",
                match &self.error_code {
                    Some(value) => CanonicalValue::text(value.clone()),
                    None => CanonicalValue::Null,
                },
            ),
            (
                "error_message",
                match &self.error_message {
                    Some(value) => CanonicalValue::text(value.clone()),
                    None => CanonicalValue::Null,
                },
            ),
            (
                "policy_reason",
                match &self.policy_reason {
                    Some(value) => CanonicalValue::text(value.clone()),
                    None => CanonicalValue::Null,
                },
            ),
            (
                "result",
                match &self.result {
                    Some(value) => thaw_object(value),
                    None => CanonicalValue::Null,
                },
            ),
            ("status", CanonicalValue::text(self.status.as_str())),
            ("step_id", CanonicalValue::text(self.step_id.clone())),
            ("tool_id", CanonicalValue::text(self.tool_id.clone())),
        ])
    }
}

/// The result of one run of the agent.
///
/// The result carries the outcome of every step, the merged results of the
/// steps that succeeded, the digest of every merged result, the degradation
/// reasons of the planner and the steps the budget refused.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRunResult {
    run_id: String,
    outcomes: Vec<AgentStepOutcome>,
    merged: CanonicalValue,
    digests: Vec<(String, String)>,
    degraded_reasons: Vec<String>,
    budget_denied: Vec<String>,
}

impl AgentRunResult {
    /// Build a run result from every field at once.
    ///
    /// The fields are validated in the order of the Python `__post_init__`:
    /// `run_id`, the outcomes, the merged results, the digests, the
    /// degradation reasons and the budget refusals.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        run_id: &str,
        outcomes: Vec<AgentStepOutcome>,
        merged: &CanonicalValue,
        digests: &[(String, String)],
        degraded_reasons: &[String],
        budget_denied: &[String],
    ) -> Result<Self, AgentError> {
        let mut result = Self {
            run_id: run_id.to_owned(),
            outcomes,
            merged: merged.clone(),
            digests: digests.to_vec(),
            degraded_reasons: degraded_reasons.to_vec(),
            budget_denied: budget_denied.to_vec(),
        };
        result.validate()?;
        Ok(result)
    }

    /// Validate every field, in the order of the Python `__post_init__`.
    fn validate(&mut self) -> Result<(), AgentError> {
        self.run_id = bounded_identifier(&self.run_id, "run_id")?;
        self.merged = freeze_object(&self.merged, "merged")?;
        let mut digests: Vec<(String, String)> = Vec::with_capacity(self.digests.len());
        for (key, value) in &self.digests {
            let name = bounded_identifier(key, "digest key")?;
            if !is_sha256_hex(value) {
                return Err(AgentError::merge(
                    "digests must be sha256 hexadecimal digests",
                ));
            }
            digests.push((name, value.clone()));
        }
        self.digests = digests;
        self.degraded_reasons = bounded_reasons(&self.degraded_reasons, "degraded_reasons")?;
        self.budget_denied = identifier_tuple(&self.budget_denied, "budget_denied")?;
        Ok(())
    }

    /// The run identifier.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// The outcome of every step, in dependency order.
    #[must_use]
    pub fn outcomes(&self) -> &[AgentStepOutcome] {
        &self.outcomes
    }

    /// The merged results of the steps that succeeded.
    #[must_use]
    pub fn merged(&self) -> &CanonicalValue {
        &self.merged
    }

    /// The digest of every merged result.
    #[must_use]
    pub fn digests(&self) -> &[(String, String)] {
        &self.digests
    }

    /// The degradation reasons the planner recorded.
    #[must_use]
    pub fn degraded_reasons(&self) -> &[String] {
        &self.degraded_reasons
    }

    /// The steps the budget refused.
    #[must_use]
    pub fn budget_denied(&self) -> &[String] {
        &self.budget_denied
    }

    /// Whether every step succeeded.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        !self.outcomes.is_empty() && self.outcomes.iter().all(AgentStepOutcome::succeeded)
    }

    /// The step identifiers, in order.
    #[must_use]
    pub fn step_ids(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .map(|outcome| outcome.step_id().to_owned())
            .collect()
    }

    /// The distinct artifact identifiers the run reported.
    #[must_use]
    pub fn artifacts(&self) -> Vec<String> {
        let mut collected: Vec<String> = Vec::new();
        for outcome in &self.outcomes {
            for artifact_id in outcome.artifact_ids() {
                if !collected.contains(artifact_id) {
                    collected.push(artifact_id.clone());
                }
            }
        }
        collected
    }

    /// The outcome of one step.
    pub fn outcome_for(&self, step_id: &str) -> Result<&AgentStepOutcome, AgentError> {
        let key = bounded_identifier(step_id, "step_id")?;
        self.outcomes
            .iter()
            .find(|outcome| outcome.step_id() == key)
            .ok_or_else(|| AgentError::merge(format!("no outcome was recorded for step {key}")))
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        let mut digests = CanonicalObject::new();
        for (key, value) in &self.digests {
            digests.insert(JsonText::new(key), CanonicalValue::text(value.clone()));
        }
        CanonicalValue::object([
            (
                "artifacts",
                CanonicalValue::array(
                    self.artifacts()
                        .into_iter()
                        .map(CanonicalValue::text)
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "budget_denied",
                CanonicalValue::array(
                    self.budget_denied
                        .iter()
                        .map(|value| CanonicalValue::text(value.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "degraded_reasons",
                CanonicalValue::array(
                    self.degraded_reasons
                        .iter()
                        .map(|value| CanonicalValue::text(value.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            ("digests", CanonicalValue::Object(digests)),
            ("merged", thaw_object(&self.merged)),
            (
                "outcomes",
                CanonicalValue::array(
                    self.outcomes
                        .iter()
                        .map(AgentStepOutcome::to_payload)
                        .collect::<Vec<_>>(),
                ),
            ),
            ("run_id", CanonicalValue::text(self.run_id.clone())),
            (
                "status",
                CanonicalValue::text(if self.succeeded() {
                    "completed"
                } else {
                    "incomplete"
                }),
            ),
        ])
    }
}

/// Whether a text is a lowercase hexadecimal SHA-256 digest.
///
/// The Python implementation uses `re.fullmatch(r"[0-9a-f]{64}", value)`.
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
