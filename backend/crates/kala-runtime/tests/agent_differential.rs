//! The live differential against the repository's Python agent runtime.
//!
//! The Python agent and the Rust agent are driven with the *same* scenario:
//! the same step catalog, the same tasks, the same scripted model provider
//! and the same tool registry (the shipped simulator tools). The two result
//! documents are compared byte for byte over canonical JSON.
//!
//! What is compared:
//!
//! * the **decisions** — the outcome of every case, the Python exception
//!   class name and the exact message of every failure;
//! * the **events** — the run result payload: every outcome, the merged
//!   results, the digests, the degradation reasons and the budget refusals;
//! * the **tool calls made** — the execution reports of the executor;
//! * the **budget ledger** — the whole exported ledger state;
//! * the **final results** — the payload of the run.
//!
//! The only field that is normalized away is the measured duration of a tool
//! call: it is a measurement, not a decision.

mod agent_support;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kala_budget::{
    BudgetDimension, BudgetError, BudgetLedger, BudgetValue, ExhaustedBudget, RunBudget,
};
use kala_contracts::{ContractModel, PlanStep};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_kernel::policy::PolicyEngine;
use kala_runtime::agent::{
    AgentCoordinator, AgentError, AgentTask, CatalogStep, DegradingPlanner, ModelPlanner,
    OfflinePlanner, Planner, StepCatalog,
};
use kala_runtime::gateway::{
    ClientError, GatewayTimestamp, ModelCapability, ModelError, ModelGateway, ModelRequest,
    ModelResponse,
};
use kala_tools::executor::{ExecutorConfig, IsolationMode, ToolExecutor};
use kala_tools::protocol::{MAX_JSON_BYTES, canonical_json_dumps};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION,
    register_reference_evaluator, register_simulator,
};

use agent_support::{
    TempDir, bool_field, float_field, get, int_field, normalize_timings, object,
    optional_text_field, run_oracle, string_list, text, text_field,
};

/// The capabilities the simulator needs.
const SIMULATOR_CAPABILITIES: [&str; 4] = [
    "artifact.read",
    "artifact.write",
    "design.read",
    "tool.execute",
];

/// The instant the coordinator clock returns.
const ISSUED_AT: (i32, u32, u32, u32, u32, u32) = (2026, 1, 2, 3, 4, 5);

// ---------------------------------------------------------------------------
// The scripted provider double.
// ---------------------------------------------------------------------------

/// A deterministic provider double.
struct ScriptedClient {
    responses: Mutex<Vec<Option<CanonicalValue>>>,
    failure: Option<String>,
    calls: AtomicU64,
}

impl ScriptedClient {
    fn new(responses: Vec<Option<CanonicalValue>>, failure: Option<String>) -> Self {
        Self {
            responses: Mutex::new(responses),
            failure,
            calls: AtomicU64::new(0),
        }
    }
}

#[async_trait::async_trait]
impl kala_runtime::gateway::LlmClient for ScriptedClient {
    fn provider_id(&self) -> &str {
        "local"
    }

    fn model_id(&self) -> &str {
        "local"
    }

    fn requires_egress(&self) -> bool {
        false
    }

    fn supports(&self, _capability: ModelCapability) -> bool {
        true
    }

    async fn complete(&self, request: &ModelRequest) -> Result<ModelResponse, ClientError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failure.as_deref() == Some("degraded") {
            return Err(ClientError::Model(ModelError::degraded(
                "provider-a",
                "model-a",
                "circuit open",
            )));
        }
        let payload = self
            .responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .flatten();
        let content = match &payload {
            None => "{}".to_owned(),
            Some(value) => canonical_json_dumps(value, MAX_JSON_BYTES)
                .map_err(|error| ClientError::Model(ModelError::request(error.to_string())))?,
        };
        let mut response = ModelResponse::new(request.request_id(), "local", "local", &content)
            .map_err(ClientError::Model)?;
        if let Some(value) = &payload {
            response = response
                .with_structured(Some(value))
                .map_err(ClientError::Model)?;
        }
        response.with_tokens(10, 5).map_err(ClientError::Model)
    }
}

/// A planner that always fails with the scripted error.
struct FailingPlanner {
    failure: String,
}

