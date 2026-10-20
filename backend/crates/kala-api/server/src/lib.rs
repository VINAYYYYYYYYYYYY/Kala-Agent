//! The composition adapter of the local Kala HTTP API.
//!
//! The routes of the accepted `kala-api` crate are transport only: they
//! match the request, validate it and write the answer, and they reach the
//! composition root through the [`Application`] port. This crate is the other
//! side of that port: it holds the accepted `kala-application` composition
//! root and answers the port with the documents the Python source writes.
//!
//! Every answer is built from the DTOs of the composition root, in the field
//! order of the Python models, and is encoded with the wire form of the
//! accepted `kala-contracts` crate: the compact separators and the UTF-8 text
//! Starlette writes. The bytes are therefore the bytes the Python API writes,
//! without a second model of the views.
//!
//! The crate also builds the router of one launch and owns the lifecycle of
//! the composition root: the launch closes the root it created and leaves a
//! root it was given alone.
//!
//! # The launch entry point
//!
//! [`launch`] is the library form of the `kala-api-server` binary: it mints
//! nothing and prints nothing, it builds the router of one launch over a
//! composition root the caller already opened, binds the loopback address,
//! serves until `SIGINT` or `SIGTERM`, and returns the facts of the launch and
//! the signal that stopped it. The binary and the `kala-agent serve` command
//! share it, so the two entry points serve the same bytes. Who closes the
//! composition root, and what a stop prints, stays with the caller.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kala_api::boundary::API_TOKEN_FILE_NAME;
use kala_api::errors::{ApplicationError, ApplicationErrorKind};
use kala_api::middleware::MAX_REQUEST_BODY_BYTES;
use kala_api::routes::{Application, Created, EventQuery, Lifespan, MechanicalRun, Routes};
use kala_api::schemas::{
    CheckpointDecisionRequest, CreateRevisionRequest, EvaluateDomainRequest,
    PrepareRevisionRequest, RunMechanicalWorkflowRequest,
};
use kala_api::server::{ApiState, serve};
use kala_application::{ErrorKind, KalaApplication, View};
use kala_contracts::WireValue;
use num_bigint::BigInt;

/// The version the API reports.
pub const API_VERSION: &str = kala_api::routes::API_VERSION;

/// The disclaimer the health answer carries.
pub const LOCAL_DISCLAIMER: &str = kala_api::routes::LOCAL_DISCLAIMER;

/// The name of the service the API answers for.
pub const SERVICE_NAME: &str = kala_api::schemas::SERVICE_NAME;

/// The composition root the routes of the local API call.
pub struct LocalApplication {
    /// The composition root.
    application: Arc<KalaApplication>,
}

impl LocalApplication {
    /// Bind the composition root to the port of the routes.
    #[must_use]
    pub fn new(application: Arc<KalaApplication>) -> Self {
        Self { application }
    }

    /// The composition root.
    #[must_use]
    pub fn application(&self) -> &Arc<KalaApplication> {
        &self.application
    }

    /// The facts of the health route.
    fn health_document(&self) -> WireValue {
        let application = &self.application;
        let schema_version = application.database().schema_version().unwrap_or_default();
        WireValue::object([
            ("status", text("ok")),
            ("service", text(SERVICE_NAME)),
            ("version", text(API_VERSION)),
            ("provider", text(application.provider_id())),
            ("model", text(application.model_id())),
            ("offline", WireValue::Bool(application.offline())),
            (
                "egress_configured",
                WireValue::Bool(application.egress_configured()),
            ),
            (
                "provider_endpoint_declared",
                WireValue::Bool(application.provider_endpoint_declared()),
            ),
            (
                "data_dir",
                text(&application.settings().data_dir.to_string_lossy()),
            ),
            ("schema_version", WireValue::Integer(schema_version.into())),
            (
                "domains",
                WireValue::array(application.domains().into_iter().map(text)),
            ),
            ("disclaimer", text(LOCAL_DISCLAIMER)),
        ])
    }

