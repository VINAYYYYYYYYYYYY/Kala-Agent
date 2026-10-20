//! The application views the API responses carry.
//!
//! The response models of `kala_agent.api.schemas` embed the views the
//! composition root produces: the domain packages, the workflow definitions,
//! the evaluation reports, the workflow state, the checkpoints, the revisions
//! and the mechanical run results. Those views are declared in
//! `kala_agent.application`, and this module is the native form of the ones
//! the API surface reaches.
//!
//! The views are strict: they refuse an undeclared field, they keep the
//! declaration order of their fields, and they never strip the whitespace of a
//! string. Only the request models of `kala_agent.api.schemas` strip.
//!
//! The contract models the views embed — `DesignRevision`, `Provenance`,
//! `EvaluationResult` and `ApprovalRequest` — are the accepted
//! `kala-contracts` types. They are validated by that crate, and their
//! failures are carried here with the kind and the message the Python source
//! reports.

use kala_contracts::{
    ContractModel, DesignRevision, EvaluationResult, Provenance, Text, WireValue,
};
use kala_json::CanonicalValue;
use num_bigint::BigInt;

use crate::datetime::Timestamp;
use crate::validation::{Failures, Fields, Issue, Location, Reading, kind, model_type_issue};

/// An enumeration the views carry, with the values of the Python enum.
macro_rules! view_enum {
    ($name:ident, $model:literal, { $($variant:ident => $value:literal),+ $(,)? }) => {
        /// One accepted value of the enumeration.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $( $variant ),+
        }

        impl $name {
            /// Every accepted value, in Python declaration order.
            pub const ALL: &'static [$name] = &[ $( $name::$variant ),+ ];

            /// The Python string value.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( $name::$variant => $value ),+
                }
            }

            /// The accepted values as the strings the Python enum declares.
            pub const VALUES: &'static [&'static str] = &[ $( $value ),+ ];

            /// Parse an exact Python enum value.
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $( $value => Some($name::$variant), )+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

view_enum!(ApprovalKind, "ApprovalKind", {
    PlanApproval => "plan_approval",
    ToolClassChange => "tool_class_change",
    Egress => "egress",
    PreCommit => "pre_commit",
    Export => "export",
    ProviderDegraded => "provider_degraded",
});

view_enum!(ApprovalStatus, "ApprovalStatus", {
    Pending => "pending",
    Approved => "approved",
    Rejected => "rejected",
    Aborted => "aborted",
    Expired => "expired",
    Invalidated => "invalidated",
});

view_enum!(WorkflowStatus, "WorkflowStatus", {
    Created => "created",
    Running => "running",
    WaitingForApproval => "waiting_for_approval",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
});

view_enum!(PolicyDecisionStatus, "PolicyDecisionStatus", {
    Allowed => "allowed",
    Denied => "denied",
    Pending => "pending",
    Invalidated => "invalidated",
    Rejected => "rejected",
    Aborted => "aborted",
});

/// Read the object of a model, or report the type failure.
fn object_of<'a>(
    value: &'a CanonicalValue,
    model: &str,
    location: &[Location],
    reading: Reading,
) -> Result<&'a kala_json::CanonicalObject, Failures> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        other => Err(vec![match reading {
            Reading::Mapping => model_type_issue(model, location, other),
            Reading::Attributes => crate::validation::body_type_issue(location, other),
        }]),
    }
}

/// The failures of a contract model, in the vocabulary of Pydantic.
///
/// The contract crate reports the field that was refused and the reason; the
/// Python source reports the same reason as the message of a `value_error`,
/// and a field that is not there at all as a `missing`.
fn contract_failures(
    error: &kala_contracts::ContractError,
    location: &[Location],
    value: &CanonicalValue,
) -> Failures {
    error
        .issues()
        .iter()
        .map(|issue| {
            let path = parse_contract_location(&issue.location);
            let mut full = location.to_vec();
            full.extend(path.iter().cloned());
            let input = value_at_path(value, &path)
                .cloned()
                .unwrap_or_else(|| value.clone());
            if issue.message == "Field required" {
                Issue::new(kind::MISSING, full, "Field required", &input)
            } else {
                Issue::with_context(
                    kind::VALUE_ERROR,
                    full,
                    format!("Value error, {}", issue.message),
                    &input,
                    vec![(
                        "error".to_owned(),
                        crate::validation::Context::Text(issue.message.clone()),
                    )],
                )
            }
        })
        .collect()
}

