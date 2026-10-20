//! The agent task, the step catalog and the task graph.
//!
//! The types mirror the Python dataclasses `AgentTask` and `CatalogStep` and
//! the class `StepCatalog` one for one, including the order in which every
//! field is validated.

use kala_contracts::PlanStep;
use kala_json::{CanonicalObject, CanonicalValue};
use kala_tools::protocol::{
    MAX_JSON_BYTES, canonical_json_dumps, sha256_hex_text, strict_json_loads,
};

use crate::gateway::python_repr;

use super::error::AgentError;
use super::value::{
    MAX_BOUNDED_TEXT, bounded_identifier, bounded_seconds, bounded_text, freeze_object,
    identifier_tuple,
};

/// The maximum number of tasks a run or a catalog may hold.
pub const MAX_AGENT_TASKS: usize = 64;

/// One unit of work of a run.
///
/// The Python dataclass validates its fields in `__post_init__`, in this
/// order: `step_id`, `tool_id`, `tool_version`, `arguments`, the
/// self-dependency check, `required_capabilities`, `depends_on`, `intent` and
/// `timeout_seconds`. The Rust port keeps that order in
/// [`AgentTask::validate`], so an invalid task reports the same first
/// failure on both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentTask {
    step_id: String,
    tool_id: String,
    arguments: CanonicalValue,
    tool_version: Option<String>,
    required_capabilities: Vec<String>,
    depends_on: Vec<String>,
    intent: String,
    timeout_seconds: Option<f64>,
}

