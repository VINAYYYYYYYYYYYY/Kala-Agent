//! The provider clients.
//!
//! The Python module defines the `LLMClient` protocol and two
//! implementations: the deterministic [`LocalModelClient`] and the opt-in
//! [`OpenAICompatibleClient`]. The Rust port keeps the same shape; the
//! provider call is asynchronous because the accepted HTTP stack (reqwest
//! over rustls, the same stack `kala-studio` uses) is asynchronous.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use kala_budget::Decimal;
use kala_json::CanonicalValue;
use kala_tools::protocol::{canonical_json_dumps, strict_json_loads};

use super::decimal::{DecimalArithmeticError, divide, multiply};
use super::endpoint::{Destination, normalize_endpoint, parse_destination};
use super::error::ModelError;
use super::model::{
    MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, ModelCapability, ModelFinishReason, ModelMessage,
    ModelRequest, ModelResponse, estimate_tokens,
};
use super::text::python_repr;
use super::value::{bounded_float, bounded_identifier, freeze_json, thaw_json};

/// A failure raised by a provider client.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientError {
    /// A model error the gateway forwards unchanged.
    Model(ModelError),
    /// An unexpected failure, reported by its Python exception class name.
    Unexpected(String),
}

impl ClientError {
    /// Build a model error.
    pub fn model(error: ModelError) -> Self {
        Self::Model(error)
    }

    /// Build an unexpected failure with a Python exception class name.
    pub fn unexpected(class_name: impl Into<String>) -> Self {
        Self::Unexpected(class_name.into())
    }

    /// The model error, when the failure is one.
    pub fn as_model(&self) -> Option<&ModelError> {
        match self {
            Self::Model(error) => Some(error),
            Self::Unexpected(_) => None,
        }
    }
}

impl From<ModelError> for ClientError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model(error) => formatter.write_str(error.message()),
            Self::Unexpected(class_name) => formatter.write_str(class_name),
        }
    }
}

impl std::error::Error for ClientError {}

/// The provider-neutral client protocol.
///
/// The Python module declares an `LLMClient` protocol and checks the duck
/// type of the client at construction time; the Rust port enforces the same
/// contract with the type system.
#[async_trait]
pub trait LlmClient: Send + Sync {
    /// The provider identifier.
    fn provider_id(&self) -> &str;

    /// The model identifier.
    fn model_id(&self) -> &str;

    /// Whether the client can leave the machine.
    fn requires_egress(&self) -> bool;

    /// The destination the provider contacts, when it has one.
    ///
    /// The Python implementation reads the `endpoint` attribute of the client
    /// with `getattr`; the Rust port declares it on the protocol with a
    /// default of "no destination".
    fn endpoint(&self) -> Option<&str> {
        None
    }

    /// Whether the provider declares a capability.
    fn supports(&self, capability: ModelCapability) -> bool;

    /// Run one completion.
    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ClientError>;
}

// ---------------------------------------------------------------------------
// The deterministic local client.
// ---------------------------------------------------------------------------

/// The deterministic, offline client.
///
/// It is a test double, not a language model: it answers with the request
/// digest or with a value synthesized from the response schema.
#[derive(Debug)]
pub struct LocalModelClient {
    provider_id: String,
    model_id: String,
    capabilities: Vec<ModelCapability>,
    calls: AtomicU64,
}

impl LocalModelClient {
    /// Build and validate a local client.
    pub fn new(
        model_id: &str,
        provider_id: &str,
        capabilities: Vec<ModelCapability>,
    ) -> Result<Self, ModelError> {
        let provider_id = bounded_identifier(provider_id, "provider_id")?;
        let model_id = bounded_identifier(model_id, "model_id")?;
        let mut capabilities = capabilities;
        capabilities.sort();
        capabilities.dedup();
        Ok(Self {
            provider_id,
            model_id,
            capabilities,
            calls: AtomicU64::new(0),
        })
    }

