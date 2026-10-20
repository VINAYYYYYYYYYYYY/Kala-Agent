//! The request and response envelopes.
//!
//! An envelope is the only thing that crosses the tool boundary, so every
//! field is validated when the envelope is built and the canonical bytes of
//! the envelope bind exactly the fields that decide what the tool runs.

use kala_contracts::{ArtifactRef, ContractModel, Timestamp, ToolCall, ToolResult, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use num_bigint::BigInt;

use crate::protocol::MAX_ARGUMENT_BYTES;
use crate::protocol::MAX_JSON_BYTES;
use crate::protocol::MAX_PATH_LENGTH;
use crate::protocol::enums::{ToolEnvelopeKind, ToolErrorCode, is_failure_status};
use crate::protocol::error::ToolProtocolError;
use crate::protocol::json::{canonical_json_digest, canonical_json_dumps, sha256_hex_text};
use crate::protocol::text::python_strip;
use crate::protocol::workspace::resolve_path;

/// The maximum length of an error message.
const MAX_ERROR_MESSAGE_LENGTH: usize = 2_048;

/// Validate a bounded identifier.
///
/// The identifier grammar is `^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` after the
/// value has been stripped.
fn bounded_identifier(value: &JsonText, field_name: &str) -> Result<String, ToolProtocolError> {
    let normalized = python_strip(value);
    let text = normalized.to_string_lossy();
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe identifier"
        )));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe identifier"
        )));
    }
    if text.len() > 128 {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe identifier"
        )));
    }
    if !characters.all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
    }) {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe identifier"
        )));
    }
    Ok(text)
}

/// Validate a bounded version string.
///
/// The version grammar is `^[A-Za-z0-9][A-Za-z0-9.+_-]{0,63}$` after the
/// value has been stripped.
fn bounded_version(value: &JsonText, field_name: &str) -> Result<String, ToolProtocolError> {
    let normalized = python_strip(value);
    let text = normalized.to_string_lossy();
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe version string"
        )));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe version string"
        )));
    }
    if text.len() > 64 {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe version string"
        )));
    }
    if !characters.all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '+' | '_' | '-')
    }) {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be a safe version string"
        )));
    }
    Ok(text)
}

/// Validate a bounded text field.
fn bounded_text(
    value: &JsonText,
    field_name: &str,
    maximum: usize,
) -> Result<String, ToolProtocolError> {
    let normalized = python_strip(value);
    let text = normalized.to_string_lossy();
    if text.is_empty() {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be nonempty"
        )));
    }
    if normalized.len() > maximum {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} exceeds the bounded length"
        )));
    }
    Ok(text)
}

/// Validate a positive finite number.
fn positive_number(value: f64, field_name: &str) -> Result<f64, ToolProtocolError> {
    if !value.is_finite() || value <= 0.0 {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be finite and positive"
        )));
    }
    Ok(value)
}

/// Validate a positive integer.
fn positive_integer(value: &BigInt, field_name: &str) -> Result<BigInt, ToolProtocolError> {
    if value.sign() != num_bigint::Sign::Plus {
        return Err(ToolProtocolError::envelope(format!(
            "{field_name} must be positive"
        )));
    }
    Ok(value.clone())
}

