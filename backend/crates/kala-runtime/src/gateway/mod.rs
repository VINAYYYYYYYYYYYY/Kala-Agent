//! The Kala model gateway.
//!
//! This module is the Rust port of `kala_agent.runtime.model_gateway`. It is
//! the provider-neutral LLM connection layer described by ADR-0001 and
//! ADR-0003:
//!
//! * provider request shaping and response validation,
//! * egress policy and allow-remote consent through the policy engine,
//! * budget reservation before the call and settlement after it,
//! * credential redaction of every prompt, artifact and transcript body,
//! * bounded, byte-safe transcripts and egress manifests.
//!
//! The Python module is synchronous and the Rust port is asynchronous,
//! because the accepted HTTP client stack (reqwest over rustls, the same
//! stack `kala-studio` uses) is asynchronous. Every other behaviour — the
//! validation messages, the error precedence, the canonical request bytes,
//! the digests and the redaction results — is reproduced exactly.

pub mod client;
pub mod decimal;
pub mod endpoint;
pub mod error;
#[allow(clippy::module_inception)]
pub mod gateway;
pub mod model;
pub mod redact;
pub mod text;
pub mod value;

pub use client::{
    ClientError, LlmClient, LocalModelClient, OpenAICompatibleClient, ProviderRequest,
    ProviderResponse, ProviderTransport, ReqwestTransport, TransportFailure, provider_int,
    schema_properties, schema_type_names, synthesize_value,
};
pub use decimal::{DecimalArithmeticError, divide, multiply};
pub use endpoint::{
    Destination, LOOPBACK_HOSTS, MAX_ENDPOINT_LENGTH, is_loopback_host, normalize_endpoint,
    normalize_path, normalized_destination, parse_destination,
};
pub use error::{
    EgressDeniedError, GatewayError, ModelCapabilityError, ModelError, ModelProviderError,
    ModelRequestError, ProviderDegradedError, ProviderSubstitutionError, ProviderUnavailableError,
    StructuredOutputError, TranscriptError,
};
pub use gateway::{CallbackError, Clock, EgressManifestCallback, ModelGateway, TranscriptCallback};
pub use model::{
    CHARS_PER_TOKEN, EGRESS_CAPABILITIES, GatewayTimestamp, LOCAL_PROVIDER_ID, MAX_MESSAGE_LENGTH,
    MAX_MESSAGES, MAX_OUTPUT_TOKENS, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, MAX_SEED,
    MAX_TIMEOUT_SECONDS, MAX_TRANSCRIPT_BODY_BYTES, MIN_TIMEOUT_SECONDS, ModelArtifact,
    ModelCapability, ModelEgressManifest, ModelFinishReason, ModelMessage, ModelRedaction,
    ModelRequest, ModelResponse, ModelRole, ModelTranscript, OPENAI_COMPATIBLE_PROVIDER_ID,
    estimate_tokens,
};
pub use redact::{REDACTED, is_secret_name, redact_metadata, redact_secrets, truncate_encoded};
pub use text::{code_point_len, is_python_whitespace, python_repr, python_strip};
pub use value::{
    DecimalInput, MAX_STRUCTURED_OUTPUT_DEPTH, bounded_float, bounded_identifier, bounded_text,
    freeze_json, json_depth, nonnegative_decimal, nonnegative_int, thaw_json,
};
