//! The application's read models.
//!
//! Every type is a one-for-one port of one `_ViewModel` subclass in
//! `kala_agent/application.py`. Each exposes three forms, deliberately:
//!
//! * the typed Rust value (used in process),
//! * [`View::to_wire_value`] / [`View::to_wire_json`], the
//!   `model_dump(mode="json")` shape in declaration order, which is what the
//!   API returns, and
//! * [`View::to_canonical_json`], the sorted/ASCII form used for digests and
//!   differential comparison.
//!
//! The wire form must never be hashed and the canonical form must never be
//! sent on the wire.

use kala_contracts::{
    ApprovalKind, ApprovalStatus, ContractModel, DesignRevision, EvaluationResult, Provenance,
    Timestamp, WireValue, WorkflowStatus,
};
use kala_json::CanonicalValue;
use kala_kernel::policy::PolicyDecisionStatus;
use num_bigint::BigInt;

use crate::error::ApplicationError;
use crate::value::{
    wire_approval_kind, wire_approval_status, wire_bool, wire_integer, wire_opt_float,
    wire_opt_text, wire_opt_timestamp, wire_policy_status, wire_text, wire_texts, wire_timestamp,
    wire_value, wire_workflow_status,
};

/// Helpers shared by every view.
pub trait View {
    /// The `model_dump(mode="json")` shape in declaration order.
    fn to_wire_value(&self) -> WireValue;

    /// Compact UTF-8 model JSON in declaration order (the HTTP form).
    ///
    /// # Errors
    ///
    /// Fails only when a value cannot be encoded, which for these validated
    /// views cannot happen.
    fn to_wire_json(&self) -> Result<String, ApplicationError> {
        self.to_wire_value()
            .encode(false)
            .map_err(|error| ApplicationError::application(error.to_string()))
    }

    /// Compact ASCII model JSON in declaration order.
    fn to_wire_json_ascii(&self) -> Result<String, ApplicationError> {
        self.to_wire_value()
            .encode(true)
            .map_err(|error| ApplicationError::application(error.to_string()))
    }

    /// The canonical (sorted-key, ASCII) JSON value.
    fn to_canonical_value(&self) -> Result<CanonicalValue, ApplicationError> {
        self.to_wire_value()
            .to_canonical_value()
            .map_err(|error| ApplicationError::application(error.to_string()))
    }

    /// The canonical (sorted-key, ASCII) JSON text.
    ///
    /// A read view may hold a full event page, so it is not bounded by the
    /// tool-protocol size here; transports should still send the wire form.
    fn to_canonical_json(&self) -> Result<String, ApplicationError> {
        crate::value::canonical_dumps(&self.to_canonical_value()?, usize::MAX)
    }
}

// ---------------------------------------------------------------------------
// Domain declarations
// ---------------------------------------------------------------------------

/// An inspectable summary of one registered domain package.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainPackageView {
    /// Registry domain name.
    pub domain: String,
    /// Package identifier.
    pub package_id: String,
    /// Package version.
    pub version: String,
    /// Native package digest.
    pub digest: String,
    /// Package description.
    pub description: String,
    /// Package disclaimer.
    pub disclaimer: String,
    /// Declared workflow definition id.
    pub workflow_definition_id: String,
    /// Reference subject identifier.
    pub reference_subject_id: String,
    /// Parameter-schema digests.
    pub schema_digests: Vec<String>,
    /// Package-level required capabilities.
    pub required_capabilities: Vec<String>,
    /// Catalog step ids used by the reference workflow.
    pub catalog_step_ids: Vec<String>,
}

