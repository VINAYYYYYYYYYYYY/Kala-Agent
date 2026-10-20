//! The native geometry backend (ADR-0010 phase 2).
//!
//! This is the default front door: it serves the preview, export, handoff,
//! design, import, intent, journal and CAD-provider routes from the reviewed
//! native crates, with **no Python process anywhere in the picture**.
//!
//! * the design, import and measurement work is the real OpenCascade
//!   adapter ([`kala_cad`]), which calls OCCT and never rewrites it;
//! * the CAD provider connections are the reviewed ones
//!   ([`kala_cad_providers`]): FreeCAD runs out of process, Solid Edge is
//!   reached over the ADR-0008 loopback bridge;
//! * the design-intent contract, the templates and the strict prompt parser
//!   are the reviewed ones ([`kala_design_intent`]);
//! * the session journal and the content-addressed artifact store are the
//!   reviewed ones ([`kala_studio_journal`]), byte-compatible with the Python
//!   journal.
//!
//! The Python worker stays available **only** behind an explicit dev opt-in
//! (`KALA_STUDIO_GEOMETRY_WORKER` set); the front door then supervises it and
//! proxies to it exactly as before.

mod documents;
mod providers;
mod service;
mod wire;

pub use providers::{
    OPENCASCADE_DISPLAY_NAME, OPENCASCADE_MODE, PROVIDER_FREECAD, PROVIDER_OPENCASCADE,
    PROVIDER_SOLIDEDGE,
};
pub use service::{
    CheckRow, CheckValue, DesignError, DesignOutcome, ExportOutcome, ImportedView,
    NativeDesignService,
};

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use kala_design_intent::{DesignIntent, DesignIntentInputError};
use serde_json::{Map, Value};

use crate::errors::{
    FlatError, INTERNAL_ERROR_MESSAGE, REQUEST_REJECTED_MESSAGE, VALIDATION_MESSAGE,
};
use crate::geometry::StepExporter;
use crate::geometry_backend::{
    BackendError, BackendFuture, BackendHealth, GeometryBackend, WorkerResponse,
};
use crate::json::Json;
use crate::native_backend::NativePrimitivesBackend;
use crate::schemas::{RequestParseError, parse_json_body};

/// The largest journal page the front door serves.
pub const MAX_JOURNAL_LIMIT: i64 = 500;
/// The default journal page size.
pub const DEFAULT_JOURNAL_LIMIT: i64 = 200;
/// The largest canonical journal payload the append route accepts.
pub const MAX_APPEND_PAYLOAD_BYTES: usize = 64 * 1024;
/// The event types the append route accepts.
pub const ALLOWED_APPEND_EVENT_TYPES: [&str; 4] = [
    "chat.turn",
    "export.completed",
    "provider.call",
    "settings.changed",
];

/// The detail of a design request without a provider.
const PROVIDER_REQUIRED_DETAIL: &str =
    "body: value_error: Value error, provider is required (choose opencascade, freecad, solidedge)";
/// The detail of a design request with an unknown provider.
const PROVIDER_ENUM_DETAIL: &str =
    "body.provider: enum: Input should be 'opencascade', 'freecad' or 'solidedge'";

/// The native geometry backend.
pub struct NativeGeometryBackend {
    design_service: Arc<NativeDesignService>,
    primitives: NativePrimitivesBackend,
}

impl NativeGeometryBackend {
    /// Build the backend over one Studio data directory.
    ///
    /// The provider connections see the same controlled environment the
    /// supervised Python worker saw, so a FreeCAD probe reports the same
    /// facts on both sides.
    ///
    /// # Errors
    ///
    /// Returns a message when the Studio data directory or the journal cannot
    /// be opened.
    pub fn new(
        data_dir: PathBuf,
        environ: BTreeMap<String, String>,
        max_step_bytes: u64,
        exporter: Arc<dyn StepExporter>,
        workdir: PathBuf,
    ) -> Result<Arc<Self>, String> {
        let design_service = Arc::new(NativeDesignService::new(
            &data_dir,
            environ,
            max_step_bytes,
        )?);
        Ok(Arc::new(Self {
            design_service,
            primitives: NativePrimitivesBackend::new(exporter, workdir),
        }))
    }

    /// The design service this backend owns.
    #[must_use]
    pub fn service(&self) -> &Arc<NativeDesignService> {
        &self.design_service
    }
}