impl AgentTask {
    /// Build a task from every field at once.
    ///
    /// The fields are validated in the order of the Python `__post_init__`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        step_id: &str,
        tool_id: &str,
        arguments: &CanonicalValue,
        tool_version: Option<&str>,
        required_capabilities: &[String],
        depends_on: &[String],
        intent: &str,
        timeout_seconds: Option<f64>,
    ) -> Result<Self, AgentError> {
        let mut task = Self {
            step_id: step_id.to_owned(),
            tool_id: tool_id.to_owned(),
            arguments: arguments.clone(),
            tool_version: tool_version.map(str::to_owned),
            required_capabilities: required_capabilities.to_vec(),
            depends_on: depends_on.to_vec(),
            intent: intent.to_owned(),
            timeout_seconds,
        };
        task.validate()?;
        Ok(task)
    }

    /// Build a task with no arguments, no version and no dependencies.
    pub fn new(step_id: &str, tool_id: &str) -> Result<Self, AgentError> {
        Self::from_parts(
            step_id,
            tool_id,
            &CanonicalValue::Object(CanonicalObject::new()),
            None,
            &[],
            &[],
            "",
            None,
        )
    }

    /// Validate every field, in the order of the Python `__post_init__`.
    fn validate(&mut self) -> Result<(), AgentError> {
        self.step_id = bounded_identifier(&self.step_id, "step_id")?;
        self.tool_id = bounded_identifier(&self.tool_id, "tool_id")?;
        if let Some(version) = self.tool_version.as_deref() {
            self.tool_version = Some(bounded_identifier(version, "tool_version")?);
        }
        self.arguments = freeze_object(&self.arguments, "arguments")?;
        let dependencies = identifier_tuple(&self.depends_on, "depends_on")?;
        if dependencies.contains(&self.step_id) {
            return Err(AgentError::plan("an agent task cannot depend on itself"));
        }
        self.required_capabilities =
            identifier_tuple(&self.required_capabilities, "required_capabilities")?;
        self.depends_on = dependencies;
        self.intent = bounded_text(&self.intent, "intent", MAX_BOUNDED_TEXT)?;
        self.timeout_seconds = bounded_seconds(self.timeout_seconds, "timeout_seconds")?;
        Ok(())
    }

    /// Return a copy with different arguments.
    ///
    /// The Python implementation rebuilds the dataclass, so every field is
    /// validated again.
    pub fn with_arguments(&self, arguments: &CanonicalValue) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.arguments = arguments.clone();
        task.validate()?;
        Ok(task)
    }

    /// Return a copy with a different tool version.
    pub fn with_tool_version(&self, tool_version: Option<&str>) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.tool_version = tool_version.map(str::to_owned);
        task.validate()?;
        Ok(task)
    }

    /// Return a copy with different required capabilities.
    pub fn with_required_capabilities(
        &self,
        required_capabilities: &[String],
    ) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.required_capabilities = required_capabilities.to_vec();
        task.validate()?;
        Ok(task)
    }

    /// Return a copy with different dependencies.
    pub fn with_depends_on(&self, depends_on: &[String]) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.depends_on = depends_on.to_vec();
        task.validate()?;
        Ok(task)
    }

    /// Return a copy with a different intent.
    pub fn with_intent(&self, intent: &str) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.intent = intent.to_owned();
        task.validate()?;
        Ok(task)
    }

    /// Return a copy with a different timeout.
    pub fn with_timeout_seconds(&self, timeout_seconds: Option<f64>) -> Result<Self, AgentError> {
        let mut task = self.clone();
        task.timeout_seconds = timeout_seconds;
        task.validate()?;
        Ok(task)
    }

    /// The step identifier.
    #[must_use]
    pub fn step_id(&self) -> &str {
        &self.step_id
    }

    /// The tool identifier.
    #[must_use]
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// The frozen task arguments.
    #[must_use]
    pub fn arguments(&self) -> &CanonicalValue {
        &self.arguments
    }

    /// The tool version, when the task pins one.
    #[must_use]
    pub fn tool_version(&self) -> Option<&str> {
        self.tool_version.as_deref()
    }

    /// The required capabilities, sorted and deduplicated.
    #[must_use]
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }

    /// The dependencies, sorted and deduplicated.
    #[must_use]
    pub fn depends_on(&self) -> &[String] {
        &self.depends_on
    }

    /// The declared intent.
    #[must_use]
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// The deadline override, in seconds.
    #[must_use]
    pub fn timeout_seconds(&self) -> Option<f64> {
        self.timeout_seconds
    }

    /// The canonical JSON text of the task arguments.
    pub fn arguments_json(&self) -> Result<String, AgentError> {
        canonical_json_dumps(&self.arguments, MAX_JSON_BYTES).map_err(AgentError::Protocol)
    }

    /// The digest of the canonical task arguments.
    pub fn arguments_digest(&self) -> Result<String, AgentError> {
        Ok(sha256_hex_text(&self.arguments_json()?))
    }

    /// The call identifier of the task inside a run.
    ///
    /// The run identifier is validated with the same rules as every other
    /// identifier, so an unsafe run identifier is refused here.
    pub fn call_id(&self, run_id: &str) -> Result<String, AgentError> {
        Ok(format!(
            "{}-{}",
            bounded_identifier(run_id, "run_id")?,
            self.step_id
        ))
    }
}

/// One entry of the step catalog.
///
/// A catalog step is the closed vocabulary a planner may choose from: it
/// names the tool, the default arguments, the capabilities the tool needs
/// and the keywords the offline planner scores it with.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogStep {
    catalog_step_id: String,
    tool_id: String,
    default_arguments: CanonicalValue,
    tool_version: Option<String>,
    required_capabilities: Vec<String>,
    keywords: Vec<String>,
    intent: String,
}

