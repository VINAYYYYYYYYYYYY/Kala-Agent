//! Reading stored rows and judging one row against its own recorded digests.
//!
//! The Python module reads every column through `str(row[...])` and judges a
//! row with `_row_is_self_consistent`. This module reproduces both rules
//! exactly, including the treatment of a column that holds a BLOB (SQLite
//! keeps the BLOB storage class in a TEXT-affinity column) and of a column
//! that holds a value which is not text at all.

use kala_json::{EventChainFields, Sha256Digest, chain_digest};
use kala_persistence::{is_lowercase_sha256_hex, sha256_text};
use rusqlite::Row;
use rusqlite::types::Value;

/// The chain-covered columns, byte-identical to the Python `CHAIN_COLUMNS`.
pub(crate) const CHAIN_COLUMNS: &str = "sequence, event_type, aggregate_id, workflow_id, run_id, \
                                 payload_json, digest, occurred_at, previous_event_digest, \
                                 event_digest";

/// One stored row, read exactly as the Python module reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawRow {
    pub(crate) sequence: i64,
    pub(crate) event_type: String,
    pub(crate) aggregate_id: String,
    pub(crate) workflow_id: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) payload_json: String,
    pub(crate) digest: String,
    pub(crate) occurred_at: String,
    pub(crate) previous_event_digest: String,
    pub(crate) event_digest: String,
}

impl RawRow {
    /// Read one row from a statement result.
    pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            sequence: row.get("sequence")?,
            event_type: text_of(&row.get::<_, Value>("event_type")?),
            aggregate_id: text_of(&row.get::<_, Value>("aggregate_id")?),
            workflow_id: optional_text_of(&row.get::<_, Value>("workflow_id")?),
            run_id: optional_text_of(&row.get::<_, Value>("run_id")?),
            payload_json: text_of(&row.get::<_, Value>("payload_json")?),
            digest: text_of(&row.get::<_, Value>("digest")?),
            occurred_at: text_of(&row.get::<_, Value>("occurred_at")?),
            previous_event_digest: text_of(&row.get::<_, Value>("previous_event_digest")?),
            event_digest: text_of(&row.get::<_, Value>("event_digest")?),
        })
    }
}

/// Return the Python `str()` form of a stored column value.
///
/// SQLite converts a number written into a TEXT-affinity column into text
/// before it is stored, so a numeric value can only appear in a column that
/// was written before the table was created with its affinity, or in a
/// database written by another engine. The number and NULL cases are handled
/// anyway so the native reader never refuses a row the Python reader accepts.
fn text_of(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => {
            kala_json::format_float(*number).unwrap_or_else(|_| format!("{number:?}"))
        }
        Value::Text(text) => text.clone(),
        Value::Blob(bytes) => python_bytes_repr(bytes),
    }
}

/// Return the Python `str()` form of a nullable stored column value.
///
/// A SQL `NULL` is the absent value, exactly like `row["workflow_id"] is None`
/// in the Python module.
fn optional_text_of(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(text_of(other)),
    }
}

/// Return the Python `repr()` form of a byte string.
///
/// A BLOB in a TEXT-affinity column reaches the Python module as `bytes`, and
/// `str(bytes)` is its `repr`. Only the escapes CPython emits are produced.
fn python_bytes_repr(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 3);
    out.push('b');
    out.push('\'');
    for byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\'' => out.push_str("\\'"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(*byte as char),
            other => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\x{other:02x}");
            }
        }
    }
    out.push('\'');
    out
}

/// Return whether one stored row verifies against its own recorded digests.
///
/// The row's payload digest must be the SHA-256 of the payload text it
/// stores, and its chain digest must be the chain digest recomputed over the
/// covered fields the row itself records, including the previous digest it
/// stores. A row that passes is a well-formed event; whether it belongs to
/// *this* chain is decided by the event before it, which is what the link to
/// the stored previous digest commits to.
pub(crate) fn row_is_self_consistent(row: &RawRow) -> bool {
    // The payload digest must be the SHA-256 of the stored payload bytes.
    if sha256_text(&row.payload_json).to_hex() != row.digest {
        return false;
    }
    // The stored chain digest must be a lowercase SHA-256 digest.
    if !is_lowercase_sha256_hex(&row.event_digest) {
        return false;
    }
    // The chain digest is recomputed over the stored previous digest. The
    // Python module uses the stored string verbatim in the preimage, so a
    // previous digest that is not exactly 64 lowercase hexadecimal digits is
    // rejected here as well. The rejection is unobservable: a row whose
    // previous digest is not a valid digest cannot carry the cached head
    // digest, so the Python fast path refuses the row as well.
    let Some(previous_event_digest) = Sha256Digest::from_hex(&row.previous_event_digest) else {
        return false;
    };
    if !is_lowercase_sha256_hex(&row.previous_event_digest) {
        return false;
    }
    let Ok(sequence) = u64::try_from(row.sequence) else {
        return false;
    };
    let fields = EventChainFields {
        sequence,
        event_type: row.event_type.clone(),
        aggregate_id: row.aggregate_id.clone(),
        workflow_id: row.workflow_id.clone(),
        run_id: row.run_id.clone(),
        occurred_at: row.occurred_at.clone(),
        payload_digest: sha256_text(&row.payload_json),
        previous_event_digest,
    };
    chain_digest(&fields).to_hex() == row.event_digest
}
