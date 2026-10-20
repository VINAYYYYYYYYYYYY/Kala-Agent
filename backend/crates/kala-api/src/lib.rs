//! The Kala core API connection layer.
//!
//! This crate is the transport's part of the local Kala API: the loopback
//! Host guard, the per-launch bearer token, the cross-origin guard, the
//! bounded request body, the fixed security header set, the sanitized error
//! envelope, and the local configuration the transport reads.
//!
//! It is a port of the Python modules `kala_agent.config`,
//! `kala_agent.api.boundary`, `kala_agent.api.envelope`,
//! `kala_agent.api.middleware` and `kala_agent.api.errors`, and of the parts
//! of `kala_agent.api` they rely on. The routes, the dependencies and the
//! application composition are not here: they belong to a later cell.
//!
//! The crate depends on no Python and no production subprocess. Python is
//! used only by the integration tests, as an oracle.
//!
//! # The trust model
//!
//! One local operator, no multi-user isolation, loopback only. A request is
//! served only when its Host header names the loopback interface, which is
//! what blocks a rebound name, and every mutating method must present the
//! per-launch bearer token that this process stored in the local data
//! directory. Reads stay open to a caller that already has that directory.
//! The token is never logged and never returned; a refusal says only that a
//! valid token is required.
//!
//! # The two error layouts
//!
//! The Python source writes two different bodies for the same envelope, and
//! this crate reproduces both byte for byte:
//!
//! * a transport refusal is `{"details":[..],"error":{"code":..,"message":..}}`,
//!   the sorted layout `json.dumps(..., sort_keys=True)` produces;
//! * a framework or routing error is
//!   `{"error":{"code":..,"message":..},"details":[..]}`, the ordered layout
//!   `json.dumps(..., ensure_ascii=False)` produces.

#![forbid(unsafe_code)]

pub mod boundary;
pub mod config;
pub mod datetime;
pub mod envelope;
pub mod errors;
pub mod jsoninput;
pub mod middleware;
pub mod patterns;
pub mod probe;
pub mod pytext;
pub mod routes;
pub mod schemas;
pub mod server;
pub mod validation;
pub mod views;
pub mod wire;

pub use boundary::{Boundary, Refusal};
pub use config::{SettingValue, Settings, SettingsError};
pub use envelope::{ErrorDetail, ErrorResponse, error_response};
pub use probe::ProbeRouter;
pub use routes::{Application, EventQuery, Lifespan, Routes};
pub use schemas::{
    ApiModel, CheckpointDecisionRequest, CheckpointListResponse, CreateRevisionRequest,
    DomainListResponse, DomainName, EvaluateDomainRequest, EvaluationResponse, HealthResponse,
    PrepareRevisionRequest, RevisionListResponse, RunMechanicalWorkflowRequest, RunResponse,
};
pub use server::{ApiState, Router, handle_request};
pub use validation::{Failures, Issue, Location};