impl GeometryBackend for NativeGeometryBackend {
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async move {
            let capability = kala_cad::probe_capability();
            BackendHealth {
                state: if capability.available {
                    "ready".to_owned()
                } else {
                    "stopped".to_owned()
                },
                version: Some(capability.kernel),
                design_available: capability.available,
                step_available: capability.available,
            }
        })
    }

    fn preview<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        self.primitives.preview(body, content_type)
    }

    fn export<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(request_parse_failure(error)),
            };
            if let Some(design_id) = map.get("design_id").and_then(Value::as_str) {
                let design_id = design_id.to_owned();
                return self.export_design(&design_id, &map).await;
            }
            self.primitives.export(body, content_type).await
        })
    }

    fn handoff<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        self.primitives.handoff(body, content_type)
    }

    fn design<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(request_parse_failure(error)),
            };
            let (intent, provider) = match validate_design_request(&map) {
                Ok(request) => request,
                Err(error) => return Ok(validation_response(error)),
            };
            let design_service = Arc::clone(&self.design_service);
            let outcome =
                tokio::task::spawn_blocking(move || design_service.design(&intent, &provider))
                    .await;
            match outcome {
                Ok(Ok(result)) => Ok(json_response(200, &documents::design_document(&result))),
                Ok(Err(error)) => Ok(design_failure(&error, None)),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn drawing<'a>(
        &'a self,
        design_id: &'a str,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let views = match parse_views_query(query) {
                Ok(views) => views,
                Err(refusal) => return Ok(query_refusal(refusal)),
            };
            let design_service = Arc::clone(&self.design_service);
            let design_id = design_id.to_owned();
            let requested = design_id.clone();
            let outcome =
                tokio::task::spawn_blocking(move || design_service.drawing(&requested, &views))
                    .await;
            match outcome {
                Ok(Ok(drawing)) => Ok(json_response(
                    200,
                    &documents::drawing_document(design_id.as_str(), &drawing),
                )),
                Ok(Err(error)) => Ok(design_failure(&error, Some(design_id))),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn section<'a>(
        &'a self,
        design_id: &'a str,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let (plane, offset) = match parse_section_query(query) {
                Ok(value) => value,
                Err(refusal) => return Ok(query_refusal(refusal)),
            };
            let design_service = Arc::clone(&self.design_service);
            let design_id = design_id.to_owned();
            let requested = design_id.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                design_service.section(&requested, &plane, offset)
            })
            .await;
            match outcome {
                Ok(Ok(section)) => Ok(json_response(
                    200,
                    &documents::section_document(design_id.as_str(), &section),
                )),
                Ok(Err(error)) => Ok(design_failure(&error, Some(design_id))),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn import_step<'a>(
        &'a self,
        body: &'a [u8],
        _content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let design_service = Arc::clone(&self.design_service);
            let bytes = body.to_vec();
            let outcome =
                tokio::task::spawn_blocking(move || design_service.import_and_store(&bytes)).await;
            match outcome {
                Ok(Ok(view)) => {
                    let digest = service::sha256_hex(body);
                    Ok(json_response(
                        200,
                        &documents::import_document(&view, &digest),
                    ))
                }
                Ok(Err(error)) => Ok(design_failure(&error, None)),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn intent_parse<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(request_parse_failure(error)),
            };
            let Some(prompt) = map.get("prompt").and_then(Value::as_str) else {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.prompt: missing: Field required".to_owned()],
                )));
            };
            if prompt.is_empty() {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec![
                        "body.prompt: string_too_short: String should have at least 1 character"
                            .to_owned(),
                    ],
                )));
            }
            if prompt.chars().count() > crate::schemas::MAX_PROMPT_LENGTH {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec![format!(
                        "body.prompt: string_too_long: String should have at most {} characters",
                        crate::schemas::MAX_PROMPT_LENGTH
                    )],
                )));
            }
            match kala_design_intent::parse_design_prompt(prompt) {
                Ok(None) => Ok(json_response(
                    200,
                    &documents::intent_parse_document(None, None, &[], &[]),
                )),
                Ok(Some(parsed)) => {
                    let intent = parsed
                        .intent()
                        .map(|intent| wire::to_response_value(&intent.to_wire_value()));
                    Ok(json_response(
                        200,
                        &documents::intent_parse_document(
                            Some(parsed.family().as_str()),
                            intent,
                            parsed.missing(),
                            parsed.notes(),
                        ),
                    ))
                }
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn intent_validate<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(request_parse_failure(error)),
            };
            let Some(text) = map.get("text").and_then(Value::as_str) else {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.text: missing: Field required".to_owned()],
                )));
            };
            if text.is_empty() {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec![
                        "body.text: string_too_short: String should have at least 1 character"
                            .to_owned(),
                    ],
                )));
            }
            match kala_design_intent::parse_design_intent_text(text) {
                Ok(intent) => Ok(json_response(
                    200,
                    &documents::intent_validate_document(wire::to_response_value(
                        &intent.to_wire_value(),
                    )),
                )),
                Err(error) => Ok(sorted_error(
                    422,
                    "invalid_intent",
                    "the model reply did not contain a valid DesignIntent",
                    vec![crate::errors::truncate_chars(error.message(), 256)],
                )),
            }
        })
    }

    fn intent_schema<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            Ok(WorkerResponse {
                status: 200,
                headers: vec![(
                    "content-type".to_owned(),
                    "text/plain; charset=utf-8".to_owned(),
                )],
                body: kala_design_intent::design_intent_schema_text()
                    .as_bytes()
                    .to_vec(),
            })
        })
    }

    fn journal_page<'a>(
        &'a self,
        query: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let (after, limit) = match parse_journal_query(query) {
                Ok(value) => value,
                Err(details) => {
                    return Ok(validation_response(FlatError::new(
                        422,
                        "invalid_request",
                        VALIDATION_MESSAGE,
                        details,
                    )));
                }
            };
            let effective_limit = limit.min(MAX_JOURNAL_LIMIT);
            let design_service = Arc::clone(&self.design_service);
            let outcome = tokio::task::spawn_blocking(move || {
                let events = design_service.journal_page(after, effective_limit)?;
                let next_after = events.last().map(|event| event.sequence).unwrap_or(after);
                let has_more = if events.is_empty() {
                    false
                } else {
                    !design_service.journal_page(next_after, 1)?.is_empty()
                };
                Ok::<_, service::DesignError>((events, next_after, has_more))
            })
            .await;
            match outcome {
                Ok(Ok((events, next_after, has_more))) => Ok(json_response(
                    200,
                    &documents::journal_page_document(
                        &events,
                        next_after,
                        has_more,
                        effective_limit,
                    ),
                )),
                Ok(Err(_)) => Ok(journal_failure()),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn journal_verify<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let design_service = Arc::clone(&self.design_service);
            match tokio::task::spawn_blocking(move || design_service.verify()).await {
                Ok(Ok(verification)) => Ok(json_response(
                    200,
                    &documents::journal_verify_document(&verification),
                )),
                Ok(Err(_)) => Ok(journal_failure()),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn journal_append<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(request_parse_failure(error)),
            };
            let Some(event_type) = map.get("event_type").and_then(Value::as_str) else {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.event_type: missing: Field required".to_owned()],
                )));
            };
            if !ALLOWED_APPEND_EVENT_TYPES.contains(&event_type) {
                let allowed = ALLOWED_APPEND_EVENT_TYPES.join(", ");
                return Ok(sorted_error(
                    422,
                    "event_type_not_allowed",
                    "the worker journal accepts only Rust-originated event types",
                    vec![format!(
                        "event_type={}; allowed={allowed}",
                        crate::errors::truncate_chars(event_type, 64)
                    )],
                ));
            }
            let Some(payload) = map.get("payload") else {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.payload: missing: Field required".to_owned()],
                )));
            };
            if !payload.is_object() {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.payload: dict_type: Input should be a valid dictionary".to_owned()],
                )));
            }
            let aggregate_id = map
                .get("aggregate_id")
                .and_then(Value::as_str)
                .unwrap_or("studio")
                .to_owned();
            let text = match serde_json::to_string(payload) {
                Ok(text) => text,
                Err(_) => return Ok(internal_failure()),
            };
            let canonical = match kala_persistence::canonicalize_json_object(&text) {
                Ok(canonical) => canonical,
                Err(_) => {
                    return Ok(sorted_error(
                        422,
                        "invalid_request",
                        "the journal payload is invalid",
                        vec!["the payload must be a canonical JSON object".to_owned()],
                    ));
                }
            };
            if canonical.len() > MAX_APPEND_PAYLOAD_BYTES {
                return Ok(sorted_error(
                    422,
                    "invalid_request",
                    "the journal payload is invalid",
                    vec![format!(
                        "the payload must not exceed {MAX_APPEND_PAYLOAD_BYTES} bytes"
                    )],
                ));
            }
            if kala_json::strict_json_loads(canonical.as_bytes()).is_err() {
                return Ok(internal_failure());
            }
            let design_service = Arc::clone(&self.design_service);
            let event_type = event_type.to_owned();
            let outcome = tokio::task::spawn_blocking(move || {
                design_service.journal().append_json_with(
                    &event_type,
                    &canonical,
                    &kala_studio_journal::AppendOptions::default().with_aggregate_id(aggregate_id),
                )
            })
            .await;
            match outcome {
                Ok(Ok(event)) => Ok(json_response(
                    201,
                    &documents::journal_event_document(&event),
                )),
                Ok(Err(error)) => Ok(sorted_error(
                    422,
                    "invalid_request",
                    "the journal payload is invalid",
                    vec![crate::errors::truncate_chars(&error.to_string(), 256)],
                )),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn providers_cad<'a>(&'a self) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let design_service = Arc::clone(&self.design_service);
            match tokio::task::spawn_blocking(move || design_service.probe_providers()).await {
                Ok(statuses) => Ok(json_response(
                    200,
                    &documents::providers_cad_document(&statuses),
                )),
                Err(_) => Ok(internal_failure()),
            }
        })
    }
}

