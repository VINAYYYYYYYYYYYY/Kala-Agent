//! The planners of the agent runtime.
//!
//! Three planners are ported from the Python module:
//!
//! * [`OfflinePlanner`] scores the catalog entries against the objective
//!   with a deterministic token overlap. It never calls a provider.
//! * [`ModelPlanner`] asks the model gateway for a bounded plan and
//!   validates the answer against the step catalog before anything runs
//!   (ADR-0003, D3).
//! * [`DegradingPlanner`] wraps a primary planner and a deterministic
//!   fallback. It degrades only when the degradation is approved, and it
//!   never degrades an authoritative refusal (ADR-0001, D8).

use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock, Mutex};

use kala_contracts::{ContractModel, PlanStep};
use kala_json::{CanonicalObject, CanonicalValue};
use kala_tools::protocol::{canonical_json_dumps, sha256_hex_text};
use regex::Regex;

use crate::gateway::{
    GatewayError, ModelError, ModelGateway, ModelMessage, ModelRequest, ModelResponse, ModelRole,
    python_strip,
};

use super::error::AgentError;
use super::task::{AgentTask, StepCatalog, validate_task_graph};
use super::value::{MAX_BOUNDED_TEXT, MAX_ERROR_TEXT, bounded_text, truncate_code_points};

/// The token pattern of the offline planner.
///
/// The Python module uses `re.compile(r"[a-z0-9]+")` and matches it against
/// the lower-cased objective.
static TOKEN_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z0-9]+").expect("the token pattern compiles"));

/// The contract every planner implements.
///
/// The Python module declares the runtime checkable `Planner` protocol with a
/// single `plan` method. The Rust port declares it as a trait; the method is
/// asynchronous because the model gateway is asynchronous.
#[async_trait::async_trait]
pub trait Planner: Send + Sync {
    /// Plan the steps of a run.
    async fn plan(&self, objective: &str) -> Result<Vec<AgentTask>, AgentError>;