    /// The number of completions the client answered.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmClient for LocalModelClient {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn requires_egress(&self) -> bool {
        false
    }

    fn supports(&self, capability: ModelCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ClientError> {
        for capability in request.capabilities() {
            if !self.supports(*capability) {
                return Err(ClientError::model(ModelError::capability(format!(
                    "provider {} does not declare {}",
                    self.provider_id,
                    capability.as_str()
                ))));
            }
        }
        let prompt_tokens: i128 = request
            .messages()
            .iter()
            .map(|message| estimate_tokens(message.content()))
            .sum();
        let content = match request.response_schema() {
            None => {
                let digest = request.request_digest()?;
                canonical_json_dumps(
                    &CanonicalValue::object([("request_digest", CanonicalValue::text(digest))]),
                    super::model::MAX_RESPONSE_BYTES,
                )
                .map_err(|error| ClientError::model(ModelError::request(error.to_string())))?
            }
            Some(schema) => {
                let synthesized = synthesize_value(schema, 0)?;
                if !matches!(synthesized, CanonicalValue::Object(_)) {
                    return Err(ClientError::model(ModelError::provider(
                        "the response schema does not describe an object",
                    )));
                }
                canonical_json_dumps(&thaw_json(&synthesized), super::model::MAX_RESPONSE_BYTES)
                    .map_err(|error| ClientError::model(ModelError::request(error.to_string())))?
            }
        };
        self.calls.fetch_add(1, Ordering::SeqCst);
        let digest = request.request_digest()?;
        let provider_request_id = format!("local-{}", &digest[..16.min(digest.len())]);
        Ok(ModelResponse::new(
            request.request_id(),
            &self.provider_id,
            &self.model_id,
            &content,
        )?
        .with_tokens(prompt_tokens, estimate_tokens(&content))?
        .with_cost(&super::value::DecimalInput::Decimal(Decimal::zero()))?
        .with_finish_reason(ModelFinishReason::Stop)
        .with_provider_request_id(Some(&provider_request_id))?)
    }
}

impl std::fmt::Display for LocalModelClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "LocalModelClient(provider_id={}, model_id={})",
            python_repr(&self.provider_id),
            python_repr(&self.model_id)
        )
    }
}

/// The declared type names of a schema.
///
/// The Python helper is `_schema_type_names`; a schema without a declared
/// type describes an object.
#[must_use]
pub fn schema_type_names(schema: &CanonicalValue) -> Vec<String> {
    let CanonicalValue::Object(entries) = schema else {
        return vec!["object".to_owned()];
    };
    match entries.get(&kala_json::JsonText::new("type")) {
        None => vec!["object".to_owned()],
        Some(CanonicalValue::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_text().map(|text| text.to_string()))
            .collect(),
        Some(CanonicalValue::Text(text)) => vec![text.to_string()],
        Some(_) => Vec::new(),
    }
}

/// The property map of a schema.
///
/// The Python helper is `_schema_properties`; a schema without a property map
/// has no properties.
#[must_use]
pub fn schema_properties(schema: &CanonicalValue) -> CanonicalValue {
    let CanonicalValue::Object(entries) = schema else {
        return CanonicalValue::object(std::iter::empty::<(&str, CanonicalValue)>());
    };
    match entries.get(&kala_json::JsonText::new("properties")) {
        Some(value @ CanonicalValue::Object(_)) => value.clone(),
        _ => CanonicalValue::object(std::iter::empty::<(&str, CanonicalValue)>()),
    }
}

