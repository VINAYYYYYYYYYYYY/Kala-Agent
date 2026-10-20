//! The model gateway data types.
//!
//! Every type here mirrors one Python dataclass of `kala_agent.runtime.
//! model_gateway`. The Python dataclasses validate their fields in
//! `__post_init__`; the Rust port validates them in the constructor and in
//! every `with_*` method, so a value can never exist in an invalid state.

use kala_budget::Decimal;
use kala_contracts::ArtifactClassification;
use kala_json::CanonicalValue;
use kala_kernel::time::KernelTimestamp;
use kala_tools::protocol::{canonical_json_dumps, sha256_hex_text};

use super::error::ModelError;
use super::text::code_point_len;
use super::value::{
    DecimalInput, bounded_float, bounded_identifier, bounded_text, freeze_json,
    nonnegative_decimal, nonnegative_int, thaw_json,
};

/// The maximum number of messages a request may carry.
pub const MAX_MESSAGES: usize = 128;
/// The maximum length of a message body, in Unicode code points.
pub const MAX_MESSAGE_LENGTH: usize = 32_768;
/// The maximum size of a canonical request body, in bytes.
pub const MAX_REQUEST_BYTES: usize = 1_048_576;
/// The maximum size of a stored transcript body, in bytes.
pub const MAX_TRANSCRIPT_BODY_BYTES: usize = 65_536;
/// The maximum size of a provider response, in bytes.
pub const MAX_RESPONSE_BYTES: usize = 1_048_576;
/// The maximum number of output tokens a request may ask for.
pub const MAX_OUTPUT_TOKENS: i128 = 32_768;
/// The maximum seed value a request may carry.
pub const MAX_SEED: i128 = 2_147_483_647;
/// The smallest accepted request timeout, in seconds.
pub const MIN_TIMEOUT_SECONDS: f64 = 0.1;
/// The largest accepted request timeout, in seconds.
pub const MAX_TIMEOUT_SECONDS: f64 = 600.0;
/// The number of characters the local estimator counts as one token.
pub const CHARS_PER_TOKEN: usize = 4;
/// The provider identifier of the deterministic local client.
pub const LOCAL_PROVIDER_ID: &str = "local";
/// The provider identifier of the OpenAI-compatible client.
pub const OPENAI_COMPATIBLE_PROVIDER_ID: &str = "openai-compatible";
/// The capabilities an egress request asks for.
pub const EGRESS_CAPABILITIES: &[&str] = &["model.egress"];

/// The role of a model message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ModelRole {
    /// The system instruction.
    System,
    /// The user turn.
    User,
    /// The assistant turn.
    Assistant,
}

impl ModelRole {
    /// Every accepted value, in Python declaration order.
    pub const ALL: &'static [Self] = &[Self::System, Self::User, Self::Assistant];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
    }
}

impl std::fmt::Display for ModelRole {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A capability a model provider may declare.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ModelCapability {
    /// The provider can answer with a schema-conforming object.
    StructuredOutput,
    /// The provider honours a deterministic seed.
    Seed,
    /// The provider can call tools.
    ToolCalling,
}

impl ModelCapability {
    /// Every accepted value, in Python declaration order.
    pub const ALL: &'static [Self] = &[Self::StructuredOutput, Self::Seed, Self::ToolCalling];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StructuredOutput => "structured_output",
            Self::Seed => "seed",
            Self::ToolCalling => "tool_calling",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
    }
}

impl std::fmt::Display for ModelCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a provider stopped generating.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ModelFinishReason {
    /// The provider finished normally.
    Stop,
    /// The provider hit the output bound.
    Length,
    /// The provider refused.
    Refusal,
    /// The provider failed.
    Error,
}

impl ModelFinishReason {
    /// Every accepted value, in Python declaration order.
    pub const ALL: &'static [Self] = &[Self::Stop, Self::Length, Self::Refusal, Self::Error];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::Refusal => "refusal",
            Self::Error => "error",
        }
    }

    /// Parse an exact Python enum value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
    }
}

impl std::fmt::Display for ModelFinishReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A timestamp that may or may not carry a timezone.
///
/// CPython's `datetime` distinguishes an aware value from a naive one and the
/// gateway refuses a naive timestamp. The Rust port keeps that distinction
/// explicit so the refusal stays observable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatewayTimestamp {
    /// A timezone-aware instant.
    Aware(KernelTimestamp),
    /// A naive local time, which the gateway refuses.
    Naive(chrono::NaiveDateTime),
}

