//! The Kala local composition root.
//!
//! `kala-application` is the native counterpart of
//! `kala_agent/application.py`. It is the only crate that wires the kernel,
//! persistence, the bounded tool plane, the provider-neutral model gateway and
//! the declarative domain packages together. It owns composition and
//! lifecycle, and every rule it applies is a rule of a component it holds.
//!
//! The root is local and offline by construction. It builds a deterministic
//! in-process model client, grants only the capabilities the declared domain
//! packages need, and never opens a socket. Egress is reported from the client
//! this root actually holds, never from a declared setting.
//!
//! A design revision is written in two phases. Preparation derives the commit,
//! reduces it to a state digest and an action digest, stores the report bytes
//! content-addressed and opens a pre-commit checkpoint bound to both. The
//! commit resolves that checkpoint against the digests recomputed from the
//! state that exists at that moment and writes under a transactional head
//! compare-and-swap. A rejection, a stale approval, a revoked capability, a
//! fork and a lost race all leave the design exactly as it was.
//!
//! The policy engine and the budget ledger are recorded at the end of every
//! operation and restored at startup. Their records live under an aggregate
//! the kernel reserves for itself; a workflow identifier that names one of
//! them is refused before any work is done, and durable state that is missing,
//! unverifiable or invalid stops the launch rather than restoring defaults.
//!
//! # Declared limitations
//!
//! Nothing here is CAD or CAE, a solver, a building-code implementation, or a
//! certification of fitness. The domain packages compute bounded arithmetic
//! surrogates only.
//!
//! # Identity boundary
//!
//! Domain package digests are the accepted `kala-domains` crate's **native**
//! digests. They are deliberately not the historical Python source digests, so
//! every digest computed over a package digest (the report artifact, the
//! specification digest, the state and action digests, the request identity and
//! the journal chain) is native too. Stored Python revisions remain readable;
//! a prepared Python commit is not silently re-approved under native identity.

#![forbid(unsafe_code)]

pub mod application;
pub mod domains;
pub mod dto;
pub mod error;
pub mod value;

pub use application::{
    APPROVAL_OPTIONS, ARTIFACT_SCHEME_PREFIX, BUDGET_STATE_AGGREGATE, CHECKPOINT_DECISIONS,
    COMMITTED_REVISION_EVENT, CONFLICTED_REVISION_EVENT, Clock, Components, DECIDED_REVISION_EVENT,
    DEFAULT_LOCAL_MODEL_ID, DEFAULT_MECHANICAL_DESIGN_ID, DEFAULT_REJECT_REASON,
    DIGEST_PREFIX_LENGTH, DOMAIN_NAMES, HARD_CONSTRAINT_CODE, KERNEL_STATE_NAMESPACE,
    KalaApplication, LOCAL_ACTOR_ID, MAX_ATTRIBUTE_COUNT, MAX_DESIGN_ID_LENGTH, MAX_EVENT_LIMIT,
    MAX_EVENT_PAYLOAD_BYTES, MAX_REFERENCE_STEPS, MAX_REPORT_BYTES, MAX_REVISION_LIMIT,
    MAX_SPEC_ATTRIBUTES_BYTES, MAX_SUMMARY_LENGTH, MECHANICAL_DOMAIN, MECHANICAL_PLAN_OBJECTIVE,
    POLICY_STATE_AGGREGATE, PRE_COMMIT_CAPABILITIES, PREPARED_REVISION_EVENT,
    REJECTED_REVISION_EVENT, REPORT_CLASSIFICATION, REPORT_MEDIA_TYPE, REVISION_COMMIT_CHECKPOINT,
    REVISION_COMMIT_OPERATION, REVISION_COMMIT_PREFIX, REVISION_DECISIONS,
    TRANSFORM_MECHANICAL_RUN, TRANSFORM_REFERENCE_EVALUATION, WORKFLOW_ID_PREFIX, artifact_digest,
    kernel_state_aggregate, mechanical_step_catalog, reserved_workflow_ids,
};
pub use domains::DomainBinding;
pub use dto::{
    BudgetUsageView, CatalogStepView, CheckpointView, DomainDetail, DomainEvaluationReport,
    DomainPackageView, EventPage, EventView, MechanicalRunResult, RevisionCommitView,
    RevisionRecord, StepAttemptView, View, WorkflowDefinitionView, WorkflowStateView,
    WorkflowTransitionRecordView, WorkflowTransitionView,
};
pub use error::{ApplicationError, ApplicationResult, ErrorKind};
