//! The routes of the local Kala HTTP adapter.
//!
//! This module is the port of `kala_agent.api.app` and of
//! `kala_agent.api.dependencies`. It is transport only: it holds no
//! engineering rule, writes no database row, calls no provider and never
//! opens an outbound connection. It matches the request, validates the path
//! parameters, the query parameters and the body, hands the work to the
//! composition root and writes the answer.
//!
//! # The dependency wiring
//!
//! The Python source resolves the composition root from the state of the
//! running ASGI application, so a test can inject an application and two
//! applications can live in one process. The native port keeps that property
//! with a port trait: the router holds an [`Application`], and the process
//! that composes the root hands it over.
//!
//! # The router
//!
//! The route table, the order of the routes, the trailing slash redirect, the
//! `404` of an unknown route and the `405` of a route that exists for another
//! method are the behaviour of the framework the Python source runs on. They
//! are reproduced here, because a caller sees them and not the framework.
//!
//! # The answers
//!
//! * a route that answers writes the document of the composition root with
//!   the bytes FastAPI writes: the field order of the model, the compact
//!   separators and the UTF-8 text;
//! * a request that is not valid answers a sanitized `422` whose detail lines
//!   are the ones the Python source renders;
//! * a failure of the composition root answers the status the Python source
//!   maps it onto, with the code and the message of the failure.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};
use hyper::{Request, Response, StatusCode};
use kala_contracts::WireValue;
use num_bigint::BigInt;

use crate::errors::{ApplicationError, ValidationFailure};
use crate::jsoninput;
use crate::middleware::raw_headers;
use crate::patterns::{DESIGN_ID_PATTERN, IDENTIFIER_PATTERN};
use crate::schemas::{
    ApiModel, CheckpointDecisionRequest, CreateRevisionRequest, EvaluateDomainRequest,
    PrepareRevisionRequest, RunMechanicalWorkflowRequest,
};
use crate::server::{self, BoxFuture, Router};

/// The version the API reports.
pub const API_VERSION: &str = "1.0.0";

/// The prefix every route of the API carries.
pub const API_PREFIX: &str = "/api/v1";

/// The disclaimer the health answer carries.
pub const LOCAL_DISCLAIMER: &str = "Bounded surrogate reference arithmetic only; not CAD, \
CAE, a code check, or an engineering certification.";

/// The server the OpenAPI document declares.
pub const LOOPBACK_SERVER: (&str, &str) = ("http://127.0.0.1:8000", "Local loopback only");

/// The tags of the OpenAPI document, in the order the Python source declares
/// them.
pub const TAGS: [(&str, &str); 5] = [
    ("health", "Process liveness"),
    (
        "domains",
        "Registered domain packages and their evaluations",
    ),
    (
        "revisions",
        "Immutable design revisions and their provenance",
    ),
    (
        "workflows",
        "Deterministic reference workflows and policy checkpoints",
    ),
    ("journal", "Append-only event journal"),
];

/// The largest number of digits an integer query parameter may carry.
///
/// CPython refuses to convert a decimal string with more digits than this, and
/// Pydantic reports the same refusal with its own kind.
pub const MAX_INTEGER_DIGITS: usize = 4_300;

/// The bound of the `limit` query parameter.
pub const MAX_LIST_LIMIT: i64 = crate::schemas::MAX_LIST_LIMIT as i64;

/// The default of the `limit` query parameter.
pub const DEFAULT_LIST_LIMIT: i64 = crate::schemas::DEFAULT_LIST_LIMIT as i64;

/// The bound of the `after_sequence` query parameter.
pub const MAX_AFTER_SEQUENCE: i64 = crate::schemas::MAX_AFTER_SEQUENCE as i64;

/// One created resource: its document and the identity its `location` header
/// names.
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    /// The document the route answers with.
    pub document: WireValue,
    /// The identifier the `location` header names.
    pub identifier: String,
}

/// One mechanical reference run: its document, the workflow it belongs to and
/// whether this call started it.
#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalRun {
    /// The document the route answers with.
    pub document: WireValue,
    /// The workflow the run belongs to.
    pub workflow_id: String,
    /// Whether this call started the run.
    pub started: bool,
}

/// The query of the journal route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventQuery {
    /// The exclusive lower bound of the sequence.
    pub after_sequence: i64,
    /// The number of events the page carries at most.
    pub limit: i64,
    /// The workflow the events belong to, if the caller names one.
    pub workflow_id: Option<String>,
    /// The type of the events, if the caller names one.
    pub event_type: Option<String>,
    /// The aggregate the events belong to, if the caller names one.
    pub aggregate_id: Option<String>,
}

/// The composition root the routes call.
///
/// Every method answers the document of the route, or the failure the
/// composition root reports. The trait is the native form of the
/// `ApplicationDependency` of the Python source: the router holds the
/// composition root and the handlers call it.
pub trait Application: Send + Sync + 'static {
    /// The facts of the health route.
    fn health(&self) -> WireValue;

    /// The registered domain packages and their workflows.
    fn list_domains(&self) -> WireValue;

    /// One registered domain package.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports for a package that is
    /// not registered.
    fn read_domain(&self, domain: &str) -> Result<WireValue, ApplicationError>;

    /// One deterministic reference design evaluation.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn evaluate_domain(
        &self,
        domain: &str,
        request: &EvaluateDomainRequest,
    ) -> Result<WireValue, ApplicationError>;

    /// One immutable design revision.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn create_revision(&self, request: &CreateRevisionRequest)
    -> Result<Created, ApplicationError>;

    /// One prepared revision commit.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn prepare_revision_commit(
        &self,
        request: &PrepareRevisionRequest,
    ) -> Result<Created, ApplicationError>;

    /// One prepared or decided revision commit.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn read_revision_commit(&self, request_id: &str) -> Result<WireValue, ApplicationError>;

    /// The revision one approved commit writes.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn approve_revision_commit(
        &self,
        request_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError>;

    /// The rejection one prepared commit records.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn reject_revision_commit(
        &self,
        request_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError>;

    /// The stored revisions of one design.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn list_revisions(&self, design_id: &str, limit: i64) -> Result<WireValue, ApplicationError>;

    /// One stored revision.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn read_revision(&self, revision_id: &str) -> Result<WireValue, ApplicationError>;

    /// One mechanical reference run.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn run_mechanical_reference(
        &self,
        request: &RunMechanicalWorkflowRequest,
    ) -> Result<MechanicalRun, ApplicationError>;

    /// The state of one stored workflow.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn read_workflow_state(&self, workflow_id: &str) -> Result<WireValue, ApplicationError>;

    /// The recorded checkpoints of one workflow.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn list_checkpoints(&self, workflow_id: &str) -> Result<WireValue, ApplicationError>;

    /// The outcome of an approved checkpoint.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn approve_checkpoint(
        &self,
        workflow_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError>;

    /// The outcome of a rejected checkpoint.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn reject_checkpoint(
        &self,
        workflow_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError>;

    /// One bounded page of journal events.
    ///
    /// # Errors
    ///
    /// Returns the failure the composition root reports.
    fn list_events(&self, query: &EventQuery) -> Result<WireValue, ApplicationError>;

    /// Whether the composition root has been released.
    fn closed(&self) -> bool;

    /// Release the composition root.
    ///
    /// The lifespan of the Python source closes the root it created when the
    /// server stops, and leaves a root it was given alone.
    fn close(&self);
}