/// The value at a path inside a document.
fn value_at_path<'a>(value: &'a CanonicalValue, path: &[Location]) -> Option<&'a CanonicalValue> {
    let mut current = value;
    for part in path {
        current = match (part, current) {
            (Location::Field(name), CanonicalValue::Object(object)) => {
                object.get(&kala_json::JsonText::new(name))?
            }
            (Location::Index(index), CanonicalValue::Array(items)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Read the dotted location of a contract failure.
fn parse_contract_location(location: &str) -> Vec<Location> {
    let mut parts = Vec::new();
    for part in location.split('.') {
        if part.is_empty() {
            continue;
        }
        if let Some(open) = part.find('[') {
            let name = &part[..open];
            if !name.is_empty() {
                parts.push(Location::Field(name.to_owned()));
            }
            let mut rest = &part[open..];
            while let Some(close) = rest.find(']') {
                let index = &rest[1..close];
                if let Ok(index) = index.parse::<usize>() {
                    parts.push(Location::Index(index));
                }
                rest = &rest[close + 1..];
            }
            continue;
        }
        parts.push(Location::Field(part.to_owned()));
    }
    parts
}

/// Read one contract model.
///
/// The contract crate validates the model; its failures are carried here with
/// the kind and the message the Python source reports.
pub fn contract_model<T: ContractModel>(
    value: &CanonicalValue,
    location: &[Location],
    reading: Reading,
) -> Result<T, Failures> {
    if !matches!(value, CanonicalValue::Object(_)) {
        return Err(vec![match reading {
            Reading::Mapping => model_type_issue(T::NAME, location, value),
            Reading::Attributes => crate::validation::body_type_issue(location, value),
        }]);
    }
    T::from_value(value).map_err(|error| contract_failures(&error, location, value))
}

/// A text value of a wire document.
fn text(value: &str) -> WireValue {
    WireValue::Text(Text::new(value))
}

/// A text value of a wire document, or null.
fn optional_text(value: &Option<String>) -> WireValue {
    match value {
        Some(value) => text(value),
        None => WireValue::Null,
    }
}

/// A sequence of texts of a wire document.
fn texts(values: &[String]) -> WireValue {
    WireValue::Array(values.iter().map(|value| text(value)).collect())
}

/// An integer value of a wire document.
fn integer(value: &BigInt) -> WireValue {
    WireValue::Integer(value.clone())
}

/// A number value of a wire document.
fn number(value: f64) -> WireValue {
    WireValue::Float(value)
}

/// A boolean value of a wire document.
fn boolean(value: bool) -> WireValue {
    WireValue::Bool(value)
}

/// A timestamp value of a wire document.
fn timestamp(value: &Timestamp) -> WireValue {
    text(&value.to_wire_string())
}

/// An optional timestamp value of a wire document.
fn optional_timestamp(value: &Option<Timestamp>) -> WireValue {
    match value {
        Some(value) => timestamp(value),
        None => WireValue::Null,
    }
}

/// An inspectable summary of one registered domain package.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainPackageView {
    /// The domain the package belongs to.
    pub domain: String,
    /// The package identifier.
    pub package_id: String,
    /// The package version.
    pub version: String,
    /// The digest of the package.
    pub digest: String,
    /// The human readable description.
    pub description: String,
    /// The disclaimer of the package.
    pub disclaimer: String,
    /// The workflow the package declares.
    pub workflow_definition_id: String,
    /// The reference subject of the package.
    pub reference_subject_id: String,
    /// The digests of the schemas the package declares.
    pub schema_digests: Vec<String>,
    /// The capabilities the package requires.
    pub required_capabilities: Vec<String>,
    /// The catalog steps the package declares.
    pub catalog_step_ids: Vec<String>,
}

impl DomainPackageView {
    /// The Python model name.
    pub const NAME: &'static str = "DomainPackageView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let domain = fields.required_text("domain");
        let package_id = fields.required_text("package_id");
        let version = fields.required_text("version");
        let digest = fields.required_text("digest");
        let description = fields.required_text("description");
        let disclaimer = fields.required_text("disclaimer");
        let workflow_definition_id = fields.required_text("workflow_definition_id");
        let reference_subject_id = fields.required_text("reference_subject_id");
        let schema_digests = fields.required_text_tuple("schema_digests");
        let required_capabilities = fields.required_text_tuple("required_capabilities");
        let catalog_step_ids = fields.required_text_tuple("catalog_step_ids");
        fields.reject_extra(&[
            "domain",
            "package_id",
            "version",
            "digest",
            "description",
            "disclaimer",
            "workflow_definition_id",
            "reference_subject_id",
            "schema_digests",
            "required_capabilities",
            "catalog_step_ids",
        ]);
        fields.finish()?;
        Ok(Self {
            domain: domain.expect("validated"),
            package_id: package_id.expect("validated"),
            version: version.expect("validated"),
            digest: digest.expect("validated"),
            description: description.expect("validated"),
            disclaimer: disclaimer.expect("validated"),
            workflow_definition_id: workflow_definition_id.expect("validated"),
            reference_subject_id: reference_subject_id.expect("validated"),
            schema_digests: schema_digests.expect("validated"),
            required_capabilities: required_capabilities.expect("validated"),
            catalog_step_ids: catalog_step_ids.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", text(&self.domain)),
            ("package_id", text(&self.package_id)),
            ("version", text(&self.version)),
            ("digest", text(&self.digest)),
            ("description", text(&self.description)),
            ("disclaimer", text(&self.disclaimer)),
            ("workflow_definition_id", text(&self.workflow_definition_id)),
            ("reference_subject_id", text(&self.reference_subject_id)),
            ("schema_digests", texts(&self.schema_digests)),
            ("required_capabilities", texts(&self.required_capabilities)),
            ("catalog_step_ids", texts(&self.catalog_step_ids)),
        ])
    }
}

/// One declared catalog step of a domain package.
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogStepView {
    /// The step identifier.
    pub step_id: String,
    /// The human readable name.
    pub name: String,
    /// The human readable description.
    pub description: String,
    /// The names the step reads.
    pub input_names: Vec<String>,
    /// The names the step produces.
    pub metric_names: Vec<String>,
    /// The capabilities the step requires.
    pub required_capabilities: Vec<String>,
    /// The kind of the checkpoint the step opens, if it opens one.
    pub checkpoint_kind: Option<ApprovalKind>,
    /// The number of attempts the step allows.
    pub max_attempts: BigInt,
}

impl CatalogStepView {
    /// The Python model name.
    pub const NAME: &'static str = "CatalogStepView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let step_id = fields.required_text("step_id");
        let name = fields.required_text("name");
        let description = fields.required_text("description");
        let input_names = fields.required_text_tuple("input_names");
        let metric_names = fields.required_text_tuple("metric_names");
        let required_capabilities = fields.required_text_tuple("required_capabilities");
        let checkpoint_kind =
            fields.required_nullable_enum("checkpoint_kind", ApprovalKind::VALUES);
        let max_attempts = fields.required_integer("max_attempts");
        fields.reject_extra(&[
            "step_id",
            "name",
            "description",
            "input_names",
            "metric_names",
            "required_capabilities",
            "checkpoint_kind",
            "max_attempts",
        ]);
        fields.finish()?;
        Ok(Self {
            step_id: step_id.expect("validated"),
            name: name.expect("validated"),
            description: description.expect("validated"),
            input_names: input_names.expect("validated"),
            metric_names: metric_names.expect("validated"),
            required_capabilities: required_capabilities.expect("validated"),
            checkpoint_kind: checkpoint_kind
                .expect("validated")
                .and_then(|value| ApprovalKind::parse(&value)),
            max_attempts: max_attempts.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("step_id", text(&self.step_id)),
            ("name", text(&self.name)),
            ("description", text(&self.description)),
            ("input_names", texts(&self.input_names)),
            ("metric_names", texts(&self.metric_names)),
            ("required_capabilities", texts(&self.required_capabilities)),
            (
                "checkpoint_kind",
                match self.checkpoint_kind {
                    Some(value) => text(value.as_str()),
                    None => WireValue::Null,
                },
            ),
            ("max_attempts", integer(&self.max_attempts)),
        ])
    }
}

