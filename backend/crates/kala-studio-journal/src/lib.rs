//! The Studio's own hash-chained session journal and artifact store.
//!
//! This crate is the native Rust equivalent of the Python module
//! `kala_agent.studio.session_journal`. The file it writes is
//! **byte-compatible** with the file the Python module writes: the same
//! `studio_events` schema, the same canonical payload JSON, the same
//! length-prefixed SHA-256 chain digests and the same content-addressed
//! artifact layout. A journal written by either implementation can be
//! opened, verified and extended by the other.
//!
//! # What is reused
//!
//! * The chain digest is the core chain digest. `kala_json::chain_digest`
//!   computes the same length-prefixed, domain-separated SHA-256 preimage
//!   the core journal uses, over the same covered fields in the same order,
//!   so a Studio chain digest is reproducible with the core function without
//!   any Studio-specific knowledge.
//! * The artifact store is `kala_artifacts::ContentAddressedArtifactStore`,
//!   the native equivalent of
//!   `kala_agent.persistence.artifacts.ContentAddressedArtifactStore`, with
//!   the same `objects/<ab>/<digest>` layout.
//! * The canonical JSON convention is not re-implemented here: payloads are
//!   canonicalised with `kala_persistence::canonicalize_json_object`, the very
//!   function the core journal uses, so "canonical" means exactly the same
//!   bytes in both places (sorted keys, no insignificant whitespace,
//!   ASCII-escaped, no NaN or Infinity).
//!
//! # Storage
//!
//! The journal is a single SQLite file inside the caller-supplied Studio data
//! directory, opened in WAL mode and guarded by one lock, so a server thread
//! pool may append, page, and verify concurrently without two threads
//! interleaving a statement. A write refuses to extend a chain that is
//! already broken, so a journal that has been edited outside the Studio fails
//! closed instead of growing a second, inconsistent history.
//!
//! The chain is tamper-*evident*, not tamper-proof, exactly like the core
//! chain: anyone who can write the database can rewrite a whole suffix
//! consistently. The head digest returned by [`StudioSessionJournal::verify`]
//! is the value to anchor outside the database.
//!
//! # When an edit is detected
//!
//! The chain is walked once, when the journal is opened, and the verified
//! head is cached from then on: the sequence and the chain digest of the last
//! event that verified. An append does not walk the chain again. It re-reads
//! only the newest stored row, re-verifies that one row against the digests
//! the row itself records, and checks that the row is still the cached head.
//! An append therefore costs one row read rather than one walk, so a long
//! session costs no more per event than a short one.
//!
//! That gives a precise, deliberately narrow, detection guarantee:
//!
//! - An edit of the **head** -- the newest event -- is detected by the next
//!   append: the row no longer matches the cached head, the journal falls
//!   back to a full walk, and the append is refused with
//!   [`StudioJournalError::Integrity`].
//! - An edit **in the middle** of the chain is detected by the next
//!   [`StudioSessionJournal::verify`] or by the next **open** of the journal,
//!   because only a full walk re-derives the digests of the events that
//!   follow the edited one.
//!
//! A broken chain is never extended: the append fails closed either way. A
//! journal that is found broken when it is opened, or that fails a
//! verification, refuses every append until a later verification walks the
//! chain and finds it intact again.
//!
//! # Recording a design turn
//!
//! ```no_run
//! use kala_studio_journal::{JournalEventType, StudioSessionJournal};
//!
//! let journal = StudioSessionJournal::open("/tmp/studio-data").unwrap();
//! let digest = journal.put_artifact(b"ISO-10303-21;\n").unwrap();
//! journal
//!     .append_json(
//!         JournalEventType::ArtifactStored.as_str(),
//!         &format!("{{\"kind\":\"step\",\"sha256\":\"{digest}\"}}"),
//!     )
//!     .unwrap();
//! ```
//!
//! [`StudioSessionJournal::put_artifact`] stores the bytes and returns their
//! SHA-256 without writing an event of its own, so the caller decides which
//! event records the blob.
//!
//! Python is used only by the integration tests as an oracle. The library
//! itself contains no Python, no subprocess and no fallback implementation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod event;
mod identifier;
mod journal;
mod path;
mod row;

pub use error::{ErrorKind, StudioJournalError};
pub use event::{JournalEventType, JournalVerification, SessionEvent};
pub use journal::{
    ARTIFACT_DIRECTORY_NAME, AppendOptions, DEFAULT_AGGREGATE_ID, DEFAULT_BUSY_TIMEOUT_MS,
    DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_PAGE_SIZE, DEFAULT_MAX_PAYLOAD_BYTES,
    JOURNAL_FILE_NAME, JournalConfig, JournalDiagnostics, StudioSessionJournal,
};
pub use kala_artifacts::ContentAddressedArtifactStore;
