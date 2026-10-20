//! The request and response models of the local Kala HTTP adapter.
//!
//! This is the native form of `kala_agent.api.schemas`. Every model, every
//! constraint, every default and every failure message of the Python source
//! is here, and the JSON shapes are the ones the Python source writes:
//!
//! * the request models are strict (`extra="forbid"`), frozen and strip the
//!   whitespace of their string fields;
//! * the response models are strict and frozen and keep the whitespace;
//! * the response models carry the application views in the declaration order
//!   of their fields, and the timestamps in the RFC 3339 spelling Pydantic
//!   writes.
//!
//! The Python module is the reference: a request that the Python source
//! accepts is accepted here with the same values, and a request it refuses is
//! refused here with the same kind, the same location, the same message and
//! the same context, which is what FastAPI renders in the `detail` of a 422.

use kala_contracts::{ApprovalRequest, ContractModel, WireValue};
use kala_json::{CanonicalObject, CanonicalValue, Limits};
use num_bigint::BigInt;

use crate::patterns::{DESIGN_ID_PATTERN, DIGEST_PATTERN, IDENTIFIER_PATTERN};
use crate::validation::{Failures, Fields, Location, Reading, value_error_issue};
use crate::views::{
    CheckpointView, DomainEvaluationReport, DomainPackageView, MechanicalRunResult, RevisionRecord,
    WorkflowDefinitionView, WorkflowStateView,
};

/// The largest canonical JSON document the parameters of a request may carry.
pub const MAX_PARAMETERS_BYTES: usize = 16_384;

/// The largest number of entries the parameters of a request may declare.
pub const MAX_PARAMETERS_ENTRIES: usize = 256;

/// The longest parameter name a request may declare.
pub const MAX_PARAMETER_KEY_LENGTH: usize = 64;

/// The longest summary a request may carry.
pub const MAX_SUMMARY_LENGTH: usize = 512;

/// The longest reason a request may carry.
pub const MAX_REASON_LENGTH: usize = 512;

/// The longest actor identifier a request may carry.
pub const MAX_ACTOR_LENGTH: usize = 64;

/// The longest design identifier a request may carry.
pub const MAX_DESIGN_ID_LENGTH: usize = 64;

/// The longest identifier a request may carry.
pub const MAX_IDENTIFIER_LENGTH: usize = 128;

/// The largest page a listing may ask for.
pub const MAX_LIST_LIMIT: usize = 200;

/// The page a listing asks for when it does not ask.
pub const DEFAULT_LIST_LIMIT: usize = 50;

/// The largest sequence number a listing may ask after.
pub const MAX_AFTER_SEQUENCE: u64 = 1_000_000_000;

/// The name of the service this API answers for.
pub const SERVICE_NAME: &str = "kala-agent";

/// The design the mechanical reference workflow runs on by default.
pub const DEFAULT_MECHANICAL_DESIGN_ID: &str = "mechanical-reference";

/// The actor a local decision is recorded for by default.
pub const DEFAULT_ACTOR_ID: &str = "local-user";

/// The registered domain packages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DomainName {
    /// The building domain package.
    Building,
    /// The mechanical domain package.
    Mechanical,
    /// The wing domain package.
    Wing,
}

impl DomainName {
    /// Every registered domain, in declaration order.
    pub const ALL: &'static [DomainName] = &[Self::Building, Self::Mechanical, Self::Wing];

    /// The accepted values as the strings the Python enum declares.
    pub const VALUES: &'static [&'static str] = &["building", "mechanical", "wing"];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::Mechanical => "mechanical",
            Self::Wing => "wing",
        }
    }

    /// Parse an exact Python enum value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "building" => Some(Self::Building),
            "mechanical" => Some(Self::Mechanical),
            "wing" => Some(Self::Wing),
            _ => None,
        }
    }
}

impl std::fmt::Display for DomainName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A model of the API surface.
///
/// The trait gives every model the two shapes the Python source writes: the
/// ordered `model_dump(mode="json")` document FastAPI sends, and the canonical
/// JSON document the digests are taken over.
pub trait ApiModel: Sized {
    /// The Python class name.
    const NAME: &'static str;

    /// Read the model from a JSON value.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value(value: &CanonicalValue) -> Result<Self, Failures>;