impl View for DomainPackageView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", wire_text(&self.domain)),
            ("package_id", wire_text(&self.package_id)),
            ("version", wire_text(&self.version)),
            ("digest", wire_text(&self.digest)),
            ("description", wire_text(&self.description)),
            ("disclaimer", wire_text(&self.disclaimer)),
            (
                "workflow_definition_id",
                wire_text(&self.workflow_definition_id),
            ),
            (
                "reference_subject_id",
                wire_text(&self.reference_subject_id),
            ),
            ("schema_digests", wire_texts(&self.schema_digests)),
            (
                "required_capabilities",
                wire_texts(&self.required_capabilities),
            ),
            ("catalog_step_ids", wire_texts(&self.catalog_step_ids)),
        ])
    }
}

/// One declared catalog step of a domain package.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogStepView {
    /// Step identifier.
    pub step_id: String,
    /// Human-readable name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Declared input parameter names.
    pub input_names: Vec<String>,
    /// Declared metric names.
    pub metric_names: Vec<String>,
    /// Capabilities the step requires.
    pub required_capabilities: Vec<String>,
    /// Checkpoint kind declared by the step, if any.
    pub checkpoint_kind: Option<ApprovalKind>,
    /// Maximum attempts.
    pub max_attempts: u32,
}

impl View for CatalogStepView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("step_id", wire_text(&self.step_id)),
            ("name", wire_text(&self.name)),
            ("description", wire_text(&self.description)),
            ("input_names", wire_texts(&self.input_names)),
            ("metric_names", wire_texts(&self.metric_names)),
            (
                "required_capabilities",
                wire_texts(&self.required_capabilities),
            ),
            ("checkpoint_kind", wire_approval_kind(self.checkpoint_kind)),
            (
                "max_attempts",
                wire_integer(&BigInt::from(self.max_attempts)),
            ),
        ])
    }
}

/// One declared transition of a workflow definition.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTransitionView {
    /// Transition identifier.
    pub transition_id: String,
    /// Source state.
    pub from_state: String,
    /// Target state.
    pub to_state: String,
    /// Step the transition runs.
    pub step_id: String,
    /// Checkpoint kind the transition carries, if any.
    pub checkpoint_type: Option<ApprovalKind>,
}

impl View for WorkflowTransitionView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("transition_id", wire_text(&self.transition_id)),
            ("from_state", wire_text(&self.from_state)),
            ("to_state", wire_text(&self.to_state)),
            ("step_id", wire_text(&self.step_id)),
            ("checkpoint_type", wire_approval_kind(self.checkpoint_type)),
        ])
    }
}

/// A summary of one declared workflow definition.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowDefinitionView {
    /// Registry domain name.
    pub domain: String,
    /// Definition identifier.
    pub definition_id: String,
    /// Definition version.
    pub version: String,
    /// Name.
    pub name: String,
    /// Description, if declared.
    pub description: Option<String>,
    /// Initial state.
    pub initial_state: String,
    /// Declared states.
    pub states: Vec<String>,
    /// Catalog steps the definition makes available.
    pub available_step_ids: Vec<String>,
    /// Declared transitions.
    pub transitions: Vec<WorkflowTransitionView>,
    /// Replanning attempt bound.
    pub max_replanning_attempts: BigInt,
}

impl View for WorkflowDefinitionView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", wire_text(&self.domain)),
            ("definition_id", wire_text(&self.definition_id)),
            ("version", wire_text(&self.version)),
            ("name", wire_text(&self.name)),
            ("description", wire_opt_text(self.description.as_deref())),
            ("initial_state", wire_text(&self.initial_state)),
            ("states", wire_texts(&self.states)),
            ("available_step_ids", wire_texts(&self.available_step_ids)),
            (
                "transitions",
                WireValue::Array(
                    self.transitions
                        .iter()
                        .map(WorkflowTransitionView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "max_replanning_attempts",
                wire_integer(&self.max_replanning_attempts),
            ),
        ])
    }
}

/// One domain package with its workflow and declared catalog steps.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainDetail {
    /// The package declaration.
    pub package: DomainPackageView,
    /// The workflow declaration.
    pub workflow: WorkflowDefinitionView,
    /// The catalog steps.
    pub catalog_steps: Vec<CatalogStepView>,
}