/// The lifecycle of one launch.
///
/// The Python source closes the application it created when the server stops
/// and leaves an application it was given alone. The native port keeps the
/// same rule: the owner closes, a borrower does not.
#[derive(Clone)]
pub struct Lifespan {
    /// The composition root of the launch.
    application: Arc<dyn Application>,
    /// Whether the launch owns the composition root.
    owns: bool,
}

impl Lifespan {
    /// Build the lifecycle of one launch.
    ///
    /// `owns` is `true` when the launch created the composition root and
    /// `false` when it was given one.
    #[must_use]
    pub fn new(application: Arc<dyn Application>, owns: bool) -> Self {
        Self { application, owns }
    }

    /// Whether the launch owns the composition root.
    #[must_use]
    pub fn owns(&self) -> bool {
        self.owns
    }

    /// Release the composition root, if the launch owns it.
    pub fn shutdown(&self) {
        if self.owns && !self.application.closed() {
            self.application.close();
        }
    }
}

/// One route of the API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    /// `GET /health`.
    Health,
    /// `GET /api/v1/domains`.
    ListDomains,
    /// `GET /api/v1/domains/{domain}`.
    ReadDomain,
    /// `POST /api/v1/domains/{domain}/evaluations`.
    EvaluateDomain,
    /// `POST /api/v1/revisions`.
    CreateRevision,
    /// `POST /api/v1/revision-commits`.
    PrepareRevisionCommit,
    /// `GET /api/v1/revision-commits/{request_id}`.
    ReadRevisionCommit,
    /// `POST /api/v1/revision-commits/{request_id}/approve`.
    ApproveRevisionCommit,
    /// `POST /api/v1/revision-commits/{request_id}/reject`.
    RejectRevisionCommit,
    /// `GET /api/v1/revisions`.
    ListRevisions,
    /// `GET /api/v1/revisions/{revision_id}`.
    ReadRevision,
    /// `POST /api/v1/workflows/mechanical/reference`.
    RunMechanicalReference,
    /// `GET /api/v1/workflows/{workflow_id}`.
    ReadWorkflowState,
    /// `GET /api/v1/workflows/{workflow_id}/checkpoints`.
    ListCheckpoints,
    /// `POST /api/v1/workflows/{workflow_id}/checkpoints/approve`.
    ApproveCheckpoint,
    /// `POST /api/v1/workflows/{workflow_id}/checkpoints/reject`.
    RejectCheckpoint,
    /// `GET /api/v1/events`.
    ListEvents,
}

/// The route table, in the order the Python source registers the routes.
const ROUTES: &[(&str, &[&str], Route)] = &[
    ("/health", &["GET"], Route::Health),
    ("/api/v1/domains", &["GET"], Route::ListDomains),
    ("/api/v1/domains/{domain}", &["GET"], Route::ReadDomain),
    (
        "/api/v1/domains/{domain}/evaluations",
        &["POST"],
        Route::EvaluateDomain,
    ),
    ("/api/v1/revisions", &["POST"], Route::CreateRevision),
    (
        "/api/v1/revision-commits",
        &["POST"],
        Route::PrepareRevisionCommit,
    ),
    (
        "/api/v1/revision-commits/{request_id}",
        &["GET"],
        Route::ReadRevisionCommit,
    ),
    (
        "/api/v1/revision-commits/{request_id}/approve",
        &["POST"],
        Route::ApproveRevisionCommit,
    ),
    (
        "/api/v1/revision-commits/{request_id}/reject",
        &["POST"],
        Route::RejectRevisionCommit,
    ),
    ("/api/v1/revisions", &["GET"], Route::ListRevisions),
    (
        "/api/v1/revisions/{revision_id}",
        &["GET"],
        Route::ReadRevision,
    ),
    (
        "/api/v1/workflows/mechanical/reference",
        &["POST"],
        Route::RunMechanicalReference,
    ),
    (
        "/api/v1/workflows/{workflow_id}",
        &["GET"],
        Route::ReadWorkflowState,
    ),
    (
        "/api/v1/workflows/{workflow_id}/checkpoints",
        &["GET"],
        Route::ListCheckpoints,
    ),
    (
        "/api/v1/workflows/{workflow_id}/checkpoints/approve",
        &["POST"],
        Route::ApproveCheckpoint,
    ),
    (
        "/api/v1/workflows/{workflow_id}/checkpoints/reject",
        &["POST"],
        Route::RejectCheckpoint,
    ),
    ("/api/v1/events", &["GET"], Route::ListEvents),
];

/// The router of the local Kala HTTP adapter.
#[derive(Clone)]
pub struct Routes {
    /// The composition root the routes call.
    application: Arc<dyn Application>,
}

impl Routes {
    /// Build the router of one launch.
    #[must_use]
    pub fn new(application: Arc<dyn Application>) -> Self {
        Self { application }
    }

    /// Answer one request.
    fn answer(&self, request: Request<Full<Bytes>>) -> Response<Full<Bytes>> {
        let method = request.method().as_str().to_owned();
        let uri = request.uri().clone();
        let headers = raw_headers(request.headers());
        let body = request.into_body().into_inner().unwrap_or_default();

        // The framework decodes the path before the application sees it.
        let path = percent_decode(uri.path());
        let query = uri.query().unwrap_or("").to_owned();

        // A closed composition root is refused before any route runs.
        if self.application.closed() {
            return error_response(&ApplicationError::new(
                crate::errors::ApplicationErrorKind::ClosedApplication,
                "the local application is closed",
            ));
        }

        // The route is matched in the order the routes are declared. The
        // first route that matches the path and the method answers, and the
        // first route that matches only the path answers a refusal.
        let mut partial: Option<Route> = None;
        for (pattern, methods, route) in ROUTES {
            let Some(parameters) = match_path(pattern, &path) else {
                continue;
            };
            if methods.contains(&method.as_str()) {
                return self.dispatch(
                    *route,
                    &parameters,
                    &RequestParts {
                        query: &query,
                        headers: &headers,
                        body: &body,
                    },
                );
            }
            if partial.is_none() {
                partial = Some(*route);
            }
        }
        if partial.is_some() {
            return method_not_allowed();
        }

        // A path that ends with a slash, or that misses one, is redirected to
        // the other spelling when that spelling names a route.
        if let Some(redirect) = redirect_path(&path) {
            for (pattern, _, _) in ROUTES {
                if match_path(pattern, &redirect).is_some() {
                    return redirect_response(&redirect, &query, &headers);
                }
            }
        }

        not_found()
    }

