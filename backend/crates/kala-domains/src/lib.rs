//! Native declarative domain packages for the Kala Agent backend.
//!
//! This crate replaces the pure declaration/arithmetic layer in
//! `kala_agent.domains` and its mechanical, building and wing packages. It has
//! no kernel, runtime, persistence, CAD, or solver dependency. Every evaluator
//! is a bounded pure function of a validated parameter model.
//!
//! Two identity families are kept deliberately separate:
//!
//! * Parameter-schema digests are computed from the same declaration payload
//!   and remain byte-identical to the Python values.
//! * Evaluator implementation and package digests are **native Rust** values
//!   under [`NATIVE_IMPLEMENTATION_DIGEST_VERSION`]. The script cannot and does
//!   not reproduce Python source-file digests; the historical values are kept
//!   only in [`provenance::legacy`] and in the compatibility record.
//!
//! Canonical JSON and CPython float formatting come from the accepted
//! `kala-json` crate, contract models from `kala-contracts`, and Python
//! `round(x, 6)` semantics from the accepted `kala-budget` compatibility
//! helper. Parameter input is parsed through `kala_json::CanonicalValue`, never
//! through a lossy serde value.

#![forbid(unsafe_code)]

mod capability;
mod context;
mod error;
mod input;
mod model_json;
mod package;
mod parameters;
mod provenance;
mod registry;
pub mod source_audit;
mod source_digest_core;
mod spec;
mod workflow;

pub mod building;
pub mod mechanical;
pub mod wing;

pub use capability::{CapabilityGrantPolicy, CapabilityRequest, enforce_capability_request};
pub use context::{EvaluationContext, reference_context, reference_evaluation_epoch};
pub use error::{DomainCallError, DomainError, ParameterError, ParameterErrorKind};
pub use kala_contracts::Timestamp;
pub use package::{DomainEvaluator, DomainPackage, DomainPackageApi, DomainPackageParts};
pub use parameters::ParameterModel;
pub use provenance::{
    EvaluatorImplementation, ImplementationIdentityChange, NativeSourceIdentity,
    PROVENANCE_BOUNDARY, ProvenanceCompatibilityRecord, legacy, native_module_name,
    native_source_identity, provenance_compatibility,
};
pub use registry::{
    DomainName, DomainParameters, evaluate, evaluate_typed, manifests, package, package_for,
    packages,
};
pub use spec::{
    CatalogStep, LEGACY_PYTHON_IMPLEMENTATION_DIGEST_VERSION, MAX_IMPLEMENTATION_NAME_LENGTH,
    NATIVE_IMPLEMENTATION_DIGEST_VERSION, NATIVE_IMPLEMENTATION_LANGUAGE,
    NON_AUTHORITATIVE_FINDING, ParameterDefault, ParameterSpec, ROUNDING_DIGITS,
    UNBOUNDED_SAFETY_FACTOR, canonical_digest, canonical_json, clamp_unit_interval,
    parameter_schema_digest, parse_checkpoint_kind, require_capabilities, require_digest,
    require_identifier, require_name,
};
pub use workflow::{ReferenceWorkflowRun, StepOutcome, run_reference_workflow};

/// The deterministic reference evaluation epoch as an RFC 3339 string.
pub const REFERENCE_EVALUATION_EPOCH_ISO: &str = "2026-01-01T00:00:00Z";
