//! Error handling for the local Kala HTTP adapter.
//!
//! Every failure leaves this process as a small, stable JSON body: an error
//! code, a bounded human message, and optional sanitized details. No
//! traceback, no exception chain, no submitted value, and no environment
//! content is ever echoed back, so a request that carries credential-shaped
//! text cannot leak it through an error.
//!
//! The security headers are attached by the transport as well, because a
//! server error answer can be produced from outside the middleware stack.
//!
//! The Python source installs these as framework exception handlers. This
//! crate exposes the same decisions as plain functions, so the transport (and
//! any later composition) can call them directly and get the same status and
//! the same bytes.

use crate::envelope::{ErrorResponse, error_response, truncate_chars};

/// The status a rejected request payload answers with.
pub const UNPROCESSABLE_CONTENT: u16 = 422;

/// The message an unknown route answers with.
pub const UNKNOWN_ROUTE_MESSAGE: &str = "the requested resource does not exist";

/// The message an unsupported method answers with.
pub const METHOD_NOT_ALLOWED_MESSAGE: &str =
    "the requested method is not allowed for this resource";

/// The message an unexpected failure answers with.
pub const INTERNAL_ERROR_MESSAGE: &str = "the request could not be completed";

/// The message a rejected request payload answers with.
pub const VALIDATION_MESSAGE: &str = "the request body or parameters are invalid";

/// The message an oversized request body answers with.
pub const PAYLOAD_TOO_LARGE_MESSAGE: &str = "the request body exceeds the configured size limit";

/// The message a rejected request answers with when it carries no detail.
pub const REQUEST_REJECTED_MESSAGE: &str = "the request was rejected";

/// The largest number of code points of a rejected request's detail the
/// handler keeps.
///
/// The Python source cuts the detail of a rejected request with
/// `str(error.detail)[:128]` before it builds the body.
pub const MAX_REJECTED_DETAIL_LENGTH: usize = 128;

/// The failure taxonomy the composition root reports.
///
/// The Python source raises a class per failure and the error handlers map
/// the class onto a status. The taxonomy is the part of the composition root
/// that the transport needs, so it lives here as an enumeration of the same
/// failures with the same codes and the same statuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationErrorKind {
    /// A domain package that is not registered.
    UnknownDomain,
    /// A revision that is not stored.
    UnknownRevision,
    /// A prepared commit that is not stored.
    UnknownPreparedRevision,
    /// A workflow that is not stored.
    UnknownWorkflow,
    /// A provenance record that cannot be produced.
    ProvenanceUnavailable,
    /// A checkpoint that is not pending.
    CheckpointNotPending,
    /// A commit that was rejected.
    RevisionCommitRejected,
    /// A prepared commit that would not extend the current head.
    RevisionForkRefused,
    /// A design head that moved between preparation and write.
    RevisionHeadMoved,
    /// A checkpoint decision that was denied.
    CheckpointDecisionDenied,
    /// A checkpoint digest that no longer describes the state.
    CheckpointDigestMismatch,
    /// A workflow that already finished.
    WorkflowFinished,
    /// Durable policy or budget state that cannot be trusted.
    CorruptKernelState,
    /// An application that was closed.
    ClosedApplication,
    /// An identifier that is not usable.
    InvalidIdentifier,
    /// An identifier this process reserves for itself.
    ReservedIdentifier,
    /// Parameters that are not usable.
    InvalidParameters,
    /// A failure the taxonomy does not name.
    Other,
}

impl ApplicationErrorKind {
    /// The stable machine readable code of this failure.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::UnknownDomain => "unknown_domain",
            Self::UnknownRevision => "unknown_revision",
            Self::UnknownPreparedRevision => "unknown_prepared_revision",
            Self::UnknownWorkflow => "unknown_workflow",
            Self::ProvenanceUnavailable => "provenance_unavailable",
            Self::CheckpointNotPending => "checkpoint_not_pending",
            Self::RevisionCommitRejected => "revision_commit_rejected",
            Self::RevisionForkRefused => "revision_fork_refused",
            Self::RevisionHeadMoved => "revision_head_moved",
            Self::CheckpointDecisionDenied => "checkpoint_denied",
            Self::CheckpointDigestMismatch => "checkpoint_digest_mismatch",
            Self::WorkflowFinished => "workflow_finished",
            Self::CorruptKernelState => "corrupt_kernel_state",
            Self::ClosedApplication => "application_closed",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::ReservedIdentifier => "reserved_identifier",
            Self::InvalidParameters => "invalid_parameters",
            Self::Other => "application_error",
        }
    }

    /// The HTTP status this failure answers with.
    #[must_use]
    pub fn status(self) -> u16 {
        match self {
            Self::UnknownDomain
            | Self::UnknownRevision
            | Self::UnknownPreparedRevision
            | Self::UnknownWorkflow
            | Self::ProvenanceUnavailable => 404,
            Self::CheckpointNotPending
            | Self::RevisionCommitRejected
            | Self::RevisionForkRefused
            | Self::RevisionHeadMoved
            | Self::CheckpointDecisionDenied
            | Self::CheckpointDigestMismatch
            | Self::WorkflowFinished => 409,
            Self::CorruptKernelState | Self::ClosedApplication => 503,
            Self::InvalidIdentifier | Self::ReservedIdentifier | Self::InvalidParameters => {
                UNPROCESSABLE_CONTENT
            }
            Self::Other => 400,
        }
    }
}