#[async_trait::async_trait]
impl Planner for FailingPlanner {
    async fn plan(&self, _objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        match self.failure.as_str() {
            "degraded" => Err(AgentError::Model(ModelError::degraded(
                "provider-a",
                "model-a",
                "circuit open",
            ))),
            "planner_failure" => Err(AgentError::planner_failure("nothing matched")),
            "structured" => Err(AgentError::Model(ModelError::structured(
                "no structured output",
            ))),
            "plan_error" => Err(AgentError::plan("bad plan")),
            "policy" => Err(AgentError::policy("no policy")),
            "egress" => Err(AgentError::Model(ModelError::egress_denied(
                "egress refused",
            ))),
            "capability" => Err(AgentError::Model(ModelError::capability(
                "capability missing",
            ))),
            "budget" => Err(AgentError::Budget(BudgetError::Exhausted(Box::new(
                ExhaustedBudget {
                    dimension: BudgetDimension::ToolCalls,
                    requested: BudgetValue::Integer(1u64.into()),
                    remaining: BudgetValue::Integer(0u64.into()),
                    limit: BudgetValue::Integer(0u64.into()),
                },
            )))),
            "base" => Err(AgentError::base("the base agent error")),
            other => panic!("unknown failure: {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// The scenario.
// ---------------------------------------------------------------------------

/// The step catalog of the differential.
fn catalog() -> Vec<CanonicalValue> {
    vec![
        object(vec![
            ("catalog_step_id", text("simulate")),
            ("tool_id", text(EVALUATOR_TOOL_ID)),
            ("tool_version", text(EVALUATOR_TOOL_VERSION)),
            (
                "default_arguments",
                object(vec![("operation", text("beam_volume"))]),
            ),
            (
                "keywords",
                CanonicalValue::Array(vec![text("beam"), text("volume")]),
            ),
            ("intent", text("Simulate the beam.")),
        ]),
        object(vec![
            ("catalog_step_id", text("summarize")),
            ("tool_id", text(EVALUATOR_TOOL_ID)),
            ("tool_version", text(EVALUATOR_TOOL_VERSION)),
            (
                "default_arguments",
                object(vec![("operation", text("bar_summary"))]),
            ),
            ("keywords", CanonicalValue::Array(vec![text("summary")])),
            ("intent", text("Summarize the beam.")),
        ]),
    ]
}

/// One task of the differential.
fn task(step_id: &str, operation: &str) -> CanonicalValue {
    object(vec![
        ("step_id", text(step_id)),
        ("tool_id", text(EVALUATOR_TOOL_ID)),
        ("tool_version", text(EVALUATOR_TOOL_VERSION)),
        ("arguments", object(vec![("operation", text(operation))])),
    ])
}

/// The whole scenario of the differential.
fn scenario() -> CanonicalValue {
    let mut cases: Vec<CanonicalValue> = Vec::new();

    // 1. The pure evaluator runs and the result is merged.
    cases.push(object(vec![
        ("name", text("evaluator-success")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 2. Two independent tasks run in one layer.
    cases.push(object(vec![
        ("name", text("parallel-layer")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![
                task("step-1", "beam_volume"),
                task("step-2", "bar_summary"),
            ]),
        ),
        ("run_id", text("run-1")),
        ("max_parallel_tasks", CanonicalValue::integer(2)),
    ]));

    // 3. A task whose dependency did not succeed is cancelled.
    cases.push(object(vec![
        ("name", text("blocked-dependency")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![
                object(vec![
                    ("step_id", text("step-1")),
                    ("tool_id", text(EVALUATOR_TOOL_ID)),
                    ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                    ("arguments", object(vec![("operation", text("shear"))])),
                ]),
                object(vec![
                    ("step_id", text("step-2")),
                    ("tool_id", text(EVALUATOR_TOOL_ID)),
                    ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                    (
                        "arguments",
                        object(vec![("operation", text("beam_volume"))]),
                    ),
                    ("depends_on", CanonicalValue::Array(vec![text("step-1")])),
                ]),
            ]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 4. An unknown tool is denied.
    cases.push(object(vec![
        ("name", text("unknown-tool")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text("absent.tool")),
            ])]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 5. A task that requests an undeclared capability is denied.
    cases.push(object(vec![
        ("name", text("undeclared-capability")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text(EVALUATOR_TOOL_ID)),
                ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                (
                    "arguments",
                    object(vec![("operation", text("beam_volume"))]),
                ),
                (
                    "required_capabilities",
                    CanonicalValue::Array(vec![text("artifact.write")]),
                ),
            ])]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 6. The budget refuses the layer.
    cases.push(object(vec![
        ("name", text("budget-exhausted")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        (
            "ledger",
            object(vec![
                ("max_model_calls", CanonicalValue::integer(1)),
                ("max_tool_calls", CanonicalValue::integer(0)),
                ("max_total_tokens", CanonicalValue::integer(10)),
                ("max_artifact_bytes", CanonicalValue::integer(10)),
                ("max_duration_seconds", CanonicalValue::Float(5.0)),
                ("max_replanning_attempts", CanonicalValue::integer(0)),
            ]),
        ),
    ]));

    // 7. The budget reserves and settles the call of the executor.
    cases.push(object(vec![
        ("name", text("budget-reserved-and-settled")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("ledger_on_executor", CanonicalValue::Bool(true)),
        (
            "ledger",
            object(vec![
                ("max_model_calls", CanonicalValue::integer(4)),
                ("max_tool_calls", CanonicalValue::integer(4)),
                ("max_total_tokens", CanonicalValue::integer(100)),
                ("max_artifact_bytes", CanonicalValue::integer(100)),
                ("max_duration_seconds", CanonicalValue::Float(30.0)),
                ("max_replanning_attempts", CanonicalValue::integer(1)),
            ]),
        ),
    ]));

    // 8. A side-effecting tool without a policy engine is a hard failure.
    cases.push(object(vec![
        ("name", text("side-effect-without-policy")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text(SIMULATOR_TOOL_ID)),
                ("tool_version", text(SIMULATOR_TOOL_VERSION)),
                (
                    "arguments",
                    object(vec![("operation", text("beam_volume"))]),
                ),
                (
                    "required_capabilities",
                    CanonicalValue::Array(vec![text("artifact.write")]),
                ),
            ])]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 9. A side-effecting tool with a policy engine is denied, not dispatched.
    cases.push(object(vec![
        ("name", text("side-effect-policy-denied")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text(SIMULATOR_TOOL_ID)),
                ("tool_version", text(SIMULATOR_TOOL_VERSION)),
                (
                    "arguments",
                    object(vec![("operation", text("beam_volume"))]),
                ),
                (
                    "required_capabilities",
                    CanonicalValue::Array(vec![text("artifact.write")]),
                ),
            ])]),
        ),
        ("run_id", text("run-1")),
        ("policy", CanonicalValue::Bool(true)),
    ]));

    // 10. The offline planner scores the catalog.
    cases.push(object(vec![
        ("name", text("offline-planner")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("offline")),
        ("objective", text("Compute the beam volume")),
    ]));

    // 11. The offline planner falls back when nothing matches.
    cases.push(object(vec![
        ("name", text("offline-planner-fallback")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("offline")),
        ("fallback_step_id", text("summarize")),
        ("objective", text("nothing in the catalog matches this")),
    ]));

    // 12. The offline planner reports a total miss.
    cases.push(object(vec![
        ("name", text("offline-planner-miss")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("offline")),
        ("objective", text("nothing matches")),
    ]));

    // 13. The model planner converts a valid answer into tasks.
    cases.push(object(vec![
        ("name", text("model-planner-valid")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(vec![object(vec![
                        ("catalog_step_id", text("simulate")),
                        ("intent", text("Simulate the beam.")),
                        (
                            "parameters",
                            object(vec![("operation", text("bar_summary"))]),
                        ),
                    ])]),
                )])]),
            )]),
        ),
    ]));