impl CatalogStep {
    /// Build a catalog step from every field at once.
    ///
    /// The fields are validated in the order of the Python `__post_init__`:
    /// `catalog_step_id`, `tool_id`, `tool_version`, `default_arguments`,
    /// `required_capabilities`, `keywords` and `intent`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        catalog_step_id: &str,
        tool_id: &str,
        default_arguments: &CanonicalValue,
        tool_version: Option<&str>,
        required_capabilities: &[String],
        keywords: &[String],
        intent: &str,
    ) -> Result<Self, AgentError> {
        let mut step = Self {
            catalog_step_id: catalog_step_id.to_owned(),
            tool_id: tool_id.to_owned(),
            default_arguments: default_arguments.clone(),
            tool_version: tool_version.map(str::to_owned),
            required_capabilities: required_capabilities.to_vec(),
            keywords: keywords.to_vec(),
            intent: intent.to_owned(),
        };
        step.validate()?;
        Ok(step)
    }

    /// Build a catalog step with no arguments, no version and no keywords.
    pub fn new(catalog_step_id: &str, tool_id: &str) -> Result<Self, AgentError> {
        Self::from_parts(
            catalog_step_id,
            tool_id,
            &CanonicalValue::Object(CanonicalObject::new()),
            None,
            &[],
            &[],
            "",
        )
    }

    /// Validate every field, in the order of the Python `__post_init__`.
    fn validate(&mut self) -> Result<(), AgentError> {
        self.catalog_step_id = bounded_identifier(&self.catalog_step_id, "catalog_step_id")?;
        self.tool_id = bounded_identifier(&self.tool_id, "tool_id")?;
        if let Some(version) = self.tool_version.as_deref() {
            self.tool_version = Some(bounded_identifier(version, "tool_version")?);
        }
        self.default_arguments = freeze_object(&self.default_arguments, "default_arguments")?;
        self.required_capabilities =
            identifier_tuple(&self.required_capabilities, "required_capabilities")?;
        self.keywords = identifier_tuple(&self.keywords, "keywords")?;
        self.intent = bounded_text(&self.intent, "intent", MAX_BOUNDED_TEXT)?;
        Ok(())
    }

    /// The catalog step identifier.
    #[must_use]
    pub fn catalog_step_id(&self) -> &str {
        &self.catalog_step_id
    }

    /// The tool identifier.
    #[must_use]
    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// The frozen default arguments.
    #[must_use]
    pub fn default_arguments(&self) -> &CanonicalValue {
        &self.default_arguments
    }

    /// The tool version, when the step pins one.
    #[must_use]
    pub fn tool_version(&self) -> Option<&str> {
        self.tool_version.as_deref()
    }

    /// The required capabilities, sorted and deduplicated.
    #[must_use]
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }

    /// The scoring keywords, sorted and deduplicated.
    #[must_use]
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    /// The declared intent.
    #[must_use]
    pub fn intent(&self) -> &str {
        &self.intent
    }

    /// Turn the catalog step into a task.
    ///
    /// The parameters are merged over the default arguments. A parameter the
    /// catalog step does not declare is refused, so a plan can never widen
    /// the argument surface of a step.
    pub fn to_task(
        &self,
        step_id: &str,
        parameters: Option<&CanonicalValue>,
        depends_on: &[String],
    ) -> Result<AgentTask, AgentError> {
        let CanonicalValue::Object(defaults) = &self.default_arguments else {
            return Err(AgentError::plan(
                "default_arguments must be a JSON object".to_owned(),
            ));
        };
        let mut merged = defaults.clone();
        match parameters {
            None => {}
            Some(CanonicalValue::Object(overrides)) => {
                for (key, value) in overrides {
                    if !defaults.contains_key(key) {
                        return Err(AgentError::plan(format!(
                            "parameter {} is not declared by catalog step {}",
                            python_repr(&key.to_string_lossy()),
                            self.catalog_step_id
                        )));
                    }
                    merged.insert(key.clone(), value.clone());
                }
            }
            Some(_) => {
                // The Python implementation reaches `parameters.items()` and
                // raises an `AttributeError` for a value that is not a
                // mapping. The Rust port refuses it explicitly instead of
                // ignoring it.
                return Err(AgentError::plan("parameters must be a JSON object"));
            }
        }
        AgentTask::from_parts(
            step_id,
            &self.tool_id,
            &CanonicalValue::Object(merged),
            self.tool_version.as_deref(),
            &self.required_capabilities,
            depends_on,
            &self.intent,
            None,
        )
    }
}

/// The closed vocabulary a planner may choose from.
///
/// The catalog keeps the registration order of its entries, exactly like the
/// Python dictionary it replaces, because the order is observable: the
/// offline planner scores the entries in that order and the model planner
/// publishes the entry identifiers as the enum of its response schema.
#[derive(Clone, Debug, PartialEq)]
pub struct StepCatalog {
    steps: Vec<CatalogStep>,
}

impl StepCatalog {
    /// Build a catalog from its entries.
    ///
    /// A duplicate catalog step identifier is refused, and so is a catalog
    /// that holds more entries than the bound.
    pub fn new(steps: Vec<CatalogStep>) -> Result<Self, AgentError> {
        Self::with_max_steps(steps, MAX_AGENT_TASKS)
    }