    /// The degradation reasons the planner recorded.
    ///
    /// The Python coordinator reads the reasons of a `DegradingPlanner` with
    /// an `isinstance` check. The Rust port asks the planner, and only a
    /// degrading planner answers with reasons.
    fn degradations(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The deterministic planner.
///
/// The planner scores every catalog entry against the tokens of the
/// objective. An entry scores the number of tokens it shares with the
/// objective, plus one when the lower-cased tool identifier is itself a
/// token of the objective. The entries that score at least one point are
/// selected, most promising first, and the identifier breaks a tie.
#[derive(Clone, Debug)]
pub struct OfflinePlanner {
    catalog: Arc<StepCatalog>,
    fallback_step_id: Option<String>,
}

impl OfflinePlanner {
    /// Build an offline planner.
    ///
    /// The fallback step must be declared by the catalog, exactly like the
    /// Python constructor.
    pub fn new(
        catalog: Arc<StepCatalog>,
        fallback_step_id: Option<&str>,
    ) -> Result<Self, AgentError> {
        if let Some(fallback_step_id) = fallback_step_id {
            catalog.require(fallback_step_id)?;
        }
        Ok(Self {
            catalog,
            fallback_step_id: fallback_step_id.map(str::to_owned),
        })
    }

    /// The catalog of the planner.
    #[must_use]
    pub fn catalog(&self) -> &StepCatalog {
        &self.catalog
    }

    /// The fallback step of the planner.
    #[must_use]
    pub fn fallback_step_id(&self) -> Option<&str> {
        self.fallback_step_id.as_deref()
    }

    /// The tokens of an objective.
    fn tokens(objective: &str) -> BTreeSet<String> {
        TOKEN_PATTERN
            .find_iter(&objective.to_lowercase())
            .map(|found| found.as_str().to_owned())
            .collect()
    }
}

#[async_trait::async_trait]
impl Planner for OfflinePlanner {
    async fn plan(&self, objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        let objective = bounded_text(objective, "objective", MAX_BOUNDED_TEXT)?;
        let tokens = Self::tokens(&objective);
        let mut scored: Vec<(usize, String)> = Vec::new();
        for catalog_step_id in self.catalog.step_ids() {
            let entry = self.catalog.require(&catalog_step_id)?;
            let mut keywords: BTreeSet<String> = BTreeSet::new();
            for keyword in entry.keywords() {
                for token in keyword.split('.') {
                    keywords.insert(token.to_owned());
                }
            }
            let mut score = tokens.intersection(&keywords).count();
            if tokens.contains(&entry.tool_id().to_lowercase()) {
                score += 1;
            }
            if score > 0 {
                scored.push((score, catalog_step_id));
            }
        }
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        let selected: Vec<String> = if !scored.is_empty() {
            scored
                .into_iter()
                .map(|(_, catalog_step_id)| catalog_step_id)
                .collect()
        } else if let Some(fallback_step_id) = &self.fallback_step_id {
            vec![fallback_step_id.clone()]
        } else {
            return Err(AgentError::planner_failure(
                "no catalog step matches the objective",
            ));
        };
        let mut tasks = Vec::with_capacity(selected.len());
        for (index, catalog_step_id) in selected.iter().enumerate() {
            tasks.push(self.catalog.require(catalog_step_id)?.to_task(
                &format!("step-{}", index + 1),
                None,
                &[],
            )?);
        }
        validate_task_graph(&tasks)
    }
}

/// The planner that asks the model gateway for a plan.
///
/// The plan the model returns is data, not control flow (ADR-0003, D3): the
/// planner validates it against the step catalog before it is handed to the
/// coordinator, and a step that does not resolve to a declared catalog entry
/// is refused.
#[derive(Clone)]
pub struct ModelPlanner {
    gateway: Arc<ModelGateway>,
    catalog: Arc<StepCatalog>,
    max_tokens: i128,
}

impl ModelPlanner {
    /// Build a model planner.
    ///
    /// The output bound must be a positive integer, exactly like the Python
    /// constructor.
    pub fn new(
        gateway: Arc<ModelGateway>,
        catalog: Arc<StepCatalog>,
        max_tokens: i128,
    ) -> Result<Self, AgentError> {
        if max_tokens < 1 {
            return Err(AgentError::plan("max_tokens must be a positive integer"));
        }
        Ok(Self {
            gateway,
            catalog,
            max_tokens,
        })
    }

    /// The catalog of the planner.
    #[must_use]
    pub fn catalog(&self) -> &StepCatalog {
        &self.catalog
    }

    /// The model gateway of the planner.
    #[must_use]
    pub fn gateway(&self) -> &ModelGateway {
        &self.gateway
    }

    /// The output bound of the planner.
    #[must_use]
    pub fn max_tokens(&self) -> i128 {
        self.max_tokens
    }

    /// The response schema the planner publishes to the model.
    ///
    /// The schema is the closed vocabulary of the plan: the entries of the
    /// catalog, the intent of the step and the parameters of the step.
    #[must_use]
    pub fn response_schema(&self) -> CanonicalValue {
        let mut catalog_step = CanonicalObject::new();
        catalog_step.insert(
            kala_json::JsonText::new("enum"),
            CanonicalValue::array(
                self.catalog
                    .step_ids()
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        catalog_step.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("string"),
        );
        let mut intent = CanonicalObject::new();
        intent.insert(
            kala_json::JsonText::new("maxLength"),
            CanonicalValue::integer(MAX_BOUNDED_TEXT as i128),
        );
        intent.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("string"),
        );
        let mut parameters = CanonicalObject::new();
        parameters.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("object"),
        );
        let mut step_properties = CanonicalObject::new();
        step_properties.insert(
            kala_json::JsonText::new("catalog_step_id"),
            CanonicalValue::Object(catalog_step),
        );
        step_properties.insert(
            kala_json::JsonText::new("intent"),
            CanonicalValue::Object(intent),
        );
        step_properties.insert(
            kala_json::JsonText::new("parameters"),
            CanonicalValue::Object(parameters),
        );
        let mut step = CanonicalObject::new();
        step.insert(
            kala_json::JsonText::new("additionalProperties"),
            CanonicalValue::Bool(false),
        );
        step.insert(
            kala_json::JsonText::new("properties"),
            CanonicalValue::Object(step_properties),
        );
        step.insert(
            kala_json::JsonText::new("required"),
            CanonicalValue::array(
                ["catalog_step_id", "intent", "parameters"]
                    .into_iter()
                    .map(CanonicalValue::text)
                    .collect::<Vec<_>>(),
            ),
        );
        step.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("object"),
        );
        let mut steps = CanonicalObject::new();
        steps.insert(
            kala_json::JsonText::new("items"),
            CanonicalValue::Object(step),
        );
        steps.insert(
            kala_json::JsonText::new("maxItems"),
            CanonicalValue::integer(super::task::MAX_AGENT_TASKS as i128),
        );
        steps.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("array"),
        );
        let mut properties = CanonicalObject::new();
        properties.insert(
            kala_json::JsonText::new("steps"),
            CanonicalValue::Object(steps),
        );
        let mut schema = CanonicalObject::new();
        schema.insert(
            kala_json::JsonText::new("additionalProperties"),
            CanonicalValue::Bool(false),
        );
        schema.insert(
            kala_json::JsonText::new("properties"),
            CanonicalValue::Object(properties),
        );
        schema.insert(
            kala_json::JsonText::new("required"),
            CanonicalValue::array(vec![CanonicalValue::text("steps")]),
        );
        schema.insert(
            kala_json::JsonText::new("title"),
            CanonicalValue::text("kala.plan"),
        );
        schema.insert(
            kala_json::JsonText::new("type"),
            CanonicalValue::text("object"),
        );
        CanonicalValue::Object(schema)
    }

    /// Turn the answer of the provider into the plan steps.
    ///
    /// The answer must carry a structured object with a nonempty `steps`
    /// array. Every entry must be a JSON object with a string
    /// `catalog_step_id`, a string `intent` and a JSON object `parameters`.
    pub fn plan_steps(&self, response: &ModelResponse) -> Result<Vec<PlanStep>, AgentError> {
        let Some(structured) = response.structured() else {
            return Err(AgentError::Model(ModelError::structured(
                "the planner response carried no structured output",
            )));
        };
        let CanonicalValue::Object(entries) = structured else {
            return Err(AgentError::Model(ModelError::structured(
                "the planner response carried no structured output",
            )));
        };
        let Some(CanonicalValue::Array(payload)) = entries.get(&kala_json::JsonText::new("steps"))
        else {
            return Err(AgentError::Model(ModelError::structured(
                "the planner response contained no steps",
            )));
        };
        if payload.is_empty() {
            return Err(AgentError::Model(ModelError::structured(
                "the planner response contained no steps",
            )));
        }
        let mut steps = Vec::with_capacity(payload.len());
        for (index, entry) in payload.iter().enumerate() {
            let CanonicalValue::Object(entry) = entry else {
                return Err(AgentError::Model(ModelError::structured(
                    "a planned step is not a JSON object",
                )));
            };
            let catalog_step_id = entry
                .get(&kala_json::JsonText::new("catalog_step_id"))
                .and_then(|value| value.as_text())
                .map(|value| value.to_string_lossy());
            let intent = entry
                .get(&kala_json::JsonText::new("intent"))
                .and_then(|value| value.as_text())
                .map(|value| value.to_string_lossy());
            let (Some(catalog_step_id), Some(intent)) = (catalog_step_id, intent) else {
                return Err(AgentError::Model(ModelError::structured(
                    "a planned step is missing its identity",
                )));
            };
            let Some(CanonicalValue::Object(parameters)) =
                entry.get(&kala_json::JsonText::new("parameters"))
            else {
                return Err(AgentError::Model(ModelError::structured(
                    "planned step parameters must be a JSON object",
                )));
            };
            let bounded = truncate_code_points(&intent, MAX_BOUNDED_TEXT);
            let intent = if bounded.is_empty() {
                "planned step".to_owned()
            } else {
                bounded
            };
            let parameters_json = canonical_json_dumps(
                &CanonicalValue::Object(parameters.clone()),
                kala_tools::protocol::MAX_JSON_BYTES,
            )
            .map_err(AgentError::Protocol)?;
            let value = CanonicalValue::object([
                (
                    "step_id",
                    CanonicalValue::text(format!("step-{}", index + 1)),
                ),
                ("catalog_step_id", CanonicalValue::text(catalog_step_id)),
                ("intent", CanonicalValue::text(intent)),
                ("parameters_json", CanonicalValue::text(parameters_json)),
            ]);
            steps.push(
                PlanStep::from_value(&value)
                    .map_err(|error| AgentError::Validation(error.message().to_owned()))?,
            );
        }
        Ok(steps)
    }
}