impl View for DomainDetail {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("package", self.package.to_wire_value()),
            ("workflow", self.workflow.to_wire_value()),
            (
                "catalog_steps",
                WireValue::Array(
                    self.catalog_steps
                        .iter()
                        .map(CatalogStepView::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

/// The deterministic outcome of one reference design evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainEvaluationReport {
    /// Registry domain name.
    pub domain: String,
    /// Package identifier.
    pub package_id: String,
    /// Package version.
    pub package_version: String,
    /// Native package digest.
    pub package_digest: String,
    /// Subject being evaluated.
    pub subject_id: String,
    /// Workflow status of the reference run.
    pub status: WorkflowStatus,
    /// Whether every hard constraint passed.
    pub passed: bool,
    /// Mean bounded score, when scored.
    pub score: Option<f64>,
    /// Digest of the parameter model dump.
    pub parameters_digest: String,
    /// Steps that completed.
    pub completed_step_ids: Vec<String>,
    /// Step that stopped the run, if one did.
    pub blocked_step_id: Option<String>,
    /// One evaluation per executed step.
    pub evaluations: Vec<EvaluationResult>,
    /// Every finding, in order.
    pub findings: Vec<String>,
    /// Package disclaimer.
    pub disclaimer: String,
}

impl View for DomainEvaluationReport {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", wire_text(&self.domain)),
            ("package_id", wire_text(&self.package_id)),
            ("package_version", wire_text(&self.package_version)),
            ("package_digest", wire_text(&self.package_digest)),
            ("subject_id", wire_text(&self.subject_id)),
            ("status", wire_workflow_status(self.status)),
            ("passed", wire_bool(self.passed)),
            ("score", wire_opt_float(self.score)),
            ("parameters_digest", wire_text(&self.parameters_digest)),
            ("completed_step_ids", wire_texts(&self.completed_step_ids)),
            (
                "blocked_step_id",
                wire_opt_text(self.blocked_step_id.as_deref()),
            ),
            (
                "evaluations",
                WireValue::Array(
                    self.evaluations
                        .iter()
                        .map(EvaluationResult::to_wire_value)
                        .collect(),
                ),
            ),
            ("findings", wire_texts(&self.findings)),
            ("disclaimer", wire_text(&self.disclaimer)),
        ])
    }
}

// ---------------------------------------------------------------------------
// Workflow state
// ---------------------------------------------------------------------------

/// One recorded step attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct StepAttemptView {
    /// Step identifier.
    pub step_id: String,
    /// Attempt number.
    pub attempt: BigInt,
    /// Whether the attempt succeeded.
    pub succeeded: bool,
    /// Failure code, if any.
    pub error_code: Option<String>,
}

impl View for StepAttemptView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("step_id", wire_text(&self.step_id)),
            ("attempt", wire_integer(&self.attempt)),
            ("succeeded", wire_bool(self.succeeded)),
            ("error_code", wire_opt_text(self.error_code.as_deref())),
        ])
    }
}

/// One recorded transition.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTransitionRecordView {
    /// Transition sequence.
    pub sequence: BigInt,
    /// Transition identifier.
    pub transition_id: String,
    /// Source state.
    pub from_state: String,
    /// Target state.
    pub to_state: String,
    /// Step identifier.
    pub step_id: String,
    /// Attempt number.
    pub attempt: BigInt,
}

impl View for WorkflowTransitionRecordView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("sequence", wire_integer(&self.sequence)),
            ("transition_id", wire_text(&self.transition_id)),
            ("from_state", wire_text(&self.from_state)),
            ("to_state", wire_text(&self.to_state)),
            ("step_id", wire_text(&self.step_id)),
            ("attempt", wire_integer(&self.attempt)),
        ])
    }
}