/// Synthesize a value that satisfies a response schema.
pub fn synthesize_value(
    schema: &CanonicalValue,
    depth: usize,
) -> Result<CanonicalValue, ModelError> {
    if depth > 16 {
        return Err(ModelError::provider(
            "the response schema nesting exceeds the synthesis bound",
        ));
    }
    let CanonicalValue::Object(entries) = schema else {
        return Ok(CanonicalValue::Null);
    };
    if let Some(value) = entries.get(&kala_json::JsonText::new("const")) {
        return freeze_json(value);
    }
    if let Some(CanonicalValue::Array(options)) = entries.get(&kala_json::JsonText::new("enum")) {
        if let Some(first) = options.first() {
            return freeze_json(first);
        }
    }
    if let Some(value) = entries.get(&kala_json::JsonText::new("default")) {
        if let Ok(frozen) = freeze_json(value) {
            return Ok(frozen);
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(CanonicalValue::Array(branches)) =
            entries.get(&kala_json::JsonText::new(keyword))
        {
            if let Some(first) = branches.first() {
                if matches!(first, CanonicalValue::Object(_)) {
                    return synthesize_value(first, depth + 1);
                }
            }
        }
    }
    let names = schema_type_names(schema);
    let properties = schema_properties(schema);
    if names.iter().any(|name| name == "object") {
        let CanonicalValue::Object(property_entries) = &properties else {
            return Ok(CanonicalValue::object(std::iter::empty::<(
                &str,
                CanonicalValue,
            )>()));
        };
        let wanted: Vec<String> = match entries.get(&kala_json::JsonText::new("required")) {
            Some(CanonicalValue::Array(required)) => required
                .iter()
                .filter_map(|value| value.as_text().map(|text| text.to_string()))
                .collect(),
            _ => property_entries.keys().map(|key| key.to_string()).collect(),
        };
        let mut synthesized: Vec<(String, CanonicalValue)> = Vec::new();
        for key in wanted {
            let subschema = property_entries.get(&kala_json::JsonText::new(&key));
            let value = match subschema {
                Some(value @ CanonicalValue::Object(_)) => synthesize_value(value, depth + 1)?,
                _ => CanonicalValue::Null,
            };
            synthesized.push((key, value));
        }
        return Ok(CanonicalValue::object(synthesized));
    }
    if names.iter().any(|name| name == "array") {
        let count = match entries.get(&kala_json::JsonText::new("minItems")) {
            Some(CanonicalValue::Integer(value)) => {
                i64::try_from(value.clone()).unwrap_or(i64::MAX).max(0)
            }
            _ => 1,
        };
        let items = entries.get(&kala_json::JsonText::new("items"));
        let Some(items @ CanonicalValue::Object(_)) = items else {
            return freeze_json(&CanonicalValue::Array(Vec::new()));
        };
        let mut collected = Vec::new();
        for _ in 0..count.min(4) {
            collected.push(synthesize_value(items, depth + 1)?);
        }
        return freeze_json(&CanonicalValue::Array(collected));
    }
    if names.iter().any(|name| name == "string") {
        return Ok(CanonicalValue::text("local-offline-deterministic"));
    }
    if names.iter().any(|name| name == "integer") {
        return Ok(CanonicalValue::integer(0i64));
    }
    if names.iter().any(|name| name == "number") {
        return Ok(CanonicalValue::Float(0.0));
    }
    if names.iter().any(|name| name == "boolean") {
        return Ok(CanonicalValue::Bool(false));
    }
    Ok(CanonicalValue::Null)
}

// ---------------------------------------------------------------------------
// The OpenAI-compatible client.
// ---------------------------------------------------------------------------

/// One outbound provider request.
///
/// The `Debug` implementation never prints the credential: the
/// `authorization` header is replaced with `<redacted>`.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderRequest {
    /// The destination URL.
    pub url: String,
    /// The request headers, in the order they are sent.
    pub headers: Vec<(String, String)>,
    /// The request body.
    pub body: Vec<u8>,
    /// The total request timeout.
    pub timeout: Duration,
    /// The largest response body the caller accepts.
    pub max_response_bytes: usize,
}

impl std::fmt::Debug for ProviderRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization") {
                    (name.as_str(), "<redacted>")
                } else {
                    (name.as_str(), value.as_str())
                }
            })
            .collect();
        formatter
            .debug_struct("ProviderRequest")
            .field("url", &self.url)
            .field("headers", &headers)
            .field("body_bytes", &self.body.len())
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

/// One provider response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response body.
    pub body: Vec<u8>,
}

/// A transport failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransportFailure {
    /// The connection timed out.
    ConnectTimeout,
    /// The response did not arrive in time.
    ReadTimeout,
    /// The connection could not be established.
    Connect(String),
    /// The response body exceeded the caller's bound.
    TooLarge,
    /// Any other transport failure.
    Other(String),
}

impl TransportFailure {
    /// The name of the `httpx` exception class the Python implementation
    /// would have raised.
    #[must_use]
    pub fn httpx_class_name(&self) -> &'static str {
        match self {
            Self::ConnectTimeout => "ConnectTimeout",
            Self::ReadTimeout => "ReadTimeout",
            Self::Connect(_) => "ConnectError",
            Self::TooLarge => "ReadError",
            Self::Other(_) => "TransportError",
        }
    }
}

/// The seam the provider call goes through.
///
/// The Python module injects an `httpx` transport for its tests; the Rust
/// port injects a [`ProviderTransport`] instead.
#[async_trait]
pub trait ProviderTransport: Send + Sync {
    /// Send one provider request.
    async fn post(&self, request: ProviderRequest) -> Result<ProviderResponse, TransportFailure>;
}