    // 14. The model planner refuses a plan that names an unknown catalog step.
    cases.push(object(vec![
        ("name", text("model-planner-unknown-step")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(vec![object(vec![
                        ("catalog_step_id", text("made_up")),
                        ("intent", text("Do something else.")),
                        ("parameters", object(vec![])),
                    ])]),
                )])]),
            )]),
        ),
    ]));

    // 15. The model planner refuses a malformed answer.
    cases.push(object(vec![
        ("name", text("model-planner-malformed")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(vec![object(vec![
                        ("catalog_step_id", text("simulate")),
                        ("intent", CanonicalValue::integer(7)),
                        ("parameters", object(vec![])),
                    ])]),
                )])]),
            )]),
        ),
    ]));

    // 16. The model planner refuses a plan with too many steps.
    cases.push(object(vec![
        ("name", text("model-planner-too-many-steps")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(
                        (0..3)
                            .map(|_index| {
                                object(vec![
                                    ("catalog_step_id", text("simulate")),
                                    ("intent", text("Simulate the beam.")),
                                    ("parameters", object(vec![])),
                                ])
                            })
                            .collect(),
                    ),
                )])]),
            )]),
        ),
    ]));

    // 17. The degrading planner refuses an unapproved degradation.
    cases.push(object(vec![
        ("name", text("degrading-refused")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("degrading")),
        ("primary", text("degraded")),
        ("allow_degraded", CanonicalValue::Bool(false)),
        ("fallback_step_id", text("simulate")),
        ("objective", text("beam volume")),
    ]));

    // 18. The degrading planner falls back on a provider failure.
    cases.push(object(vec![
        ("name", text("degrading-provider")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("degrading")),
        ("primary", text("degraded")),
        ("allow_degraded", CanonicalValue::Bool(true)),
        ("fallback_step_id", text("simulate")),
        ("objective", text("beam volume")),
    ]));

    // 19. The degrading planner falls back on a planner failure.
    cases.push(object(vec![
        ("name", text("degrading-planner-failure")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("degrading")),
        ("primary", text("planner_failure")),
        ("allow_degraded", CanonicalValue::Bool(true)),
        ("fallback_step_id", text("summarize")),
        ("objective", text("beam volume")),
    ]));

    // 20. The degrading planner never degrades an authoritative refusal.
    for refusal in ["policy", "egress", "capability", "budget"] {
        cases.push(object(vec![
            ("name", text(format!("degrading-never-{refusal}").as_str())),
            ("catalog", CanonicalValue::Array(catalog())),
            (
                "tasks",
                CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
            ),
            ("run_id", text("run-1")),
            ("planner", text("degrading")),
            ("primary", text(refusal)),
            ("allow_degraded", CanonicalValue::Bool(true)),
            ("fallback_step_id", text("simulate")),
            ("objective", text("beam volume")),
        ]));
    }

    // 21. The coordinator plans a declared plan.
    cases.push(object(vec![
        ("name", text("run-plan")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        (
            "plan_steps",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("catalog_step_id", text("simulate")),
                ("intent", text("Simulate the beam.")),
                ("parameters_json", text("{\"operation\":\"bar_summary\"}")),
            ])]),
        ),
    ]));

    // 22. The coordinator refuses a plan that names an undeclared parameter.
    cases.push(object(vec![
        ("name", text("run-plan-undeclared-parameter")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        (
            "plan_steps",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("catalog_step_id", text("simulate")),
                ("intent", text("Simulate the beam.")),
                ("parameters_json", text("{\"made_up\":1}")),
            ])]),
        ),
    ]));

    // 23. The coordinator refuses a plan with an undeclared dependency.
    cases.push(object(vec![
        ("name", text("run-plan-undeclared-dependency")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        (
            "plan_steps",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("catalog_step_id", text("simulate")),
                ("intent", text("Simulate the beam.")),
                ("depends_on", CanonicalValue::Array(vec![text("step-9")])),
            ])]),
        ),
    ]));

    // 24. The coordinator refuses an unsafe run identifier.
    cases.push(object(vec![
        ("name", text("unsafe-run-id")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("bad run")),
    ]));

    // 25. The coordinator refuses a duplicate task identifier.
    cases.push(object(vec![
        ("name", text("duplicate-task")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![
                task("step-1", "beam_volume"),
                task("step-1", "bar_summary"),
            ]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 26. The coordinator refuses an empty objective.
    cases.push(object(vec![
        ("name", text("empty-objective")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("offline")),
        ("objective", text("   ")),
    ]));

    // 27. The model planner replaces an empty intent.
    cases.push(object(vec![
        ("name", text("model-planner-empty-intent")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(vec![object(vec![
                        ("catalog_step_id", text("simulate")),
                        ("intent", text("")),
                        ("parameters", object(vec![])),
                    ])]),
                )])]),
            )]),
        ),
    ]));

    // 28. The model planner converts a two-step plan.
    cases.push(object(vec![
        ("name", text("model-planner-two-steps")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("model")),
        ("objective", text("beam volume")),
        (
            "model",
            object(vec![(
                "responses",
                CanonicalValue::Array(vec![object(vec![(
                    "steps",
                    CanonicalValue::Array(vec![
                        object(vec![
                            ("catalog_step_id", text("simulate")),
                            ("intent", text("Simulate the beam.")),
                            ("parameters", object(vec![])),
                        ]),
                        object(vec![
                            ("catalog_step_id", text("summarize")),
                            ("intent", text("Summarize the beam.")),
                            ("parameters", object(vec![])),
                        ]),
                    ]),
                )])]),
            )]),
        ),
    ]));

    // 29. A task with an invalid timeout is refused.
    cases.push(object(vec![
        ("name", text("task-invalid-timeout")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text(EVALUATOR_TOOL_ID)),
                ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                (
                    "arguments",
                    object(vec![("operation", text("beam_volume"))]),
                ),
                ("timeout_seconds", CanonicalValue::Float(0.0)),
            ])]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 30. A task with a valid timeout runs.
    cases.push(object(vec![
        ("name", text("task-valid-timeout")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![object(vec![
                ("step_id", text("step-1")),
                ("tool_id", text(EVALUATOR_TOOL_ID)),
                ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                (
                    "arguments",
                    object(vec![("operation", text("beam_volume"))]),
                ),
                ("timeout_seconds", CanonicalValue::Float(5.0)),
            ])]),
        ),
        ("run_id", text("run-1")),
    ]));

    // 31. A dotted keyword scores the same tokens.
    let dotted = CanonicalValue::Array(vec![object(vec![
        ("catalog_step_id", text("simulate")),
        ("tool_id", text(EVALUATOR_TOOL_ID)),
        ("tool_version", text(EVALUATOR_TOOL_VERSION)),
        (
            "default_arguments",
            object(vec![("operation", text("beam_volume"))]),
        ),
        ("keywords", CanonicalValue::Array(vec![text("beam.volume")])),
        ("intent", text("Simulate the beam.")),
    ])]);
    cases.push(object(vec![
        ("name", text("offline-planner-dotted-keyword")),
        ("catalog", dotted),
        (
            "tasks",
            CanonicalValue::Array(vec![task("step-1", "beam_volume")]),
        ),
        ("run_id", text("run-1")),
        ("planner", text("offline")),
        ("objective", text("Compute the beam volume")),
    ]));

    // 32. The budget refuses a whole layer.
    cases.push(object(vec![
        ("name", text("budget-denied-layer")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![
                task("step-1", "beam_volume"),
                task("step-2", "bar_summary"),
            ]),
        ),
        ("run_id", text("run-1")),
        (
            "ledger",
            object(vec![
                ("max_model_calls", CanonicalValue::integer(1)),
                ("max_tool_calls", CanonicalValue::integer(1)),
                ("max_total_tokens", CanonicalValue::integer(10)),
                ("max_artifact_bytes", CanonicalValue::integer(10)),
                ("max_duration_seconds", CanonicalValue::Float(5.0)),
                ("max_replanning_attempts", CanonicalValue::integer(0)),
            ]),
        ),
    ]));

    // 33. A task that depends on a task that succeeded runs after it.
    cases.push(object(vec![
        ("name", text("chained-dependency")),
        ("catalog", CanonicalValue::Array(catalog())),
        (
            "tasks",
            CanonicalValue::Array(vec![
                task("step-1", "beam_volume"),
                object(vec![
                    ("step_id", text("step-2")),
                    ("tool_id", text(EVALUATOR_TOOL_ID)),
                    ("tool_version", text(EVALUATOR_TOOL_VERSION)),
                    (
                        "arguments",
                        object(vec![("operation", text("bar_summary"))]),
                    ),
                    ("depends_on", CanonicalValue::Array(vec![text("step-1")])),
                ]),
            ]),
        ),
        ("run_id", text("run-1")),
    ]));

    object(vec![("cases", CanonicalValue::Array(cases))])
}

