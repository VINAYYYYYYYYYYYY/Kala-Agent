//! The parked native primitive backend (ADR-0010).
//!
//! ADR-0010 parks the native Rust geometry crates: they are kept in the
//! tree, they are not deleted, and they are **not** on the default path.
//! This module is the parked alternative behind the non-default
//! `native-primitives` feature. It serves the three analytic primitive
//! routes from the reviewed `kala-geometry` mesh builder and the reviewed
//! `kala-step` analytic B-rep writer.
//!
//! The design, journal, intent, import and CAD-provider routes have no
//! native implementation. They are answered by the supervised Python
//! geometry worker, which is the only backend that can compile a design.
//! The parked backend therefore refuses them instead of inventing an answer.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::errors::{FlatError, VALIDATION_MESSAGE};
use crate::geometry::{
    GeometryRouteError, STEP_UNAVAILABLE_MESSAGE, StepExporter, build_mesh, preview_body, stl_bytes,
};
use crate::geometry_backend::{
    BackendError, BackendFuture, BackendHealth, GeometryBackend, WorkerResponse,
};
use crate::integrations::integration_by_id;
use crate::schemas::{
    ExportFormat, PrimitiveKind, RequestParseError, parse_json_body, validate_export,
    validate_handoff, validate_preview,
};

/// The parked native primitive backend.
pub struct NativePrimitivesBackend {
    exporter: Arc<dyn StepExporter>,
    workdir: PathBuf,
}

impl NativePrimitivesBackend {
    /// Build the backend over one STEP exporter and one work directory.
    #[must_use]
    pub fn new(exporter: Arc<dyn StepExporter>, workdir: PathBuf) -> Self {
        Self { exporter, workdir }
    }
}

impl GeometryBackend for NativePrimitivesBackend {
    fn health(&self) -> BackendFuture<'_, BackendHealth> {
        Box::pin(async move {
            let capability = self.exporter.capability();
            BackendHealth {
                state: "ready".to_owned(),
                version: Some(crate::server::STUDIO_VERSION.to_owned()),
                design_available: false,
                step_available: capability.available,
            }
        })
    }

    fn preview<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(validation_failure(error)),
            };
            let request = match validate_preview(&map) {
                Ok(request) => request,
                Err(error) => return Ok(validation_response(error)),
            };
            let mesh = match build_mesh(request.kind, request.parameters.as_ref()) {
                Ok(mesh) => mesh,
                Err(GeometryRouteError::Invalid(error)) => {
                    return Ok(error_response(FlatError::new(
                        422,
                        "invalid_request",
                        VALIDATION_MESSAGE,
                        vec![crate::errors::truncate_chars(error.message(), 256)],
                    )));
                }
                Err(GeometryRouteError::Overflow) => {
                    return Ok(WorkerResponse {
                        status: 500,
                        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                        body: b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}".to_vec(),
                    });
                }
            };
            match preview_body(&mesh) {
                Ok(body) => Ok(WorkerResponse {
                    status: 200,
                    headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                    body,
                }),
                Err(_) => Ok(internal_failure()),
            }
        })
    }

    fn export<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(validation_failure(error)),
            };
            if map.contains_key("design_id") {
                // A stored design lives in the worker's journal; the parked
                // native path cannot reach it and never fabricates bytes.
                return Err(BackendError::Unsupported(
                    "a stored design can only be exported by the geometry worker".to_owned(),
                ));
            }
            let request = match validate_export(&map) {
                Ok(request) => request,
                Err(error) => return Ok(validation_response(error)),
            };
            self.render(
                request.kind,
                request.parameters.as_ref(),
                request.format,
                &format!(
                    "kala-studio-{}.{}",
                    request.kind.as_str(),
                    request.format.as_str()
                ),
                None,
            )
            .await
        })
    }

    fn handoff<'a>(
        &'a self,
        body: &'a [u8],
        content_type: Option<&'a str>,
    ) -> BackendFuture<'a, Result<WorkerResponse, BackendError>> {
        Box::pin(async move {
            let map = match parse_json_body(body, content_type) {
                Ok(map) => map,
                Err(error) => return Ok(validation_failure(error)),
            };
            let request = match validate_handoff(&map) {
                Ok(request) => request,
                Err(error) => return Ok(validation_response(error)),
            };
            let Some(record) = integration_by_id(&request.integration_id) else {
                return Ok(error_response(FlatError::new(
                    404,
                    "unknown_integration",
                    "the requested integration is not in the registry",
                    vec![format!(
                        "unknown integration: {}",
                        crate::errors::truncate_chars(&request.integration_id, 128)
                    )],
                )));
            };
            if !record.capabilities.contains(&request.format.as_str()) {
                return Ok(error_response(FlatError::new(
                    409,
                    "handoff_format_unsupported",
                    "the integration does not list this format as a supported neutral-file capability",
                    vec![format!(
                        "integration={}; requested_format={}; supported={}",
                        crate::errors::truncate_chars(&request.integration_id, 64),
                        request.format.as_str(),
                        record.capabilities.join(",")
                    )],
                )));
            }
            self.render(
                request.kind,
                request.parameters.as_ref(),
                request.format,
                &format!("{}.{}", request.integration_id, request.format.as_str()),
                Some(&request.integration_id),
            )
            .await
        })
    }
}