/// The production transport: reqwest over rustls with the platform trust
/// store, no redirects and no environment proxy, exactly like the accepted
/// `kala-studio` client.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Build the production transport.
    ///
    /// # Errors
    ///
    /// Returns a message when the HTTP client cannot be initialised.
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| "the provider HTTPS client could not be initialised".to_owned())?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ProviderTransport for ReqwestTransport {
    async fn post(&self, request: ProviderRequest) -> Result<ProviderResponse, TransportFailure> {
        let mut builder = self
            .client
            .post(&request.url)
            .timeout(request.timeout)
            .body(request.body);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        let mut response = builder.send().await.map_err(map_reqwest_error)?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > request.max_response_bytes {
                        return Err(TransportFailure::TooLarge);
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => return Err(map_reqwest_error(error)),
            }
        }
        Ok(ProviderResponse { status, body })
    }
}

fn map_reqwest_error(error: reqwest::Error) -> TransportFailure {
    if error.is_timeout() {
        return if error.is_connect() {
            TransportFailure::ConnectTimeout
        } else {
            TransportFailure::ReadTimeout
        };
    }
    if error.is_connect() {
        return TransportFailure::Connect("the connection could not be established".to_owned());
    }
    if error.is_body() || error.is_decode() {
        return TransportFailure::Other("the response could not be read".to_owned());
    }
    TransportFailure::Other("the request could not be sent".to_owned())
}

/// The OpenAI-compatible provider client.
pub struct OpenAICompatibleClient {
    provider_id: String,
    model_id: String,
    endpoint: String,
    allowed: Vec<String>,
    api_key: Option<String>,
    consent: bool,
    timeout_seconds: f64,
    max_response_bytes: usize,
    input_cost: Decimal,
    output_cost: Decimal,
    transport: Arc<dyn ProviderTransport>,
}

