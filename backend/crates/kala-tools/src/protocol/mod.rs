//! The Kala tool protocol.
//!
//! This module is the Rust port of `kala_agent.tools.protocol`. It owns the
//! bounded canonical JSON encoding, the JSON schema validation, the request
//! and response envelopes, the workspace path rules, the worker bootstrap and
//! the child environment.
//!
//! The public names match the Python module one for one, so a caller can read
//! the Python implementation and the Rust implementation side by side.

mod diagnostics;
mod enums;
mod envelope;
mod error;
mod handler;
pub(crate) mod json;
mod pattern;
mod schema;
mod text;
mod workspace;

pub use diagnostics::{sanitize_child_stderr, summarize_error};
pub use enums::{ToolEnvelopeKind, ToolErrorCode, is_failure_status};
pub use envelope::{ToolRequestEnvelope, ToolResponseEnvelope};
pub use error::ToolProtocolError;
pub use handler::{
    PROTECTED_CHILD_ENVIRONMENT_KEYS, SubprocessToolHandler, ToolHandler, WORKER_BOOTSTRAP,
    build_child_environment, build_worker_command, read_request_stream, write_response_stream,
};
pub use json::{
    canonical_json_digest, canonical_json_dumps, sha256_hex, sha256_hex_text, strict_json_loads,
};
pub use schema::{check_object_schema, validate_against_schema};
pub use workspace::resolve_workspace_path;

/// The protocol version of the envelope format.
pub const PROTOCOL_VERSION: &str = "1.0";

/// The maximum size of a JSON payload or of a canonical JSON document.
pub const MAX_JSON_BYTES: usize = 1_048_576;

/// The maximum size of the canonical encoding of a tool's arguments.
pub const MAX_ARGUMENT_BYTES: usize = 262_144;

/// The maximum length of a JSON string, in Unicode code points.
pub const MAX_STRING_LENGTH: usize = 32_768;

/// The maximum number of items in a JSON array.
pub const MAX_COLLECTION_ITEMS: usize = 2_048;

/// The maximum length of a JSON object key, in Unicode code points.
pub const MAX_KEY_LENGTH: usize = 256;

/// The maximum nesting depth of a JSON document.
pub const MAX_JSON_DEPTH: usize = 48;

/// The maximum nesting depth of a schema.
pub const MAX_SCHEMA_DEPTH: usize = 32;

/// The maximum number of segments in a workspace path.
pub const MAX_PATH_SEGMENTS: usize = 32;

/// The maximum length of a workspace path.
pub const MAX_PATH_LENGTH: usize = 1_024;

/// The maximum length of a schema pattern.
pub const MAX_PATTERN_LENGTH: usize = 256;

/// The maximum length of a string a schema pattern may be run against.
pub const MAX_PATTERN_INPUT_LENGTH: usize = 1_024;

/// The maximum repetition count a schema pattern may declare.
pub const MAX_PATTERN_REPEAT: usize = 1_000;

/// The maximum number of unbounded repetitions in a schema pattern.
pub const MAX_PATTERN_UNBOUNDED_REPEATS: usize = 1;

/// The maximum number of ways a schema pattern may split a subject.
pub const MAX_PATTERN_BACKTRACK_STATES: usize = 2_048;

/// The maximum length of a sanitized diagnostic.
pub const MAX_SANITIZED_DETAIL_LENGTH: usize = 512;