    /// Run the handler of one route.
    fn dispatch(
        &self,
        route: Route,
        parameters: &Parameters,
        request: &RequestParts<'_>,
    ) -> Response<Full<Bytes>> {
        match self.handle(route, parameters, request) {
            Ok(response) => response,
            Err(failure) => failure.into_response(),
        }
    }

    /// The handler of one route.
    ///
    /// Every failure of the request is collected before the work starts, so
    /// an answer carries the failures of the path parameters, of the query
    /// parameters and of the body together, in that order, exactly as the
    /// framework collects them.
    fn handle(
        &self,
        route: Route,
        parameters: &Parameters,
        request: &RequestParts<'_>,
    ) -> Result<Response<Full<Bytes>>, Failure> {
        let mut failures = Failures::default();
        match route {
            Route::Health => Ok(document_response(200, &self.application.health())?),
            Route::ListDomains => Ok(document_response(200, &self.application.list_domains())?),
            Route::ReadDomain => {
                let domain = parameters.identifier("domain", &mut failures);
                failures.finish()?;
                let domain = domain.expect("the path parameter was read");
                Ok(document_response(
                    200,
                    &self.application.read_domain(&domain)?,
                )?)
            }
            Route::EvaluateDomain => {
                let domain = parameters.identifier("domain", &mut failures);
                let payload = optional_body_model::<EvaluateDomainRequest>(request, &mut failures);
                failures.finish()?;
                let domain = domain.expect("the path parameter was read");
                let payload = payload.expect("the body was read");
                Ok(document_response(
                    200,
                    &self.application.evaluate_domain(&domain, &payload)?,
                )?)
            }
            Route::CreateRevision => {
                let payload = body_model::<CreateRevisionRequest>(request, &mut failures);
                failures.finish()?;
                let payload = payload.expect("the body was read");
                let created = self.application.create_revision(&payload)?;
                let mut response = document_response(201, &created.document)?;
                set_location(
                    &mut response,
                    &format!("{API_PREFIX}/revisions/{}", created.identifier),
                );
                Ok(response)
            }
            Route::PrepareRevisionCommit => {
                let payload = body_model::<PrepareRevisionRequest>(request, &mut failures);
                failures.finish()?;
                let payload = payload.expect("the body was read");
                let prepared = self.application.prepare_revision_commit(&payload)?;
                let mut response = document_response(201, &prepared.document)?;
                set_location(
                    &mut response,
                    &format!("{API_PREFIX}/revision-commits/{}", prepared.identifier),
                );
                Ok(response)
            }
            Route::ReadRevisionCommit => {
                let request_id = parameters.identifier("request_id", &mut failures);
                failures.finish()?;
                let request_id = request_id.expect("the path parameter was read");
                Ok(document_response(
                    200,
                    &self.application.read_revision_commit(&request_id)?,
                )?)
            }
            Route::ApproveRevisionCommit => {
                let request_id = parameters.identifier("request_id", &mut failures);
                let payload =
                    optional_body_model::<CheckpointDecisionRequest>(request, &mut failures);
                failures.finish()?;
                let request_id = request_id.expect("the path parameter was read");
                let payload = payload.expect("the body was read");
                Ok(document_response(
                    200,
                    &self
                        .application
                        .approve_revision_commit(&request_id, &payload)?,
                )?)
            }
            Route::RejectRevisionCommit => {
                let request_id = parameters.identifier("request_id", &mut failures);
                let payload =
                    optional_body_model::<CheckpointDecisionRequest>(request, &mut failures);
                failures.finish()?;
                let request_id = request_id.expect("the path parameter was read");
                let payload = payload.expect("the body was read");
                Ok(document_response(
                    200,
                    &self
                        .application
                        .reject_revision_commit(&request_id, &payload)?,
                )?)
            }
            Route::ListRevisions => {
                let design_id =
                    query_identifier(request, "design_id", 64, DESIGN_ID_PATTERN, &mut failures);
                let limit = query_limit(request, "limit", DEFAULT_LIST_LIMIT, &mut failures);
                failures.finish()?;
                let design_id = design_id.expect("the query parameter was read");
                let limit = limit.expect("the query parameter was read");
                Ok(document_response(
                    200,
                    &self.application.list_revisions(&design_id, limit)?,
                )?)
            }
            Route::ReadRevision => {
                let revision_id = parameters.identifier("revision_id", &mut failures);
                failures.finish()?;
                let revision_id = revision_id.expect("the path parameter was read");
                Ok(document_response(
                    200,
                    &self.application.read_revision(&revision_id)?,
                )?)
            }
            Route::RunMechanicalReference => {
                let payload =
                    optional_body_model::<RunMechanicalWorkflowRequest>(request, &mut failures);
                failures.finish()?;
                let payload = payload.expect("the body was read");
                let run = self.application.run_mechanical_reference(&payload)?;
                let status = if run.started { 201 } else { 200 };
                let mut response = document_response(status, &run.document)?;
                set_location(
                    &mut response,
                    &format!("{API_PREFIX}/workflows/{}", run.workflow_id),
                );
                Ok(response)
            }
            Route::ReadWorkflowState => {
                let workflow_id = parameters.identifier("workflow_id", &mut failures);
                failures.finish()?;
                let workflow_id = workflow_id.expect("the path parameter was read");
                Ok(document_response(
                    200,
                    &self.application.read_workflow_state(&workflow_id)?,
                )?)
            }
            Route::ListCheckpoints => {
                let workflow_id = parameters.identifier("workflow_id", &mut failures);
                failures.finish()?;
                let workflow_id = workflow_id.expect("the path parameter was read");
                Ok(document_response(
                    200,
                    &self.application.list_checkpoints(&workflow_id)?,
                )?)
            }
            Route::ApproveCheckpoint => {
                let workflow_id = parameters.identifier("workflow_id", &mut failures);
                let payload =
                    optional_body_model::<CheckpointDecisionRequest>(request, &mut failures);
                failures.finish()?;
                let workflow_id = workflow_id.expect("the path parameter was read");
                let payload = payload.expect("the body was read");
                Ok(document_response(
                    200,
                    &self
                        .application
                        .approve_checkpoint(&workflow_id, &payload)?,
                )?)
            }
            Route::RejectCheckpoint => {
                let workflow_id = parameters.identifier("workflow_id", &mut failures);
                let payload =
                    optional_body_model::<CheckpointDecisionRequest>(request, &mut failures);
                failures.finish()?;
                let workflow_id = workflow_id.expect("the path parameter was read");
                let payload = payload.expect("the body was read");
                Ok(document_response(
                    200,
                    &self.application.reject_checkpoint(&workflow_id, &payload)?,
                )?)
            }
            Route::ListEvents => {
                let after_sequence = query_integer(
                    request,
                    "after_sequence",
                    0,
                    0,
                    MAX_AFTER_SEQUENCE,
                    &mut failures,
                );
                let limit = query_limit(request, "limit", DEFAULT_LIST_LIMIT, &mut failures);
                let workflow_id = query_optional_identifier(
                    request,
                    "workflow_id",
                    128,
                    IDENTIFIER_PATTERN,
                    &mut failures,
                );
                let event_type = query_optional_identifier(
                    request,
                    "event_type",
                    128,
                    IDENTIFIER_PATTERN,
                    &mut failures,
                );
                let aggregate_id = query_optional_identifier(
                    request,
                    "aggregate_id",
                    128,
                    IDENTIFIER_PATTERN,
                    &mut failures,
                );
                failures.finish()?;
                let query = EventQuery {
                    after_sequence: after_sequence.expect("the query parameter was read"),
                    limit: limit.expect("the query parameter was read"),
                    workflow_id,
                    event_type,
                    aggregate_id,
                };
                Ok(document_response(
                    200,
                    &self.application.list_events(&query)?,
                )?)
            }
        }
    }
}