/// A summary of one declared workflow definition.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowDefinitionView {
    /// The domain the workflow belongs to.
    pub domain: String,
    /// The definition identifier.
    pub definition_id: String,
    /// The definition version.
    pub version: String,
    /// The human readable name.
    pub name: String,
    /// The human readable description, if the definition declares one.
    pub description: Option<String>,
    /// The state the workflow starts in.
    pub initial_state: String,
    /// The states of the workflow.
    pub states: Vec<String>,
    /// The steps the workflow may run.
    pub available_step_ids: Vec<String>,
    /// The transitions of the workflow.
    pub transitions: Vec<WorkflowTransitionView>,
    /// The number of replanning attempts the workflow allows.
    pub max_replanning_attempts: BigInt,
}

impl WorkflowDefinitionView {
    /// The Python model name.
    pub const NAME: &'static str = "WorkflowDefinitionView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let domain = fields.required_text("domain");
        let definition_id = fields.required_text("definition_id");
        let version = fields.required_text("version");
        let name = fields.required_text("name");
        let description = fields.required_nullable_text("description");
        let initial_state = fields.required_text("initial_state");
        let states = fields.required_text_tuple("states");
        let available_step_ids = fields.required_text_tuple("available_step_ids");
        let transitions = fields.required_model_tuple("transitions", |value, location, reading| {
            WorkflowTransitionView::from_value_at(value, location, reading)
        });
        let max_replanning_attempts = fields.required_integer("max_replanning_attempts");
        fields.reject_extra(&[
            "domain",
            "definition_id",
            "version",
            "name",
            "description",
            "initial_state",
            "states",
            "available_step_ids",
            "transitions",
            "max_replanning_attempts",
        ]);
        fields.finish()?;
        Ok(Self {
            domain: domain.expect("validated"),
            definition_id: definition_id.expect("validated"),
            version: version.expect("validated"),
            name: name.expect("validated"),
            description: description.expect("validated"),
            initial_state: initial_state.expect("validated"),
            states: states.expect("validated"),
            available_step_ids: available_step_ids.expect("validated"),
            transitions: transitions.expect("validated"),
            max_replanning_attempts: max_replanning_attempts.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", text(&self.domain)),
            ("definition_id", text(&self.definition_id)),
            ("version", text(&self.version)),
            ("name", text(&self.name)),
            ("description", optional_text(&self.description)),
            ("initial_state", text(&self.initial_state)),
            ("states", texts(&self.states)),
            ("available_step_ids", texts(&self.available_step_ids)),
            (
                "transitions",
                WireValue::Array(
                    self.transitions
                        .iter()
                        .map(WorkflowTransitionView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "max_replanning_attempts",
                integer(&self.max_replanning_attempts),
            ),
        ])
    }
}

/// One declared transition of a workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTransitionView {
    /// The transition identifier.
    pub transition_id: String,
    /// The state the transition leaves.
    pub from_state: String,
    /// The state the transition enters.
    pub to_state: String,
    /// The step the transition belongs to.
    pub step_id: String,
    /// The kind of the checkpoint the transition opens, if it opens one.
    pub checkpoint_type: Option<ApprovalKind>,
}

