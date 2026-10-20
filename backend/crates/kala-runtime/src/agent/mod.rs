//! The Kala agent runtime.
//!
//! This module is the Rust port of `kala_agent.runtime.agent`. It owns the
//! bounded planning surface, the task graph, the tool dispatch and the
//! merge of the answers.
//!
//! The module is the deterministic substrate of ADR-0003:
//!
//! * a plan is **data**: the model may only propose steps that resolve to
//!   the step catalog, with declared parameters, and the plan is validated
//!   before anything runs (D3);
//! * a tool call goes **only** through the tool executor, and a tool that
//!   needs a decision gets one from the policy engine (D5);
//! * the budget is **reserved and settled** by the executor, and the
//!   coordinator refuses a layer the budget does not admit (ADR-0006);
//! * the offline planner is deterministic, and the degrading planner only
//!   degrades when the degradation was approved in advance (ADR-0001, D8).
//!
//! # The public surface
//!
//! | Python | Rust |
//! |---|---|
//! | `AgentTask` | [`AgentTask`] |
//! | `CatalogStep` | [`CatalogStep`] |
//! | `StepCatalog` | [`StepCatalog`] |
//! | `AgentStepOutcome` | [`AgentStepOutcome`] |
//! | `AgentRunResult` | [`AgentRunResult`] |
//! | `Planner` (protocol) | [`Planner`] (trait) |
//! | `OfflinePlanner` | [`OfflinePlanner`] |
//! | `ModelPlanner` | [`ModelPlanner`] |
//! | `DegradingPlanner` | [`DegradingPlanner`] |
//! | `AgentCoordinator` | [`AgentCoordinator`] |
//! | `AgentError` and its subclasses | [`AgentError`] (one enum) |
//! | `PlannerFailure` | [`AgentError::PlannerFailure`] |
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use kala_runtime::agent::{AgentTask, StepCatalog, CatalogStep};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = StepCatalog::new(vec![CatalogStep::new("simulate", "mechanical.reference-evaluator")?])?;
//! let task = AgentTask::new("step-1", "mechanical.reference-evaluator")?;
//! # let _ = (catalog, task, Arc::new(()));
//! # Ok(())
//! # }
//! ```

pub mod coordinator;
pub mod error;
pub mod outcome;
pub mod planner;
pub mod task;
pub mod value;

pub use coordinator::{
    APPROVAL_REQUIRED_EFFECTS, AgentCoordinator, MAX_MERGED_RESULT_BYTES, MAX_PARALLEL_TASKS,
};
pub use error::{
    AgentError, AgentMergeError, AgentPlanError, AgentPolicyError, NEVER_DEGRADED_ERRORS,
    PlannerFailure,
};
pub use outcome::{AgentRunResult, AgentStepOutcome};
pub use planner::{DegradingPlanner, ModelPlanner, OfflinePlanner, Planner};
pub use task::{
    AgentTask, CatalogStep, MAX_AGENT_TASKS, StepCatalog, dependency_layers, dependency_order,
    plan_parameters, validate_task_graph,
};
pub use value::{
    MAX_ARGUMENT_DEPTH, MAX_BOUNDED_TEXT, MAX_ERROR_TEXT, bounded_identifier, bounded_reasons,
    bounded_seconds, bounded_text, freeze_object, freeze_value, identifier_tuple, thaw,
    thaw_object, truncate_code_points,
};
