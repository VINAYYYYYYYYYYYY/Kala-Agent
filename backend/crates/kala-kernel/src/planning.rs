//! Native plan normalization and validation.
//!
//! Mirrors `kala_agent.kernel.planning`: the same validation codes, error
//! precedence, detail payloads, capability intersections, and deterministic
//! depth-first cycle discovery. The dependency walk is iterative so a plan
//! accepted by the Python validator (whose recursion limit is far higher than
//! the state depth) cannot overflow the Rust thread stack.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;

use kala_contracts::{ContractModel, PlanStep, Text, WorkflowDefinition};
use kala_json::CanonicalValue;
use num_bigint::BigInt;

use crate::json_input::parse_model_json;
use crate::value::{canonical_state_json, display_text, value_text};

/// A Python `frozenset[str]` of capability names.
pub type CapabilitySet = BTreeSet<Text>;

/// The Python `PlanValidationCode` enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PlanValidationCode {
    /// `invalid_definition`.
    InvalidDefinition,
    /// `empty_plan`.
    EmptyPlan,
    /// `too_many_steps`.
    TooManySteps,
    /// `invalid_step`.
    InvalidStep,
    /// `duplicate_step_id`.
    DuplicateStepId,
    /// `unknown_catalog_step`.
    UnknownCatalogStep,
    /// `unknown_dependency`.
    UnknownDependency,
    /// `self_dependency`.
    SelfDependency,
    /// `dependency_cycle`.
    DependencyCycle,
    /// `capability_not_granted`.
    CapabilityNotGranted,
    /// `invalid_attempts`.
    InvalidAttempts,
    /// `too_many_attempts`.
    TooManyAttempts,
    /// `replanning_limit`.
    ReplanningLimit,
}

impl PlanValidationCode {
    /// Every accepted value in Python declaration order.
    pub const ALL: &'static [Self] = &[
        Self::InvalidDefinition,
        Self::EmptyPlan,
        Self::TooManySteps,
        Self::InvalidStep,
        Self::DuplicateStepId,
        Self::UnknownCatalogStep,
        Self::UnknownDependency,
        Self::SelfDependency,
        Self::DependencyCycle,
        Self::CapabilityNotGranted,
        Self::InvalidAttempts,
        Self::TooManyAttempts,
        Self::ReplanningLimit,
    ];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidDefinition => "invalid_definition",
            Self::EmptyPlan => "empty_plan",
            Self::TooManySteps => "too_many_steps",
            Self::InvalidStep => "invalid_step",
            Self::DuplicateStepId => "duplicate_step_id",
            Self::UnknownCatalogStep => "unknown_catalog_step",
            Self::UnknownDependency => "unknown_dependency",
            Self::SelfDependency => "self_dependency",
            Self::DependencyCycle => "dependency_cycle",
            Self::CapabilityNotGranted => "capability_not_granted",
            Self::InvalidAttempts => "invalid_attempts",
            Self::TooManyAttempts => "too_many_attempts",
            Self::ReplanningLimit => "replanning_limit",
        }
    }

    /// Parse an exact value.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|item| item.as_str() == value)
    }
}

impl fmt::Display for PlanValidationCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A structured plan validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanValidationError {
    code: PlanValidationCode,
    message: String,
    step_id: Option<Text>,
    details: Vec<(String, String)>,
}