impl GatewayTimestamp {
    /// The current UTC instant.
    #[must_use]
    pub fn now() -> Self {
        Self::Aware(KernelTimestamp::now())
    }

    /// Whether the timestamp carries a timezone.
    #[must_use]
    pub fn is_aware(&self) -> bool {
        matches!(self, Self::Aware(_))
    }

    /// The aware instant, when the timestamp carries a timezone.
    #[must_use]
    pub fn as_aware(&self) -> Option<KernelTimestamp> {
        match self {
            Self::Aware(value) => Some(*value),
            Self::Naive(_) => None,
        }
    }
}

/// One message of a model request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelMessage {
    role: ModelRole,
    content: String,
}

impl ModelMessage {
    /// Build and validate a message.
    pub fn new(role: ModelRole, content: &str) -> Result<Self, ModelError> {
        let content = bounded_text(content, "content", MAX_MESSAGE_LENGTH)?;
        Ok(Self { role, content })
    }

    /// Build and validate a message from the Python role text.
    pub fn from_text(role: &str, content: &str) -> Result<Self, ModelError> {
        let role = ModelRole::parse(role)
            .ok_or_else(|| ModelError::request("role is not a model message role"))?;
        Self::new(role, content)
    }

    /// The message role.
    #[must_use]
    pub fn role(&self) -> ModelRole {
        self.role
    }

    /// The message content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("content", CanonicalValue::text(self.content.clone())),
            ("role", CanonicalValue::text(self.role.as_str())),
        ])
    }
}

/// An artifact reference that may travel with a model request.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelArtifact {
    artifact_id: String,
    digest: String,
    classification: ArtifactClassification,
    media_type: String,
    size_bytes: i128,
    preview_text: Option<String>,
    path: Option<String>,
    metadata: CanonicalValue,
}

impl ModelArtifact {
    /// Build and validate an artifact reference.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        artifact_id: &str,
        digest: &str,
        classification: ArtifactClassification,
        media_type: &str,
        size_bytes: i128,
    ) -> Result<Self, ModelError> {
        let artifact_id = bounded_identifier(artifact_id, "artifact_id")?;
        let digest = normalize_digest(digest)?;
        let media_type = bounded_text(media_type, "media_type", 255)?.to_lowercase();
        let size_bytes = nonnegative_int(size_bytes, "size_bytes")?;
        Ok(Self {
            artifact_id,
            digest,
            classification,
            media_type,
            size_bytes,
            preview_text: None,
            path: None,
            metadata: CanonicalValue::object(std::iter::empty::<(&str, CanonicalValue)>()),
        })
    }

    /// Build and validate an artifact reference from the Python class text.
    #[allow(clippy::too_many_arguments)]
    pub fn from_text(
        artifact_id: &str,
        digest: &str,
        classification: &str,
        media_type: &str,
        size_bytes: i128,
        preview_text: Option<&str>,
        path: Option<&str>,
        metadata: Option<&CanonicalValue>,
    ) -> Result<Self, ModelError> {
        let classification = ArtifactClassification::parse(classification).ok_or_else(|| {
            ModelError::request("classification is not an artifact classification")
        })?;
        let mut artifact = Self::new(artifact_id, digest, classification, media_type, size_bytes)?;
        artifact = artifact.with_preview_text(preview_text)?;
        artifact = artifact.with_path(path)?;
        if let Some(metadata) = metadata {
            artifact = artifact.with_metadata(metadata)?;
        }
        Ok(artifact)
    }

    /// Return a copy with a different preview text.
    pub fn with_preview_text(mut self, value: Option<&str>) -> Result<Self, ModelError> {
        self.preview_text = match value {
            Some(value) => Some(bounded_text(value, "preview_text", 8_192)?),
            None => None,
        };
        Ok(self)
    }

    /// Return a copy with a different path.
    pub fn with_path(mut self, value: Option<&str>) -> Result<Self, ModelError> {
        self.path = match value {
            Some(value) => Some(bounded_text(value, "path", 1_024)?),
            None => None,
        };
        Ok(self)
    }

    /// Return a copy with different metadata.
    pub fn with_metadata(mut self, value: &CanonicalValue) -> Result<Self, ModelError> {
        if !matches!(value, CanonicalValue::Object(_)) {
            return Err(ModelError::request("metadata must be a JSON object"));
        }
        self.metadata = freeze_json(value)?;
        Ok(self)
    }

    /// The artifact identifier.
    #[must_use]
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    /// The content digest.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The artifact classification.
    #[must_use]
    pub fn classification(&self) -> ArtifactClassification {
        self.classification
    }

    /// The media type.
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    /// The artifact size in bytes.
    #[must_use]
    pub fn size_bytes(&self) -> i128 {
        self.size_bytes
    }

    /// The bounded preview text.
    #[must_use]
    pub fn preview_text(&self) -> Option<&str> {
        self.preview_text.as_deref()
    }

    /// The local path of the artifact.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// The artifact metadata.
    #[must_use]
    pub fn metadata(&self) -> &CanonicalValue {
        &self.metadata
    }

    /// Whether the artifact is a design binary.
    #[must_use]
    pub fn is_design_binary(&self) -> bool {
        self.classification == ArtifactClassification::DesignBinary
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        let mut entries: Vec<(&str, CanonicalValue)> = vec![
            (
                "artifact_id",
                CanonicalValue::text(self.artifact_id.clone()),
            ),
            (
                "classification",
                CanonicalValue::text(self.classification.as_str()),
            ),
            ("digest", CanonicalValue::text(self.digest.clone())),
            ("media_type", CanonicalValue::text(self.media_type.clone())),
            ("metadata", thaw_json(&self.metadata)),
            ("size_bytes", CanonicalValue::integer(self.size_bytes)),
        ];
        if let Some(preview_text) = &self.preview_text {
            entries.push(("preview_text", CanonicalValue::text(preview_text.clone())));
        }
        if let Some(path) = &self.path {
            entries.push(("path", CanonicalValue::text(path.clone())));
        }
        CanonicalValue::object(entries)
    }

    /// The Python `redacted()` shape: no preview, no path and no metadata.
    #[must_use]
    pub fn redacted(&self) -> Self {
        Self {
            preview_text: None,
            path: None,
            metadata: CanonicalValue::object(std::iter::empty::<(&str, CanonicalValue)>()),
            ..self.clone()
        }
    }
}