impl NativePrimitivesBackend {
    async fn render(
        &self,
        kind: PrimitiveKind,
        parameters: Option<&Value>,
        format: ExportFormat,
        filename: &str,
        handoff: Option<&str>,
    ) -> Result<WorkerResponse, BackendError> {
        let mesh = match build_mesh(kind, parameters) {
            Ok(mesh) => mesh,
            Err(GeometryRouteError::Invalid(error)) => {
                return Ok(error_response(FlatError::new(
                    422,
                    "invalid_request",
                    VALIDATION_MESSAGE,
                    vec![crate::errors::truncate_chars(error.message(), 256)],
                )));
            }
            Err(GeometryRouteError::Overflow) => return Ok(internal_failure()),
        };
        let extension = format.as_str();
        let media = if format == ExportFormat::Stl {
            "model/stl"
        } else {
            "application/step"
        };
        let data = if format == ExportFormat::Stl {
            stl_bytes(&mesh)
        } else {
            match self
                .exporter
                .export_primitive(kind, parameters, &self.workdir)
            {
                Ok(data) => data,
                Err(crate::geometry::StepError::Invalid(message)) => {
                    return Ok(error_response(FlatError::new(
                        422,
                        "invalid_request",
                        VALIDATION_MESSAGE,
                        vec![crate::errors::truncate_chars(&message, 256)],
                    )));
                }
                Err(crate::geometry::StepError::Unavailable(message)) => {
                    return Ok(error_response(FlatError::new(
                        503,
                        "step_unavailable",
                        STEP_UNAVAILABLE_MESSAGE,
                        vec![crate::errors::truncate_chars(&message, 256)],
                    )));
                }
                Err(crate::geometry::StepError::Export(_)) => return Ok(internal_failure()),
            }
        };
        let mut headers = vec![
            ("content-type".to_owned(), media.to_owned()),
            (
                "content-disposition".to_owned(),
                format!("attachment; filename=\"{filename}\""),
            ),
        ];
        if let Some(handoff) = handoff {
            headers.push((
                "x-studio-handoff".to_owned(),
                format!("{handoff};format={extension};bytes={}", data.len()),
            ));
        }
        Ok(WorkerResponse {
            status: 200,
            headers,
            body: data,
        })
    }
}

/// A framework validation refusal.
///
/// The source answers a rejected request model with the Pydantic model's own
/// field order (`code`, `message`, `details`); a route refusal uses the
/// sorted `json.dumps` form. The two are different byte sequences.
fn validation_response(error: FlatError) -> WorkerResponse {
    let body = error
        .ordered_body()
        .to_bytes()
        .unwrap_or_else(|_| b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}".to_vec());
    WorkerResponse {
        status: error.status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

fn error_response(error: FlatError) -> WorkerResponse {
    let body = error
        .sorted_body()
        .to_sorted_ascii_bytes()
        .unwrap_or_else(|_| b"{\"code\":\"internal_error\",\"details\":[],\"message\":\"the studio request could not be completed\"}".to_vec());
    WorkerResponse {
        status: error.status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
    }
}

fn internal_failure() -> WorkerResponse {
    WorkerResponse {
        status: 500,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: b"{\"code\":\"internal_error\",\"message\":\"the studio request could not be completed\",\"details\":[]}".to_vec(),
    }
}

fn validation_failure(error: RequestParseError) -> WorkerResponse {
    let details = match error {
        RequestParseError::Missing => vec!["body: missing: Field required".to_owned()],
        RequestParseError::ModelAttributes => vec![
            "body: model_attributes_type: Input should be a valid dictionary or object to extract fields from"
                .to_owned(),
        ],
        RequestParseError::InvalidJson(offset) => {
            vec![format!("body.{offset}: json_invalid: JSON decode error")]
        }
        RequestParseError::Rejected => vec!["There was an error parsing the body".to_owned()],
    };
    let (status, code, message) = if matches!(error, RequestParseError::Rejected) {
        (
            400,
            "request_rejected",
            crate::errors::REQUEST_REJECTED_MESSAGE,
        )
    } else {
        (422, "invalid_request", VALIDATION_MESSAGE)
    };
    validation_response(FlatError::new(status, code, message, details))
}
