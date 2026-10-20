//! The agent coordinator.
//!
//! The coordinator runs a plan: it orders the tasks, dispatches the layers
//! that may run together, accounts for the budget, asks the policy engine
//! for the decisions the tools need and merges the answers.
//!
//! The order of the decisions is the security property of the module, so it
//! is reproduced exactly:
//!
//! 1. the task graph is validated and ordered;
//! 2. the budget admits the layer, or the layer is denied as a whole;
//! 3. a task whose dependency did not succeed is cancelled, never
//!    dispatched;
//! 4. the tool is resolved, and a task that requests a capability the tool
//!    does not declare is denied;
//! 5. the policy engine decides, and a tool that needs a decision without a
//!    policy engine is a hard failure, never a silent dispatch;
//! 6. the tool runs through the executor, which is the only path to a tool;
//! 7. the answers are merged and every merged result is validated against
//!    the output schema of its tool.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kala_budget::{BudgetError, BudgetLedger, BudgetUsage};
use kala_contracts::{ContractModel, SideEffectLevel, ToolCall, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_kernel::policy::PolicyEngine;
use kala_tools::executor::{ExecutionOptions, ToolExecutor};
use kala_tools::protocol::{ToolErrorCode, canonical_json_digest, canonical_json_dumps};
use kala_tools::registry::RegisteredTool;

use crate::gateway::{Clock, GatewayTimestamp, ModelGateway, python_strip};

use super::error::AgentError;
use super::outcome::{AgentRunResult, AgentStepOutcome};
use super::planner::Planner;
use super::task::{
    AgentTask, StepCatalog, dependency_layers, dependency_order, validate_task_graph,
};
use super::value::{MAX_ERROR_TEXT, bounded_identifier, truncate_code_points};

impl std::fmt::Debug for AgentCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentCoordinator")
            .field("has_policy", &self.policy.is_some())
            .field("has_planner", &self.planner.is_some())
            .field("has_ledger", &self.ledger.is_some())
            .field("has_gateway", &self.gateway.is_some())
            .field("has_catalog", &self.catalog.is_some())
            .field("max_parallel_tasks", &self.max_parallel)
            .finish()
    }
}

/// The maximum number of tasks of a layer that may run at the same time.
pub const MAX_PARALLEL_TASKS: usize = 8;

/// The maximum size of the merged results, in bytes.
pub const MAX_MERGED_RESULT_BYTES: usize = 4_194_304;

/// The side effect levels that need an explicit approval.
pub const APPROVAL_REQUIRED_EFFECTS: [SideEffectLevel; 3] = [
    SideEffectLevel::LocalWrite,
    SideEffectLevel::Network,
    SideEffectLevel::Egress,
];

/// The coordinator of the agent runs.
///
/// The coordinator is safe to share between threads, exactly like the Python
/// coordinator, which dispatches the tasks of a layer on a thread pool.
pub struct AgentCoordinator {
    executor: Arc<ToolExecutor>,
    policy: Option<Arc<PolicyEngine>>,
    planner: Option<Arc<dyn Planner>>,
    ledger: Option<Arc<BudgetLedger>>,
    gateway: Option<Arc<ModelGateway>>,
    catalog: Option<Arc<StepCatalog>>,
    max_parallel: usize,
    clock: Clock,
}

impl AgentCoordinator {
    /// Build a coordinator around a tool executor.
    #[must_use]
    pub fn new(executor: Arc<ToolExecutor>) -> Self {
        Self {
            executor,
            policy: None,
            planner: None,
            ledger: None,
            gateway: None,
            catalog: None,
            max_parallel: 4,
            clock: Arc::new(GatewayTimestamp::now),
        }
    }

    /// Attach a policy engine.
    #[must_use]
    pub fn with_policy(mut self, policy: Arc<PolicyEngine>) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Attach a planner.
    #[must_use]
    pub fn with_planner(mut self, planner: Arc<dyn Planner>) -> Self {
        self.planner = Some(planner);
        self
    }

    /// Attach a budget ledger.
    #[must_use]
    pub fn with_ledger(mut self, ledger: Arc<BudgetLedger>) -> Self {
        self.ledger = Some(ledger);
        self
    }