fn normalize_digest(digest: &str) -> Result<String, ModelError> {
    let normalized = digest.trim().to_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ModelError::request(
            "digest must be a sha256 hexadecimal digest",
        ));
    }
    Ok(normalized)
}

/// A model request.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelRequest {
    request_id: String,
    messages: Vec<ModelMessage>,
    capabilities: Vec<ModelCapability>,
    response_schema: Option<CanonicalValue>,
    artifacts: Vec<ModelArtifact>,
    seed: Option<i128>,
    max_output_tokens: i128,
    timeout_seconds: f64,
}

impl ModelRequest {
    /// Build and validate a request from every field at once.
    ///
    /// The fields are validated in the same order as the Python
    /// `__post_init__` method, so an invalid request reports the same first
    /// failure on both sides.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        request_id: &str,
        messages: Vec<ModelMessage>,
        capabilities: Vec<ModelCapability>,
        response_schema: Option<&CanonicalValue>,
        artifacts: Vec<ModelArtifact>,
        seed: Option<i128>,
        max_output_tokens: i128,
        timeout_seconds: f64,
    ) -> Result<Self, ModelError> {
        let request_id = bounded_identifier(request_id, "request_id")?;
        if messages.is_empty() {
            return Err(ModelError::request(
                "a model request requires at least one message",
            ));
        }
        if messages.len() > MAX_MESSAGES {
            return Err(ModelError::request("messages exceed the bounded count"));
        }
        let mut capabilities = capabilities;
        capabilities.sort();
        capabilities.dedup();
        if let Some(schema) = response_schema {
            kala_tools::protocol::check_object_schema(schema, "response_schema")
                .map_err(|error| ModelError::request(error.to_string()))?;
        }
        let mut seen: Vec<&str> = Vec::new();
        for artifact in &artifacts {
            if seen.contains(&artifact.artifact_id.as_str()) {
                return Err(ModelError::request("artifact identifiers must be unique"));
            }
            seen.push(artifact.artifact_id());
        }
        let seed = match seed {
            Some(seed) => {
                let seed = nonnegative_int(seed, "seed")?;
                if seed > MAX_SEED {
                    return Err(ModelError::request("seed exceeds the supported bound"));
                }
                Some(seed)
            }
            None => None,
        };
        let max_output_tokens = nonnegative_int(max_output_tokens, "max_output_tokens")?;
        if max_output_tokens > MAX_OUTPUT_TOKENS {
            return Err(ModelError::request(
                "max_output_tokens exceeds the supported bound",
            ));
        }
        let timeout_seconds = bounded_float(
            timeout_seconds,
            "timeout_seconds",
            MIN_TIMEOUT_SECONDS,
            MAX_TIMEOUT_SECONDS,
        )?;
        let request = Self {
            request_id,
            messages,
            capabilities,
            response_schema: response_schema.cloned(),
            artifacts,
            seed,
            max_output_tokens,
            timeout_seconds,
        };
        request.validate()?;
        Ok(request)
    }

    /// Build and validate a request.
    pub fn new(request_id: &str, messages: Vec<ModelMessage>) -> Result<Self, ModelError> {
        let request_id = bounded_identifier(request_id, "request_id")?;
        if messages.is_empty() {
            return Err(ModelError::request(
                "a model request requires at least one message",
            ));
        }
        if messages.len() > MAX_MESSAGES {
            return Err(ModelError::request("messages exceed the bounded count"));
        }
        Ok(Self {
            request_id,
            messages,
            capabilities: Vec::new(),
            response_schema: None,
            artifacts: Vec::new(),
            seed: None,
            max_output_tokens: 1_024,
            timeout_seconds: 60.0,
        })
    }

    /// Return a copy with a different message set.
    pub fn with_messages(mut self, messages: Vec<ModelMessage>) -> Result<Self, ModelError> {
        if messages.is_empty() {
            return Err(ModelError::request(
                "a model request requires at least one message",
            ));
        }
        if messages.len() > MAX_MESSAGES {
            return Err(ModelError::request("messages exceed the bounded count"));
        }
        self.messages = messages;
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different capability set.
    pub fn with_capabilities(
        mut self,
        mut capabilities: Vec<ModelCapability>,
    ) -> Result<Self, ModelError> {
        capabilities.sort();
        capabilities.dedup();
        self.capabilities = capabilities;
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different response schema.
    pub fn with_response_schema(
        mut self,
        schema: Option<&CanonicalValue>,
    ) -> Result<Self, ModelError> {
        if let Some(schema) = schema {
            kala_tools::protocol::check_object_schema(schema, "response_schema")
                .map_err(|error| ModelError::request(error.to_string()))?;
        }
        self.response_schema = schema.cloned();
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different artifact set.
    pub fn with_artifacts(mut self, artifacts: Vec<ModelArtifact>) -> Result<Self, ModelError> {
        let mut seen: Vec<&str> = Vec::new();
        for artifact in &artifacts {
            if seen.contains(&artifact.artifact_id.as_str()) {
                return Err(ModelError::request("artifact identifiers must be unique"));
            }
            seen.push(artifact.artifact_id());
        }
        self.artifacts = artifacts;
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different seed.
    pub fn with_seed(mut self, seed: Option<i128>) -> Result<Self, ModelError> {
        self.seed = match seed {
            Some(seed) => {
                let seed = nonnegative_int(seed, "seed")?;
                if seed > MAX_SEED {
                    return Err(ModelError::request("seed exceeds the supported bound"));
                }
                Some(seed)
            }
            None => None,
        };
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different output bound.
    pub fn with_max_output_tokens(mut self, value: i128) -> Result<Self, ModelError> {
        let value = nonnegative_int(value, "max_output_tokens")?;
        if value > MAX_OUTPUT_TOKENS {
            return Err(ModelError::request(
                "max_output_tokens exceeds the supported bound",
            ));
        }
        self.max_output_tokens = value;
        self.validate()?;
        Ok(self)
    }

    /// Return a copy with a different timeout.
    pub fn with_timeout_seconds(mut self, value: f64) -> Result<Self, ModelError> {
        self.timeout_seconds = bounded_float(
            value,
            "timeout_seconds",
            MIN_TIMEOUT_SECONDS,
            MAX_TIMEOUT_SECONDS,
        )?;
        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<(), ModelError> {
        if self
            .capabilities
            .contains(&ModelCapability::StructuredOutput)
            && self.response_schema.is_none()
        {
            return Err(ModelError::capability(
                "structured output requires a response schema",
            ));
        }
        if self.capabilities.contains(&ModelCapability::Seed) && self.seed.is_none() {
            return Err(ModelError::capability(
                "a seeded request requires a seed value",
            ));
        }
        Ok(())
    }

    /// The request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// The request messages.
    #[must_use]
    pub fn messages(&self) -> &[ModelMessage] {
        &self.messages
    }

    /// The declared capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &[ModelCapability] {
        &self.capabilities
    }

    /// The response schema.
    #[must_use]
    pub fn response_schema(&self) -> Option<&CanonicalValue> {
        self.response_schema.as_ref()
    }

    /// The artifact references.
    #[must_use]
    pub fn artifacts(&self) -> &[ModelArtifact] {
        &self.artifacts
    }

    /// The deterministic seed.
    #[must_use]
    pub fn seed(&self) -> Option<i128> {
        self.seed
    }

    /// The output token bound.
    #[must_use]
    pub fn max_output_tokens(&self) -> i128 {
        self.max_output_tokens
    }

    /// The request timeout in seconds.
    #[must_use]
    pub fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds
    }

    /// Whether the request expects a structured answer.
    #[must_use]
    pub fn requires_structured_output(&self) -> bool {
        self.response_schema.is_some()
    }

    /// The canonical request body.
    pub fn request_body(&self) -> Result<String, ModelError> {
        let mut capabilities: Vec<&str> = self
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect();
        capabilities.sort_unstable();
        let body = CanonicalValue::object([
            (
                "artifacts",
                CanonicalValue::Array(
                    self.artifacts
                        .iter()
                        .map(ModelArtifact::to_payload)
                        .collect(),
                ),
            ),
            (
                "capabilities",
                CanonicalValue::Array(capabilities.into_iter().map(CanonicalValue::text).collect()),
            ),
            (
                "max_output_tokens",
                CanonicalValue::integer(self.max_output_tokens),
            ),
            (
                "messages",
                CanonicalValue::Array(self.messages.iter().map(ModelMessage::to_payload).collect()),
            ),
            ("request_id", CanonicalValue::text(self.request_id.clone())),
            (
                "response_schema",
                match &self.response_schema {
                    Some(schema) => thaw_json(schema),
                    None => CanonicalValue::Null,
                },
            ),
            (
                "seed",
                match self.seed {
                    Some(seed) => CanonicalValue::integer(seed),
                    None => CanonicalValue::Null,
                },
            ),
        ]);
        canonical_json_dumps(&body, MAX_REQUEST_BYTES)
            .map_err(|error| ModelError::request(error.to_string()))
    }

    /// The digest of the canonical request body.
    pub fn request_digest(&self) -> Result<String, ModelError> {
        Ok(sha256_hex_text(&self.request_body()?))
    }
}

/// A provider answer.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelResponse {
    request_id: String,
    provider_id: String,
    model_id: String,
    content: String,
    structured: Option<CanonicalValue>,
    input_tokens: i128,
    output_tokens: i128,
    cost: Decimal,
    finish_reason: ModelFinishReason,
    provider_request_id: Option<String>,
}

impl ModelResponse {
    /// Build and validate a provider answer.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: &str,
        provider_id: &str,
        model_id: &str,
        content: &str,
    ) -> Result<Self, ModelError> {
        let request_id = bounded_identifier(request_id, "request_id")?;
        let provider_id = bounded_identifier(provider_id, "provider_id")?;
        let model_id = bounded_identifier(model_id, "model_id")?;
        let content = bounded_text(content, "content", MAX_RESPONSE_BYTES)?;
        Ok(Self {
            request_id,
            provider_id,
            model_id,
            content,
            structured: None,
            input_tokens: 0,
            output_tokens: 0,
            cost: Decimal::zero(),
            finish_reason: ModelFinishReason::Stop,
            provider_request_id: None,
        })
    }

    /// Return a copy with a different answer body.
    pub fn with_content(mut self, content: &str) -> Result<Self, ModelError> {
        self.content = bounded_text(content, "content", MAX_RESPONSE_BYTES)?;
        Ok(self)
    }

    /// Return a copy with a different structured answer.
    pub fn with_structured(mut self, value: Option<&CanonicalValue>) -> Result<Self, ModelError> {
        self.structured = match value {
            Some(value) => {
                if !matches!(value, CanonicalValue::Object(_)) {
                    return Err(ModelError::provider(
                        "structured output must be a JSON object",
                    ));
                }
                Some(freeze_json(value)?)
            }
            None => None,
        };
        Ok(self)
    }

    /// Return a copy with different token counters.
    pub fn with_tokens(
        mut self,
        input_tokens: i128,
        output_tokens: i128,
    ) -> Result<Self, ModelError> {
        self.input_tokens = nonnegative_int(input_tokens, "input_tokens")?;
        self.output_tokens = nonnegative_int(output_tokens, "output_tokens")?;
        Ok(self)
    }

    /// Return a copy with a different cost.
    pub fn with_cost(mut self, cost: &DecimalInput) -> Result<Self, ModelError> {
        self.cost = nonnegative_decimal(cost, "cost")?;
        Ok(self)
    }

    /// Return a copy with a different finish reason.
    pub fn with_finish_reason(mut self, finish_reason: ModelFinishReason) -> Self {
        self.finish_reason = finish_reason;
        self
    }

    /// Return a copy with a different provider request identifier.
    pub fn with_provider_request_id(mut self, value: Option<&str>) -> Result<Self, ModelError> {
        self.provider_request_id = match value {
            Some(value) => Some(bounded_text(value, "provider_request_id", 256)?),
            None => None,
        };
        Ok(self)
    }

    /// The request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// The provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// The answer content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// The structured answer.
    #[must_use]
    pub fn structured(&self) -> Option<&CanonicalValue> {
        self.structured.as_ref()
    }

    /// The input token counter.
    #[must_use]
    pub fn input_tokens(&self) -> i128 {
        self.input_tokens
    }

    /// The output token counter.
    #[must_use]
    pub fn output_tokens(&self) -> i128 {
        self.output_tokens
    }

    /// The total token counter.
    #[must_use]
    pub fn total_tokens(&self) -> i128 {
        self.input_tokens + self.output_tokens
    }

    /// The estimated cost.
    #[must_use]
    pub fn cost(&self) -> &Decimal {
        &self.cost
    }

    /// The finish reason.
    #[must_use]
    pub fn finish_reason(&self) -> ModelFinishReason {
        self.finish_reason
    }

    /// The provider request identifier.
    #[must_use]
    pub fn provider_request_id(&self) -> Option<&str> {
        self.provider_request_id.as_deref()
    }

    /// The digest of the answer content.
    #[must_use]
    pub fn response_digest(&self) -> String {
        sha256_hex_text(&self.content)
    }

    /// The canonical answer body.
    pub fn canonical_content(&self) -> Result<String, ModelError> {
        match &self.structured {
            None => Ok(self.content.clone()),
            Some(structured) => canonical_json_dumps(&thaw_json(structured), MAX_RESPONSE_BYTES)
                .map_err(|error| ModelError::request(error.to_string())),
        }
    }
}

/// One redaction applied to a request or a response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRedaction {
    kind: String,
    subject: String,
    reason: String,
}

