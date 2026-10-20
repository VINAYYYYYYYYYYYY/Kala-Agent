//! Native ConstructionScript lowering and loopback bridge client for the Kala
//! Agent Solid Edge integration.
//!
//! This crate is the Windows production path described by ADR-0010 and
//! ADR-0008: it lowers the shared `ConstructionScript` into bridge protocol
//! v1 operations and talks to the local bridge over an authenticated loopback
//! connection. **It computes no geometry.** Solid Edge performs every
//! operation; the closed-form oracle, the volume model and the STEP writer
//! stay in the Python geometry worker and are not ported here.
//!
//! The crate has three parts:
//!
//! * [`compile`] lowers a validated construction script into typed bridge
//!   operations and refuses everything outside the supported subset — helix
//!   and threads, fuse, compound, analysis, fillets, and a rectangle revolved
//!   on `XZ` — before a socket is opened. The refusals carry the same
//!   messages as the Python compiler.
//! * [`client`] is a small typed HTTP client for the bridge. It accepts only
//!   `http://127.0.0.1[:port]`, never follows a redirect, bounds every
//!   response and every deadline, and maps the bridge's error envelope onto
//!   typed failures. The bearer token is read from a private file and never
//!   appears in a message or a log line.
//! * [`journal`] and [`paths`] use the same on-disk locations and the same
//!   record format as the Python client, so either client can read the
//!   other's records.
//!
//! The wire types live in [`protocol`]. They are the native mirror of
//! `kala_solidedge_bridge.protocol`, including the deterministic canonical
//! encoding used for the request digest.
//!
//! # Example
//!
//! ```no_run
//! use kala_contracts::load_construction_json_str;
//! use kala_solidedge_client::{BridgeClient, build_request, new_request_id};
//!
//! let script = load_construction_json_str(
//!     r#"{"summary":"plate","components":[{"name":"plate","steps":[
//!        {"op":"sketch","profile":"rectangle","width":20,"height":20},
//!        {"op":"extrude","length":10}]}]}"#,
//! )
//! .expect("a valid construction script");
//! let request = build_request(&script, &new_request_id(), Some("cal-1"), 120.0, None)
//!     .expect("a supported script");
//! let client = BridgeClient::new(
//!     "http://127.0.0.1:8797",
//!     "the-bearer-token",
//!     30.0,
//!     300.0,
//! )
//! .expect("a loopback bridge");
//! let accepted = client.submit(&request).expect("accepted");
//! println!("{}", accepted.build_id);
//! ```
//!
//! # What this crate does not claim
//!
//! Nothing here is evidence about Solid Edge. There is no Windows host, no
//! Solid Edge installation, and no native build in this repository. The client
//! is tested against the bridge contract and an in-process mock bridge only.

#![forbid(unsafe_code)]

mod auth;
mod client;
mod compile;
mod errors;
mod http;
mod journal;
mod paths;
mod protocol;

pub use auth::{MAX_TOKEN_CHARS, MIN_TOKEN_CHARS, read_token_file, token_matches};
pub use client::{
    BridgeClient, BridgeUrl, DEFAULT_ARTIFACT_TIMEOUT_SECONDS, DEFAULT_TIMEOUT_SECONDS, Download,
    MAX_ARTIFACT_BYTES, MAX_JSON_BYTES, validate_bridge_url,
};
pub use compile::{
    ASSEMBLY_NAME, DEFAULT_BUILD_TIMEOUT_SECONDS, MAX_SUMMARY_CHARS, build_request,
    build_request_with, compile_operations, new_request_id, summary,
};
pub use errors::{ErrorKind, MAX_MESSAGE_CHARS, SolidEdgeClientError};
pub use journal::{
    JOURNAL_FILENAME, Journal, JournalEntry, MAX_DIGEST_CHARS, MAX_ENTRIES, MAX_ID_CHARS,
    MAX_SCRIPT_CHARS, MAX_TARGET_CHARS,
};
pub use paths::{
    BRIDGE_URL_ENVIRONMENT_KEY, DATA_DIR_PARTS, DEFAULT_BRIDGE_URL, Environment, RUNS_DIR_PARTS,
    TOKEN_FILE_ENVIRONMENT_KEY, TOKEN_FILENAME, build_output_dir, default_output_dir,
    default_token_file, environment_snapshot, home_directory, is_windows, local_app_data,
    resolve_bridge_url, resolve_output_dir, resolve_token_file, safe_build_id,
};
pub use protocol::{
    Artifact, ArtifactFormat, BridgeOp, BuildAccepted, BuildRequest, BuildResult, BuildStatus,
    ErrorCode, ErrorResponse, Health, JobState, MAX_ANGLE_DEG, MAX_ARTIFACTS, MAX_DENSITY_KG_M3,
    MAX_OFFSET_MM, MAX_OPERATIONS, MAX_PARTS, MAX_SIZE_MM, MAX_TIMEOUT_SECONDS, MAX_TRACE_STEPS,
    MAX_WARNINGS, MIN_SIZE_MM, MIN_TIMEOUT_SECONDS, PROTOCOL_VERSION, PartResult, Plane,
    ProtocolViolation, REVOLVE_AXIS, RangeBox, StepTrace, sha256_hex, validate_procedure,
};
