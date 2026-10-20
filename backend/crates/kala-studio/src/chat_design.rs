//! Chat with designs (ADR-0010, ADR-0011).
//!
//! A chat turn may carry a design. The front door never parses a design
//! intent itself and never compiles geometry: it asks the worker, which owns
//! the strict parser, the compiler and the journal.
//!
//! * **Local provider:** the worker parses the prompt with `intent/parse`.
//!   A prompt that is not a design request keeps the ordinary analytic
//!   reply; a design prompt gets the worker's strict reading, and the design
//!   reply text is built from the parsed family, the missing values and the
//!   parser notes.
//! * **Remote providers:** the front door adds the DesignIntent JSON Schema
//!   (fetched from `intent/schema`) to the prompt, extracts the fenced JSON
//!   from the reply and validates it with `intent/validate`. One repair
//!   round-trip is allowed.
//!
//! ADR-0011 is binding on the provider choice: `opencascade` is never
//! selected automatically. When the request carries no `cad_provider`, the
//! turn asks the operator which provider to use and compiles nothing. A
//! selected provider is passed to the worker unchanged: the worker decides
//! whether it can build on this host, the front door relays its refusal in
//! the source's own wording, and no other provider is ever tried.

use std::future::Future;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::geometry_backend::{BackendError, GeometryBackend};
use crate::providers::{ChatOutcome, ProviderFailure};

/// The instruction the remote provider prompt carries.
pub const DESIGN_INTENT_INSTRUCTION: &str = "You are Kala Studio's design planner. When the user asks you to design, size, or draft a part, answer with a short prose explanation and exactly one fenced ```json block containing a DesignIntent object. The block must validate against the JSON Schema below. All dimensions are millimetres. Use a template call when one of the templates matches the request; use a raw ConstructionScript only when the user supplied a complete script. Never invent a dimension the user did not state, and never state a volume, surface area, or envelope as a result: the geometry kernel measures those. If required values are missing, do not emit a JSON block; ask for the missing values in prose instead.";

/// The provider that builds a design with the worker's own compiler.
pub const CAD_PROVIDER_OPENCASCADE: &str = "opencascade";

/// The aggregate the Studio journal stores its events under.
pub const JOURNAL_AGGREGATE_ID: &str = "studio";

/// The event type the front door appends for a chat turn.
pub const CHAT_TURN_EVENT_TYPE: &str = "chat.turn";

/// The largest number of missing values the journal payload carries.
pub const MAX_JOURNAL_MISSING: usize = 16;

/// The largest length of one journaled missing value.
pub const MAX_JOURNAL_MISSING_LENGTH: usize = 64;

/// The design families the strict parser names, and their readable labels.
const FAMILY_LABELS: &[(&str, &str)] = &[
    ("l_bracket", "L-bracket"),
    ("stepped_shaft", "stepped shaft"),
    ("housing_envelope", "rectangular housing envelope"),
];

/// The result of the design hook of one chat turn.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignTurn {
    /// The reply text the turn produces.
    pub text: String,
    /// The design result the worker returned.
    pub design: Option<Value>,
    /// The design identifier, when a design was compiled.
    pub design_id: Option<String>,
    /// Whether the turn asked for a design.
    pub design_requested: bool,
    /// The values the operator still has to state.
    pub missing: Vec<String>,
    /// The model that answered the turn.
    pub model: Option<String>,
}

/// Why a design turn could not be completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesignTurnError {
    /// The worker could not be reached.
    Backend(BackendError),
    /// The provider failed while the remote design prompt was answered.
    Provider(ProviderFailure),
}

impl std::fmt::Display for DesignTurnError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Backend(error) => write!(formatter, "{error}"),
            Self::Provider(failure) => write!(formatter, "{failure:?}"),
        }
    }
}

impl std::error::Error for DesignTurnError {}

/// The strict reading of one prompt, as the worker reports it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedIntent {
    /// The template family, when the prompt asks for a design.
    pub family: Option<String>,
    /// The complete intent, when every required value was stated.
    pub intent: Option<Value>,
    /// The values the operator still has to state.
    pub missing: Vec<String>,
    /// The parser's notes.
    pub notes: Vec<String>,
}

