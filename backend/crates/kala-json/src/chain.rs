use thiserror::Error;

use crate::digest::{Sha256Digest, sha256_bytes};

/// Chain hash domain, exactly as stored by the Python journal.
pub const CHAIN_DOMAIN: &str = "kala-agent.event-chain.v1.sha256";
/// Digest algorithm recorded in the chain domain.
pub const CHAIN_ALGORITHM: &str = "sha256";
/// Previous digest of the first journal event.
pub const GENESIS_PREVIOUS_DIGEST: Sha256Digest = Sha256Digest::ZERO;

/// The immutable journal content bound by one event digest.
///
/// `workflow_id` and `run_id` are optional because the journal stores SQL
/// `NULL` for absent values; an absent id and an empty id deliberately collapse
/// to the same empty preimage part, exactly like Python's
/// `workflow_id or ""`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventChainFields {
    /// One-based journal sequence number.
    pub sequence: u64,
    /// Event type, for example `design.created`.
    pub event_type: String,
    /// Aggregate identifier.
    pub aggregate_id: String,
    /// Optional workflow identifier.
    pub workflow_id: Option<String>,
    /// Optional run identifier.
    pub run_id: Option<String>,
    /// Timestamp string as persisted (never re-parsed by the chain).
    pub occurred_at: String,
    /// SHA-256 of the canonical payload JSON bytes.
    pub payload_digest: Sha256Digest,
    /// Digest of the preceding event, or the genesis zero digest.
    pub previous_event_digest: Sha256Digest,
}

impl EventChainFields {
    /// The eight covered values in Python's exact order.
    ///
    /// Optional ids use an empty part when absent or empty.
    pub fn preimage_parts(&self) -> [String; 8] {
        [
            self.sequence.to_string(),
            self.event_type.clone(),
            self.aggregate_id.clone(),
            self.workflow_id.clone().unwrap_or_default(),
            self.run_id.clone().unwrap_or_default(),
            self.occurred_at.clone(),
            self.payload_digest.to_hex(),
            self.previous_event_digest.to_hex(),
        ]
    }
}

/// Compute the exact Python chain preimage:
///
/// ```text
/// len8(domain) || domain
/// len8(sequence) || sequence
/// len8(event_type) || event_type
/// len8(aggregate_id) || aggregate_id
/// len8(workflow_id or "") || ...
/// len8(run_id or "") || ...
/// len8(occurred_at) || occurred_at
/// len8(payload_digest) || payload_digest
/// len8(previous_event_digest) || previous_event_digest
/// ```
///
/// Every length is a big-endian `u64` of the UTF-8 byte length. The domain is
/// covered too, unlike a bare field concatenation.
pub fn chain_preimage(fields: &EventChainFields) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(256);
    append_length_prefixed(&mut preimage, CHAIN_DOMAIN);
    for part in fields.preimage_parts() {
        append_length_prefixed(&mut preimage, &part);
    }
    preimage
}

/// Compute the journal event digest for typed fields.
pub fn chain_digest(fields: &EventChainFields) -> Sha256Digest {
    sha256_bytes(&chain_preimage(fields))
}

/// SHA-256 of the UTF-8 bytes of a persisted canonical payload JSON string.
///
/// Python computes `sha256(payload_json.encode("utf-8")).hexdigest()`; the
/// payload is not re-encoded or re-serialized at this step.
pub fn payload_digest(payload_json: &str) -> Sha256Digest {
    sha256_bytes(payload_json.as_bytes())
}

/// Result of verifying a contiguous, genesis-rooted event chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainVerification {
    /// Number of verified events.
    pub event_count: u64,
    /// Last verified sequence, or `None` for an empty chain.
    pub head_sequence: Option<u64>,
    /// Last verified event digest, or `None` for an empty chain.
    pub head_event_digest: Option<Sha256Digest>,
}

