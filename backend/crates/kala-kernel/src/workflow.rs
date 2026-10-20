//! Native workflow approval/attempt/transition state machine.
//!
//! Complete port of `kala_agent.kernel.workflow`: immutable validated records,
//! the exact transition/history ordering, retry and replanning bounds,
//! checkpoint resolution, stale-approval invalidation, canonical state/action
//! digests, and restore validation. Event sinks are invoked outside the engine
//! mutex so a re-entrant or slow sink cannot deadlock another thread; the
//! mutation order relative to each event matches the Python source.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use indexmap::IndexMap;
use kala_contracts::{
    ApprovalKind, ApprovalStatus, Text, WorkflowDefinition, WorkflowStatus, WorkflowTransition,
};
use kala_json::{CanonicalValue, Sha256Digest};
use num_bigint::BigInt;

use crate::json_input::parse_model_json;
use crate::time::{IsoformatError, KernelTimestamp};
use crate::value::{
    canonical_state_digest, canonical_state_json, is_python_whitespace, value_bool, value_integer,
    value_text,
};

const STEP_FAILED: &str = "step_failed";
const RETRY_BOUND_EXHAUSTED: &str = "step_retry_bound_exhausted";
const STALE_APPROVAL: &str = "state_or_action_digest_changed";

/// Base workflow error, mapping the Python exception hierarchy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowError {
    /// `WorkflowError` (`RuntimeError`).
    Base(String),
    /// `WorkflowTransitionError` (`WorkflowError`, `ValueError`).
    Transition(String),
    /// `WorkflowLimitExceeded` (`WorkflowTransitionError`).
    LimitExceeded(String),
    /// `WorkflowApprovalError` (`WorkflowTransitionError`).
    Approval(String),
    /// An event sink refused an event. Python propagates the sink's own
    /// exception; the Rust sink contract is typed, so the message is wrapped
    /// here.
    Callback(String),
}

impl WorkflowError {
    /// The closest Python exception class.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Base(_) | Self::Callback(_) => "WorkflowError",
            Self::Transition(_) => "WorkflowTransitionError",
            Self::LimitExceeded(_) => "WorkflowLimitExceeded",
            Self::Approval(_) => "WorkflowApprovalError",
        }
    }

    /// The error message.
    pub fn message(&self) -> &str {
        match self {
            Self::Base(message)
            | Self::Transition(message)
            | Self::LimitExceeded(message)
            | Self::Approval(message)
            | Self::Callback(message) => message,
        }
    }
}

impl fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for WorkflowError {}

fn transition_error(message: impl Into<String>) -> WorkflowError {
    WorkflowError::Transition(message.into())
}

fn limit_exceeded(message: impl Into<String>) -> WorkflowError {
    WorkflowError::LimitExceeded(message.into())
}

fn approval_error(message: impl Into<String>) -> WorkflowError {
    WorkflowError::Approval(message.into())
}

fn safe_identifier(value: &Text, field_name: &str) -> Result<Text, WorkflowError> {
    let normalized = value.python_strip();
    if normalized.is_empty() || normalized.len() > 128 {
        return Err(transition_error(format!(
            "{field_name} must be nonempty and bounded"
        )));
    }
    if normalized
        .code_points()
        .iter()
        .copied()
        .any(|point| is_python_whitespace(point) || point < 32)
    {
        return Err(transition_error(format!(
            "{field_name} contains unsafe characters"
        )));
    }
    Ok(normalized)
}

fn status_from_text(value: &Text) -> Result<WorkflowStatus, WorkflowError> {
    let source = value.to_string_lossy();
    WorkflowStatus::parse(&source).ok_or_else(|| transition_error("workflow status is invalid"))
}

fn approval_status_from_text(value: &Text) -> Result<ApprovalStatus, WorkflowError> {
    let source = value.to_string_lossy();
    ApprovalStatus::parse(&source).ok_or_else(|| transition_error("approval status is invalid"))
}

fn approval_kind_from_text(value: &Text) -> Result<ApprovalKind, WorkflowError> {
    let source = value.to_string_lossy();
    ApprovalKind::parse(&source).ok_or_else(|| transition_error("checkpoint_type is invalid"))
}

fn parse_serialized_datetime(
    value: Option<&CanonicalValue>,
) -> Result<Option<KernelTimestamp>, WorkflowError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let Some(text) = value_text(value) else {
        return Err(transition_error("serialized timestamp is invalid"));
    };
    if text.has_surrogates() {
        return Err(transition_error("serialized timestamp is invalid"));
    }
    let source = text.to_string_lossy();
    KernelTimestamp::parse_isoformat_z_replaced(&source)
        .map_err(|error| match error {
            IsoformatError::Invalid => transition_error("serialized timestamp is invalid"),
            IsoformatError::Naive => {
                transition_error("serialized timestamp must include a timezone")
            }
        })
        .map(Some)
}

fn required_text(
    payload: &kala_json::CanonicalObject,
    field: &str,
    missing: &str,
) -> Result<Text, WorkflowError> {
    let Some(value) = payload.get(&kala_json::JsonText::new(field)) else {
        return Err(transition_error(missing));
    };
    value_text(value).ok_or_else(|| transition_error(format!("{field} must be a string")))
}

fn optional_text(
    payload: &kala_json::CanonicalObject,
    field: &str,
) -> Result<Option<Text>, WorkflowError> {
    match payload.get(&kala_json::JsonText::new(field)) {
        None | Some(CanonicalValue::Null) => Ok(None),
        Some(value) => value_text(value)
            .map(Some)
            .ok_or_else(|| transition_error(format!("{field} must be a string"))),
    }
}

fn text_value(value: &Text) -> CanonicalValue {
    CanonicalValue::Text(value.as_json_text().clone())
}

fn opt_text_value(value: Option<&Text>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, text_value)
}

fn opt_timestamp_value(value: Option<KernelTimestamp>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| {
        CanonicalValue::Text(kala_json::JsonText::new(&value.isoformat()))
    })
}

fn status_value(value: WorkflowStatus) -> CanonicalValue {
    CanonicalValue::Text(kala_json::JsonText::new(value.as_str()))
}

fn approval_status_value(value: ApprovalStatus) -> CanonicalValue {
    CanonicalValue::Text(kala_json::JsonText::new(value.as_str()))
}

fn integer_value(value: &BigInt) -> CanonicalValue {
    CanonicalValue::Integer(value.clone())
}

/// An immutable checkpoint approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowApproval {
    approval_id: Text,
    transition_id: Text,
    step_id: Text,
    checkpoint_type: ApprovalKind,
    state_digest: Text,
    action_digest: Text,
    status: ApprovalStatus,
    actor_id: Option<Text>,
    decided_at: Option<KernelTimestamp>,
    reason: Option<Text>,
}