/// A policy checkpoint awaiting or carrying a local decision.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointView {
    /// Approval identifier.
    pub approval_id: String,
    /// Transition identifier.
    pub transition_id: String,
    /// Step identifier.
    pub step_id: String,
    /// Checkpoint kind.
    pub checkpoint_type: ApprovalKind,
    /// Bound state digest.
    pub state_digest: String,
    /// Bound action digest.
    pub action_digest: String,
    /// Approval status.
    pub status: ApprovalStatus,
    /// Deciding actor, if decided.
    pub actor_id: Option<String>,
    /// Decision timestamp, if decided.
    pub decided_at: Option<Timestamp>,
    /// Decision note, if any.
    pub reason: Option<String>,
}

impl View for CheckpointView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("approval_id", wire_text(&self.approval_id)),
            ("transition_id", wire_text(&self.transition_id)),
            ("step_id", wire_text(&self.step_id)),
            (
                "checkpoint_type",
                wire_approval_kind(Some(self.checkpoint_type)),
            ),
            ("state_digest", wire_text(&self.state_digest)),
            ("action_digest", wire_text(&self.action_digest)),
            ("status", wire_approval_status(self.status)),
            ("actor_id", wire_opt_text(self.actor_id.as_deref())),
            ("decided_at", wire_opt_timestamp(self.decided_at.as_ref())),
            ("reason", wire_opt_text(self.reason.as_deref())),
        ])
    }
}

/// A bounded, serializable mirror of the kernel workflow state.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowStateView {
    /// Workflow identifier.
    pub workflow_id: String,
    /// Definition identifier.
    pub definition_id: String,
    /// Definition version.
    pub definition_version: String,
    /// Workflow status.
    pub status: WorkflowStatus,
    /// Current state name.
    pub current_state: String,
    /// Steps that completed.
    pub completed_step_ids: Vec<String>,
    /// Recorded attempts.
    pub attempts: Vec<StepAttemptView>,
    /// Pending checkpoint, if any.
    pub pending_approval: Option<CheckpointView>,
    /// Replanning attempts used.
    pub replanning_attempts: BigInt,
    /// Transition history.
    pub history: Vec<WorkflowTransitionRecordView>,
    /// Last recorded failure, if any.
    pub last_error: Option<String>,
}

impl View for WorkflowStateView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("workflow_id", wire_text(&self.workflow_id)),
            ("definition_id", wire_text(&self.definition_id)),
            ("definition_version", wire_text(&self.definition_version)),
            ("status", wire_workflow_status(self.status)),
            ("current_state", wire_text(&self.current_state)),
            ("completed_step_ids", wire_texts(&self.completed_step_ids)),
            (
                "attempts",
                WireValue::Array(
                    self.attempts
                        .iter()
                        .map(StepAttemptView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "pending_approval",
                match &self.pending_approval {
                    Some(checkpoint) => checkpoint.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "replanning_attempts",
                wire_integer(&self.replanning_attempts),
            ),
            (
                "history",
                WireValue::Array(
                    self.history
                        .iter()
                        .map(WorkflowTransitionRecordView::to_wire_value)
                        .collect(),
                ),
            ),
            ("last_error", wire_opt_text(self.last_error.as_deref())),
        ])
    }
}

// ---------------------------------------------------------------------------
// Budget, revisions and runs
// ---------------------------------------------------------------------------

/// The shared budget usage as reported by one operation.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetUsageView {
    /// Model calls charged.
    pub model_calls: BigInt,
    /// Tool calls charged.
    pub tool_calls: BigInt,
    /// Total tokens charged.
    pub total_tokens: BigInt,
    /// Artifact bytes charged.
    pub artifact_bytes: BigInt,
    /// Duration charged, in seconds.
    pub duration_seconds: f64,
    /// Exact cost as a Python `Decimal` string.
    pub cost: String,
    /// Replanning attempts charged.
    pub replanning_attempts: BigInt,
}