// ---------------------------------------------------------------------------
// The Rust side of the differential.
// ---------------------------------------------------------------------------

/// The way the case is run.
enum RunMode {
    Tasks(Vec<AgentTask>),
    Plan(Vec<PlanStep>),
    Objective(String),
}

/// Everything a case needs.
struct Built {
    executor: Arc<ToolExecutor>,
    ledger: Option<Arc<BudgetLedger>>,
    mode: RunMode,
    coordinator: AgentCoordinator,
}

fn build_catalog(case: &CanonicalValue) -> Result<Arc<StepCatalog>, AgentError> {
    let mut steps = Vec::new();
    if let Some(CanonicalValue::Array(entries)) = get(case, "catalog") {
        for entry in entries {
            let default_arguments = get(entry, "default_arguments")
                .cloned()
                .unwrap_or_else(|| CanonicalValue::Object(CanonicalObject::new()));
            steps.push(CatalogStep::from_parts(
                &text_field(entry, "catalog_step_id"),
                &text_field(entry, "tool_id"),
                &default_arguments,
                optional_text_field(entry, "tool_version").as_deref(),
                &string_list(entry, "required_capabilities"),
                &string_list(entry, "keywords"),
                &text_field(entry, "intent"),
            )?);
        }
    }
    Ok(Arc::new(StepCatalog::new(steps)?))
}