impl WorkflowTransitionView {
    /// The Python model name.
    pub const NAME: &'static str = "WorkflowTransitionView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let transition_id = fields.required_text("transition_id");
        let from_state = fields.required_text("from_state");
        let to_state = fields.required_text("to_state");
        let step_id = fields.required_text("step_id");
        let checkpoint_type =
            fields.required_nullable_enum("checkpoint_type", ApprovalKind::VALUES);
        fields.reject_extra(&[
            "transition_id",
            "from_state",
            "to_state",
            "step_id",
            "checkpoint_type",
        ]);
        fields.finish()?;
        Ok(Self {
            transition_id: transition_id.expect("validated"),
            from_state: from_state.expect("validated"),
            to_state: to_state.expect("validated"),
            step_id: step_id.expect("validated"),
            checkpoint_type: checkpoint_type
                .expect("validated")
                .and_then(|value| ApprovalKind::parse(&value)),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("transition_id", text(&self.transition_id)),
            ("from_state", text(&self.from_state)),
            ("to_state", text(&self.to_state)),
            ("step_id", text(&self.step_id)),
            (
                "checkpoint_type",
                match self.checkpoint_type {
                    Some(value) => text(value.as_str()),
                    None => WireValue::Null,
                },
            ),
        ])
    }
}

/// The deterministic outcome of one reference design evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainEvaluationReport {
    /// The domain the evaluation belongs to.
    pub domain: String,
    /// The package the evaluation ran against.
    pub package_id: String,
    /// The version of that package.
    pub package_version: String,
    /// The digest of that package.
    pub package_digest: String,
    /// The subject the evaluation ran on.
    pub subject_id: String,
    /// The status of the evaluation.
    pub status: WorkflowStatus,
    /// Whether the evaluation passed.
    pub passed: bool,
    /// The score of the evaluation, if it has one.
    pub score: Option<f64>,
    /// The digest of the parameters the evaluation ran with.
    pub parameters_digest: String,
    /// The steps the evaluation completed.
    pub completed_step_ids: Vec<String>,
    /// The step that blocked the evaluation, if one did.
    pub blocked_step_id: Option<String>,
    /// The evaluations of the individual requirements.
    pub evaluations: Vec<EvaluationResult>,
    /// The findings of the evaluation.
    pub findings: Vec<String>,
    /// The disclaimer of the package.
    pub disclaimer: String,
}

impl DomainEvaluationReport {
    /// The Python model name.
    pub const NAME: &'static str = "DomainEvaluationReport";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let domain = fields.required_text("domain");
        let package_id = fields.required_text("package_id");
        let package_version = fields.required_text("package_version");
        let package_digest = fields.required_text("package_digest");
        let subject_id = fields.required_text("subject_id");
        let status = fields.required_enum("status", WorkflowStatus::VALUES);
        let passed = fields.required_bool("passed");
        let score = fields.required_nullable_number("score");
        let parameters_digest = fields.required_text("parameters_digest");
        let completed_step_ids = fields.required_text_tuple("completed_step_ids");
        let blocked_step_id = fields.required_nullable_text("blocked_step_id");
        let evaluations = fields.required_model_tuple("evaluations", |value, location, reading| {
            contract_model::<EvaluationResult>(value, location, reading)
        });
        let findings = fields.required_text_tuple("findings");
        let disclaimer = fields.required_text("disclaimer");
        fields.reject_extra(&[
            "domain",
            "package_id",
            "package_version",
            "package_digest",
            "subject_id",
            "status",
            "passed",
            "score",
            "parameters_digest",
            "completed_step_ids",
            "blocked_step_id",
            "evaluations",
            "findings",
            "disclaimer",
        ]);
        fields.finish()?;
        Ok(Self {
            domain: domain.expect("validated"),
            package_id: package_id.expect("validated"),
            package_version: package_version.expect("validated"),
            package_digest: package_digest.expect("validated"),
            subject_id: subject_id.expect("validated"),
            status: WorkflowStatus::parse(&status.expect("validated")).expect("validated"),
            passed: passed.expect("validated"),
            score: score.expect("validated"),
            parameters_digest: parameters_digest.expect("validated"),
            completed_step_ids: completed_step_ids.expect("validated"),
            blocked_step_id: blocked_step_id.expect("validated"),
            evaluations: evaluations.expect("validated"),
            findings: findings.expect("validated"),
            disclaimer: disclaimer.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("domain", text(&self.domain)),
            ("package_id", text(&self.package_id)),
            ("package_version", text(&self.package_version)),
            ("package_digest", text(&self.package_digest)),
            ("subject_id", text(&self.subject_id)),
            ("status", text(self.status.as_str())),
            ("passed", boolean(self.passed)),
            (
                "score",
                match self.score {
                    Some(value) => number(value),
                    None => WireValue::Null,
                },
            ),
            ("parameters_digest", text(&self.parameters_digest)),
            ("completed_step_ids", texts(&self.completed_step_ids)),
            ("blocked_step_id", optional_text(&self.blocked_step_id)),
            (
                "evaluations",
                WireValue::Array(
                    self.evaluations
                        .iter()
                        .map(EvaluationResult::to_wire_value)
                        .collect(),
                ),
            ),
            ("findings", texts(&self.findings)),
            ("disclaimer", text(&self.disclaimer)),
        ])
    }
}