impl ModelRedaction {
    /// Build a redaction record.
    pub fn new(
        kind: impl Into<String>,
        subject: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            subject: subject.into(),
            reason: reason.into(),
        }
    }

    /// The redaction kind.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The redaction subject.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The redaction reason.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("kind", CanonicalValue::text(self.kind.clone())),
            ("reason", CanonicalValue::text(self.reason.clone())),
            ("subject", CanonicalValue::text(self.subject.clone())),
        ])
    }
}

/// A stored transcript of one model interaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelTranscript {
    request_id: String,
    provider_id: String,
    model_id: String,
    request_digest: String,
    response_digest: String,
    request_body: String,
    response_body: String,
    input_tokens: i128,
    output_tokens: i128,
    cost: Decimal,
    redactions: Vec<ModelRedaction>,
    artifact_digests: Vec<String>,
    recorded_at: KernelTimestamp,
}

impl ModelTranscript {
    /// Build and validate a transcript.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: &str,
        provider_id: &str,
        model_id: &str,
        request_digest: &str,
        response_digest: &str,
        request_body: &str,
        response_body: &str,
        input_tokens: i128,
        output_tokens: i128,
        cost: Decimal,
        redactions: Vec<ModelRedaction>,
        artifact_digests: Vec<String>,
        recorded_at: GatewayTimestamp,
    ) -> Result<Self, ModelError> {
        let Some(recorded_at) = recorded_at.as_aware() else {
            return Err(ModelError::request("recorded_at must include a timezone"));
        };
        if request_body.len() > MAX_TRANSCRIPT_BODY_BYTES {
            return Err(ModelError::transcript(
                "the transcript request body exceeds the bounded size",
            ));
        }
        if response_body.len() > MAX_TRANSCRIPT_BODY_BYTES {
            return Err(ModelError::transcript(
                "the transcript response body exceeds the bounded size",
            ));
        }
        Ok(Self {
            request_id: request_id.to_owned(),
            provider_id: provider_id.to_owned(),
            model_id: model_id.to_owned(),
            request_digest: request_digest.to_owned(),
            response_digest: response_digest.to_owned(),
            request_body: request_body.to_owned(),
            response_body: response_body.to_owned(),
            input_tokens,
            output_tokens,
            cost,
            redactions,
            artifact_digests,
            recorded_at,
        })
    }

    /// The request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// The provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// The request digest.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    /// The response digest.
    #[must_use]
    pub fn response_digest(&self) -> &str {
        &self.response_digest
    }

    /// The stored request body.
    #[must_use]
    pub fn request_body(&self) -> &str {
        &self.request_body
    }

    /// The stored response body.
    #[must_use]
    pub fn response_body(&self) -> &str {
        &self.response_body
    }

    /// The input token counter.
    #[must_use]
    pub fn input_tokens(&self) -> i128 {
        self.input_tokens
    }

    /// The output token counter.
    #[must_use]
    pub fn output_tokens(&self) -> i128 {
        self.output_tokens
    }

    /// The estimated cost.
    #[must_use]
    pub fn cost(&self) -> &Decimal {
        &self.cost
    }

    /// The applied redactions.
    #[must_use]
    pub fn redactions(&self) -> &[ModelRedaction] {
        &self.redactions
    }

    /// The digests of the referenced artifacts.
    #[must_use]
    pub fn artifact_digests(&self) -> &[String] {
        &self.artifact_digests
    }

    /// The recording instant.
    #[must_use]
    pub fn recorded_at(&self) -> KernelTimestamp {
        self.recorded_at
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "artifact_digests",
                CanonicalValue::Array(
                    self.artifact_digests
                        .iter()
                        .map(|digest| CanonicalValue::text(digest.clone()))
                        .collect(),
                ),
            ),
            ("cost", CanonicalValue::text(self.cost.to_python_string())),
            ("input_tokens", CanonicalValue::integer(self.input_tokens)),
            ("model_id", CanonicalValue::text(self.model_id.clone())),
            ("output_tokens", CanonicalValue::integer(self.output_tokens)),
            (
                "provider_id",
                CanonicalValue::text(self.provider_id.clone()),
            ),
            (
                "recorded_at",
                CanonicalValue::text(self.recorded_at.isoformat()),
            ),
            (
                "redactions",
                CanonicalValue::Array(
                    self.redactions
                        .iter()
                        .map(ModelRedaction::to_payload)
                        .collect(),
                ),
            ),
            (
                "request_body",
                CanonicalValue::text(self.request_body.clone()),
            ),
            (
                "request_digest",
                CanonicalValue::text(self.request_digest.clone()),
            ),
            ("request_id", CanonicalValue::text(self.request_id.clone())),
            (
                "response_body",
                CanonicalValue::text(self.response_body.clone()),
            ),
            (
                "response_digest",
                CanonicalValue::text(self.response_digest.clone()),
            ),
        ])
    }
}