    /// Read the model from a request body, the way FastAPI reads it.
    ///
    /// A body that is not an object is refused by the body layer of the
    /// framework and not by the model, and a body that is not there at all is
    /// a missing field.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_body(body: Option<&CanonicalValue>) -> Result<Self, Failures> {
        match body {
            None => Err(vec![crate::validation::body_missing_issue()]),
            Some(CanonicalValue::Null) => Err(vec![crate::validation::body_missing_issue()]),
            Some(value) if !matches!(value, CanonicalValue::Object(_)) => {
                Err(vec![crate::validation::body_type_issue(&[], value)])
            }
            Some(value) => Self::from_value_at(value, &[], Reading::Attributes),
        }
    }

    /// Read the model from a JSON value at a path.
    ///
    /// The path is the path of the model itself, which a nested model carries
    /// so that a failure of a nested field is reported where it happened.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures>;

    /// The `model_dump(mode="json")` shape, in field order.
    fn to_wire_value(&self) -> WireValue;

    /// The bytes FastAPI writes for this model.
    ///
    /// The document is compact, the field order is the declaration order, and
    /// the text is UTF-8, which is what Starlette's JSON response writes.
    #[must_use]
    fn to_wire_json(&self) -> String {
        self.to_wire_value()
            .encode(false)
            .unwrap_or_else(|_| String::from("{}"))
    }

    /// The canonical JSON bytes of this model.
    ///
    /// The keys are sorted by code point and the text is ASCII escaped, which
    /// is the form the digests are taken over.
    ///
    /// # Errors
    ///
    /// Returns the reason the document cannot be canonicalized.
    fn to_canonical_json(&self) -> Result<String, String> {
        let value = self
            .to_wire_value()
            .to_canonical_value()
            .map_err(|error| error.to_string())?;
        kala_json::canonical_dumps(&value, &request_body_limits())
            .map_err(|error| error.to_string())
    }
}

/// Validate the parameters of a request.
///
/// The Python source refuses more than 256 entries, refuses an empty or a
/// long parameter name, and refuses a document whose canonical JSON is larger
/// than 16 KiB or that leaves the bounded JSON domain of the tool protocol.
fn validate_parameters(value: &CanonicalObject) -> Result<(), String> {
    if value.len() > MAX_PARAMETERS_ENTRIES {
        return Err(format!(
            "parameters cannot declare more than {MAX_PARAMETERS_ENTRIES} entries"
        ));
    }
    for key in value.keys() {
        let length = key.code_points().len();
        if length == 0 || length > MAX_PARAMETER_KEY_LENGTH {
            return Err("parameter names must be nonempty and bounded".to_owned());
        }
    }
    let document = CanonicalValue::Object(value.clone());
    let limits = Limits {
        max_json_bytes: MAX_PARAMETERS_BYTES,
        ..Limits::default()
    };
    kala_json::canonical_dumps(&document, &limits)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Read the parameters of a request and apply the parameter bounds.
fn read_parameters(fields: &mut Fields<'_>, name: &str) -> Option<Option<CanonicalObject>> {
    let object = fields.optional_object(name)?;
    let Some(object) = object else {
        return Some(None);
    };
    let location = vec![Location::Field(name.to_owned())];
    let document = CanonicalValue::Object(object.clone());
    match validate_parameters(&object) {
        Ok(()) => Some(Some(object)),
        Err(reason) => {
            fields.push_failure(value_error_issue(&location, &document, &reason));
            None
        }
    }
}

/// A request for one deterministic reference design evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluateDomainRequest {
    /// The design the evaluation runs on, if the caller names one.
    pub design_id: Option<String>,
    /// The overrides of the declared domain parameters.
    pub parameters: Option<CanonicalObject>,
}

impl Default for EvaluateDomainRequest {
    /// The request the Python source builds when the body is not there.
    fn default() -> Self {
        Self {
            design_id: None,
            parameters: None,
        }
    }
}

impl ApiModel for EvaluateDomainRequest {
    const NAME: &'static str = "EvaluateDomainRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, true, location, reading);
        let design_id = fields.optional_text_bounded_pattern(
            "design_id",
            None,
            Some(MAX_DESIGN_ID_LENGTH),
            DESIGN_ID_PATTERN,
        );
        let parameters = read_parameters(&mut fields, "parameters");
        fields.reject_extra(&["design_id", "parameters"]);
        fields.finish()?;
        Ok(Self {
            design_id: design_id.flatten(),
            parameters: parameters.flatten(),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("design_id", wire_optional_text(&self.design_id)),
            ("parameters", wire_object(&self.parameters)),
        ])
    }
}