/// One failure the composition root reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationError {
    /// The failure taxonomy entry.
    pub kind: ApplicationErrorKind,
    /// The stable machine readable code.
    pub code: String,
    /// The bounded human readable message.
    pub message: String,
}

impl ApplicationError {
    /// Build a failure with the code the taxonomy names.
    #[must_use]
    pub fn new(kind: ApplicationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: kind.code().to_owned(),
            message: message.into(),
        }
    }

    /// Build a failure with an explicit code.
    #[must_use]
    pub fn with_code(
        kind: ApplicationErrorKind,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            message: message.into(),
        }
    }
}

/// One rejected request field, as the validation layer reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationFailure {
    /// The location of the rejected value, most specific part last.
    ///
    /// The Python source joins the parts with a dot and falls back to
    /// `body` when there is no location at all.
    pub location: Vec<String>,
    /// The human readable reason.
    pub message: Option<String>,
    /// The machine readable kind of the failure.
    pub kind: Option<String>,
}

/// The sanitized detail lines of a rejected request payload.
///
/// Every line is `<location>: <kind>: <message>`, the message is cut to 128
/// code points, the kind to 64, and only the first
/// [`crate::envelope::MAX_ERROR_DETAILS`] failures are reported.
#[must_use]
pub fn validation_details(failures: &[ValidationFailure]) -> Vec<String> {
    failures
        .iter()
        .take(crate::envelope::MAX_ERROR_DETAILS)
        .map(|failure| {
            let location = if failure.location.is_empty() {
                "body".to_owned()
            } else {
                failure.location.join(".")
            };
            let message =
                truncate_chars(failure.message.as_deref().unwrap_or("invalid value"), 128);
            let kind = truncate_chars(failure.kind.as_deref().unwrap_or("invalid"), 64);
            format!("{location}: {kind}: {message}")
        })
        .collect()
}

/// Return a sanitized 422 for a rejected request payload.
///
/// `failures` is `None` when the rejection did not come from the request
/// validator, which the Python source answers without any detail.
#[must_use]
pub fn handle_request_validation(failures: Option<&[ValidationFailure]>) -> ErrorResponse {
    match failures {
        None => error_response("invalid_request", VALIDATION_MESSAGE, &[]),
        Some(failures) => {
            let details = validation_details(failures);
            let borrowed: Vec<&str> = details.iter().map(String::as_str).collect();
            error_response("invalid_request", VALIDATION_MESSAGE, &borrowed)
        }
    }
}

/// Map a composition root failure onto a bounded HTTP status.
#[must_use]
pub fn handle_application_error(error: &ApplicationError) -> (u16, ErrorResponse) {
    (
        error.kind.status(),
        error_response(&error.code, &error.message, &[]),
    )
}

/// Return a sanitized body for framework and routing errors.
///
/// The Python source answers a `404`, a `405` and a `413` with their own
/// codes, a status from `400` up to (but not including) `500` with the
/// `request_rejected` code and the detail cut to
/// [`MAX_REJECTED_DETAIL_LENGTH`] code points, and every other status with the
/// `internal_error` code and the status the failure carried.
///
/// The status of a rejected request is part of the contract of this API, so it
/// is answered as it was raised: a `409` stays a `409` and a `429` stays a
/// `429`. A detail that carries nothing answers with
/// [`REQUEST_REJECTED_MESSAGE`], exactly as the Python source does with its
/// `or` fallback.
#[must_use]
pub fn handle_http_exception(status: u16, detail: &str) -> (u16, ErrorResponse) {
    match status {
        404 => (404, error_response("not_found", UNKNOWN_ROUTE_MESSAGE, &[])),
        405 => (
            405,
            error_response("method_not_allowed", METHOD_NOT_ALLOWED_MESSAGE, &[]),
        ),
        413 => (
            413,
            error_response("payload_too_large", PAYLOAD_TOO_LARGE_MESSAGE, &[]),
        ),
        code if (400..500).contains(&code) => {
            let detail = truncate_chars(detail, MAX_REJECTED_DETAIL_LENGTH);
            let message = if detail.is_empty() {
                REQUEST_REJECTED_MESSAGE.to_owned()
            } else {
                detail
            };
            (code, error_response("request_rejected", &message, &[]))
        }
        code => (
            code,
            error_response("internal_error", INTERNAL_ERROR_MESSAGE, &[]),
        ),
    }
}