fn build_tasks(case: &CanonicalValue) -> Result<Vec<AgentTask>, AgentError> {
    let mut tasks = Vec::new();
    if let Some(CanonicalValue::Array(entries)) = get(case, "tasks") {
        for entry in entries {
            let arguments = get(entry, "arguments")
                .cloned()
                .unwrap_or_else(|| CanonicalValue::Object(CanonicalObject::new()));
            tasks.push(AgentTask::from_parts(
                &text_field(entry, "step_id"),
                &text_field(entry, "tool_id"),
                &arguments,
                optional_text_field(entry, "tool_version").as_deref(),
                &string_list(entry, "required_capabilities"),
                &string_list(entry, "depends_on"),
                &text_field(entry, "intent"),
                float_field(entry, "timeout_seconds"),
            )?);
        }
    }
    Ok(tasks)
}

fn build_plan(case: &CanonicalValue) -> Result<Vec<PlanStep>, AgentError> {
    let mut steps = Vec::new();
    if let Some(CanonicalValue::Array(entries)) = get(case, "plan_steps") {
        for entry in entries {
            let mut map = CanonicalObject::new();
            map.insert(
                JsonText::new("step_id"),
                CanonicalValue::text(text_field(entry, "step_id")),
            );
            map.insert(
                JsonText::new("catalog_step_id"),
                CanonicalValue::text(text_field(entry, "catalog_step_id")),
            );
            map.insert(
                JsonText::new("intent"),
                CanonicalValue::text(text_field(entry, "intent")),
            );
            map.insert(
                JsonText::new("parameters_json"),
                CanonicalValue::text(
                    optional_text_field(entry, "parameters_json")
                        .unwrap_or_else(|| "{}".to_owned()),
                ),
            );
            if !string_list(entry, "depends_on").is_empty() {
                map.insert(
                    JsonText::new("depends_on"),
                    CanonicalValue::Array(
                        string_list(entry, "depends_on")
                            .into_iter()
                            .map(CanonicalValue::text)
                            .collect(),
                    ),
                );
            }
            steps.push(
                PlanStep::from_value(&CanonicalValue::Object(map))
                    .map_err(|error| AgentError::Validation(error.message().to_owned()))?,
            );
        }
    }
    Ok(steps)
}