/// One attempt of one step.
#[derive(Clone, Debug, PartialEq)]
pub struct StepAttemptView {
    /// The step the attempt belongs to.
    pub step_id: String,
    /// The number of the attempt.
    pub attempt: BigInt,
    /// Whether the attempt succeeded.
    pub succeeded: bool,
    /// The error code of the attempt, if it failed.
    pub error_code: Option<String>,
}

impl StepAttemptView {
    /// The Python model name.
    pub const NAME: &'static str = "StepAttemptView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let step_id = fields.required_text("step_id");
        let attempt = fields.required_integer("attempt");
        let succeeded = fields.required_bool("succeeded");
        let error_code = fields.required_nullable_text("error_code");
        fields.reject_extra(&["step_id", "attempt", "succeeded", "error_code"]);
        fields.finish()?;
        Ok(Self {
            step_id: step_id.expect("validated"),
            attempt: attempt.expect("validated"),
            succeeded: succeeded.expect("validated"),
            error_code: error_code.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("step_id", text(&self.step_id)),
            ("attempt", integer(&self.attempt)),
            ("succeeded", boolean(self.succeeded)),
            ("error_code", optional_text(&self.error_code)),
        ])
    }
}

/// One recorded transition of a workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTransitionRecordView {
    /// The sequence number of the record.
    pub sequence: BigInt,
    /// The transition the record describes.
    pub transition_id: String,
    /// The state the transition left.
    pub from_state: String,
    /// The state the transition entered.
    pub to_state: String,
    /// The step the transition belongs to.
    pub step_id: String,
    /// The attempt the transition belongs to.
    pub attempt: BigInt,
}

impl WorkflowTransitionRecordView {
    /// The Python model name.
    pub const NAME: &'static str = "WorkflowTransitionRecordView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let sequence = fields.required_integer("sequence");
        let transition_id = fields.required_text("transition_id");
        let from_state = fields.required_text("from_state");
        let to_state = fields.required_text("to_state");
        let step_id = fields.required_text("step_id");
        let attempt = fields.required_integer("attempt");
        fields.reject_extra(&[
            "sequence",
            "transition_id",
            "from_state",
            "to_state",
            "step_id",
            "attempt",
        ]);
        fields.finish()?;
        Ok(Self {
            sequence: sequence.expect("validated"),
            transition_id: transition_id.expect("validated"),
            from_state: from_state.expect("validated"),
            to_state: to_state.expect("validated"),
            step_id: step_id.expect("validated"),
            attempt: attempt.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("sequence", integer(&self.sequence)),
            ("transition_id", text(&self.transition_id)),
            ("from_state", text(&self.from_state)),
            ("to_state", text(&self.to_state)),
            ("step_id", text(&self.step_id)),
            ("attempt", integer(&self.attempt)),
        ])
    }
}

/// A policy checkpoint awaiting or carrying a local decision.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointView {
    /// The identifier of the approval.
    pub approval_id: String,
    /// The transition the checkpoint belongs to.
    pub transition_id: String,
    /// The step the checkpoint belongs to.
    pub step_id: String,
    /// The kind of the checkpoint.
    pub checkpoint_type: ApprovalKind,
    /// The digest of the state the decision is bound to.
    pub state_digest: String,
    /// The digest of the action the decision is bound to.
    pub action_digest: String,
    /// The status of the checkpoint.
    pub status: ApprovalStatus,
    /// The actor that decided, if it was decided.
    pub actor_id: Option<String>,
    /// The moment it was decided, if it was decided.
    pub decided_at: Option<Timestamp>,
    /// The reason of the decision, if one was given.
    pub reason: Option<String>,
}