    /// Build a catalog with a smaller bound.
    pub fn with_max_steps(steps: Vec<CatalogStep>, max_steps: usize) -> Result<Self, AgentError> {
        if max_steps < 1 {
            return Err(AgentError::plan("max_steps must be a positive integer"));
        }
        let mut selected: Vec<CatalogStep> = Vec::with_capacity(steps.len());
        for step in steps {
            if selected
                .iter()
                .any(|entry| entry.catalog_step_id() == step.catalog_step_id())
            {
                return Err(AgentError::plan(format!(
                    "catalog step {} is declared twice",
                    step.catalog_step_id()
                )));
            }
            selected.push(step);
        }
        if selected.len() > max_steps {
            return Err(AgentError::plan(format!(
                "the step catalog exceeds {max_steps} entries"
            )));
        }
        Ok(Self { steps: selected })
    }

    /// The number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether the catalog holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Whether a catalog step identifier is declared.
    #[must_use]
    pub fn contains(&self, catalog_step_id: &str) -> bool {
        self.steps
            .iter()
            .any(|step| step.catalog_step_id() == catalog_step_id)
    }

    /// The catalog step identifiers, in registration order.
    #[must_use]
    pub fn step_ids(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|step| step.catalog_step_id().to_owned())
            .collect()
    }

    /// Look a catalog step up.
    pub fn require(&self, catalog_step_id: &str) -> Result<&CatalogStep, AgentError> {
        let key = bounded_identifier(catalog_step_id, "catalog_step_id")?;
        self.steps
            .iter()
            .find(|step| step.catalog_step_id() == key)
            .ok_or_else(|| AgentError::plan(format!("catalog step {key} is not declared")))
    }

    /// Turn a plan into the tasks of a run.
    ///
    /// The plan is bounded, every step must resolve to a declared catalog
    /// step, every parameter must be declared by that step, and the resulting
    /// task graph is validated.
    pub fn to_tasks(&self, plan_steps: &[PlanStep]) -> Result<Vec<AgentTask>, AgentError> {
        if plan_steps.is_empty() {
            return Err(AgentError::plan("a plan must contain at least one step"));
        }
        if plan_steps.len() > self.steps.len().min(MAX_AGENT_TASKS) {
            return Err(AgentError::plan("the plan exceeds the bounded step count"));
        }
        let mut tasks = Vec::with_capacity(plan_steps.len());
        for plan_step in plan_steps {
            let entry = self.require(plan_step.catalog_step_id())?;
            let parameters = plan_parameters(plan_step.parameters_json().as_str(), entry)?;
            let dependencies = plan_step
                .depends_on()
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect::<Vec<_>>();
            tasks.push(entry.to_task(plan_step.step_id(), Some(&parameters), &dependencies)?);
        }
        validate_task_graph(&tasks)
    }
}

/// Decode and check the parameters of a planned step.
///
/// The parameters must be strict JSON, they must be a JSON object and every
/// key must be declared by the catalog step.
pub fn plan_parameters(
    parameters_json: &str,
    entry: &CatalogStep,
) -> Result<CanonicalValue, AgentError> {
    let decoded =
        strict_json_loads(parameters_json.as_bytes(), MAX_JSON_BYTES).map_err(|error| {
            AgentError::plan(format!("plan parameters are not strict JSON: {error}"))
        })?;
    let CanonicalValue::Object(entries) = &decoded else {
        return Err(AgentError::plan("plan parameters must be a JSON object"));
    };
    let CanonicalValue::Object(defaults) = &entry.default_arguments else {
        return Err(AgentError::plan("default_arguments must be a JSON object"));
    };
    let mut unknown: Vec<String> = entries
        .keys()
        .filter(|key| !defaults.contains_key(*key))
        .map(|key| key.to_string_lossy())
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(AgentError::plan(format!(
            "parameters are not declared by catalog step {}: {}",
            entry.catalog_step_id(),
            unknown.join(", ")
        )));
    }
    Ok(decoded)
}