impl PlanValidationError {
    /// Build a validation failure.
    pub fn new(
        code: PlanValidationCode,
        message: impl Into<String>,
        step_id: Option<Text>,
        details: Vec<(String, String)>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            step_id,
            details,
        }
    }

    /// The validation code.
    pub fn code(&self) -> PlanValidationCode {
        self.code
    }
    /// Python's `reason` property.
    pub fn reason(&self) -> PlanValidationCode {
        self.code
    }
    /// The message.
    pub fn message(&self) -> &str {
        &self.message
    }
    /// Python's `field` property.
    pub fn field(&self) -> Option<&Text> {
        self.step_id.as_ref()
    }
    /// The step identifier, when the failure is step-specific.
    pub fn step_id(&self) -> Option<&Text> {
        self.step_id.as_ref()
    }
    /// Ordered detail pairs.
    pub fn details(&self) -> &[(String, String)] {
        &self.details
    }

    /// The Python `as_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        let mut details = BTreeMap::new();
        for (key, value) in &self.details {
            details.insert(
                kala_json::JsonText::new(key),
                CanonicalValue::Text(kala_json::JsonText::new(value)),
            );
        }
        CanonicalValue::object([
            (
                "code",
                CanonicalValue::Text(kala_json::JsonText::new(self.code.as_str())),
            ),
            ("details", CanonicalValue::Object(details)),
            (
                "message",
                CanonicalValue::Text(kala_json::JsonText::new(&self.message)),
            ),
            (
                "step_id",
                self.step_id
                    .as_ref()
                    .map_or(CanonicalValue::Null, |step_id| {
                        CanonicalValue::Text(step_id.as_json_text().clone())
                    }),
            ),
        ])
    }

    /// Canonical ASCII JSON.
    pub fn to_json(&self) -> String {
        canonical_state_json(&self.as_value()).expect("validation payload is canonicalizable")
    }
}

impl fmt::Display for PlanValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.step_id {
            None => formatter.write_str(&self.message),
            Some(step_id) => write!(formatter, "{} ({})", self.message, display_text(step_id)),
        }
    }
}

impl std::error::Error for PlanValidationError {}

/// The Python `PlanRejectedError`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanRejectedError {
    error: PlanValidationError,
}

impl PlanRejectedError {
    /// Wrap a validation failure.
    pub fn new(error: PlanValidationError) -> Self {
        Self { error }
    }
    /// The wrapped failure.
    pub fn error(&self) -> &PlanValidationError {
        &self.error
    }
    /// Consume the wrapper.
    pub fn into_error(self) -> PlanValidationError {
        self.error
    }
}

impl fmt::Display for PlanRejectedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}

impl std::error::Error for PlanRejectedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// A validated, normalized plan.
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedPlan {
    workflow_definition_id: Text,
    steps: Vec<PlanStep>,
    required_capabilities: Vec<Text>,
    max_attempts: BigInt,
}

impl NormalizedPlan {
    /// Build a normalized plan and validate its invariants.
    pub fn new(
        workflow_definition_id: Text,
        steps: Vec<PlanStep>,
        required_capabilities: Vec<Text>,
        max_attempts: BigInt,
    ) -> Result<Self, PlanValidationError> {
        if workflow_definition_id.python_strip().is_empty() {
            return Err(PlanValidationError::new(
                PlanValidationCode::InvalidDefinition,
                "workflow_definition_id must be nonempty",
                None,
                Vec::new(),
            ));
        }
        if steps.is_empty() {
            return Err(PlanValidationError::new(
                PlanValidationCode::EmptyPlan,
                "a normalized plan cannot be empty",
                None,
                Vec::new(),
            ));
        }
        if max_attempts < BigInt::from(1) {
            return Err(PlanValidationError::new(
                PlanValidationCode::InvalidAttempts,
                "max_attempts must be positive",
                None,
                Vec::new(),
            ));
        }
        Ok(Self {
            workflow_definition_id,
            steps,
            required_capabilities,
            max_attempts,
        })
    }