/// A recorded egress manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelEgressManifest {
    request_id: String,
    egress_request_id: String,
    provider_id: String,
    model_id: String,
    destination: String,
    request_digest: String,
    state_digest: String,
    action_digest: String,
    capabilities: Vec<String>,
    redactions: Vec<ModelRedaction>,
    artifact_digests: Vec<String>,
    recorded_at: KernelTimestamp,
}

impl ModelEgressManifest {
    /// Build and validate an egress manifest.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: &str,
        egress_request_id: &str,
        provider_id: &str,
        model_id: &str,
        destination: &str,
        request_digest: &str,
        state_digest: &str,
        action_digest: &str,
        capabilities: Vec<String>,
        redactions: Vec<ModelRedaction>,
        artifact_digests: Vec<String>,
        recorded_at: GatewayTimestamp,
    ) -> Result<Self, ModelError> {
        let Some(recorded_at) = recorded_at.as_aware() else {
            return Err(ModelError::request("recorded_at must include a timezone"));
        };
        let request_id = bounded_identifier(request_id, "request_id")?;
        let egress_request_id = bounded_identifier(egress_request_id, "egress_request_id")?;
        let provider_id = bounded_identifier(provider_id, "provider_id")?;
        let model_id = bounded_identifier(model_id, "model_id")?;
        let destination = bounded_text(destination, "destination", MAX_MESSAGE_LENGTH)?;
        for (name, digest) in [
            ("request_digest", request_digest),
            ("state_digest", state_digest),
            ("action_digest", action_digest),
        ] {
            if !is_sha256_hex(digest) {
                return Err(ModelError::request(format!(
                    "{name} must be a sha256 hexadecimal digest"
                )));
            }
        }
        let mut capabilities = capabilities;
        capabilities.sort();
        Ok(Self {
            request_id,
            egress_request_id,
            provider_id,
            model_id,
            destination,
            request_digest: request_digest.to_owned(),
            state_digest: state_digest.to_owned(),
            action_digest: action_digest.to_owned(),
            capabilities,
            redactions,
            artifact_digests,
            recorded_at,
        })
    }

    /// The request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// The egress request identifier.
    #[must_use]
    pub fn egress_request_id(&self) -> &str {
        &self.egress_request_id
    }

    /// The provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// The contacted destination.
    #[must_use]
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// The request digest.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }

    /// The policy state digest.
    #[must_use]
    pub fn state_digest(&self) -> &str {
        &self.state_digest
    }

    /// The policy action digest.
    #[must_use]
    pub fn action_digest(&self) -> &str {
        &self.action_digest
    }

    /// The requested capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// The applied redactions.
    #[must_use]
    pub fn redactions(&self) -> &[ModelRedaction] {
        &self.redactions
    }

    /// The digests of the referenced artifacts.
    #[must_use]
    pub fn artifact_digests(&self) -> &[String] {
        &self.artifact_digests
    }

    /// The recording instant.
    #[must_use]
    pub fn recorded_at(&self) -> KernelTimestamp {
        self.recorded_at
    }

    /// The Python `to_payload()` shape.
    #[must_use]
    pub fn to_payload(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "action_digest",
                CanonicalValue::text(self.action_digest.clone()),
            ),
            (
                "artifact_digests",
                CanonicalValue::Array(
                    self.artifact_digests
                        .iter()
                        .map(|digest| CanonicalValue::text(digest.clone()))
                        .collect(),
                ),
            ),
            (
                "capabilities",
                CanonicalValue::Array(
                    self.capabilities
                        .iter()
                        .map(|capability| CanonicalValue::text(capability.clone()))
                        .collect(),
                ),
            ),
            (
                "destination",
                CanonicalValue::text(self.destination.clone()),
            ),
            (
                "egress_request_id",
                CanonicalValue::text(self.egress_request_id.clone()),
            ),
            ("model_id", CanonicalValue::text(self.model_id.clone())),
            (
                "provider_id",
                CanonicalValue::text(self.provider_id.clone()),
            ),
            (
                "recorded_at",
                CanonicalValue::text(self.recorded_at.isoformat()),
            ),
            (
                "redactions",
                CanonicalValue::Array(
                    self.redactions
                        .iter()
                        .map(ModelRedaction::to_payload)
                        .collect(),
                ),
            ),
            (
                "request_digest",
                CanonicalValue::text(self.request_digest.clone()),
            ),
            ("request_id", CanonicalValue::text(self.request_id.clone())),
            (
                "state_digest",
                CanonicalValue::text(self.state_digest.clone()),
            ),
        ])
    }
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Estimate the number of tokens a text uses.
///
/// The estimator counts Unicode code points, like CPython's `len()`.
#[must_use]
pub fn estimate_tokens(text: &str) -> i128 {
    if text.is_empty() {
        return 0;
    }
    let length = code_point_len(text);
    let per_token = CHARS_PER_TOKEN as i128;
    let rounded = (length as i128 + per_token - 1) / per_token;
    rounded.max(1)
}