/// A request for one immutable, content-addressed design revision.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateRevisionRequest {
    /// The design the revision belongs to.
    pub design_id: String,
    /// The domain package the revision belongs to.
    pub domain: DomainName,
    /// The summary of the revision.
    pub summary: Option<String>,
    /// The revision this one extends.
    pub parent_revision_id: Option<String>,
    /// The overrides of the declared domain parameters.
    pub parameters: Option<CanonicalObject>,
    /// The digest of the state the caller prepared.
    pub state_digest: Option<String>,
    /// The digest of the action the caller prepared.
    pub action_digest: Option<String>,
}

impl ApiModel for CreateRevisionRequest {
    const NAME: &'static str = "CreateRevisionRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, true, location, reading);
        let design_id = fields.required_text_bounded_pattern(
            "design_id",
            Some(1),
            Some(MAX_DESIGN_ID_LENGTH),
            DESIGN_ID_PATTERN,
        );
        let domain = fields.required_enum("domain", DomainName::VALUES);
        let summary = fields.optional_text_bounded("summary", None, Some(MAX_SUMMARY_LENGTH));
        let parent_revision_id = fields.optional_text_bounded_pattern(
            "parent_revision_id",
            None,
            Some(MAX_IDENTIFIER_LENGTH),
            IDENTIFIER_PATTERN,
        );
        let parameters = read_parameters(&mut fields, "parameters");
        let state_digest = fields.optional_text_pattern("state_digest", DIGEST_PATTERN);
        let action_digest = fields.optional_text_pattern("action_digest", DIGEST_PATTERN);
        fields.reject_extra(&[
            "design_id",
            "domain",
            "summary",
            "parent_revision_id",
            "parameters",
            "state_digest",
            "action_digest",
        ]);
        fields.finish()?;
        Ok(Self {
            design_id: design_id.expect("validated"),
            domain: DomainName::parse(&domain.expect("validated")).expect("validated"),
            summary: summary.flatten(),
            parent_revision_id: parent_revision_id.flatten(),
            parameters: parameters.flatten(),
            state_digest: state_digest.flatten(),
            action_digest: action_digest.flatten(),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("design_id", wire_text(&self.design_id)),
            ("domain", wire_text(self.domain.as_str())),
            ("summary", wire_optional_text(&self.summary)),
            (
                "parent_revision_id",
                wire_optional_text(&self.parent_revision_id),
            ),
            ("parameters", wire_object(&self.parameters)),
            ("state_digest", wire_optional_text(&self.state_digest)),
            ("action_digest", wire_optional_text(&self.action_digest)),
        ])
    }
}

/// A request for one prepared revision commit, and for nothing to be written.
#[derive(Clone, Debug, PartialEq)]
pub struct PrepareRevisionRequest {
    /// The design the revision belongs to.
    pub design_id: String,
    /// The domain package the revision belongs to.
    pub domain: DomainName,
    /// The summary of the revision.
    pub summary: Option<String>,
    /// The revision this one extends.
    pub parent_revision_id: Option<String>,
    /// The overrides of the declared domain parameters.
    pub parameters: Option<CanonicalObject>,
}

impl ApiModel for PrepareRevisionRequest {
    const NAME: &'static str = "PrepareRevisionRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, true, location, reading);
        let design_id = fields.required_text_bounded_pattern(
            "design_id",
            Some(1),
            Some(MAX_DESIGN_ID_LENGTH),
            DESIGN_ID_PATTERN,
        );
        let domain = fields.required_enum("domain", DomainName::VALUES);
        let summary = fields.optional_text_bounded("summary", None, Some(MAX_SUMMARY_LENGTH));
        let parent_revision_id = fields.optional_text_bounded_pattern(
            "parent_revision_id",
            None,
            Some(MAX_IDENTIFIER_LENGTH),
            IDENTIFIER_PATTERN,
        );
        let parameters = read_parameters(&mut fields, "parameters");
        fields.reject_extra(&[
            "design_id",
            "domain",
            "summary",
            "parent_revision_id",
            "parameters",
        ]);
        fields.finish()?;
        Ok(Self {
            design_id: design_id.expect("validated"),
            domain: DomainName::parse(&domain.expect("validated")).expect("validated"),
            summary: summary.flatten(),
            parent_revision_id: parent_revision_id.flatten(),
            parameters: parameters.flatten(),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("design_id", wire_text(&self.design_id)),
            ("domain", wire_text(self.domain.as_str())),
            ("summary", wire_optional_text(&self.summary)),
            (
                "parent_revision_id",
                wire_optional_text(&self.parent_revision_id),
            ),
            ("parameters", wire_object(&self.parameters)),
        ])
    }
}