impl WorkflowApproval {
    /// Build and validate an approval.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        approval_id: Text,
        transition_id: Text,
        step_id: Text,
        checkpoint_type: ApprovalKind,
        state_digest: Text,
        action_digest: Text,
        status: ApprovalStatus,
        actor_id: Option<Text>,
        decided_at: Option<KernelTimestamp>,
        reason: Option<Text>,
    ) -> Result<Self, WorkflowError> {
        let approval_id = safe_identifier(&approval_id, "approval_id")?;
        let transition_id = safe_identifier(&transition_id, "transition_id")?;
        let step_id = safe_identifier(&step_id, "step_id")?;
        if state_digest.is_empty() {
            return Err(transition_error("state_digest must be nonempty"));
        }
        if action_digest.is_empty() {
            return Err(transition_error("action_digest must be nonempty"));
        }
        let actor_id = match actor_id {
            Some(actor_id) => Some(safe_identifier(&actor_id, "actor_id")?),
            None => None,
        };
        if status == ApprovalStatus::Pending {
            if actor_id.is_some() || decided_at.is_some() || reason.is_some() {
                return Err(transition_error(
                    "pending approval cannot contain a decision",
                ));
            }
        } else if matches!(
            status,
            ApprovalStatus::Approved | ApprovalStatus::Rejected | ApprovalStatus::Aborted
        ) && (actor_id.is_none() || decided_at.is_none())
        {
            return Err(transition_error(
                "resolved approval requires actor and decision time",
            ));
        }
        Ok(Self {
            approval_id,
            transition_id,
            step_id,
            checkpoint_type,
            state_digest,
            action_digest,
            status,
            actor_id,
            decided_at,
            reason,
        })
    }

    /// The approval identifier.
    pub fn approval_id(&self) -> &Text {
        &self.approval_id
    }
    /// The bound transition.
    pub fn transition_id(&self) -> &Text {
        &self.transition_id
    }
    /// The bound step.
    pub fn step_id(&self) -> &Text {
        &self.step_id
    }
    /// The checkpoint kind.
    pub fn checkpoint_type(&self) -> ApprovalKind {
        self.checkpoint_type
    }
    /// The bound state digest.
    pub fn state_digest(&self) -> &Text {
        &self.state_digest
    }
    /// The bound action digest.
    pub fn action_digest(&self) -> &Text {
        &self.action_digest
    }
    /// The approval status.
    pub fn status(&self) -> ApprovalStatus {
        self.status
    }
    /// The deciding actor.
    pub fn actor_id(&self) -> Option<&Text> {
        self.actor_id.as_ref()
    }
    /// The decision instant.
    pub fn decided_at(&self) -> Option<KernelTimestamp> {
        self.decided_at
    }
    /// The decision note.
    pub fn reason(&self) -> Option<&Text> {
        self.reason.as_ref()
    }
    /// Python's `is_pending` property.
    pub fn is_pending(&self) -> bool {
        self.status == ApprovalStatus::Pending
    }
    /// Python's `is_approved` property.
    pub fn is_approved(&self) -> bool {
        self.status == ApprovalStatus::Approved
    }

    /// The Python `to_dict()` shape.
    pub fn to_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `to_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("action_digest", text_value(&self.action_digest)),
            ("actor_id", opt_text_value(self.actor_id.as_ref())),
            ("approval_id", text_value(&self.approval_id)),
            (
                "checkpoint_type",
                CanonicalValue::Text(kala_json::JsonText::new(self.checkpoint_type.as_str())),
            ),
            ("decided_at", opt_timestamp_value(self.decided_at)),
            ("reason", opt_text_value(self.reason.as_ref())),
            ("state_digest", text_value(&self.state_digest)),
            ("status", approval_status_value(self.status)),
            ("step_id", text_value(&self.step_id)),
            ("transition_id", text_value(&self.transition_id)),
        ])
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_dict(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        Self::from_value(value)
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        let CanonicalValue::Object(payload) = value else {
            return Err(transition_error("serialized approval is invalid"));
        };
        let missing = "serialized approval is missing a field";
        let approval_id = required_text(payload, "approval_id", missing)?;
        let transition_id = required_text(payload, "transition_id", missing)?;
        let step_id = required_text(payload, "step_id", missing)?;
        let checkpoint_type_text = match payload.get(&kala_json::JsonText::new("checkpoint_type")) {
            Some(value) => {
                value_text(value).ok_or_else(|| transition_error("checkpoint_type is invalid"))?
            }
            None => return Err(transition_error(missing)),
        };
        let checkpoint_type = approval_kind_from_text(&checkpoint_type_text)?;
        let state_digest = required_text(payload, "state_digest", missing)?;
        if state_digest.is_empty() {
            return Err(transition_error("state_digest must be nonempty"));
        }
        let action_digest = required_text(payload, "action_digest", missing)?;
        if action_digest.is_empty() {
            return Err(transition_error("action_digest must be nonempty"));
        }
        let status_text = match payload.get(&kala_json::JsonText::new("status")) {
            Some(value) => {
                value_text(value).ok_or_else(|| transition_error("approval status is invalid"))?
            }
            None => return Err(transition_error(missing)),
        };
        let status = approval_status_from_text(&status_text)?;
        let actor_id = optional_text(payload, "actor_id")?;
        let decided_at =
            parse_serialized_datetime(payload.get(&kala_json::JsonText::new("decided_at")))?;
        let reason = optional_text(payload, "reason")?;
        Self::from_parts(
            approval_id,
            transition_id,
            step_id,
            checkpoint_type,
            state_digest,
            action_digest,
            status,
            actor_id,
            decided_at,
            reason,
        )
    }
}

fn supplied_action_key(
    approval: &WorkflowApproval,
    state_digest: Option<&Text>,
    action_digest: Option<&Text>,
) -> Result<(Text, Text), WorkflowError> {
    let supplied_state = match state_digest {
        Some(value) => safe_identifier(value, "state_digest")?,
        None => approval.state_digest().clone(),
    };
    let supplied_action = match action_digest {
        Some(value) => safe_identifier(value, "action_digest")?,
        None => approval.action_digest().clone(),
    };
    Ok((supplied_state, supplied_action))
}

/// One recorded step attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepAttempt {
    step_id: Text,
    attempt: BigInt,
    succeeded: bool,
    error_code: Option<Text>,
}

impl StepAttempt {
    /// Build and validate an attempt.
    pub fn from_parts(
        step_id: Text,
        attempt: BigInt,
        succeeded: bool,
        error_code: Option<Text>,
    ) -> Result<Self, WorkflowError> {
        let step_id = safe_identifier(&step_id, "step_id")?;
        if attempt < BigInt::from(1) {
            return Err(transition_error("step attempt must be a positive integer"));
        }
        let error_code = match error_code {
            Some(error_code) => Some(safe_identifier(&error_code, "error_code")?),
            None => None,
        };
        Ok(Self {
            step_id,
            attempt,
            succeeded,
            error_code,
        })
    }

    /// The step identifier.
    pub fn step_id(&self) -> &Text {
        &self.step_id
    }
    /// The one-based attempt number.
    pub fn attempt(&self) -> &BigInt {
        &self.attempt
    }
    /// Whether the attempt succeeded.
    pub fn succeeded(&self) -> bool {
        self.succeeded
    }
    /// The failure code.
    pub fn error_code(&self) -> Option<&Text> {
        self.error_code.as_ref()
    }

    /// The Python `to_dict()` shape.
    pub fn to_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `to_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("attempt", integer_value(&self.attempt)),
            ("error_code", opt_text_value(self.error_code.as_ref())),
            ("step_id", text_value(&self.step_id)),
            ("succeeded", CanonicalValue::Bool(self.succeeded)),
        ])
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_dict(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        Self::from_value(value)
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        let CanonicalValue::Object(payload) = value else {
            return Err(transition_error("serialized step attempt is invalid"));
        };
        let missing = "serialized step attempt is missing a field";
        let step_id = required_text(payload, "step_id", missing)?;
        let attempt = payload
            .get(&kala_json::JsonText::new("attempt"))
            .and_then(value_integer)
            .cloned()
            .ok_or_else(|| {
                if payload.contains_key(&kala_json::JsonText::new("attempt")) {
                    transition_error("step attempt must be a positive integer")
                } else {
                    transition_error(missing)
                }
            })?;
        let succeeded = payload
            .get(&kala_json::JsonText::new("succeeded"))
            .and_then(value_bool)
            .ok_or_else(|| {
                if payload.contains_key(&kala_json::JsonText::new("succeeded")) {
                    transition_error("step succeeded must be a boolean")
                } else {
                    transition_error(missing)
                }
            })?;
        let error_code = optional_text(payload, "error_code")?;
        Self::from_parts(step_id, attempt, succeeded, error_code)
    }
}

/// One immutable transition-history entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowTransitionRecord {
    sequence: BigInt,
    transition_id: Text,
    from_state: Text,
    to_state: Text,
    step_id: Text,
    attempt: BigInt,
}

impl WorkflowTransitionRecord {
    /// Build and validate a history record.
    pub fn from_parts(
        sequence: BigInt,
        transition_id: Text,
        from_state: Text,
        to_state: Text,
        step_id: Text,
        attempt: BigInt,
    ) -> Result<Self, WorkflowError> {
        if sequence < BigInt::from(1) {
            return Err(transition_error("transition sequence must be positive"));
        }
        let transition_id = safe_identifier(&transition_id, "transition_id")?;
        let from_state = safe_identifier(&from_state, "from_state")?;
        let to_state = safe_identifier(&to_state, "to_state")?;
        let step_id = safe_identifier(&step_id, "step_id")?;
        if attempt < BigInt::from(1) {
            return Err(transition_error("transition attempt must be positive"));
        }
        Ok(Self {
            sequence,
            transition_id,
            from_state,
            to_state,
            step_id,
            attempt,
        })
    }

    /// The one-based sequence number.
    pub fn sequence(&self) -> &BigInt {
        &self.sequence
    }
    /// The transition identifier.
    pub fn transition_id(&self) -> &Text {
        &self.transition_id
    }
    /// The origin state.
    pub fn from_state(&self) -> &Text {
        &self.from_state
    }
    /// The destination state.
    pub fn to_state(&self) -> &Text {
        &self.to_state
    }
    /// The step identifier.
    pub fn step_id(&self) -> &Text {
        &self.step_id
    }
    /// The attempt number.
    pub fn attempt(&self) -> &BigInt {
        &self.attempt
    }

