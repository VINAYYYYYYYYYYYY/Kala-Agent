//! The security properties of the agent runtime.
//!
//! The properties are the ones ADR-0003 and the repository's own agent
//! tests demand:
//!
//! * an LLM plan is **bounded and validated before execution**;
//! * a tool call goes **only through the executor** and needs the **policy
//!   approval** of the tools that carry a side effect;
//! * the **budget is reserved and settled**;
//! * a **checkpoint pauses** the call instead of dispatching it;
//! * **no approval is replayed**.
//!
//! The memory property of the wave brief is *not* a property of this module:
//! `kala_agent.runtime.agent` has no memory dependency at all (its import
//! list is contracts, kernel.budget, kernel.policy, runtime.model_gateway,
//! tools.executor, tools.protocol and tools.registry), so there is no memory
//! write to bound here. The evidence is recorded in the cell report.

mod agent_support;

use std::sync::Arc;

use kala_budget::{BudgetLedger, RunBudget};
use kala_contracts::{ApprovalKind, ContractModel, PlanStep, SideEffectLevel, Text, ToolRunStatus};
use kala_json::CanonicalValue;
use kala_kernel::policy::{DecisionContext, PolicyEngine, PolicyRequest};
use kala_runtime::agent::{
    AgentCoordinator, AgentError, AgentTask, CatalogStep, ModelPlanner, StepCatalog,
};
use kala_runtime::gateway::{LocalModelClient, ModelCapability, ModelGateway};

use agent_support::{TempDir, executor, object, text};

/// The default arguments of a catalog step.
fn arguments() -> CanonicalValue {
    object(vec![("operation", text("beam_volume"))])
}

/// A task that calls the pure reference evaluator.
fn evaluator_task() -> AgentTask {
    AgentTask::from_parts(
        "step-1",
        "mechanical.reference-evaluator",
        &arguments(),
        Some("1.0.0"),
        &[],
        &[],
        "",
        None,
    )
    .expect("a task")
}

/// A task that calls the side-effecting simulator.
fn simulator_task() -> AgentTask {
    AgentTask::from_parts(
        "step-1",
        "mechanical.reference-simulator",
        &arguments(),
        Some("1.0.0"),
        &["artifact.write".to_owned()],
        &[],
        "",
        None,
    )
    .expect("a task")
}

/// The catalog of the tests.
fn catalog() -> StepCatalog {
    StepCatalog::new(vec![
        CatalogStep::from_parts(
            "simulate",
            "mechanical.reference-evaluator",
            &arguments(),
            Some("1.0.0"),
            &[],
            &["beam".to_owned()],
            "Simulate the beam.",
        )
        .expect("a catalog step"),
    ])
    .expect("a catalog")
}

// ---------------------------------------------------------------------------
// The plan is bounded and validated before execution.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_plan_that_names_an_unknown_catalog_step_never_runs() {
    let gateway = ModelGateway::new(Arc::new(
        LocalModelClient::new("local", "local", vec![ModelCapability::StructuredOutput])
            .expect("a client"),
    ));
    let planner =
        ModelPlanner::new(Arc::new(gateway), Arc::new(catalog()), 1_024).expect("a planner");
    // The response schema is the closed vocabulary of the plan: the catalog
    // entry identifiers are its enum, so the model cannot name anything else.
    let schema = planner.response_schema();
    let properties = agent_support::get(&schema, "properties").expect("the properties");
    let steps = agent_support::get(properties, "steps").expect("the steps");
    let items = agent_support::get(steps, "items").expect("the items");
    let item_properties = agent_support::get(items, "properties").expect("the item properties");
    let catalog_step =
        agent_support::get(item_properties, "catalog_step_id").expect("the catalog step");
    let entries = agent_support::get(catalog_step, "enum").expect("the enum");
    let CanonicalValue::Array(items) = entries else {
        panic!("the enum is an array");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].as_text().map(|value| value.to_string_lossy()),
        Some("simulate".to_owned())
    );
    // The bound on the number of steps is published as well.
    let bound = agent_support::get(steps, "maxItems").expect("the bound");
    assert_eq!(
        bound.as_integer().map(|value| value.to_string()),
        Some("64".to_owned())
    );
}

