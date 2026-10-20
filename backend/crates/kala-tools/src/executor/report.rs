//! The provenance record of a tool run.
//!
//! The record is what an operator reads when a run has to be explained: which
//! tool ran, in which isolation, under which controls, with which
//! capabilities, and which envelope the approval was bound to. It never
//! carries a secret: it holds the names of the capabilities, the digests and
//! the counters.

use kala_contracts::ToolRunStatus;
use kala_json::{CanonicalObject, CanonicalValue, JsonText};

use crate::executor::profile::IsolationMode;

/// The provenance record of one tool run.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolExecutionReport {
    /// The identifier of the call.
    pub call_id: String,
    /// The identifier of the tool.
    pub tool_id: String,
    /// The version of the tool.
    pub tool_version: String,
    /// The isolation the run was dispatched under.
    pub isolation: IsolationMode,
    /// The name of the sandbox profile.
    pub sandbox_profile: String,
    /// The status of the run.
    pub status: ToolRunStatus,
    /// The error code of the run, when it failed.
    pub error_code: Option<String>,
    /// The process identifier of the worker, when it ran out of process.
    pub process_id: Option<u32>,
    /// The capabilities the run was authorized with.
    pub granted_capabilities: Vec<String>,
    /// The number of artifact bytes the run produced.
    pub artifact_bytes: u64,
    /// The controls the host enforced.
    pub enforced_controls: Vec<String>,
    /// The digest of the envelope the approval was bound to.
    pub envelope_digest: String,
}

impl ToolExecutionReport {
    /// The canonical payload of the record.
    pub fn to_payload(&self) -> CanonicalValue {
        let mut payload = CanonicalObject::new();
        payload.insert(
            JsonText::new("artifact_bytes"),
            CanonicalValue::Integer(num_bigint::BigInt::from(self.artifact_bytes)),
        );
        payload.insert(
            JsonText::new("call_id"),
            CanonicalValue::text(&self.call_id),
        );
        payload.insert(
            JsonText::new("enforced_controls"),
            CanonicalValue::array(
                self.enforced_controls
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        payload.insert(
            JsonText::new("envelope_digest"),
            CanonicalValue::text(&self.envelope_digest),
        );
        payload.insert(
            JsonText::new("error_code"),
            match &self.error_code {
                None => CanonicalValue::Null,
                Some(code) => CanonicalValue::text(code),
            },
        );
        payload.insert(
            JsonText::new("granted_capabilities"),
            CanonicalValue::array(
                self.granted_capabilities
                    .iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        payload.insert(
            JsonText::new("isolation"),
            CanonicalValue::text(self.isolation.as_str()),
        );
        payload.insert(
            JsonText::new("process_id"),
            match self.process_id {
                None => CanonicalValue::Null,
                Some(process_id) => CanonicalValue::Integer(num_bigint::BigInt::from(process_id)),
            },
        );
        payload.insert(
            JsonText::new("sandbox_profile"),
            CanonicalValue::text(&self.sandbox_profile),
        );
        payload.insert(
            JsonText::new("status"),
            CanonicalValue::text(self.status.as_str()),
        );
        payload.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text(&self.tool_id),
        );
        payload.insert(
            JsonText::new("tool_version"),
            CanonicalValue::text(&self.tool_version),
        );
        CanonicalValue::Object(payload)
    }
}