    /// The registered domain packages and their workflows.
    fn domains_document(&self) -> WireValue {
        let packages = self.application.list_domain_packages();
        let workflows = self.application.list_workflow_definitions();
        WireValue::object([
            ("count", WireValue::Integer(BigInt::from(packages.len()))),
            (
                "packages",
                WireValue::array(packages.iter().map(View::to_wire_value)),
            ),
            (
                "workflows",
                WireValue::array(workflows.iter().map(View::to_wire_value)),
            ),
            (
                "domains",
                WireValue::array(self.application.domains().into_iter().map(text)),
            ),
        ])
    }

    /// One evaluation and the empty list of further evaluations.
    fn evaluation_document(&self, report: &kala_application::DomainEvaluationReport) -> WireValue {
        WireValue::object([
            ("evaluation", report.to_wire_value()),
            ("evaluations", WireValue::Array(Vec::new())),
        ])
    }

    /// One mechanical run, its state, its revision and its checkpoint.
    fn run_document(&self, result: &kala_application::MechanicalRunResult) -> WireValue {
        WireValue::object([
            ("run", result.to_wire_value()),
            ("state", result.state.to_wire_value()),
            (
                "revision",
                match &result.revision {
                    Some(record) => record.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "checkpoint",
                match &result.checkpoint {
                    Some(checkpoint) => checkpoint.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
        ])
    }
}

impl Application for LocalApplication {
    fn health(&self) -> WireValue {
        self.health_document()
    }

    fn list_domains(&self) -> WireValue {
        self.domains_document()
    }

    fn read_domain(&self, domain: &str) -> Result<WireValue, ApplicationError> {
        let detail = self
            .application
            .describe_domain(domain)
            .map_err(port_error)?;
        Ok(detail.to_wire_value())
    }

    fn evaluate_domain(
        &self,
        domain: &str,
        request: &EvaluateDomainRequest,
    ) -> Result<WireValue, ApplicationError> {
        let parameters = request
            .parameters
            .as_ref()
            .map(|value| kala_json::CanonicalValue::Object(value.clone()));
        let report = self
            .application
            .evaluate_domain_design(domain, parameters.as_ref(), request.design_id.as_deref())
            .map_err(port_error)?;
        Ok(self.evaluation_document(&report))
    }

    fn create_revision(
        &self,
        request: &CreateRevisionRequest,
    ) -> Result<Created, ApplicationError> {
        let parameters = request
            .parameters
            .as_ref()
            .map(|value| kala_json::CanonicalValue::Object(value.clone()));
        let record = self
            .application
            .create_design_revision(
                &request.design_id,
                request.domain.as_str(),
                request.summary.as_deref(),
                parameters.as_ref(),
                request.parent_revision_id.as_deref(),
                &[],
                &[],
                None,
                kala_application::LOCAL_ACTOR_ID,
                request.state_digest.as_deref(),
                request.action_digest.as_deref(),
            )
            .map_err(port_error)?;
        Ok(Created {
            identifier: record.revision.revision_id().to_owned(),
            document: record.to_wire_value(),
        })
    }

    fn prepare_revision_commit(
        &self,
        request: &PrepareRevisionRequest,
    ) -> Result<Created, ApplicationError> {
        let parameters = request
            .parameters
            .as_ref()
            .map(|value| kala_json::CanonicalValue::Object(value.clone()));
        let prepared = self
            .application
            .prepare_design_revision(
                &request.design_id,
                request.domain.as_str(),
                request.summary.as_deref(),
                parameters.as_ref(),
                request.parent_revision_id.as_deref(),
                &[],
                &[],
                None,
            )
            .map_err(port_error)?;
        Ok(Created {
            identifier: prepared.request_id.clone(),
            document: prepared.to_wire_value(),
        })
    }

    fn read_revision_commit(&self, request_id: &str) -> Result<WireValue, ApplicationError> {
        let commit = self
            .application
            .get_revision_commit(request_id)
            .map_err(port_error)?;
        Ok(commit.to_wire_value())
    }

    fn approve_revision_commit(
        &self,
        request_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError> {
        let record = self
            .application
            .commit_design_revision(
                request_id,
                &request.actor_id,
                request.state_digest.as_deref(),
                request.action_digest.as_deref(),
                request.reason.as_deref(),
                None,
            )
            .map_err(port_error)?;
        Ok(record.to_wire_value())
    }

    fn reject_revision_commit(
        &self,
        request_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError> {
        let commit = self
            .application
            .decide_revision_commit(
                request_id,
                "reject",
                &request.actor_id,
                request.state_digest.as_deref(),
                request.action_digest.as_deref(),
                request.reason.as_deref(),
                None,
            )
            .map_err(port_error)?;
        Ok(commit.to_wire_value())
    }

    fn list_revisions(&self, design_id: &str, limit: i64) -> Result<WireValue, ApplicationError> {
        let revisions = self
            .application
            .list_design_revisions(design_id, limit)
            .map_err(port_error)?;
        Ok(WireValue::object([
            ("design_id", text(design_id)),
            ("count", WireValue::Integer(BigInt::from(revisions.len()))),
            (
                "revisions",
                WireValue::array(revisions.iter().map(View::to_wire_value)),
            ),
        ]))
    }

    fn read_revision(&self, revision_id: &str) -> Result<WireValue, ApplicationError> {
        let record = self
            .application
            .get_design_revision(revision_id)
            .map_err(port_error)?;
        Ok(record.to_wire_value())
    }

    fn run_mechanical_reference(
        &self,
        request: &RunMechanicalWorkflowRequest,
    ) -> Result<MechanicalRun, ApplicationError> {
        let result = self
            .application
            .run_mechanical_reference_workflow(
                request.workflow_id.as_deref(),
                &request.design_id,
                &request.actor_id,
            )
            .map_err(port_error)?;
        Ok(MechanicalRun {
            workflow_id: result.workflow_id.clone(),
            started: result.started,
            document: self.run_document(&result),
        })
    }

    fn read_workflow_state(&self, workflow_id: &str) -> Result<WireValue, ApplicationError> {
        let state = self
            .application
            .get_workflow_state(workflow_id)
            .map_err(port_error)?;
        Ok(state.to_wire_value())
    }

    fn list_checkpoints(&self, workflow_id: &str) -> Result<WireValue, ApplicationError> {
        let state = self
            .application
            .get_workflow_state(workflow_id)
            .map_err(port_error)?;
        let approvals = self
            .application
            .list_workflow_checkpoints(workflow_id)
            .map_err(port_error)?;
        Ok(WireValue::object([
            ("workflow_id", text(workflow_id)),
            ("count", WireValue::Integer(BigInt::from(approvals.len()))),
            (
                "pending",
                match &state.pending_approval {
                    Some(checkpoint) => checkpoint.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "approvals",
                WireValue::array(
                    approvals
                        .iter()
                        .map(kala_contracts::ContractModel::to_wire_value),
                ),
            ),
        ]))
    }

    fn approve_checkpoint(
        &self,
        workflow_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError> {
        let result = self
            .application
            .approve_workflow_checkpoint(
                workflow_id,
                &request.actor_id,
                request.state_digest.as_deref(),
                request.action_digest.as_deref(),
                request.reason.as_deref(),
            )
            .map_err(port_error)?;
        Ok(self.run_document(&result))
    }

    fn reject_checkpoint(
        &self,
        workflow_id: &str,
        request: &CheckpointDecisionRequest,
    ) -> Result<WireValue, ApplicationError> {
        let result = self
            .application
            .reject_workflow_checkpoint(
                workflow_id,
                &request.actor_id,
                request.state_digest.as_deref(),
                request.action_digest.as_deref(),
                request.reason.as_deref(),
            )
            .map_err(port_error)?;
        Ok(self.run_document(&result))
    }

    fn list_events(&self, query: &EventQuery) -> Result<WireValue, ApplicationError> {
        let page = self
            .application
            .list_events(
                query.after_sequence,
                query.limit,
                query.workflow_id.as_deref(),
                query.event_type.as_deref(),
                query.aggregate_id.as_deref(),
            )
            .map_err(port_error)?;
        Ok(page.to_wire_value())
    }

    fn closed(&self) -> bool {
        self.application.closed()
    }

    fn close(&self) {
        let _ = self.application.close();
    }
}

/// One text of an answer.
fn text(value: &str) -> WireValue {
    WireValue::Text(kala_contracts::Text::new(value))
}

/// Map a failure of the composition root onto the failure of the transport.
///
/// The codes and the statuses are the ones the Python error handlers map
/// onto, and the message of the failure is carried as it is.
fn port_error(error: kala_application::ApplicationError) -> ApplicationError {
    let kind = match error.kind() {
        ErrorKind::Application => ApplicationErrorKind::Other,
        ErrorKind::UnknownDomain => ApplicationErrorKind::UnknownDomain,
        ErrorKind::InvalidIdentifier => ApplicationErrorKind::InvalidIdentifier,
        ErrorKind::ReservedIdentifier => ApplicationErrorKind::ReservedIdentifier,
        ErrorKind::InvalidParameters => ApplicationErrorKind::InvalidParameters,
        ErrorKind::UnknownRevision => ApplicationErrorKind::UnknownRevision,
        ErrorKind::UnknownWorkflow => ApplicationErrorKind::UnknownWorkflow,
        ErrorKind::ProvenanceUnavailable => ApplicationErrorKind::ProvenanceUnavailable,
        ErrorKind::CheckpointNotPending => ApplicationErrorKind::CheckpointNotPending,
        ErrorKind::CheckpointDecisionDenied => ApplicationErrorKind::CheckpointDecisionDenied,
        ErrorKind::CheckpointDigestMismatch => ApplicationErrorKind::CheckpointDigestMismatch,
        ErrorKind::RevisionCommitRejected => ApplicationErrorKind::RevisionCommitRejected,
        ErrorKind::RevisionForkRefused => ApplicationErrorKind::RevisionForkRefused,
        ErrorKind::RevisionHeadMoved => ApplicationErrorKind::RevisionHeadMoved,
        ErrorKind::UnknownPreparedRevision => ApplicationErrorKind::UnknownPreparedRevision,
        ErrorKind::CorruptKernelState => ApplicationErrorKind::CorruptKernelState,
        ErrorKind::WorkflowFinished => ApplicationErrorKind::WorkflowFinished,
        ErrorKind::ClosedApplication => ApplicationErrorKind::ClosedApplication,
    };
    ApplicationError::new(kind, error.message())
}

// ---------------------------------------------------------------------------
// The launch entry point
// ---------------------------------------------------------------------------

/// The signal that stopped a launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopSignal {
    /// `SIGINT`, the interrupt of a terminal.
    Interrupt,
    /// `SIGTERM`, the request to terminate.
    Terminate,
}

/// The facts of a launch that stopped.
///
/// The token itself is not here: the caller passes it to the launch, and
/// neither the summary nor its `Debug` form may carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchSummary {
    /// The address the launch bound.
    pub host: String,
    /// The port the launch bound.
    pub port: u16,
    /// The path the launch token was stored at.
    pub token_path: PathBuf,
}

impl LaunchSummary {
    /// The `http://host:port` URL, bracketing an IPv6 literal.
    #[must_use]
    pub fn url(&self) -> String {
        let authority = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!("http://{authority}:{}", self.port)
    }
}

/// The outcome of a launch that stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchOutcome {
    /// The facts of the launch.
    pub summary: LaunchSummary,
    /// The signal that stopped it.
    pub stopped_by: StopSignal,
}

/// A failure of a launch, after the composition root exists.
#[derive(Debug)]
pub enum LaunchError {
    /// The caller named an address the launch may not bind.
    Address(String),
    /// The router or the request boundary could not be built.
    State(String),
    /// The runtime could not be built, or a signal handler could not be
    /// installed.
    Runtime(String),
    /// The listening socket could not be bound.
    Bind {
        /// The address the caller named.
        host: String,
        /// The port the caller named.
        port: u16,
        /// The failure the operating system reported.
        source: std::io::Error,
    },
    /// The accept loop failed.
    Server(String),
}

impl LaunchError {
    /// The machine readable code the entry point reports.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Address(_) => "invalid_address",
            Self::State(_) => "invalid_configuration",
            Self::Runtime(_) | Self::Bind { .. } | Self::Server(_) => "local_failure",
        }
    }

    /// The message of the failure.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Address(message) | Self::State(message) | Self::Runtime(message) => {
                message.clone()
            }
            Self::Bind { source, .. } => source.to_string(),
            Self::Server(message) => message.clone(),
        }
    }
}