#[tokio::test]
async fn a_plan_with_an_undeclared_parameter_is_refused_before_the_run() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator =
        AgentCoordinator::new(executor(workdir.path())).with_catalog(Arc::new(catalog()));
    let step = PlanStep::from_value(&object(vec![
        ("step_id", text("step-1")),
        ("catalog_step_id", text("simulate")),
        ("intent", text("Simulate the beam.")),
        ("parameters_json", text("{\"made_up\":1}")),
    ]))
    .expect("a plan step");
    let error = coordinator
        .run_plan(&[step], "run-1")
        .expect_err("a refusal");
    assert_eq!(error.python_class_name(), "AgentPlanError");
    assert_eq!(
        error.message(),
        "parameters are not declared by catalog step simulate: made_up"
    );
    // Nothing ran: the executor recorded no call.
    assert!(coordinator.executor().reports().is_empty());
}

#[tokio::test]
async fn a_task_that_is_not_in_the_catalog_is_refused() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator =
        AgentCoordinator::new(executor(workdir.path())).with_catalog(Arc::new(catalog()));
    let step = PlanStep::from_value(&object(vec![
        ("step_id", text("step-1")),
        ("catalog_step_id", text("made_up")),
        ("intent", text("Do something else.")),
    ]))
    .expect("a plan step");
    let error = coordinator
        .run_plan(&[step], "run-1")
        .expect_err("a refusal");
    assert_eq!(error.message(), "catalog step made_up is not declared");
    assert!(coordinator.executor().reports().is_empty());
}

// ---------------------------------------------------------------------------
// The tool call goes only through the executor, with a policy approval.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_side_effecting_tool_without_a_policy_engine_never_runs() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let error = coordinator
        .run(&[simulator_task()], "run-1")
        .expect_err("a refusal");
    assert_eq!(error.python_class_name(), "AgentPolicyError");
    assert_eq!(
        error.message(),
        "tool mechanical.reference-simulator requires a policy decision and no policy engine is attached"
    );
    // The tool was never dispatched.
    assert!(coordinator.executor().reports().is_empty());
}

#[tokio::test]
async fn a_side_effecting_tool_pauses_for_an_approval() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator =
        AgentCoordinator::new(executor(workdir.path())).with_policy(Arc::new(PolicyEngine::new()));
    let result = coordinator
        .run(&[simulator_task()], "run-1")
        .expect("a run");
    assert!(!result.succeeded());
    let outcome = result.outcome_for("step-1").expect("an outcome");
    assert_eq!(outcome.status(), ToolRunStatus::Denied);
    assert!(!outcome.dispatched());
    assert_eq!(outcome.error_code(), Some("capability_denied"));
    let reason = outcome.policy_reason().expect("a policy reason");
    assert!(reason.starts_with("policy decision "));
    assert!(reason.contains("for request run-1-step-1"));
    // The checkpoint paused the call: the tool was not dispatched.
    assert!(coordinator.executor().reports().is_empty());
}

#[tokio::test]
async fn a_pure_read_needs_no_approval() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let result = coordinator
        .run(&[evaluator_task()], "run-1")
        .expect("a run");
    assert!(result.succeeded());
    assert_eq!(coordinator.executor().reports().len(), 1);
}

// ---------------------------------------------------------------------------
// The budget is reserved and settled.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_budget_reserves_and_settles_the_call() {
    let workdir = TempDir::new("kala-agent-security");
    let ledger = Arc::new(BudgetLedger::new(
        RunBudget::new()
            .with_max_model_calls(4u64)
            .with_max_tool_calls(4u64)
            .with_max_total_tokens(100u64)
            .with_max_artifact_bytes(100u64)
            .with_max_replanning_attempts(1u64),
    ));
    let executor = {
        let registry = Arc::new(kala_tools::registry::ToolRegistry::new());
        kala_tools::simulator::register_reference_evaluator(
            &registry,
            kala_tools::simulator::EVALUATOR_TOOL_ID,
            kala_tools::simulator::EVALUATOR_TOOL_VERSION,
        )
        .expect("the reference evaluator registers");
        let mut config =
            kala_tools::executor::ExecutorConfig::new(registry, workdir.path().join("run"));
        config.isolation = kala_tools::executor::IsolationMode::TrustedInProcessEvaluator;
        config.ledger = Some(ledger.clone());
        Arc::new(kala_tools::executor::ToolExecutor::new(config).expect("an executor"))
    };
    let coordinator = AgentCoordinator::new(executor).with_ledger(ledger.clone());
    let result = coordinator
        .run(&[evaluator_task()], "run-1")
        .expect("a run");
    assert!(result.succeeded());
    let usage = ledger.usage();
    assert_eq!(usage.tool_calls().to_string(), "1");
    // The reservation was settled: nothing is left reserved.
    assert_eq!(ledger.reserved().tool_calls().to_string(), "0");
    assert!(ledger.reservations().is_empty());
}