/// Fail closed: report the failure type locally and return a bare 500.
#[must_use]
pub fn handle_unexpected_error() -> (u16, ErrorResponse) {
    (
        500,
        error_response("internal_error", INTERNAL_ERROR_MESSAGE, &[]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_table_matches_the_python_source() {
        assert_eq!(ApplicationErrorKind::UnknownDomain.status(), 404);
        assert_eq!(ApplicationErrorKind::ProvenanceUnavailable.status(), 404);
        assert_eq!(ApplicationErrorKind::CheckpointNotPending.status(), 409);
        assert_eq!(ApplicationErrorKind::RevisionHeadMoved.status(), 409);
        assert_eq!(ApplicationErrorKind::WorkflowFinished.status(), 409);
        assert_eq!(ApplicationErrorKind::CorruptKernelState.status(), 503);
        assert_eq!(ApplicationErrorKind::ClosedApplication.status(), 503);
        assert_eq!(ApplicationErrorKind::InvalidIdentifier.status(), 422);
        assert_eq!(ApplicationErrorKind::ReservedIdentifier.status(), 422);
        assert_eq!(ApplicationErrorKind::InvalidParameters.status(), 422);
        assert_eq!(ApplicationErrorKind::Other.status(), 400);
    }

    #[test]
    fn validation_details_are_formatted_like_the_python_source() {
        let failures = vec![
            ValidationFailure {
                location: vec!["query".to_owned(), "design_id".to_owned()],
                message: Some("Field required".to_owned()),
                kind: Some("missing".to_owned()),
            },
            ValidationFailure {
                location: vec!["body".to_owned(), "1".to_owned()],
                message: Some("JSON decode error".to_owned()),
                kind: Some("json_invalid".to_owned()),
            },
            ValidationFailure {
                location: Vec::new(),
                message: None,
                kind: None,
            },
        ];
        assert_eq!(
            validation_details(&failures),
            vec![
                "query.design_id: missing: Field required",
                "body.1: json_invalid: JSON decode error",
                "body: invalid: invalid value",
            ]
        );
    }

    #[test]
    fn validation_details_are_bounded() {
        let failures = vec![ValidationFailure {
            location: vec!["body".to_owned()],
            message: Some("x".repeat(400)),
            kind: Some("y".repeat(200)),
        }];
        let details = validation_details(&failures);
        assert_eq!(details[0].len(), "body: ".len() + 64 + ": ".len() + 128);
    }

    #[test]
    fn http_exceptions_map_onto_the_python_codes() {
        let (status, body) = handle_http_exception(404, "Not Found");
        assert_eq!(status, 404);
        assert_eq!(body.error.code, "not_found");
        let (status, body) = handle_http_exception(405, "Method Not Allowed");
        assert_eq!(status, 405);
        assert_eq!(body.error.code, "method_not_allowed");
        let (status, body) = handle_http_exception(413, "");
        assert_eq!(status, 413);
        assert_eq!(body.error.code, "payload_too_large");
        let (status, body) = handle_http_exception(301, "moved");
        assert_eq!(status, 301);
        assert_eq!(body.error.code, "internal_error");
        assert_eq!(body.error.message, INTERNAL_ERROR_MESSAGE);
        // Every status of 400 or more that is not 404, 405 or 413 is a
        // rejected request: it keeps its own status and reports the detail.
        for status in [400, 401, 403, 409, 418, 429, 499] {
            let (answered, body) = handle_http_exception(status, "a refused request");
            assert_eq!(answered, status, "{status}");
            assert_eq!(body.error.code, "request_rejected", "{status}");
            assert_eq!(body.error.message, "a refused request", "{status}");
        }
        // A status of 500 or more is a server failure: the detail is not
        // echoed and the status is answered as it was raised.
        for status in [500, 501, 502, 503, 504] {
            let (answered, body) = handle_http_exception(status, "an internal detail");
            assert_eq!(answered, status, "{status}");
            assert_eq!(body.error.code, "internal_error", "{status}");
            assert_eq!(body.error.message, INTERNAL_ERROR_MESSAGE, "{status}");
        }
    }

    #[test]
    fn a_rejected_request_keeps_at_most_128_code_points_of_its_detail() {
        let (status, body) = handle_http_exception(400, &"d".repeat(200));
        assert_eq!(status, 400);
        assert_eq!(body.error.message, "d".repeat(128));
        let (_, body) = handle_http_exception(400, &"é".repeat(200));
        assert_eq!(body.error.message.chars().count(), 128);
    }

    #[test]
    fn a_rejected_request_without_a_detail_answers_with_the_generic_message() {
        let (status, body) = handle_http_exception(400, "");
        assert_eq!(status, 400);
        assert_eq!(body.error.code, "request_rejected");
        assert_eq!(body.error.message, REQUEST_REJECTED_MESSAGE);
    }
}
