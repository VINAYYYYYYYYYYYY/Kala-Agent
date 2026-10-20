//! The semantics of the agent runtime.
//!
//! The tests mirror `tests/unit/test_agent_branches.py` of the repository:
//! every public function of the Python module is exercised here with the
//! same values and the same expectations.

mod agent_support;

use std::sync::Arc;

use kala_contracts::{ContractModel, PlanStep, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_runtime::agent::{
    AgentCoordinator, AgentError, AgentRunResult, AgentStepOutcome, AgentTask, CatalogStep,
    DegradingPlanner, MAX_AGENT_TASKS, MAX_BOUNDED_TEXT, MAX_ERROR_TEXT, MAX_PARALLEL_TASKS,
    ModelPlanner, OfflinePlanner, Planner, StepCatalog, bounded_identifier, bounded_reasons,
    bounded_seconds, bounded_text, dependency_layers, dependency_order, freeze_object,
    freeze_value, identifier_tuple, plan_parameters, thaw, thaw_object, validate_task_graph,
};
use kala_runtime::gateway::{GatewayTimestamp, ModelError, ModelResponse};

use agent_support::{TempDir, executor, object, text};

/// The default arguments of a catalog step.
fn arguments() -> CanonicalValue {
    object(vec![("operation", text("beam_volume"))])
}

/// A catalog step of the tests.
fn catalog_step() -> CatalogStep {
    CatalogStep::from_parts(
        "simulate",
        "mechanical.reference-evaluator",
        &arguments(),
        Some("1.0.0"),
        &[],
        &["beam".to_owned(), "volume".to_owned()],
        "Simulate the beam.",
    )
    .expect("the catalog step is valid")
}

/// A task of the tests.
fn task(step_id: &str) -> AgentTask {
    AgentTask::from_parts(
        step_id,
        "mechanical.reference-evaluator",
        &arguments(),
        Some("1.0.0"),
        &[],
        &[],
        "",
        None,
    )
    .expect("the task is valid")
}

/// The catalog of the tests.
///
/// The Python test catalog holds two entries with the same keywords, so both
/// of them score against the objective.
fn catalog() -> StepCatalog {
    StepCatalog::new(vec![
        catalog_step(),
        CatalogStep::from_parts(
            "summarize",
            "mechanical.reference-evaluator",
            &arguments(),
            Some("1.0.0"),
            &[],
            &["beam".to_owned(), "volume".to_owned()],
            "Simulate the beam.",
        )
        .expect("the catalog step is valid"),
    ])
    .expect("the catalog is valid")
}

// ---------------------------------------------------------------------------
// The bounded validators.
// ---------------------------------------------------------------------------

#[test]
fn the_bounded_identifier_rejects_bad_values() {
    let error = bounded_identifier("has space", "step_id").expect_err("a space is refused");
    assert_eq!(error.message(), "step_id must be a safe identifier");
    assert_eq!(error.python_class_name(), "AgentPlanError");
    let error = bounded_identifier("", "step_id").expect_err("an empty value is refused");
    assert_eq!(error.message(), "step_id must be a safe identifier");
    let error = bounded_identifier("a/b", "step_id").expect_err("a slash is refused");
    assert_eq!(error.message(), "step_id must be a safe identifier");
    assert_eq!(
        bounded_identifier("  step-1  ", "step_id").expect("a safe identifier"),
        "step-1"
    );
    assert_eq!(
        bounded_identifier("A.b:c-1", "step_id").expect("a safe identifier"),
        "A.b:c-1"
    );
}

#[test]
fn the_bounded_text_rejects_an_oversized_value() {
    let error = bounded_text(&"x".repeat(600), "intent", MAX_BOUNDED_TEXT)
        .expect_err("an oversized value is refused");
    assert_eq!(error.message(), "intent exceeds the bounded length");
    assert_eq!(bounded_text("x", "intent", 1).expect("a bounded text"), "x");
}

#[test]
fn the_bounded_seconds_accepts_none_and_bounds_the_values() {
    assert_eq!(
        bounded_seconds(None, "timeout_seconds").expect("none"),
        None
    );
    assert_eq!(
        bounded_seconds(Some(5.0), "timeout_seconds").expect("a bound"),
        Some(5.0)
    );
    for refused in [Some(0.0), Some(3_601.0), Some(f64::NAN), Some(-1.0)] {
        let error = bounded_seconds(refused, "timeout_seconds").expect_err("a refusal");
        assert_eq!(
            error.message(),
            "timeout_seconds must be positive and bounded"
        );
    }
}

#[test]
fn the_identifier_tuple_sorts_and_deduplicates() {
    assert_eq!(
        identifier_tuple(&["b".to_owned(), "a".to_owned(), "a".to_owned()], "caps")
            .expect("a tuple"),
        vec!["a".to_owned(), "b".to_owned()]
    );
    let error = identifier_tuple(&["a b".to_owned()], "caps").expect_err("a refusal");
    assert_eq!(error.message(), "caps must be a safe identifier");
}

#[test]
fn the_bounded_reasons_validates_every_entry() {
    assert_eq!(
        bounded_reasons(&["because".to_owned()], "reasons").expect("reasons"),
        vec!["because".to_owned()]
    );
    let error = bounded_reasons(&["x".repeat(2_000)], "reasons").expect_err("a refusal");
    assert_eq!(error.message(), "reasons entry exceeds the bounded length");
    assert_eq!(MAX_ERROR_TEXT, 1_024);
}

#[test]
fn the_freeze_accepts_the_json_scalars_and_containers() {
    assert!(matches!(
        freeze_value(&CanonicalValue::Null),
        Ok(CanonicalValue::Null)
    ));
    assert!(matches!(
        freeze_value(&CanonicalValue::Bool(true)),
        Ok(CanonicalValue::Bool(true))
    ));
    assert!(matches!(
        freeze_value(&CanonicalValue::text("text")),
        Ok(CanonicalValue::Text(_))
    ));
    let frozen = freeze_value(&object(vec![("a", CanonicalValue::integer(1))])).expect("a freeze");
    assert!(matches!(frozen, CanonicalValue::Object(_)));
}

#[test]
fn the_freeze_refuses_a_non_finite_number() {
    let error = freeze_value(&CanonicalValue::Float(f64::NAN)).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "task arguments must contain finite numbers"
    );
    let error = freeze_value(&CanonicalValue::Float(f64::INFINITY)).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "task arguments must contain finite numbers"
    );
}