impl CheckpointView {
    /// The Python model name.
    pub const NAME: &'static str = "CheckpointView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let approval_id = fields.required_text("approval_id");
        let transition_id = fields.required_text("transition_id");
        let step_id = fields.required_text("step_id");
        let checkpoint_type = fields.required_enum("checkpoint_type", ApprovalKind::VALUES);
        let state_digest = fields.required_text("state_digest");
        let action_digest = fields.required_text("action_digest");
        let status = fields.required_enum("status", ApprovalStatus::VALUES);
        let actor_id = fields.required_nullable_text("actor_id");
        let decided_at = fields.required_nullable_timestamp("decided_at");
        let reason = fields.required_nullable_text("reason");
        fields.reject_extra(&[
            "approval_id",
            "transition_id",
            "step_id",
            "checkpoint_type",
            "state_digest",
            "action_digest",
            "status",
            "actor_id",
            "decided_at",
            "reason",
        ]);
        fields.finish()?;
        Ok(Self {
            approval_id: approval_id.expect("validated"),
            transition_id: transition_id.expect("validated"),
            step_id: step_id.expect("validated"),
            checkpoint_type: ApprovalKind::parse(&checkpoint_type.expect("validated"))
                .expect("validated"),
            state_digest: state_digest.expect("validated"),
            action_digest: action_digest.expect("validated"),
            status: ApprovalStatus::parse(&status.expect("validated")).expect("validated"),
            actor_id: actor_id.expect("validated"),
            decided_at: decided_at.expect("validated"),
            reason: reason.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("approval_id", text(&self.approval_id)),
            ("transition_id", text(&self.transition_id)),
            ("step_id", text(&self.step_id)),
            ("checkpoint_type", text(self.checkpoint_type.as_str())),
            ("state_digest", text(&self.state_digest)),
            ("action_digest", text(&self.action_digest)),
            ("status", text(self.status.as_str())),
            ("actor_id", optional_text(&self.actor_id)),
            ("decided_at", optional_timestamp(&self.decided_at)),
            ("reason", optional_text(&self.reason)),
        ])
    }
}

/// A bounded, serializable mirror of the kernel workflow state.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowStateView {
    /// The identifier of the workflow.
    pub workflow_id: String,
    /// The identifier of the workflow definition.
    pub definition_id: String,
    /// The version of the workflow definition.
    pub definition_version: String,
    /// The status of the workflow.
    pub status: WorkflowStatus,
    /// The state the workflow is in.
    pub current_state: String,
    /// The steps the workflow completed.
    pub completed_step_ids: Vec<String>,
    /// The attempts of the workflow.
    pub attempts: Vec<StepAttemptView>,
    /// The checkpoint that is still pending, if one is.
    pub pending_approval: Option<CheckpointView>,
    /// The number of replanning attempts the workflow used.
    pub replanning_attempts: BigInt,
    /// The recorded transitions of the workflow.
    pub history: Vec<WorkflowTransitionRecordView>,
    /// The last error of the workflow, if it failed.
    pub last_error: Option<String>,
}