/// A request to start or resume the mechanical reference workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct RunMechanicalWorkflowRequest {
    /// The design the run belongs to.
    pub design_id: String,
    /// The workflow to run or resume.
    pub workflow_id: Option<String>,
    /// The actor that asks for the run.
    pub actor_id: String,
}

impl Default for RunMechanicalWorkflowRequest {
    /// The request the Python source builds when the body is not there.
    fn default() -> Self {
        Self {
            design_id: DEFAULT_MECHANICAL_DESIGN_ID.to_owned(),
            workflow_id: None,
            actor_id: DEFAULT_ACTOR_ID.to_owned(),
        }
    }
}

impl ApiModel for RunMechanicalWorkflowRequest {
    const NAME: &'static str = "RunMechanicalWorkflowRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, true, location, reading);
        let design_id = fields.default_text_bounded_pattern(
            "design_id",
            "mechanical-reference",
            Some(1),
            Some(MAX_DESIGN_ID_LENGTH),
            DESIGN_ID_PATTERN,
        );
        let workflow_id = fields.optional_text_bounded_pattern(
            "workflow_id",
            None,
            Some(MAX_IDENTIFIER_LENGTH),
            IDENTIFIER_PATTERN,
        );
        let actor_id = fields.default_text_bounded_pattern(
            "actor_id",
            "local-user",
            Some(1),
            Some(MAX_ACTOR_LENGTH),
            IDENTIFIER_PATTERN,
        );
        fields.reject_extra(&["design_id", "workflow_id", "actor_id"]);
        fields.finish()?;
        Ok(Self {
            design_id,
            workflow_id: workflow_id.flatten(),
            actor_id,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("design_id", wire_text(&self.design_id)),
            ("workflow_id", wire_optional_text(&self.workflow_id)),
            ("actor_id", wire_text(&self.actor_id)),
        ])
    }
}

/// A request to record a local approve or reject decision.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointDecisionRequest {
    /// The actor that decides.
    pub actor_id: String,
    /// The digest of the state the decision is bound to.
    pub state_digest: Option<String>,
    /// The digest of the action the decision is bound to.
    pub action_digest: Option<String>,
    /// The reason of the decision.
    pub reason: Option<String>,
}

impl Default for CheckpointDecisionRequest {
    /// The request the Python source builds when the body is not there.
    fn default() -> Self {
        Self {
            actor_id: DEFAULT_ACTOR_ID.to_owned(),
            state_digest: None,
            action_digest: None,
            reason: None,
        }
    }
}

impl ApiModel for CheckpointDecisionRequest {
    const NAME: &'static str = "CheckpointDecisionRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, true, location, reading);
        let actor_id = fields.default_text_bounded_pattern(
            "actor_id",
            "local-user",
            Some(1),
            Some(MAX_ACTOR_LENGTH),
            IDENTIFIER_PATTERN,
        );
        let state_digest = fields.optional_text_pattern("state_digest", DIGEST_PATTERN);
        let action_digest = fields.optional_text_pattern("action_digest", DIGEST_PATTERN);
        let reason = fields.optional_text_bounded("reason", None, Some(MAX_REASON_LENGTH));
        fields.reject_extra(&["actor_id", "state_digest", "action_digest", "reason"]);
        fields.finish()?;
        Ok(Self {
            actor_id,
            state_digest: state_digest.flatten(),
            action_digest: action_digest.flatten(),
            reason: reason.flatten(),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("actor_id", wire_text(&self.actor_id)),
            ("state_digest", wire_optional_text(&self.state_digest)),
            ("action_digest", wire_optional_text(&self.action_digest)),
            ("reason", wire_optional_text(&self.reason)),
        ])
    }
}

/// The liveness of the local process.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthResponse {
    /// The status of the process.
    pub status: String,
    /// The name of the service.
    pub service: String,
    /// The version of the service.
    pub version: String,
    /// The model provider the process holds.
    pub provider: String,
    /// The model the process holds.
    pub model: String,
    /// Whether the process is offline.
    pub offline: bool,
    /// Whether an egress is configured.
    pub egress_configured: bool,
    /// Whether a provider endpoint is declared.
    pub provider_endpoint_declared: bool,
    /// The local data directory.
    pub data_dir: String,
    /// The version of the database schema.
    pub schema_version: BigInt,
    /// The registered domains.
    pub domains: Vec<String>,
    /// The disclaimer of the service.
    pub disclaimer: String,
}