#[test]
fn the_freeze_refuses_an_empty_key_and_a_deep_document() {
    let mut map = CanonicalObject::new();
    map.insert(JsonText::new(""), CanonicalValue::integer(1));
    let error = freeze_value(&CanonicalValue::Object(map)).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "task argument keys must be nonempty strings"
    );

    let mut deep = CanonicalValue::integer(1);
    for _ in 0..20 {
        let mut map = CanonicalObject::new();
        map.insert(JsonText::new("a"), deep);
        deep = CanonicalValue::Object(map);
    }
    let error = freeze_value(&deep).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "task arguments exceed the supported nesting depth"
    );
}

#[test]
fn the_freeze_object_requires_an_object() {
    let error =
        freeze_object(&CanonicalValue::Array(Vec::new()), "arguments").expect_err("a refusal");
    assert_eq!(error.message(), "arguments must be a JSON object");
    let mut map = CanonicalObject::new();
    map.insert(JsonText::new(""), CanonicalValue::integer(1));
    let error = freeze_object(&CanonicalValue::Object(map), "arguments").expect_err("a refusal");
    assert_eq!(error.message(), "arguments keys must be nonempty strings");
}

#[test]
fn the_thaw_round_trips_the_frozen_containers() {
    let frozen = freeze_value(&object(vec![(
        "a",
        CanonicalValue::Array(vec![CanonicalValue::integer(1)]),
    )]))
    .expect("a freeze");
    assert!(matches!(thaw(&frozen), CanonicalValue::Object(_)));
    assert!(matches!(thaw_object(&frozen), CanonicalValue::Object(_)));
}