    /// The Python `to_dict()` shape.
    pub fn to_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `to_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("attempt", integer_value(&self.attempt)),
            ("from_state", text_value(&self.from_state)),
            ("sequence", integer_value(&self.sequence)),
            ("step_id", text_value(&self.step_id)),
            ("to_state", text_value(&self.to_state)),
            ("transition_id", text_value(&self.transition_id)),
        ])
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_dict(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        Self::from_value(value)
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        let CanonicalValue::Object(payload) = value else {
            return Err(transition_error("serialized transition is invalid"));
        };
        let missing = "serialized transition is missing a field";
        let sequence = match payload.get(&kala_json::JsonText::new("sequence")) {
            Some(value) => value_integer(value)
                .cloned()
                .ok_or_else(|| transition_error("transition sequence must be positive"))?,
            None => return Err(transition_error(missing)),
        };
        let transition_id = required_text(payload, "transition_id", missing)?;
        let from_state = required_text(payload, "from_state", missing)?;
        let to_state = required_text(payload, "to_state", missing)?;
        let step_id = required_text(payload, "step_id", missing)?;
        let attempt = match payload.get(&kala_json::JsonText::new("attempt")) {
            Some(value) => value_integer(value)
                .cloned()
                .ok_or_else(|| transition_error("transition attempt must be positive"))?,
            None => return Err(transition_error(missing)),
        };
        Self::from_parts(
            sequence,
            transition_id,
            from_state,
            to_state,
            step_id,
            attempt,
        )
    }
}

/// An immutable workflow state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowState {
    workflow_id: Text,
    definition_id: Text,
    definition_version: Text,
    status: WorkflowStatus,
    current_state: Text,
    completed_step_ids: Vec<Text>,
    attempts: Vec<StepAttempt>,
    pending_approval: Option<WorkflowApproval>,
    replanning_attempts: BigInt,
    history: Vec<WorkflowTransitionRecord>,
    last_error: Option<Text>,
}

impl WorkflowState {
    /// Build and validate a workflow state.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        workflow_id: Text,
        definition_id: Text,
        definition_version: Text,
        status: WorkflowStatus,
        current_state: Text,
        completed_step_ids: Vec<Text>,
        attempts: Vec<StepAttempt>,
        pending_approval: Option<WorkflowApproval>,
        replanning_attempts: BigInt,
        history: Vec<WorkflowTransitionRecord>,
        last_error: Option<Text>,
    ) -> Result<Self, WorkflowError> {
        let workflow_id = safe_identifier(&workflow_id, "workflow_id")?;
        let definition_id = safe_identifier(&definition_id, "definition_id")?;
        let definition_version = safe_identifier(&definition_version, "definition_version")?;
        let current_state = safe_identifier(&current_state, "current_state")?;
        let completed_step_ids = completed_step_ids
            .into_iter()
            .map(|value| safe_identifier(&value, "completed_step_id"))
            .collect::<Result<Vec<_>, _>>()?;
        if replanning_attempts < BigInt::from(0) {
            return Err(transition_error("replanning_attempts must be nonnegative"));
        }
        if status == WorkflowStatus::WaitingForApproval && pending_approval.is_none() {
            return Err(transition_error(
                "waiting workflows require a pending approval",
            ));
        }
        Ok(Self {
            workflow_id,
            definition_id,
            definition_version,
            status,
            current_state,
            completed_step_ids,
            attempts,
            pending_approval,
            replanning_attempts,
            history,
            last_error,
        })
    }

    /// The workflow identifier.
    pub fn workflow_id(&self) -> &Text {
        &self.workflow_id
    }
    /// The definition identifier.
    pub fn definition_id(&self) -> &Text {
        &self.definition_id
    }
    /// The definition version.
    pub fn definition_version(&self) -> &Text {
        &self.definition_version
    }
    /// The workflow status.
    pub fn status(&self) -> WorkflowStatus {
        self.status
    }
    /// The current state.
    pub fn current_state(&self) -> &Text {
        &self.current_state
    }
    /// Completed step identifiers in completion order.
    pub fn completed_step_ids(&self) -> &[Text] {
        &self.completed_step_ids
    }
    /// Attempt records in order.
    pub fn attempts(&self) -> &[StepAttempt] {
        &self.attempts
    }
    /// The pending approval, when present.
    pub fn pending_approval(&self) -> Option<&WorkflowApproval> {
        self.pending_approval.as_ref()
    }
    /// The replanning counter.
    pub fn replanning_attempts(&self) -> &BigInt {
        &self.replanning_attempts
    }
    /// Transition history in order.
    pub fn history(&self) -> &[WorkflowTransitionRecord] {
        &self.history
    }
    /// The last error or cancellation reason.
    pub fn last_error(&self) -> Option<&Text> {
        self.last_error.as_ref()
    }
    /// Python's `is_terminal` property.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        )
    }
    /// Python's `is_cancelled` property.
    pub fn is_cancelled(&self) -> bool {
        self.status == WorkflowStatus::Cancelled
    }

    /// The Python `to_dict()` shape.
    pub fn to_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `to_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "attempts",
                CanonicalValue::Array(self.attempts.iter().map(StepAttempt::as_value).collect()),
            ),
            (
                "completed_step_ids",
                CanonicalValue::Array(self.completed_step_ids.iter().map(text_value).collect()),
            ),
            ("current_state", text_value(&self.current_state)),
            ("definition_id", text_value(&self.definition_id)),
            ("definition_version", text_value(&self.definition_version)),
            (
                "history",
                CanonicalValue::Array(
                    self.history
                        .iter()
                        .map(WorkflowTransitionRecord::as_value)
                        .collect(),
                ),
            ),
            ("last_error", opt_text_value(self.last_error.as_ref())),
            (
                "pending_approval",
                self.pending_approval
                    .as_ref()
                    .map_or(CanonicalValue::Null, WorkflowApproval::as_value),
            ),
            (
                "replanning_attempts",
                integer_value(&self.replanning_attempts),
            ),
            ("status", status_value(self.status)),
            ("workflow_id", text_value(&self.workflow_id)),
        ])
    }

    /// The Python `to_json()` canonical bytes.
    pub fn to_json(&self) -> Result<String, WorkflowError> {
        canonical_state_json(&self.as_value()).map_err(|error| {
            transition_error(format!("serialized workflow state is invalid: {error}"))
        })
    }

    /// Parse the Python `from_dict` shape.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, WorkflowError> {
        let CanonicalValue::Object(payload) = value else {
            return Err(transition_error(
                "serialized workflow state must be an object",
            ));
        };
        let missing = "serialized workflow state is missing a field";
        let pending_value = payload.get(&kala_json::JsonText::new("pending_approval"));
        let attempts_value = payload.get(&kala_json::JsonText::new("attempts"));
        let history_value = payload.get(&kala_json::JsonText::new("history"));
        let pending = match pending_value {
            None | Some(CanonicalValue::Null) => None,
            Some(CanonicalValue::Object(_)) => Some(WorkflowApproval::from_value(
                pending_value.expect("present"),
            )?),
            Some(_) => return Err(transition_error("serialized approval is invalid")),
        };
        let attempts = match attempts_value {
            None => Vec::new(),
            Some(CanonicalValue::Array(values)) => values
                .iter()
                .map(StepAttempt::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(transition_error("serialized attempts are invalid")),
        };
        let history = match history_value {
            None => Vec::new(),
            Some(CanonicalValue::Array(values)) => values
                .iter()
                .map(WorkflowTransitionRecord::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(transition_error("serialized history is invalid")),
        };
        let workflow_id = required_text(payload, "workflow_id", missing)?;
        let definition_id = required_text(payload, "definition_id", missing)?;
        let definition_version = required_text(payload, "definition_version", missing)?;
        let status_text = match payload.get(&kala_json::JsonText::new("status")) {
            Some(value) => {
                value_text(value).ok_or_else(|| transition_error("workflow status is invalid"))?
            }
            None => return Err(transition_error(missing)),
        };
        let status = status_from_text(&status_text)?;
        let current_state = required_text(payload, "current_state", missing)?;
        let completed_step_ids = match payload.get(&kala_json::JsonText::new("completed_step_ids"))
        {
            None => Vec::new(),
            Some(CanonicalValue::Array(values)) => values
                .iter()
                .map(|value| {
                    value_text(value)
                        .ok_or_else(|| transition_error("completed_step_id must be a string"))
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => {
                return Err(transition_error(
                    "serialized completed_step_ids are invalid",
                ));
            }
        };
        let replanning_attempts =
            match payload.get(&kala_json::JsonText::new("replanning_attempts")) {
                None => BigInt::from(0),
                Some(value) => value_integer(value)
                    .cloned()
                    .ok_or_else(|| transition_error("replanning_attempts must be an integer"))?,
            };
        let last_error = optional_text(payload, "last_error")?;
        Self::from_parts(
            workflow_id,
            definition_id,
            definition_version,
            status,
            current_state,
            completed_step_ids,
            attempts,
            pending,
            replanning_attempts,
            history,
            last_error,
        )
    }

    /// Parse the Python `from_json` entry point.
    pub fn from_json(text: &str) -> Result<Self, WorkflowError> {
        let value = parse_model_json(text)
            .map_err(|_| transition_error("serialized workflow state is invalid"))?;
        if !matches!(value, CanonicalValue::Object(_)) {
            return Err(transition_error(
                "serialized workflow state must be an object",
            ));
        }
        Self::from_value(&value)
    }
}

/// An immutable workflow event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowEvent {
    /// The event type string.
    pub event_type: Text,
    /// The workflow identifier.
    pub workflow_id: Text,
    /// The previous status, when the event is a transition.
    pub from_status: Option<WorkflowStatus>,
    /// The new status.
    pub to_status: WorkflowStatus,
    /// The current workflow state at emission.
    pub current_state: Text,
    /// The transition identifier, when applicable.
    pub transition_id: Option<Text>,
    /// The step identifier, when applicable.
    pub step_id: Option<Text>,
    /// The attempt number, when applicable.
    pub attempt: Option<BigInt>,
    /// The event reason, when applicable.
    pub reason: Option<Text>,
    /// The approval snapshot, when applicable.
    pub approval: Option<WorkflowApproval>,
}

impl WorkflowEvent {
    /// The Python `payload` property.
    pub fn payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "approval",
                self.approval
                    .as_ref()
                    .map_or(CanonicalValue::Null, WorkflowApproval::as_value),
            ),
            (
                "attempt",
                self.attempt
                    .as_ref()
                    .map_or(CanonicalValue::Null, integer_value),
            ),
            ("current_state", text_value(&self.current_state)),
            (
                "from_status",
                self.from_status.map_or(CanonicalValue::Null, status_value),
            ),
            ("reason", opt_text_value(self.reason.as_ref())),
            ("step_id", opt_text_value(self.step_id.as_ref())),
            ("to_status", status_value(self.to_status)),
            ("transition_id", opt_text_value(self.transition_id.as_ref())),
            ("workflow_id", text_value(&self.workflow_id)),
        ])
    }

    /// The Python `to_dict()` shape.
    pub fn to_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `to_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        match &self.payload() {
            CanonicalValue::Object(object) => {
                let mut object = object.clone();
                object.insert(
                    kala_json::JsonText::new("event_type"),
                    text_value(&self.event_type),
                );
                CanonicalValue::Object(object)
            }
            _ => unreachable!("payload is an object"),
        }
    }
}

/// Python alias.
pub type WorkflowCheckpointStatus = ApprovalStatus;
/// Python alias.
pub type WorkflowCheckpoint = WorkflowApproval;
/// Python alias.
pub type WorkflowStateStore = WorkflowState;
/// Python alias for the transition exception class.
pub type WorkflowTransitionError = WorkflowError;
/// Python alias for the limit exception class.
pub type WorkflowLimitExceeded = WorkflowError;
/// Python alias for the approval exception class.
pub type WorkflowApprovalError = WorkflowError;

/// A typed event sink. Events are delivered in source order.
pub trait WorkflowEventSink: Send + Sync {
    /// Deliver one event.
    fn emit(&self, event: &WorkflowEvent) -> Result<(), WorkflowError>;
}

/// A sink backed by a closure receiving the full event.
pub struct EventCallbackSink<F>
where
    F: Fn(&WorkflowEvent) -> Result<(), WorkflowError> + Send + Sync,
{
    callback: F,
}

impl<F> EventCallbackSink<F>
where
    F: Fn(&WorkflowEvent) -> Result<(), WorkflowError> + Send + Sync,
{
    /// Build the adapter.
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> WorkflowEventSink for EventCallbackSink<F>
where
    F: Fn(&WorkflowEvent) -> Result<(), WorkflowError> + Send + Sync,
{
    fn emit(&self, event: &WorkflowEvent) -> Result<(), WorkflowError> {
        (self.callback)(event)
    }
}

/// A sink backed by a closure receiving `(event_type, payload)`.
pub struct PayloadCallbackSink<F>
where
    F: Fn(&Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    callback: F,
}

impl<F> PayloadCallbackSink<F>
where
    F: Fn(&Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    /// Build the adapter.
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> WorkflowEventSink for PayloadCallbackSink<F>
where
    F: Fn(&Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    fn emit(&self, event: &WorkflowEvent) -> Result<(), WorkflowError> {
        (self.callback)(&event.event_type, &event.payload())
    }
}

/// A sink backed by a closure receiving `(event_type, workflow_id, payload)`.
pub struct JournalCallbackSink<F>
where
    F: Fn(&Text, &Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    callback: F,
}

impl<F> JournalCallbackSink<F>
where
    F: Fn(&Text, &Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    /// Build the adapter.
    pub fn new(callback: F) -> Self {
        Self { callback }
    }
}

impl<F> WorkflowEventSink for JournalCallbackSink<F>
where
    F: Fn(&Text, &Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync,
{
    fn emit(&self, event: &WorkflowEvent) -> Result<(), WorkflowError> {
        (self.callback)(&event.event_type, &event.workflow_id, &event.payload())
    }
}

/// A thread-safe collecting sink, the analogue of `list.append`.
#[derive(Default)]
pub struct CollectingSink {
    events: Mutex<Vec<WorkflowEvent>>,
}

impl CollectingSink {
    /// Build an empty sink.
    pub fn new() -> Self {
        Self::default()
    }
    /// Snapshot the collected events.
    pub fn events(&self) -> Vec<WorkflowEvent> {
        self.events
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
    /// Discard collected events.
    pub fn clear(&self) {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.clear();
    }
}

impl WorkflowEventSink for CollectingSink {
    fn emit(&self, event: &WorkflowEvent) -> Result<(), WorkflowError> {
        let mut guard = self
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.push(event.clone());
        Ok(())
    }
}

/// Python `EventJournalCallback` / `WorkflowEventCallback` alias.
pub type EventJournalCallback = Arc<dyn WorkflowEventSink>;
/// Python alias.
pub type WorkflowEventCallback = Arc<dyn WorkflowEventSink>;

/// Extra arguments for checkpoint decisions.
#[derive(Clone, Debug)]
pub struct CheckpointContext {
    /// Optional expected state digest.
    pub state_digest: Option<Text>,
    /// Optional expected action digest.
    pub action_digest: Option<Text>,
    /// The deciding actor.
    pub actor_id: Text,
    /// The decision note.
    pub reason: Option<Text>,
    /// The decision time.
    pub now: Option<KernelTimestamp>,
}

impl Default for CheckpointContext {
    fn default() -> Self {
        Self {
            state_digest: None,
            action_digest: None,
            actor_id: Text::new("local-user"),
            reason: None,
            now: None,
        }
    }
}

impl CheckpointContext {
    /// Default context.
    pub fn new() -> Self {
        Self::default()
    }
    /// Override the state digest.
    pub fn with_state_digest(mut self, value: Text) -> Self {
        self.state_digest = Some(value);
        self
    }
    /// Override the action digest.
    pub fn with_action_digest(mut self, value: Text) -> Self {
        self.action_digest = Some(value);
        self
    }
    /// Override the actor.
    pub fn with_actor_id(mut self, value: Text) -> Self {
        self.actor_id = value;
        self
    }
    /// Set a reason.
    pub fn with_reason(mut self, value: Text) -> Self {
        self.reason = Some(value);
        self
    }
    /// Set the decision time.
    pub fn with_now(mut self, value: KernelTimestamp) -> Self {
        self.now = Some(value);
        self
    }
}

/// Thread-safe workflow engine.
pub struct WorkflowEngine {
    definition: WorkflowDefinition,
    callback: Option<Arc<dyn WorkflowEventSink>>,
    max_attempts: IndexMap<Text, BigInt>,
    state: Mutex<Option<WorkflowState>>,
    clock: Option<KernelTimestamp>,
}

impl fmt::Debug for WorkflowEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowEngine")
            .field("definition", &self.definition.definition_id())
            .field("has_callback", &self.callback.is_some())
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl WorkflowEngine {
    /// Create an engine for a validated definition.
    pub fn new(definition: WorkflowDefinition) -> Self {
        Self {
            definition,
            callback: None,
            max_attempts: IndexMap::new(),
            state: Mutex::new(None),
            clock: None,
        }
    }

    /// Attach an event sink.
    pub fn with_sink(mut self, sink: Arc<dyn WorkflowEventSink>) -> Self {
        self.callback = Some(sink);
        self
    }

    /// Attach a closure receiving the full event.
    pub fn with_event_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&WorkflowEvent) -> Result<(), WorkflowError> + Send + Sync + 'static,
    {
        self.callback = Some(Arc::new(EventCallbackSink::new(callback)));
        self
    }

    /// Attach a closure receiving `(event_type, payload)`.
    pub fn with_payload_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync + 'static,
    {
        self.callback = Some(Arc::new(PayloadCallbackSink::new(callback)));
        self
    }

    /// Attach a closure receiving `(event_type, workflow_id, payload)`.
    pub fn with_journal_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&Text, &Text, &CanonicalValue) -> Result<(), WorkflowError> + Send + Sync + 'static,
    {
        self.callback = Some(Arc::new(JournalCallbackSink::new(callback)));
        self
    }

    /// Set or clear the injectable clock.
    pub fn with_clock(mut self, clock: Option<KernelTimestamp>) -> Self {
        self.clock = clock;
        self
    }

    /// Set a fixed injectable clock.
    pub fn with_now(mut self, now: KernelTimestamp) -> Self {
        self.clock = Some(now);
        self
    }

    /// Configure per-step retry bounds.
    pub fn with_max_attempts<I>(mut self, values: I) -> Result<Self, WorkflowError>
    where
        I: IntoIterator<Item = (Text, BigInt)>,
    {
        let mut normalized = IndexMap::new();
        for (step_id, value) in values {
            let step_id = safe_identifier(&step_id, "step_id")?;
            if value < BigInt::from(1) {
                return Err(transition_error(
                    "step retry bounds must be positive integers",
                ));
            }
            normalized.insert(step_id, value);
        }
        self.max_attempts = normalized;
        Ok(self)
    }

    /// The workflow definition.
    pub fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }

    /// The current state, when started or restored.
    pub fn state(&self) -> Option<WorkflowState> {
        self.lock_state().clone()
    }

    /// Python alias.
    pub fn current_state(&self) -> Option<WorkflowState> {
        self.state()
    }

    /// Per-step retry bounds.
    pub fn max_attempts(&self) -> &IndexMap<Text, BigInt> {
        &self.max_attempts
    }

    fn lock_state(&self) -> MutexGuard<'_, Option<WorkflowState>> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn store(&self, state: WorkflowState) -> WorkflowState {
        *self.lock_state() = Some(state.clone());
        state
    }

    fn emit(&self, event: WorkflowEvent) -> Result<(), WorkflowError> {
        let Some(callback) = &self.callback else {
            return Ok(());
        };
        callback.emit(&event)
    }

    fn resolve_now(&self, value: Option<KernelTimestamp>) -> KernelTimestamp {
        value.or(self.clock).unwrap_or_else(KernelTimestamp::now)
    }

    /// Start (or return) a workflow.
    pub fn start(
        &self,
        workflow_id: &Text,
        replanning_attempts: Option<&BigInt>,
    ) -> Result<WorkflowState, WorkflowError> {
        let normalized_workflow_id = safe_identifier(workflow_id, "workflow_id")?;
        {
            let guard = self.lock_state();
            if let Some(state) = guard.as_ref() {
                if state.workflow_id() == &normalized_workflow_id {
                    return Ok(state.clone());
                }
            }
        }
        let replanning_attempts = replanning_attempts
            .cloned()
            .unwrap_or_else(|| BigInt::from(0));
        if replanning_attempts < BigInt::from(0)
            || replanning_attempts
                > self
                    .definition
                    .max_replanning_attempts()
                    .as_bigint()
                    .clone()
        {
            return Err(limit_exceeded(
                "replanning attempts exceed the workflow limit",
            ));
        }
        let state = WorkflowState::from_parts(
            normalized_workflow_id,
            Text::new(self.definition.definition_id()),
            Text::new(&self.definition.version().to_string_lossy()),
            WorkflowStatus::Created,
            Text::new(self.definition.initial_state()),
            Vec::new(),
            Vec::new(),
            None,
            replanning_attempts,
            Vec::new(),
            None,
        )?;
        self.store(state.clone());
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.created"),
            workflow_id: state.workflow_id().clone(),
            from_status: None,
            to_status: state.status(),
            current_state: state.current_state().clone(),
            transition_id: None,
            step_id: None,
            attempt: None,
            reason: None,
            approval: None,
        })?;
        Ok(state)
    }

    /// Advance, or record a step result when `succeeded`/`step_id` is given.
    pub fn run(
        &self,
        state: Option<&WorkflowState>,
        transition_id: Option<&Text>,
        step_id: Option<&Text>,
        succeeded: Option<bool>,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        if succeeded.is_some() || step_id.is_some() {
            let mut selected_step = step_id.cloned();
            if selected_step.is_none() {
                let transition = self.select_transition(&current, transition_id)?;
                let Some(transition) = transition else {
                    return self.complete_if_terminal(current);
                };
                selected_step = Some(Text::new(transition.step_id()));
            }
            let Some(succeeded) = succeeded else {
                return Err(transition_error("succeeded must be supplied with step_id"));
            };
            let updated = self.record_step_result(
                &current,
                selected_step.as_ref().expect("selected"),
                succeeded,
                transition_id,
                None,
            )?;
            return Ok(self.store(updated));
        }
        let updated = self.advance_internal(&current, transition_id)?;
        Ok(self.store(updated))
    }

    /// Python alias for `run`.
    pub fn advance(
        &self,
        state: Option<&WorkflowState>,
        transition_id: Option<&Text>,
        step_id: Option<&Text>,
        succeeded: Option<bool>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.run(state, transition_id, step_id, succeeded)
    }
    /// Python alias for `run`.
    pub fn execute(
        &self,
        state: Option<&WorkflowState>,
        transition_id: Option<&Text>,
        step_id: Option<&Text>,
        succeeded: Option<bool>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.run(state, transition_id, step_id, succeeded)
    }
    /// Python alias for `run`.
    pub fn step(
        &self,
        state: Option<&WorkflowState>,
        transition_id: Option<&Text>,
        step_id: Option<&Text>,
        succeeded: Option<bool>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.run(state, transition_id, step_id, succeeded)
    }

    /// Resume an approved checkpoint (or advance normally).
    pub fn resume(&self, state: Option<&WorkflowState>) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        if let Some(pending) = current.pending_approval() {
            if !pending.is_approved() {
                return Ok(current);
            }
            if !self.approval_matches_inner(&current, pending)? {
                return self.invalidate_approval(&current, &Text::new(STALE_APPROVAL));
            }
            let pending = pending.clone();
            let transition = self.get_transition(pending.transition_id())?;
            let current = Self::replace_state(
                &current,
                Some(None),
                Some(WorkflowStatus::Running),
                None,
                None,
                None,
            )?;
            let applied = self.apply_transition(&current, &transition)?;
            return Ok(self.store(applied));
        }
        self.run(Some(&current), None, None, None)
    }

    /// Record a step outcome against an explicit state (does not store).
    pub fn record_step_result(
        &self,
        state: &WorkflowState,
        step_id: &Text,
        succeeded: bool,
        transition_id: Option<&Text>,
        error_code: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let mut current = self.validated_state(state)?;
        if current.is_terminal() {
            return Ok(current);
        }
        let mut approved_transition_id: Option<Text> = None;
        if let Some(pending) = current.pending_approval().cloned() {
            if !pending.is_approved() {
                return Err(approval_error("step cannot run before checkpoint approval"));
            }
            if !self.approval_matches_inner(&current, &pending)? {
                return self.invalidate_approval(&current, &Text::new(STALE_APPROVAL));
            }
            approved_transition_id = Some(pending.transition_id().clone());
            current = Self::replace_state(
                &current,
                Some(None),
                Some(WorkflowStatus::Running),
                None,
                None,
                None,
            )?;
        }
        let transition = self.select_transition(&current, transition_id)?;
        let Some(transition) = transition else {
            return self.complete_if_terminal(current);
        };
        if Text::new(transition.step_id()) != *step_id {
            return Err(transition_error(
                "step result does not match the declared transition",
            ));
        }
        let transition_id_text = Text::new(transition.transition_id());
        if succeeded
            && transition.checkpoint_type().is_some()
            && approved_transition_id.as_ref() != Some(&transition_id_text)
        {
            return Err(approval_error("step cannot run before checkpoint approval"));
        }
        if succeeded {
            if self.attempt_count(&current, step_id) >= self.attempt_limit(step_id) {
                return self.fail_state(&current, &Text::new(RETRY_BOUND_EXHAUSTED), step_id);
            }
            return self.apply_transition(&current, &transition);
        }
        self.record_failure(&current, &transition, error_code)
    }

    /// Record a successful step.
    pub fn record_step_success(
        &self,
        state: &WorkflowState,
        step_id: &Text,
        transition_id: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.record_step_result(state, step_id, true, transition_id, None)
    }

    /// Record a failed step attempt.
    pub fn record_step_failure(
        &self,
        state: &WorkflowState,
        step_id: &Text,
        error_code: Option<&Text>,
        transition_id: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        // Python's `record_step_failure(error_code="step_failed")` defaults to
        // this literal; passing `None` here means "use the default".
        let default_error = Text::new(STEP_FAILED);
        let error_code = error_code.unwrap_or(&default_error);
        self.record_step_result(state, step_id, false, transition_id, Some(error_code))
    }

    /// Python alias.
    pub fn retry_step(
        &self,
        state: &WorkflowState,
        step_id: &Text,
        error_code: Option<&Text>,
        transition_id: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.record_step_failure(state, step_id, error_code, transition_id)
    }

    /// Approve a pending checkpoint.
    pub fn approve_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        let Some(pending) = current.pending_approval().cloned() else {
            return Err(approval_error("workflow has no pending checkpoint"));
        };
        if let Some(approval_id) = approval_id {
            if approval_id != pending.approval_id() {
                return Err(approval_error(
                    "approval_id does not match the pending checkpoint",
                ));
            }
        }
        if pending.status() != ApprovalStatus::Pending {
            if pending.status() == ApprovalStatus::Approved {
                return Ok(current);
            }
            return Err(approval_error(format!(
                "checkpoint is already {}",
                pending.status().as_str()
            )));
        }
        if supplied_action_key(
            &pending,
            context.state_digest.as_ref(),
            context.action_digest.as_ref(),
        )? != (
            pending.state_digest().clone(),
            pending.action_digest().clone(),
        ) {
            return self.invalidate_approval(&current, &Text::new(STALE_APPROVAL));
        }
        let decision_time = self.resolve_now(context.now);
        let approved = Self::approval_with(
            &pending,
            ApprovalStatus::Approved,
            Some(safe_identifier(&context.actor_id, "actor_id")?),
            Some(decision_time),
            None,
        )?;
        let updated = Self::replace_state(
            &current,
            Some(Some(approved.clone())),
            None,
            None,
            None,
            None,
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.approval_resolved"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(updated.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: Some(approved.transition_id().clone()),
            step_id: Some(approved.step_id().clone()),
            attempt: None,
            reason: None,
            approval: Some(approved),
        })?;
        Ok(self.store(updated))
    }

    /// Python alias.
    pub fn approve(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.approve_checkpoint(state, approval_id, context)
    }

    /// Reject a pending checkpoint (fails the workflow).
    pub fn reject_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        let reason = context
            .reason
            .clone()
            .unwrap_or_else(|| Text::new("checkpoint rejected"));
        self.resolve_checkpoint(
            state,
            approval_id,
            ApprovalStatus::Rejected,
            context,
            Some(reason),
        )
    }

    /// Python alias.
    pub fn reject(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.reject_checkpoint(state, approval_id, context)
    }

    /// Abort a pending checkpoint (cancels the workflow).
    pub fn abort_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        let reason = context
            .reason
            .clone()
            .unwrap_or_else(|| Text::new("checkpoint aborted"));
        self.resolve_checkpoint(
            state,
            approval_id,
            ApprovalStatus::Aborted,
            context,
            Some(reason),
        )
    }

    /// Python alias.
    pub fn abort(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.abort_checkpoint(state, approval_id, context)
    }

    /// Resolve a checkpoint with an already parsed approval status.
    pub fn resolve_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        status: ApprovalStatus,
        context: &CheckpointContext,
        reason: Option<Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        if status == ApprovalStatus::Approved {
            return self.approve_checkpoint(state, approval_id, context);
        }
        if status == ApprovalStatus::Rejected {
            return self.do_resolve_checkpoint(
                state,
                approval_id,
                ApprovalStatus::Rejected,
                context,
                Some(reason.unwrap_or_else(|| {
                    context
                        .reason
                        .clone()
                        .unwrap_or_else(|| Text::new("checkpoint rejected"))
                })),
            );
        }
        if status == ApprovalStatus::Aborted {
            return self.do_resolve_checkpoint(
                state,
                approval_id,
                ApprovalStatus::Aborted,
                context,
                Some(reason.unwrap_or_else(|| {
                    context
                        .reason
                        .clone()
                        .unwrap_or_else(|| Text::new("checkpoint rejected"))
                })),
            );
        }
        Err(approval_error(
            "checkpoint decision must be approve, reject, or abort",
        ))
    }

    /// Resolve a checkpoint from a spelling accepted by Python.
    pub fn resolve_checkpoint_str(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        decision: &str,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        let selected = match decision {
            "approve" | "approved" => ApprovalStatus::Approved,
            "reject" | "rejected" => ApprovalStatus::Rejected,
            "abort" | "aborted" => ApprovalStatus::Aborted,
            other => ApprovalStatus::parse(other)
                .ok_or_else(|| transition_error("approval status is invalid"))?,
        };
        self.resolve_checkpoint(state, approval_id, selected, context, None)
    }

    /// Resolve with Python's default approved decision.
    pub fn resolve_checkpoint_default(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.resolve_checkpoint(
            state,
            approval_id,
            ApprovalStatus::Approved,
            context,
            Some(Text::new("checkpoint rejected")),
        )
    }

    /// Decide with Python's default approved decision.
    pub fn decide_checkpoint_default(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.decide_checkpoint(state, approval_id, ApprovalStatus::Approved, context)
    }

    /// Python alias with an enum decision.
    pub fn decide_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        decision: ApprovalStatus,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.resolve_checkpoint(state, approval_id, decision, context, None)
    }

    /// Python alias with a string decision.
    pub fn decide_checkpoint_str(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        decision: &str,
        context: &CheckpointContext,
    ) -> Result<WorkflowState, WorkflowError> {
        self.resolve_checkpoint_str(state, approval_id, decision, context)
    }

    fn do_resolve_checkpoint(
        &self,
        state: Option<&WorkflowState>,
        approval_id: Option<&Text>,
        status: ApprovalStatus,
        context: &CheckpointContext,
        reason: Option<Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        let Some(pending) = current.pending_approval().cloned() else {
            return Err(approval_error("workflow has no pending checkpoint"));
        };
        if let Some(approval_id) = approval_id {
            if approval_id != pending.approval_id() {
                return Err(approval_error(
                    "approval_id does not match the pending checkpoint",
                ));
            }
        }
        if pending.status() != ApprovalStatus::Pending {
            return Err(approval_error(format!(
                "checkpoint is already {}",
                pending.status().as_str()
            )));
        }
        if supplied_action_key(
            &pending,
            context.state_digest.as_ref(),
            context.action_digest.as_ref(),
        )? != (
            pending.state_digest().clone(),
            pending.action_digest().clone(),
        ) {
            return self.invalidate_approval(&current, &Text::new(STALE_APPROVAL));
        }
        let resolved = Self::approval_with(
            &pending,
            status,
            Some(safe_identifier(&context.actor_id, "actor_id")?),
            Some(self.resolve_now(context.now)),
            reason.clone(),
        )?;
        let next_status = if status == ApprovalStatus::Aborted {
            WorkflowStatus::Cancelled
        } else {
            WorkflowStatus::Failed
        };
        let updated = Self::replace_state(
            &current,
            Some(None),
            Some(next_status),
            None,
            None,
            Some(reason.clone()),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.approval_resolved"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: Some(pending.transition_id().clone()),
            step_id: Some(pending.step_id().clone()),
            attempt: None,
            reason,
            approval: Some(resolved),
        })?;
        Ok(self.store(updated))
    }

    /// Cancel a non-terminal workflow.
    pub fn cancel(
        &self,
        state: Option<&WorkflowState>,
        reason: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        if current.status() == WorkflowStatus::Cancelled {
            return Ok(current);
        }
        if matches!(
            current.status(),
            WorkflowStatus::Completed | WorkflowStatus::Failed
        ) {
            return Ok(current);
        }
        let reason = reason
            .cloned()
            .unwrap_or_else(|| Text::new("workflow cancelled"));
        let approval = if let Some(pending) = current.pending_approval().cloned() {
            Some(Self::approval_with(
                &pending,
                ApprovalStatus::Aborted,
                Some(Text::new("local-user")),
                Some(self.resolve_now(None)),
                Some(reason.clone()),
            )?)
        } else {
            None
        };
        let updated = Self::replace_state(
            &current,
            Some(approval.clone()),
            Some(WorkflowStatus::Cancelled),
            None,
            None,
            Some(Some(reason.clone())),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.cancelled"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: WorkflowStatus::Cancelled,
            current_state: updated.current_state().clone(),
            transition_id: None,
            step_id: None,
            attempt: None,
            reason: Some(reason),
            approval,
        })?;
        Ok(self.store(updated))
    }

    /// Request a replanning attempt.
    pub fn request_replan(
        &self,
        state: Option<&WorkflowState>,
        reason: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.require_state(state)?;
        if current.is_terminal() {
            return Ok(current);
        }
        if current.replanning_attempts() >= self.definition.max_replanning_attempts().as_bigint() {
            return Err(limit_exceeded("replanning attempts exhausted"));
        }
        let reason = reason
            .cloned()
            .unwrap_or_else(|| Text::new("replan requested"));
        let updated = Self::replace_state(
            &current,
            None,
            None,
            None,
            Some(current.replanning_attempts() + BigInt::from(1)),
            Some(None),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.replanning_requested"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: None,
            step_id: None,
            attempt: None,
            reason: Some(reason),
            approval: None,
        })?;
        Ok(self.store(updated))
    }

    /// Python alias.
    pub fn replan(
        &self,
        state: Option<&WorkflowState>,
        reason: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        self.request_replan(state, reason)
    }

    /// Serialize a state to canonical JSON.
    pub fn serialize(&self, state: Option<&WorkflowState>) -> Result<String, WorkflowError> {
        self.require_state(state)?.to_json()
    }

    /// Return the current state snapshot.
    pub fn snapshot(&self, state: Option<&WorkflowState>) -> Result<WorkflowState, WorkflowError> {
        self.require_state(state)
    }

    /// Restore a typed state and validate definition consistency.
    pub fn restore(&self, state: WorkflowState) -> Result<WorkflowState, WorkflowError> {
        self.validate_restored(&state)?;
        Ok(self.store(state))
    }

    /// Restore from a dynamic state value.
    pub fn restore_value(&self, state: &CanonicalValue) -> Result<WorkflowState, WorkflowError> {
        self.restore(WorkflowState::from_value(state)?)
    }

    /// Restore from JSON text.
    pub fn restore_json(&self, state: &str) -> Result<WorkflowState, WorkflowError> {
        self.restore(WorkflowState::from_json(state)?)
    }

    /// Python alias.
    pub fn restore_state(&self, state: WorkflowState) -> Result<WorkflowState, WorkflowError> {
        self.restore(state)
    }
    /// Python alias.
    pub fn resume_from(&self, state: WorkflowState) -> Result<WorkflowState, WorkflowError> {
        self.restore(state)
    }

    /// The canonical state digest for a state (or the current state).
    pub fn state_digest(&self, state: Option<&WorkflowState>) -> Result<String, WorkflowError> {
        let current = self.require_state(state)?;
        self.state_digest_of(&current)
    }

    /// The canonical action digest for a transition and state digest.
    pub fn action_digest(
        &self,
        transition: &WorkflowTransition,
        state_digest: &str,
    ) -> Result<String, WorkflowError> {
        let payload = CanonicalValue::object([
            (
                "definition_id",
                CanonicalValue::from(self.definition.definition_id()),
            ),
            (
                "definition_version",
                CanonicalValue::from(self.definition.version().to_string_lossy()),
            ),
            ("from_state", CanonicalValue::from(transition.from_state())),
            ("state_digest", CanonicalValue::from(state_digest)),
            ("step_id", CanonicalValue::from(transition.step_id())),
            ("to_state", CanonicalValue::from(transition.to_state())),
        ]);
        let digest: Sha256Digest = canonical_state_digest(&payload)
            .map_err(|error| transition_error(format!("action digest failed: {error}")))?;
        Ok(digest.to_hex())
    }

    /// The canonical state digest for an explicit state.
    pub fn state_digest_of(&self, state: &WorkflowState) -> Result<String, WorkflowError> {
        let payload = CanonicalValue::object([
            (
                "attempts",
                CanonicalValue::Array(state.attempts().iter().map(StepAttempt::as_value).collect()),
            ),
            (
                "completed_step_ids",
                CanonicalValue::Array(state.completed_step_ids().iter().map(text_value).collect()),
            ),
            ("current_state", text_value(state.current_state())),
            ("definition_id", text_value(state.definition_id())),
            ("definition_version", text_value(state.definition_version())),
            (
                "replanning_attempts",
                integer_value(state.replanning_attempts()),
            ),
            ("workflow_id", text_value(state.workflow_id())),
        ]);
        let digest: Sha256Digest = canonical_state_digest(&payload)
            .map_err(|error| transition_error(format!("state digest failed: {error}")))?;
        Ok(digest.to_hex())
    }

    fn advance_internal(
        &self,
        state: &WorkflowState,
        transition_id: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let mut current = self.validated_state(state)?;
        if current.is_terminal() {
            return Ok(current);
        }
        if let Some(pending) = current.pending_approval().cloned() {
            if !pending.is_approved() {
                return Ok(current);
            }
            if !self.approval_matches_inner(&current, &pending)? {
                return self.invalidate_approval(&current, &Text::new(STALE_APPROVAL));
            }
            let transition = self.get_transition(pending.transition_id())?;
            current = Self::replace_state(
                &current,
                Some(None),
                Some(WorkflowStatus::Running),
                None,
                None,
                None,
            )?;
            return self.apply_transition(&current, &transition);
        }
        if current.status() == WorkflowStatus::Created {
            let previous = current.status();
            current = Self::replace_state(
                &current,
                None,
                Some(WorkflowStatus::Running),
                None,
                None,
                None,
            )?;
            self.emit(WorkflowEvent {
                event_type: Text::new("workflow.status_changed"),
                workflow_id: current.workflow_id().clone(),
                from_status: Some(previous),
                to_status: current.status(),
                current_state: current.current_state().clone(),
                transition_id: None,
                step_id: None,
                attempt: None,
                reason: None,
                approval: None,
            })?;
        }
        let selected = self.select_transition(&current, transition_id)?;
        let Some(selected) = selected else {
            return self.complete_if_terminal(current);
        };
        if selected.checkpoint_type().is_some() {
            let approval = self.make_approval(&current, &selected)?;
            let waiting = Self::replace_state(
                &current,
                Some(Some(approval.clone())),
                Some(WorkflowStatus::WaitingForApproval),
                None,
                None,
                None,
            )?;
            self.emit(WorkflowEvent {
                event_type: Text::new("workflow.checkpoint"),
                workflow_id: waiting.workflow_id().clone(),
                from_status: Some(current.status()),
                to_status: waiting.status(),
                current_state: waiting.current_state().clone(),
                transition_id: Some(Text::new(selected.transition_id())),
                step_id: Some(Text::new(selected.step_id())),
                attempt: None,
                reason: None,
                approval: Some(approval),
            })?;
            return Ok(waiting);
        }
        self.apply_transition(&current, &selected)
    }

    /// Apply one transition to a validated state (does not store).
    pub fn apply_transition(
        &self,
        state: &WorkflowState,
        transition: &WorkflowTransition,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.validated_state(state)?;
        if current.current_state().to_string_lossy() != transition.from_state() {
            return Err(transition_error(
                "transition is not legal from the current state",
            ));
        }
        if !self
            .definition
            .available_step_ids()
            .iter()
            .any(|step| step.as_str() == transition.step_id())
        {
            return Err(transition_error(
                "transition step is not declared by the workflow",
            ));
        }
        let attempt =
            self.attempt_count(&current, &Text::new(transition.step_id())) + BigInt::from(1);
        if attempt > self.attempt_limit(&Text::new(transition.step_id())) {
            return self.fail_state(
                &current,
                &Text::new(RETRY_BOUND_EXHAUSTED),
                &Text::new(transition.step_id()),
            );
        }
        let attempt_record =
            StepAttempt::from_parts(Text::new(transition.step_id()), attempt.clone(), true, None)?;
        let history_record = WorkflowTransitionRecord::from_parts(
            BigInt::from(current.history().len() + 1),
            Text::new(transition.transition_id()),
            Text::new(transition.from_state()),
            Text::new(transition.to_state()),
            Text::new(transition.step_id()),
            attempt.clone(),
        )?;
        let next_status = if self.outgoing(&Text::new(transition.to_state())).is_empty() {
            WorkflowStatus::Completed
        } else {
            WorkflowStatus::Running
        };
        let mut completed = current.completed_step_ids().to_vec();
        completed.push(Text::new(transition.step_id()));
        let mut attempts = current.attempts().to_vec();
        attempts.push(attempt_record);
        let mut history = current.history().to_vec();
        history.push(history_record);
        let updated = Self::replace_state(
            &current,
            None,
            Some(next_status),
            Some(Text::new(transition.to_state())),
            None,
            Some(None),
        )?
        .with_records(completed, attempts, history)?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.transition"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: Some(Text::new(transition.transition_id())),
            step_id: Some(Text::new(transition.step_id())),
            attempt: Some(attempt),
            reason: None,
            approval: None,
        })?;
        Ok(updated)
    }

    /// Record a failed attempt (does not store).
    pub fn record_failure(
        &self,
        state: &WorkflowState,
        transition: &WorkflowTransition,
        error_code: Option<&Text>,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.validated_state(state)?;
        let step_id = Text::new(transition.step_id());
        let attempt = self.attempt_count(&current, &step_id) + BigInt::from(1);
        if attempt > self.attempt_limit(&step_id) {
            return self.fail_state(&current, &Text::new(RETRY_BOUND_EXHAUSTED), &step_id);
        }
        let attempt_record =
            StepAttempt::from_parts(step_id.clone(), attempt.clone(), false, error_code.cloned())?;
        let mut attempts = current.attempts().to_vec();
        attempts.push(attempt_record);
        let updated = Self::replace_state(
            &current,
            None,
            Some(WorkflowStatus::Running),
            None,
            None,
            Some(error_code.cloned()),
        )?
        .with_records(
            current.completed_step_ids().to_vec(),
            attempts,
            current.history().to_vec(),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.step_retry"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: Some(Text::new(transition.transition_id())),
            step_id: Some(step_id.clone()),
            attempt: Some(attempt.clone()),
            reason: error_code.cloned(),
            approval: None,
        })?;
        if attempt >= self.attempt_limit(&step_id) {
            let resolved = error_code
                .cloned()
                .unwrap_or_else(|| Text::new(STEP_FAILED));
            return self.fail_state(&updated, &resolved, &step_id);
        }
        Ok(updated)
    }

    /// Mark a state failed and emit the failure event.
    pub fn fail_state(
        &self,
        state: &WorkflowState,
        reason: &Text,
        step_id: &Text,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.validated_state(state)?;
        let failed = Self::replace_state(
            &current,
            None,
            Some(WorkflowStatus::Failed),
            None,
            None,
            Some(Some(reason.clone())),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.failed"),
            workflow_id: failed.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: WorkflowStatus::Failed,
            current_state: failed.current_state().clone(),
            transition_id: None,
            step_id: Some(step_id.clone()),
            attempt: None,
            reason: Some(reason.clone()),
            approval: None,
        })?;
        Ok(failed)
    }

    /// Complete a non-terminal state when no transition remains.
    pub fn complete_if_terminal(
        &self,
        state: WorkflowState,
    ) -> Result<WorkflowState, WorkflowError> {
        let current = self.validated_state(&state)?;
        if matches!(
            current.status(),
            WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled
        ) {
            return Ok(current);
        }
        let completed = Self::replace_state(
            &current,
            None,
            Some(WorkflowStatus::Completed),
            None,
            None,
            Some(None),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.completed"),
            workflow_id: completed.workflow_id().clone(),
            from_status: Some(current.status()),
            to_status: WorkflowStatus::Completed,
            current_state: completed.current_state().clone(),
            transition_id: None,
            step_id: None,
            attempt: None,
            reason: None,
            approval: None,
        })?;
        Ok(completed)
    }

    /// Invalidate the pending approval and store the updated state.
    pub fn invalidate_approval(
        &self,
        state: &WorkflowState,
        reason: &Text,
    ) -> Result<WorkflowState, WorkflowError> {
        let Some(pending) = state.pending_approval().cloned() else {
            return Ok(state.clone());
        };
        let actor = pending
            .actor_id()
            .cloned()
            .unwrap_or_else(|| Text::new("local-user"));
        let decided_at = pending
            .decided_at()
            .unwrap_or_else(|| self.resolve_now(None));
        let invalidated = Self::approval_with(
            &pending,
            ApprovalStatus::Invalidated,
            Some(actor),
            Some(decided_at),
            Some(reason.clone()),
        )?;
        let updated = Self::replace_state(
            state,
            Some(Some(invalidated.clone())),
            Some(WorkflowStatus::WaitingForApproval),
            None,
            None,
            Some(Some(reason.clone())),
        )?;
        self.emit(WorkflowEvent {
            event_type: Text::new("workflow.approval_invalidated"),
            workflow_id: updated.workflow_id().clone(),
            from_status: Some(state.status()),
            to_status: updated.status(),
            current_state: updated.current_state().clone(),
            transition_id: Some(pending.transition_id().clone()),
            step_id: Some(pending.step_id().clone()),
            attempt: None,
            reason: Some(reason.clone()),
            approval: Some(invalidated),
        })?;
        Ok(self.store(updated))
    }

    /// Build a pending approval for a checkpoint transition.
    pub fn make_approval(
        &self,
        state: &WorkflowState,
        transition: &WorkflowTransition,
    ) -> Result<WorkflowApproval, WorkflowError> {
        let state_digest = self.state_digest_of(state)?;
        let action_digest = self.action_digest(transition, &state_digest)?;
        // Build the id from code points so a lone surrogate in a workflow id
        // survives exactly as it does in the Python f-string.
        let mut approval_points: Vec<u32> = "approval-".chars().map(u32::from).collect();
        approval_points.extend(state.workflow_id().code_points().iter().copied());
        approval_points.push(u32::from(b'-'));
        approval_points.extend(transition.transition_id().chars().map(u32::from));
        let approval_id = Text::from_code_points(approval_points)
            .map_err(|error| transition_error(error.to_string()))?;
        let Some(checkpoint_type) = transition.checkpoint_type() else {
            return Err(transition_error(
                "transition does not declare a checkpoint type",
            ));
        };
        WorkflowApproval::from_parts(
            approval_id,
            Text::new(transition.transition_id()),
            Text::new(transition.step_id()),
            checkpoint_type,
            Text::new(&state_digest),
            Text::new(&action_digest),
            ApprovalStatus::Pending,
            None,
            None,
            None,
        )
    }

    /// Whether an approval still binds this state and transition.
    ///
    /// The Python method returns false for an unknown transition but lets a
    /// canonicalization failure propagate. This convenience wrapper reports
    /// such a failure as `false`; internal transitions use
    /// `approval_matches_inner` so the engine preserves the source error.
    pub fn approval_matches(&self, state: &WorkflowState, approval: &WorkflowApproval) -> bool {
        self.approval_matches_inner(state, approval)
            .unwrap_or(false)
    }

    fn approval_matches_inner(
        &self,
        state: &WorkflowState,
        approval: &WorkflowApproval,
    ) -> Result<bool, WorkflowError> {
        let Ok(transition) = self.get_transition(approval.transition_id()) else {
            return Ok(false);
        };
        if transition.from_state() != state.current_state().to_string_lossy() {
            return Ok(false);
        }
        let current_state_digest = self.state_digest_of(state)?;
        let expected_action_digest = self.action_digest(&transition, &current_state_digest)?;
        Ok(
            approval.state_digest().to_string_lossy() == current_state_digest
                && approval.action_digest().to_string_lossy() == expected_action_digest,
        )
    }

    /// Select the requested outgoing transition (or the first outgoing one).
    pub fn select_transition(
        &self,
        state: &WorkflowState,
        transition_id: Option<&Text>,
    ) -> Result<Option<WorkflowTransition>, WorkflowError> {
        let outgoing = self.outgoing(state.current_state());
        let Some(transition_id) = transition_id else {
            return Ok(outgoing.into_iter().next());
        };
        let normalized = safe_identifier(transition_id, "transition_id")?;
        for transition in &outgoing {
            if transition.transition_id() == normalized.to_string_lossy() {
                return Ok(Some(transition.clone()));
            }
        }
        Err(transition_error(
            "transition is not declared from the current state",
        ))
    }

    /// Find a transition by identifier.
    pub fn get_transition(
        &self,
        transition_id: &Text,
    ) -> Result<WorkflowTransition, WorkflowError> {
        let normalized = safe_identifier(transition_id, "transition_id")?;
        for transition in self.definition.transitions() {
            if transition.transition_id() == normalized.to_string_lossy() {
                return Ok(transition.clone());
            }
        }
        Err(transition_error(
            "transition is not declared by the workflow",
        ))
    }

    /// Outgoing transitions for a state.
    pub fn outgoing(&self, state: &Text) -> Vec<WorkflowTransition> {
        let state = state.to_string_lossy();
        self.definition
            .transitions()
            .iter()
            .filter(|transition| transition.from_state() == state)
            .cloned()
            .collect()
    }

    /// Attempts recorded for a step.
    pub fn attempt_count(&self, state: &WorkflowState, step_id: &Text) -> BigInt {
        let step = step_id.to_string_lossy();
        BigInt::from(
            state
                .attempts()
                .iter()
                .filter(|attempt| attempt.step_id().to_string_lossy() == step)
                .count(),
        )
    }

    /// The retry bound for a step (default one).
    pub fn attempt_limit(&self, step_id: &Text) -> BigInt {
        let normalized = step_id.to_string_lossy();
        self.max_attempts
            .iter()
            .find(|(key, _)| key.to_string_lossy() == normalized)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| BigInt::from(1))
    }

    fn validated_state(&self, state: &WorkflowState) -> Result<WorkflowState, WorkflowError> {
        if state.definition_id().to_string_lossy() != self.definition.definition_id() {
            return Err(transition_error(
                "state definition_id does not match the engine",
            ));
        }
        if state.definition_version().to_string_lossy()
            != self.definition.version().to_string_lossy()
        {
            return Err(transition_error(
                "state definition version does not match the engine",
            ));
        }
        let current = state.current_state().to_string_lossy();
        if !self
            .definition
            .states()
            .iter()
            .any(|candidate| candidate.as_str() == current)
        {
            return Err(transition_error(
                "state current_state is not declared by the definition",
            ));
        }
        Ok(state.clone())
    }

    fn require_state(&self, state: Option<&WorkflowState>) -> Result<WorkflowState, WorkflowError> {
        if let Some(state) = state {
            return self.validated_state(state);
        }
        let guard = self.lock_state();
        guard
            .clone()
            .ok_or_else(|| transition_error("workflow has not been started"))
    }

    fn validate_restored(&self, state: &WorkflowState) -> Result<(), WorkflowError> {
        self.validated_state(state)?;
        if state.replanning_attempts() > self.definition.max_replanning_attempts().as_bigint() {
            return Err(transition_error(
                "state replanning attempts exceed the workflow definition limit",
            ));
        }
        for step_id in state.completed_step_ids() {
            if !self.step_declared(step_id) {
                return Err(transition_error(
                    "state completed_step_id is not declared by the workflow",
                ));
            }
        }
        for attempt in state.attempts() {
            if !self.step_declared(attempt.step_id()) {
                return Err(transition_error(
                    "state attempt step is not declared by the workflow",
                ));
            }
        }
        if state.status() == WorkflowStatus::Created && !state.history().is_empty() {
            return Err(transition_error(
                "created workflows cannot have transition history",
            ));
        }
        let successes: Vec<&StepAttempt> = state
            .attempts()
            .iter()
            .filter(|attempt| attempt.succeeded())
            .collect();
        if successes.len() != state.history().len() {
            return Err(transition_error(
                "state successful attempts do not match transition history",
            ));
        }
        for (attempt, record) in successes.iter().zip(state.history()) {
            if attempt.step_id() != record.step_id() || attempt.attempt() != record.attempt() {
                return Err(transition_error(
                    "state successful attempts do not match transition history",
                ));
            }
        }
        let completed_from_attempts: Vec<Text> = successes
            .iter()
            .map(|attempt| attempt.step_id().clone())
            .collect();
        if completed_from_attempts != state.completed_step_ids() {
            return Err(transition_error(
                "state completed steps do not match successful attempts",
            ));
        }
        let mut expected = Text::new(self.definition.initial_state());
        for (index, record) in state.history().iter().enumerate() {
            if record.sequence() != &BigInt::from(index + 1) {
                return Err(transition_error(
                    "state transition history is not contiguous",
                ));
            }
            let transition = self.get_transition(record.transition_id())?;
            if record.from_state() != &expected
                || transition.from_state() != record.from_state().to_string_lossy()
                || transition.to_state() != record.to_state().to_string_lossy()
                || transition.step_id() != record.step_id().to_string_lossy()
            {
                return Err(transition_error(
                    "state transition history does not match the definition",
                ));
            }
            expected = record.to_state().clone();
        }
        if expected != *state.current_state() {
            return Err(transition_error(
                "state current_state does not match the transition history",
            ));
        }
        if let Some(pending) = state.pending_approval() {
            let transition = self.get_transition(pending.transition_id())?;
            if transition.step_id() != pending.step_id().to_string_lossy()
                || transition.from_state() != state.current_state().to_string_lossy()
                || transition.checkpoint_type() != Some(pending.checkpoint_type())
            {
                return Err(transition_error(
                    "state pending approval does not match the definition",
                ));
            }
        }
        Ok(())
    }

    fn step_declared(&self, step_id: &Text) -> bool {
        let value = step_id.to_string_lossy();
        self.definition
            .available_step_ids()
            .iter()
            .any(|step| step.as_str() == value)
    }

    fn replace_state(
        state: &WorkflowState,
        pending: Option<Option<WorkflowApproval>>,
        status: Option<WorkflowStatus>,
        current_state: Option<Text>,
        replanning_attempts: Option<BigInt>,
        last_error: Option<Option<Text>>,
    ) -> Result<WorkflowState, WorkflowError> {
        WorkflowState::from_parts(
            state.workflow_id().clone(),
            state.definition_id().clone(),
            state.definition_version().clone(),
            status.unwrap_or(state.status()),
            current_state.unwrap_or_else(|| state.current_state().clone()),
            state.completed_step_ids().to_vec(),
            state.attempts().to_vec(),
            pending.unwrap_or_else(|| state.pending_approval().cloned()),
            replanning_attempts.unwrap_or_else(|| state.replanning_attempts().clone()),
            state.history().to_vec(),
            last_error.unwrap_or_else(|| state.last_error().cloned()),
        )
    }

    fn approval_with(
        approval: &WorkflowApproval,
        status: ApprovalStatus,
        actor_id: Option<Text>,
        decided_at: Option<KernelTimestamp>,
        reason: Option<Text>,
    ) -> Result<WorkflowApproval, WorkflowError> {
        WorkflowApproval::from_parts(
            approval.approval_id().clone(),
            approval.transition_id().clone(),
            approval.step_id().clone(),
            approval.checkpoint_type(),
            approval.state_digest().clone(),
            approval.action_digest().clone(),
            status,
            actor_id,
            decided_at,
            reason,
        )
    }
}

impl WorkflowState {
    fn with_records(
        self,
        completed_step_ids: Vec<Text>,
        attempts: Vec<StepAttempt>,
        history: Vec<WorkflowTransitionRecord>,
    ) -> Result<Self, WorkflowError> {
        WorkflowState::from_parts(
            self.workflow_id().clone(),
            self.definition_id().clone(),
            self.definition_version().clone(),
            self.status(),
            self.current_state().clone(),
            completed_step_ids,
            attempts,
            self.pending_approval().cloned(),
            self.replanning_attempts().clone(),
            history,
            self.last_error().cloned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kala_contracts::ContractModel;

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition::from_json_str(
            r#"{
                "definition_id":"workflow-def",
                "version":"1.0.0",
                "name":"Single step",
                "initial_state":"start",
                "states":["start","done"],
                "transitions":[
                    {"transition_id":"transition-1","from_state":"start","to_state":"done","step_id":"step-1"}
                ],
                "available_step_ids":["step-1"],
                "max_replanning_attempts":1
            }"#,
        )
        .expect("definition")
    }

    #[test]
    fn start_and_run_complete_a_single_step() {
        let engine = WorkflowEngine::new(definition());
        let state = engine.start(&Text::new("workflow-1"), None).expect("start");
        assert_eq!(state.status(), WorkflowStatus::Created);
        let completed = engine.run(None, None, None, None).expect("run");
        assert_eq!(completed.status(), WorkflowStatus::Completed);
        assert_eq!(completed.current_state().to_string_lossy(), "done");
    }

    #[test]
    fn state_digest_is_deterministic_and_binds_attempts() {
        let engine = WorkflowEngine::new(definition());
        let state = engine.start(&Text::new("workflow-1"), None).expect("start");
        let first = engine.state_digest_of(&state).expect("digest");
        let second = engine.state_digest_of(&state).expect("digest");
        assert_eq!(first, second);
        let completed = engine
            .record_step_result(&state, &Text::new("step-1"), true, None, None)
            .expect("result");
        assert_ne!(engine.state_digest_of(&completed).expect("digest"), first);
    }

    #[test]
    fn restore_rejects_inconsistent_history() {
        let engine = WorkflowEngine::new(definition());
        let state = engine.start(&Text::new("workflow-1"), None).expect("start");
        let mut history = state.history().to_vec();
        history.push(
            WorkflowTransitionRecord::from_parts(
                BigInt::from(1),
                Text::new("transition-1"),
                Text::new("start"),
                Text::new("done"),
                Text::new("step-1"),
                BigInt::from(1),
            )
            .expect("record"),
        );
        let forged = WorkflowState::from_parts(
            state.workflow_id().clone(),
            state.definition_id().clone(),
            state.definition_version().clone(),
            WorkflowStatus::Running,
            Text::new("start"),
            Vec::new(),
            Vec::new(),
            None,
            BigInt::from(0),
            history,
            None,
        )
        .expect("state");
        assert!(engine.restore(forged).is_err());
    }
}