impl ApiModel for HealthResponse {
    const NAME: &'static str = "HealthResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let status = fields.required_text("status");
        let service = fields.required_text("service");
        let version = fields.required_text("version");
        let provider = fields.required_text("provider");
        let model = fields.required_text("model");
        let offline = fields.required_bool("offline");
        let egress_configured = fields.required_bool("egress_configured");
        let provider_endpoint_declared = fields.required_bool("provider_endpoint_declared");
        let data_dir = fields.required_text("data_dir");
        let schema_version = fields.required_integer("schema_version");
        let domains = fields.required_text_tuple("domains");
        let disclaimer = fields.required_text("disclaimer");
        fields.reject_extra(&[
            "status",
            "service",
            "version",
            "provider",
            "model",
            "offline",
            "egress_configured",
            "provider_endpoint_declared",
            "data_dir",
            "schema_version",
            "domains",
            "disclaimer",
        ]);
        fields.finish()?;
        Ok(Self {
            status: status.expect("validated"),
            service: service.expect("validated"),
            version: version.expect("validated"),
            provider: provider.expect("validated"),
            model: model.expect("validated"),
            offline: offline.expect("validated"),
            egress_configured: egress_configured.expect("validated"),
            provider_endpoint_declared: provider_endpoint_declared.expect("validated"),
            data_dir: data_dir.expect("validated"),
            schema_version: schema_version.expect("validated"),
            domains: domains.expect("validated"),
            disclaimer: disclaimer.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("status", wire_text(&self.status)),
            ("service", wire_text(&self.service)),
            ("version", wire_text(&self.version)),
            ("provider", wire_text(&self.provider)),
            ("model", wire_text(&self.model)),
            ("offline", WireValue::Bool(self.offline)),
            ("egress_configured", WireValue::Bool(self.egress_configured)),
            (
                "provider_endpoint_declared",
                WireValue::Bool(self.provider_endpoint_declared),
            ),
            ("data_dir", wire_text(&self.data_dir)),
            (
                "schema_version",
                WireValue::Integer(self.schema_version.clone()),
            ),
            (
                "domains",
                WireValue::Array(self.domains.iter().map(|value| wire_text(value)).collect()),
            ),
            ("disclaimer", wire_text(&self.disclaimer)),
        ])
    }
}

/// Every registered domain package and workflow definition.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainListResponse {
    /// The number of registered packages.
    pub count: BigInt,
    /// The registered packages.
    pub packages: Vec<DomainPackageView>,
    /// The registered workflow definitions.
    pub workflows: Vec<WorkflowDefinitionView>,
    /// The registered domains.
    pub domains: Vec<String>,
}