// ---------------------------------------------------------------------------
// The task and the catalog.
// ---------------------------------------------------------------------------

#[test]
fn the_task_validates_every_field() {
    let error = AgentTask::from_parts(
        "bad id",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &[],
        "",
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "step_id must be a safe identifier");

    let error = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Array(Vec::new()),
        None,
        &[],
        &[],
        "",
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "arguments must be a JSON object");

    let error = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &["step-1".to_owned()],
        "",
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "an agent task cannot depend on itself");

    let error = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &[],
        &"x".repeat(600),
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "intent exceeds the bounded length");

    let error = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &[],
        "",
        Some(-1.0),
    )
    .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "timeout_seconds must be positive and bounded"
    );
}

#[test]
fn the_task_exposes_its_digests_and_its_call_identifier() {
    let task = task("step-1");
    assert_eq!(
        task.arguments_json().expect("the canonical arguments"),
        "{\"operation\":\"beam_volume\"}"
    );
    assert_eq!(task.arguments_digest().expect("the digest").len(), 64);
    assert_eq!(
        task.call_id("run-1").expect("the call identifier"),
        "run-1-step-1"
    );
    let error = task.call_id("bad run").expect_err("a refusal");
    assert_eq!(error.message(), "run_id must be a safe identifier");
    let replaced = task
        .with_arguments(&object(vec![("operation", text("bar_summary"))]))
        .expect("a replacement");
    assert_eq!(
        agent_support::get(replaced.arguments(), "operation")
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy()),
        Some("bar_summary".to_owned())
    );
}

#[test]
fn the_catalog_step_merges_the_declared_parameters() {
    let step = catalog_step();
    let task = step
        .to_task(
            "step-1",
            Some(&object(vec![("operation", text("bar_summary"))])),
            &[],
        )
        .expect("a task");
    assert_eq!(task.tool_id(), "mechanical.reference-evaluator");
    assert_eq!(task.tool_version(), Some("1.0.0"));
    assert_eq!(task.intent(), "Simulate the beam.");
    let task = step.to_task("step-1", None, &[]).expect("a task");
    assert_eq!(
        agent_support::get(task.arguments(), "operation")
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy()),
        Some("beam_volume".to_owned())
    );
}

#[test]
fn the_catalog_step_refuses_an_undeclared_parameter() {
    let error = catalog_step()
        .to_task(
            "step-1",
            Some(&object(vec![("made_up", CanonicalValue::integer(1))])),
            &[],
        )
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "parameter 'made_up' is not declared by catalog step simulate"
    );
}

#[test]
fn the_catalog_validates_its_configuration() {
    let error = StepCatalog::with_max_steps(Vec::new(), 0).expect_err("a refusal");
    assert_eq!(error.message(), "max_steps must be a positive integer");
    let error = StepCatalog::new(vec![catalog_step(), catalog_step()]).expect_err("a refusal");
    assert_eq!(error.message(), "catalog step simulate is declared twice");
    let error = StepCatalog::with_max_steps(
        vec![
            CatalogStep::new("a", "tool").expect("a step"),
            CatalogStep::new("b", "tool").expect("a step"),
        ],
        1,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "the step catalog exceeds 1 entries");
}

#[test]
fn the_catalog_exposes_its_entries() {
    let catalog = catalog();
    assert_eq!(catalog.len(), 2);
    assert!(catalog.contains("simulate"));
    assert!(!catalog.contains("absent"));
    assert_eq!(
        catalog.step_ids(),
        vec!["simulate".to_owned(), "summarize".to_owned()]
    );
    assert_eq!(
        catalog
            .require("  simulate  ")
            .expect("an entry")
            .catalog_step_id(),
        "simulate"
    );
    let error = catalog.require("absent").expect_err("a refusal");
    assert_eq!(error.message(), "catalog step absent is not declared");
}