    /// Attach a model gateway.
    #[must_use]
    pub fn with_gateway(mut self, gateway: Arc<ModelGateway>) -> Self {
        self.gateway = Some(gateway);
        self
    }

    /// Attach a step catalog.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<StepCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Set the number of tasks that may run at the same time.
    ///
    /// The bound is the same as the Python module: between one and eight.
    pub fn with_max_parallel_tasks(
        mut self,
        max_parallel_tasks: usize,
    ) -> Result<Self, AgentError> {
        if !(1..=MAX_PARALLEL_TASKS).contains(&max_parallel_tasks) {
            return Err(AgentError::plan(format!(
                "max_parallel_tasks must be between 1 and {MAX_PARALLEL_TASKS}"
            )));
        }
        self.max_parallel = max_parallel_tasks;
        Ok(self)
    }

    /// Attach a clock.
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// The tool executor of the coordinator.
    #[must_use]
    pub fn executor(&self) -> &ToolExecutor {
        &self.executor
    }

    /// The tool registry of the coordinator.
    #[must_use]
    pub fn registry(&self) -> &kala_tools::registry::ToolRegistry {
        self.executor.registry()
    }

    /// The policy engine of the coordinator.
    #[must_use]
    pub fn policy(&self) -> Option<&Arc<PolicyEngine>> {
        self.policy.as_ref()
    }

    /// The planner of the coordinator.
    #[must_use]
    pub fn planner(&self) -> Option<&Arc<dyn Planner>> {
        self.planner.as_ref()
    }

    /// The budget ledger of the coordinator.
    #[must_use]
    pub fn ledger(&self) -> Option<&Arc<BudgetLedger>> {
        self.ledger.as_ref()
    }

    /// The model gateway of the coordinator.
    #[must_use]
    pub fn gateway(&self) -> Option<&Arc<ModelGateway>> {
        self.gateway.as_ref()
    }

    /// The step catalog of the coordinator.
    #[must_use]
    pub fn catalog(&self) -> Option<&Arc<StepCatalog>> {
        self.catalog.as_ref()
    }

    /// The number of tasks that may run at the same time.
    #[must_use]
    pub fn max_parallel_tasks(&self) -> usize {
        self.max_parallel
    }

    /// Plan the steps of a run.
    ///
    /// The coordinator needs a planner, and the objective must be a nonempty
    /// string. The plan the planner returns is validated before it is
    /// handed back, so a planner cannot widen the authority of a run.
    pub async fn plan(&self, objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        let Some(planner) = &self.planner else {
            return Err(AgentError::plan(
                "no planner is configured for this coordinator",
            ));
        };
        if python_strip(objective).is_empty() {
            return Err(AgentError::plan(
                "the planning objective must be a nonempty string",
            ));
        }
        let tasks = planner.plan(objective).await?;
        validate_task_graph(&tasks)
    }

    /// Run a set of tasks.
    ///
    /// The tasks are ordered, the layers that may run together are
    /// dispatched, and the answers are merged.
    pub fn run(&self, tasks: &[AgentTask], run_id: &str) -> Result<AgentRunResult, AgentError> {
        let identifier = bounded_identifier(run_id, "run_id")?;
        let normalized = dependency_order(&validate_task_graph(tasks)?)?;
        let mut outcomes: Vec<(String, AgentStepOutcome)> = Vec::with_capacity(normalized.len());
        let mut budget_denied: Vec<String> = Vec::new();
        for layer in dependency_layers(&normalized)? {
            let mut dispatchable: Vec<AgentTask> = Vec::new();
            for task in &layer {
                let blocked: Vec<String> = task
                    .depends_on()
                    .iter()
                    .filter(|dependency| {
                        !outcomes
                            .iter()
                            .find(|(step_id, _)| step_id == *dependency)
                            .map(|(_, outcome)| outcome.succeeded())
                            .unwrap_or(false)
                    })
                    .cloned()
                    .collect();
                if !blocked.is_empty() {
                    let outcome = self.blocked_outcome(
                        task,
                        &identifier,
                        &format!("dependency did not succeed: {}", blocked.join(", ")),
                    )?;
                    outcomes.push((task.step_id().to_owned(), outcome));
                    continue;
                }
                dispatchable.push(task.clone());
            }
            for outcome in self.dispatch_layer(&dispatchable, &identifier)? {
                if outcome.error_code() == Some(ToolErrorCode::BudgetExhausted.as_str()) {
                    budget_denied.push(outcome.step_id().to_owned());
                }
                outcomes.push((outcome.step_id().to_owned(), outcome));
            }
        }
        let ordered: Vec<AgentStepOutcome> = normalized
            .iter()
            .map(|task| {
                outcomes
                    .iter()
                    .find(|(step_id, _)| step_id == task.step_id())
                    .map(|(_, outcome)| outcome.clone())
                    .expect("every task has an outcome")
            })
            .collect();
        let (merged, digests) = self.merge(&ordered)?;
        let mut denied: Vec<String> = budget_denied.clone();
        denied.sort();
        denied.dedup();
        AgentRunResult::from_parts(
            &identifier,
            ordered,
            &merged,
            &digests,
            &self.degradation_reasons(),
            &denied,
        )
    }