impl Router for Routes {
    fn handle<'a>(&'a self, request: Request<Full<Bytes>>) -> BoxFuture<'a, Response<Full<Bytes>>> {
        Box::pin(async move { self.answer(request) })
    }
}

/// The parts of one request the routes read.
struct RequestParts<'a> {
    /// The raw query string.
    query: &'a str,
    /// The headers of the request.
    headers: &'a [(Vec<u8>, Vec<u8>)],
    /// The body of the request.
    body: &'a Bytes,
}

/// The failures of one request.
///
/// The framework collects every failure of a request before it answers, so an
/// answer carries the failures of the path parameters, of the query parameters
/// and of the body together.
#[derive(Clone, Debug, Default)]
struct Failures {
    /// The failures, in the order they were found.
    values: Vec<ValidationFailure>,
    /// Whether the transport failed in a way the caller must not learn about.
    internal: bool,
    /// The framework failure of the request, when the body could not be read.
    http: Option<(u16, &'static str)>,
}

impl Failures {
    /// Add one failure.
    fn push(&mut self, failure: ValidationFailure) {
        self.values.push(failure);
    }

    /// Add one failure of the transport.
    fn push_internal(&mut self) {
        self.internal = true;
    }

    /// Add the framework failure of the request.
    fn push_http(&mut self, status: u16, detail: &'static str) {
        self.http = Some((status, detail));
    }

