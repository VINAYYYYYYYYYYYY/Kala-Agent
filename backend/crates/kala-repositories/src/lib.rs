//! Native typed repositories over the accepted Kala Agent persistence schema
//! and contracts.
//!
//! This crate is the native counterpart of
//! `kala_agent.persistence.repositories`. It owns no schema, migration,
//! connection pool or database lock: every repository borrows the accepted
//! [`Database`] from `kala-persistence`, which provides the frozen schema-v2
//! tables, `BEGIN IMMEDIATE`/`SAVEPOINT` transaction semantics and one
//! serialized writer.
//!
//! The six repositories are:
//!
//! * [`DesignSpecRepository`] — insert-or-replace current design specifications.
//! * [`DesignRevisionRepository`] — append-only revisions with a required
//!   expected-head compare-and-swap; there is no unconditional write path.
//! * [`ApprovalRequestRepository`] — pending approvals may change, decided
//!   approvals are final except for a byte-identical no-op.
//! * [`ToolCallRepository`] — insert-or-replace tool calls.
//! * [`ToolResultRepository`] — insert-or-replace results after validating the
//!   call foreign key and `tool_id` identity in the same transaction.
//! * [`MemoryRepository`] — durable memory with strict expiry filtering.
//!
//! Stored models are always decoded with the accepted contract validation.
//! Persisted `CanonicalJsonObject` values use the unbounded persistence JSON
//! domain, never the bounded tool-protocol defaults. Writers emit the same
//! compact JSON bytes as Pydantic's `model_dump_json()`, and readers never
//! rewrite stored `model_json`.
//!
//! Python is used only by the integration tests as an oracle. The library
//! contains no Python or subprocess calls.

#![forbid(unsafe_code)]

mod error;
mod memory;
mod repositories;
mod support;

pub use error::{ApprovalAlreadyDecidedError, HeadMovedError, RepositoryError};
pub use memory::{MEMORY_RECORD, MemoryRecord, memory_metadata_limits};
pub use repositories::{
    ApprovalRequestRepository, DesignRevisionRepository, DesignSpecRepository, MemoryGetOptions,
    MemoryListOptions, MemoryRepository, ToolCallRepository, ToolResultRepository,
};
pub use support::DEFAULT_LIST_LIMIT;

// Re-export the accepted types consumers need to build and inspect repository
// values without a second dependency path. These are the accepted crates'
// own types, not wrappers.
pub use kala_contracts::{
    ApprovalKind, ApprovalRequest, ApprovalStatus, CanonicalJsonObject, ContractError,
    ContractIssue, ContractModel, DesignRevision, DesignSpec, Identifier, NonNegativeInteger,
    Sha256Digest, SideEffectLevel, Text, Timestamp, ToolCall, ToolResult, ToolRunStatus,
};
pub use kala_json::{CanonicalValue, Limits};
pub use kala_persistence::{Database, EventJournal, PersistenceError};
pub use num_bigint::BigInt;