impl NativeGeometryBackend {
    /// Export a stored design.
    async fn export_design(
        &self,
        design_id: &str,
        map: &Map<String, Value>,
    ) -> Result<WorkerResponse, BackendError> {
        let format = match map.get("format").and_then(Value::as_str) {
            Some(format) if format == "stl" || format == "step" => format.to_owned(),
            Some(_) => {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.format: enum: Input should be 'stl' or 'step'".to_owned()],
                )));
            }
            None => {
                return Ok(validation_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec!["body.format: missing: Field required".to_owned()],
                )));
            }
        };
        let design_service = Arc::clone(&self.design_service);
        let design_id = design_id.to_owned();
        let exported = design_id.clone();
        let outcome =
            tokio::task::spawn_blocking(move || design_service.export(&exported, &format)).await;
        match outcome {
            Ok(Ok(result)) => {
                let media = if result.format == "stl" {
                    "model/stl"
                } else {
                    "application/step"
                };
                Ok(WorkerResponse {
                    status: 200,
                    headers: vec![
                        ("content-type".to_owned(), media.to_owned()),
                        (
                            "content-disposition".to_owned(),
                            format!("attachment; filename=\"{}\"", result.filename),
                        ),
                        ("x-studio-design-id".to_owned(), result.design_id.clone()),
                        ("x-studio-digest".to_owned(), result.sha256.clone()),
                        ("x-studio-artifact-kind".to_owned(), result.format.clone()),
                    ],
                    body: result.data,
                })
            }
            Ok(Err(error)) => Ok(design_failure(&error, Some(design_id))),
            Err(_) => Ok(internal_failure()),
        }
    }
}

