//! Typed Rust contracts shared by the Kala Agent backend cells.
//!
//! This crate is a native reimplementation of the frozen Pydantic surface in
//! `kala_agent.contracts.models` and `kala_agent.contracts.construction`. It
//! owns three separate representations deliberately:
//!
//! * **Wire JSON**: the bytes produced by
//!   `json.dumps(model.model_dump(mode="json"), ensure_ascii=False,
//!   separators=(",", ":"))`, which is what the HTTP services return. Field
//!   order and UTF-8 text are preserved. [`WireValue`] and
//!   [`ContractModel::to_wire_json`] model this form.
//! * **Canonical JSON**: the sorted, ASCII-escaped, compact bytes used for
//!   digests by `kala_agent.tools.protocol.canonical_json_dumps`. This is
//!   produced through [`kala_json`], never through a lossy intermediate value,
//!   so arbitrary integers, signed zero and lone-surrogate code points survive.
//! * **Typed values**: validated, immutable Rust structs and newtypes. Every
//!   model can only be created through [`ContractModel::from_value`] (or a
//!   strict JSON entry point) so cross-field invariants cannot be bypassed.
//!
//! Ordinary model JSON entry points use FastAPI/Pydantic-compatible duplicate
//! key handling (last wins, first insertion position retained), preserve
//! codepoint-faithful text and arbitrary integers together, and keep object
//! insertion order for construction profile aliases. The permissive parser is
//! a small iterative ordered representation owned by this crate; no serde
//! value is involved, so an ordinary key such as
//! `$serde_json::private::Number` is plain user data. Persisted
//! [`CanonicalJsonObject`] values and all canonical/hash bytes remain strict
//! about duplicate keys.
//!
//! Persisted [`CanonicalJsonObject`] values are validated with an effectively
//! unbounded JSON domain, matching the Python contract; the tool-protocol
//! 1 MiB/48-depth limits are never applied to them. The CPython 4,300 digit
//! integer conversion limit is retained because the source contract inherits
//! it from `json.loads`/`json.dumps`.
//!
//! Python is used only by the test-time differential oracle. Production code
//! never invokes Python.

#![forbid(unsafe_code)]

mod construction;
mod error;
mod input;
mod models;
mod scalar;
mod text;
mod timestamp;
mod uri;
mod wire;

pub use construction::{
    ANALYSIS_OPS, ASSEMBLY_OPS, AnalysisReading, AnalysisScript, AnalysisStep, AssemblyStep,
    COMPONENT_OPS, ComponentScript, ComponentStep, ConstructionError, ConstructionScript, EDIT_OPS,
    EditStep, MAX_COMPONENTS, MAX_DIMENSION_MM, MAX_STEPS, RevisionAction, load_analysis,
    load_analysis_json_bytes, load_analysis_json_str, load_construction,
    load_construction_json_bytes, load_construction_json_str, load_reading,
    load_reading_json_bytes, load_reading_json_str,
};
pub use error::{ContractError, ContractIssue};
pub use models::{
    ApprovalKind, ApprovalRequest, ApprovalStatus, ArtifactClassification, ArtifactRef,
    DesignRevision, DesignSpec, DomainPackageManifest, EvaluationMetric, EvaluationResult,
    PlanStep, Provenance, Requirement, RequirementKind, RequirementPriority, RequirementStatus,
    RunBudget, SideEffectLevel, StepStatus, ToolCall, ToolDescriptor, ToolResult, ToolRunStatus,
    WorkflowDefinition, WorkflowStatus, WorkflowTransition,
};
pub use scalar::{
    ArtifactUri, CanonicalJsonObject, FiniteNonNegativeFloat, Identifier, MediaType, NonEmptyText,
    NonNegativeDecimal, NonNegativeInteger, Sha256Digest, UnitInterval, model_json_limits,
};
pub use text::Text;
pub use timestamp::Timestamp;
pub use wire::{ContractModel, WireValue};