impl ApiModel for DomainListResponse {
    const NAME: &'static str = "DomainListResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let count = fields.required_integer("count");
        let packages = fields.required_model_tuple("packages", |value, location, reading| {
            DomainPackageView::from_value_at(value, location, reading)
        });
        let workflows = fields.required_model_tuple("workflows", |value, location, reading| {
            WorkflowDefinitionView::from_value_at(value, location, reading)
        });
        let domains = fields.required_text_tuple("domains");
        fields.reject_extra(&["count", "packages", "workflows", "domains"]);
        fields.finish()?;
        Ok(Self {
            count: count.expect("validated"),
            packages: packages.expect("validated"),
            workflows: workflows.expect("validated"),
            domains: domains.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("count", WireValue::Integer(self.count.clone())),
            (
                "packages",
                WireValue::Array(
                    self.packages
                        .iter()
                        .map(DomainPackageView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "workflows",
                WireValue::Array(
                    self.workflows
                        .iter()
                        .map(WorkflowDefinitionView::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "domains",
                WireValue::Array(self.domains.iter().map(|value| wire_text(value)).collect()),
            ),
        ])
    }
}

/// One deterministic reference design evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationResponse {
    /// The evaluation of the request.
    pub evaluation: DomainEvaluationReport,
    /// Every evaluation of the request.
    pub evaluations: Vec<DomainEvaluationReport>,
}

impl ApiModel for EvaluationResponse {
    const NAME: &'static str = "EvaluationResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let evaluation = fields.required_model("evaluation", |value, location, reading| {
            DomainEvaluationReport::from_value_at(value, location, reading)
        });
        let evaluations = fields.default_model_tuple("evaluations", |value, location, reading| {
            DomainEvaluationReport::from_value_at(value, location, reading)
        });
        fields.reject_extra(&["evaluation", "evaluations"]);
        fields.finish()?;
        Ok(Self {
            evaluation: evaluation.expect("validated"),
            evaluations,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("evaluation", self.evaluation.to_wire_value()),
            (
                "evaluations",
                WireValue::Array(
                    self.evaluations
                        .iter()
                        .map(DomainEvaluationReport::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

/// Every stored revision of one design, in sequence order.
#[derive(Clone, Debug, PartialEq)]
pub struct RevisionListResponse {
    /// The design the revisions belong to.
    pub design_id: String,
    /// The number of stored revisions.
    pub count: BigInt,
    /// The stored revisions.
    pub revisions: Vec<RevisionRecord>,
}

impl ApiModel for RevisionListResponse {
    const NAME: &'static str = "RevisionListResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let design_id = fields.required_text("design_id");
        let count = fields.required_integer("count");
        let revisions = fields.required_model_tuple("revisions", |value, location, reading| {
            RevisionRecord::from_value_at(value, location, reading)
        });
        fields.reject_extra(&["design_id", "count", "revisions"]);
        fields.finish()?;
        Ok(Self {
            design_id: design_id.expect("validated"),
            count: count.expect("validated"),
            revisions: revisions.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("design_id", wire_text(&self.design_id)),
            ("count", WireValue::Integer(self.count.clone())),
            (
                "revisions",
                WireValue::Array(
                    self.revisions
                        .iter()
                        .map(RevisionRecord::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

/// Every recorded policy checkpoint of one workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckpointListResponse {
    /// The workflow the checkpoints belong to.
    pub workflow_id: String,
    /// The number of recorded checkpoints.
    pub count: BigInt,
    /// The checkpoint that is still pending, if one is.
    pub pending: Option<CheckpointView>,
    /// The recorded checkpoints.
    pub approvals: Vec<ApprovalRequest>,
}

impl ApiModel for CheckpointListResponse {
    const NAME: &'static str = "CheckpointListResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let workflow_id = fields.required_text("workflow_id");
        let count = fields.required_integer("count");
        let pending = fields.required_nullable_model("pending", |value, location, reading| {
            CheckpointView::from_value_at(value, location, reading)
        });
        let approvals = fields.required_model_tuple("approvals", |value, location, reading| {
            crate::views::contract_model::<ApprovalRequest>(value, location, reading)
        });
        fields.reject_extra(&["workflow_id", "count", "pending", "approvals"]);
        fields.finish()?;
        Ok(Self {
            workflow_id: workflow_id.expect("validated"),
            count: count.expect("validated"),
            pending: pending.expect("validated"),
            approvals: approvals.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("workflow_id", wire_text(&self.workflow_id)),
            ("count", WireValue::Integer(self.count.clone())),
            (
                "pending",
                match &self.pending {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "approvals",
                WireValue::Array(
                    self.approvals
                        .iter()
                        .map(ApprovalRequest::to_wire_value)
                        .collect(),
                ),
            ),
        ])
    }
}

/// One mechanical reference run or one checkpoint decision outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct RunResponse {
    /// The outcome of the run.
    pub run: MechanicalRunResult,
    /// The state of the workflow.
    pub state: WorkflowStateView,
    /// The revision the run committed, if it committed one.
    pub revision: Option<RevisionRecord>,
    /// The checkpoint the run stopped at, if it stopped at one.
    pub checkpoint: Option<CheckpointView>,
}

impl ApiModel for RunResponse {
    const NAME: &'static str = "RunResponse";

    fn from_value(value: &CanonicalValue) -> Result<Self, Failures> {
        Self::from_value_at(value, &[], Reading::Mapping)
    }

    /// Read one model at a path.
    ///
    /// # Errors
    ///
    /// Returns every failure, in the order the model declares its fields.
    fn from_value_at(
        value: &CanonicalValue,
        location: &[Location],
        reading: Reading,
    ) -> Result<Self, Failures> {
        let object = match value {
            CanonicalValue::Object(object) => object,
            other => {
                return Err(vec![match reading {
                    Reading::Mapping => {
                        crate::validation::model_type_issue(Self::NAME, location, other)
                    }
                    Reading::Attributes => crate::validation::body_type_issue(location, other),
                }]);
            }
        };
        let mut fields = Fields::new_reading_at(object, false, location, reading);
        let run = fields.required_model("run", |value, location, reading| {
            MechanicalRunResult::from_value_at(value, location, reading)
        });
        let state = fields.required_model("state", |value, location, reading| {
            WorkflowStateView::from_value_at(value, location, reading)
        });
        let revision = fields.optional_model("revision", |value, location, reading| {
            RevisionRecord::from_value_at(value, location, reading)
        });
        let checkpoint = fields.optional_model("checkpoint", |value, location, reading| {
            CheckpointView::from_value_at(value, location, reading)
        });
        fields.reject_extra(&["run", "state", "revision", "checkpoint"]);
        fields.finish()?;
        Ok(Self {
            run: run.expect("validated"),
            state: state.expect("validated"),
            revision: revision.flatten(),
            checkpoint: checkpoint.flatten(),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("run", self.run.to_wire_value()),
            ("state", self.state.to_wire_value()),
            (
                "revision",
                match &self.revision {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
            (
                "checkpoint",
                match &self.checkpoint {
                    Some(value) => value.to_wire_value(),
                    None => WireValue::Null,
                },
            ),
        ])
    }
}

/// A text value of a wire document.
fn wire_text(value: &str) -> WireValue {
    WireValue::Text(kala_contracts::Text::new(value))
}

/// An optional text value of a wire document.
fn wire_optional_text(value: &Option<String>) -> WireValue {
    match value {
        Some(value) => wire_text(value),
        None => WireValue::Null,
    }
}

/// A mapping of free values of a wire document.
///
/// The keys are sorted by code point, which is the order the canonical JSON
/// uses. The Python source keeps the order of the input; the two orders only
/// differ when the caller sends the keys of a parameter object in an order
/// that is not the sorted one.
fn wire_object(value: &Option<CanonicalObject>) -> WireValue {
    match value {
        Some(object) => WireValue::Object(
            object
                .iter()
                .map(|(key, item)| {
                    (
                        kala_contracts::Text::new(&key.to_string_lossy()),
                        wire_value(item),
                    )
                })
                .collect(),
        ),
        None => WireValue::Null,
    }
}

/// A free value of a wire document.
fn wire_value(value: &CanonicalValue) -> WireValue {
    match value {
        CanonicalValue::Null => WireValue::Null,
        CanonicalValue::Bool(flag) => WireValue::Bool(*flag),
        CanonicalValue::Integer(number) => WireValue::Integer(number.clone()),
        CanonicalValue::Float(number) => WireValue::Float(*number),
        CanonicalValue::Text(text) => {
            WireValue::Text(kala_contracts::Text::new(&text.to_string_lossy()))
        }
        CanonicalValue::Array(items) => WireValue::Array(items.iter().map(wire_value).collect()),
        CanonicalValue::Object(entries) => WireValue::Object(
            entries
                .iter()
                .map(|(key, item)| {
                    (
                        kala_contracts::Text::new(&key.to_string_lossy()),
                        wire_value(item),
                    )
                })
                .collect(),
        ),
    }
}

/// The JSON limits a request body is parsed with.
///
/// The Python transport parses a body with `json.loads`, which applies no
/// protocol limit at all: the bounded JSON domain of the tool protocol is
/// applied by the model that reads the body, not by the parser. A native
/// transport that parsed a body with the protocol limits would refuse a
/// payload the Python source refuses later, and with another failure.
///
/// The depth is bounded the way CPython's parser bounds it, and the integer
/// digit limit is the one CPython applies.
#[must_use]
pub fn request_body_limits() -> Limits {
    Limits {
        max_json_bytes: Limits::DEFAULT_MAX_JSON_BYTES,
        max_string_length: usize::MAX,
        max_collection_items: usize::MAX,
        max_key_length: usize::MAX,
        max_json_depth: 512,
        max_integer_digits: Limits::DEFAULT_MAX_INTEGER_DIGITS,
    }
}

/// The sanitized failures of a rejected body.
///
/// The location of every failure carries the `body` prefix the live API
/// writes, so the detail lines are the ones the Python source renders and the
/// error handlers turn into a 422.
#[must_use]
pub fn transport_failures(
    failures: &[crate::validation::Issue],
) -> Vec<crate::errors::ValidationFailure> {
    failures
        .iter()
        .map(|failure| crate::errors::ValidationFailure {
            location: std::iter::once("body".to_owned())
                .chain(failure.location.iter().map(|part| match part {
                    Location::Field(name) => name.clone(),
                    Location::Index(index) => index.to_string(),
                }))
                .collect(),
            message: Some(failure.message.clone()),
            kind: Some(failure.kind.clone()),
        })
        .collect()
}

/// The kinds the schemas report, kept beside the models.
pub use crate::validation::kind as schema_kind;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::kind;

    fn parse(model: &str, payload: &str) -> Result<(), Failures> {
        let value = kala_json::strict_json_loads(payload.as_bytes()).expect("valid JSON");
        let failures = match model {
            "EvaluateDomainRequest" => EvaluateDomainRequest::from_value(&value).err(),
            "CreateRevisionRequest" => CreateRevisionRequest::from_value(&value).err(),
            "PrepareRevisionRequest" => PrepareRevisionRequest::from_value(&value).err(),
            "RunMechanicalWorkflowRequest" => {
                RunMechanicalWorkflowRequest::from_value(&value).err()
            }
            "CheckpointDecisionRequest" => CheckpointDecisionRequest::from_value(&value).err(),
            other => panic!("unknown model {other}"),
        };
        match failures {
            None => Ok(()),
            Some(failures) => Err(failures),
        }
    }

    #[test]
    fn the_defaults_are_the_python_defaults() {
        let value = CanonicalValue::Object(CanonicalObject::new());
        let request = RunMechanicalWorkflowRequest::from_value(&value).unwrap();
        assert_eq!(request.design_id, "mechanical-reference");
        assert_eq!(request.actor_id, "local-user");
        assert!(request.workflow_id.is_none());
        let request = CheckpointDecisionRequest::from_value(&value).unwrap();
        assert_eq!(request.actor_id, "local-user");
        assert!(request.reason.is_none());
    }

    #[test]
    fn the_request_models_strip_the_whitespace() {
        let value = CanonicalValue::Object(
            [(
                kala_json::JsonText::new("design_id"),
                CanonicalValue::from("  a-design  "),
            )]
            .into_iter()
            .collect(),
        );
        let request = EvaluateDomainRequest::from_value(&value).unwrap();
        assert_eq!(request.design_id.as_deref(), Some("a-design"));
    }

    #[test]
    fn the_parameters_are_bounded() {
        parse(
            "EvaluateDomainRequest",
            r#"{"parameters":{"a":"b","c":"d","e":"f"}}"#,
        )
        .expect("three parameters are accepted");
        let failures = parse("EvaluateDomainRequest", r#"{"parameters":{"":"b"}}"#).unwrap_err();
        assert_eq!(failures[0].kind, kind::VALUE_ERROR);
        assert_eq!(
            failures[0].message,
            "Value error, parameter names must be nonempty and bounded"
        );
        let failures = parse(
            "EvaluateDomainRequest",
            r#"{"parameters":{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":"b"}}"#,
        )
        .unwrap_err();
        assert_eq!(failures[0].kind, kind::VALUE_ERROR);
    }

    #[test]
    fn the_parameters_are_bounded_in_size() {
        let mut parameters = String::from("{\"a\":\"");
        parameters.push_str(&"x".repeat(MAX_PARAMETERS_BYTES));
        parameters.push_str("\"}");
        let failures = parse(
            "EvaluateDomainRequest",
            &format!("{{\"parameters\":{parameters}}}"),
        )
        .unwrap_err();
        assert_eq!(failures[0].kind, kind::VALUE_ERROR);
        assert_eq!(
            failures[0].message,
            "Value error, canonical JSON exceeds the bounded size"
        );
    }

    #[test]
    fn an_undeclared_field_is_refused() {
        let failures = parse(
            "CreateRevisionRequest",
            r#"{"design_id":"a","domain":"building","x":1}"#,
        )
        .unwrap_err();
        assert_eq!(failures[0].kind, kind::EXTRA_FORBIDDEN);
        assert_eq!(failures[0].message, "Extra inputs are not permitted");
    }

    #[test]
    fn a_missing_field_is_reported() {
        let failures = parse("CreateRevisionRequest", r#"{"domain":"building"}"#).unwrap_err();
        assert_eq!(failures[0].kind, kind::MISSING);
        assert_eq!(failures[0].message, "Field required");
    }

    #[test]
    fn the_domain_enum_is_reported_the_way_pydantic_reports_it() {
        let failures = parse(
            "CreateRevisionRequest",
            r#"{"design_id":"a","domain":"nope"}"#,
        )
        .unwrap_err();
        assert_eq!(failures[0].kind, kind::ENUM);
        assert_eq!(
            failures[0].message,
            "Input should be 'building', 'mechanical' or 'wing'"
        );
    }
}