#[test]
fn the_catalog_refuses_an_oversized_plan() {
    let catalog = catalog();
    let error = catalog.to_tasks(&[]).expect_err("a refusal");
    assert_eq!(error.message(), "a plan must contain at least one step");
    let steps: Vec<PlanStep> = (0..3)
        .map(|index| {
            PlanStep::from_value(&object(vec![
                ("step_id", text(&format!("step-{index}"))),
                ("catalog_step_id", text("simulate")),
                ("intent", text("Simulate the beam.")),
            ]))
            .expect("a plan step")
        })
        .collect();
    let error = catalog.to_tasks(&steps).expect_err("a refusal");
    assert_eq!(error.message(), "the plan exceeds the bounded step count");
}

#[test]
fn the_plan_parameters_reject_a_malformed_document() {
    let error = plan_parameters("{not json", &catalog_step()).expect_err("a refusal");
    assert!(
        error
            .message()
            .starts_with("plan parameters are not strict JSON")
    );
    let error = plan_parameters("[]", &catalog_step()).expect_err("a refusal");
    assert_eq!(error.message(), "plan parameters must be a JSON object");
    let error = plan_parameters("{\"made_up\":1}", &catalog_step()).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "parameters are not declared by catalog step simulate: made_up"
    );
}

// ---------------------------------------------------------------------------
// The task graph.
// ---------------------------------------------------------------------------

#[test]
fn the_task_graph_rejects_a_bad_graph() {
    let error = validate_task_graph(&[]).expect_err("a refusal");
    assert_eq!(error.message(), "a run requires at least one task");
    let error = validate_task_graph(&[task("step-1"), task("step-1")]).expect_err("a refusal");
    assert_eq!(error.message(), "duplicate task identifier: step-1");
    let undeclared = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &["step-9".to_owned()],
        "",
        None,
    )
    .expect("a task");
    let error = validate_task_graph(&[undeclared]).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "task step-1 depends on undeclared tasks: step-9"
    );
    let oversized: Vec<AgentTask> = (0..MAX_AGENT_TASKS + 1)
        .map(|index| task(&format!("step-{index}")))
        .collect();
    let error = validate_task_graph(&oversized).expect_err("a refusal");
    assert_eq!(error.message(), "a run cannot exceed 64 tasks");
}

#[test]
fn the_dependency_order_and_the_layers_follow_the_dependencies() {
    let dependent = AgentTask::from_parts(
        "step-2",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &["step-1".to_owned()],
        "",
        None,
    )
    .expect("a task");
    let tasks = vec![task("step-1"), dependent];
    let ordered = dependency_order(&tasks).expect("an order");
    assert_eq!(ordered[0].step_id(), "step-1");
    assert_eq!(ordered[1].step_id(), "step-2");
    let layers = dependency_layers(&tasks).expect("layers");
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].len(), 1);
    assert_eq!(layers[1].len(), 1);
}

#[test]
fn the_dependency_helpers_detect_a_cycle() {
    let first = AgentTask::from_parts(
        "step-1",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &["step-2".to_owned()],
        "",
        None,
    )
    .expect("a task");
    let second = AgentTask::from_parts(
        "step-2",
        "tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &["step-1".to_owned()],
        "",
        None,
    )
    .expect("a task");
    let tasks = vec![first, second];
    let error = dependency_order(&tasks).expect_err("a refusal");
    assert_eq!(error.message(), "task dependencies contain a cycle");
    let error = dependency_layers(&tasks).expect_err("a refusal");
    assert_eq!(error.message(), "task dependencies contain a cycle");
}

// ---------------------------------------------------------------------------
// The planners.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_offline_planner_scores_the_catalog() {
    let planner = OfflinePlanner::new(Arc::new(catalog()), None).expect("a planner");
    let tasks = planner
        .plan("Compute the beam volume")
        .await
        .expect("a plan");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].step_id(), "step-1");
    assert_eq!(tasks[1].step_id(), "step-2");
    assert!(planner.degradations().is_empty());
}

#[tokio::test]
async fn the_offline_planner_prefers_the_fallback() {
    let planner = OfflinePlanner::new(Arc::new(catalog()), Some("summarize")).expect("a planner");
    let tasks = planner
        .plan("nothing in the catalog matches this")
        .await
        .expect("a plan");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tool_id(), "mechanical.reference-evaluator");
}