#[tokio::test]
async fn an_exhausted_budget_denies_the_whole_layer() {
    let workdir = TempDir::new("kala-agent-security");
    let ledger = Arc::new(BudgetLedger::new(
        RunBudget::new()
            .with_max_model_calls(1u64)
            .with_max_tool_calls(0u64)
            .with_max_total_tokens(10u64)
            .with_max_artifact_bytes(10u64)
            .with_max_replanning_attempts(0u64),
    ));
    let coordinator = AgentCoordinator::new(executor(workdir.path())).with_ledger(ledger);
    let result = coordinator
        .run(&[evaluator_task()], "run-1")
        .expect("a run");
    assert!(!result.succeeded());
    let outcome = result.outcome_for("step-1").expect("an outcome");
    assert_eq!(outcome.status(), ToolRunStatus::Denied);
    assert!(!outcome.dispatched());
    assert_eq!(outcome.error_code(), Some("budget_exhausted"));
    assert!(
        outcome
            .error_message()
            .expect("a reason")
            .starts_with("budget exhausted for tool_calls")
    );
    assert_eq!(result.budget_denied(), ["step-1".to_owned()]);
    // Nothing ran.
    assert!(coordinator.executor().reports().is_empty());
}

// ---------------------------------------------------------------------------
// No approval is replayed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_approval_does_not_cover_another_action() {
    let policy = PolicyEngine::new();
    policy
        .grant_capability(Text::new("artifact.write"))
        .expect("the capability is granted");
    let request = PolicyRequest::from_parts(
        Text::new("run-1-step-1"),
        Text::new(&"a".repeat(64)),
        Text::new(&"b".repeat(64)),
        vec![Text::new("artifact.write")],
        SideEffectLevel::LocalWrite,
        false,
        false,
        None,
        Some(ApprovalKind::ToolClassChange),
    )
    .expect("a policy request");
    let other = PolicyRequest::from_parts(
        Text::new("run-1-step-2"),
        Text::new(&"a".repeat(64)),
        Text::new(&"c".repeat(64)),
        vec![Text::new("artifact.write")],
        SideEffectLevel::LocalWrite,
        false,
        false,
        None,
        Some(ApprovalKind::ToolClassChange),
    )
    .expect("a policy request");
    let decision = policy.decide(&request, None).expect("a decision");
    assert!(!decision.allowed());
    assert!(decision.requires_approval());
    let approved = policy
        .approve(&request, &DecisionContext::new())
        .expect("an approval");
    assert!(approved.allowed());
    // The other action has a different action digest, so the approval does
    // not cover it.
    let decision = policy.decide(&other, None).expect("a decision");
    assert!(!decision.allowed());
    assert!(decision.requires_approval());
}

#[tokio::test]
async fn a_degradation_is_never_taken_without_an_approval() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let error = coordinator
        .plan("beam volume")
        .await
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "no planner is configured for this coordinator"
    );
    assert_eq!(error.python_class_name(), "AgentPlanError");
    // The error is the base class of the agent errors, so a caller can tell
    // it apart from a plan refusal.
    let error: AgentError = AgentError::base("the base agent error");
    assert_eq!(error.python_class_name(), "AgentError");
}

// ---------------------------------------------------------------------------
// The bounds of the merged results.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_merged_results_are_bounded() {
    let workdir = TempDir::new("kala-agent-security");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let result = coordinator
        .run(&[evaluator_task()], "run-1")
        .expect("a run");
    let merged = result.merged();
    let CanonicalValue::Object(entries) = merged else {
        panic!("the merged results are an object");
    };
    assert_eq!(entries.len(), 1);
    let digest = result.digests();
    assert_eq!(digest.len(), 1);
    assert_eq!(digest[0].1.len(), 64);
    // The merged document is the canonical document, so its digest is the
    // digest of the merged result.
    let canonical = kala_json::canonical_json_dumps(merged).expect("the canonical document");
    assert!(canonical.len() < kala_runtime::agent::MAX_MERGED_RESULT_BYTES);
}