impl View for BudgetUsageView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("model_calls", wire_integer(&self.model_calls)),
            ("tool_calls", wire_integer(&self.tool_calls)),
            ("total_tokens", wire_integer(&self.total_tokens)),
            ("artifact_bytes", wire_integer(&self.artifact_bytes)),
            ("duration_seconds", WireValue::Float(self.duration_seconds)),
            ("cost", wire_text(&self.cost)),
            (
                "replanning_attempts",
                wire_integer(&self.replanning_attempts),
            ),
        ])
    }
}

/// An immutable revision together with its provenance record.
#[derive(Clone, Debug, PartialEq)]
pub struct RevisionRecord {
    /// The stored revision.
    pub revision: DesignRevision,
    /// The journalled provenance.
    pub provenance: Provenance,
    /// Digests of the revision artifacts.
    pub artifact_digests: Vec<String>,
}

impl View for RevisionRecord {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("revision", self.revision.to_wire_value()),
            ("provenance", self.provenance.to_wire_value()),
            ("artifact_digests", wire_texts(&self.artifact_digests)),
        ])
    }
}

/// One prepared revision commit and the decision recorded against it.
#[derive(Clone, Debug, PartialEq)]
pub struct RevisionCommitView {
    /// Commit request identifier.
    pub request_id: String,
    /// Design identifier.
    pub design_id: String,
    /// Registry domain name.
    pub domain: String,
    /// Planned revision identifier.
    pub revision_id: String,
    /// Planned sequence.
    pub sequence: BigInt,
    /// Parent revision, if any.
    pub parent_revision_id: Option<String>,
    /// Bound state digest.
    pub state_digest: String,
    /// Bound action digest.
    pub action_digest: String,
    /// Digest of the design specification.
    pub spec_digest: String,
    /// Digest of the parameter model dump.
    pub parameters_digest: String,
    /// Domain package digests.
    pub package_digests: Vec<String>,
    /// Report artifact digests.
    pub artifact_digests: Vec<String>,
    /// Authorizing workflow, if any.
    pub workflow_id: Option<String>,
    /// Approval status.
    pub status: ApprovalStatus,
    /// Deciding actor, if decided.
    pub actor_id: Option<String>,
    /// Decision timestamp, if decided.
    pub decided_at: Option<Timestamp>,
    /// Decision note, if any.
    pub reason: Option<String>,
    /// Preparation timestamp.
    pub created_at: Timestamp,
    /// Whether the revision was written.
    pub committed: bool,
}

impl View for RevisionCommitView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("request_id", wire_text(&self.request_id)),
            ("design_id", wire_text(&self.design_id)),
            ("domain", wire_text(&self.domain)),
            ("revision_id", wire_text(&self.revision_id)),
            ("sequence", wire_integer(&self.sequence)),
            (
                "parent_revision_id",
                wire_opt_text(self.parent_revision_id.as_deref()),
            ),
            ("state_digest", wire_text(&self.state_digest)),
            ("action_digest", wire_text(&self.action_digest)),
            ("spec_digest", wire_text(&self.spec_digest)),
            ("parameters_digest", wire_text(&self.parameters_digest)),
            ("package_digests", wire_texts(&self.package_digests)),
            ("artifact_digests", wire_texts(&self.artifact_digests)),
            ("workflow_id", wire_opt_text(self.workflow_id.as_deref())),
            ("status", wire_approval_status(self.status)),
            ("actor_id", wire_opt_text(self.actor_id.as_deref())),
            ("decided_at", wire_opt_timestamp(self.decided_at.as_ref())),
            ("reason", wire_opt_text(self.reason.as_deref())),
            ("created_at", wire_timestamp(&self.created_at)),
            ("committed", wire_bool(self.committed)),
        ])
    }
}