    /// Refuse the request when it carries a failure.
    fn finish(self) -> Result<(), Failure> {
        if let Some((status, detail)) = self.http {
            return Err(Failure::Http(status, detail));
        }
        if self.internal {
            return Err(Failure::Internal);
        }
        if self.values.is_empty() {
            Ok(())
        } else {
            Err(Failure::Validation(self.values))
        }
    }
}

/// One failure of a route.
enum Failure {
    /// The request is not valid.
    Validation(Vec<ValidationFailure>),
    /// The composition root refused the request.
    Application(ApplicationError),
    /// The transport failed in a way the caller must not learn about.
    Internal,
    /// The framework refused the request before it could be read.
    Http(u16, &'static str),
}

impl From<ApplicationError> for Failure {
    fn from(error: ApplicationError) -> Self {
        Self::Application(error)
    }
}

impl Failure {
    /// The answer of the failure.
    fn into_response(self) -> Response<Full<Bytes>> {
        match self {
            Self::Validation(failures) => {
                let body = crate::errors::handle_request_validation(Some(&failures));
                server::error_response(crate::errors::UNPROCESSABLE_CONTENT, &body)
            }
            Self::Application(error) => error_response(&error),
            Self::Internal => server::internal_error_response(),
            Self::Http(status, detail) => {
                let (status, body) = crate::errors::handle_http_exception(status, detail);
                server::error_response(status, &body)
            }
        }
    }
}

/// The answer of a failure of the composition root.
fn error_response(error: &ApplicationError) -> Response<Full<Bytes>> {
    let (status, body) = crate::errors::handle_application_error(error);
    server::error_response(status, &body)
}

/// The answer of an unknown route.
fn not_found() -> Response<Full<Bytes>> {
    let (status, body) = crate::errors::handle_http_exception(404, "");
    server::error_response(status, &body)
}

/// The answer of a route that exists for another method.
fn method_not_allowed() -> Response<Full<Bytes>> {
    let (status, body) = crate::errors::handle_http_exception(405, "");
    server::error_response(status, &body)
}

/// The answer of a route that carries a document.
fn document_response(status: u16, document: &WireValue) -> Result<Response<Full<Bytes>>, Failure> {
    let body = crate::wire::encode_answer(document).map_err(|_| Failure::Internal)?;
    let length = body.len();
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let headers = response.headers_mut();
    headers.insert(CONTENT_LENGTH, hyper::header::HeaderValue::from(length));
    headers.insert(
        CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

/// Name the place of a created resource.
fn set_location(response: &mut Response<Full<Bytes>>, location: &str) {
    if let Ok(value) = hyper::header::HeaderValue::from_str(location) {
        response
            .headers_mut()
            .insert(hyper::header::LOCATION, value);
    }
}

/// The answer of a redirected path.
fn redirect_response(
    path: &str,
    query: &str,
    headers: &[(Vec<u8>, Vec<u8>)],
) -> Response<Full<Bytes>> {
    let host = crate::middleware::header_value(headers, "host").unwrap_or_default();
    let mut url = format!("http://{host}{path}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(query);
    }
    let location = quote(&url);
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    let headers = response.headers_mut();
    headers.insert(CONTENT_LENGTH, hyper::header::HeaderValue::from(0));
    if let Ok(value) = hyper::header::HeaderValue::from_str(&location) {
        headers.insert(hyper::header::LOCATION, value);
    }
    response
}

/// The path the redirect of one path names.
fn redirect_path(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    if path.ends_with('/') {
        let stripped = path.trim_end_matches('/');
        if stripped.is_empty() {
            return None;
        }
        Some(stripped.to_owned())
    } else {
        Some(format!("{path}/"))
    }
}

/// The path parameters of one route.
struct Parameters {
    /// The parameters, in the order the route declares them.
    values: Vec<(&'static str, String)>,
}

impl Parameters {
    /// One identifier of the path, validated against its pattern.
    ///
    /// The value is `None` when the request carries a failure, which the
    /// caller collects before it answers.
    fn identifier(&self, name: &'static str, failures: &mut Failures) -> Option<String> {
        let value = self
            .values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())?;
        let maximum = if name == "design_id" { 64 } else { 128 };
        let pattern = if name == "design_id" {
            DESIGN_ID_PATTERN
        } else {
            IDENTIFIER_PATTERN
        };
        match validate_text(value, maximum, pattern) {
            Ok(()) => Some(value.to_owned()),
            Err((kind, message)) => {
                failures.push(ValidationFailure {
                    location: vec!["path".to_owned(), name.to_owned()],
                    message: Some(message),
                    kind: Some(kind.to_owned()),
                });
                None
            }
        }
    }
}

/// One failure of a path or a query parameter.
///
/// The kind and the message are the ones Pydantic reports; the place of the
/// failure is added by the caller, because a path parameter and a query
/// parameter are reported under different places.
fn validate_text(value: &str, maximum: usize, pattern: &str) -> Result<(), (&'static str, String)> {
    let length = value.chars().count();
    if length > maximum {
        return Err((
            crate::validation::kind::STRING_TOO_LONG,
            format!("String should have at most {maximum} characters"),
        ));
    }
    if !crate::patterns::matches(pattern, value) {
        return Err((
            crate::validation::kind::STRING_PATTERN_MISMATCH,
            format!("String should match pattern '{pattern}'"),
        ));
    }
    Ok(())
}

/// Read the body of a request as a model.
///
/// The model is `None` when the request carries a failure, which the caller
/// collects before it answers.
fn body_model<M: ApiModel>(request: &RequestParts<'_>, failures: &mut Failures) -> Option<M> {
    match read_body(request) {
        Ok(Body::Value(value)) => match M::from_body(Some(&value)) {
            Ok(model) => Some(model),
            Err(reported) => {
                push_model_failures(failures, reported);
                None
            }
        },
        Ok(Body::Missing) => match M::from_body(None) {
            Ok(model) => Some(model),
            Err(reported) => {
                push_model_failures(failures, reported);
                None
            }
        },
        Ok(Body::NotAnObject) => match M::from_body(Some(&not_an_object())) {
            Ok(model) => Some(model),
            Err(reported) => {
                push_model_failures(failures, reported);
                None
            }
        },
        Err(Failure::Validation(reported)) => {
            for failure in reported {
                failures.push(failure);
            }
            None
        }
        Err(Failure::Internal) => {
            failures.push_internal();
            None
        }
        Err(Failure::Http(status, detail)) => {
            failures.push_http(status, detail);
            None
        }
        Err(Failure::Application(error)) => {
            failures.push(ValidationFailure {
                location: vec!["body".to_owned()],
                message: Some(error.message),
                kind: Some(error.code),
            });
            None
        }
    }
}

/// Read the body of a request as a model that may be absent.
///
/// The model is `None` when the request carries a failure, which the caller
/// collects before it answers.
fn optional_body_model<M: ApiModel + Default>(
    request: &RequestParts<'_>,
    failures: &mut Failures,
) -> Option<M> {
    match read_body(request) {
        Ok(Body::Value(value)) => match M::from_body(Some(&value)) {
            Ok(model) => Some(model),
            Err(reported) => {
                push_model_failures(failures, reported);
                None
            }
        },
        Ok(Body::Missing) => Some(M::default()),
        Ok(Body::NotAnObject) => match M::from_body(Some(&not_an_object())) {
            Ok(model) => Some(model),
            Err(reported) => {
                push_model_failures(failures, reported);
                None
            }
        },
        Err(Failure::Validation(reported)) => {
            for failure in reported {
                failures.push(failure);
            }
            None
        }
        Err(Failure::Internal) => {
            failures.push_internal();
            None
        }
        Err(Failure::Http(status, detail)) => {
            failures.push_http(status, detail);
            None
        }
        Err(Failure::Application(error)) => {
            failures.push(ValidationFailure {
                location: vec!["body".to_owned()],
                message: Some(error.message),
                kind: Some(error.code),
            });
            None
        }
    }
}

/// Add the failures of a model to the failures of the request.
fn push_model_failures(failures: &mut Failures, reported: Vec<crate::validation::Issue>) {
    for failure in crate::schemas::transport_failures(&reported) {
        failures.push(failure);
    }
}

/// A body that is not an object.
fn not_an_object() -> kala_json::CanonicalValue {
    kala_json::CanonicalValue::Text(kala_json::JsonText::new(""))
}

/// What a body carries.
enum Body {
    /// The body is a JSON document.
    Value(kala_json::CanonicalValue),
    /// The body is not there.
    Missing,
    /// The body is there but is not a JSON object.
    NotAnObject,
}

/// Read the body of a request.
///
/// The Python source reads a body as JSON when the content type names JSON
/// and reads it as bytes otherwise; a body that is not JSON at all is a
/// failure of the body layer.
fn read_body(request: &RequestParts<'_>) -> Result<Body, Failure> {
    if request.body.is_empty() {
        return Ok(Body::Missing);
    }
    if !json_content_type(request.headers) {
        return Ok(Body::NotAnObject);
    }
    let Ok(text) = std::str::from_utf8(request.body) else {
        // The framework refuses a body it cannot decode, and it refuses it
        // before the request is read.
        return Err(Failure::Http(400, "There was an error parsing the body"));
    };
    match jsoninput::parse(text) {
        Ok(value) => Ok(Body::Value(value)),
        Err(error) => Err(Failure::Validation(vec![ValidationFailure {
            location: vec!["body".to_owned(), error.position.to_string()],
            message: Some("JSON decode error".to_owned()),
            kind: Some("json_invalid".to_owned()),
        }])),
    }
}

/// Whether the content type of a request names a JSON document.
///
/// The framework reads a body as JSON only when the content type names it.
/// A request that carries no content type at all, or a content type that
/// names something else, hands its raw bytes to the model, and the model
/// refuses them because they are not an object.
fn json_content_type(headers: &[(Vec<u8>, Vec<u8>)]) -> bool {
    let Some(value) = crate::middleware::header_value(headers, "content-type") else {
        return false;
    };
    let media = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let Some((main, sub)) = media.split_once('/') else {
        return false;
    };
    main.trim() == "application" && (sub.trim() == "json" || sub.trim().ends_with("+json"))
}

/// Read one identifier of the query string that the caller must name.
fn query_identifier(
    request: &RequestParts<'_>,
    name: &str,
    maximum: usize,
    pattern: &str,
    failures: &mut Failures,
) -> Option<String> {
    let query = Query::parse(request.query);
    let Some(value) = query.get(name) else {
        failures.push(ValidationFailure {
            location: vec!["query".to_owned(), name.to_owned()],
            message: Some("Field required".to_owned()),
            kind: Some(crate::validation::kind::MISSING.to_owned()),
        });
        return None;
    };
    match validate_text(value, maximum, pattern) {
        Ok(()) => Some(value.to_owned()),
        Err((kind, message)) => {
            failures.push(ValidationFailure {
                location: vec!["query".to_owned(), name.to_owned()],
                message: Some(message),
                kind: Some(kind.to_owned()),
            });
            None
        }
    }
}

/// Read one identifier of the query string that the caller may name.
fn query_optional_identifier(
    request: &RequestParts<'_>,
    name: &str,
    maximum: usize,
    pattern: &str,
    failures: &mut Failures,
) -> Option<String> {
    let query = Query::parse(request.query);
    let value = query.get(name)?;
    match validate_text(value, maximum, pattern) {
        Ok(()) => Some(value.to_owned()),
        Err((kind, message)) => {
            failures.push(ValidationFailure {
                location: vec!["query".to_owned(), name.to_owned()],
                message: Some(message),
                kind: Some(kind.to_owned()),
            });
            None
        }
    }
}

/// Read the `limit` of the query string.
fn query_limit(
    request: &RequestParts<'_>,
    name: &str,
    default: i64,
    failures: &mut Failures,
) -> Option<i64> {
    let query = Query::parse(request.query);
    if query.get(name).is_none() {
        return Some(default);
    }
    query_integer(request, name, default, 1, MAX_LIST_LIMIT, failures)
}

/// Read one integer of the query string.
fn query_integer(
    request: &RequestParts<'_>,
    name: &str,
    default: i64,
    minimum: i64,
    maximum: i64,
    failures: &mut Failures,
) -> Option<i64> {
    let query = Query::parse(request.query);
    let Some(value) = query.get(name) else {
        return Some(default);
    };
    let number = match parse_integer(value) {
        Ok(number) => number,
        Err(kind) => {
            failures.push(ValidationFailure {
                location: vec!["query".to_owned(), name.to_owned()],
                message: Some(
                    match kind {
                        IntegerFailure::Parsing => {
                            "Input should be a valid integer, unable to parse string as an integer"
                        }
                        IntegerFailure::TooLarge => {
                            "Unable to parse input string as an integer, exceeded maximum size"
                        }
                    }
                    .to_owned(),
                ),
                kind: Some(
                    match kind {
                        IntegerFailure::Parsing => crate::validation::kind::INT_PARSING,
                        IntegerFailure::TooLarge => "int_parsing_size",
                    }
                    .to_owned(),
                ),
            });
            return None;
        }
    };
    if number < BigInt::from(minimum) {
        failures.push(ValidationFailure {
            location: vec!["query".to_owned(), name.to_owned()],
            message: Some(format!(
                "Input should be greater than or equal to {minimum}"
            )),
            kind: Some("greater_than_equal".to_owned()),
        });
        return None;
    }
    if number > BigInt::from(maximum) {
        failures.push(ValidationFailure {
            location: vec!["query".to_owned(), name.to_owned()],
            message: Some(format!("Input should be less than or equal to {maximum}")),
            kind: Some("less_than_equal".to_owned()),
        });
        return None;
    }
    Some(i64::try_from(&number).unwrap_or_default())
}

/// Why an integer of the query string could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IntegerFailure {
    /// The value is not an integer.
    Parsing,
    /// The value carries more digits than CPython converts.
    TooLarge,
}

/// Read one integer of the query string the way Pydantic reads it.
///
/// The surrounding whitespace is ignored, an underscore between two digits is
/// ignored, a decimal spelling without a fractional part is an integer too,
/// and a value that carries more digits than CPython converts is refused.
fn parse_integer(value: &str) -> Result<BigInt, IntegerFailure> {
    let trimmed = value.trim_matches(|character: char| character.is_ascii_whitespace());
    if trimmed.is_empty() {
        return Err(IntegerFailure::Parsing);
    }
    if trimmed.len() > MAX_INTEGER_DIGITS {
        return Err(IntegerFailure::TooLarge);
    }
    let (sign, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (-1_i64, rest),
        None => (1_i64, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    // A decimal spelling is an integer when its fractional part is zero.
    let (whole, fraction) = match rest.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (rest, None),
    };
    let digits = remove_underscores(whole).ok_or(IntegerFailure::Parsing)?;
    if digits.is_empty() {
        return Err(IntegerFailure::Parsing);
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(IntegerFailure::Parsing);
        }
        if fraction.bytes().any(|byte| byte != b'0') {
            return Err(IntegerFailure::Parsing);
        }
    }
    let number = BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(IntegerFailure::Parsing)?;
    Ok(if sign < 0 { -number } else { number })
}

/// Remove the underscores that CPython's `int()` ignores.
fn remove_underscores(value: &str) -> Option<String> {
    let mut out = String::with_capacity(value.len());
    let mut previous_was_digit = false;
    for character in value.chars() {
        if character == '_' {
            if !previous_was_digit {
                return None;
            }
            previous_was_digit = false;
            continue;
        }
        if !character.is_ascii_digit() {
            return None;
        }
        previous_was_digit = true;
        out.push(character);
    }
    if !previous_was_digit {
        return None;
    }
    Some(out)
}

/// The parameters of one query string.
struct Query {
    /// The pairs of the query string, in the order they appear.
    pairs: Vec<(String, String)>,
}

impl Query {
    /// Read the pairs of a query string.
    ///
    /// The reader is the one of CPython: the pairs are separated by `&`, a
    /// pair without a `=` carries an empty value, a `+` is a space and a
    /// percent escape is decoded.
    fn parse(raw: &str) -> Self {
        let mut pairs = Vec::new();
        for part in raw.split('&') {
            if part.is_empty() {
                continue;
            }
            let (name, value) = match part.split_once('=') {
                Some((name, value)) => (name, value),
                None => (part, ""),
            };
            pairs.push((decode_query(name), decode_query(value)));
        }
        Self { pairs }
    }

    /// The last value of one name.
    fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Decode one part of a query string.
fn decode_query(value: &str) -> String {
    let replaced = value.replace('+', " ");
    if !replaced.contains('%') {
        return replaced;
    }
    let bytes = replaced.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high as u8) * 16 + (low as u8));
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode the path of a request.
///
/// The framework decodes the path before the application sees it, and a
/// percent escape that is not valid UTF-8 becomes the replacement character,
/// exactly as CPython's `unquote` does.
fn percent_decode(path: &str) -> String {
    if !path.contains('%') {
        return path.to_owned();
    }
    let bytes = path.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high as u8) * 16 + (low as u8));
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Quote one URL the way CPython's `urllib.parse.quote` quotes it.
fn quote(url: &str) -> String {
    const SAFE: &str = ":/%#?=@[]!$&'()*+,;";
    let mut out = String::with_capacity(url.len());
    for character in url.chars() {
        if character.is_ascii_alphanumeric()
            || "_.-~".contains(character)
            || SAFE.contains(character)
        {
            out.push(character);
            continue;
        }
        let mut buffer = [0_u8; 4];
        for byte in character.encode_utf8(&mut buffer).as_bytes() {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

/// Match one path against one pattern.
///
/// A pattern segment in braces matches one segment that is not empty, which
/// is the rule the framework's own patterns declare.
fn match_path(pattern: &'static str, path: &str) -> Option<Parameters> {
    let mut pattern_parts = pattern.split('/');
    let mut path_parts = path.split('/');
    let mut values: Vec<(&'static str, String)> = Vec::new();
    loop {
        match (pattern_parts.next(), path_parts.next()) {
            (None, None) => {
                return Some(Parameters { values });
            }
            (Some(part), Some(segment)) => {
                if let Some(name) = part
                    .strip_prefix('{')
                    .and_then(|part| part.strip_suffix('}'))
                {
                    if segment.is_empty() {
                        return None;
                    }
                    values.push((name, segment.to_owned()));
                } else if part != segment {
                    return None;
                }
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A composition root that answers fixed documents.
    struct Fake {
        /// Whether the composition root is closed.
        closed: bool,
    }

    impl Application for Fake {
        fn health(&self) -> WireValue {
            WireValue::object([("status", WireValue::Text(kala_contracts::Text::new("ok")))])
        }

        fn list_domains(&self) -> WireValue {
            WireValue::object([("count", WireValue::Integer(0_u64.into()))])
        }

        fn read_domain(&self, domain: &str) -> Result<WireValue, ApplicationError> {
            Err(ApplicationError::new(
                crate::errors::ApplicationErrorKind::UnknownDomain,
                format!("domain {domain} is not registered"),
            ))
        }

        fn evaluate_domain(
            &self,
            _domain: &str,
            _request: &EvaluateDomainRequest,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "evaluation",
                WireValue::Integer(1_u64.into()),
            )]))
        }

        fn create_revision(
            &self,
            _request: &CreateRevisionRequest,
        ) -> Result<Created, ApplicationError> {
            Ok(Created {
                document: WireValue::object([(
                    "revision_id",
                    WireValue::Text(kala_contracts::Text::new("rev-a-r0")),
                )]),
                identifier: "rev-a-r0".to_owned(),
            })
        }

        fn prepare_revision_commit(
            &self,
            _request: &PrepareRevisionRequest,
        ) -> Result<Created, ApplicationError> {
            Ok(Created {
                document: WireValue::object([(
                    "request_id",
                    WireValue::Text(kala_contracts::Text::new("commit-a")),
                )]),
                identifier: "commit-a".to_owned(),
            })
        }

        fn read_revision_commit(&self, _request_id: &str) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "request_id",
                WireValue::Text(kala_contracts::Text::new("commit-a")),
            )]))
        }