#[async_trait::async_trait]
impl Planner for ModelPlanner {
    async fn plan(&self, objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        if python_strip(objective).is_empty() {
            return Err(AgentError::plan(
                "the planning objective must be a nonempty string",
            ));
        }
        let prompt = bounded_text(objective, "objective", MAX_BOUNDED_TEXT)?;
        let request_id = format!("plan-{}", &sha256_hex_text(&prompt)[..16]);
        let request = ModelRequest::from_parts(
            &request_id,
            vec![
                ModelMessage::new(ModelRole::System, "Return a bounded plan as a JSON object.")
                    .map_err(AgentError::Model)?,
                ModelMessage::new(ModelRole::User, &prompt).map_err(AgentError::Model)?,
            ],
            Vec::new(),
            Some(&self.response_schema()),
            Vec::new(),
            None,
            self.max_tokens,
            60.0,
        )
        .map_err(AgentError::Model)?;
        let response = self
            .gateway
            .complete(&request)
            .await
            .map_err(map_gateway_error)?;
        let steps = self.plan_steps(&response)?;
        self.catalog.to_tasks(&steps)
    }
}

/// The planner that degrades to a deterministic fallback.
///
/// The degradation is only taken when it was approved in advance. An
/// authoritative refusal — a policy refusal, a refused egress, a missing
/// model capability and an exhausted budget — is never degraded, because the
/// refusal is the answer, not a provider failure.
#[derive(Clone)]
pub struct DegradingPlanner {
    primary: Arc<dyn Planner>,
    fallback: Arc<dyn Planner>,
    allow_degraded: bool,
    degradations: Arc<Mutex<Vec<String>>>,
}