    /// Run the tasks of a plan.
    pub fn run_plan(
        &self,
        plan_steps: &[kala_contracts::PlanStep],
        run_id: &str,
    ) -> Result<AgentRunResult, AgentError> {
        let Some(catalog) = &self.catalog else {
            return Err(AgentError::plan(
                "no step catalog is configured for this coordinator",
            ));
        };
        let tasks = catalog.to_tasks(plan_steps)?;
        self.run(&tasks, run_id)
    }

    /// Plan an objective and run it.
    pub async fn run_objective(
        &self,
        objective: &str,
        run_id: &str,
    ) -> Result<AgentRunResult, AgentError> {
        let tasks = self.plan(objective).await?;
        self.run(&tasks, run_id)
    }

    /// Dispatch one layer of tasks.
    ///
    /// The budget admits the layer as a whole, and the tasks of the layer run
    /// in parallel. A failure of one task does not cancel the others: the
    /// first failure in task order is reported, exactly like the Python
    /// thread pool, which runs every task of the layer and raises the first
    /// error it yields.
    fn dispatch_layer(
        &self,
        tasks: &[AgentTask],
        run_id: &str,
    ) -> Result<Vec<AgentStepOutcome>, AgentError> {
        if tasks.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(denied) = self.admission_check(tasks, run_id)? {
            return Ok(denied);
        }
        if tasks.len() == 1 {
            return Ok(vec![self.dispatch(&tasks[0], run_id)?]);
        }
        let workers = self.max_parallel.min(tasks.len());
        let slots: Vec<std::sync::Mutex<Option<Result<AgentStepOutcome, AgentError>>>> = (0..tasks
            .len())
            .map(|_| std::sync::Mutex::new(None))
            .collect();
        let next = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        if index >= tasks.len() {
                            break;
                        }
                        let outcome = self.dispatch(&tasks[index], run_id);
                        let mut slot = slots[index]
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        *slot = Some(outcome);
                    }
                });
            }
        });
        let mut collected: Vec<Result<AgentStepOutcome, AgentError>> =
            Vec::with_capacity(tasks.len());
        for slot in &slots {
            let value = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .expect("every task of the layer was dispatched");
            collected.push(value);
        }
        for value in &collected {
            if let Err(error) = value {
                return Err(error.clone());
            }
        }
        Ok(collected
            .into_iter()
            .map(|value| value.expect("every task of the layer succeeded"))
            .collect())
    }

    /// Ask the budget whether the layer may run.
    ///
    /// The check is a check and not a charge: the executor reserves and
    /// settles the budget of every call it dispatches.
    fn admission_check(
        &self,
        tasks: &[AgentTask],
        run_id: &str,
    ) -> Result<Option<Vec<AgentStepOutcome>>, AgentError> {
        let Some(ledger) = &self.ledger else {
            return Ok(None);
        };
        let usage = BudgetUsage::new().with_tool_calls(tasks.len() as u64);
        match ledger.check(&usage) {
            Ok(()) => Ok(None),
            Err(BudgetError::Exhausted(details)) => {
                let message = truncate_code_points(&details.to_string(), MAX_ERROR_TEXT);
                let mut denied = Vec::with_capacity(tasks.len());
                for task in tasks {
                    denied.push(AgentStepOutcome::from_parts(
                        task.step_id(),
                        &task.call_id(run_id)?,
                        ToolRunStatus::Denied,
                        false,
                        task.tool_id(),
                        None,
                        Some(ToolErrorCode::BudgetExhausted.as_str()),
                        Some(&message),
                        &[],
                        0.0,
                        None,
                    )?);
                }
                Ok(Some(denied))
            }
            Err(error) => Err(AgentError::Budget(error)),
        }
    }

    /// The outcome of a task whose dependency did not succeed.
    fn blocked_outcome(
        &self,
        task: &AgentTask,
        run_id: &str,
        reason: &str,
    ) -> Result<AgentStepOutcome, AgentError> {
        AgentStepOutcome::from_parts(
            task.step_id(),
            &task.call_id(run_id)?,
            ToolRunStatus::Cancelled,
            false,
            task.tool_id(),
            None,
            None,
            Some(reason),
            &[],
            0.0,
            None,
        )
    }

    /// Dispatch one task.
    fn dispatch(&self, task: &AgentTask, run_id: &str) -> Result<AgentStepOutcome, AgentError> {
        let call_id = task.call_id(run_id)?;
        let tool = match self.registry().require(task.tool_id(), task.tool_version()) {
            Ok(tool) => tool,
            Err(error) => {
                return AgentStepOutcome::from_parts(
                    task.step_id(),
                    &call_id,
                    ToolRunStatus::Denied,
                    false,
                    task.tool_id(),
                    None,
                    Some(ToolErrorCode::UnknownTool.as_str()),
                    Some(&error.to_string()),
                    &[],
                    0.0,
                    None,
                );
            }
        };
        let undeclared: Vec<String> = task
            .required_capabilities()
            .iter()
            .filter(|capability| {
                !tool
                    .required_capabilities()
                    .iter()
                    .any(|required| required.as_str() == capability.as_str())
            })
            .cloned()
            .collect();
        if !undeclared.is_empty() {
            return AgentStepOutcome::from_parts(
                task.step_id(),
                &call_id,
                ToolRunStatus::Denied,
                false,
                tool.tool_id(),
                None,
                Some(ToolErrorCode::CapabilityDenied.as_str()),
                Some(&format!(
                    "the task requests undeclared capabilities: {}",
                    undeclared.join(", ")
                )),
                &[],
                0.0,
                None,
            );
        }
        let call = self.build_call(task, &tool, &call_id)?;
        if let Some((reason, code)) = self.preflight(&tool, &call)? {
            return AgentStepOutcome::from_parts(
                task.step_id(),
                &call_id,
                ToolRunStatus::Denied,
                false,
                tool.tool_id(),
                None,
                Some(code.as_str()),
                Some(&reason),
                &[],
                0.0,
                Some(&reason),
            );
        }
        let result = self.executor.execute(
            &call,
            &ExecutionOptions::new().with_timeout_seconds(task.timeout_seconds()),
        );
        AgentStepOutcome::from_tool_result(task.step_id(), &result, tool.tool_id())
    }

    /// Build the call of a task.
    fn build_call(
        &self,
        task: &AgentTask,
        tool: &RegisteredTool,
        call_id: &str,
    ) -> Result<ToolCall, AgentError> {
        let timeout_seconds = self
            .executor
            .profile()
            .timeout_for(task.timeout_seconds(), tool.descriptor().timeout_seconds());
        let issued_at = self.now()?;
        let value = CanonicalValue::object([
            ("call_id", CanonicalValue::text(call_id)),
            ("tool_id", CanonicalValue::text(tool.tool_id())),
            ("tool_version", CanonicalValue::text(tool.version())),
            (
                "arguments_json",
                CanonicalValue::text(task.arguments_json()?),
            ),
            (
                "requested_capabilities",
                CanonicalValue::array(
                    task.required_capabilities()
                        .iter()
                        .map(|capability| CanonicalValue::text(capability.clone()))
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "side_effect_level",
                CanonicalValue::text(tool.side_effect_level().as_str()),
            ),
            ("timeout_seconds", CanonicalValue::Float(timeout_seconds)),
            (
                "issued_at",
                CanonicalValue::text(issued_at.to_wire_string()),
            ),
        ]);
        ToolCall::from_value(&value)
            .map_err(|error| AgentError::Validation(error.message().to_owned()))
    }

    /// Ask the policy engine for the decision a tool needs.
    ///
    /// A tool that needs a decision and finds no policy engine is a hard
    /// failure: the call is never dispatched.
    fn preflight(
        &self,
        tool: &RegisteredTool,
        call: &ToolCall,
    ) -> Result<Option<(String, ToolErrorCode)>, AgentError> {
        if !Self::requires_approval(tool) {
            return Ok(None);
        }
        let Some(policy) = &self.policy else {
            return Err(AgentError::policy(format!(
                "tool {} requires a policy decision and no policy engine is attached",
                tool.tool_id()
            )));
        };
        let request = self
            .executor
            .policy_request(call)
            .map_err(AgentError::Executor)?;
        let decision = policy
            .decide(&request, None)
            .map_err(AgentError::PolicyError)?;
        if decision.allowed() {
            return Ok(None);
        }
        let reason = format!(
            "policy decision {} for request {}",
            decision.reason().as_str(),
            decision.request_id().as_str().unwrap_or_default()
        );
        let code = if decision.missing_capabilities().is_empty() {
            ToolErrorCode::ApprovalRequired
        } else {
            ToolErrorCode::CapabilityDenied
        };
        Ok(Some((reason, code)))
    }

    /// Whether a tool needs an explicit approval.
    #[must_use]
    pub fn requires_approval(tool: &RegisteredTool) -> bool {
        tool.descriptor().destructive()
            || APPROVAL_REQUIRED_EFFECTS.contains(&tool.side_effect_level())
    }

    /// Merge the answers of the steps that succeeded.
    ///
    /// Every merged result is validated against the output schema of its
    /// tool, and the merged document is bounded.
    fn merge(
        &self,
        outcomes: &[AgentStepOutcome],
    ) -> Result<(CanonicalValue, Vec<(String, String)>), AgentError> {
        let mut merged = CanonicalObject::new();
        let mut digests: Vec<(String, String)> = Vec::new();
        for outcome in outcomes {
            if !outcome.succeeded() {
                continue;
            }
            let Some(result) = outcome.result() else {
                continue;
            };
            let tool = self
                .registry()
                .require(outcome.tool_id(), None)
                .map_err(AgentError::Registry)?;
            tool.validate_result(result).map_err(|error| {
                AgentError::merge(format!(
                    "the merged result of step {} failed schema validation: {error}",
                    outcome.step_id()
                ))
            })?;
            let key = JsonText::new(outcome.step_id());
            if merged.contains_key(&key) {
                return Err(AgentError::merge(format!(
                    "a merged result already exists for {}",
                    outcome.step_id()
                )));
            }
            merged.insert(key, result.clone());
            digests.push((
                outcome.step_id().to_owned(),
                canonical_json_digest(result).map_err(AgentError::Protocol)?,
            ));
        }
        let serialized = canonical_json_dumps(
            &CanonicalValue::Object(merged.clone()),
            MAX_MERGED_RESULT_BYTES,
        )
        .map_err(AgentError::Protocol)?;
        if serialized.len() > MAX_MERGED_RESULT_BYTES {
            return Err(AgentError::merge(
                "the merged results exceed the bounded size",
            ));
        }
        Ok((CanonicalValue::Object(merged), digests))
    }

    /// The degradation reasons the planner recorded.
    fn degradation_reasons(&self) -> Vec<String> {
        match &self.planner {
            Some(planner) => planner.degradations(),
            None => Vec::new(),
        }
    }

    /// The current instant, as an aware timestamp.
    fn now(&self) -> Result<kala_contracts::Timestamp, AgentError> {
        let value = (self.clock)();
        let Some(aware) = value.as_aware() else {
            return Err(AgentError::base(
                "the coordinator clock must return an aware datetime",
            ));
        };
        kala_contracts::Timestamp::from_absolute_micros(aware.unix_micros())
            .map_err(|error| AgentError::Validation(error.message().to_owned()))
    }
}