/// Serve the routes of the accepted `kala-api` crate over one composition root.
///
/// The caller has already validated the address, opened the composition root
/// and minted the launch token; this function binds the loopback address,
/// serves until `SIGINT` or `SIGTERM`, stops accepting, drains the in-flight
/// connections, and closes the composition root through the lifespan of the
/// launch. It prints nothing and mints nothing.
///
/// # Errors
///
/// Fails when the address is not a loopback address, when the router cannot
/// be built, when the socket cannot be bound, or when the accept loop fails
/// fatally. The composition root is left open on a failure, exactly as the
/// binary leaves it.
pub fn launch(
    application: Arc<KalaApplication>,
    host: &str,
    port: u16,
    api_token: &str,
) -> Result<LaunchOutcome, LaunchError> {
    if !kala_api::boundary::is_loopback_host(host) {
        return Err(LaunchError::Address(
            "serve binds the loopback interface only; a routable or wildcard address is refused"
                .to_owned(),
        ));
    }
    if port == 0 {
        return Err(LaunchError::Address(
            "the port must be between 1 and 65535".to_owned(),
        ));
    }

    let adapter = Arc::new(LocalApplication::new(Arc::clone(&application)));
    let lifespan = Lifespan::new(Arc::clone(&adapter) as Arc<dyn Application>, true);
    let router = Arc::new(Routes::new(Arc::clone(&adapter) as Arc<dyn Application>));
    let state = ApiState::new(api_token, MAX_REQUEST_BODY_BYTES as i64, router)
        .map_err(LaunchError::State)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| LaunchError::Runtime(error.to_string()))?;

    let stopped = runtime.block_on(async move {
        // The signal handlers are installed before the socket exists, so a
        // signal that arrives while the launch is starting is handled the
        // way the Python server handles it.
        #[cfg(unix)]
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                .map_err(|error| LaunchError::Runtime(error.to_string()))?;
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|error| LaunchError::Runtime(error.to_string()))?;

        let listener = tokio::net::TcpListener::bind((host, port))
            .await
            .map_err(|source| LaunchError::Bind {
                host: host.to_owned(),
                port,
                source,
            })?;

        let recorded: Arc<Mutex<Option<StopSignal>>> = Arc::new(Mutex::new(None));
        let recorder = Arc::clone(&recorded);
        serve(Arc::new(state), listener, async move {
            #[cfg(unix)]
            let signal = tokio::select! {
                _ = interrupt.recv() => StopSignal::Interrupt,
                _ = terminate.recv() => StopSignal::Terminate,
            };
            #[cfg(not(unix))]
            let signal = {
                let _ = tokio::signal::ctrl_c().await;
                StopSignal::Interrupt
            };
            *recorder.lock().expect("the stop is recorded once") = Some(signal);
        })
        .await
        .map_err(|error| LaunchError::Server(error.to_string()))?;

        let signal = recorded
            .lock()
            .expect("the stop is recorded once")
            .take()
            .unwrap_or(StopSignal::Interrupt);
        Ok::<StopSignal, LaunchError>(signal)
    })?;

    lifespan.shutdown();

    Ok(LaunchOutcome {
        summary: LaunchSummary {
            host: host.to_owned(),
            port,
            token_path: application.settings().data_dir.join(API_TOKEN_FILE_NAME),
        },
        stopped_by: stopped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_failure_taxonomy_carries_the_python_codes() {
        let error = kala_application::ApplicationError::unknown_domain("nope");
        let port = port_error(error);
        assert_eq!(port.code, "unknown_domain");
        assert_eq!(port.kind.status(), 404);
    }
}