// ------------------------------------------------------------ request validation

/// Validate one design request the way the Python `DesignRequest` does.
///
/// The provider is checked first, exactly like the Python model validator
/// that runs before the field validation: a request without a provider is
/// refused with the provider message, whatever else it carries.
fn validate_design_request(map: &Map<String, Value>) -> Result<(DesignIntent, String), FlatError> {
    if !map.contains_key("provider") {
        return Err(FlatError::new(
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec![PROVIDER_REQUIRED_DETAIL.to_owned()],
        ));
    }
    let mut provider_details: Vec<String> = Vec::new();
    let provider = match map.get("provider").and_then(Value::as_str) {
        Some(value) if crate::schemas::CAD_PROVIDER_IDS.contains(&value) => value.to_owned(),
        _ => {
            provider_details.push(PROVIDER_ENUM_DETAIL.to_owned());
            String::new()
        }
    };
    let mut intent_map = map.clone();
    intent_map.remove("provider");
    let text = serde_json::to_string(&Value::Object(intent_map)).unwrap_or_default();
    match DesignIntent::from_json_str(&text) {
        Ok(intent) => {
            if provider_details.is_empty() {
                Ok((intent, provider))
            } else {
                Err(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    provider_details,
                ))
            }
        }
        Err(DesignIntentInputError::Validation(error)) => {
            let mut details: Vec<String> = Vec::new();
            for issue in error.issues() {
                let location = if issue.location().is_empty() {
                    "body".to_owned()
                } else {
                    format!(
                        "body.{}",
                        issue
                            .location()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(".")
                    )
                };
                let (kind, message) = pydantic_issue(issue.message());
                details.push(format!("{location}: {kind}: {message}"));
            }
            details.extend(provider_details);
            Err(FlatError::new(
                422,
                "invalid_request",
                VALIDATION_MESSAGE,
                details,
            ))
        }
        Err(DesignIntentInputError::Json(message)) => Err(FlatError::new(
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec![format!("body: json_invalid: {message}")],
        )),
    }
}