        fn approve_revision_commit(
            &self,
            _request_id: &str,
            _request: &CheckpointDecisionRequest,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "revision_id",
                WireValue::Text(kala_contracts::Text::new("rev-a-r0")),
            )]))
        }

        fn reject_revision_commit(
            &self,
            _request_id: &str,
            _request: &CheckpointDecisionRequest,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "status",
                WireValue::Text(kala_contracts::Text::new("rejected")),
            )]))
        }

        fn list_revisions(
            &self,
            _design_id: &str,
            _limit: i64,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "count",
                WireValue::Integer(0_u64.into()),
            )]))
        }

        fn read_revision(&self, _revision_id: &str) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "revision_id",
                WireValue::Text(kala_contracts::Text::new("rev-a-r0")),
            )]))
        }

        fn run_mechanical_reference(
            &self,
            _request: &RunMechanicalWorkflowRequest,
        ) -> Result<MechanicalRun, ApplicationError> {
            Ok(MechanicalRun {
                document: WireValue::object([("started", WireValue::Bool(true))]),
                workflow_id: "workflow-a".to_owned(),
                started: true,
            })
        }

        fn read_workflow_state(&self, _workflow_id: &str) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "status",
                WireValue::Text(kala_contracts::Text::new("created")),
            )]))
        }

        fn list_checkpoints(&self, _workflow_id: &str) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "count",
                WireValue::Integer(0_u64.into()),
            )]))
        }

        fn approve_checkpoint(
            &self,
            _workflow_id: &str,
            _request: &CheckpointDecisionRequest,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([("checkpoint", WireValue::Null)]))
        }

        fn reject_checkpoint(
            &self,
            _workflow_id: &str,
            _request: &CheckpointDecisionRequest,
        ) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([("checkpoint", WireValue::Null)]))
        }

        fn list_events(&self, _query: &EventQuery) -> Result<WireValue, ApplicationError> {
            Ok(WireValue::object([(
                "count",
                WireValue::Integer(0_u64.into()),
            )]))
        }

        fn closed(&self) -> bool {
            self.closed
        }

        fn close(&self) {}
    }

    /// One request of the router.
    fn ask(
        routes: &Routes,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, String, Vec<(String, String)>) {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8000");
        if !body.is_empty() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(Full::new(Bytes::from(body.to_owned())))
            .expect("the request is built");
        let response = routes.answer(request);
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect();
        let bytes = response.into_body().into_inner().unwrap_or_default();
        (
            status,
            String::from_utf8_lossy(&bytes).into_owned(),
            headers,
        )
    }

    /// The router of a launch that is not closed.
    fn router() -> Routes {
        Routes::new(Arc::new(Fake { closed: false }))
    }

    #[test]
    fn a_known_route_answers_its_document() {
        let (status, body, _) = ask(&router(), "GET", "/health", "");
        assert_eq!(status, 200);
        assert_eq!(body, "{\"status\":\"ok\"}");
    }

    #[test]
    fn an_unknown_route_answers_the_sanitized_refusal() {
        let (status, body, _) = ask(&router(), "GET", "/nope", "");
        assert_eq!(status, 404);
        assert_eq!(
            body,
            "{\"error\":{\"code\":\"not_found\",\"message\":\"the requested resource does not exist\"},\"details\":[]}"
        );
    }

    #[test]
    fn a_route_that_exists_for_another_method_answers_the_refusal() {
        let (status, body, _) = ask(&router(), "POST", "/health", "");
        assert_eq!(status, 405);
        assert_eq!(
            body,
            "{\"error\":{\"code\":\"method_not_allowed\",\"message\":\"the requested method is not allowed for this resource\"},\"details\":[]}"
        );
    }

    #[test]
    fn a_path_with_a_trailing_slash_is_redirected() {
        let (status, _, headers) = ask(&router(), "GET", "/health/", "");
        assert_eq!(status, 307);
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "location" && value == "http://127.0.0.1:8000/health")
        );
    }

    #[test]
    fn a_created_resource_names_its_place() {
        let (status, _, headers) = ask(
            &router(),
            "POST",
            "/api/v1/revisions",
            "{\"design_id\":\"a\",\"domain\":\"building\"}",
        );
        assert_eq!(status, 201);
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "location" && value == "/api/v1/revisions/rev-a-r0")
        );
    }

    #[test]
    fn a_rejected_path_parameter_answers_the_pydantic_detail() {
        let (status, body, _) = ask(&router(), "GET", "/api/v1/domains/not%20valid", "");
        assert_eq!(status, 422);
        assert_eq!(
            body,
            "{\"error\":{\"code\":\"invalid_request\",\"message\":\"the request body or parameters are invalid\"},\"details\":[\"path.domain: string_pattern_mismatch: String should match pattern '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'\"]}"
        );
    }

    #[test]
    fn a_rejected_query_parameter_answers_the_pydantic_detail() {
        let (status, body, _) = ask(&router(), "GET", "/api/v1/revisions?limit=0", "");
        assert_eq!(status, 422);
        assert!(body.contains(
            "query.limit: greater_than_equal: Input should be greater than or equal to 1"
        ));
    }

    #[test]
    fn a_rejected_body_answers_the_pydantic_detail() {
        let (status, body, _) = ask(
            &router(),
            "POST",
            "/api/v1/revisions",
            "{\"design_id\":\"a\",\"domain\":\"nope\"}",
        );
        assert_eq!(status, 422);
        assert!(
            body.contains("body.domain: enum: Input should be 'building', 'mechanical' or 'wing'")
        );
    }

    #[test]
    fn a_body_that_is_not_json_answers_the_reading_failure() {
        let (status, body, _) = ask(&router(), "POST", "/api/v1/revisions", "{\"a\": ");
        assert_eq!(status, 422);
        assert!(body.contains("body.6: json_invalid: JSON decode error"));
    }

    #[test]
    fn a_closed_composition_root_is_refused_before_any_route_runs() {
        let routes = Routes::new(Arc::new(Fake { closed: true }));
        let (status, body, _) = ask(&routes, "GET", "/health", "");
        assert_eq!(status, 503);
        assert_eq!(
            body,
            "{\"error\":{\"code\":\"application_closed\",\"message\":\"the local application is closed\"},\"details\":[]}"
        );
    }

    #[test]
    fn a_path_matches_the_pattern_it_declares() {
        let matched = match_path("/api/v1/domains/{domain}", "/api/v1/domains/building")
            .expect("the path matches");
        assert_eq!(matched.values, vec![("domain", "building".to_owned())]);
        assert!(match_path("/api/v1/domains/{domain}", "/api/v1/domains").is_none());
        assert!(match_path("/api/v1/domains/{domain}", "/api/v1/domains/a/b").is_none());
        assert!(match_path("/api/v1/domains/{domain}", "/api/v1/domains/").is_none());
    }

    #[test]
    fn a_query_string_carries_the_last_value_of_a_name() {
        let query = Query::parse("design_id=a&design_id=b&limit=3");
        assert_eq!(query.get("design_id"), Some("b"));
        assert_eq!(query.get("limit"), Some("3"));
        assert_eq!(query.get("missing"), None);
    }

    #[test]
    fn a_query_string_is_decoded_like_the_python_reader() {
        let query = Query::parse("a=one+two&b=%2F&c=%zz&d");
        assert_eq!(query.get("a"), Some("one two"));
        assert_eq!(query.get("b"), Some("/"));
        assert_eq!(query.get("c"), Some("%zz"));
        assert_eq!(query.get("d"), Some(""));
    }

    #[test]
    fn an_integer_of_the_query_string_is_read_like_pydantic_reads_it() {
        assert_eq!(parse_integer("5"), Ok(BigInt::from(5)));
        assert_eq!(parse_integer(" 5 "), Ok(BigInt::from(5)));
        assert_eq!(parse_integer("+5"), Ok(BigInt::from(5)));
        assert_eq!(parse_integer("-5"), Ok(BigInt::from(-5)));
        assert_eq!(parse_integer("1_0"), Ok(BigInt::from(10)));
        assert_eq!(parse_integer("007"), Ok(BigInt::from(7)));
        assert_eq!(parse_integer("5.0"), Ok(BigInt::from(5)));
        assert_eq!(parse_integer("5.5"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("5."), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("1e3"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("0x10"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("1__0"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("_1"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("1_"), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer(""), Err(IntegerFailure::Parsing));
        assert_eq!(parse_integer("abc"), Err(IntegerFailure::Parsing));
        assert_eq!(
            parse_integer(&"9".repeat(4_301)),
            Err(IntegerFailure::TooLarge)
        );
    }

    #[test]
    fn a_url_is_quoted_like_the_python_reader() {
        assert_eq!(
            quote("http://127.0.0.1:8000/health"),
            "http://127.0.0.1:8000/health"
        );
        assert_eq!(quote("http://host/a b"), "http://host/a%20b");
        assert_eq!(quote("http://host/a?b=1&c=2"), "http://host/a?b=1&c=2");
    }

    #[test]
    fn a_path_is_decoded_like_the_python_reader() {
        assert_eq!(
            percent_decode("/api/v1/domains/building"),
            "/api/v1/domains/building"
        );
        assert_eq!(
            percent_decode("/api/v1/domains/a%2Fb"),
            "/api/v1/domains/a/b"
        );
        assert_eq!(
            percent_decode("/api/v1/domains/a%20b"),
            "/api/v1/domains/a b"
        );
        assert_eq!(percent_decode("/api/v1/domains/%zz"), "/api/v1/domains/%zz");
    }
}