/// Validate and normalize a collection of capability names.
fn capability_tuple(
    values: &[CanonicalValue],
    field_name: &str,
) -> Result<Vec<String>, ToolProtocolError> {
    let mut names: Vec<String> = Vec::with_capacity(values.len());
    for item in values {
        let CanonicalValue::Text(text) = item else {
            return Err(ToolProtocolError::envelope(format!(
                "{field_name} entry must be a string"
            )));
        };
        let name = bounded_identifier(text, &format!("{field_name} entry"))?;
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// A request envelope: everything a tool needs to run exactly once.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolRequestEnvelope {
    protocol_version: String,
    call_id: String,
    tool_id: String,
    tool_version: String,
    arguments: CanonicalObject,
    arguments_json: String,
    workspace: String,
    deadline_seconds: f64,
    max_output_bytes: BigInt,
    max_artifact_bytes: BigInt,
    granted_capabilities: Vec<String>,
}

impl ToolRequestEnvelope {
    /// Build a request envelope, validating every field.
    ///
    /// The fields are validated in the order of the Python implementation:
    /// the protocol version, the call identity, the arguments, the workspace,
    /// the numeric limits and finally the granted capabilities.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        protocol_version: &str,
        call_id: &str,
        tool_id: &str,
        tool_version: &str,
        arguments: &CanonicalObject,
        workspace: &str,
        deadline_seconds: f64,
        max_output_bytes: &BigInt,
        max_artifact_bytes: &BigInt,
        granted_capabilities: &[String],
    ) -> Result<Self, ToolProtocolError> {
        let protocol_version =
            bounded_version(&JsonText::new(protocol_version), "protocol_version")?;
        let call_id = bounded_identifier(&JsonText::new(call_id), "call_id")?;
        let tool_id = bounded_identifier(&JsonText::new(tool_id), "tool_id")?;
        let tool_version = bounded_version(&JsonText::new(tool_version), "tool_version")?;
        let arguments_json = canonical_json_dumps(
            &CanonicalValue::Object(arguments.clone()),
            MAX_ARGUMENT_BYTES,
        )?;
        let workspace = bounded_text(&JsonText::new(workspace), "workspace", MAX_PATH_LENGTH)?;
        if !workspace.starts_with('/') {
            return Err(ToolProtocolError::envelope(
                "workspace must be an absolute path",
            ));
        }
        let deadline_seconds = positive_number(deadline_seconds, "deadline_seconds")?;
        let max_output_bytes = positive_integer(max_output_bytes, "max_output_bytes")?;
        let max_artifact_bytes = positive_integer(max_artifact_bytes, "max_artifact_bytes")?;
        let granted = capability_tuple(
            &granted_capabilities
                .iter()
                .map(CanonicalValue::text)
                .collect::<Vec<_>>(),
            "granted_capabilities",
        )?;
        Ok(Self::build(
            protocol_version,
            call_id,
            tool_id,
            tool_version,
            arguments.clone(),
            arguments_json,
            workspace,
            deadline_seconds,
            max_output_bytes,
            max_artifact_bytes,
            granted,
        ))
    }

    /// Assemble a validated request envelope.
    #[allow(clippy::too_many_arguments)]
    fn build(
        protocol_version: String,
        call_id: String,
        tool_id: String,
        tool_version: String,
        arguments: CanonicalObject,
        arguments_json: String,
        workspace: String,
        deadline_seconds: f64,
        max_output_bytes: BigInt,
        max_artifact_bytes: BigInt,
        granted_capabilities: Vec<String>,
    ) -> Self {
        Self {
            protocol_version,
            call_id,
            tool_id,
            tool_version,
            arguments,
            arguments_json,
            workspace,
            deadline_seconds,
            max_output_bytes,
            max_artifact_bytes,
            granted_capabilities,
        }
    }

    /// The protocol version.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// The envelope kind, always a request.
    pub fn kind(&self) -> ToolEnvelopeKind {
        ToolEnvelopeKind::Request
    }

    /// The call identifier.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The tool identifier.
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// The tool version.
    pub fn tool_version(&self) -> &str {
        &self.tool_version
    }

    /// The tool arguments.
    pub fn arguments(&self) -> &CanonicalObject {
        &self.arguments
    }

    /// The canonical JSON text of the arguments.
    pub fn arguments_json(&self) -> &str {
        &self.arguments_json
    }

    /// The SHA-256 of the canonical argument text.
    pub fn arguments_digest(&self) -> String {
        sha256_hex_text(&self.arguments_json)
    }

    /// The workspace directory of the run.
    pub fn workspace(&self) -> &str {
        &self.workspace
    }

    /// The deadline in seconds.
    pub fn deadline_seconds(&self) -> f64 {
        self.deadline_seconds
    }

    /// The output byte budget.
    pub fn max_output_bytes(&self) -> &BigInt {
        &self.max_output_bytes
    }

    /// The artifact byte budget.
    pub fn max_artifact_bytes(&self) -> &BigInt {
        &self.max_artifact_bytes
    }

    /// The granted capabilities, sorted and unique.
    pub fn granted_capabilities(&self) -> &[String] {
        &self.granted_capabilities
    }

    /// The canonical payload of the envelope.
    pub fn to_payload(&self) -> CanonicalValue {
        let mut payload = CanonicalObject::new();
        payload.insert(
            JsonText::new("arguments"),
            CanonicalValue::Object(self.arguments.clone()),
        );
        payload.insert(
            JsonText::new("call_id"),
            CanonicalValue::text(&self.call_id),
        );
        payload.insert(
            JsonText::new("deadline_seconds"),
            CanonicalValue::Float(self.deadline_seconds),
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
            JsonText::new("kind"),
            CanonicalValue::text(ToolEnvelopeKind::Request.as_str()),
        );
        payload.insert(
            JsonText::new("max_artifact_bytes"),
            CanonicalValue::Integer(self.max_artifact_bytes.clone()),
        );
        payload.insert(
            JsonText::new("max_output_bytes"),
            CanonicalValue::Integer(self.max_output_bytes.clone()),
        );
        payload.insert(
            JsonText::new("protocol_version"),
            CanonicalValue::text(&self.protocol_version),
        );
        payload.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text(&self.tool_id),
        );
        payload.insert(
            JsonText::new("tool_version"),
            CanonicalValue::text(&self.tool_version),
        );
        payload.insert(
            JsonText::new("workspace"),
            CanonicalValue::text(&self.workspace),
        );
        CanonicalValue::Object(payload)
    }

    /// The canonical JSON text of the envelope.
    pub fn to_json(&self) -> Result<String, ToolProtocolError> {
        canonical_json_dumps(&self.to_payload(), MAX_JSON_BYTES)
    }

    /// The digest that binds every dispatch relevant field of this envelope.
    pub fn envelope_digest(&self) -> Result<String, ToolProtocolError> {
        canonical_json_digest(&self.to_payload())
    }

    /// Rebuild a request envelope from a decoded payload.
    ///
    /// Every member is read first, exactly like the Python implementation,
    /// which builds the constructor arguments before the envelope validates
    /// them. The members are then validated in the order of the Python
    /// implementation, so the reported failure is the same one.
    pub fn from_payload(payload: &CanonicalValue) -> Result<Self, ToolProtocolError> {
        let CanonicalValue::Object(payload) = payload else {
            // The Python implementation reads the members of the payload
            // directly and therefore reports the missing `get` attribute.
            return Err(ToolProtocolError::protocol(format!(
                "'{}' object has no attribute 'get'",
                python_type_name(payload)
            )));
        };
        let protocol_version = extract_text(
            payload,
            "protocol_version",
            crate::protocol::PROTOCOL_VERSION,
        );
        let call_id = extract_text(payload, "call_id", "");
        let tool_id = extract_text(payload, "tool_id", "");
        let tool_version = extract_text(payload, "tool_version", "");
        let arguments = extract_arguments(payload);
        let workspace = extract_text(payload, "workspace", "");
        let deadline_seconds = extract_number(payload, "deadline_seconds");
        let max_output_bytes = extract_integer(payload, "max_output_bytes");
        let max_artifact_bytes = extract_integer(payload, "max_artifact_bytes");
        // The Python implementation converts the capabilities with `tuple(...)`
        // while it builds the constructor arguments, so a member that is not
        // iterable is reported before any field is validated.
        let granted_capabilities = extract_capabilities(payload)?;

        let protocol_version =
            bounded_version(&JsonText::new(&protocol_version?), "protocol_version")?;
        let call_id = bounded_identifier(&JsonText::new(&call_id?), "call_id")?;
        let tool_id = bounded_identifier(&JsonText::new(&tool_id?), "tool_id")?;
        let tool_version = bounded_version(&JsonText::new(&tool_version?), "tool_version")?;
        let arguments = arguments?;
        let arguments_json = canonical_json_dumps(
            &CanonicalValue::Object(arguments.clone()),
            MAX_ARGUMENT_BYTES,
        )?;
        let workspace = bounded_text(&JsonText::new(&workspace?), "workspace", MAX_PATH_LENGTH)?;
        if !workspace.starts_with('/') {
            return Err(ToolProtocolError::envelope(
                "workspace must be an absolute path",
            ));
        }
        let deadline_seconds = positive_number(deadline_seconds?, "deadline_seconds")?;
        let max_output_bytes = positive_integer(&max_output_bytes?, "max_output_bytes")?;
        let max_artifact_bytes = positive_integer(&max_artifact_bytes?, "max_artifact_bytes")?;
        let granted_capabilities = capability_tuple(&granted_capabilities, "granted_capabilities")?;
        Ok(Self::build(
            protocol_version,
            call_id,
            tool_id,
            tool_version,
            arguments,
            arguments_json,
            workspace,
            deadline_seconds,
            max_output_bytes,
            max_artifact_bytes,
            granted_capabilities,
        ))
    }

    /// Rebuild a request envelope from JSON text.
    pub fn from_json(payload: &[u8]) -> Result<Self, ToolProtocolError> {
        let decoded = crate::protocol::json::strict_json_loads(payload, MAX_JSON_BYTES)?;
        Self::from_payload(&decoded)
    }

    /// Build a request envelope from an accepted tool call.
    pub fn from_call(
        call: &ToolCall,
        workspace: &std::path::Path,
        deadline_seconds: f64,
        max_output_bytes: &BigInt,
        max_artifact_bytes: &BigInt,
        granted_capabilities: &[String],
    ) -> Result<Self, ToolProtocolError> {
        let arguments = crate::protocol::json::strict_json_loads(
            call.arguments_json().as_str().as_bytes(),
            MAX_JSON_BYTES,
        )?;
        let CanonicalValue::Object(arguments) = &arguments else {
            return Err(ToolProtocolError::envelope(
                "arguments must be a JSON object",
            ));
        };
        let workspace = resolve_path(workspace);
        let workspace = workspace.to_string_lossy().replace('\\', "/");
        Self::new(
            crate::protocol::PROTOCOL_VERSION,
            call.call_id(),
            call.tool_id(),
            &call.tool_version().to_string_lossy(),
            arguments,
            &workspace,
            deadline_seconds,
            max_output_bytes,
            max_artifact_bytes,
            granted_capabilities,
        )
    }
}