#[tokio::test]
async fn the_offline_planner_reports_a_total_miss() {
    let planner = OfflinePlanner::new(Arc::new(catalog()), None).expect("a planner");
    let error = planner
        .plan("nothing matches")
        .await
        .expect_err("a refusal");
    assert_eq!(error.message(), "no catalog step matches the objective");
    assert_eq!(error.python_class_name(), "PlannerFailure");
}

#[tokio::test]
async fn the_offline_planner_validates_the_objective() {
    let planner = OfflinePlanner::new(Arc::new(catalog()), None).expect("a planner");
    let error = planner.plan(&"x".repeat(600)).await.expect_err("a refusal");
    assert_eq!(error.message(), "objective exceeds the bounded length");
}

#[tokio::test]
async fn the_offline_planner_refuses_an_undeclared_fallback() {
    let error = OfflinePlanner::new(Arc::new(catalog()), Some("absent")).expect_err("a refusal");
    assert_eq!(error.message(), "catalog step absent is not declared");
}

/// A planner that always fails.
struct FailingPlanner {
    error: AgentError,
}

#[async_trait::async_trait]
impl Planner for FailingPlanner {
    async fn plan(&self, _objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        Err(self.error.clone())
    }
}

#[tokio::test]
async fn the_degrading_planner_never_degrades_an_authoritative_refusal() {
    let refusals = vec![
        AgentError::policy("no policy"),
        AgentError::Model(ModelError::egress_denied("egress refused")),
        AgentError::Model(ModelError::capability("capability missing")),
    ];
    for error in refusals {
        let planner = DegradingPlanner::new(
            Arc::new(FailingPlanner {
                error: error.clone(),
            }),
            Arc::new(
                OfflinePlanner::new(Arc::new(catalog()), Some("simulate")).expect("a planner"),
            ),
            true,
        );
        let failure = planner.plan("beam volume").await.expect_err("a refusal");
        assert_eq!(failure.python_class_name(), error.python_class_name());
        assert!(planner.degradations().is_empty());
    }
    assert_eq!(kala_runtime::agent::NEVER_DEGRADED_ERRORS.len(), 4);
}

#[tokio::test]
async fn the_degrading_planner_refuses_an_unapproved_degradation() {
    let planner = DegradingPlanner::new(
        Arc::new(FailingPlanner {
            error: AgentError::Model(ModelError::degraded(
                "provider-a",
                "model-a",
                "circuit open",
            )),
        }),
        Arc::new(OfflinePlanner::new(Arc::new(catalog()), Some("simulate")).expect("a planner")),
        false,
    );
    let error = planner.plan("beam volume").await.expect_err("a refusal");
    assert_eq!(error.python_class_name(), "ProviderDegradedError");
    assert!(
        error
            .message()
            .contains("provider degraded: provider-a/model-a")
    );
    assert!(
        error
            .message()
            .contains("deterministic planning was not approved")
    );
}

#[tokio::test]
async fn the_degrading_planner_falls_back_and_records_the_reason() {
    let planner = DegradingPlanner::new(
        Arc::new(FailingPlanner {
            error: AgentError::Model(ModelError::degraded(
                "provider-a",
                "model-a",
                "circuit open",
            )),
        }),
        Arc::new(OfflinePlanner::new(Arc::new(catalog()), Some("simulate")).expect("a planner")),
        true,
    );
    let tasks = planner
        .plan("nothing in the catalog matches")
        .await
        .expect("a plan");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        planner.degradations(),
        vec!["provider degraded: provider-a/model-a".to_owned()]
    );
}