/// The Pydantic error kind and message of one validation issue.
fn pydantic_issue(message: &str) -> (&'static str, String) {
    if message == "Extra inputs are not permitted" {
        ("extra_forbidden", message.to_owned())
    } else if message == "Field required" {
        ("missing", message.to_owned())
    } else if message.starts_with("Input should be a valid string") {
        ("string_type", message.to_owned())
    } else if message.starts_with("Input should be a valid dictionary") {
        ("model_type", message.to_owned())
    } else if message.starts_with("Input should be '") {
        ("enum", message.to_owned())
    } else {
        ("value_error", format!("Value error, {message}"))
    }
}

/// Parse the `after` and `limit` query parameters of the journal page.
fn parse_journal_query(query: Option<&str>) -> Result<(i64, i64), Vec<String>> {
    let mut after = 0_i64;
    let mut limit = DEFAULT_JOURNAL_LIMIT;
    let Some(query) = query else {
        return Ok((after, limit));
    };
    for pair in query.split('&') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = percent_decode(value);
        match name {
            "after" => match decoded.parse::<i64>() {
                Ok(number) => {
                    if number < 0 {
                        return Err(vec![
                            "query.after: greater_than_equal: Input should be greater than or equal to 0"
                                .to_owned(),
                        ]);
                    }
                    after = number;
                }
                Err(_) => {
                    return Err(vec![
                        "query.after: int_parsing: Input should be a valid integer, unable to parse string as an integer"
                            .to_owned(),
                    ]);
                }
            },
            "limit" => match decoded.parse::<i64>() {
                Ok(number) => {
                    if number < 1 {
                        return Err(vec![
                            "query.limit: greater_than_equal: Input should be greater than or equal to 1"
                                .to_owned(),
                        ]);
                    }
                    if number > MAX_JOURNAL_LIMIT {
                        return Err(vec![
                            "query.limit: less_than_equal: Input should be less than or equal to 500"
                                .to_owned(),
                        ]);
                    }
                    limit = number;
                }
                Err(_) => {
                    return Err(vec![
                        "query.limit: int_parsing: Input should be a valid integer, unable to parse string as an integer"
                            .to_owned(),
                    ]);
                }
            },
            _ => {}
        }
    }
    Ok((after, limit))
}

/// A refused query parameter.
///
/// The two kinds are different bodies: a framework query-parameter failure is
/// Pydantic model output (`code`, `message`, `details`), and a route refusal
/// is the sorted `json.dumps(..., sort_keys=True)` form.
#[derive(Clone, Debug)]
enum QueryRefusal {
    /// The framework refused the query parameter itself.
    Framework(Vec<String>),
    /// The route refused the value of the query parameter.
    Route(Vec<String>),
}

/// The refusal body of one refused query parameter.
fn query_refusal(refusal: QueryRefusal) -> WorkerResponse {
    match refusal {
        QueryRefusal::Framework(details) => validation_response(FlatError::new(
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            details,
        )),
        QueryRefusal::Route(details) => {
            sorted_error(422, "invalid_request", VALIDATION_MESSAGE, details)
        }
    }
}