/// A response envelope: what a tool produced.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResponseEnvelope {
    protocol_version: String,
    call_id: String,
    tool_id: String,
    tool_version: String,
    status: ToolRunStatus,
    result: Option<CanonicalObject>,
    error_code: Option<String>,
    error_message: Option<String>,
    artifacts: Vec<ArtifactRef>,
    duration_ms: f64,
}

impl ToolResponseEnvelope {
    /// Build a response envelope, validating every field.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        protocol_version: &str,
        call_id: &str,
        tool_id: &str,
        tool_version: &str,
        status: ToolRunStatus,
        result: Option<&CanonicalObject>,
        error_code: Option<&str>,
        error_message: Option<&str>,
        artifacts: &[ArtifactRef],
        duration_ms: f64,
    ) -> Result<Self, ToolProtocolError> {
        let protocol_version =
            bounded_version(&JsonText::new(protocol_version), "protocol_version")?;
        let call_id = bounded_identifier(&JsonText::new(call_id), "call_id")?;
        let tool_id = bounded_identifier(&JsonText::new(tool_id), "tool_id")?;
        let tool_version = bounded_version(&JsonText::new(tool_version), "tool_version")?;
        let result = match result {
            None => None,
            Some(result) => {
                canonical_json_dumps(&CanonicalValue::Object(result.clone()), MAX_JSON_BYTES)?;
                Some(result.clone())
            }
        };
        let error_code = match error_code {
            None => {
                if is_failure_status(status) {
                    return Err(ToolProtocolError::envelope(
                        "failed tool runs require an error code",
                    ));
                }
                None
            }
            Some(code) => Some(bounded_identifier(&JsonText::new(code), "error_code")?),
        };
        let error_message = match error_message {
            None => None,
            Some(message) => Some(bounded_text(
                &JsonText::new(message),
                "error_message",
                MAX_ERROR_MESSAGE_LENGTH,
            )?),
        };
        if !duration_ms.is_finite() || duration_ms < 0.0 {
            return Err(ToolProtocolError::envelope(
                "duration_ms must be finite and nonnegative",
            ));
        }
        if is_failure_status(status) && result.is_some() {
            return Err(ToolProtocolError::envelope(
                "failed tool runs cannot carry a result payload",
            ));
        }
        if status == ToolRunStatus::Succeeded && result.is_none() {
            return Err(ToolProtocolError::envelope(
                "successful tool runs require a result payload",
            ));
        }
        Ok(Self {
            protocol_version,
            call_id,
            tool_id,
            tool_version,
            status,
            result,
            error_code,
            error_message,
            artifacts: artifacts.to_vec(),
            duration_ms,
        })
    }

    /// The protocol version.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// The envelope kind, always a response.
    pub fn kind(&self) -> ToolEnvelopeKind {
        ToolEnvelopeKind::Response
    }

    /// The call identifier.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// The tool identifier.
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// The tool version.
    pub fn tool_version(&self) -> &str {
        &self.tool_version
    }

    /// The run status.
    pub fn status(&self) -> ToolRunStatus {
        self.status
    }

    /// The result payload.
    pub fn result(&self) -> Option<&CanonicalObject> {
        self.result.as_ref()
    }

    /// The error code.
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    /// The error message.
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }

    /// The produced artifacts.
    pub fn artifacts(&self) -> &[ArtifactRef] {
        &self.artifacts
    }

    /// The run duration in milliseconds.
    pub fn duration_ms(&self) -> f64 {
        self.duration_ms
    }

    /// Whether the run succeeded.
    pub fn succeeded(&self) -> bool {
        self.status == ToolRunStatus::Succeeded
    }

    /// The canonical JSON text of the result payload.
    pub fn result_json(&self) -> Result<Option<String>, ToolProtocolError> {
        match &self.result {
            None => Ok(None),
            Some(result) => Ok(Some(canonical_json_dumps(
                &CanonicalValue::Object(result.clone()),
                MAX_JSON_BYTES,
            )?)),
        }
    }

    /// The SHA-256 of the result payload.
    ///
    /// A run without a result digests the empty object, exactly like the
    /// Python implementation.
    pub fn result_digest(&self) -> Result<String, ToolProtocolError> {
        let payload = match self.result_json()? {
            Some(payload) => payload,
            None => "{}".to_owned(),
        };
        Ok(sha256_hex_text(&payload))
    }

    /// The canonical payload of the envelope.
    pub fn to_payload(&self) -> Result<CanonicalValue, ToolProtocolError> {
        let artifacts = self
            .artifacts
            .iter()
            .map(|artifact| {
                artifact
                    .to_wire_value()
                    .to_canonical_value()
                    .map_err(|error| {
                        ToolProtocolError::envelope(format!(
                            "artifact reference is invalid: {error}"
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut payload = CanonicalObject::new();
        payload.insert(JsonText::new("artifacts"), CanonicalValue::Array(artifacts));
        payload.insert(
            JsonText::new("call_id"),
            CanonicalValue::text(&self.call_id),
        );
        payload.insert(
            JsonText::new("duration_ms"),
            CanonicalValue::Float(self.duration_ms),
        );
        payload.insert(
            JsonText::new("error_code"),
            match &self.error_code {
                None => CanonicalValue::Null,
                Some(code) => CanonicalValue::text(code),
            },
        );
        payload.insert(
            JsonText::new("error_message"),
            match &self.error_message {
                None => CanonicalValue::Null,
                Some(message) => CanonicalValue::text(message),
            },
        );
        payload.insert(
            JsonText::new("kind"),
            CanonicalValue::text(ToolEnvelopeKind::Response.as_str()),
        );
        payload.insert(
            JsonText::new("protocol_version"),
            CanonicalValue::text(&self.protocol_version),
        );
        payload.insert(
            JsonText::new("result"),
            match &self.result {
                None => CanonicalValue::Null,
                Some(result) => CanonicalValue::Object(result.clone()),
            },
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
        Ok(CanonicalValue::Object(payload))
    }

    /// The canonical JSON text of the envelope.
    pub fn to_json(&self) -> Result<String, ToolProtocolError> {
        canonical_json_dumps(&self.to_payload()?, MAX_JSON_BYTES)
    }

    /// Build a successful response envelope.
    pub fn succeeded_envelope(
        call_id: &str,
        tool_id: &str,
        tool_version: &str,
        result: &CanonicalObject,
        artifacts: &[ArtifactRef],
        duration_ms: f64,
    ) -> Result<Self, ToolProtocolError> {
        Self::new(
            crate::protocol::PROTOCOL_VERSION,
            call_id,
            tool_id,
            tool_version,
            ToolRunStatus::Succeeded,
            Some(result),
            None,
            None,
            artifacts,
            duration_ms,
        )
    }

    /// Build a failed response envelope.
    pub fn failed_envelope(
        call_id: &str,
        tool_id: &str,
        tool_version: &str,
        status: ToolRunStatus,
        error_code: Option<&str>,
        error_message: Option<&str>,
        duration_ms: f64,
    ) -> Result<Self, ToolProtocolError> {
        Self::new(
            crate::protocol::PROTOCOL_VERSION,
            call_id,
            tool_id,
            tool_version,
            status,
            None,
            error_code,
            error_message,
            &[],
            duration_ms,
        )
    }

    /// Build a failed response envelope for an error code.
    pub fn failed_envelope_with_code(
        call_id: &str,
        tool_id: &str,
        tool_version: &str,
        status: ToolRunStatus,
        error_code: ToolErrorCode,
        error_message: Option<&str>,
        duration_ms: f64,
    ) -> Result<Self, ToolProtocolError> {
        Self::failed_envelope(
            call_id,
            tool_id,
            tool_version,
            status,
            Some(error_code.as_str()),
            error_message,
            duration_ms,
        )
    }

    /// Rebuild a response envelope from a decoded payload.
    ///
    /// The artifacts, the status and the result are read first, exactly like
    /// the Python implementation, which builds the constructor arguments
    /// before the envelope validates them.
    pub fn from_payload(payload: &CanonicalValue) -> Result<Self, ToolProtocolError> {
        let CanonicalValue::Object(payload) = payload else {
            // The Python implementation reads the members of the payload
            // directly and therefore reports the missing `get` attribute.
            return Err(ToolProtocolError::protocol(format!(
                "'{}' object has no attribute 'get'",
                python_type_name(payload)
            )));
        };
        let raw_artifacts = payload.get(&JsonText::new("artifacts"));
        let artifacts = match raw_artifacts {
            None => Vec::new(),
            Some(CanonicalValue::Array(items)) => {
                let mut artifacts = Vec::with_capacity(items.len());
                for item in items {
                    let CanonicalValue::Object(_) = item else {
                        return Err(ToolProtocolError::envelope(
                            "artifacts must be ArtifactRef objects",
                        ));
                    };
                    match ArtifactRef::from_value(item) {
                        Ok(artifact) => artifacts.push(artifact),
                        Err(_) => {
                            return Err(ToolProtocolError::envelope(
                                "artifact reference is invalid",
                            ));
                        }
                    }
                }
                artifacts
            }
            Some(_) => {
                return Err(ToolProtocolError::envelope("artifacts must be an array"));
            }
        };
        let status = match payload.get(&JsonText::new("status")) {
            Some(CanonicalValue::Text(text)) => {
                let value = text.to_string_lossy();
                ToolRunStatus::parse(&value)
                    .ok_or_else(|| ToolProtocolError::envelope("status is not a tool run status"))?
            }
            _ => {
                return Err(ToolProtocolError::envelope(
                    "status is not a tool run status",
                ));
            }
        };
        let result = match payload.get(&JsonText::new("result")) {
            None | Some(CanonicalValue::Null) => None,
            Some(CanonicalValue::Object(result)) => Some(result.clone()),
            Some(_) => {
                return Err(ToolProtocolError::envelope("result must be a JSON object"));
            }
        };
        let protocol_version = extract_text(
            payload,
            "protocol_version",
            crate::protocol::PROTOCOL_VERSION,
        );
        let call_id = extract_text(payload, "call_id", "");
        let tool_id = extract_text(payload, "tool_id", "");
        let tool_version = extract_text(payload, "tool_version", "");
        let error_code = extract_optional_text(payload, "error_code");
        let error_message = extract_optional_text(payload, "error_message");
        let duration_ms = extract_number(payload, "duration_ms");

        let protocol_version =
            bounded_version(&JsonText::new(&protocol_version?), "protocol_version")?;
        let call_id = bounded_identifier(&JsonText::new(&call_id?), "call_id")?;
        let tool_id = bounded_identifier(&JsonText::new(&tool_id?), "tool_id")?;
        let tool_version = bounded_version(&JsonText::new(&tool_version?), "tool_version")?;
        let error_code = error_code?;
        let error_message = error_message?;
        let duration_ms = duration_ms?;
        Self::new(
            &protocol_version,
            &call_id,
            &tool_id,
            &tool_version,
            status,
            result.as_ref(),
            error_code.as_deref(),
            error_message.as_deref(),
            &artifacts,
            duration_ms,
        )
    }

    /// Rebuild a response envelope from JSON text.
    pub fn from_json(payload: &[u8]) -> Result<Self, ToolProtocolError> {
        let decoded = crate::protocol::json::strict_json_loads(payload, MAX_JSON_BYTES)?;
        Self::from_payload(&decoded)
    }

    /// Record the envelope as an accepted tool result.
    ///
    /// The Python implementation refuses naive timestamps; the accepted
    /// contract timestamps always carry an offset, so only the ordering rule
    /// remains.
    pub fn to_result(
        &self,
        started_at: &Timestamp,
        finished_at: &Timestamp,
    ) -> Result<ToolResult, ToolProtocolError> {
        if finished_at < started_at {
            return Err(ToolProtocolError::envelope(
                "finished_at cannot precede started_at",
            ));
        }
        let result_json = self.result_json()?;
        let mut payload = CanonicalObject::new();
        payload.insert(
            JsonText::new("call_id"),
            CanonicalValue::text(&self.call_id),
        );
        payload.insert(
            JsonText::new("tool_id"),
            CanonicalValue::text(&self.tool_id),
        );
        payload.insert(
            JsonText::new("status"),
            CanonicalValue::text(self.status.as_str()),
        );
        payload.insert(
            JsonText::new("result_json"),
            match &result_json {
                None => CanonicalValue::Null,
                Some(text) => CanonicalValue::text(text),
            },
        );
        payload.insert(
            JsonText::new("error_code"),
            match &self.error_code {
                None => CanonicalValue::Null,
                Some(code) => CanonicalValue::text(code),
            },
        );
        payload.insert(
            JsonText::new("error_message"),
            match &self.error_message {
                None => CanonicalValue::Null,
                Some(message) => CanonicalValue::text(message),
            },
        );
        payload.insert(
            JsonText::new("artifacts"),
            CanonicalValue::Array(
                self.artifacts
                    .iter()
                    .map(|artifact| {
                        artifact
                            .to_wire_value()
                            .to_canonical_value()
                            .map_err(|error| {
                                ToolProtocolError::envelope(format!(
                                    "artifact reference is invalid: {error}"
                                ))
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        );
        payload.insert(
            JsonText::new("started_at"),
            CanonicalValue::text(started_at.to_wire_string()),
        );
        payload.insert(
            JsonText::new("finished_at"),
            CanonicalValue::text(finished_at.to_wire_string()),
        );
        payload.insert(
            JsonText::new("duration_ms"),
            CanonicalValue::Float(self.duration_ms),
        );
        ToolResult::from_value(&CanonicalValue::Object(payload)).map_err(|error| {
            ToolProtocolError::envelope(format!("the tool result is invalid: {error}"))
        })
    }
}

/// The numeric value of a JSON number.
fn number_value(value: &CanonicalValue) -> f64 {
    match value {
        CanonicalValue::Integer(number) => {
            use num_traits::ToPrimitive;
            number.to_f64().unwrap_or_else(|| {
                if number.sign() == num_bigint::Sign::Minus {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                }
            })
        }
        CanonicalValue::Float(number) => *number,
        _ => 0.0,
    }
}

/// The integer value of a JSON number.
fn integer_value(value: &CanonicalValue) -> BigInt {
    match value {
        CanonicalValue::Integer(number) => number.clone(),
        CanonicalValue::Float(number) => BigInt::from(*number as i64),
        _ => BigInt::from(0),
    }
}

/// Read a text member of a payload.
///
/// A member that is present but is not a string is a type failure, reported
/// with the field name exactly like the Python implementation.
fn extract_text(
    payload: &CanonicalObject,
    key: &str,
    default: &str,
) -> Result<String, ToolProtocolError> {
    match payload.get(&JsonText::new(key)) {
        None => Ok(default.to_owned()),
        Some(CanonicalValue::Text(text)) => Ok(text.to_string_lossy()),
        Some(_) => Err(ToolProtocolError::envelope(format!(
            "{key} must be a string"
        ))),
    }
}

/// Read the arguments member of a payload.
fn extract_arguments(payload: &CanonicalObject) -> Result<CanonicalObject, ToolProtocolError> {
    match payload.get(&JsonText::new("arguments")) {
        None => Ok(CanonicalObject::new()),
        Some(CanonicalValue::Object(arguments)) => Ok(arguments.clone()),
        Some(_) => Err(ToolProtocolError::envelope(
            "arguments must be a JSON object",
        )),
    }
}

/// Read a numeric member of a payload.
fn extract_number(payload: &CanonicalObject, key: &str) -> Result<f64, ToolProtocolError> {
    match payload.get(&JsonText::new(key)) {
        None => Ok(0.0),
        Some(CanonicalValue::Integer(_)) | Some(CanonicalValue::Float(_)) => Ok(number_value(
            payload.get(&JsonText::new(key)).expect("number"),
        )),
        Some(_) => Err(ToolProtocolError::envelope(format!(
            "{key} must be a number"
        ))),
    }
}

/// Read an integer member of a payload.
fn extract_integer(payload: &CanonicalObject, key: &str) -> Result<BigInt, ToolProtocolError> {
    match payload.get(&JsonText::new(key)) {
        None => Ok(BigInt::from(0)),
        Some(CanonicalValue::Integer(_)) => Ok(integer_value(
            payload.get(&JsonText::new(key)).expect("integer"),
        )),
        Some(CanonicalValue::Float(_)) => Err(ToolProtocolError::envelope(format!(
            "{key} must be an integer"
        ))),
        Some(_) => Err(ToolProtocolError::envelope(format!(
            "{key} must be an integer"
        ))),
    }
}

/// Read an optional text member of a payload.
///
/// A member that is present but is not a string is a type failure.
fn extract_optional_text(
    payload: &CanonicalObject,
    key: &str,
) -> Result<Option<String>, ToolProtocolError> {
    match payload.get(&JsonText::new(key)) {
        None | Some(CanonicalValue::Null) => Ok(None),
        Some(CanonicalValue::Text(text)) => Ok(Some(text.to_string_lossy())),
        Some(_) => Err(ToolProtocolError::envelope(format!(
            "{key} must be a string"
        ))),
    }
}

/// The Python type name of a JSON value.
fn python_type_name(value: &CanonicalValue) -> &'static str {
    match value {
        CanonicalValue::Null => "NoneType",
        CanonicalValue::Bool(_) => "bool",
        CanonicalValue::Integer(_) => "int",
        CanonicalValue::Float(_) => "float",
        CanonicalValue::Text(_) => "str",
        CanonicalValue::Array(_) => "list",
        CanonicalValue::Object(_) => "dict",
    }
}

/// Read the granted capabilities member of a payload.
///
/// The Python implementation converts the member with `tuple(...)`, so a
/// string becomes a tuple of its characters and an object becomes a tuple of
/// its keys. A member that is not iterable is a `TypeError`.
fn extract_capabilities(
    payload: &CanonicalObject,
) -> Result<Vec<CanonicalValue>, ToolProtocolError> {
    match payload.get(&JsonText::new("granted_capabilities")) {
        None => Ok(Vec::new()),
        Some(CanonicalValue::Array(items)) => Ok(items.clone()),
        Some(CanonicalValue::Object(entries)) => Ok(entries
            .keys()
            .map(|key| CanonicalValue::Text(key.clone()))
            .collect()),
        Some(CanonicalValue::Text(text)) => Ok(text
            .code_points()
            .iter()
            .map(|code_point| {
                CanonicalValue::Text(
                    JsonText::from_code_points(vec![*code_point]).expect("code point"),
                )
            })
            .collect()),
        Some(other) => Err(ToolProtocolError::protocol(format!(
            "'{}' object is not iterable",
            python_type_name(other)
        ))),
    }
}
