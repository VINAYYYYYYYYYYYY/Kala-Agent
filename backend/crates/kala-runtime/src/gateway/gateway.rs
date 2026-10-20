//! The model gateway.
//!
//! The gateway is the single choke point every model call goes through
//! (ADR-0001, D2). It sanitizes the request, asks the policy engine for
//! permission to leave the machine, reserves budget, dispatches the call,
//! verifies the answer's identity, validates a structured answer and records
//! a bounded transcript.

use std::sync::{Arc, Mutex, MutexGuard};

use kala_budget::{BudgetLedger, BudgetReservation, BudgetUsage, Counter, Decimal, UsageDelta};
use kala_contracts::{ApprovalKind, SideEffectLevel, Text};
use kala_json::CanonicalValue;
use kala_kernel::policy::{PolicyDecision, PolicyEngine, PolicyRequest};
use kala_tools::protocol::canonical_json_digest;
use num_bigint::BigUint;

use super::client::{ClientError, LlmClient};
use super::error::{GatewayError, ModelError};
use super::model::{
    EGRESS_CAPABILITIES, GatewayTimestamp, MAX_TRANSCRIPT_BODY_BYTES, ModelEgressManifest,
    ModelMessage, ModelRedaction, ModelRequest, ModelResponse, ModelTranscript, estimate_tokens,
};
use super::redact::{redact_metadata, redact_secrets, truncate_encoded};

/// A callback failure, reported by the Python exception class name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallbackError {
    class_name: String,
}

impl CallbackError {
    /// Build a callback failure with a Python exception class name.
    pub fn new(class_name: impl Into<String>) -> Self {
        Self {
            class_name: class_name.into(),
        }
    }

    /// The Python exception class name.
    #[must_use]
    pub fn class_name(&self) -> &str {
        &self.class_name
    }
}

impl std::fmt::Display for CallbackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.class_name)
    }
}

impl std::error::Error for CallbackError {}

/// The callback that receives every transcript.
pub type TranscriptCallback =
    Arc<dyn Fn(&ModelTranscript) -> Result<(), CallbackError> + Send + Sync + 'static>;

/// The callback that receives every egress manifest.
pub type EgressManifestCallback =
    Arc<dyn Fn(&ModelEgressManifest) -> Result<(), CallbackError> + Send + Sync + 'static>;

/// The clock the gateway uses to stamp transcripts and manifests.
pub type Clock = Arc<dyn Fn() -> GatewayTimestamp + Send + Sync + 'static>;

#[derive(Default)]
struct GatewayState {
    transcripts: Vec<ModelTranscript>,
    manifests: Vec<ModelEgressManifest>,
}

/// The provider-neutral model gateway.
pub struct ModelGateway {
    client: Arc<dyn LlmClient>,
    ledger: Option<Arc<BudgetLedger>>,
    transcript: Option<TranscriptCallback>,
    policy: Option<Arc<PolicyEngine>>,
    egress_manifest: Option<EgressManifestCallback>,
    clock: Option<Clock>,
    state: Mutex<GatewayState>,
}

impl ModelGateway {
    /// Build a gateway around a provider client.
    #[must_use]
    pub fn new(client: Arc<dyn LlmClient>) -> Self {
        Self {
            client,
            ledger: None,
            transcript: None,
            policy: None,
            egress_manifest: None,
            clock: None,
            state: Mutex::new(GatewayState::default()),
        }
    }