/// Parse the `views` query of the drawing route.
///
/// The query parameter is a scalar, so only its first occurrence counts,
/// exactly like the framework's scalar query binding.
///
/// # Errors
///
/// Returns the bounded 422 details of an unusable view selection.
fn parse_views_query(query: Option<&str>) -> Result<Vec<&'static str>, QueryRefusal> {
    const ALLOWED_VIEWS: [&str; 4] = ["front", "top", "right", "iso"];
    let Some(raw) = first_query_value(query, "views") else {
        return Ok(ALLOWED_VIEWS.to_vec());
    };
    if raw.chars().count() > 64 {
        return Err(QueryRefusal::Framework(vec![
            "query.views: string_too_long: String should have at most 64 characters".to_owned(),
        ]));
    }
    if raw.trim().is_empty() {
        return Ok(ALLOWED_VIEWS.to_vec());
    }
    let mut views: Vec<&'static str> = Vec::new();
    for piece in raw.split(',') {
        let candidate = piece.trim().to_ascii_lowercase();
        if candidate.is_empty() {
            return Err(QueryRefusal::Route(vec![
                "views must not contain an empty name".to_owned(),
            ]));
        }
        let Some(allowed) = ALLOWED_VIEWS.iter().find(|view| **view == candidate) else {
            return Err(QueryRefusal::Route(vec![
                "views must be a subset of front, top, right, iso".to_owned(),
            ]));
        };
        if views.contains(allowed) {
            return Err(QueryRefusal::Route(vec![
                "views must not repeat a name".to_owned(),
            ]));
        }
        views.push(allowed);
    }
    if views.is_empty() || views.len() > ALLOWED_VIEWS.len() {
        return Err(QueryRefusal::Route(vec![format!(
            "views must select between 1 and {} names",
            ALLOWED_VIEWS.len()
        )]));
    }
    Ok(views)
}

/// Parse the `plane` and `offset` query of the section route.
///
/// # Errors
///
/// Returns the bounded 422 details of an unusable section request.
fn parse_section_query(query: Option<&str>) -> Result<(String, Option<f64>), QueryRefusal> {
    const ALLOWED_PLANES: [&str; 3] = ["XY", "XZ", "YZ"];
    let mut plane = "XZ".to_owned();
    let mut offset: Option<f64> = None;
    if let Some(raw) = first_query_value(query, "plane") {
        if raw.chars().count() > 2 {
            return Err(QueryRefusal::Framework(vec![
                "query.plane: string_too_long: String should have at most 2 characters".to_owned(),
            ]));
        }
        let candidate = raw.trim().to_ascii_uppercase();
        if !ALLOWED_PLANES.contains(&candidate.as_str()) {
            return Err(QueryRefusal::Route(vec![
                "plane must be one of XY, XZ, or YZ".to_owned(),
            ]));
        }
        plane = candidate;
    }
    if let Some(raw) = first_query_value(query, "offset") {
        let Ok(value) = raw.trim().parse::<f64>() else {
            return Err(QueryRefusal::Framework(vec![
                "query.offset: float_parsing: Input should be a valid number, unable to parse string as a number"
                    .to_owned(),
            ]));
        };
        if !value.is_finite() || value.abs() > 10_000.0 {
            return Err(QueryRefusal::Route(vec![
                "offset must be finite and within +/-10000 mm".to_owned(),
            ]));
        }
        offset = Some(value);
    }
    Ok((plane, offset))
}