impl DegradingPlanner {
    /// Build a degrading planner.
    pub fn new(
        primary: Arc<dyn Planner>,
        fallback: Arc<dyn Planner>,
        allow_degraded: bool,
    ) -> Self {
        Self {
            primary,
            fallback,
            allow_degraded,
            degradations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Whether the degradation is approved.
    #[must_use]
    pub fn allow_degraded(&self) -> bool {
        self.allow_degraded
    }

    /// The degradation reasons the planner recorded.
    #[must_use]
    pub fn degradations(&self) -> Vec<String> {
        self.degradations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The reason a failure degrades to the deterministic planner.
    ///
    /// The failure is either a provider degradation, a planner failure, a
    /// planner rejection or a failure that is not a degradation at all.
    fn degradation_reason(error: &AgentError) -> Option<String> {
        if error.is_never_degraded() {
            return None;
        }
        match error {
            AgentError::Model(ModelError::Degraded {
                provider_id,
                model_id,
                ..
            }) => Some(format!("provider degraded: {provider_id}/{model_id}")),
            AgentError::PlannerFailure(_) => {
                Some(format!("planner failure: {}", error.python_class_name()))
            }
            AgentError::Model(ModelError::Structured(_)) => {
                Some(format!("planner failure: {}", error.python_class_name()))
            }
            AgentError::Base(_) | AgentError::Plan(_) | AgentError::Merge(_) => {
                Some(format!("planner rejection: {}", error.python_class_name()))
            }
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl Planner for DegradingPlanner {
    async fn plan(&self, objective: &str) -> Result<Vec<AgentTask>, AgentError> {
        let reason = match self.primary.plan(objective).await {
            Ok(tasks) => return Ok(tasks),
            Err(error) => match Self::degradation_reason(&error) {
                Some(reason) => reason,
                None => return Err(error),
            },
        };
        if !self.allow_degraded {
            return Err(AgentError::Model(ModelError::degraded(
                "offline",
                "offline",
                format!("{reason}; deterministic planning was not approved"),
            )));
        }
        self.degradations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(truncate_code_points(&reason, MAX_ERROR_TEXT));
        self.fallback.plan(objective).await
    }

    fn degradations(&self) -> Vec<String> {
        DegradingPlanner::degradations(self)
    }
}

/// Map a gateway failure onto the agent error hierarchy.
///
/// The gateway raises the model, the budget and the policy families; the
/// agent keeps the same three families, so the error precedence of the
/// degrading planner is unchanged.
pub(crate) fn map_gateway_error(error: GatewayError) -> AgentError {
    match error {
        GatewayError::Model(error) => AgentError::Model(error),
        GatewayError::Budget(error) => AgentError::Budget(error),
        GatewayError::Policy(error) => AgentError::PolicyError(error),
    }
}