/// The outcome of one mechanical reference workflow run or decision.
#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalRunResult {
    /// Workflow identifier.
    pub workflow_id: String,
    /// Design identifier.
    pub design_id: String,
    /// Workflow status.
    pub status: WorkflowStatus,
    /// Full workflow state.
    pub state: WorkflowStateView,
    /// Evaluations recorded by the run.
    pub evaluations: Vec<EvaluationResult>,
    /// Checkpoint registered or resolved by this operation.
    pub checkpoint: Option<CheckpointView>,
    /// Policy decision status, when a policy decision was made.
    pub policy_status: Option<PolicyDecisionStatus>,
    /// Policy decision reason, when a policy decision was made.
    pub policy_reason: Option<String>,
    /// Planned task ids.
    pub planned_task_ids: Vec<String>,
    /// Whether this call started the workflow.
    pub started: bool,
    /// Revision written when the workflow completed.
    pub revision: Option<RevisionRecord>,
    /// Budget usage at the end of the operation.
    pub budget: BudgetUsageView,
}

impl View for MechanicalRunResult {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("workflow_id", wire_text(&self.workflow_id)),
            ("design_id", wire_text(&self.design_id)),
            ("status", wire_workflow_status(self.status)),
            ("state", self.state.to_wire_value()),
            (
                "evaluations",
                WireValue::Array(
                    self.evaluations
                        .iter()
                        .map(EvaluationResult::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "checkpoint",
                match &self.checkpoint {
                    Some(checkpoint) => checkpoint.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            ("policy_status", wire_policy_status(self.policy_status)),
            (
                "policy_reason",
                wire_opt_text(self.policy_reason.as_deref()),
            ),
            ("planned_task_ids", wire_texts(&self.planned_task_ids)),
            ("started", wire_bool(self.started)),
            (
                "revision",
                match &self.revision {
                    Some(record) => record.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            ("budget", self.budget.to_wire_value()),
        ])
    }
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// One journalled event as a bounded read view.
#[derive(Clone, Debug, PartialEq)]
pub struct EventView {
    /// One-based journal sequence.
    pub sequence: i64,
    /// Event type.
    pub event_type: String,
    /// Aggregate identifier.
    pub aggregate_id: String,
    /// Workflow identifier, when present.
    pub workflow_id: Option<String>,
    /// Run identifier, when present.
    pub run_id: Option<String>,
    /// Payload digest.
    pub digest: String,
    /// Occurred-at timestamp.
    pub occurred_at: Timestamp,
    /// Parsed payload, when published.
    pub payload: Option<CanonicalValue>,
    /// Whether the payload was withheld for size.
    pub payload_omitted: bool,
}

impl View for EventView {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("sequence", wire_integer(&BigInt::from(self.sequence))),
            ("event_type", wire_text(&self.event_type)),
            ("aggregate_id", wire_text(&self.aggregate_id)),
            ("workflow_id", wire_opt_text(self.workflow_id.as_deref())),
            ("run_id", wire_opt_text(self.run_id.as_deref())),
            ("digest", wire_text(&self.digest)),
            ("occurred_at", wire_timestamp(&self.occurred_at)),
            (
                "payload",
                match &self.payload {
                    Some(payload) => wire_value(payload),
                    None => WireValue::Null,
                },
            ),
            ("payload_omitted", wire_bool(self.payload_omitted)),
        ])
    }
}

/// A bounded page of journal events.
#[derive(Clone, Debug, PartialEq)]
pub struct EventPage {
    /// The events of this page.
    pub events: Vec<EventView>,
    /// Number of events in this page.
    pub count: usize,
    /// Requested exclusive lower sequence bound.
    pub after_sequence: i64,
    /// Sequence to pass as `after_sequence` for the next page.
    pub next_after_sequence: i64,
    /// Whether any event of this page withheld its payload.
    pub payload_truncated: bool,
}

impl View for EventPage {
    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            (
                "events",
                WireValue::Array(self.events.iter().map(EventView::to_wire_value).collect()),
            ),
            ("count", wire_integer(&BigInt::from(self.count))),
            (
                "after_sequence",
                wire_integer(&BigInt::from(self.after_sequence)),
            ),
            (
                "next_after_sequence",
                wire_integer(&BigInt::from(self.next_after_sequence)),
            ),
            ("payload_truncated", wire_bool(self.payload_truncated)),
        ])
    }
}