/// The first value of one query parameter, like the framework's scalar query
/// binding.
fn first_query_value(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == name {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ------------------------------------------------------------------- responses

/// A JSON success body.
fn json_response(status: u16, document: &Value) -> WorkerResponse {
    let body = serde_json::to_vec(document).unwrap_or_else(|_| internal_body());
    WorkerResponse {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

/// A framework validation refusal: the Pydantic model order.
fn validation_response(error: FlatError) -> WorkerResponse {
    let body = error
        .ordered_body()
        .to_bytes()
        .unwrap_or_else(|_| internal_body());
    WorkerResponse {
        status: error.status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

/// A route refusal: the sorted order with ASCII escaping.
fn sorted_error(
    status: u16,
    code: &'static str,
    message: &'static str,
    details: Vec<String>,
) -> WorkerResponse {
    let error = FlatError::new(status, code, message, details);
    let body = error
        .sorted_body()
        .to_sorted_ascii_bytes()
        .unwrap_or_else(|_| internal_body());
    WorkerResponse {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

/// A route refusal with an owned message.
fn sorted_error_owned(
    status: u16,
    code: &str,
    message: &str,
    details: Vec<String>,
) -> WorkerResponse {
    let body = Json::object(vec![
        ("code", Json::String(code.to_owned())),
        ("message", Json::String(message.to_owned())),
        (
            "details",
            Json::Array(details.into_iter().map(Json::String).collect()),
        ),
    ])
    .to_sorted_ascii_bytes()
    .unwrap_or_else(|_| internal_body());
    WorkerResponse {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

/// The refusal of a request body that could not be parsed.
fn request_parse_failure(error: RequestParseError) -> WorkerResponse {
    let (status, code, message, details) = match error {
        RequestParseError::Missing => (
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec!["body: missing: Field required".to_owned()],
        ),
        RequestParseError::ModelAttributes => (
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec![
                "body: model_attributes_type: Input should be a valid dictionary or object to extract fields from"
                    .to_owned(),
            ],
        ),
        RequestParseError::InvalidJson(offset) => (
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec![format!("body.{offset}: json_invalid: JSON decode error")],
        ),
        RequestParseError::Rejected => (
            400,
            "request_rejected",
            REQUEST_REJECTED_MESSAGE,
            vec!["There was an error parsing the body".to_owned()],
        ),
    };
    validation_response(FlatError::new(status, code, message, details))
}

/// The refusal of a design turn that could not be completed.
fn design_failure(error: &DesignError, design_id: Option<String>) -> WorkerResponse {
    match error {
        DesignError::Rejected(reason) => sorted_error(
            422,
            "design_rejected",
            "the design could not be compiled or measured",
            vec![crate::errors::truncate_chars(reason, 256)],
        ),
        DesignError::NotFound(reason) => {
            let detail = match design_id {
                Some(design_id) => {
                    format!(
                        "design_id={}",
                        crate::errors::truncate_chars(&design_id, 64)
                    )
                }
                None => reason.clone(),
            };
            sorted_error(
                404,
                "design_not_found",
                "no design is stored under that id",
                vec![detail],
            )
        }
        DesignError::Invalid(reason) => sorted_error(
            422,
            "invalid_request",
            VALIDATION_MESSAGE,
            vec![crate::errors::truncate_chars(reason, 256)],
        ),
        DesignError::Unavailable(reason) => sorted_error(
            503,
            "design_unavailable",
            "the design pipeline requires the native OpenCascade kernel",
            vec![crate::errors::truncate_chars(reason, 256)],
        ),
        DesignError::ProviderUnavailable(provider) => {
            let mut details = vec![crate::errors::truncate_chars(provider.message(), 256)];
            let reason = provider.reason();
            if !reason.is_empty() && reason != provider.message() {
                details.push(crate::errors::truncate_chars(
                    &format!("reason: {reason}"),
                    256,
                ));
            }
            let available = if provider.available().is_empty() {
                "none".to_owned()
            } else {
                provider.available().join(", ")
            };
            details.push(crate::errors::truncate_chars(
                &format!("available: {available}"),
                256,
            ));
            sorted_error_owned(
                503,
                "provider_unavailable",
                &format!("the {} CAD provider is not available", provider.provider()),
                details,
            )
        }
        DesignError::StepImport {
            code,
            message,
            detail,
        } => {
            let status = if *code == "payload_too_large" {
                413
            } else {
                422
            };
            sorted_error(
                status,
                code,
                message,
                vec![crate::errors::truncate_chars(detail, 256)],
            )
        }
        DesignError::Internal(reason) => {
            eprintln!("unhandled studio failure: {reason}");
            internal_failure()
        }
        DesignError::Journal(_) => journal_failure(),
    }
}

/// The refusal of a journal that could not record the turn.
fn journal_failure() -> WorkerResponse {
    sorted_error(
        500,
        "journal_unavailable",
        "the Studio journal could not record this turn",
        Vec::new(),
    )
}

/// The internal failure body.
fn internal_failure() -> WorkerResponse {
    WorkerResponse {
        status: 500,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: internal_body(),
    }
}

fn internal_body() -> Vec<u8> {
    FlatError::new(500, "internal_error", INTERNAL_ERROR_MESSAGE, Vec::new())
        .ordered_body()
        .to_bytes()
        .unwrap_or_else(|_| {
            b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}".to_vec()
        })
}