#[tokio::test]
async fn the_degrading_planner_truncates_a_long_reason() {
    // The Python test builds a refusal whose *class name* is 2,000
    // characters long. Rust has no dynamic class name, so the long reason
    // comes from the provider identifier of a degraded provider, which the
    // reason quotes.
    let planner = DegradingPlanner::new(
        Arc::new(FailingPlanner {
            error: AgentError::Model(ModelError::degraded(
                "E".repeat(2_000),
                "model-a",
                "circuit open",
            )),
        }),
        Arc::new(OfflinePlanner::new(Arc::new(catalog()), Some("simulate")).expect("a planner")),
        true,
    );
    let _ = planner.plan("beam volume").await;
    let reasons = planner.degradations();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].chars().count(), MAX_ERROR_TEXT);
    assert!(reasons[0].starts_with("provider degraded: EEE"));
}

#[tokio::test]
async fn the_model_planner_publishes_its_response_schema() {
    let planner = ModelPlanner::new(
        Arc::new(kala_runtime::gateway::ModelGateway::new(Arc::new(
            kala_runtime::gateway::LocalModelClient::new(
                "local",
                "local",
                vec![kala_runtime::gateway::ModelCapability::StructuredOutput],
            )
            .expect("a client"),
        ))),
        Arc::new(catalog()),
        1_024,
    )
    .expect("a planner");
    let schema = planner.response_schema();
    assert_eq!(
        agent_support::get(&schema, "title")
            .and_then(|value| value.as_text())
            .map(|value| value.to_string_lossy()),
        Some("kala.plan".to_owned())
    );
    let properties = agent_support::get(&schema, "properties").expect("the properties");
    let steps = agent_support::get(properties, "steps").expect("the steps");
    let items = agent_support::get(steps, "items").expect("the items");
    let item_properties = agent_support::get(items, "properties").expect("the item properties");
    let catalog_step =
        agent_support::get(item_properties, "catalog_step_id").expect("the catalog step");
    let steps = agent_support::get(catalog_step, "enum").expect("the enum of the catalog");
    let CanonicalValue::Array(items) = steps else {
        panic!("the enum is an array");
    };
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn the_model_planner_refuses_a_missing_structured_answer() {
    let planner = ModelPlanner::new(
        Arc::new(kala_runtime::gateway::ModelGateway::new(Arc::new(
            kala_runtime::gateway::LocalModelClient::new(
                "local",
                "local",
                vec![kala_runtime::gateway::ModelCapability::StructuredOutput],
            )
            .expect("a client"),
        ))),
        Arc::new(catalog()),
        1_024,
    )
    .expect("a planner");
    let response = ModelResponse::new("plan-1", "local", "local", "{}").expect("a response");
    let error = planner.plan_steps(&response).expect_err("a refusal");
    assert_eq!(
        error.message(),
        "the planner response carried no structured output"
    );
    assert_eq!(error.python_class_name(), "StructuredOutputError");
}

// ---------------------------------------------------------------------------
// The outcome and the run result.
// ---------------------------------------------------------------------------

#[test]
fn the_outcome_validates_every_field() {
    let error = AgentStepOutcome::from_parts(
        "bad id",
        "call-1",
        ToolRunStatus::Succeeded,
        true,
        "tool",
        None,
        None,
        None,
        &[],
        0.0,
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "step_id must be a safe identifier");

    let error = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Succeeded,
        true,
        "tool",
        Some(&CanonicalValue::Array(Vec::new())),
        None,
        None,
        &[],
        0.0,
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "the outcome result must be a JSON object");

    let error = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Succeeded,
        true,
        "tool",
        None,
        None,
        Some(&"x".repeat(2_000)),
        &[],
        0.0,
        None,
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "error_message exceeds the bounded length");

    let error = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Denied,
        true,
        "tool",
        None,
        None,
        None,
        &[],
        0.0,
        Some(&"x".repeat(2_000)),
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "policy_reason exceeds the bounded length");
}

#[test]
fn the_outcome_reports_a_denial() {
    let outcome = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Denied,
        false,
        "mechanical.reference-evaluator",
        None,
        None,
        None,
        &[],
        0.0,
        None,
    )
    .expect("an outcome");
    assert!(outcome.denied());
    assert!(!outcome.succeeded());
}