impl WorkflowStateView {
    /// The Python model name.
    pub const NAME: &'static str = "WorkflowStateView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let workflow_id = fields.required_text("workflow_id");
        let definition_id = fields.required_text("definition_id");
        let definition_version = fields.required_text("definition_version");
        let status = fields.required_enum("status", WorkflowStatus::VALUES);
        let current_state = fields.required_text("current_state");
        let completed_step_ids = fields.required_text_tuple("completed_step_ids");
        let attempts = fields.required_model_tuple("attempts", |value, location, reading| {
            StepAttemptView::from_value_at(value, location, reading)
        });
        let pending_approval = fields
            .required_nullable_model("pending_approval", |value, location, reading| {
                CheckpointView::from_value_at(value, location, reading)
            });
        let replanning_attempts = fields.required_integer("replanning_attempts");
        let history = fields.required_model_tuple("history", |value, location, reading| {
            WorkflowTransitionRecordView::from_value_at(value, location, reading)
        });
        let last_error = fields.required_nullable_text("last_error");
        fields.reject_extra(&[
            "workflow_id",
            "definition_id",
            "definition_version",
            "status",
            "current_state",
            "completed_step_ids",
            "attempts",
            "pending_approval",
            "replanning_attempts",
            "history",
            "last_error",
        ]);
        fields.finish()?;
        Ok(Self {
            workflow_id: workflow_id.expect("validated"),
            definition_id: definition_id.expect("validated"),
            definition_version: definition_version.expect("validated"),
            status: WorkflowStatus::parse(&status.expect("validated")).expect("validated"),
            current_state: current_state.expect("validated"),
            completed_step_ids: completed_step_ids.expect("validated"),
            attempts: attempts.expect("validated"),
            pending_approval: pending_approval.expect("validated"),
            replanning_attempts: replanning_attempts.expect("validated"),
            history: history.expect("validated"),
            last_error: last_error.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("workflow_id", text(&self.workflow_id)),
            ("definition_id", text(&self.definition_id)),
            ("definition_version", text(&self.definition_version)),
            ("status", text(self.status.as_str())),
            ("current_state", text(&self.current_state)),
            ("completed_step_ids", texts(&self.completed_step_ids)),
            (
                "attempts",
                WireValue::Array(
                    self.attempts
                        .iter()
                        .map(StepAttemptView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "pending_approval",
                match &self.pending_approval {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            ("replanning_attempts", integer(&self.replanning_attempts)),
            (
                "history",
                WireValue::Array(
                    self.history
                        .iter()
                        .map(WorkflowTransitionRecordView::to_wire_value)
                        .collect(),
                ),
            ),
            ("last_error", optional_text(&self.last_error)),
        ])
    }
}

/// The usage of the budget of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct BudgetUsageView {
    /// The number of model calls.
    pub model_calls: BigInt,
    /// The number of tool calls.
    pub tool_calls: BigInt,
    /// The number of tokens that were used.
    pub total_tokens: BigInt,
    /// The number of bytes that were written.
    pub artifact_bytes: BigInt,
    /// The duration of the run in seconds.
    pub duration_seconds: f64,
    /// The cost of the run.
    pub cost: String,
    /// The number of replanning attempts.
    pub replanning_attempts: BigInt,
}

impl BudgetUsageView {
    /// The Python model name.
    pub const NAME: &'static str = "BudgetUsageView";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let model_calls = fields.required_integer("model_calls");
        let tool_calls = fields.required_integer("tool_calls");
        let total_tokens = fields.required_integer("total_tokens");
        let artifact_bytes = fields.required_integer("artifact_bytes");
        let duration_seconds = fields.required_number("duration_seconds");
        let cost = fields.required_text("cost");
        let replanning_attempts = fields.required_integer("replanning_attempts");
        fields.reject_extra(&[
            "model_calls",
            "tool_calls",
            "total_tokens",
            "artifact_bytes",
            "duration_seconds",
            "cost",
            "replanning_attempts",
        ]);
        fields.finish()?;
        Ok(Self {
            model_calls: model_calls.expect("validated"),
            tool_calls: tool_calls.expect("validated"),
            total_tokens: total_tokens.expect("validated"),
            artifact_bytes: artifact_bytes.expect("validated"),
            duration_seconds: duration_seconds.expect("validated"),
            cost: cost.expect("validated"),
            replanning_attempts: replanning_attempts.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("model_calls", integer(&self.model_calls)),
            ("tool_calls", integer(&self.tool_calls)),
            ("total_tokens", integer(&self.total_tokens)),
            ("artifact_bytes", integer(&self.artifact_bytes)),
            ("duration_seconds", number(self.duration_seconds)),
            ("cost", text(&self.cost)),
            ("replanning_attempts", integer(&self.replanning_attempts)),
        ])
    }
}

/// An immutable revision together with its provenance record.
#[derive(Clone, Debug, PartialEq)]
pub struct RevisionRecord {
    /// The revision that was committed.
    pub revision: DesignRevision,
    /// The provenance of the revision.
    pub provenance: Provenance,
    /// The digests of the artifacts the revision carries.
    pub artifact_digests: Vec<String>,
}

impl RevisionRecord {
    /// The Python model name.
    pub const NAME: &'static str = "RevisionRecord";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let revision = fields.required_model("revision", |value, location, reading| {
            contract_model::<DesignRevision>(value, location, reading)
        });
        let provenance = fields.required_model("provenance", |value, location, reading| {
            contract_model::<Provenance>(value, location, reading)
        });
        let artifact_digests = fields.required_text_tuple("artifact_digests");
        fields.reject_extra(&["revision", "provenance", "artifact_digests"]);
        fields.finish()?;
        Ok(Self {
            revision: revision.expect("validated"),
            provenance: provenance.expect("validated"),
            artifact_digests: artifact_digests.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("revision", self.revision.to_wire_value()),
            ("provenance", self.provenance.to_wire_value()),
            ("artifact_digests", texts(&self.artifact_digests)),
        ])
    }
}

/// The outcome of one mechanical reference workflow run or decision.
#[derive(Clone, Debug, PartialEq)]
pub struct MechanicalRunResult {
    /// The identifier of the workflow.
    pub workflow_id: String,
    /// The design the run belongs to.
    pub design_id: String,
    /// The status of the workflow.
    pub status: WorkflowStatus,
    /// The state of the workflow.
    pub state: WorkflowStateView,
    /// The evaluations of the run.
    pub evaluations: Vec<EvaluationResult>,
    /// The checkpoint the run stopped at, if it stopped at one.
    pub checkpoint: Option<CheckpointView>,
    /// The policy decision of the run, if the policy judged it.
    pub policy_status: Option<PolicyDecisionStatus>,
    /// The reason of the policy decision, if the policy judged it.
    pub policy_reason: Option<String>,
    /// The tasks the run planned.
    pub planned_task_ids: Vec<String>,
    /// Whether the run started a new workflow.
    pub started: bool,
    /// The revision the run committed, if it committed one.
    pub revision: Option<RevisionRecord>,
    /// The budget the run used.
    pub budget: BudgetUsageView,
}