impl ParsedIntent {
    /// Whether the prompt asked for a design at all.
    #[must_use]
    pub fn is_design_request(&self) -> bool {
        self.family.is_some()
    }
}

/// Read the strict reading of a prompt from the worker.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn parse_intent(
    backend: &dyn GeometryBackend,
    prompt: &str,
) -> Result<ParsedIntent, BackendError> {
    let mut body = Map::new();
    body.insert("prompt".to_owned(), Value::String(prompt.to_owned()));
    let response = backend
        .intent_parse(
            &serde_json::to_vec(&Value::Object(body)).unwrap_or_default(),
            Some("application/json"),
        )
        .await?;
    let parsed = response.json().ok_or_else(|| {
        BackendError::Transport("the worker returned an unreadable intent".to_owned())
    })?;
    Ok(ParsedIntent {
        family: parsed
            .get("family")
            .and_then(Value::as_str)
            .map(str::to_owned),
        intent: parsed
            .get("intent")
            .cloned()
            .filter(|value| !value.is_null()),
        missing: string_list(parsed.get("missing")),
        notes: string_list(parsed.get("notes")),
    })
}

fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The reply the local helper gives for a design prompt.
///
/// The text is the worker's strict reading rendered in the source's own
/// wording: the family, the missing values and the notes. The front door
/// never guesses a dimension and never states a measurement.
#[must_use]
pub fn design_reply_text(parsed: &ParsedIntent) -> String {
    let family = parsed.family.clone().unwrap_or_default();
    let label = FAMILY_LABELS
        .iter()
        .find(|(name, _)| *name == family)
        .map(|(_, label)| (*label).to_owned())
        .unwrap_or_else(|| family.replace('_', " "));
    let article = if label
        .chars()
        .next()
        .is_some_and(|first| "aeiou".contains(first.to_ascii_lowercase()))
    {
        "an"
    } else {
        "a"
    };
    let notes = parsed.notes.join(" ");
    match &parsed.intent {
        None => {
            let mut notes = notes;
            if !notes.is_empty() {
                notes.insert(0, ' ');
            }
            let need = if parsed.missing.is_empty() {
                "a complete set of declared dimensions".to_owned()
            } else {
                parsed.missing.join(", ")
            };
            format!(
                "Local helper. I can draft {article} {label}, but I need these declared values: {need}.{notes} No design was compiled."
            )
        }
        Some(_) => {
            let mut notes = notes;
            if !notes.is_empty() {
                notes.push(' ');
            }
            format!(
                "Local helper. Design intent parsed from declared values. {notes}Compiling and measuring with OpenCASCADE."
            )
        }
    }
}

/// The remote provider prompt that asks for a DesignIntent.
#[must_use]
pub fn design_prompt(prompt: &str, schema_text: &str) -> String {
    format!(
        "{DESIGN_INTENT_INSTRUCTION}\n\nDesignIntent JSON Schema:\n{schema_text}\n\nUser request:\n{prompt}"
    )
}

/// The single repair prompt the remote provider is allowed.
#[must_use]
pub fn repair_prompt(prompt: &str, error: &str) -> String {
    let error: String = error.chars().take(512).collect();
    format!(
        "Your previous reply could not be validated as a DesignIntent: {error}\n\nReturn exactly one fenced ```json block containing a corrected DesignIntent for the same request. Do not invent a value the user did not state; if a required value is missing, reply in prose with the missing values instead.\n\nUser request:\n{prompt}"
    )
}

