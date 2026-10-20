//! Native working and durable memory management for the Kala Agent kernel.
//!
//! This crate is the standalone Rust port of
//! `kala_agent.kernel.memory`. It owns the working-memory layer that sits in
//! front of the accepted `kala-repositories` durable store:
//!
//! * [`MemoryManager`] keeps a bounded, insertion-ordered working map and
//!   reads and writes the accepted [`MemoryRepository`].
//! * One source may hold only one content. A different content for the same
//!   source is a *conflict*; the same content is a *duplicate*.
//! * Records expire against an explicit instant, and an expiry is inclusive:
//!   `expires_at == now` is already expired.
//! * Promotion is the only way into durable storage, and a promotion can be
//!   refused when it would contradict a durable record, when it would
//!   contradict another working record of the same source, or when the
//!   manager is configured to require a provenance identifier.
//!
//! The durable store, the [`MemoryRecord`] model and the contract types are
//! the accepted crates' own values, not wrappers:
//!
//! ```rust
//! use kala_memory::{AddOptions, MemoryManager};
//! use kala_persistence::Database;
//! use kala_repositories::MemoryRepository;
//!
//! let database = Database::in_memory()?;
//! let manager = MemoryManager::new(MemoryRepository::new(&database));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Python is used only by the integration tests as an oracle. The library
//! contains no Python or subprocess calls.

#![forbid(unsafe_code)]

mod error;
mod manager;
mod repr;
mod result;
mod status;

pub use error::{MemoryConflictError, MemoryError};
pub use manager::{
    AddOptions, DEFAULT_MAX_WORKING_RECORDS, DURABLE_RECORD_SCAN_LIMIT, GetOptions, ListOptions,
    ListWorkingOptions, MemoryManager, MemoryManagerConfig, PromoteOptions, PromoteTarget,
    is_expired, new_record, resolve_now, validated_record,
};
pub use repr::python_repr_str;
pub use result::{MemoryAddResult, MemoryEntry};
pub use status::MemoryStatus;

// The accepted crates' own types, re-exported so a consumer can name every
// value in this crate's signatures without pinning a second dependency path.
pub use kala_contracts::{CanonicalJsonObject, ContractModel, Text, Timestamp};
pub use kala_json::CanonicalValue;
pub use kala_repositories::{MemoryRecord, MemoryRepository};