#[test]
fn the_run_result_validates_its_digests() {
    let outcome = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Succeeded,
        true,
        "mechanical.reference-evaluator",
        Some(&object(vec![("operation", text("beam_volume"))])),
        None,
        None,
        &[],
        0.0,
        None,
    )
    .expect("an outcome");
    let result = AgentRunResult::from_parts(
        "run-1",
        vec![outcome.clone()],
        &object(vec![(
            "step-1",
            object(vec![("operation", text("beam_volume"))]),
        )]),
        &[("step-1".to_owned(), "a".repeat(64))],
        &[],
        &[],
    )
    .expect("a result");
    assert!(result.succeeded());
    assert_eq!(result.step_ids(), vec!["step-1".to_owned()]);
    assert!(result.artifacts().is_empty());
    assert!(result.outcome_for("step-1").is_ok());
    let error = result.outcome_for("step-9").expect_err("a refusal");
    assert_eq!(error.message(), "no outcome was recorded for step step-9");
    let error = result.outcome_for("bad id").expect_err("a refusal");
    assert_eq!(error.message(), "step_id must be a safe identifier");

    let error = AgentRunResult::from_parts(
        "run-1",
        vec![outcome.clone()],
        &CanonicalValue::Object(CanonicalObject::new()),
        &[("step-1".to_owned(), "not-a-digest".to_owned())],
        &[],
        &[],
    )
    .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "digests must be sha256 hexadecimal digests"
    );

    let error = AgentRunResult::from_parts(
        "bad run",
        vec![],
        &CanonicalValue::Object(CanonicalObject::new()),
        &[],
        &[],
        &[],
    )
    .expect_err("a refusal");
    assert_eq!(error.message(), "run_id must be a safe identifier");
}

#[test]
fn the_run_result_collects_the_distinct_artifacts() {
    let first = AgentStepOutcome::from_parts(
        "step-1",
        "call-1",
        ToolRunStatus::Succeeded,
        true,
        "tool",
        None,
        None,
        None,
        &["artifact-1".to_owned()],
        0.0,
        None,
    )
    .expect("an outcome");
    let second = AgentStepOutcome::from_parts(
        "step-2",
        "call-2",
        ToolRunStatus::Succeeded,
        true,
        "tool",
        None,
        None,
        None,
        &["artifact-1".to_owned(), "artifact-2".to_owned()],
        0.0,
        None,
    )
    .expect("an outcome");
    let result = AgentRunResult::from_parts(
        "run-1",
        vec![first, second],
        &CanonicalValue::Object(CanonicalObject::new()),
        &[],
        &["provider degraded".to_owned()],
        &["step-2".to_owned(), "step-2".to_owned()],
    )
    .expect("a result");
    assert_eq!(
        result.artifacts(),
        vec!["artifact-1".to_owned(), "artifact-2".to_owned()]
    );
    assert_eq!(result.budget_denied(), ["step-2".to_owned()]);
    assert!(!result.succeeded());
}

// ---------------------------------------------------------------------------
// The coordinator.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_coordinator_requires_a_planner() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let error = coordinator
        .plan("beam volume")
        .await
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "no planner is configured for this coordinator"
    );
}

#[tokio::test]
async fn the_coordinator_validates_the_objective() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path())).with_planner(Arc::new(
        OfflinePlanner::new(Arc::new(catalog()), None).expect("a planner"),
    ));
    let error = coordinator.plan("   ").await.expect_err("a refusal");
    assert_eq!(
        error.message(),
        "the planning objective must be a nonempty string"
    );
}

#[tokio::test]
async fn the_coordinator_bounds_its_parallelism() {
    let workdir = TempDir::new("kala-agent-semantics");
    let error = AgentCoordinator::new(executor(workdir.path()))
        .with_max_parallel_tasks(0)
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "max_parallel_tasks must be between 1 and 8"
    );
    let error = AgentCoordinator::new(executor(workdir.path()))
        .with_max_parallel_tasks(MAX_PARALLEL_TASKS + 1)
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "max_parallel_tasks must be between 1 and 8"
    );
    let coordinator = AgentCoordinator::new(executor(workdir.path()))
        .with_max_parallel_tasks(2)
        .expect("a bound");
    assert_eq!(coordinator.max_parallel_tasks(), 2);
}