impl MechanicalRunResult {
    /// The Python model name.
    pub const NAME: &'static str = "MechanicalRunResult";

    /// Read one view.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one view at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    pub fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = object_of(value, Self::NAME, location, reading)?;
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let workflow_id = fields.required_text("workflow_id");
        let design_id = fields.required_text("design_id");
        let status = fields.required_enum("status", WorkflowStatus::VALUES);
        let state = fields.required_model("state", |value, location, reading| {
            WorkflowStateView::from_value_at(value, location, reading)
        });
        let evaluations = fields.required_model_tuple("evaluations", |value, location, reading| {
            contract_model::<EvaluationResult>(value, location, reading)
        });
        let checkpoint = fields
            .required_nullable_model("checkpoint", |value, location, reading| {
                CheckpointView::from_value_at(value, location, reading)
            });
        let policy_status =
            fields.required_nullable_enum("policy_status", PolicyDecisionStatus::VALUES);
        let policy_reason = fields.required_nullable_text("policy_reason");
        let planned_task_ids = fields.required_text_tuple("planned_task_ids");
        let started = fields.required_bool("started");
        let revision = fields.required_nullable_model("revision", |value, location, reading| {
            RevisionRecord::from_value_at(value, location, reading)
        });
        let budget = fields.required_model("budget", |value, location, reading| {
            BudgetUsageView::from_value_at(value, location, reading)
        });
        fields.reject_extra(&[
            "workflow_id",
            "design_id",
            "status",
            "state",
            "evaluations",
            "checkpoint",
            "policy_status",
            "policy_reason",
            "planned_task_ids",
            "started",
            "revision",
            "budget",
        ]);
        fields.finish()?;
        Ok(Self {
            workflow_id: workflow_id.expect("validated"),
            design_id: design_id.expect("validated"),
            status: WorkflowStatus::parse(&status.expect("validated")).expect("validated"),
            state: state.expect("validated"),
            evaluations: evaluations.expect("validated"),
            checkpoint: checkpoint.expect("validated"),
            policy_status: policy_status
                .expect("validated")
                .and_then(|value| PolicyDecisionStatus::parse(&value)),
            policy_reason: policy_reason.expect("validated"),
            planned_task_ids: planned_task_ids.expect("validated"),
            started: started.expect("validated"),
            revision: revision.expect("validated"),
            budget: budget.expect("validated"),
        })
    }

    /// The `model_dump(mode="json")` shape, in field order.
    #[must_use]
    pub fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("workflow_id", text(&self.workflow_id)),
            ("design_id", text(&self.design_id)),
            ("status", text(self.status.as_str())),
            ("state", self.state.to_wire_value()),
            (
                "evaluations",
                WireValue::Array(
                    self.evaluations
                        .iter()
                        .map(EvaluationResult::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "checkpoint",
                match &self.checkpoint {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "policy_status",
                match self.policy_status {
                    Some(value) => text(value.as_str()),
                    None => WireValue::Null,
                },
            ),
            ("policy_reason", optional_text(&self.policy_reason)),
            ("planned_task_ids", texts(&self.planned_task_ids)),
            ("started", boolean(self.started)),
            (
                "revision",
                match &self.revision {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            ("budget", self.budget.to_wire_value()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_refuses_a_value_that_is_not_an_object() {
        let failures = DomainPackageView::from_value(&CanonicalValue::from(5)).unwrap_err();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].kind, kind::MODEL_TYPE);
        assert_eq!(
            failures[0].message,
            "Input should be a valid dictionary or instance of DomainPackageView"
        );
    }

    #[test]
    fn the_contract_location_is_read() {
        let location = parse_contract_location("evaluations[0].metrics[1].name");
        assert_eq!(
            location,
            vec![
                Location::Field("evaluations".to_owned()),
                Location::Index(0),
                Location::Field("metrics".to_owned()),
                Location::Index(1),
                Location::Field("name".to_owned()),
            ]
        );
    }

    #[test]
    fn the_enums_carry_the_python_values() {
        assert_eq!(ApprovalKind::VALUES.len(), 6);
        assert_eq!(ApprovalKind::PlanApproval.as_str(), "plan_approval");
        assert_eq!(ApprovalStatus::Pending.as_str(), "pending");
        assert_eq!(
            WorkflowStatus::WaitingForApproval.as_str(),
            "waiting_for_approval"
        );
        assert_eq!(PolicyDecisionStatus::Invalidated.as_str(), "invalidated");
    }
}