    /// The definition identifier.
    pub fn workflow_definition_id(&self) -> &Text {
        &self.workflow_definition_id
    }
    /// The normalized steps.
    pub fn steps(&self) -> &[PlanStep] {
        &self.steps
    }
    /// Python's `step_ids` property.
    pub fn step_ids(&self) -> Vec<Text> {
        self.steps
            .iter()
            .map(|step| Text::new(step.step_id()))
            .collect()
    }
    /// The sorted union of required capabilities.
    pub fn required_capabilities(&self) -> &[Text] {
        &self.required_capabilities
    }
    /// Python's `capabilities` property.
    pub fn capabilities(&self) -> &[Text] {
        &self.required_capabilities
    }
    /// The configured attempt bound.
    pub fn max_attempts(&self) -> &BigInt {
        &self.max_attempts
    }
    /// Number of steps.
    pub fn len(&self) -> usize {
        self.steps.len()
    }
    /// Whether the plan has no steps (never true for a valid plan).
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
    /// Step by index.
    pub fn get(&self, index: usize) -> Option<&PlanStep> {
        self.steps.get(index)
    }
    /// Iterate steps.
    pub fn iter(&self) -> std::slice::Iter<'_, PlanStep> {
        self.steps.iter()
    }
}

impl<'a> IntoIterator for &'a NormalizedPlan {
    type Item = &'a PlanStep;
    type IntoIter = std::slice::Iter<'a, PlanStep>;

    fn into_iter(self) -> Self::IntoIter {
        self.steps.iter()
    }
}

/// Python aliases.
pub type ValidatedPlan = NormalizedPlan;
/// Python alias.
pub type Plan = NormalizedPlan;

/// A Python `TypeError` or `ValueError` from validator configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanConfigError {
    /// Python `TypeError`.
    Type(String),
    /// Python `ValueError`.
    Value(String),
}

impl PlanConfigError {
    /// The Python exception class this variant corresponds to.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Type(_) => "TypeError",
            Self::Value(_) => "ValueError",
        }
    }
    /// The exception message.
    pub fn message(&self) -> &str {
        match self {
            Self::Type(message) | Self::Value(message) => message,
        }
    }
}

impl fmt::Display for PlanConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PlanConfigError {}

/// A plan validator bound to a workflow definition and capability grant.
#[derive(Clone, Debug)]
pub struct PlanValidator {
    definition: WorkflowDefinition,
    max_steps: BigInt,
    max_attempts: BigInt,
    max_total_attempts: Option<BigInt>,
    allowed_capabilities: Option<CapabilitySet>,
}

impl PlanValidator {
    /// Create an unrestricted validator with Python defaults (256 steps, one
    /// attempt, no aggregate attempt bound).
    pub fn new(definition: WorkflowDefinition) -> Self {
        Self {
            definition,
            max_steps: BigInt::from(256),
            max_attempts: BigInt::from(1),
            max_total_attempts: None,
            allowed_capabilities: None,
        }
    }

    /// Create a validator with explicit bounds and capability inputs.
    pub fn from_config(
        definition: WorkflowDefinition,
        allowed_capabilities: Option<CapabilitySet>,
        granted_capabilities: Option<CapabilitySet>,
        max_steps: BigInt,
        max_attempts: BigInt,
        max_total_attempts: Option<BigInt>,
    ) -> Result<Self, PlanConfigError> {
        if max_steps < BigInt::from(1) {
            return Err(PlanConfigError::Value(
                "max_steps must be positive".to_owned(),
            ));
        }
        if max_attempts < BigInt::from(1) {
            return Err(PlanConfigError::Value(
                "max_attempts must be positive".to_owned(),
            ));
        }
        if let Some(max_total_attempts) = &max_total_attempts {
            if max_total_attempts < &BigInt::from(1) {
                return Err(PlanConfigError::Value(
                    "max_total_attempts must be positive".to_owned(),
                ));
            }
        }
        Ok(Self {
            definition,
            max_steps,
            max_attempts,
            max_total_attempts,
            allowed_capabilities: combine_capability_sets(
                allowed_capabilities,
                granted_capabilities,
            ),
        })
    }

