//! Native Kala tool protocol layer.
//!
//! This crate is a standalone workspace port of the `kala_agent.tools`
//! package. It consumes the accepted `kala-json` canonical JSON
//! implementation and the accepted `kala-contracts` typed models read-only.
//! It never invokes Python in production; the live Python differential is
//! test-only.
//!
//! The crate owns three modules today:
//!
//! * [`protocol`] — the wire protocol: bounded canonical JSON, schema
//!   validation, the request and response envelopes, the workspace path
//!   rules, the worker bootstrap and the child environment.
//! * [`registry`] — the tool registry: the binding of a `(tool_id, version)`
//!   pair to a descriptor, a handler and the two schemas of a tool.
//! * [`simulator`] — the mechanical reference simulator, its pure reference
//!   evaluator and the misbehaviour entry points.
//!
//! Later cells add the sandbox and the executor to this same crate, so the
//! module layout is kept shallow and the crate root re-exports only what the
//! protocol layer owns.
//!
//! # Compatibility
//!
//! The port reproduces the Python modules exactly:
//!
//! * the same bounded canonical JSON bytes and the same digests, produced by
//!   the accepted `kala-json` crate,
//! * the same error classes and the same message text, including the error
//!   precedence of the envelope and schema validators,
//! * the same schema validation rules, the same admitted keyword set and the
//!   same pattern admission analysis,
//! * a pattern matcher with CPython `re` semantics, so a schema pattern
//!   matches exactly the strings it matches in Python,
//! * the same registration validation, the same capability declarations, the
//!   same trust and provenance checks and the same duplicate refusals,
//! * the same deterministic reference calculation, the same bounds and the
//!   same published report.
//!
//! The differences that remain are listed in the crate README.

#![forbid(unsafe_code)]

pub mod executor;
pub mod protocol;
pub mod registry;
pub mod simulator;

// Accepted contract and JSON types used by the public signatures.
// Re-exporting them keeps a consumer from having to pin a second direct
// dependency just to name a request's run status or a canonical argument
// object.
pub use kala_contracts::{
    ArtifactRef, ContractModel, Timestamp, ToolCall, ToolDescriptor, ToolResult, ToolRunStatus,
};
pub use kala_json::{CanonicalObject, CanonicalValue, JsonText};

pub use protocol::{
    MAX_ARGUMENT_BYTES, MAX_COLLECTION_ITEMS, MAX_JSON_BYTES, MAX_KEY_LENGTH, MAX_PATH_LENGTH,
    MAX_PATH_SEGMENTS, MAX_PATTERN_BACKTRACK_STATES, MAX_PATTERN_INPUT_LENGTH, MAX_PATTERN_LENGTH,
    MAX_PATTERN_REPEAT, MAX_PATTERN_UNBOUNDED_REPEATS, MAX_SANITIZED_DETAIL_LENGTH,
    MAX_SCHEMA_DEPTH, MAX_STRING_LENGTH, PROTECTED_CHILD_ENVIRONMENT_KEYS, PROTOCOL_VERSION,
    SubprocessToolHandler, ToolEnvelopeKind, ToolErrorCode, ToolHandler, ToolProtocolError,
    ToolRequestEnvelope, ToolResponseEnvelope, WORKER_BOOTSTRAP, build_child_environment,
    build_worker_command, canonical_json_digest, canonical_json_dumps, check_object_schema,
    read_request_stream, resolve_workspace_path, sanitize_child_stderr, sha256_hex,
    sha256_hex_text, strict_json_loads, summarize_error, validate_against_schema,
    write_response_stream,
};

pub use executor::{
    ExecutionOptions, ExecutorConfig, IsolationMode, SandboxProfile, ToolExecutionReport,
    ToolExecutor, ToolExecutorError,
};