fn build_ledger(case: &CanonicalValue) -> Option<Arc<BudgetLedger>> {
    let limits = get(case, "ledger")?;
    let mut budget = RunBudget::new()
        .with_max_model_calls(int_field(limits, "max_model_calls").unwrap_or(32) as u64)
        .with_max_tool_calls(int_field(limits, "max_tool_calls").unwrap_or(64) as u64)
        .with_max_total_tokens(int_field(limits, "max_total_tokens").unwrap_or(100_000) as u64)
        .with_max_artifact_bytes(
            int_field(limits, "max_artifact_bytes").unwrap_or(1_000_000_000) as u64,
        )
        .with_max_replanning_attempts(
            int_field(limits, "max_replanning_attempts").unwrap_or(3) as u64
        );
    if let Some(duration) = float_field(limits, "max_duration_seconds") {
        budget = budget
            .try_with_max_duration_seconds(duration)
            .expect("a bounded duration");
    }
    Some(Arc::new(BudgetLedger::new(budget)))
}

fn scripted_client(case: &CanonicalValue) -> Arc<ScriptedClient> {
    let model = get(case, "model");
    let mut responses = Vec::new();
    if let Some(CanonicalValue::Array(items)) = model.and_then(|value| get(value, "responses")) {
        for item in items {
            responses.push(Some(item.clone()));
        }
    }
    let failure = model.and_then(|value| optional_text_field(value, "failure"));
    Arc::new(ScriptedClient::new(responses, failure))
}