    /// Attach a budget ledger.
    #[must_use]
    pub fn with_ledger(mut self, ledger: Arc<BudgetLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Attach a transcript callback.
    #[must_use]
    pub fn with_transcript(mut self, callback: TranscriptCallback) -> Self {
        self.transcript = Some(callback);
        self
    }

    /// Attach a policy engine.
    #[must_use]
    pub fn with_policy(mut self, policy: Arc<PolicyEngine>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Attach an egress manifest callback.
    #[must_use]
    pub fn with_egress_manifest(mut self, callback: EgressManifestCallback) -> Self {
        self.egress_manifest = Some(callback);
        self
    }

    /// Attach a clock.
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// The provider client.
    #[must_use]
    pub fn client(&self) -> &dyn LlmClient {
        self.client.as_ref()
    }

    /// The attached policy engine.
    #[must_use]
    pub fn policy(&self) -> Option<&PolicyEngine> {
        self.policy.as_deref()
    }

    /// The provider identifier.
    #[must_use]
    pub fn provider_id(&self) -> &str {
        self.client.provider_id()
    }

    /// The model identifier.
    #[must_use]
    pub fn model_id(&self) -> &str {
        self.client.model_id()
    }

    /// Whether the provider can leave the machine.
    #[must_use]
    pub fn requires_egress(&self) -> bool {
        self.client.requires_egress()
    }

    /// The contacted destination.
    #[must_use]
    pub fn destination(&self) -> String {
        match self.client.endpoint() {
            Some(endpoint) if !endpoint.is_empty() => endpoint.to_owned(),
            _ => format!("provider:{}", self.provider_id()),
        }
    }

    /// The recorded transcripts.
    #[must_use]
    pub fn transcripts(&self) -> Vec<ModelTranscript> {
        self.lock().transcripts.clone()
    }

    /// The recorded egress manifests.
    #[must_use]
    pub fn manifests(&self) -> Vec<ModelEgressManifest> {
        self.lock().manifests.clone()
    }

    fn lock(&self) -> MutexGuard<'_, GatewayState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Run one model call.
    ///
    /// The order of the steps is the security-relevant part: the request is
    /// sanitized first, then the egress policy decides, then the provider
    /// capabilities are checked, then the budget is reserved, and only then
    /// is the provider called.
    pub async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, GatewayError> {
        let (sanitized, redactions) = self.sanitize(request)?;
        let manifest = self.authorize_egress(&sanitized, &redactions)?;
        self.check_capabilities(&sanitized)?;
        let reservation = self.reserve(&sanitized)?;
        match self
            .complete_reserved(&sanitized, &redactions, manifest, reservation.clone())
            .await
        {
            Ok(response) => Ok(response),
            Err(error) => {
                self.release_if_held(reservation);
                Err(error)
            }
        }
    }

    async fn complete_reserved(
        &self,
        sanitized: &ModelRequest,
        redactions: &[ModelRedaction],
        manifest: Option<ModelEgressManifest>,
        reservation: Option<BudgetReservation>,
    ) -> Result<ModelResponse, GatewayError> {
        if let Some(manifest) = manifest {
            self.record_manifest(manifest)?;
        }
        let started = self.now()?;
        let raw = match self.client.complete(sanitized).await {
            Ok(response) => response,
            Err(ClientError::Model(ModelError::Unavailable(message))) => {
                return Err(GatewayError::Model(ModelError::degraded(
                    self.provider_id(),
                    self.model_id(),
                    self.safe_reason(&message),
                )));
            }
            Err(ClientError::Model(error)) => return Err(GatewayError::Model(error)),
            Err(ClientError::Unexpected(class_name)) => {
                return Err(GatewayError::Model(ModelError::degraded(
                    self.provider_id(),
                    self.model_id(),
                    class_name,
                )));
            }
        };
        let elapsed = elapsed_seconds(self.now()?, started).max(0.0);
        let response = self.verify_identity(raw)?;
        let response = match sanitized.response_schema() {
            Some(schema) => self.validate_structured(response, schema)?,
            None => response,
        };
        self.record(
            sanitized,
            &response,
            redactions,
            reservation.as_ref(),
            elapsed,
        )?;
        Ok(response)
    }

    fn check_capabilities(&self, request: &ModelRequest) -> Result<(), GatewayError> {
        for capability in request.capabilities() {
            if !self.client.supports(*capability) {
                return Err(GatewayError::Model(ModelError::capability(format!(
                    "provider {} does not declare {}",
                    self.provider_id(),
                    capability.as_str()
                ))));
            }
        }
        Ok(())
    }

    /// The policy request the gateway would use for a request.
    pub fn egress_policy_request(
        &self,
        request: &ModelRequest,
    ) -> Result<PolicyRequest, GatewayError> {
        let (sanitized, redactions) = self.sanitize(request)?;
        self.egress_policy_request_for(&sanitized, &redactions)
    }

    fn egress_policy_request_for(
        &self,
        request: &ModelRequest,
        redactions: &[ModelRedaction],
    ) -> Result<PolicyRequest, GatewayError> {
        PolicyRequest::from_parts(
            Text::new(&egress_request_id(request.request_id())),
            Text::new(&self.egress_state_digest_for(request, redactions)?),
            Text::new(&self.egress_action_digest_for(request, redactions)?),
            EGRESS_CAPABILITIES
                .iter()
                .map(|capability| Text::new(capability)),
            SideEffectLevel::Egress,
            false,
            true,
            None,
            Some(ApprovalKind::Egress),
        )
        .map_err(GatewayError::Policy)
    }

    /// The state digest of an egress request.
    pub fn egress_state_digest(&self, request: &ModelRequest) -> Result<String, GatewayError> {
        let (sanitized, redactions) = self.sanitize(request)?;
        self.egress_state_digest_for(&sanitized, &redactions)
    }

    /// The action digest of an egress request.
    pub fn egress_action_digest(&self, request: &ModelRequest) -> Result<String, GatewayError> {
        let (sanitized, redactions) = self.sanitize(request)?;
        self.egress_action_digest_for(&sanitized, &redactions)
    }

    fn egress_state_digest_for(
        &self,
        request: &ModelRequest,
        redactions: &[ModelRedaction],
    ) -> Result<String, GatewayError> {
        let digest = canonical_json_digest(&CanonicalValue::object([
            ("destination", CanonicalValue::text(self.destination())),
            ("model_id", CanonicalValue::text(self.model_id())),
            ("provider", CanonicalValue::text(self.provider_id())),
            (
                "redactions",
                CanonicalValue::text(self.redaction_digest(redactions)?),
            ),
            (
                "request",
                CanonicalValue::text(egress_request_id(request.request_id())),
            ),
        ]))
        .map_err(|error| GatewayError::Model(ModelError::request(error.to_string())))?;
        Ok(digest)
    }

    fn egress_action_digest_for(
        &self,
        request: &ModelRequest,
        redactions: &[ModelRedaction],
    ) -> Result<String, GatewayError> {
        let request_digest = request.request_digest()?;
        let digest = canonical_json_digest(&CanonicalValue::object([
            (
                "capabilities",
                CanonicalValue::Array(
                    EGRESS_CAPABILITIES
                        .iter()
                        .map(|capability| CanonicalValue::text(*capability))
                        .collect(),
                ),
            ),
            ("destination", CanonicalValue::text(self.destination())),
            ("model_id", CanonicalValue::text(self.model_id())),
            ("provider", CanonicalValue::text(self.provider_id())),
            (
                "redactions",
                CanonicalValue::text(self.redaction_digest(redactions)?),
            ),
            ("request_digest", CanonicalValue::text(request_digest)),
        ]))
        .map_err(|error| GatewayError::Model(ModelError::request(error.to_string())))?;
        Ok(digest)
    }

    fn redaction_digest(&self, redactions: &[ModelRedaction]) -> Result<String, GatewayError> {
        // The Python implementation sorts the redaction records by their
        // canonical JSON text before hashing them.
        let mut encoded: Vec<(String, CanonicalValue)> = Vec::with_capacity(redactions.len());
        for redaction in redactions {
            let value = CanonicalValue::object([
                ("kind", CanonicalValue::text(redaction.kind())),
                ("reason", CanonicalValue::text(redaction.reason())),
                ("subject", CanonicalValue::text(redaction.subject())),
            ]);
            let text =
                kala_tools::protocol::canonical_json_dumps(&value, super::model::MAX_REQUEST_BYTES)
                    .map_err(|error| GatewayError::Model(ModelError::request(error.to_string())))?;
            encoded.push((text, value));
        }
        encoded.sort_by(|left, right| left.0.cmp(&right.0));
        let entries: Vec<CanonicalValue> = encoded.into_iter().map(|(_, value)| value).collect();
        canonical_json_digest(&CanonicalValue::Array(entries))
            .map_err(|error| GatewayError::Model(ModelError::request(error.to_string())))
    }

    fn authorize_egress(
        &self,
        request: &ModelRequest,
        redactions: &[ModelRedaction],
    ) -> Result<Option<ModelEgressManifest>, GatewayError> {
        if !self.requires_egress() {
            return Ok(None);
        }
        let Some(policy) = &self.policy else {
            return Err(GatewayError::Model(ModelError::egress_denied(format!(
                "provider {} requires egress and no policy engine is attached",
                self.provider_id()
            ))));
        };
        let policy_request = self.egress_policy_request_for(request, redactions)?;
        let decision: PolicyDecision = policy.decide(&policy_request, None)?;
        if !decision.allowed() {
            return Err(GatewayError::Model(ModelError::egress_denied(format!(
                "egress policy denied the provider call: {}",
                decision.reason().as_str()
            ))));
        }
        let manifest = ModelEgressManifest::new(
            request.request_id(),
            &egress_request_id(request.request_id()),
            self.provider_id(),
            self.model_id(),
            &self.destination(),
            &request.request_digest()?,
            &self.egress_state_digest_for(request, redactions)?,
            &self.egress_action_digest_for(request, redactions)?,
            EGRESS_CAPABILITIES
                .iter()
                .map(|capability| (*capability).to_owned())
                .collect(),
            redactions.to_vec(),
            request
                .artifacts()
                .iter()
                .map(|artifact| artifact.digest().to_owned())
                .collect(),
            self.now()?,
        )?;
        Ok(Some(manifest))
    }

    fn record_manifest(&self, manifest: ModelEgressManifest) -> Result<(), GatewayError> {
        let Some(callback) = &self.egress_manifest else {
            return Err(GatewayError::Model(ModelError::egress_denied(format!(
                "provider {} requires egress and no egress manifest is attached",
                self.provider_id()
            ))));
        };
        self.lock().manifests.push(manifest.clone());
        if let Err(error) = callback(&manifest) {
            return Err(GatewayError::Model(ModelError::egress_denied(format!(
                "the egress manifest could not be recorded: {}",
                error.class_name()
            ))));
        }
        Ok(())
    }

    fn sanitize(
        &self,
        request: &ModelRequest,
    ) -> Result<(ModelRequest, Vec<ModelRedaction>), GatewayError> {
        let mut redactions: Vec<ModelRedaction> = Vec::new();
        let mut artifacts = Vec::with_capacity(request.artifacts().len());
        for artifact in request.artifacts() {
            let (sanitized, artifact_redactions) = sanitize_artifact(artifact)?;
            artifacts.push(sanitized);
            redactions.extend(artifact_redactions);
        }
        let mut messages: Vec<ModelMessage> = Vec::with_capacity(request.messages().len());
        for message in request.messages() {
            let (content, findings) = redact_secrets(message.content());
            if findings > 0 {
                redactions.push(ModelRedaction::new(
                    "secret",
                    message.role().as_str(),
                    "credential-shaped text was removed from the transcript",
                ));
            }
            messages.push(ModelMessage::new(message.role(), &content)?);
        }
        let sanitized = request
            .clone()
            .with_messages(messages)?
            .with_artifacts(artifacts)?;
        Ok((sanitized, redactions))
    }

    fn verify_identity(&self, response: ModelResponse) -> Result<ModelResponse, GatewayError> {
        if response.provider_id() != self.provider_id() || response.model_id() != self.model_id() {
            return Err(GatewayError::Model(ModelError::substitution(format!(
                "provider {} model {} does not match the configured provider {} model {}",
                response.provider_id(),
                response.model_id(),
                self.provider_id(),
                self.model_id()
            ))));
        }
        Ok(response)
    }

    fn validate_structured(
        &self,
        response: ModelResponse,
        schema: &CanonicalValue,
    ) -> Result<ModelResponse, GatewayError> {
        let decoded = kala_tools::protocol::strict_json_loads(
            response.content().as_bytes(),
            super::model::MAX_RESPONSE_BYTES,
        )
        .map_err(|_| {
            GatewayError::Model(ModelError::structured(
                "the model response is not strict JSON",
            ))
        })?;
        if !matches!(decoded, CanonicalValue::Object(_)) {
            return Err(GatewayError::Model(ModelError::structured(
                "the model response is not a JSON object",
            )));
        }
        kala_tools::protocol::validate_against_schema(&decoded, schema, "$", 0).map_err(
            |error| {
                GatewayError::Model(ModelError::structured(format!(
                    "the model response failed its schema: {error}"
                )))
            },
        )?;
        let content = kala_tools::protocol::canonical_json_dumps(
            &super::value::thaw_json(&decoded),
            super::model::MAX_RESPONSE_BYTES,
        )
        .map_err(|error| GatewayError::Model(ModelError::request(error.to_string())))?;
        Ok(response
            .with_structured(Some(&decoded))?
            .with_content(&content)?)
    }

    fn estimate(&self, request: &ModelRequest) -> (i128, Decimal) {
        let prompt_tokens: i128 = request
            .messages()
            .iter()
            .map(|message| estimate_tokens(message.content()))
            .sum();
        (prompt_tokens, Decimal::zero())
    }

    fn reserve(&self, request: &ModelRequest) -> Result<Option<BudgetReservation>, GatewayError> {
        let Some(ledger) = &self.ledger else {
            return Ok(None);
        };
        let (prompt_tokens, cost) = self.estimate(request);
        let total = prompt_tokens + request.max_output_tokens();
        let usage = BudgetUsage::new()
            .with_model_calls(1u32)
            .with_total_tokens(counter_from_i128(total))
            .try_with_cost(cost)?;
        Ok(Some(ledger.reserve(&usage)?))
    }

    fn release_if_held(&self, reservation: Option<BudgetReservation>) {
        let (Some(ledger), Some(reservation)) = (&self.ledger, reservation) else {
            return;
        };
        // The Python implementation suppresses `BudgetReservationUnknownError`
        // here: a reservation that was already committed is not an error.
        let _ = ledger.release(&reservation);
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        request: &ModelRequest,
        response: &ModelResponse,
        redactions: &[ModelRedaction],
        reservation: Option<&BudgetReservation>,
        elapsed_seconds: f64,
    ) -> Result<(), GatewayError> {
        if let (Some(ledger), Some(reservation)) = (&self.ledger, reservation) {
            let delta = UsageDelta::new()
                .model_calls(1u32)
                .total_tokens(counter_from_i128(response.total_tokens()))
                .cost(response.cost().clone())
                .duration_seconds(elapsed_seconds);
            ledger.commit_delta(reservation, &delta)?;
        }
        let (request_body, request_findings) = redact_secrets(&request.request_body()?);
        let (response_body, response_findings) = redact_secrets(&response.canonical_content()?);
        let mut entries = redactions.to_vec();
        if request_findings > 0 {
            entries.push(ModelRedaction::new(
                "secret",
                "request",
                "credential-shaped text was removed from the transcript",
            ));
        }
        if response_findings > 0 {
            entries.push(ModelRedaction::new(
                "secret",
                "response",
                "credential-shaped text was removed from the transcript",
            ));
        }
        let mut request_body = request_body;
        let mut response_body = response_body;
        if request_body.len() > MAX_TRANSCRIPT_BODY_BYTES {
            entries.push(ModelRedaction::new(
                "truncated",
                "request",
                "the transcript request body was truncated",
            ));
            request_body = truncate_encoded(&request_body, MAX_TRANSCRIPT_BODY_BYTES);
        }
        if response_body.len() > MAX_TRANSCRIPT_BODY_BYTES {
            entries.push(ModelRedaction::new(
                "truncated",
                "response",
                "the transcript response body was truncated",
            ));
            response_body = truncate_encoded(&response_body, MAX_TRANSCRIPT_BODY_BYTES);
        }
        let transcript = ModelTranscript::new(
            request.request_id(),
            response.provider_id(),
            response.model_id(),
            &request.request_digest()?,
            &response.response_digest(),
            &request_body,
            &response_body,
            response.input_tokens(),
            response.output_tokens(),
            response.cost().clone(),
            entries,
            request
                .artifacts()
                .iter()
                .map(|artifact| artifact.digest().to_owned())
                .collect(),
            self.now()?,
        )?;
        self.lock().transcripts.push(transcript.clone());
        if let Some(callback) = &self.transcript {
            if let Err(error) = callback(&transcript) {
                return Err(GatewayError::Model(ModelError::transcript(format!(
                    "the model transcript could not be persisted: {}",
                    error.class_name()
                ))));
            }
        }
        Ok(())
    }

    fn now(&self) -> Result<GatewayTimestamp, GatewayError> {
        let value = match &self.clock {
            Some(clock) => clock(),
            None => GatewayTimestamp::now(),
        };
        if !value.is_aware() {
            return Err(GatewayError::Model(ModelError::request(
                "the gateway clock must return an aware datetime",
            )));
        }
        Ok(value)
    }

    fn safe_reason(&self, message: &str) -> String {
        let (_, findings) = redact_secrets(message);
        if findings == 0 {
            message.to_owned()
        } else {
            "<redacted provider error>".to_owned()
        }
    }
}

/// A budget counter for a token count.
fn counter_from_i128(value: i128) -> Counter {
    if value <= 0 {
        return Counter::zero();
    }
    Counter::from(
        BigUint::parse_bytes(value.to_string().as_bytes(), 10)
            .unwrap_or_else(|| BigUint::from(0u32)),
    )
}

/// The egress request identifier of a model request.
fn egress_request_id(request_id: &str) -> String {
    format!("egress-{request_id}")
}

/// The seconds between two aware instants.
fn elapsed_seconds(now: GatewayTimestamp, started: GatewayTimestamp) -> f64 {
    let (Some(now), Some(started)) = (now.as_aware(), started.as_aware()) else {
        return 0.0;
    };
    let micros = now.unix_micros() - started.unix_micros();
    micros as f64 / 1_000_000.0
}

fn sanitize_artifact(
    artifact: &super::model::ModelArtifact,
) -> Result<(super::model::ModelArtifact, Vec<ModelRedaction>), GatewayError> {
    if artifact.is_design_binary() {
        return Ok((
            artifact.redacted(),
            vec![ModelRedaction::new(
                "design_binary",
                artifact.artifact_id(),
                "design binaries are referenced by digest only",
            )],
        ));
    }
    let mut preview_findings = 0usize;
    let preview_text = match artifact.preview_text() {
        Some(text) => {
            let (text, findings) = redact_secrets(text);
            preview_findings = findings;
            Some(text)
        }
        None => None,
    };
    let (metadata, metadata_findings) = redact_metadata(artifact.metadata());
    if preview_findings == 0 && metadata_findings == 0 {
        return Ok((artifact.clone(), Vec::new()));
    }
    let mut redactions = Vec::new();
    if preview_findings > 0 {
        redactions.push(ModelRedaction::new(
            "secret",
            format!("{}.preview_text", artifact.artifact_id()),
            "credential-shaped artifact preview text was redacted",
        ));
    }
    if metadata_findings > 0 {
        redactions.push(ModelRedaction::new(
            "secret",
            format!("{}.metadata", artifact.artifact_id()),
            "credential-shaped artifact metadata was redacted",
        ));
    }
    let sanitized = artifact
        .clone()
        .with_preview_text(preview_text.as_deref())?
        .with_metadata(&metadata)?;
    Ok((sanitized, redactions))
}