impl OpenAICompatibleClient {
    /// Build and validate the client.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: &str,
        model_id: &str,
        allowed_endpoints: Vec<String>,
        api_key: Option<String>,
        consent: bool,
        provider_id: &str,
        timeout_seconds: f64,
        max_response_bytes: usize,
        transport: Arc<dyn ProviderTransport>,
        input_cost: Decimal,
        output_cost: Decimal,
    ) -> Result<Self, ModelError> {
        let model_id = bounded_identifier(model_id, "model_id")?;
        let provider_id = bounded_identifier(provider_id, "provider_id")?;
        let mut allowed = Vec::with_capacity(allowed_endpoints.len());
        for item in &allowed_endpoints {
            allowed.push(super::endpoint::normalized_destination(item)?);
        }
        allowed.sort();
        allowed.dedup();
        let endpoint = normalize_endpoint(endpoint, &allowed, consent)?;
        let timeout_seconds = bounded_float(
            timeout_seconds,
            "timeout_seconds",
            super::model::MIN_TIMEOUT_SECONDS,
            super::model::MAX_TIMEOUT_SECONDS,
        )?;
        if max_response_bytes == 0 || max_response_bytes > MAX_RESPONSE_BYTES {
            return Err(ModelError::request("max_response_bytes is out of range"));
        }
        if let Some(api_key) = &api_key {
            if api_key.trim().is_empty() || api_key.chars().count() > 1_024 {
                return Err(ModelError::request(
                    "api_key must be a bounded nonempty string",
                ));
            }
        }
        Ok(Self {
            provider_id,
            model_id,
            endpoint,
            allowed,
            api_key,
            consent,
            timeout_seconds,
            max_response_bytes,
            input_cost,
            output_cost,
            transport,
        })
    }

    /// The normalized destination.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The normalized allow-list.
    #[must_use]
    pub fn allowed_endpoints(&self) -> &[String] {
        &self.allowed
    }

    /// The request headers.
    ///
    /// The credential is only ever placed in the `authorization` header and is
    /// never logged or echoed.
    #[must_use]
    pub fn headers(&self) -> Vec<(String, String)> {
        let mut headers = vec![("content-type".to_owned(), "application/json".to_owned())];
        if let Some(api_key) = &self.api_key {
            headers.push(("authorization".to_owned(), format!("Bearer {api_key}")));
        }
        headers
    }

    /// The provider request body.
    pub fn payload(&self, request: &ModelRequest) -> Result<Vec<u8>, ModelError> {
        let messages: Vec<CanonicalValue> = request
            .messages()
            .iter()
            .map(ModelMessage::to_payload)
            .collect();
        let mut entries: Vec<(&str, CanonicalValue)> = vec![
            (
                "max_tokens",
                CanonicalValue::integer(request.max_output_tokens()),
            ),
            ("messages", CanonicalValue::Array(messages)),
            ("model", CanonicalValue::text(self.model_id.clone())),
        ];
        if let Some(seed) = request.seed() {
            entries.push(("seed", CanonicalValue::integer(seed)));
        }
        if let Some(schema) = request.response_schema() {
            if !self.supports(ModelCapability::StructuredOutput) {
                return Err(ModelError::capability(
                    "the provider does not declare structured output",
                ));
            }
            entries.push((
                "response_format",
                CanonicalValue::object([
                    (
                        "json_schema",
                        CanonicalValue::object([
                            ("name", CanonicalValue::text("kala_response")),
                            ("schema", thaw_json(schema)),
                            ("strict", CanonicalValue::Bool(true)),
                        ]),
                    ),
                    ("type", CanonicalValue::text("json_schema")),
                ]),
            ));
        }
        if !request.artifacts().is_empty() {
            return Err(ModelError::egress_denied(
                "artifact payloads are never sent to a remote provider",
            ));
        }
        let payload = CanonicalValue::object(entries);
        let encoded = canonical_json_dumps(&payload, MAX_REQUEST_BYTES)
            .map_err(|error| ModelError::request(error.to_string()))?;
        Ok(encoded.into_bytes())
    }

    /// Check the destination against the consent and the allow-list.
    pub fn authorize_destination(&self) -> Result<(), ModelError> {
        let destination: Destination = parse_destination(&self.endpoint)?;
        if !self.consent {
            return Err(ModelError::egress_denied(
                "outbound provider calls require explicit egress consent",
            ));
        }
        if !destination.is_loopback() && !destination.is_tls() {
            return Err(ModelError::egress_denied(
                "plaintext provider endpoints are limited to loopback hosts",
            ));
        }
        if !self.allowed.iter().any(|item| item == &self.endpoint) {
            return Err(ModelError::egress_denied(
                "the provider endpoint is not in the egress allowlist",
            ));
        }
        Ok(())
    }

    /// Parse a provider answer.
    pub fn parse_response(
        &self,
        request: &ModelRequest,
        payload: &[u8],
    ) -> Result<ModelResponse, ModelError> {
        let decoded = strict_json_loads(payload, self.max_response_bytes)
            .map_err(|_| ModelError::provider("the provider response is not strict JSON"))?;
        let CanonicalValue::Object(decoded) = &decoded else {
            return Err(ModelError::provider(
                "the provider response must be a JSON object",
            ));
        };
        let choices = match decoded.get(&kala_json::JsonText::new("choices")) {
            Some(CanonicalValue::Array(choices)) if !choices.is_empty() => choices,
            _ => {
                return Err(ModelError::provider(
                    "the provider response contains no choices",
                ));
            }
        };
        let Some(CanonicalValue::Object(first)) = choices.first() else {
            return Err(ModelError::provider("the provider choice is malformed"));
        };
        let Some(CanonicalValue::Object(message)) = first.get(&kala_json::JsonText::new("message"))
        else {
            return Err(ModelError::provider("the provider choice has no message"));
        };
        let content = match message.get(&kala_json::JsonText::new("content")) {
            Some(CanonicalValue::Text(content)) if !content.to_string_lossy().trim().is_empty() => {
                content.to_string_lossy()
            }
            _ => {
                return Err(ModelError::provider(
                    "the provider returned no textual content",
                ));
            }
        };
        let mut prompt_tokens = 0i128;
        let mut completion_tokens = 0i128;
        if let Some(CanonicalValue::Object(usage)) = decoded.get(&kala_json::JsonText::new("usage"))
        {
            prompt_tokens = provider_int(usage.get(&kala_json::JsonText::new("prompt_tokens")));
            completion_tokens =
                provider_int(usage.get(&kala_json::JsonText::new("completion_tokens")));
        }
        let resolved_finish = match first.get(&kala_json::JsonText::new("finish_reason")) {
            Some(CanonicalValue::Text(reason)) => {
                ModelFinishReason::parse(&reason.to_string_lossy())
                    .unwrap_or(ModelFinishReason::Stop)
            }
            _ => ModelFinishReason::Stop,
        };
        let provider_request_id = match decoded.get(&kala_json::JsonText::new("id")) {
            Some(CanonicalValue::Text(value)) => Some(value.to_string()),
            _ => None,
        };
        let model_id = match decoded.get(&kala_json::JsonText::new("model")) {
            Some(value) => python_json_text(value),
            None => self.model_id.clone(),
        };
        let cost = self
            .cost(prompt_tokens, completion_tokens)
            .map_err(|error| ModelError::provider(error.python_class_name()))?;
        ModelResponse::new(request.request_id(), &self.provider_id, &model_id, &content)?
            .with_tokens(prompt_tokens, completion_tokens)?
            .with_cost(&super::value::DecimalInput::Decimal(cost))?
            .with_finish_reason(resolved_finish)
            .with_provider_request_id(provider_request_id.as_deref())
    }

    /// The estimated cost of an answer.
    pub fn cost(
        &self,
        input_tokens: i128,
        output_tokens: i128,
    ) -> Result<Decimal, DecimalArithmeticError> {
        let million = Decimal::parse("1000000").expect("a valid decimal");
        let input = multiply(
            &Decimal::parse(&input_tokens.to_string()).expect("an integer decimal"),
            &self.input_cost,
        )?;
        let output = multiply(
            &Decimal::parse(&output_tokens.to_string()).expect("an integer decimal"),
            &self.output_cost,
        )?;
        let total = input
            .add(&output)
            .map_err(|_| DecimalArithmeticError::InvalidOperation)?;
        divide(&total, &million)
    }
}