#[tokio::test]
async fn the_coordinator_runs_a_trusted_evaluator() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path())).with_clock(Arc::new(|| {
        GatewayTimestamp::Aware(
            kala_kernel::time::KernelTimestamp::from_civil(2026, 1, 2, 3, 4, 5, 0, 0)
                .expect("the fixed instant"),
        )
    }));
    let result = coordinator.run(&[task("step-1")], "run-1").expect("a run");
    assert!(result.succeeded());
    assert_eq!(result.step_ids(), vec!["step-1".to_owned()]);
    assert_eq!(result.digests().len(), 1);
    assert!(result.degraded_reasons().is_empty());
    assert!(result.budget_denied().is_empty());
}

#[tokio::test]
async fn the_coordinator_blocks_a_failed_dependency() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let failing = AgentTask::from_parts(
        "step-1",
        "mechanical.reference-evaluator",
        &object(vec![("operation", text("shear"))]),
        Some("1.0.0"),
        &[],
        &[],
        "",
        None,
    )
    .expect("a task");
    let dependent = AgentTask::from_parts(
        "step-2",
        "mechanical.reference-evaluator",
        &arguments(),
        Some("1.0.0"),
        &[],
        &["step-1".to_owned()],
        "",
        None,
    )
    .expect("a task");
    let result = coordinator
        .run(&[failing, dependent], "run-1")
        .expect("a run");
    assert!(!result.succeeded());
    let blocked = result.outcome_for("step-2").expect("an outcome");
    assert_eq!(blocked.status(), ToolRunStatus::Cancelled);
    assert!(!blocked.dispatched());
    assert!(
        blocked
            .error_message()
            .expect("a reason")
            .contains("dependency did not succeed")
    );
}

#[tokio::test]
async fn the_coordinator_denies_an_unknown_tool() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let unknown = AgentTask::from_parts(
        "step-1",
        "absent.tool",
        &CanonicalValue::Object(CanonicalObject::new()),
        None,
        &[],
        &[],
        "",
        None,
    )
    .expect("a task");
    let result = coordinator.run(&[unknown], "run-1").expect("a run");
    let outcome = result.outcome_for("step-1").expect("an outcome");
    assert_eq!(outcome.status(), ToolRunStatus::Denied);
    assert_eq!(outcome.error_code(), Some("unknown_tool"));
    assert!(
        outcome
            .error_message()
            .expect("a reason")
            .contains("is not registered")
    );
}

#[tokio::test]
async fn the_coordinator_denies_an_undeclared_capability() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path()));
    let greedy = AgentTask::from_parts(
        "step-1",
        "mechanical.reference-evaluator",
        &arguments(),
        Some("1.0.0"),
        &["artifact.write".to_owned()],
        &[],
        "",
        None,
    )
    .expect("a task");
    let result = coordinator.run(&[greedy], "run-1").expect("a run");
    let outcome = result.outcome_for("step-1").expect("an outcome");
    assert_eq!(outcome.status(), ToolRunStatus::Denied);
    assert_eq!(outcome.error_code(), Some("capability_denied"));
    assert!(
        outcome
            .error_message()
            .expect("a reason")
            .contains("undeclared capabilities")
    );
}

#[tokio::test]
async fn the_coordinator_requires_an_aware_clock() {
    let workdir = TempDir::new("kala-agent-semantics");
    let coordinator = AgentCoordinator::new(executor(workdir.path())).with_clock(Arc::new(|| {
        GatewayTimestamp::Naive(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
                .expect("a date")
                .and_hms_opt(0, 0, 0)
                .expect("a time"),
        )
    }));
    let error = coordinator
        .run(&[task("step-1")], "run-1")
        .expect_err("a refusal");
    assert_eq!(
        error.message(),
        "the coordinator clock must return an aware datetime"
    );
    assert_eq!(error.python_class_name(), "AgentError");
}