fn build_executor(
    case: &CanonicalValue,
    workdir: &std::path::Path,
    ledger: Option<Arc<BudgetLedger>>,
) -> Result<Arc<ToolExecutor>, AgentError> {
    let registry = Arc::new(ToolRegistry::new());
    register_reference_evaluator(&registry, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
        .map_err(|error| AgentError::Validation(error.to_string()))?;
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .map_err(|error| AgentError::Validation(error.to_string()))?;
    let mut config = ExecutorConfig::new(registry, workdir.join("run"));
    config.granted_capabilities = SIMULATOR_CAPABILITIES
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    config.isolation = IsolationMode::TrustedInProcessEvaluator;
    if bool_field(case, "ledger_on_executor") {
        config.ledger = ledger.clone();
    }
    Ok(Arc::new(
        ToolExecutor::new(config).map_err(|error| AgentError::Executor(error.clone()))?,
    ))
}

fn build_case(
    case: &CanonicalValue,
    executor: Arc<ToolExecutor>,
    ledger: Option<Arc<BudgetLedger>>,
    catalog: Arc<StepCatalog>,
) -> Result<Built, AgentError> {
    let fallback_step_id = optional_text_field(case, "fallback_step_id");
    let allow_degraded = bool_field(case, "allow_degraded");
    let max_tokens = int_field(case, "max_tokens").unwrap_or(1_024) as i128;
    let objective = optional_text_field(case, "objective");

    let mut gateway: Option<Arc<ModelGateway>> = None;
    let planner: Option<Arc<dyn Planner>> = match optional_text_field(case, "planner").as_deref() {
        Some("offline") => Some(Arc::new(OfflinePlanner::new(
            catalog.clone(),
            fallback_step_id.as_deref(),
        )?)),
        Some("model") => {
            let client = scripted_client(case);
            let model_gateway = Arc::new(ModelGateway::new(client));
            gateway = Some(model_gateway.clone());
            Some(Arc::new(ModelPlanner::new(
                model_gateway,
                catalog.clone(),
                max_tokens,
            )?))
        }
        Some("degrading") => {
            let client = scripted_client(case);
            let model_gateway = Arc::new(ModelGateway::new(client));
            gateway = Some(model_gateway.clone());
            let fallback: Arc<dyn Planner> = Arc::new(OfflinePlanner::new(
                catalog.clone(),
                fallback_step_id.as_deref(),
            )?);
            let primary: Arc<dyn Planner> = match optional_text_field(case, "primary").as_deref() {
                Some("model") => Arc::new(ModelPlanner::new(
                    model_gateway,
                    catalog.clone(),
                    max_tokens,
                )?),
                other => Arc::new(FailingPlanner {
                    failure: other.unwrap_or("degraded").to_owned(),
                }),
            };
            Some(Arc::new(DegradingPlanner::new(
                primary,
                fallback,
                allow_degraded,
            )))
        }
        Some("failing") => Some(Arc::new(FailingPlanner {
            failure: optional_text_field(case, "primary").unwrap_or_else(|| "degraded".to_owned()),
        })),
        _ => None,
    };

    let mut coordinator = AgentCoordinator::new(executor.clone())
        .with_catalog(catalog.clone())
        .with_clock(Arc::new(|| {
            GatewayTimestamp::Aware(
                kala_kernel::time::KernelTimestamp::from_civil(
                    ISSUED_AT.0,
                    ISSUED_AT.1,
                    ISSUED_AT.2,
                    ISSUED_AT.3,
                    ISSUED_AT.4,
                    ISSUED_AT.5,
                    0,
                    0,
                )
                .expect("the fixed instant"),
            )
        }));
    if let Some(planner) = planner {
        coordinator = coordinator.with_planner(planner);
    }
    if let Some(ledger) = ledger.clone() {
        coordinator = coordinator.with_ledger(ledger);
    }
    if let Some(gateway) = gateway {
        coordinator = coordinator.with_gateway(gateway);
    }
    if bool_field(case, "policy") {
        coordinator = coordinator.with_policy(Arc::new(PolicyEngine::new()));
    }
    let max_parallel = int_field(case, "max_parallel_tasks").unwrap_or(4) as usize;
    coordinator = coordinator.with_max_parallel_tasks(max_parallel)?;

    let mode = if get(case, "plan_steps").is_some() {
        RunMode::Plan(build_plan(case)?)
    } else if let Some(objective) = &objective {
        RunMode::Objective(objective.clone())
    } else {
        RunMode::Tasks(build_tasks(case)?)
    };

    Ok(Built {
        executor,
        ledger,
        mode,
        coordinator,
    })
}

/// The canonical state of the budget ledger.
fn ledger_state(ledger: &BudgetLedger) -> CanonicalValue {
    let state = ledger.export_state();
    let text = serde_json::to_string(&state).expect("the ledger state");
    kala_json::strict_json_loads(text.as_bytes())
        .unwrap_or_else(|error| panic!("parse the ledger state: {error}"))
}

/// The canonical reports of the executor.
fn calls_payload(executor: &ToolExecutor) -> CanonicalValue {
    CanonicalValue::Array(
        executor
            .reports()
            .iter()
            .map(|report| {
                let payload = report.to_payload();
                let field = |name: &str| {
                    agent_support::get(&payload, name)
                        .cloned()
                        .unwrap_or(CanonicalValue::Null)
                };
                object(vec![
                    ("artifact_bytes", field("artifact_bytes")),
                    ("call_id", field("call_id")),
                    ("error_code", field("error_code")),
                    ("granted_capabilities", field("granted_capabilities")),
                    ("status", field("status")),
                    ("tool_id", field("tool_id")),
                    ("tool_version", field("tool_version")),
                ])
            })
            .collect(),
    )
}

/// Run one case on the Rust side.
async fn run_case(case: &CanonicalValue) -> CanonicalValue {
    let workdir = TempDir::new("kala-agent-rust");
    let mut outcome = CanonicalObject::new();
    outcome.insert(
        JsonText::new("name"),
        CanonicalValue::text(text_field(case, "name")),
    );
    // The ledger and the catalog are built before the executor, exactly like
    // the oracle, so a refusal in the catalog leaves no execution report.
    let ledger = build_ledger(case);
    let catalog = match build_catalog(case) {
        Ok(catalog) => catalog,
        Err(error) => {
            finish_error(&mut outcome, error, ledger.as_deref(), None);
            return CanonicalValue::Object(outcome);
        }
    };
    let executor = match build_executor(case, workdir.path(), ledger.clone()) {
        Ok(executor) => executor,
        Err(error) => {
            finish_error(&mut outcome, error, ledger.as_deref(), None);
            return CanonicalValue::Object(outcome);
        }
    };
    let built = build_case(case, executor.clone(), ledger.clone(), catalog);
    match built {
        Err(error) => {
            finish_error(&mut outcome, error, ledger.as_deref(), Some(&executor));
        }
        Ok(built) => {
            let run_id = optional_text_field(case, "run_id").unwrap_or_else(|| "run-1".to_owned());
            let result = match &built.mode {
                RunMode::Tasks(tasks) => built.coordinator.run(tasks, &run_id),
                RunMode::Plan(steps) => built.coordinator.run_plan(steps, &run_id),
                RunMode::Objective(objective) => {
                    built.coordinator.run_objective(objective, &run_id).await
                }
            };
            match result {
                Ok(result) => {
                    outcome.insert(JsonText::new("status"), CanonicalValue::text("ok"));
                    outcome.insert(JsonText::new("result"), result.to_payload());
                    outcome.insert(
                        JsonText::new("degradations"),
                        CanonicalValue::Array(
                            result
                                .degraded_reasons()
                                .iter()
                                .map(|value| CanonicalValue::text(value.clone()))
                                .collect(),
                        ),
                    );
                }
                Err(error) => {
                    outcome.insert(JsonText::new("status"), CanonicalValue::text("error"));
                    outcome.insert(
                        JsonText::new("error_class"),
                        CanonicalValue::text(error.python_class_name()),
                    );
                    outcome.insert(
                        JsonText::new("error_message"),
                        CanonicalValue::text(error.message()),
                    );
                }
            }
            if let Some(ledger) = &built.ledger {
                outcome.insert(JsonText::new("ledger"), ledger_state(ledger));
            }
            outcome.insert(JsonText::new("calls"), calls_payload(&built.executor));
        }
    }
    CanonicalValue::Object(outcome)
}

/// Record a refusal of a case.
fn finish_error(
    outcome: &mut CanonicalObject,
    error: AgentError,
    ledger: Option<&BudgetLedger>,
    executor: Option<&ToolExecutor>,
) {
    outcome.insert(JsonText::new("status"), CanonicalValue::text("error"));
    outcome.insert(
        JsonText::new("error_class"),
        CanonicalValue::text(error.python_class_name()),
    );
    outcome.insert(
        JsonText::new("error_message"),
        CanonicalValue::text(error.message()),
    );
    if let Some(ledger) = ledger {
        outcome.insert(JsonText::new("ledger"), ledger_state(ledger));
    }
    if let Some(executor) = executor {
        outcome.insert(JsonText::new("calls"), calls_payload(executor));
    }
}

/// The path of the first difference between two JSON documents.
fn first_difference(left: &CanonicalValue, right: &CanonicalValue, path: &str) -> Option<String> {
    match (left, right) {
        (CanonicalValue::Object(left), CanonicalValue::Object(right)) => {
            for (key, value) in left {
                let child = format!("{path}.{}", key.to_string_lossy());
                let Some(other) = right.get(key) else {
                    return Some(child);
                };
                if let Some(found) = first_difference(value, other, &child) {
                    return Some(found);
                }
            }
            for key in right.keys() {
                if !left.contains_key(key) {
                    return Some(format!("{path}.{}", key.to_string_lossy()));
                }
            }
            None
        }
        (CanonicalValue::Array(left), CanonicalValue::Array(right)) => {
            if left.len() != right.len() {
                return Some(format!("{path}.length"));
            }
            for (index, (value, other)) in left.iter().zip(right.iter()).enumerate() {
                if let Some(found) = first_difference(value, other, &format!("{path}[{index}]")) {
                    return Some(found);
                }
            }
            None
        }
        _ => {
            if left == right {
                None
            } else {
                Some(path.to_owned())
            }
        }
    }
}

/// The case of a result document.
fn left_value(cases: &[CanonicalValue], index: usize) -> &CanonicalValue {
    &cases[index]
}

/// Run the whole scenario on the Rust side.
async fn run_scenario(scenario: &CanonicalValue) -> CanonicalValue {
    let mut cases = Vec::new();
    if let Some(CanonicalValue::Array(entries)) = get(scenario, "cases") {
        for entry in entries {
            cases.push(run_case(entry).await);
        }
    }
    object(vec![("cases", CanonicalValue::Array(cases))])
}

#[tokio::test]
async fn the_agent_matches_the_live_python_agent() {
    let scenario = scenario();
    let oracle = run_oracle(&scenario);
    let rust = run_scenario(&scenario).await;
    let oracle = normalize_timings(&oracle);
    let rust = normalize_timings(&rust);
    // Compare case by case so a mismatch names the case that differs.
    let oracle_cases = match get(&oracle, "cases") {
        Some(CanonicalValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let rust_cases = match get(&rust, "cases") {
        Some(CanonicalValue::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    assert_eq!(
        oracle_cases.len(),
        rust_cases.len(),
        "the case count differs"
    );
    for (index, (left, right)) in oracle_cases.iter().zip(rust_cases.iter()).enumerate() {
        if let Some(path) = first_difference(left, right, "") {
            panic!(
                "the Rust agent differs from the Python agent in case {index} ({}) at {path}",
                text_field(left_value(&oracle_cases, index), "name")
            );
        }
    }
    let expected = kala_json::canonical_json_dumps(&oracle).expect("the canonical oracle document");
    let actual = kala_json::canonical_json_dumps(&rust).expect("the canonical Rust document");
    assert_eq!(
        expected, actual,
        "the Rust agent differs from the Python agent"
    );
    assert_eq!(oracle_cases.len(), 36, "the differential covers every case");
}