/// The prose of a reply with its fenced design block removed.
#[must_use]
pub fn strip_design_json_block(text: &str) -> String {
    let stripped = strip_blocks(text);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        "Design intent received.".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Remove every fenced design block from a reply.
fn strip_blocks(text: &str) -> String {
    static PATTERN: &str = r"(?is)```(?:json)?\s*(\{.*\})\s*```";
    match regex::Regex::new(PATTERN) {
        Ok(pattern) => pattern.replace_all(text, "").into_owned(),
        Err(_) => text.to_owned(),
    }
}

/// The measured summary the turn appends after a design was compiled.
///
/// The provider that measured the model is named, and a provider that
/// reported no number is never given a substitute.
#[must_use]
pub fn measured_summary(design: &Value) -> String {
    let label = design
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let verified = design
        .get("verified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let verdict = if verified {
        "B-Rep verified"
    } else {
        "B-Rep NOT verified"
    };
    let measured_by = design
        .get("measured_by")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let design_id = design
        .get("design_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let measured = design.get("measured");
    let volume = measured
        .and_then(|value| value.get("volume_mm3"))
        .and_then(Value::as_f64);
    let area = measured
        .and_then(|value| value.get("surface_area_mm2"))
        .and_then(Value::as_f64);
    let size = measured
        .and_then(|value| value.get("size_mm"))
        .and_then(Value::as_array)
        .filter(|values| values.len() >= 3)
        .map(|values| {
            values
                .iter()
                .map(|value| value.as_f64().unwrap_or(0.0))
                .collect::<Vec<_>>()
        });
    match (volume, area, size) {
        (Some(volume), Some(area), Some(size)) => format!(
            "{label}: {verdict}; measured by {measured_by}. Measured volume {volume:.3} mm^3, surface area {area:.3} mm^2, envelope {:.3} x {:.3} x {:.3} mm. design_id {design_id}.",
            size[0], size[1], size[2],
        ),
        _ => format!(
            "{label}: {verdict}; measured by {measured_by}. The provider did not report volume, surface area, or envelope; no number is substituted. design_id {design_id}."
        ),
    }
}

/// Join the reply parts the way the source joins them.
#[must_use]
pub fn join_text(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter_map(|part| {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    joined.chars().take(4_096).collect()
}

/// One CAD provider row, as the front door can prove it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CadProviderStatus {
    /// The provider identifier.
    pub id: String,
    /// The provider's role: `opt-in` for the built-in builder, `external`
    /// for a provider that builds outside the studio.
    pub mode: String,
    /// Whether this front door can build with the provider right now.
    pub available: bool,
}

/// The CAD providers the studio can offer.
///
/// The listing is best effort: an unreachable worker must not turn a chat
/// turn into a failure. The worker's own `providers/cad` route is the
/// authority when it answers; otherwise the front door reports the
/// ADR-0011 provider set and the availability it can prove itself, which is
/// the built-in OpenCascade builder.
pub async fn available_cad_providers(backend: &dyn GeometryBackend) -> Vec<CadProviderStatus> {
    if let Ok(response) = backend.providers_cad().await {
        if response.is_success() {
            if let Some(parsed) = response.json() {
                let entries = parsed
                    .get("providers")
                    .and_then(Value::as_array)
                    .or_else(|| parsed.as_array())
                    .cloned()
                    .unwrap_or_default();
                let rows: Vec<CadProviderStatus> = entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Value::String(text) => Some(CadProviderStatus {
                            id: text.clone(),
                            mode: "external".to_owned(),
                            available: false,
                        }),
                        Value::Object(map) => {
                            let id = map.get("id").and_then(Value::as_str)?.to_owned();
                            let mode = map
                                .get("mode")
                                .and_then(Value::as_str)
                                .unwrap_or("external")
                                .to_owned();
                            let available = map
                                .get("available")
                                .and_then(Value::as_bool)
                                .unwrap_or(false);
                            Some(CadProviderStatus {
                                id,
                                mode,
                                available,
                            })
                        }
                        _ => None,
                    })
                    .collect();
                if !rows.is_empty() {
                    return rows;
                }
            }
        }
    }
    let opencascade = backend.health().await.design_available;
    vec![
        CadProviderStatus {
            id: CAD_PROVIDER_OPENCASCADE.to_owned(),
            mode: "opt-in".to_owned(),
            available: opencascade,
        },
        CadProviderStatus {
            id: "freecad".to_owned(),
            mode: "external".to_owned(),
            available: false,
        },
        CadProviderStatus {
            id: "solidedge".to_owned(),
            mode: "external".to_owned(),
            available: false,
        },
    ]
}

/// The message that asks the operator which CAD provider to use.
#[must_use]
pub fn provider_choice_message(providers: &[CadProviderStatus]) -> String {
    let options = providers
        .iter()
        .map(|provider| format!("{} ({})", provider.id, provider.mode))
        .collect::<Vec<_>>()
        .join(", ");
    let available = providers
        .iter()
        .filter(|provider| provider.available)
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Choose a CAD provider before I build this design: {options}. Available on this host: {}. The studio never selects a provider automatically. No design was built.",
        if available.is_empty() {
            "none"
        } else {
            &available
        }
    )
}