/// Validate a task graph and return it in dependency order.
///
/// The checks are the ones of the Python `_validate_task_graph`: the graph
/// must not be empty, it must not hold more than [`MAX_AGENT_TASKS`] tasks,
/// the step identifiers must be unique and every dependency must be declared.
pub fn validate_task_graph(tasks: &[AgentTask]) -> Result<Vec<AgentTask>, AgentError> {
    if tasks.is_empty() {
        return Err(AgentError::plan("a run requires at least one task"));
    }
    if tasks.len() > MAX_AGENT_TASKS {
        return Err(AgentError::plan(format!(
            "a run cannot exceed {MAX_AGENT_TASKS} tasks"
        )));
    }
    let mut step_ids: Vec<String> = Vec::with_capacity(tasks.len());
    for task in tasks {
        if step_ids.iter().any(|step_id| step_id == task.step_id()) {
            return Err(AgentError::plan(format!(
                "duplicate task identifier: {}",
                task.step_id()
            )));
        }
        step_ids.push(task.step_id().to_owned());
    }
    for task in tasks {
        let unknown: Vec<String> = task
            .depends_on()
            .iter()
            .filter(|dependency| !step_ids.iter().any(|step_id| step_id == *dependency))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            return Err(AgentError::plan(format!(
                "task {} depends on undeclared tasks: {}",
                task.step_id(),
                unknown.join(", ")
            )));
        }
    }
    dependency_order(tasks)
}

/// Order a task graph so that every task follows its dependencies.
///
/// The tasks of one batch are ordered by their step identifier, exactly like
/// the Python implementation, which sorts the remaining tasks by identifier
/// before it picks the ones that are ready.
pub fn dependency_order(tasks: &[AgentTask]) -> Result<Vec<AgentTask>, AgentError> {
    let mut remaining: Vec<AgentTask> = tasks.to_vec();
    let mut ordered: Vec<AgentTask> = Vec::with_capacity(tasks.len());
    while !remaining.is_empty() {
        let mut ready: Vec<AgentTask> = Vec::new();
        for task in sort_by_step_id(&remaining) {
            let satisfied = task
                .depends_on()
                .iter()
                .all(|dependency| !remaining.iter().any(|other| other.step_id() == dependency));
            if satisfied {
                ready.push(task);
            }
        }
        if ready.is_empty() {
            return Err(AgentError::plan("task dependencies contain a cycle"));
        }
        for task in &ready {
            ordered.push(task.clone());
        }
        let step_ids: std::collections::BTreeSet<String> =
            ready.iter().map(|task| task.step_id().to_owned()).collect();
        remaining.retain(|task| !step_ids.contains(task.step_id()));
    }
    Ok(ordered)
}

/// Split a task graph into the layers that may run together.
///
/// A task belongs to the first layer whose predecessors are all complete.
pub fn dependency_layers(tasks: &[AgentTask]) -> Result<Vec<Vec<AgentTask>>, AgentError> {
    let mut remaining: Vec<AgentTask> = tasks.to_vec();
    let mut completed: Vec<String> = Vec::new();
    let mut layers: Vec<Vec<AgentTask>> = Vec::new();
    while !remaining.is_empty() {
        let mut ready: Vec<AgentTask> = Vec::new();
        for task in sort_by_step_id(&remaining) {
            let satisfied = task
                .depends_on()
                .iter()
                .all(|dependency| completed.contains(dependency));
            if satisfied {
                ready.push(task);
            }
        }
        if ready.is_empty() {
            return Err(AgentError::plan("task dependencies contain a cycle"));
        }
        for task in &ready {
            completed.push(task.step_id().to_owned());
        }
        let step_ids: std::collections::BTreeSet<String> =
            ready.iter().map(|task| task.step_id().to_owned()).collect();
        remaining.retain(|task| !step_ids.contains(task.step_id()));
        layers.push(ready);
    }
    Ok(layers)
}

/// Sort a task list by its step identifiers.
fn sort_by_step_id(tasks: &[AgentTask]) -> Vec<AgentTask> {
    let mut sorted = tasks.to_vec();
    sorted.sort_by(|left, right| left.step_id().cmp(right.step_id()));
    sorted
}