#[async_trait]
impl LlmClient for OpenAICompatibleClient {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn requires_egress(&self) -> bool {
        true
    }

    fn endpoint(&self) -> Option<&str> {
        Some(&self.endpoint)
    }

    fn supports(&self, capability: ModelCapability) -> bool {
        matches!(
            capability,
            ModelCapability::StructuredOutput | ModelCapability::Seed
        )
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ClientError> {
        self.authorize_destination()?;
        for capability in request.capabilities() {
            if !self.supports(*capability) {
                return Err(ClientError::model(ModelError::capability(format!(
                    "provider {} does not declare {}",
                    self.provider_id,
                    capability.as_str()
                ))));
            }
        }
        let body = self.payload(request)?;
        let timeout = Duration::from_secs_f64(request.timeout_seconds().min(self.timeout_seconds));
        let transport_request = ProviderRequest {
            url: self.endpoint.clone(),
            headers: self.headers(),
            body,
            timeout,
            max_response_bytes: self.max_response_bytes,
        };
        let response = match self.transport.post(transport_request).await {
            Ok(response) => response,
            // The Python implementation raises the bounded-size error while
            // it streams the body, so it is a provider error and not an
            // unreachable provider.
            Err(TransportFailure::TooLarge) => {
                return Err(ClientError::model(ModelError::provider(
                    "the provider response exceeds the bounded size",
                )));
            }
            Err(failure) => {
                return Err(ClientError::model(ModelError::unavailable(format!(
                    "provider {} is unreachable: {}",
                    self.provider_id,
                    failure.httpx_class_name()
                ))));
            }
        };
        if response.status != 200 {
            return Err(ClientError::model(ModelError::unavailable(format!(
                "provider {} returned status {}",
                self.provider_id, response.status
            ))));
        }
        self.parse_response(request, &response.body)
            .map_err(ClientError::model)
    }
}

impl std::fmt::Display for OpenAICompatibleClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "OpenAICompatibleClient(provider_id={}, model_id={}, endpoint={}, api_key={}, consent={})",
            python_repr(&self.provider_id),
            python_repr(&self.model_id),
            python_repr(&self.endpoint),
            match &self.api_key {
                Some(_) => python_repr("<redacted>"),
                None => "None".to_owned(),
            },
            if self.consent { "True" } else { "False" },
        )
    }
}

/// Coerce a provider usage counter into a nonnegative integer.
#[must_use]
pub fn provider_int(value: Option<&CanonicalValue>) -> i128 {
    match value {
        Some(CanonicalValue::Integer(value)) => {
            i128::try_from(value.clone()).unwrap_or(i128::MAX).max(0)
        }
        Some(CanonicalValue::Float(value)) => {
            if !value.is_finite() {
                0
            } else {
                (value.trunc() as i128).max(0)
            }
        }
        _ => 0,
    }
}

/// The Python `str()` of a JSON value.
fn python_json_text(value: &CanonicalValue) -> String {
    match value {
        CanonicalValue::Text(text) => text.to_string_lossy(),
        CanonicalValue::Integer(value) => value.to_string(),
        CanonicalValue::Bool(value) => {
            if *value {
                "True".to_owned()
            } else {
                "False".to_owned()
            }
        }
        CanonicalValue::Null => "None".to_owned(),
        CanonicalValue::Float(value) => kala_json::format_float(*value).unwrap_or_default(),
        other => canonical_json_dumps(other, MAX_RESPONSE_BYTES).unwrap_or_default(),
    }
}