/// One refused design request, as the worker answered it.
///
/// The worker answers a `provider_unavailable` design request with its error
/// envelope: the message, then `reason: …` and `available: …` details. The
/// source's chat route renders the same facts, so the front door relays the
/// worker's answer instead of inventing a refusal of its own. Nothing is
/// substituted and no other provider is tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesignFailure {
    /// The worker's HTTP status.
    pub status: u16,
    /// The worker's bounded error code.
    pub code: String,
    /// The worker's bounded error message.
    pub message: String,
    /// The worker's bounded error details.
    pub details: Vec<String>,
}

impl DesignFailure {
    /// Read one worker error envelope.
    ///
    /// A body that is not the Studio error document keeps the status and the
    /// front door's own fallback code and message, exactly like the previous
    /// refusal text did.
    #[must_use]
    pub fn from_response(response: &crate::geometry_backend::WorkerResponse) -> Self {
        let parsed = response.json();
        let text = |key: &str| {
            parsed
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(Value::as_str)
        };
        Self {
            status: response.status,
            code: text("code").unwrap_or("design_unavailable").to_owned(),
            message: text("message")
                .unwrap_or("the design could not be built")
                .to_owned(),
            details: parsed
                .as_ref()
                .and_then(|value| value.get("details"))
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Whether the worker refused because the selected provider cannot build
    /// on this host (the source's `provider_unavailable`, 503).
    #[must_use]
    pub fn is_provider_unavailable(&self) -> bool {
        self.status == 503 && self.code == "provider_unavailable"
    }

    /// The code/message/detail form used when the turn reports a failure that
    /// is not the source's CAD provider refusal.
    #[must_use]
    pub fn summary(&self) -> String {
        let detail = self.details.first().map(String::as_str).unwrap_or_default();
        if detail.is_empty() {
            format!("{}: {}", self.code, self.message)
        } else {
            format!("{}: {} ({})", self.code, self.message, detail)
        }
    }

    /// The chat reply for this refusal, in the source's own wording.
    ///
    /// A CAD provider refusal uses the source's `_provider_unavailable_chat_message`:
    /// the requested provider, the worker's reason, the providers that can
    /// build here and `No design was built.`
    #[must_use]
    pub fn chat_message(&self, provider: &str) -> String {
        if self.is_provider_unavailable() {
            let reason = self
                .detail_value("reason: ")
                .unwrap_or(self.message.as_str());
            let available = self.detail_value("available: ").unwrap_or("none");
            return format!(
                "The {provider} CAD provider is not available: {reason}. Available: {available}. No design was built."
            );
        }
        format!("The design could not be built: {}.", self.summary())
    }

    fn detail_value(&self, prefix: &str) -> Option<&str> {
        self.details
            .iter()
            .find_map(|detail| detail.strip_prefix(prefix))
    }
}

/// Compile one intent with the worker.
///
/// The ADR-0011 provider is part of the design request and is passed through
/// unchanged. The worker decides whether the provider can build on this host:
/// a provider it refuses comes back as a [`DesignFailure`] and the front door
/// never retries with another provider.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn compile_design(
    backend: &dyn GeometryBackend,
    intent: &Value,
    cad_provider: &str,
) -> Result<Result<Value, DesignFailure>, BackendError> {
    let mut body = match intent {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    // The operator's selection is authoritative: it always replaces anything
    // an intent might carry under the same name.
    body.insert(
        "provider".to_owned(),
        Value::String(cad_provider.to_owned()),
    );
    let payload = serde_json::to_vec(&Value::Object(body)).unwrap_or_default();
    let response = backend.design(&payload, Some("application/json")).await?;
    if response.is_success() {
        return Ok(Ok(response.json().unwrap_or(Value::Null)));
    }
    Ok(Err(DesignFailure::from_response(&response)))
}

/// Append the `chat.turn` event for one turn.
///
/// The payload never carries the prompt text, only its digest and length,
/// exactly like the source.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
#[allow(clippy::too_many_arguments)]
pub async fn append_chat_turn(
    backend: &dyn GeometryBackend,
    provider: &str,
    model: &str,
    offline: bool,
    prompt: &str,
    design_id: Option<&str>,
    design_requested: bool,
    cad_provider: Option<&str>,
    missing: &[String],
) -> Result<(), BackendError> {
    let digest = Sha256::digest(prompt.as_bytes());
    let mut payload = Map::new();
    payload.insert("provider".to_owned(), Value::String(provider.to_owned()));
    payload.insert("model".to_owned(), Value::String(model.to_owned()));
    payload.insert("offline".to_owned(), Value::Bool(offline));
    payload.insert(
        "prompt_sha256".to_owned(),
        Value::String(digest.iter().map(|byte| format!("{byte:02x}")).collect()),
    );
    payload.insert(
        "prompt_chars".to_owned(),
        Value::Number(prompt.chars().count().into()),
    );
    payload.insert(
        "design_id".to_owned(),
        design_id
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    payload.insert("design_requested".to_owned(), Value::Bool(design_requested));
    payload.insert(
        "cad_provider".to_owned(),
        cad_provider
            .map(|value| Value::String(value.to_owned()))
            .unwrap_or(Value::Null),
    );
    payload.insert(
        "missing".to_owned(),
        Value::Array(
            missing
                .iter()
                .take(MAX_JOURNAL_MISSING)
                .map(|item| Value::String(item.chars().take(MAX_JOURNAL_MISSING_LENGTH).collect()))
                .collect(),
        ),
    );
    let mut request = Map::new();
    request.insert(
        "event_type".to_owned(),
        Value::String(CHAT_TURN_EVENT_TYPE.to_owned()),
    );
    request.insert("payload".to_owned(), Value::Object(payload));
    request.insert(
        "aggregate_id".to_owned(),
        Value::String(JOURNAL_AGGREGATE_ID.to_owned()),
    );
    let encoded = serde_json::to_vec(&Value::Object(request)).unwrap_or_default();
    backend
        .journal_append(&encoded, Some("application/json"))
        .await?;
    Ok(())
}

/// Run the local design turn.
///
/// The local helper's answer for a design prompt is the strict reading: the
/// family, the values that are missing and the parser notes. A complete
/// intent is compiled when the operator selected a CAD provider.
///
/// # Errors
///
/// Returns the backend failure when the worker cannot answer.
pub async fn local_design_turn(
    backend: &dyn GeometryBackend,
    parsed: &ParsedIntent,
    cad_provider: Option<&str>,
) -> DesignTurn {
    let mut turn = DesignTurn {
        text: design_reply_text(parsed),
        design: None,
        design_id: None,
        design_requested: true,
        missing: parsed.missing.clone(),
        model: None,
    };
    if let Some(intent) = parsed.intent.clone() {
        let _ = finish_design(backend, &intent, cad_provider, &mut turn).await;
    }
    turn
}

/// Run the remote design flow: schema, prompt, validate, one repair, design.
///
/// # Errors
///
/// Returns the provider or backend failure that stopped the turn.
pub async fn remote_design_turn<F, Fut>(
    backend: &dyn GeometryBackend,
    prompt: &str,
    cad_provider: Option<&str>,
    mut call: F,
) -> Result<DesignTurn, DesignTurnError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<ChatOutcome, ProviderFailure>>,
{
    let schema = backend
        .intent_schema()
        .await
        .map_err(DesignTurnError::Backend)?;
    let schema_text = String::from_utf8(schema.body).unwrap_or_default();
    let first = call(design_prompt(prompt, &schema_text))
        .await
        .map_err(DesignTurnError::Provider)?;
    let mut reply_text = strip_design_json_block(&first.text);
    let intent = match validate_reply(backend, &first.text).await {
        Ok(intent) => Some(intent),
        Err(first_error) => {
            let repair = call(repair_prompt(prompt, &first_error))
                .await
                .map_err(DesignTurnError::Provider)?;
            reply_text = strip_design_json_block(&repair.text);
            match validate_reply(backend, &repair.text).await {
                Ok(intent) => Some(intent),
                Err(second_error) => {
                    let message = format!(
                        "{} No validated design was produced: {}",
                        reply_text,
                        second_error.chars().take(512).collect::<String>()
                    );
                    return Ok(DesignTurn {
                        text: message,
                        design: None,
                        design_id: None,
                        design_requested: true,
                        missing: Vec::new(),
                        model: Some(repair.model.clone()),
                    });
                }
            }
        }
    };
    let mut turn = DesignTurn {
        text: reply_text,
        design: None,
        design_id: None,
        design_requested: true,
        missing: Vec::new(),
        model: Some(first.model.clone()),
    };
    if let Some(intent) = intent {
        finish_design(backend, &intent, cad_provider, &mut turn).await?;
    }
    Ok(turn)
}

/// Validate one model reply with the worker.
async fn validate_reply(backend: &dyn GeometryBackend, text: &str) -> Result<Value, String> {
    let mut body = Map::new();
    body.insert("text".to_owned(), Value::String(text.to_owned()));
    let payload = serde_json::to_vec(&Value::Object(body)).unwrap_or_default();
    let response = backend
        .intent_validate(&payload, Some("application/json"))
        .await
        .map_err(|error| error.to_string())?;
    if !response.is_success() {
        let parsed = response.json();
        let detail = parsed
            .as_ref()
            .and_then(|value| value.get("details"))
            .and_then(Value::as_array)
            .and_then(|entries| entries.first())
            .and_then(Value::as_str)
            .unwrap_or("the model reply did not contain a valid DesignIntent");
        return Err(detail.to_owned());
    }
    let parsed = response
        .json()
        .ok_or_else(|| "the worker returned an unreadable intent".to_owned())?;
    parsed
        .get("intent")
        .cloned()
        .ok_or_else(|| "the worker returned no intent".to_owned())
}

/// Compile the design and record it on the turn.
///
/// The selected ADR-0011 provider is passed to the worker unchanged, and the
/// worker alone decides whether it can build on this host. A refusal is
/// relayed in the source's own chat wording; the front door never refuses a
/// provider itself and never falls back to another one.
async fn finish_design(
    backend: &dyn GeometryBackend,
    intent: &Value,
    cad_provider: Option<&str>,
    turn: &mut DesignTurn,
) -> Result<(), DesignTurnError> {
    let Some(cad_provider) = cad_provider else {
        let providers = available_cad_providers(backend).await;
        turn.text = join_text(&[&turn.text, &provider_choice_message(&providers)]);
        return Ok(());
    };
    let outcome = compile_design(backend, intent, cad_provider)
        .await
        .map_err(DesignTurnError::Backend)?;
    match outcome {
        Ok(design) => {
            let design_id = design
                .get("design_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let summary = measured_summary(&design);
            turn.text = join_text(&[&turn.text, &summary]);
            turn.design = Some(design);
            turn.design_id = design_id;
        }
        Err(failure) => {
            turn.text = join_text(&[&turn.text, &failure.chat_message(cad_provider)]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_missing_value_reply_names_the_family_and_the_values() {
        let parsed = ParsedIntent {
            family: Some("l_bracket".to_owned()),
            intent: None,
            missing: vec!["width (mm)".to_owned(), "depth (mm)".to_owned()],
            notes: Vec::new(),
        };
        assert_eq!(
            design_reply_text(&parsed),
            "Local helper. I can draft a L-bracket, but I need these declared values: width (mm), depth (mm). No design was compiled."
        );
    }

    #[test]
    fn the_complete_intent_reply_mentions_the_compiler() {
        let parsed = ParsedIntent {
            family: Some("stepped_shaft".to_owned()),
            intent: Some(Value::Object(Map::new())),
            missing: Vec::new(),
            notes: vec!["a note".to_owned()],
        };
        assert_eq!(
            design_reply_text(&parsed),
            "Local helper. Design intent parsed from declared values. a note Compiling and measuring with OpenCASCADE."
        );
    }

    #[test]
    fn the_fenced_block_is_stripped_from_the_prose() {
        let reply = "Here you go.\n```json\n{\"template\": null}\n```\nDone.";
        assert_eq!(strip_design_json_block(reply), "Here you go.\n\nDone.");
        assert_eq!(strip_design_json_block(""), "Design intent received.");
    }

    #[test]
    fn the_measured_summary_matches_the_source_shape() {
        let design = serde_json::json!({
            "design_id": "abc",
            "label": "Bracket",
            "verified": true,
            "measured_by": "OpenCascade (opt-in)",
            "measured": {
                "volume_mm3": 35571.416047,
                "surface_area_mm2": 1234.5,
                "size_mm": [60.0, 45.0, 40.0]
            }
        });
        assert_eq!(
            measured_summary(&design),
            "Bracket: B-Rep verified; measured by OpenCascade (opt-in). Measured volume 35571.416 mm^3, surface area 1234.500 mm^2, envelope 60.000 x 45.000 x 40.000 mm. design_id abc."
        );
    }

    #[test]
    fn a_provider_without_numbers_never_gets_a_substitute() {
        let design = serde_json::json!({
            "design_id": "abc",
            "label": "Bracket",
            "verified": false,
            "measured_by": "FreeCAD",
            "measured": {
                "volume_mm3": null,
                "surface_area_mm2": null,
                "size_mm": []
            }
        });
        assert_eq!(
            measured_summary(&design),
            "Bracket: B-Rep NOT verified; measured by FreeCAD. The provider did not report volume, surface area, or envelope; no number is substituted. design_id abc."
        );
    }

    #[test]
    fn the_journal_payload_never_carries_the_prompt() {
        let digest = Sha256::digest("hello".as_bytes());
        let expected: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(expected.len(), 64);
    }

    fn refusal_response(status: u16, body: Value) -> crate::geometry_backend::WorkerResponse {
        crate::geometry_backend::WorkerResponse {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body: serde_json::to_vec(&body).unwrap(),
        }
    }

    #[test]
    fn a_provider_refusal_is_relayed_in_the_source_wording() {
        // The exact envelope the worker's public design route answers with for
        // Solid Edge on Linux: the message, the reason and the availability
        // list. The source's chat route renders the same three facts.
        let response = refusal_response(
            503,
            serde_json::json!({
                "code": "provider_unavailable",
                "message": "the solidedge CAD provider is not available",
                "details": [
                    "the solidedge CAD provider is not available",
                    "reason: Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here",
                    "available: opencascade, freecad"
                ]
            }),
        );
        let failure = DesignFailure::from_response(&response);
        assert!(failure.is_provider_unavailable());
        assert_eq!(
            failure.chat_message("solidedge"),
            "The solidedge CAD provider is not available: Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here. Available: opencascade, freecad. No design was built."
        );
    }

    #[test]
    fn a_refusal_without_a_reason_detail_falls_back_to_the_message() {
        let response = refusal_response(
            503,
            serde_json::json!({
                "code": "provider_unavailable",
                "message": "the freecad CAD provider is not available",
                "details": [
                    "the freecad CAD provider is not available",
                    "available: none"
                ]
            }),
        );
        assert_eq!(
            DesignFailure::from_response(&response).chat_message("freecad"),
            "The freecad CAD provider is not available: the freecad CAD provider is not available. Available: none. No design was built."
        );
    }

    #[test]
    fn another_refusal_keeps_the_front_door_summary() {
        let response = refusal_response(
            422,
            serde_json::json!({
                "code": "design_rejected",
                "message": "the design could not be compiled or measured",
                "details": ["the profile is not supported"]
            }),
        );
        let failure = DesignFailure::from_response(&response);
        assert!(!failure.is_provider_unavailable());
        assert_eq!(
            failure.summary(),
            "design_rejected: the design could not be compiled or measured (the profile is not supported)"
        );
        assert_eq!(
            failure.chat_message("freecad"),
            "The design could not be built: design_rejected: the design could not be compiled or measured (the profile is not supported)."
        );
    }

    #[test]
    fn an_unreadable_refusal_keeps_the_status_and_the_fallback_code() {
        let response = crate::geometry_backend::WorkerResponse {
            status: 502,
            headers: Vec::new(),
            body: b"not json".to_vec(),
        };
        let failure = DesignFailure::from_response(&response);
        assert_eq!(failure.status, 502);
        assert_eq!(failure.code, "design_unavailable");
        assert_eq!(failure.message, "the design could not be built");
        assert!(failure.details.is_empty());
    }
}
