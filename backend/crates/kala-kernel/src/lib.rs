//! Native policy, workflow and planning state machinery for the Kala Agent
//! kernel.
//!
//! This crate is a standalone workspace port of
//! `kala_agent.kernel.policy`, `kala_agent.kernel.workflow` and
//! `kala_agent.kernel.planning`. It consumes the accepted `kala-contracts`
//! typed models and the accepted `kala-json` canonical JSON implementation
//! read-only. It does not compose persistence, memory, tools, runtime or HTTP,
//! and it never invokes Python in production; the live Python differential is
//! test-only.
//!
//! Dynamic state and request boundaries use [`kala_json::CanonicalValue`] and
//! [`kala_contracts::Text`] so Python strings (including lone surrogates),
//! arbitrary integers, strict booleans, `null` and timestamps retain their
//! exact domains. Canonical state/action digest bytes are produced by
//! `kala-json`, not by serde or a float/private-number shortcut.

#![forbid(unsafe_code)]

mod json_input;
pub mod planning;
pub mod policy;
pub mod time;
mod value;
pub mod workflow;

pub use json_input::{ModelJsonError, parse_model_json};
// Accepted contract types used by the public signatures. Re-exporting them
// keeps native consumers from having to pin a second direct dependency just to
// name a request's side-effect level or a plan step.
pub use kala_contracts::{
    ApprovalKind, ApprovalStatus, PlanStep, SideEffectLevel, Text, WorkflowDefinition,
    WorkflowStatus, WorkflowTransition,
};
pub use kala_json::{CanonicalObject, CanonicalValue, Sha256Digest};
pub use planning::{
    CapabilitySet, NormalizedPlan, Plan, PlanConfigError, PlanRejectedError, PlanValidationCode,
    PlanValidationError, PlanValidator, ValidatedPlan, capability_set, combine_capability_sets,
    find_cycle, topological_order,
};
pub use policy::{
    ApprovalDecision, ApprovalKey, ApprovalRecord, ApprovalRequest, CapabilityGrant,
    DEFAULT_MAX_TRACKED_REQUESTS, DecisionContext, DecisionStatus, GrantSpec, MAX_NOTE_LENGTH,
    POLICY_STATE_VERSION, PolicyApproval, PolicyApprovalRequest, PolicyConfigError, PolicyDecision,
    PolicyDecisionStatus, PolicyEngine, PolicyError, PolicyReason, PolicyRequest,
    PolicyRequestError, PolicyRequestModel, PolicyState, PolicyStateError, PolicyStatus,
    PolicyToolClass, PolicyTransitionError, ToolClassKey,
};
pub use time::{IsoformatError, KernelTimestamp};
pub use value::{canonical_state_digest, canonical_state_json};
pub use workflow::{
    CheckpointContext, CollectingSink, EventCallbackSink, EventJournalCallback,
    JournalCallbackSink, PayloadCallbackSink, StepAttempt, WorkflowApproval, WorkflowApprovalError,
    WorkflowCheckpoint, WorkflowCheckpointStatus, WorkflowEngine, WorkflowError, WorkflowEvent,
    WorkflowEventCallback, WorkflowEventSink, WorkflowLimitExceeded, WorkflowState,
    WorkflowStateStore, WorkflowTransitionError, WorkflowTransitionRecord,
};