/// A journal row that does not belong to the chain.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChainVerifyError {
    /// Sequences are not exactly `1..=N`.
    #[error("event journal is missing sequence {expected}; found {actual}")]
    SequenceGap {
        /// Expected one-based sequence.
        expected: u64,
        /// Actual stored sequence.
        actual: u64,
    },
    /// A row's `previous_event_digest` does not match the prior event.
    #[error("event {sequence} previous digest does not match the chain")]
    PreviousDigestMismatch {
        /// Offending sequence.
        sequence: u64,
    },
    /// A row's computed digest does not match its stored `event_digest`.
    #[error("event {sequence} event digest does not match its content")]
    EventDigestMismatch {
        /// Offending sequence.
        sequence: u64,
    },
    /// The caller supplied the wrong number of stored digests.
    #[error("stored digest count {stored} does not match event count {events}")]
    DigestCountMismatch {
        /// Number of events.
        events: usize,
        /// Number of stored digests.
        stored: usize,
    },
}

/// Verify sequence continuity, previous-digest linkage and (when supplied)
/// stored event digests, following `EventJournal.verify_chain`.
///
/// The first event must link to [`GENESIS_PREVIOUS_DIGEST`]. If
/// `stored_digests` is provided it must contain one digest per event; it is
/// typically read from the `event_digest` column.
pub fn verify_chain(
    events: &[EventChainFields],
    stored_digests: Option<&[Sha256Digest]>,
) -> Result<ChainVerification, ChainVerifyError> {
    if let Some(digests) = stored_digests {
        if digests.len() != events.len() {
            return Err(ChainVerifyError::DigestCountMismatch {
                events: events.len(),
                stored: digests.len(),
            });
        }
    }
    let mut expected_previous = GENESIS_PREVIOUS_DIGEST;
    for (index, fields) in events.iter().enumerate() {
        let expected_sequence = index as u64 + 1;
        if fields.sequence != expected_sequence {
            return Err(ChainVerifyError::SequenceGap {
                expected: expected_sequence,
                actual: fields.sequence,
            });
        }
        if fields.previous_event_digest != expected_previous {
            return Err(ChainVerifyError::PreviousDigestMismatch {
                sequence: fields.sequence,
            });
        }
        let digest = chain_digest(fields);
        if let Some(stored) = stored_digests {
            if stored[index] != digest {
                return Err(ChainVerifyError::EventDigestMismatch {
                    sequence: fields.sequence,
                });
            }
        }
        expected_previous = digest;
    }
    Ok(ChainVerification {
        event_count: events.len() as u64,
        head_sequence: events.last().map(|fields| fields.sequence),
        head_event_digest: if events.is_empty() {
            None
        } else {
            Some(expected_previous)
        },
    })
}

fn append_length_prefixed(target: &mut Vec<u8>, part: &str) {
    let encoded = part.as_bytes();
    target.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
    target.extend_from_slice(encoded);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> EventChainFields {
        EventChainFields {
            sequence: 1,
            event_type: "design.created".to_owned(),
            aggregate_id: "design-corpus".to_owned(),
            workflow_id: None,
            run_id: None,
            occurred_at: "2026-01-01T00:00:00.000000Z".to_owned(),
            payload_digest: Sha256Digest::ZERO,
            previous_event_digest: GENESIS_PREVIOUS_DIGEST,
        }
    }

    #[test]
    fn domain_is_covered_and_length_prefixed() {
        let preimage = chain_preimage(&fields());
        let domain = CHAIN_DOMAIN.as_bytes();
        assert_eq!(&preimage[..8], &(domain.len() as u64).to_be_bytes());
        assert_eq!(&preimage[8..8 + domain.len()], domain);
        assert_eq!(
            preimage.len(),
            8 + domain.len()
                + 8
                + "1".len()
                + 8
                + "design.created".len()
                + 8
                + "design-corpus".len()
                + 8
                + 8
                + 8
                + "2026-01-01T00:00:00.000000Z".len()
                + 8
                + 64
                + 8
                + 64
        );
    }

    #[test]
    fn absent_and_empty_optional_ids_collapse() {
        let mut empty = fields();
        empty.workflow_id = Some(String::new());
        empty.run_id = Some(String::new());
        assert_eq!(chain_digest(&fields()), chain_digest(&empty));
    }
}
