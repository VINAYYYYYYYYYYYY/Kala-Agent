//! Native Kala runtime: the provider-neutral model gateway and the agent
//! runtime that plans and dispatches the tools.
//!
//! This crate is the Rust port of `kala_agent.runtime.model_gateway` and
//! `kala_agent.runtime.agent`. It consumes the accepted `kala-kernel` policy
//! engine, `kala-budget` ledger and decimal type, `kala-contracts` enums,
//! `kala-json` canonical JSON and `kala-tools` protocol, registry and
//! executor read-only. It never invokes Python at runtime; the live Python
//! differential is test-only.
//!
//! The gateway is the single egress choke point of the Kala kernel
//! (ADR-0001, D2): a hosted provider call needs an explicit consent, a
//! per-request policy approval and a recorded egress manifest, and a design
//! binary never leaves the machine.
//!
//! The agent runtime is the deterministic substrate of ADR-0003: a plan is
//! data that is validated before it runs, a tool call goes only through the
//! executor, and the budget is reserved and settled.
//!
//! ```no_run
//! use std::sync::Arc;
//! use kala_runtime::gateway::{
//!     LocalModelClient, ModelCapability, ModelGateway, ModelMessage, ModelRole,
//! };
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Arc::new(LocalModelClient::new(
//!     "local-deterministic",
//!     "local",
//!     vec![ModelCapability::StructuredOutput, ModelCapability::Seed],
//! )?);
//! let gateway = ModelGateway::new(client);
//! let request = kala_runtime::gateway::ModelRequest::new(
//!     "request-1",
//!     vec![ModelMessage::new(ModelRole::User, "summarize the run")?],
//! )?;
//! let response = gateway.complete(&request).await?;
//! assert_eq!(response.provider_id(), "local");
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

pub mod agent;
pub mod gateway;

pub use agent::{
    APPROVAL_REQUIRED_EFFECTS, AgentCoordinator, AgentError, AgentMergeError, AgentPlanError,
    AgentPolicyError, AgentRunResult, AgentStepOutcome, AgentTask, CatalogStep, DegradingPlanner,
    MAX_AGENT_TASKS, MAX_BOUNDED_TEXT, MAX_ERROR_TEXT, MAX_MERGED_RESULT_BYTES, MAX_PARALLEL_TASKS,
    ModelPlanner, NEVER_DEGRADED_ERRORS, OfflinePlanner, Planner, PlannerFailure, StepCatalog,
};
pub use gateway::error::{GatewayError, ModelError};
pub use gateway::gateway::ModelGateway;
pub use gateway::model::{
    EGRESS_CAPABILITIES, LOCAL_PROVIDER_ID, ModelArtifact, ModelCapability, ModelEgressManifest,
    ModelFinishReason, ModelMessage, ModelRedaction, ModelRequest, ModelResponse, ModelRole,
    ModelTranscript, OPENAI_COMPATIBLE_PROVIDER_ID,
};
pub use gateway::redact::{is_secret_name, redact_secrets, truncate_encoded};