    /// The workflow definition.
    pub fn definition(&self) -> &WorkflowDefinition {
        &self.definition
    }
    /// The effective allowed capability set, or `None` when unrestricted.
    pub fn allowed_capabilities(&self) -> Option<&CapabilitySet> {
        self.allowed_capabilities.as_ref()
    }
    /// Python alias.
    pub fn granted_capabilities(&self) -> Option<&CapabilitySet> {
        self.allowed_capabilities.as_ref()
    }
    /// Whether no capability restriction is active.
    pub fn unrestricted(&self) -> bool {
        self.allowed_capabilities.is_none()
    }
    /// The configured maximum step count.
    pub fn max_steps(&self) -> &BigInt {
        &self.max_steps
    }
    /// The configured maximum per-step attempts.
    pub fn max_attempts(&self) -> &BigInt {
        &self.max_attempts
    }
    /// The configured aggregate attempt bound.
    pub fn max_total_attempts(&self) -> Option<&BigInt> {
        self.max_total_attempts.as_ref()
    }

    /// Validate typed steps. Capability sets are already normalized.
    pub fn validate(
        &self,
        steps: &[PlanStep],
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &BigInt,
    ) -> Result<NormalizedPlan, PlanValidationError> {
        if steps.is_empty() {
            return Err(PlanValidationError::new(
                PlanValidationCode::EmptyPlan,
                "a plan must contain at least one step",
                None,
                Vec::new(),
            ));
        }
        if BigInt::from(steps.len()) > self.max_steps {
            return Err(PlanValidationError::new(
                PlanValidationCode::TooManySteps,
                format!(
                    "plan contains {} steps; maximum is {}",
                    steps.len(),
                    self.max_steps
                ),
                None,
                vec![
                    ("count".to_owned(), steps.len().to_string()),
                    ("maximum".to_owned(), self.max_steps.to_string()),
                ],
            ));
        }
        if replanning_attempts < &BigInt::from(0) {
            return Err(PlanValidationError::new(
                PlanValidationCode::ReplanningLimit,
                "replanning_attempts must be nonnegative",
                None,
                Vec::new(),
            ));
        }
        if replanning_attempts > self.definition.max_replanning_attempts().as_bigint() {
            return Err(PlanValidationError::new(
                PlanValidationCode::ReplanningLimit,
                "replanning attempts exceed the workflow definition limit",
                None,
                Vec::new(),
            ));
        }
        let selected_capabilities =
            self.selected_capabilities(allowed_capabilities, granted_capabilities);
        self.validate_normalized(steps, selected_capabilities.as_ref())
    }

    /// Validate JSON step values with Python's raw self-dependency precedence.
    ///
    /// This is the dynamic boundary corresponding to `validate(list)` in
    /// Python, including the `INVALID_STEP` payload for non-`PlanStep`
    /// entries. `steps_json` must be a JSON array value.
    pub fn validate_values(
        &self,
        steps: &CanonicalValue,
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &BigInt,
    ) -> Result<NormalizedPlan, PlanValidationError> {
        let CanonicalValue::Array(items) = steps else {
            return Err(PlanValidationError::new(
                PlanValidationCode::InvalidStep,
                "steps must be a sequence of PlanStep objects",
                None,
                Vec::new(),
            ));
        };
        if items.is_empty() {
            return Err(PlanValidationError::new(
                PlanValidationCode::EmptyPlan,
                "a plan must contain at least one step",
                None,
                Vec::new(),
            ));
        }
        if BigInt::from(items.len()) > self.max_steps {
            return Err(PlanValidationError::new(
                PlanValidationCode::TooManySteps,
                format!(
                    "plan contains {} steps; maximum is {}",
                    items.len(),
                    self.max_steps
                ),
                None,
                vec![
                    ("count".to_owned(), items.len().to_string()),
                    ("maximum".to_owned(), self.max_steps.to_string()),
                ],
            ));
        }
        if replanning_attempts < &BigInt::from(0) {
            return Err(PlanValidationError::new(
                PlanValidationCode::ReplanningLimit,
                "replanning_attempts must be nonnegative",
                None,
                Vec::new(),
            ));
        }
        if replanning_attempts > self.definition.max_replanning_attempts().as_bigint() {
            return Err(PlanValidationError::new(
                PlanValidationCode::ReplanningLimit,
                "replanning attempts exceed the workflow definition limit",
                None,
                Vec::new(),
            ));
        }
        let selected_capabilities =
            self.selected_capabilities(allowed_capabilities, granted_capabilities);
        let mut normalized_steps = Vec::with_capacity(items.len());
        for item in items {
            let CanonicalValue::Object(payload) = item else {
                return Err(PlanValidationError::new(
                    PlanValidationCode::InvalidStep,
                    "every plan entry must be a PlanStep",
                    None,
                    Vec::new(),
                ));
            };
            let raw_step_id = payload
                .get(&kala_json::JsonText::new("step_id"))
                .and_then(value_text);
            let raw_dependencies: Vec<Text> = payload
                .get(&kala_json::JsonText::new("depends_on"))
                .and_then(|value| match value {
                    CanonicalValue::Array(values) => {
                        Some(values.iter().filter_map(value_text).collect::<Vec<Text>>())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            if let Some(step_id) = &raw_step_id {
                if raw_dependencies.contains(step_id) {
                    return Err(PlanValidationError::new(
                        PlanValidationCode::SelfDependency,
                        "a plan step cannot depend on itself",
                        Some(step_id.clone()),
                        Vec::new(),
                    ));
                }
            }
            let step = PlanStep::from_value(item).map_err(|_| {
                PlanValidationError::new(
                    PlanValidationCode::InvalidStep,
                    "plan step failed contract validation",
                    raw_step_id.clone(),
                    vec![("reason".to_owned(), "ValidationError".to_owned())],
                )
            })?;
            normalized_steps.push(step);
        }
        self.validate_normalized(&normalized_steps, selected_capabilities.as_ref())
    }

    /// Validate dynamic steps with a dynamic replanning counter.
    ///
    /// Python returns `REPLANNING_LIMIT` for any non-integer counter, including
    /// booleans, strings and null. This entry point enforces that request
    /// boundary before delegating to [`Self::validate_values`].
    pub fn validate_values_request(
        &self,
        steps: &CanonicalValue,
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &CanonicalValue,
    ) -> Result<NormalizedPlan, PlanValidationError> {
        let Some(attempts) = replanning_attempts.as_integer() else {
            return Err(PlanValidationError::new(
                PlanValidationCode::ReplanningLimit,
                "replanning_attempts must be an integer",
                None,
                Vec::new(),
            ));
        };
        self.validate_values(steps, allowed_capabilities, granted_capabilities, attempts)
    }

    /// Validate JSON text.
    pub fn validate_json(
        &self,
        steps_json: &str,
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &BigInt,
    ) -> Result<NormalizedPlan, PlanValidationError> {
        let value = parse_model_json(steps_json).map_err(|_| {
            PlanValidationError::new(
                PlanValidationCode::InvalidStep,
                "steps must be a sequence of PlanStep objects",
                None,
                Vec::new(),
            )
        })?;
        self.validate_values(
            &value,
            allowed_capabilities,
            granted_capabilities,
            replanning_attempts,
        )
    }

    fn validate_normalized(
        &self,
        steps: &[PlanStep],
        selected_capabilities: Option<&CapabilitySet>,
    ) -> Result<NormalizedPlan, PlanValidationError> {
        let mut normalized_steps = Vec::with_capacity(steps.len());
        let mut step_ids: HashSet<Text> = HashSet::new();
        for step in steps {
            let step_id = Text::new(step.step_id());
            if step_ids.contains(&step_id) {
                return Err(PlanValidationError::new(
                    PlanValidationCode::DuplicateStepId,
                    "plan step identifiers must be unique",
                    Some(step_id),
                    Vec::new(),
                ));
            }
            let catalog_step_id = Text::new(step.catalog_step_id());
            if !self
                .definition
                .available_step_ids()
                .iter()
                .any(|available| available.as_str() == step.catalog_step_id())
            {
                return Err(PlanValidationError::new(
                    PlanValidationCode::UnknownCatalogStep,
                    "catalog step is not declared by the workflow",
                    Some(step_id),
                    vec![("catalog_step_id".to_owned(), display_text(&catalog_step_id))],
                ));
            }
            if step
                .depends_on()
                .iter()
                .any(|dependency| dependency.as_str() == step.step_id())
            {
                return Err(PlanValidationError::new(
                    PlanValidationCode::SelfDependency,
                    "a plan step cannot depend on itself",
                    Some(step_id),
                    Vec::new(),
                ));
            }
            let attempts = step.max_attempts().as_bigint();
            if attempts < &BigInt::from(1) {
                return Err(PlanValidationError::new(
                    PlanValidationCode::InvalidAttempts,
                    "max_attempts must be positive",
                    Some(step_id),
                    Vec::new(),
                ));
            }
            if attempts > &self.max_attempts {
                return Err(PlanValidationError::new(
                    PlanValidationCode::TooManyAttempts,
                    "step attempts exceed the configured bound",
                    Some(step_id),
                    vec![
                        ("requested".to_owned(), attempts.to_string()),
                        ("maximum".to_owned(), self.max_attempts.to_string()),
                    ],
                ));
            }
            step_ids.insert(step_id);
            normalized_steps.push(step.clone());
        }

        for step in &normalized_steps {
            let dependencies: BTreeSet<Text> = step
                .depends_on()
                .iter()
                .map(|value| Text::new(value.as_str()))
                .collect();
            let unknown: Vec<Text> = dependencies
                .into_iter()
                .filter(|dependency| !step_ids.contains(dependency))
                .collect();
            if !unknown.is_empty() {
                return Err(PlanValidationError::new(
                    PlanValidationCode::UnknownDependency,
                    "plan dependency is not a declared step",
                    Some(Text::new(step.step_id())),
                    unknown
                        .into_iter()
                        .map(|dependency| ("dependency".to_owned(), display_text(&dependency)))
                        .collect(),
                ));
            }
        }

        if let Some(cycle) = find_cycle(&normalized_steps) {
            // Python joins the closed cycle and then appends `cycle[0]`
            // again, so `a -> b -> a` is reported as `a -> b -> a -> a`.
            let mut rendered = cycle.clone();
            rendered.push(cycle[0].clone());
            let path = rendered
                .iter()
                .map(display_text)
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(PlanValidationError::new(
                PlanValidationCode::DependencyCycle,
                "plan dependencies contain a cycle",
                Some(cycle[0].clone()),
                vec![("cycle".to_owned(), path)],
            ));
        }

        let required_capabilities: Vec<Text> = {
            let mut values = BTreeSet::new();
            for step in &normalized_steps {
                for capability in step.required_capabilities() {
                    values.insert(Text::new(capability.as_str()));
                }
            }
            values.into_iter().collect()
        };
        let missing_capabilities: Vec<Text> = match selected_capabilities {
            None => Vec::new(),
            Some(selected) => required_capabilities
                .iter()
                .filter(|capability| !selected.contains(*capability))
                .cloned()
                .collect(),
        };
        if !missing_capabilities.is_empty() {
            return Err(PlanValidationError::new(
                PlanValidationCode::CapabilityNotGranted,
                "plan requests capabilities outside the active grant",
                None,
                missing_capabilities
                    .iter()
                    .map(|capability| ("capability".to_owned(), display_text(capability)))
                    .collect(),
            ));
        }

        let total_attempts: BigInt = normalized_steps
            .iter()
            .map(|step| step.max_attempts().as_bigint().clone())
            .fold(BigInt::from(0), |total, value| total + value);
        if let Some(max_total_attempts) = &self.max_total_attempts {
            if &total_attempts > max_total_attempts {
                return Err(PlanValidationError::new(
                    PlanValidationCode::TooManyAttempts,
                    "plan attempts exceed the aggregate bound",
                    None,
                    vec![
                        ("requested".to_owned(), total_attempts.to_string()),
                        ("maximum".to_owned(), max_total_attempts.to_string()),
                    ],
                ));
            }
        }

        NormalizedPlan::new(
            Text::new(self.definition.definition_id()),
            normalized_steps,
            required_capabilities,
            self.max_attempts.clone(),
        )
    }

    fn selected_capabilities(
        &self,
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
    ) -> Option<CapabilitySet> {
        let supplied =
            combine_capability_sets(allowed_capabilities.cloned(), granted_capabilities.cloned());
        match (supplied, &self.allowed_capabilities) {
            (None, None) => None,
            (None, Some(current)) => Some(current.clone()),
            (Some(supplied), None) => Some(supplied),
            (Some(supplied), Some(current)) => {
                Some(supplied.intersection(current).cloned().collect())
            }
        }
    }

    /// Validate or wrap the failure in [`PlanRejectedError`].
    pub fn validate_or_raise(
        &self,
        steps: &[PlanStep],
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &BigInt,
    ) -> Result<NormalizedPlan, PlanRejectedError> {
        self.validate(
            steps,
            allowed_capabilities,
            granted_capabilities,
            replanning_attempts,
        )
        .map_err(PlanRejectedError::new)
    }

    /// Whether the plan validates.
    pub fn is_valid(
        &self,
        steps: &[PlanStep],
        allowed_capabilities: Option<&CapabilitySet>,
        granted_capabilities: Option<&CapabilitySet>,
        replanning_attempts: &BigInt,
    ) -> bool {
        self.validate(
            steps,
            allowed_capabilities,
            granted_capabilities,
            replanning_attempts,
        )
        .is_ok()
    }
}

/// Build a capability set with Python's strip/nonempty rules.
pub fn capability_set<I>(values: I) -> Result<CapabilitySet, PlanConfigError>
where
    I: IntoIterator<Item = Text>,
{
    let mut result = BTreeSet::new();
    for value in values {
        let normalized = value.python_strip();
        if normalized.is_empty() {
            return Err(PlanConfigError::Value(
                "capabilities cannot be empty".to_owned(),
            ));
        }
        result.insert(normalized);
    }
    Ok(result)
}

/// Intersect all supplied capability sets; `None` when none are supplied.
pub fn combine_capability_sets(
    first: Option<CapabilitySet>,
    second: Option<CapabilitySet>,
) -> Option<CapabilitySet> {
    match (first, second) {
        (None, None) => None,
        (Some(values), None) | (None, Some(values)) => Some(values),
        (Some(first), Some(second)) => Some(first.intersection(&second).cloned().collect()),
    }
}

/// Deterministic depth-first cycle discovery, returning the closed path with
/// the repeated start node (for example `a -> b -> a`).
pub fn find_cycle(steps: &[PlanStep]) -> Option<Vec<Text>> {
    let graph: BTreeMap<Text, Vec<Text>> = steps
        .iter()
        .map(|step| {
            (
                Text::new(step.step_id()),
                step.depends_on()
                    .iter()
                    .map(|dependency| Text::new(dependency.as_str()))
                    .collect(),
            )
        })
        .collect();
    let mut visiting: Vec<Text> = Vec::new();
    let mut positions: HashMap<Text, usize> = HashMap::new();
    let mut visited: HashSet<Text> = HashSet::new();
    for step in steps {
        let start = Text::new(step.step_id());
        if visited.contains(&start) {
            continue;
        }
        // Each frame is (node, next dependency index).
        let mut stack: Vec<(Text, usize)> = vec![(start.clone(), 0)];
        visiting.push(start.clone());
        positions.insert(start.clone(), 0);
        while let Some((node, index)) = stack.last().cloned() {
            let dependencies = graph.get(&node);
            let next = dependencies.and_then(|values| values.get(index)).cloned();
            if let Some(dependency) = next {
                if let Some(last) = stack.last_mut() {
                    last.1 += 1;
                }
                if let Some(position) = positions.get(&dependency).copied() {
                    let mut cycle = visiting[position..].to_vec();
                    cycle.push(dependency);
                    return Some(cycle);
                }
                if visited.contains(&dependency) {
                    continue;
                }
                positions.insert(dependency.clone(), visiting.len());
                visiting.push(dependency.clone());
                stack.push((dependency, 0));
            } else {
                stack.pop();
                if let Some(removed) = visiting.pop() {
                    positions.remove(&removed);
                    visited.insert(removed);
                }
            }
        }
    }
    None
}

/// A deterministic dependency-first topological order.
///
/// Native convenience: the Python module only discovers cycles, but consumers
/// composing plans with the runtime need an ordering. Returns `None` when a
/// cycle exists, using the same graph and dependency order as [`find_cycle`].
pub fn topological_order(steps: &[PlanStep]) -> Option<Vec<Text>> {
    if find_cycle(steps).is_some() {
        return None;
    }
    let graph: BTreeMap<Text, Vec<Text>> = steps
        .iter()
        .map(|step| {
            (
                Text::new(step.step_id()),
                step.depends_on()
                    .iter()
                    .map(|dependency| Text::new(dependency.as_str()))
                    .collect(),
            )
        })
        .collect();
    let mut visited: HashSet<Text> = HashSet::new();
    let mut order: Vec<Text> = Vec::with_capacity(steps.len());
    for step in steps {
        let start = Text::new(step.step_id());
        if visited.contains(&start) {
            continue;
        }
        // Explicit post-order stack: `false` expands dependencies, `true`
        // appends the node. A 100k-step chain therefore cannot overflow the
        // thread stack.
        let mut stack: Vec<(Text, bool)> = vec![(start, false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                if visited.insert(node.clone()) {
                    order.push(node);
                }
                continue;
            }
            if visited.contains(&node) {
                continue;
            }
            stack.push((node.clone(), true));
            if let Some(dependencies) = graph.get(&node) {
                for dependency in dependencies.iter().rev() {
                    if !visited.contains(dependency) {
                        stack.push((dependency.clone(), false));
                    }
                }
            }
        }
    }
    Some(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition::from_json_str(
            r#"{
                "definition_id":"workflow-1",
                "version":"1.0.0",
                "name":"Test workflow",
                "initial_state":"start",
                "states":["start","done"],
                "transitions":[
                    {"transition_id":"transition-1","from_state":"start","to_state":"done","step_id":"step-1"}
                ],
                "available_step_ids":["step-1","step-2"],
                "max_replanning_attempts":1
            }"#,
        )
        .expect("definition")
    }

    fn step(id: &str, catalog: &str, depends_on: &[&str]) -> PlanStep {
        let depends = depends_on
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(",");
        PlanStep::from_json_str(&format!(
            r#"{{"step_id":"{id}","catalog_step_id":"{catalog}","intent":"Do it.","depends_on":[{depends}]}}"#
        ))
        .expect("step")
    }

    #[test]
    fn empty_plan_has_the_source_code() {
        let validator = PlanValidator::new(definition());
        let error = validator
            .validate(&[], None, None, &BigInt::from(0))
            .expect_err("empty");
        assert_eq!(error.code(), PlanValidationCode::EmptyPlan);
    }

    #[test]
    fn cycle_path_matches_depth_first_discovery() {
        let steps = vec![step("a", "step-1", &["b"]), step("b", "step-2", &["a"])];
        let cycle = find_cycle(&steps).expect("cycle");
        assert_eq!(
            cycle.iter().map(display_text).collect::<Vec<_>>(),
            ["a", "b", "a"]
        );
        let order = topological_order(&steps);
        assert!(order.is_none());
    }

    #[test]
    fn capability_intersection_narrows() {
        let validator = PlanValidator::from_config(
            definition(),
            Some(capability_set([Text::new("read"), Text::new("write")]).expect("set")),
            Some(capability_set([Text::new("read")]).expect("set")),
            BigInt::from(256),
            BigInt::from(1),
            None,
        )
        .expect("validator");
        assert_eq!(
            validator
                .allowed_capabilities()
                .expect("restricted")
                .iter()
                .map(display_text)
                .collect::<Vec<_>>(),
            ["read"]
        );
    }
}
