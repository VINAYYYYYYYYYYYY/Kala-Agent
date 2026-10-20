//! The design, workflow, tool, evaluation, approval and budget contracts.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use kala_json::{CanonicalObject, CanonicalValue};

use crate::error::{ContractError, ContractIssue};
use crate::scalar::{
    ArtifactUri, CanonicalJsonObject, FiniteNonNegativeFloat, Identifier, MediaType,
    NonNegativeDecimal, NonNegativeInteger, Sha256Digest, UnitInterval, as_artifact_uri,
    as_bool_strict, as_canonical_object, as_decimal_nonnegative, as_digest, as_float_nonnegative,
    as_identifier, as_media_type, as_nonempty_text, as_nonnegative_integer, as_text, as_timestamp,
    as_unit_interval,
};
use crate::text::Text;
use crate::timestamp::Timestamp;
use crate::wire::{ContractModel, WireValue, key};

macro_rules! contract_enum {
    ($name:ident, $model:literal, { $($variant:ident => $value:literal),+ $(,)? }) => {
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

            /// Parse an exact Python enum value.
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $( $value => Some($name::$variant), )+
                    _ => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value).ok_or_else(|| {
                    ContractError::new(
                        $model,
                        concat!("Input should be a valid ", $model, " value"),
                    )
                })
            }
        }
    };
}

contract_enum!(RequirementKind, "RequirementKind", {
    Functional => "functional",
    NonFunctional => "non_functional",
    Constraint => "constraint",
    Safety => "safety",
    Quality => "quality",
    Performance => "performance",
});

contract_enum!(RequirementPriority, "RequirementPriority", {
    Low => "low",
    Medium => "medium",
    High => "high",
    Critical => "critical",
});

contract_enum!(RequirementStatus, "RequirementStatus", {
    Draft => "draft",
    Proposed => "proposed",
    Approved => "approved",
    Rejected => "rejected",
    Implemented => "implemented",
    Verified => "verified",
});

contract_enum!(SideEffectLevel, "SideEffectLevel", {
    None => "none",
    Read => "read",
    LocalWrite => "local_write",
    Network => "network",
    Egress => "egress",
});

contract_enum!(WorkflowStatus, "WorkflowStatus", {
    Created => "created",
    Running => "running",
    WaitingForApproval => "waiting_for_approval",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
});

contract_enum!(StepStatus, "StepStatus", {
    Pending => "pending",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Skipped => "skipped",
    Cancelled => "cancelled",
});

contract_enum!(ToolRunStatus, "ToolRunStatus", {
    Requested => "requested",
    Running => "running",
    Succeeded => "succeeded",
    Failed => "failed",
    Denied => "denied",
    TimedOut => "timed_out",
    Cancelled => "cancelled",
});

contract_enum!(ApprovalKind, "ApprovalKind", {
    PlanApproval => "plan_approval",
    ToolClassChange => "tool_class_change",
    Egress => "egress",
    PreCommit => "pre_commit",
    Export => "export",
    ProviderDegraded => "provider_degraded",
});

contract_enum!(ApprovalStatus, "ApprovalStatus", {
    Pending => "pending",
    Approved => "approved",
    Rejected => "rejected",
    Aborted => "aborted",
    Expired => "expired",
    Invalidated => "invalidated",
});

contract_enum!(ArtifactClassification, "ArtifactClassification", {
    Metadata => "metadata",
    Text => "text",
    DerivedData => "derived_data",
    DesignBinary => "design_binary",
});

trait ParsedEnum: Sized {
    fn parse_enum(value: &CanonicalValue) -> Result<Self, String>;
}

macro_rules! enum_parser {
    ($name:ident) => {
        impl ParsedEnum for $name {
            fn parse_enum(value: &CanonicalValue) -> Result<Self, String> {
                let text =
                    as_text(value).ok_or_else(|| "Input should be a valid string".to_owned())?;
                let utf8 = text
                    .to_utf8()
                    .map_err(|_| "Input should be a valid string".to_owned())?;
                Self::parse(&utf8).ok_or_else(|| {
                    let values = Self::ALL
                        .iter()
                        .map(|value| format!("'{}'", value.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Input should be one of {values}")
                })
            }
        }
    };
}

enum_parser!(RequirementKind);
enum_parser!(RequirementPriority);
enum_parser!(RequirementStatus);
enum_parser!(SideEffectLevel);
enum_parser!(WorkflowStatus);
enum_parser!(StepStatus);
enum_parser!(ToolRunStatus);
enum_parser!(ApprovalKind);
enum_parser!(ApprovalStatus);
enum_parser!(ArtifactClassification);

// ---------------------------------------------------------------------------
// Parsing helpers.
// ---------------------------------------------------------------------------

fn object<'a>(
    value: &'a CanonicalValue,
    model: &'static str,
) -> Result<&'a CanonicalObject, ContractError> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(ContractError::new(
            model,
            "Input should be a valid dictionary or object to extract fields from",
        )),
    }
}

fn push(issues: &mut Vec<ContractIssue>, field: &str, message: impl Into<String>) {
    issues.push(ContractIssue::new(field, message));
}

fn push_nested(issues: &mut Vec<ContractIssue>, field: &str, error: ContractError) {
    for issue in error.issues() {
        issues.push(issue.clone().under(field));
    }
}

fn required<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
        None => {
            push(issues, field, "Field required");
            None
        }
    }
}

fn optional<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Option<T> {
    match map.get(&key(field)) {
        None | Some(CanonicalValue::Null) => None,
        Some(value) => match parse(value) {
            Ok(value) => Some(value),
            Err(message) => {
                push(issues, field, message);
                None
            }
        },
    }
}

fn defaulted<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    default: impl FnOnce() -> T,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> T {
    match map.get(&key(field)) {
        None => default(),
        Some(value) => match parse(value) {
            Ok(value) => value,
            Err(message) => {
                push(issues, field, message);
                default()
            }
        },
    }
}

fn array_required<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Vec<T> {
    if !map.contains_key(&key(field)) {
        push(issues, field, "Field required");
        return Vec::new();
    }
    array(map, field, issues, parse)
}

fn array<T>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
    parse: impl Fn(&CanonicalValue) -> Result<T, String>,
) -> Vec<T> {
    match map.get(&key(field)) {
        None => Vec::new(),
        Some(CanonicalValue::Array(values)) => {
            let mut parsed = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                match parse(value) {
                    Ok(value) => parsed.push(value),
                    Err(message) => push(issues, &format!("{field}[{index}]"), message),
                }
            }
            parsed
        }
        Some(_) => {
            push(issues, field, "Input should be a valid tuple");
            Vec::new()
        }
    }
}

fn model_required<T: ContractModel>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Option<T> {
    match map.get(&key(field)) {
        Some(value) => match T::from_value(value) {
            Ok(value) => Some(value),
            Err(error) => {
                push_nested(issues, field, error);
                None
            }
        },
        None => {
            push(issues, field, "Field required");
            None
        }
    }
}

fn model_array<T: ContractModel>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Vec<T> {
    match map.get(&key(field)) {
        None => Vec::new(),
        Some(CanonicalValue::Array(values)) => {
            let mut parsed = Vec::with_capacity(values.len());
            for (index, value) in values.iter().enumerate() {
                match T::from_value(value) {
                    Ok(value) => parsed.push(value),
                    Err(error) => push_nested(issues, &format!("{field}[{index}]"), error),
                }
            }
            parsed
        }
        Some(_) => {
            push(issues, field, "Input should be a valid tuple");
            Vec::new()
        }
    }
}

fn reject_extra(map: &CanonicalObject, allowed: &[&str], issues: &mut Vec<ContractIssue>) {
    for name in map.keys() {
        let rendered = name.to_string_lossy();
        if !allowed.contains(&rendered.as_str()) {
            push(issues, &rendered, "Extra inputs are not permitted");
        }
    }
}

fn canonicalize_identifiers(
    mut values: Vec<Identifier>,
    issues: &mut Vec<ContractIssue>,
    field: &str,
    sort: bool,
) -> Vec<Identifier> {
    if sort {
        values.sort();
    }
    if has_duplicates(&values) {
        push(issues, field, "collection values must be unique");
    }
    values
}

fn canonicalize_texts(
    mut values: Vec<Text>,
    issues: &mut Vec<ContractIssue>,
    field: &str,
    sort: bool,
) -> Vec<Text> {
    if sort {
        values.sort();
    }
    if has_duplicates(&values) {
        push(issues, field, "collection values must be unique");
    }
    values
}

fn has_duplicates<T: Eq + std::hash::Hash>(values: &[T]) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(values.len());
    values.iter().any(|value| !seen.insert(value))
}

fn unique_digests(
    values: Vec<Sha256Digest>,
    issues: &mut Vec<ContractIssue>,
    field: &str,
) -> Vec<Sha256Digest> {
    for (index, digest) in values.iter().enumerate() {
        if values[..index].contains(digest) {
            push(issues, field, "collection values must be unique");
            break;
        }
    }
    values
}

fn unique_by<T>(
    values: &[T],
    key_of: impl Fn(&T) -> String,
    issues: &mut Vec<ContractIssue>,
    field: &str,
) {
    let keys: Vec<String> = values.iter().map(key_of).collect();
    for index in 0..keys.len() {
        if keys[..index].contains(&keys[index]) {
            push(issues, field, "collection values must be unique");
            break;
        }
    }
}

fn enum_field<E: ParsedEnum>(
    map: &CanonicalObject,
    field: &str,
    issues: &mut Vec<ContractIssue>,
) -> Option<E> {
    required(map, field, issues, E::parse_enum)
}

// ---------------------------------------------------------------------------
// Requirement
// ---------------------------------------------------------------------------

/// A single product or design requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct Requirement {
    id: Identifier,
    title: Text,
    kind: RequirementKind,
    description: Option<Text>,
    priority: RequirementPriority,
    status: RequirementStatus,
    acceptance_criteria: Vec<Text>,
    tags: Vec<Identifier>,
}

impl ContractModel for Requirement {
    const NAME: &'static str = "Requirement";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let id = required(map, "id", &mut issues, as_identifier);
        let title = required(map, "title", &mut issues, as_nonempty_text);
        let kind = enum_field(map, "kind", &mut issues);
        let description = optional(map, "description", &mut issues, as_nonempty_text);
        let priority = defaulted(
            map,
            "priority",
            &mut issues,
            || RequirementPriority::Medium,
            RequirementPriority::parse_enum,
        );
        let status = defaulted(
            map,
            "status",
            &mut issues,
            || RequirementStatus::Draft,
            RequirementStatus::parse_enum,
        );
        let acceptance_criteria = array(map, "acceptance_criteria", &mut issues, as_nonempty_text);
        let tags = array(map, "tags", &mut issues, as_identifier);
        let tags = canonicalize_identifiers(tags, &mut issues, "tags", true);
        reject_extra(
            map,
            &[
                "id",
                "title",
                "kind",
                "description",
                "priority",
                "status",
                "acceptance_criteria",
                "tags",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            id: id.expect("validated"),
            title: title.expect("validated"),
            kind: kind.expect("validated"),
            description,
            priority,
            status,
            acceptance_criteria,
            tags,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("id", text(self.id.as_str())),
            ("title", text_value(&self.title)),
            ("kind", text(self.kind.as_str())),
            ("description", opt_text(&self.description)),
            ("priority", text(self.priority.as_str())),
            ("status", text(self.status.as_str())),
            ("acceptance_criteria", texts(&self.acceptance_criteria)),
            ("tags", identifiers(&self.tags)),
        ])
    }
}

impl Requirement {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }
    pub fn title(&self) -> &Text {
        &self.title
    }
    pub fn kind(&self) -> RequirementKind {
        self.kind
    }
    pub fn description(&self) -> Option<&Text> {
        self.description.as_ref()
    }
    pub fn priority(&self) -> RequirementPriority {
        self.priority
    }
    pub fn status(&self) -> RequirementStatus {
        self.status
    }
    pub fn acceptance_criteria(&self) -> &[Text] {
        &self.acceptance_criteria
    }
    pub fn tags(&self) -> &[Identifier] {
        &self.tags
    }
}

// ---------------------------------------------------------------------------
// DesignSpec
// ---------------------------------------------------------------------------

/// The declarative state of a design.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignSpec {
    id: Identifier,
    name: Text,
    summary: Text,
    parameter_schema_digest: Option<Sha256Digest>,
    attributes_json: CanonicalJsonObject,
    requirement_ids: Vec<Identifier>,
    constraints: Vec<Text>,
}

impl ContractModel for DesignSpec {
    const NAME: &'static str = "DesignSpec";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let id = required(map, "id", &mut issues, as_identifier);
        let name = required(map, "name", &mut issues, as_nonempty_text);
        let summary = required(map, "summary", &mut issues, as_nonempty_text);
        let parameter_schema_digest =
            optional(map, "parameter_schema_digest", &mut issues, as_digest);
        let attributes_json = defaulted(
            map,
            "attributes_json",
            &mut issues,
            CanonicalJsonObject::empty,
            as_canonical_object,
        );
        let requirement_ids = array(map, "requirement_ids", &mut issues, as_identifier);
        let requirement_ids =
            canonicalize_identifiers(requirement_ids, &mut issues, "requirement_ids", true);
        let constraints = array(map, "constraints", &mut issues, as_nonempty_text);
        let constraints = canonicalize_texts(constraints, &mut issues, "constraints", false);
        reject_extra(
            map,
            &[
                "id",
                "name",
                "summary",
                "parameter_schema_digest",
                "attributes_json",
                "requirement_ids",
                "constraints",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            id: id.expect("validated"),
            name: name.expect("validated"),
            summary: summary.expect("validated"),
            parameter_schema_digest,
            attributes_json,
            requirement_ids,
            constraints,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("id", text(self.id.as_str())),
            ("name", text_value(&self.name)),
            ("summary", text_value(&self.summary)),
            (
                "parameter_schema_digest",
                opt_digest(&self.parameter_schema_digest),
            ),
            ("attributes_json", text(self.attributes_json.as_str())),
            ("requirement_ids", identifiers(&self.requirement_ids)),
            ("constraints", texts(&self.constraints)),
        ])
    }
}

impl DesignSpec {
    pub fn id(&self) -> &str {
        self.id.as_str()
    }
    pub fn name(&self) -> &Text {
        &self.name
    }
    pub fn summary(&self) -> &Text {
        &self.summary
    }
    pub fn parameter_schema_digest(&self) -> Option<Sha256Digest> {
        self.parameter_schema_digest
    }
    pub fn attributes_json(&self) -> &CanonicalJsonObject {
        &self.attributes_json
    }
    pub fn requirement_ids(&self) -> &[Identifier] {
        &self.requirement_ids
    }
    pub fn constraints(&self) -> &[Text] {
        &self.constraints
    }
}

// ---------------------------------------------------------------------------
// DomainPackageManifest
// ---------------------------------------------------------------------------

/// A signed domain package manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct DomainPackageManifest {
    package_id: Identifier,
    version: Text,
    digest: Sha256Digest,
    description: Option<Text>,
    workflow_definition_ids: Vec<Identifier>,
    tool_ids: Vec<Identifier>,
    schema_digests: Vec<Sha256Digest>,
    required_capabilities: Vec<Identifier>,
    policy_overlay_digest: Option<Sha256Digest>,
    signer_id: Option<Identifier>,
    signature_digest: Option<Sha256Digest>,
}

impl ContractModel for DomainPackageManifest {
    const NAME: &'static str = "DomainPackageManifest";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let package_id = required(map, "package_id", &mut issues, as_identifier);
        let version = required(map, "version", &mut issues, as_nonempty_text);
        let digest = required(map, "digest", &mut issues, as_digest);
        let description = optional(map, "description", &mut issues, as_nonempty_text);
        let workflow_definition_ids =
            array(map, "workflow_definition_ids", &mut issues, as_identifier);
        let workflow_definition_ids = canonicalize_identifiers(
            workflow_definition_ids,
            &mut issues,
            "workflow_definition_ids",
            true,
        );
        let tool_ids = array(map, "tool_ids", &mut issues, as_identifier);
        let tool_ids = canonicalize_identifiers(tool_ids, &mut issues, "tool_ids", true);
        let schema_digests = array(map, "schema_digests", &mut issues, as_digest);
        let schema_digests = unique_digests(schema_digests, &mut issues, "schema_digests");
        let required_capabilities = array(map, "required_capabilities", &mut issues, as_identifier);
        let required_capabilities = canonicalize_identifiers(
            required_capabilities,
            &mut issues,
            "required_capabilities",
            true,
        );
        let policy_overlay_digest = optional(map, "policy_overlay_digest", &mut issues, as_digest);
        let signer_id = optional(map, "signer_id", &mut issues, as_identifier);
        let signature_digest = optional(map, "signature_digest", &mut issues, as_digest);
        reject_extra(
            map,
            &[
                "package_id",
                "version",
                "digest",
                "description",
                "workflow_definition_ids",
                "tool_ids",
                "schema_digests",
                "required_capabilities",
                "policy_overlay_digest",
                "signer_id",
                "signature_digest",
            ],
            &mut issues,
        );
        if issues.is_empty() && signer_id.is_some() != signature_digest.is_some() {
            push(
                &mut issues,
                "",
                "signer_id and signature_digest must be provided together",
            );
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            package_id: package_id.expect("validated"),
            version: version.expect("validated"),
            digest: digest.expect("validated"),
            description,
            workflow_definition_ids,
            tool_ids,
            schema_digests,
            required_capabilities,
            policy_overlay_digest,
            signer_id,
            signature_digest,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("package_id", text(self.package_id.as_str())),
            ("version", text_value(&self.version)),
            ("digest", digest(self.digest)),
            ("description", opt_text(&self.description)),
            (
                "workflow_definition_ids",
                identifiers(&self.workflow_definition_ids),
            ),
            ("tool_ids", identifiers(&self.tool_ids)),
            ("schema_digests", digests(&self.schema_digests)),
            (
                "required_capabilities",
                identifiers(&self.required_capabilities),
            ),
            (
                "policy_overlay_digest",
                opt_digest(&self.policy_overlay_digest),
            ),
            ("signer_id", opt_identifier(&self.signer_id)),
            ("signature_digest", opt_digest(&self.signature_digest)),
        ])
    }
}

impl DomainPackageManifest {
    pub fn package_id(&self) -> &str {
        self.package_id.as_str()
    }
    pub fn version(&self) -> &Text {
        &self.version
    }
    pub fn digest(&self) -> Sha256Digest {
        self.digest
    }
    pub fn description(&self) -> Option<&Text> {
        self.description.as_ref()
    }
    pub fn workflow_definition_ids(&self) -> &[Identifier] {
        &self.workflow_definition_ids
    }
    pub fn tool_ids(&self) -> &[Identifier] {
        &self.tool_ids
    }
    pub fn schema_digests(&self) -> &[Sha256Digest] {
        &self.schema_digests
    }
    pub fn required_capabilities(&self) -> &[Identifier] {
        &self.required_capabilities
    }
    pub fn policy_overlay_digest(&self) -> Option<Sha256Digest> {
        self.policy_overlay_digest
    }
    pub fn signer_id(&self) -> Option<&str> {
        self.signer_id.as_ref().map(Identifier::as_str)
    }
    pub fn signature_digest(&self) -> Option<Sha256Digest> {
        self.signature_digest
    }
}

// ---------------------------------------------------------------------------
// ArtifactRef
// ---------------------------------------------------------------------------

/// A reference to a content-addressed artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactRef {
    artifact_id: Identifier,
    uri: ArtifactUri,
    digest: Sha256Digest,
    media_type: MediaType,
    size_bytes: NonNegativeInteger,
    classification: ArtifactClassification,
    preview_uri: Option<ArtifactUri>,
    provenance_id: Option<Identifier>,
}

impl ContractModel for ArtifactRef {
    const NAME: &'static str = "ArtifactRef";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let artifact_id = required(map, "artifact_id", &mut issues, as_identifier);
        let uri = required(map, "uri", &mut issues, as_artifact_uri);
        let digest = required(map, "digest", &mut issues, as_digest);
        let media_type = required(map, "media_type", &mut issues, as_media_type);
        let size_bytes = required(map, "size_bytes", &mut issues, as_nonnegative_integer);
        let classification = defaulted(
            map,
            "classification",
            &mut issues,
            || ArtifactClassification::Metadata,
            ArtifactClassification::parse_enum,
        );
        let preview_uri = optional(map, "preview_uri", &mut issues, as_artifact_uri);
        let provenance_id = optional(map, "provenance_id", &mut issues, as_identifier);
        reject_extra(
            map,
            &[
                "artifact_id",
                "uri",
                "digest",
                "media_type",
                "size_bytes",
                "classification",
                "preview_uri",
                "provenance_id",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            artifact_id: artifact_id.expect("validated"),
            uri: uri.expect("validated"),
            digest: digest.expect("validated"),
            media_type: media_type.expect("validated"),
            size_bytes: size_bytes.expect("validated"),
            classification,
            preview_uri,
            provenance_id,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("artifact_id", text(self.artifact_id.as_str())),
            ("uri", text_value(self.uri.as_text())),
            ("digest", digest(self.digest)),
            ("media_type", text(self.media_type.as_str())),
            ("size_bytes", integer(&self.size_bytes)),
            ("classification", text(self.classification.as_str())),
            (
                "preview_uri",
                self.preview_uri
                    .as_ref()
                    .map_or(WireValue::Null, |uri| text_value(uri.as_text())),
            ),
            ("provenance_id", opt_identifier(&self.provenance_id)),
        ])
    }
}

impl ArtifactRef {
    pub fn artifact_id(&self) -> &str {
        self.artifact_id.as_str()
    }
    pub fn uri(&self) -> &ArtifactUri {
        &self.uri
    }
    pub fn digest(&self) -> Sha256Digest {
        self.digest
    }
    pub fn media_type(&self) -> &str {
        self.media_type.as_str()
    }
    pub fn size_bytes(&self) -> &NonNegativeInteger {
        &self.size_bytes
    }
    pub fn classification(&self) -> ArtifactClassification {
        self.classification
    }
    pub fn preview_uri(&self) -> Option<&ArtifactUri> {
        self.preview_uri.as_ref()
    }
    pub fn provenance_id(&self) -> Option<&str> {
        self.provenance_id.as_ref().map(Identifier::as_str)
    }
}

// ---------------------------------------------------------------------------
// DesignRevision
// ---------------------------------------------------------------------------

/// One immutable revision of a design.
#[derive(Clone, Debug, PartialEq)]
pub struct DesignRevision {
    revision_id: Identifier,
    design_id: Identifier,
    sequence: NonNegativeInteger,
    spec: DesignSpec,
    parent_revision_id: Option<Identifier>,
    artifacts: Vec<ArtifactRef>,
    domain_packages: Vec<DomainPackageManifest>,
    provenance_id: Option<Identifier>,
    created_at: Timestamp,
}

impl ContractModel for DesignRevision {
    const NAME: &'static str = "DesignRevision";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let revision_id = required(map, "revision_id", &mut issues, as_identifier);
        let design_id = required(map, "design_id", &mut issues, as_identifier);
        let sequence = required(map, "sequence", &mut issues, as_nonnegative_integer);
        let spec = model_required::<DesignSpec>(map, "spec", &mut issues);
        let parent_revision_id = optional(map, "parent_revision_id", &mut issues, as_identifier);
        let artifacts = model_array::<ArtifactRef>(map, "artifacts", &mut issues);
        let domain_packages =
            model_array::<DomainPackageManifest>(map, "domain_packages", &mut issues);
        let provenance_id = optional(map, "provenance_id", &mut issues, as_identifier);
        let created_at = required(map, "created_at", &mut issues, as_timestamp);
        unique_by(
            &artifacts,
            |artifact| artifact.artifact_id().to_owned(),
            &mut issues,
            "artifacts",
        );
        unique_by(
            &domain_packages,
            |package| package.package_id().to_owned(),
            &mut issues,
            "domain_packages",
        );
        reject_extra(
            map,
            &[
                "revision_id",
                "design_id",
                "sequence",
                "spec",
                "parent_revision_id",
                "artifacts",
                "domain_packages",
                "provenance_id",
                "created_at",
            ],
            &mut issues,
        );
        if issues.is_empty()
            && parent_revision_id.as_ref().map(Identifier::as_str)
                == revision_id.as_ref().map(Identifier::as_str)
        {
            push(
                &mut issues,
                "parent_revision_id",
                "a revision cannot be its own parent",
            );
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            revision_id: revision_id.expect("validated"),
            design_id: design_id.expect("validated"),
            sequence: sequence.expect("validated"),
            spec: spec.expect("validated"),
            parent_revision_id,
            artifacts,
            domain_packages,
            provenance_id,
            created_at: created_at.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("revision_id", text(self.revision_id.as_str())),
            ("design_id", text(self.design_id.as_str())),
            ("sequence", integer(&self.sequence)),
            ("spec", self.spec.to_wire_value()),
            (
                "parent_revision_id",
                opt_identifier(&self.parent_revision_id),
            ),
            (
                "artifacts",
                WireValue::Array(
                    self.artifacts
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            (
                "domain_packages",
                WireValue::Array(
                    self.domain_packages
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            ("provenance_id", opt_identifier(&self.provenance_id)),
            ("created_at", timestamp(&self.created_at)),
        ])
    }
}

impl DesignRevision {
    pub fn revision_id(&self) -> &str {
        self.revision_id.as_str()
    }
    pub fn design_id(&self) -> &str {
        self.design_id.as_str()
    }
    pub fn sequence(&self) -> &NonNegativeInteger {
        &self.sequence
    }
    pub fn spec(&self) -> &DesignSpec {
        &self.spec
    }
    pub fn parent_revision_id(&self) -> Option<&str> {
        self.parent_revision_id.as_ref().map(Identifier::as_str)
    }
    pub fn artifacts(&self) -> &[ArtifactRef] {
        &self.artifacts
    }
    pub fn domain_packages(&self) -> &[DomainPackageManifest] {
        &self.domain_packages
    }
    pub fn provenance_id(&self) -> Option<&str> {
        self.provenance_id.as_ref().map(Identifier::as_str)
    }
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

// ---------------------------------------------------------------------------
// PlanStep
// ---------------------------------------------------------------------------

/// One planned step of a workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanStep {
    step_id: Identifier,
    catalog_step_id: Identifier,
    intent: Text,
    parameters_json: CanonicalJsonObject,
    input_artifact_ids: Vec<Identifier>,
    required_capabilities: Vec<Identifier>,
    depends_on: Vec<Identifier>,
    estimated_duration_seconds: Option<FiniteNonNegativeFloat>,
    max_attempts: NonNegativeInteger,
}

impl ContractModel for PlanStep {
    const NAME: &'static str = "PlanStep";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let step_id = required(map, "step_id", &mut issues, as_identifier);
        let catalog_step_id = required(map, "catalog_step_id", &mut issues, as_identifier);
        let intent = required(map, "intent", &mut issues, as_nonempty_text);
        let parameters_json = defaulted(
            map,
            "parameters_json",
            &mut issues,
            CanonicalJsonObject::empty,
            as_canonical_object,
        );
        let input_artifact_ids = array(map, "input_artifact_ids", &mut issues, as_identifier);
        let input_artifact_ids =
            canonicalize_identifiers(input_artifact_ids, &mut issues, "input_artifact_ids", true);
        let required_capabilities = array(map, "required_capabilities", &mut issues, as_identifier);
        let required_capabilities = canonicalize_identifiers(
            required_capabilities,
            &mut issues,
            "required_capabilities",
            true,
        );
        let depends_on = array(map, "depends_on", &mut issues, as_identifier);
        let depends_on = canonicalize_identifiers(depends_on, &mut issues, "depends_on", true);
        let estimated_duration_seconds = optional(
            map,
            "estimated_duration_seconds",
            &mut issues,
            as_float_nonnegative,
        );
        let max_attempts = defaulted(
            map,
            "max_attempts",
            &mut issues,
            || NonNegativeInteger::from_u64(1),
            as_nonnegative_integer,
        );
        reject_extra(
            map,
            &[
                "step_id",
                "catalog_step_id",
                "intent",
                "parameters_json",
                "input_artifact_ids",
                "required_capabilities",
                "depends_on",
                "estimated_duration_seconds",
                "max_attempts",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            if let Some(step_id) = &step_id {
                if depends_on
                    .iter()
                    .any(|dependency| dependency.as_str() == step_id.as_str())
                {
                    push(
                        &mut issues,
                        "depends_on",
                        "a plan step cannot depend on itself",
                    );
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            step_id: step_id.expect("validated"),
            catalog_step_id: catalog_step_id.expect("validated"),
            intent: intent.expect("validated"),
            parameters_json,
            input_artifact_ids,
            required_capabilities,
            depends_on,
            estimated_duration_seconds,
            max_attempts,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("step_id", text(self.step_id.as_str())),
            ("catalog_step_id", text(self.catalog_step_id.as_str())),
            ("intent", text_value(&self.intent)),
            ("parameters_json", text(self.parameters_json.as_str())),
            ("input_artifact_ids", identifiers(&self.input_artifact_ids)),
            (
                "required_capabilities",
                identifiers(&self.required_capabilities),
            ),
            ("depends_on", identifiers(&self.depends_on)),
            (
                "estimated_duration_seconds",
                opt_float(self.estimated_duration_seconds),
            ),
            ("max_attempts", integer(&self.max_attempts)),
        ])
    }
}

impl PlanStep {
    pub fn step_id(&self) -> &str {
        self.step_id.as_str()
    }
    pub fn catalog_step_id(&self) -> &str {
        self.catalog_step_id.as_str()
    }
    pub fn intent(&self) -> &Text {
        &self.intent
    }
    pub fn parameters_json(&self) -> &CanonicalJsonObject {
        &self.parameters_json
    }
    pub fn input_artifact_ids(&self) -> &[Identifier] {
        &self.input_artifact_ids
    }
    pub fn required_capabilities(&self) -> &[Identifier] {
        &self.required_capabilities
    }
    pub fn depends_on(&self) -> &[Identifier] {
        &self.depends_on
    }
    pub fn estimated_duration_seconds(&self) -> Option<FiniteNonNegativeFloat> {
        self.estimated_duration_seconds
    }
    pub fn max_attempts(&self) -> &NonNegativeInteger {
        &self.max_attempts
    }
}

// ---------------------------------------------------------------------------
// WorkflowTransition / WorkflowDefinition
// ---------------------------------------------------------------------------

/// A state-machine edge.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTransition {
    transition_id: Identifier,
    from_state: Identifier,
    to_state: Identifier,
    step_id: Identifier,
    checkpoint_type: Option<ApprovalKind>,
}

impl ContractModel for WorkflowTransition {
    const NAME: &'static str = "WorkflowTransition";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let transition_id = required(map, "transition_id", &mut issues, as_identifier);
        let from_state = required(map, "from_state", &mut issues, as_identifier);
        let to_state = required(map, "to_state", &mut issues, as_identifier);
        let step_id = required(map, "step_id", &mut issues, as_identifier);
        let checkpoint_type = optional(
            map,
            "checkpoint_type",
            &mut issues,
            ApprovalKind::parse_enum,
        );
        reject_extra(
            map,
            &[
                "transition_id",
                "from_state",
                "to_state",
                "step_id",
                "checkpoint_type",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            transition_id: transition_id.expect("validated"),
            from_state: from_state.expect("validated"),
            to_state: to_state.expect("validated"),
            step_id: step_id.expect("validated"),
            checkpoint_type,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("transition_id", text(self.transition_id.as_str())),
            ("from_state", text(self.from_state.as_str())),
            ("to_state", text(self.to_state.as_str())),
            ("step_id", text(self.step_id.as_str())),
            (
                "checkpoint_type",
                self.checkpoint_type
                    .map_or(WireValue::Null, |value| text(value.as_str())),
            ),
        ])
    }
}

impl WorkflowTransition {
    pub fn transition_id(&self) -> &str {
        self.transition_id.as_str()
    }
    pub fn from_state(&self) -> &str {
        self.from_state.as_str()
    }
    pub fn to_state(&self) -> &str {
        self.to_state.as_str()
    }
    pub fn step_id(&self) -> &str {
        self.step_id.as_str()
    }
    pub fn checkpoint_type(&self) -> Option<ApprovalKind> {
        self.checkpoint_type
    }
}

/// A complete workflow state machine and step catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowDefinition {
    definition_id: Identifier,
    version: Text,
    name: Text,
    description: Option<Text>,
    initial_state: Identifier,
    states: Vec<Identifier>,
    transitions: Vec<WorkflowTransition>,
    available_step_ids: Vec<Identifier>,
    max_replanning_attempts: NonNegativeInteger,
}

impl ContractModel for WorkflowDefinition {
    const NAME: &'static str = "WorkflowDefinition";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let definition_id = required(map, "definition_id", &mut issues, as_identifier);
        let version = required(map, "version", &mut issues, as_nonempty_text);
        let name = required(map, "name", &mut issues, as_nonempty_text);
        let description = optional(map, "description", &mut issues, as_nonempty_text);
        let initial_state = required(map, "initial_state", &mut issues, as_identifier);
        let states = array_required(map, "states", &mut issues, as_identifier);
        let states = canonicalize_identifiers(states, &mut issues, "states", true);
        let transitions = model_array::<WorkflowTransition>(map, "transitions", &mut issues);
        unique_by(
            &transitions,
            |transition| transition.transition_id().to_owned(),
            &mut issues,
            "transitions",
        );
        for index in 0..transitions.len() {
            let current = &transitions[index];
            if transitions[..index].iter().any(|previous| {
                previous.from_state() == current.from_state()
                    && previous.to_state() == current.to_state()
                    && previous.step_id() == current.step_id()
            }) {
                push(
                    &mut issues,
                    "transitions",
                    "workflow transitions must be unique",
                );
                break;
            }
        }
        let available_step_ids =
            array_required(map, "available_step_ids", &mut issues, as_identifier);
        let available_step_ids =
            canonicalize_identifiers(available_step_ids, &mut issues, "available_step_ids", true);
        let max_replanning_attempts = defaulted(
            map,
            "max_replanning_attempts",
            &mut issues,
            || NonNegativeInteger::from_u64(1),
            as_nonnegative_integer,
        );
        reject_extra(
            map,
            &[
                "definition_id",
                "version",
                "name",
                "description",
                "initial_state",
                "states",
                "transitions",
                "available_step_ids",
                "max_replanning_attempts",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            let initial = initial_state.as_ref().expect("validated");
            if states.is_empty() {
                push(&mut issues, "states", "states cannot be empty");
            } else if !states.iter().any(|state| state == initial) {
                push(
                    &mut issues,
                    "initial_state",
                    "initial_state must be declared in states",
                );
            }
            let state_set: BTreeSet<&str> = states.iter().map(|state| state.as_str()).collect();
            let step_set: BTreeSet<&str> = available_step_ids
                .iter()
                .map(|step| step.as_str())
                .collect();
            for transition in &transitions {
                if !state_set.contains(transition.from_state())
                    || !state_set.contains(transition.to_state())
                {
                    push(
                        &mut issues,
                        "transitions",
                        "transitions can only reference declared states",
                    );
                    break;
                }
                if !step_set.contains(transition.step_id()) {
                    push(
                        &mut issues,
                        "transitions",
                        "transitions can only reference available steps",
                    );
                    break;
                }
            }
            if states.is_empty() {
                // The Python validator raises before the reachability walk.
            } else {
                let mut reachable = BTreeSet::new();
                reachable.insert(initial.as_str());
                let mut pending = vec![initial.as_str()];
                while let Some(current) = pending.pop() {
                    for transition in &transitions {
                        if transition.from_state() == current
                            && !reachable.contains(transition.to_state())
                        {
                            reachable.insert(transition.to_state());
                            pending.push(transition.to_state());
                        }
                    }
                }
                if states
                    .iter()
                    .any(|state| !reachable.contains(state.as_str()))
                {
                    push(
                        &mut issues,
                        "states",
                        "all workflow states must be reachable",
                    );
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            definition_id: definition_id.expect("validated"),
            version: version.expect("validated"),
            name: name.expect("validated"),
            description,
            initial_state: initial_state.expect("validated"),
            states,
            transitions,
            available_step_ids,
            max_replanning_attempts,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("definition_id", text(self.definition_id.as_str())),
            ("version", text_value(&self.version)),
            ("name", text_value(&self.name)),
            ("description", opt_text(&self.description)),
            ("initial_state", text(self.initial_state.as_str())),
            ("states", identifiers(&self.states)),
            (
                "transitions",
                WireValue::Array(
                    self.transitions
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            ("available_step_ids", identifiers(&self.available_step_ids)),
            (
                "max_replanning_attempts",
                integer(&self.max_replanning_attempts),
            ),
        ])
    }
}

impl WorkflowDefinition {
    pub fn definition_id(&self) -> &str {
        self.definition_id.as_str()
    }
    pub fn version(&self) -> &Text {
        &self.version
    }
    pub fn name(&self) -> &Text {
        &self.name
    }
    pub fn description(&self) -> Option<&Text> {
        self.description.as_ref()
    }
    pub fn initial_state(&self) -> &str {
        self.initial_state.as_str()
    }
    pub fn states(&self) -> &[Identifier] {
        &self.states
    }
    pub fn transitions(&self) -> &[WorkflowTransition] {
        &self.transitions
    }
    pub fn available_step_ids(&self) -> &[Identifier] {
        &self.available_step_ids
    }
    pub fn max_replanning_attempts(&self) -> &NonNegativeInteger {
        &self.max_replanning_attempts
    }
}

// ---------------------------------------------------------------------------
// ToolDescriptor / ToolCall / ToolResult
// ---------------------------------------------------------------------------

/// The registered description of a tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDescriptor {
    tool_id: Identifier,
    version: Text,
    name: Text,
    description: Text,
    input_schema_digest: Sha256Digest,
    output_schema_digest: Sha256Digest,
    side_effect_level: SideEffectLevel,
    required_capabilities: Vec<Identifier>,
    timeout_seconds: FiniteNonNegativeFloat,
    destructive: bool,
    supports_structured_output: bool,
    idempotent: bool,
}

impl ContractModel for ToolDescriptor {
    const NAME: &'static str = "ToolDescriptor";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let tool_id = required(map, "tool_id", &mut issues, as_identifier);
        let version = required(map, "version", &mut issues, as_nonempty_text);
        let name = required(map, "name", &mut issues, as_nonempty_text);
        let description = required(map, "description", &mut issues, as_nonempty_text);
        let input_schema_digest = required(map, "input_schema_digest", &mut issues, as_digest);
        let output_schema_digest = required(map, "output_schema_digest", &mut issues, as_digest);
        let side_effect_level = enum_field(map, "side_effect_level", &mut issues);
        let required_capabilities = array(map, "required_capabilities", &mut issues, as_identifier);
        let required_capabilities = canonicalize_identifiers(
            required_capabilities,
            &mut issues,
            "required_capabilities",
            true,
        );
        let timeout_seconds = defaulted(
            map,
            "timeout_seconds",
            &mut issues,
            FiniteNonNegativeFloat::default,
            as_float_nonnegative,
        );
        let destructive = defaulted(map, "destructive", &mut issues, || false, as_bool_strict);
        let supports_structured_output = defaulted(
            map,
            "supports_structured_output",
            &mut issues,
            || true,
            as_bool_strict,
        );
        let idempotent = defaulted(map, "idempotent", &mut issues, || true, as_bool_strict);
        reject_extra(
            map,
            &[
                "tool_id",
                "version",
                "name",
                "description",
                "input_schema_digest",
                "output_schema_digest",
                "side_effect_level",
                "required_capabilities",
                "timeout_seconds",
                "destructive",
                "supports_structured_output",
                "idempotent",
            ],
            &mut issues,
        );
        if issues.is_empty() && destructive && side_effect_level == Some(SideEffectLevel::None) {
            push(
                &mut issues,
                "destructive",
                "destructive tools must declare a side effect",
            );
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            tool_id: tool_id.expect("validated"),
            version: version.expect("validated"),
            name: name.expect("validated"),
            description: description.expect("validated"),
            input_schema_digest: input_schema_digest.expect("validated"),
            output_schema_digest: output_schema_digest.expect("validated"),
            side_effect_level: side_effect_level.expect("validated"),
            required_capabilities,
            timeout_seconds,
            destructive,
            supports_structured_output,
            idempotent,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("tool_id", text(self.tool_id.as_str())),
            ("version", text_value(&self.version)),
            ("name", text_value(&self.name)),
            ("description", text_value(&self.description)),
            ("input_schema_digest", digest(self.input_schema_digest)),
            ("output_schema_digest", digest(self.output_schema_digest)),
            ("side_effect_level", text(self.side_effect_level.as_str())),
            (
                "required_capabilities",
                identifiers(&self.required_capabilities),
            ),
            ("timeout_seconds", float(self.timeout_seconds)),
            ("destructive", WireValue::Bool(self.destructive)),
            (
                "supports_structured_output",
                WireValue::Bool(self.supports_structured_output),
            ),
            ("idempotent", WireValue::Bool(self.idempotent)),
        ])
    }
}

impl ToolDescriptor {
    pub fn tool_id(&self) -> &str {
        self.tool_id.as_str()
    }
    pub fn version(&self) -> &Text {
        &self.version
    }
    pub fn name(&self) -> &Text {
        &self.name
    }
    pub fn description(&self) -> &Text {
        &self.description
    }
    pub fn input_schema_digest(&self) -> Sha256Digest {
        self.input_schema_digest
    }
    pub fn output_schema_digest(&self) -> Sha256Digest {
        self.output_schema_digest
    }
    pub fn side_effect_level(&self) -> SideEffectLevel {
        self.side_effect_level
    }
    pub fn required_capabilities(&self) -> &[Identifier] {
        &self.required_capabilities
    }
    pub fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds.get()
    }
    pub fn destructive(&self) -> bool {
        self.destructive
    }
    pub fn supports_structured_output(&self) -> bool {
        self.supports_structured_output
    }
    pub fn idempotent(&self) -> bool {
        self.idempotent
    }
}

/// A request to run a tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    call_id: Identifier,
    tool_id: Identifier,
    tool_version: Text,
    arguments_json: CanonicalJsonObject,
    requested_capabilities: Vec<Identifier>,
    side_effect_level: SideEffectLevel,
    timeout_seconds: FiniteNonNegativeFloat,
    issued_at: Timestamp,
}

impl ContractModel for ToolCall {
    const NAME: &'static str = "ToolCall";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let call_id = required(map, "call_id", &mut issues, as_identifier);
        let tool_id = required(map, "tool_id", &mut issues, as_identifier);
        let tool_version = required(map, "tool_version", &mut issues, as_nonempty_text);
        let arguments_json = defaulted(
            map,
            "arguments_json",
            &mut issues,
            CanonicalJsonObject::empty,
            as_canonical_object,
        );
        let requested_capabilities =
            array(map, "requested_capabilities", &mut issues, as_identifier);
        let requested_capabilities = canonicalize_identifiers(
            requested_capabilities,
            &mut issues,
            "requested_capabilities",
            true,
        );
        let side_effect_level = enum_field(map, "side_effect_level", &mut issues);
        let timeout_seconds = required(map, "timeout_seconds", &mut issues, as_float_nonnegative);
        let issued_at = required(map, "issued_at", &mut issues, as_timestamp);
        reject_extra(
            map,
            &[
                "call_id",
                "tool_id",
                "tool_version",
                "arguments_json",
                "requested_capabilities",
                "side_effect_level",
                "timeout_seconds",
                "issued_at",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            call_id: call_id.expect("validated"),
            tool_id: tool_id.expect("validated"),
            tool_version: tool_version.expect("validated"),
            arguments_json,
            requested_capabilities,
            side_effect_level: side_effect_level.expect("validated"),
            timeout_seconds: timeout_seconds.expect("validated"),
            issued_at: issued_at.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("call_id", text(self.call_id.as_str())),
            ("tool_id", text(self.tool_id.as_str())),
            ("tool_version", text_value(&self.tool_version)),
            ("arguments_json", text(self.arguments_json.as_str())),
            (
                "requested_capabilities",
                identifiers(&self.requested_capabilities),
            ),
            ("side_effect_level", text(self.side_effect_level.as_str())),
            ("timeout_seconds", float(self.timeout_seconds)),
            ("issued_at", timestamp(&self.issued_at)),
        ])
    }
}

impl ToolCall {
    pub fn call_id(&self) -> &str {
        self.call_id.as_str()
    }
    pub fn tool_id(&self) -> &str {
        self.tool_id.as_str()
    }
    pub fn tool_version(&self) -> &Text {
        &self.tool_version
    }
    pub fn arguments_json(&self) -> &CanonicalJsonObject {
        &self.arguments_json
    }
    pub fn requested_capabilities(&self) -> &[Identifier] {
        &self.requested_capabilities
    }
    pub fn side_effect_level(&self) -> SideEffectLevel {
        self.side_effect_level
    }
    pub fn timeout_seconds(&self) -> f64 {
        self.timeout_seconds.get()
    }
    pub fn issued_at(&self) -> &Timestamp {
        &self.issued_at
    }
}

/// The outcome of a tool call.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    call_id: Identifier,
    tool_id: Identifier,
    status: ToolRunStatus,
    result_json: Option<CanonicalJsonObject>,
    error_code: Option<Identifier>,
    error_message: Option<Text>,
    artifacts: Vec<ArtifactRef>,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    duration_ms: Option<FiniteNonNegativeFloat>,
}

impl ContractModel for ToolResult {
    const NAME: &'static str = "ToolResult";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let call_id = required(map, "call_id", &mut issues, as_identifier);
        let tool_id = required(map, "tool_id", &mut issues, as_identifier);
        let status = enum_field(map, "status", &mut issues);
        let result_json = optional(map, "result_json", &mut issues, as_canonical_object);
        let error_code = optional(map, "error_code", &mut issues, as_identifier);
        let error_message = optional(map, "error_message", &mut issues, as_nonempty_text);
        let artifacts = model_array::<ArtifactRef>(map, "artifacts", &mut issues);
        let started_at = optional(map, "started_at", &mut issues, as_timestamp);
        let finished_at = optional(map, "finished_at", &mut issues, as_timestamp);
        let duration_ms = optional(map, "duration_ms", &mut issues, as_float_nonnegative);
        unique_by(
            &artifacts,
            |artifact| artifact.artifact_id().to_owned(),
            &mut issues,
            "artifacts",
        );
        reject_extra(
            map,
            &[
                "call_id",
                "tool_id",
                "status",
                "result_json",
                "error_code",
                "error_message",
                "artifacts",
                "started_at",
                "finished_at",
                "duration_ms",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            let status = status.expect("validated");
            let failure = matches!(
                status,
                ToolRunStatus::Failed | ToolRunStatus::Denied | ToolRunStatus::TimedOut
            );
            if failure {
                if error_code.is_none() {
                    push(
                        &mut issues,
                        "error_code",
                        "failed tool runs require an error_code",
                    );
                }
                if result_json.is_some() {
                    push(
                        &mut issues,
                        "result_json",
                        "failed tool runs cannot contain result_json",
                    );
                }
            } else if error_code.is_some() {
                push(
                    &mut issues,
                    "error_code",
                    "successful or active tool runs cannot contain an error_code",
                );
            }
            if finished_at.is_some() && started_at.is_none() {
                push(
                    &mut issues,
                    "finished_at",
                    "finished_at requires started_at",
                );
            }
            if let (Some(started), Some(finished)) = (&started_at, &finished_at) {
                if finished < started {
                    push(
                        &mut issues,
                        "finished_at",
                        "finished_at cannot precede started_at",
                    );
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            call_id: call_id.expect("validated"),
            tool_id: tool_id.expect("validated"),
            status: status.expect("validated"),
            result_json,
            error_code,
            error_message,
            artifacts,
            started_at,
            finished_at,
            duration_ms,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("call_id", text(self.call_id.as_str())),
            ("tool_id", text(self.tool_id.as_str())),
            ("status", text(self.status.as_str())),
            (
                "result_json",
                self.result_json
                    .as_ref()
                    .map_or(WireValue::Null, |value| text(value.as_str())),
            ),
            ("error_code", opt_identifier(&self.error_code)),
            ("error_message", opt_text(&self.error_message)),
            (
                "artifacts",
                WireValue::Array(
                    self.artifacts
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            ("started_at", opt_timestamp(self.started_at.as_ref())),
            ("finished_at", opt_timestamp(self.finished_at.as_ref())),
            ("duration_ms", opt_float(self.duration_ms)),
        ])
    }
}

impl ToolResult {
    pub fn call_id(&self) -> &str {
        self.call_id.as_str()
    }
    pub fn tool_id(&self) -> &str {
        self.tool_id.as_str()
    }
    pub fn status(&self) -> ToolRunStatus {
        self.status
    }
    pub fn result_json(&self) -> Option<&CanonicalJsonObject> {
        self.result_json.as_ref()
    }
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_ref().map(Identifier::as_str)
    }
    pub fn error_message(&self) -> Option<&Text> {
        self.error_message.as_ref()
    }
    pub fn artifacts(&self) -> &[ArtifactRef] {
        &self.artifacts
    }
    pub fn started_at(&self) -> Option<&Timestamp> {
        self.started_at.as_ref()
    }
    pub fn finished_at(&self) -> Option<&Timestamp> {
        self.finished_at.as_ref()
    }
    pub fn duration_ms(&self) -> Option<f64> {
        self.duration_ms.map(FiniteNonNegativeFloat::get)
    }
}

// ---------------------------------------------------------------------------
// EvaluationMetric / EvaluationResult
// ---------------------------------------------------------------------------

/// One evaluation measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationMetric {
    name: Identifier,
    value: FiniteNonNegativeFloat,
    unit: Option<Text>,
    passed: Option<bool>,
}

impl ContractModel for EvaluationMetric {
    const NAME: &'static str = "EvaluationMetric";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let name = required(map, "name", &mut issues, as_identifier);
        let value = required(map, "value", &mut issues, as_float_nonnegative);
        let unit = optional(map, "unit", &mut issues, as_nonempty_text);
        let passed = optional(map, "passed", &mut issues, as_bool_strict);
        reject_extra(map, &["name", "value", "unit", "passed"], &mut issues);
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            name: name.expect("validated"),
            value: value.expect("validated"),
            unit,
            passed,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("name", text(self.name.as_str())),
            ("value", float(self.value)),
            ("unit", opt_text(&self.unit)),
            (
                "passed",
                self.passed.map_or(WireValue::Null, WireValue::Bool),
            ),
        ])
    }
}

impl EvaluationMetric {
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    pub fn value(&self) -> f64 {
        self.value.get()
    }
    pub fn unit(&self) -> Option<&Text> {
        self.unit.as_ref()
    }
    pub fn passed(&self) -> Option<bool> {
        self.passed
    }
}

/// The complete result of evaluating a design or requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationResult {
    evaluation_id: Identifier,
    subject_id: Identifier,
    requirement_id: Option<Identifier>,
    passed: bool,
    score: Option<UnitInterval>,
    metrics: Vec<EvaluationMetric>,
    findings: Vec<Text>,
    artifact_ids: Vec<Identifier>,
    evaluated_at: Timestamp,
}

impl ContractModel for EvaluationResult {
    const NAME: &'static str = "EvaluationResult";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let evaluation_id = required(map, "evaluation_id", &mut issues, as_identifier);
        let subject_id = required(map, "subject_id", &mut issues, as_identifier);
        let requirement_id = optional(map, "requirement_id", &mut issues, as_identifier);
        let passed = required(map, "passed", &mut issues, as_bool_strict);
        let score = optional(map, "score", &mut issues, as_unit_interval);
        let metrics = model_array::<EvaluationMetric>(map, "metrics", &mut issues);
        unique_by(
            &metrics,
            |metric| metric.name().to_owned(),
            &mut issues,
            "metrics",
        );
        let findings = array(map, "findings", &mut issues, as_nonempty_text);
        let findings = canonicalize_texts(findings, &mut issues, "findings", false);
        let artifact_ids = array(map, "artifact_ids", &mut issues, as_identifier);
        let artifact_ids =
            canonicalize_identifiers(artifact_ids, &mut issues, "artifact_ids", true);
        let evaluated_at = required(map, "evaluated_at", &mut issues, as_timestamp);
        reject_extra(
            map,
            &[
                "evaluation_id",
                "subject_id",
                "requirement_id",
                "passed",
                "score",
                "metrics",
                "findings",
                "artifact_ids",
                "evaluated_at",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            evaluation_id: evaluation_id.expect("validated"),
            subject_id: subject_id.expect("validated"),
            requirement_id,
            passed: passed.expect("validated"),
            score,
            metrics,
            findings,
            artifact_ids,
            evaluated_at: evaluated_at.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("evaluation_id", text(self.evaluation_id.as_str())),
            ("subject_id", text(self.subject_id.as_str())),
            ("requirement_id", opt_identifier(&self.requirement_id)),
            ("passed", WireValue::Bool(self.passed)),
            (
                "score",
                self.score
                    .map_or(WireValue::Null, |value| WireValue::Float(value.get())),
            ),
            (
                "metrics",
                WireValue::Array(
                    self.metrics
                        .iter()
                        .map(ContractModel::to_wire_value)
                        .collect(),
                ),
            ),
            ("findings", texts(&self.findings)),
            ("artifact_ids", identifiers(&self.artifact_ids)),
            ("evaluated_at", timestamp(&self.evaluated_at)),
        ])
    }
}

impl EvaluationResult {
    pub fn evaluation_id(&self) -> &str {
        self.evaluation_id.as_str()
    }
    pub fn subject_id(&self) -> &str {
        self.subject_id.as_str()
    }
    pub fn requirement_id(&self) -> Option<&str> {
        self.requirement_id.as_ref().map(Identifier::as_str)
    }
    pub fn passed(&self) -> bool {
        self.passed
    }
    pub fn score(&self) -> Option<f64> {
        self.score.map(UnitInterval::get)
    }
    pub fn metrics(&self) -> &[EvaluationMetric] {
        &self.metrics
    }
    pub fn findings(&self) -> &[Text] {
        &self.findings
    }
    pub fn artifact_ids(&self) -> &[Identifier] {
        &self.artifact_ids
    }
    pub fn evaluated_at(&self) -> &Timestamp {
        &self.evaluated_at
    }
}

// ---------------------------------------------------------------------------
// ApprovalRequest
// ---------------------------------------------------------------------------

/// A request for a human or policy approval checkpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRequest {
    request_id: Identifier,
    checkpoint_type: ApprovalKind,
    status: ApprovalStatus,
    workflow_id: Identifier,
    step_id: Option<Identifier>,
    action_digest: Sha256Digest,
    state_digest: Sha256Digest,
    options: Vec<Identifier>,
    selected_option_id: Option<Identifier>,
    requested_capabilities: Vec<Identifier>,
    artifact_ids: Vec<Identifier>,
    created_at: Timestamp,
    decided_at: Option<Timestamp>,
    actor_id: Option<Identifier>,
    modification_digest: Option<Sha256Digest>,
}

impl ContractModel for ApprovalRequest {
    const NAME: &'static str = "ApprovalRequest";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let request_id = required(map, "request_id", &mut issues, as_identifier);
        let checkpoint_type = enum_field(map, "checkpoint_type", &mut issues);
        let status = enum_field(map, "status", &mut issues);
        let workflow_id = required(map, "workflow_id", &mut issues, as_identifier);
        let step_id = optional(map, "step_id", &mut issues, as_identifier);
        let action_digest = required(map, "action_digest", &mut issues, as_digest);
        let state_digest = required(map, "state_digest", &mut issues, as_digest);
        let mut options = array_required(map, "options", &mut issues, as_identifier);
        if matches!(map.get(&key("options")), Some(CanonicalValue::Array(values)) if values.is_empty())
        {
            push(&mut issues, "options", "approval options cannot be empty");
        }
        options = canonicalize_identifiers(options, &mut issues, "options", true);
        let selected_option_id = optional(map, "selected_option_id", &mut issues, as_identifier);
        let requested_capabilities =
            array(map, "requested_capabilities", &mut issues, as_identifier);
        let requested_capabilities = canonicalize_identifiers(
            requested_capabilities,
            &mut issues,
            "requested_capabilities",
            true,
        );
        let artifact_ids = array(map, "artifact_ids", &mut issues, as_identifier);
        let artifact_ids =
            canonicalize_identifiers(artifact_ids, &mut issues, "artifact_ids", true);
        let created_at = required(map, "created_at", &mut issues, as_timestamp);
        let decided_at = optional(map, "decided_at", &mut issues, as_timestamp);
        let actor_id = optional(map, "actor_id", &mut issues, as_identifier);
        let modification_digest = optional(map, "modification_digest", &mut issues, as_digest);
        reject_extra(
            map,
            &[
                "request_id",
                "checkpoint_type",
                "status",
                "workflow_id",
                "step_id",
                "action_digest",
                "state_digest",
                "options",
                "selected_option_id",
                "requested_capabilities",
                "artifact_ids",
                "created_at",
                "decided_at",
                "actor_id",
                "modification_digest",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            if let Some(selected) = &selected_option_id {
                if !options.iter().any(|option| option == selected) {
                    push(
                        &mut issues,
                        "selected_option_id",
                        "selected_option_id must be declared in options",
                    );
                }
            }
            let status = status.expect("validated");
            if status == ApprovalStatus::Pending {
                if selected_option_id.is_some() {
                    push(
                        &mut issues,
                        "selected_option_id",
                        "pending approvals cannot have a selected option",
                    );
                }
                if decided_at.is_some() || actor_id.is_some() {
                    push(
                        &mut issues,
                        "decided_at",
                        "pending approvals cannot have a decision",
                    );
                }
                if modification_digest.is_some() {
                    push(
                        &mut issues,
                        "modification_digest",
                        "pending approvals cannot have a modification",
                    );
                }
            } else {
                if selected_option_id.is_none() {
                    push(
                        &mut issues,
                        "selected_option_id",
                        "resolved approvals require a selected option",
                    );
                }
                if modification_digest.is_some() && status != ApprovalStatus::Approved {
                    push(
                        &mut issues,
                        "modification_digest",
                        "only approved requests can contain a modification",
                    );
                }
            }
            if let (Some(decided), Some(created)) = (&decided_at, &created_at) {
                if decided < created {
                    push(
                        &mut issues,
                        "decided_at",
                        "decided_at cannot precede created_at",
                    );
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            request_id: request_id.expect("validated"),
            checkpoint_type: checkpoint_type.expect("validated"),
            status: status.expect("validated"),
            workflow_id: workflow_id.expect("validated"),
            step_id,
            action_digest: action_digest.expect("validated"),
            state_digest: state_digest.expect("validated"),
            options,
            selected_option_id,
            requested_capabilities,
            artifact_ids,
            created_at: created_at.expect("validated"),
            decided_at,
            actor_id,
            modification_digest,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("request_id", text(self.request_id.as_str())),
            ("checkpoint_type", text(self.checkpoint_type.as_str())),
            ("status", text(self.status.as_str())),
            ("workflow_id", text(self.workflow_id.as_str())),
            ("step_id", opt_identifier(&self.step_id)),
            ("action_digest", digest(self.action_digest)),
            ("state_digest", digest(self.state_digest)),
            ("options", identifiers(&self.options)),
            (
                "selected_option_id",
                opt_identifier(&self.selected_option_id),
            ),
            (
                "requested_capabilities",
                identifiers(&self.requested_capabilities),
            ),
            ("artifact_ids", identifiers(&self.artifact_ids)),
            ("created_at", timestamp(&self.created_at)),
            ("decided_at", opt_timestamp(self.decided_at.as_ref())),
            ("actor_id", opt_identifier(&self.actor_id)),
            ("modification_digest", opt_digest(&self.modification_digest)),
        ])
    }
}

impl ApprovalRequest {
    pub fn request_id(&self) -> &str {
        self.request_id.as_str()
    }
    pub fn checkpoint_type(&self) -> ApprovalKind {
        self.checkpoint_type
    }
    pub fn status(&self) -> ApprovalStatus {
        self.status
    }
    pub fn workflow_id(&self) -> &str {
        self.workflow_id.as_str()
    }
    pub fn step_id(&self) -> Option<&str> {
        self.step_id.as_ref().map(Identifier::as_str)
    }
    pub fn action_digest(&self) -> Sha256Digest {
        self.action_digest
    }
    pub fn state_digest(&self) -> Sha256Digest {
        self.state_digest
    }
    pub fn options(&self) -> &[Identifier] {
        &self.options
    }
    pub fn selected_option_id(&self) -> Option<&str> {
        self.selected_option_id.as_ref().map(Identifier::as_str)
    }
    pub fn requested_capabilities(&self) -> &[Identifier] {
        &self.requested_capabilities
    }
    pub fn artifact_ids(&self) -> &[Identifier] {
        &self.artifact_ids
    }
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
    pub fn decided_at(&self) -> Option<&Timestamp> {
        self.decided_at.as_ref()
    }
    pub fn actor_id(&self) -> Option<&str> {
        self.actor_id.as_ref().map(Identifier::as_str)
    }
    pub fn modification_digest(&self) -> Option<Sha256Digest> {
        self.modification_digest
    }
}

// ---------------------------------------------------------------------------
// RunBudget
// ---------------------------------------------------------------------------

/// The seven budget limits used by a run.
#[derive(Clone, Debug, PartialEq)]
pub struct RunBudget {
    max_model_calls: NonNegativeInteger,
    max_tool_calls: NonNegativeInteger,
    max_total_tokens: NonNegativeInteger,
    max_artifact_bytes: NonNegativeInteger,
    max_duration_seconds: FiniteNonNegativeFloat,
    max_cost: NonNegativeDecimal,
    max_replanning_attempts: NonNegativeInteger,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            max_model_calls: NonNegativeInteger::from_u64(32),
            max_tool_calls: NonNegativeInteger::from_u64(64),
            max_total_tokens: NonNegativeInteger::from_u64(100_000),
            max_artifact_bytes: NonNegativeInteger::from_u64(1_000_000_000),
            max_duration_seconds: FiniteNonNegativeFloat::new(3_600.0).expect("constant"),
            max_cost: NonNegativeDecimal::default(),
            max_replanning_attempts: NonNegativeInteger::from_u64(3),
        }
    }
}

impl RunBudget {
    /// The Python default budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the maximum model calls.
    #[must_use]
    pub fn with_max_model_calls(mut self, value: NonNegativeInteger) -> Self {
        self.max_model_calls = value;
        self
    }
    /// Set the maximum tool calls.
    #[must_use]
    pub fn with_max_tool_calls(mut self, value: NonNegativeInteger) -> Self {
        self.max_tool_calls = value;
        self
    }
    /// Set the maximum total tokens.
    #[must_use]
    pub fn with_max_total_tokens(mut self, value: NonNegativeInteger) -> Self {
        self.max_total_tokens = value;
        self
    }
    /// Set the maximum artifact bytes.
    #[must_use]
    pub fn with_max_artifact_bytes(mut self, value: NonNegativeInteger) -> Self {
        self.max_artifact_bytes = value;
        self
    }
    /// Set the maximum duration.
    #[must_use]
    pub fn with_max_duration_seconds(mut self, value: FiniteNonNegativeFloat) -> Self {
        self.max_duration_seconds = value;
        self
    }
    /// Set the maximum cost.
    #[must_use]
    pub fn with_max_cost(mut self, value: NonNegativeDecimal) -> Self {
        self.max_cost = value;
        self
    }
    /// Set the maximum replanning attempts.
    #[must_use]
    pub fn with_max_replanning_attempts(mut self, value: NonNegativeInteger) -> Self {
        self.max_replanning_attempts = value;
        self
    }

    pub fn max_model_calls(&self) -> &NonNegativeInteger {
        &self.max_model_calls
    }
    pub fn max_tool_calls(&self) -> &NonNegativeInteger {
        &self.max_tool_calls
    }
    pub fn max_total_tokens(&self) -> &NonNegativeInteger {
        &self.max_total_tokens
    }
    pub fn max_artifact_bytes(&self) -> &NonNegativeInteger {
        &self.max_artifact_bytes
    }
    pub fn max_duration_seconds(&self) -> f64 {
        self.max_duration_seconds.get()
    }
    pub fn max_cost(&self) -> &NonNegativeDecimal {
        &self.max_cost
    }
    pub fn max_replanning_attempts(&self) -> &NonNegativeInteger {
        &self.max_replanning_attempts
    }
}

impl ContractModel for RunBudget {
    const NAME: &'static str = "RunBudget";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let max_model_calls = defaulted(
            map,
            "max_model_calls",
            &mut issues,
            || NonNegativeInteger::from_u64(32),
            as_nonnegative_integer,
        );
        let max_tool_calls = defaulted(
            map,
            "max_tool_calls",
            &mut issues,
            || NonNegativeInteger::from_u64(64),
            as_nonnegative_integer,
        );
        let max_total_tokens = defaulted(
            map,
            "max_total_tokens",
            &mut issues,
            || NonNegativeInteger::from_u64(100_000),
            as_nonnegative_integer,
        );
        let max_artifact_bytes = defaulted(
            map,
            "max_artifact_bytes",
            &mut issues,
            || NonNegativeInteger::from_u64(1_000_000_000),
            as_nonnegative_integer,
        );
        let max_duration_seconds = defaulted(
            map,
            "max_duration_seconds",
            &mut issues,
            || FiniteNonNegativeFloat::new(3_600.0).expect("constant"),
            as_float_nonnegative,
        );
        let max_cost = defaulted(
            map,
            "max_cost",
            &mut issues,
            NonNegativeDecimal::default,
            as_decimal_nonnegative,
        );
        let max_replanning_attempts = defaulted(
            map,
            "max_replanning_attempts",
            &mut issues,
            || NonNegativeInteger::from_u64(3),
            as_nonnegative_integer,
        );
        reject_extra(
            map,
            &[
                "max_model_calls",
                "max_tool_calls",
                "max_total_tokens",
                "max_artifact_bytes",
                "max_duration_seconds",
                "max_cost",
                "max_replanning_attempts",
            ],
            &mut issues,
        );
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            max_model_calls,
            max_tool_calls,
            max_total_tokens,
            max_artifact_bytes,
            max_duration_seconds,
            max_cost,
            max_replanning_attempts,
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("max_model_calls", integer(&self.max_model_calls)),
            ("max_tool_calls", integer(&self.max_tool_calls)),
            ("max_total_tokens", integer(&self.max_total_tokens)),
            ("max_artifact_bytes", integer(&self.max_artifact_bytes)),
            ("max_duration_seconds", float(self.max_duration_seconds)),
            ("max_cost", text(&self.max_cost.to_python_string())),
            (
                "max_replanning_attempts",
                integer(&self.max_replanning_attempts),
            ),
        ])
    }
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// The recorded lineage of one or more artifacts or revisions.
#[derive(Clone, Debug, PartialEq)]
pub struct Provenance {
    provenance_id: Identifier,
    subject_ids: Vec<Identifier>,
    source_artifact_ids: Vec<Identifier>,
    output_artifact_ids: Vec<Identifier>,
    tool_call_ids: Vec<Identifier>,
    domain_package_digests: Vec<Sha256Digest>,
    parent_provenance_ids: Vec<Identifier>,
    transformation_ids: Vec<Identifier>,
    model_transcript_digest: Option<Sha256Digest>,
    seed: Option<NonNegativeInteger>,
    recorded_at: Timestamp,
}

impl ContractModel for Provenance {
    const NAME: &'static str = "Provenance";

    fn from_value(value: &CanonicalValue) -> Result<Self, ContractError> {
        let map = object(value, Self::NAME)?;
        let mut issues = Vec::new();
        let provenance_id = required(map, "provenance_id", &mut issues, as_identifier);
        let mut subject_ids = array_required(map, "subject_ids", &mut issues, as_identifier);
        if matches!(map.get(&key("subject_ids")), Some(CanonicalValue::Array(values)) if values.is_empty())
        {
            push(&mut issues, "subject_ids", "subject_ids cannot be empty");
        }
        subject_ids = canonicalize_identifiers(subject_ids, &mut issues, "subject_ids", true);
        let source_artifact_ids = array(map, "source_artifact_ids", &mut issues, as_identifier);
        let source_artifact_ids = canonicalize_identifiers(
            source_artifact_ids,
            &mut issues,
            "source_artifact_ids",
            true,
        );
        let output_artifact_ids = array(map, "output_artifact_ids", &mut issues, as_identifier);
        let output_artifact_ids = canonicalize_identifiers(
            output_artifact_ids,
            &mut issues,
            "output_artifact_ids",
            true,
        );
        let tool_call_ids = array(map, "tool_call_ids", &mut issues, as_identifier);
        let tool_call_ids =
            canonicalize_identifiers(tool_call_ids, &mut issues, "tool_call_ids", true);
        let domain_package_digests = array(map, "domain_package_digests", &mut issues, as_digest);
        let domain_package_digests = unique_digests(
            domain_package_digests,
            &mut issues,
            "domain_package_digests",
        );
        let mut domain_package_digests = domain_package_digests;
        domain_package_digests.sort_by_key(|digest| digest.to_hex());
        let parent_provenance_ids = array(map, "parent_provenance_ids", &mut issues, as_identifier);
        let parent_provenance_ids = canonicalize_identifiers(
            parent_provenance_ids,
            &mut issues,
            "parent_provenance_ids",
            true,
        );
        let transformation_ids = array(map, "transformation_ids", &mut issues, as_identifier);
        let transformation_ids =
            canonicalize_identifiers(transformation_ids, &mut issues, "transformation_ids", true);
        let model_transcript_digest =
            optional(map, "model_transcript_digest", &mut issues, as_digest);
        let seed = optional(map, "seed", &mut issues, as_nonnegative_integer);
        let recorded_at = required(map, "recorded_at", &mut issues, as_timestamp);
        reject_extra(
            map,
            &[
                "provenance_id",
                "subject_ids",
                "source_artifact_ids",
                "output_artifact_ids",
                "tool_call_ids",
                "domain_package_digests",
                "parent_provenance_ids",
                "transformation_ids",
                "model_transcript_digest",
                "seed",
                "recorded_at",
            ],
            &mut issues,
        );
        if issues.is_empty() {
            if let Some(provenance_id) = &provenance_id {
                if parent_provenance_ids
                    .iter()
                    .any(|parent| parent.as_str() == provenance_id.as_str())
                {
                    push(
                        &mut issues,
                        "parent_provenance_ids",
                        "provenance cannot be its own parent",
                    );
                }
            }
        }
        if !issues.is_empty() {
            return Err(ContractError::from_issues(Self::NAME, issues));
        }
        Ok(Self {
            provenance_id: provenance_id.expect("validated"),
            subject_ids,
            source_artifact_ids,
            output_artifact_ids,
            tool_call_ids,
            domain_package_digests,
            parent_provenance_ids,
            transformation_ids,
            model_transcript_digest,
            seed,
            recorded_at: recorded_at.expect("validated"),
        })
    }

    fn to_wire_value(&self) -> WireValue {
        WireValue::object([
            ("provenance_id", text(self.provenance_id.as_str())),
            ("subject_ids", identifiers(&self.subject_ids)),
            (
                "source_artifact_ids",
                identifiers(&self.source_artifact_ids),
            ),
            (
                "output_artifact_ids",
                identifiers(&self.output_artifact_ids),
            ),
            ("tool_call_ids", identifiers(&self.tool_call_ids)),
            (
                "domain_package_digests",
                digests(&self.domain_package_digests),
            ),
            (
                "parent_provenance_ids",
                identifiers(&self.parent_provenance_ids),
            ),
            ("transformation_ids", identifiers(&self.transformation_ids)),
            (
                "model_transcript_digest",
                opt_digest(&self.model_transcript_digest),
            ),
            ("seed", self.seed.as_ref().map_or(WireValue::Null, integer)),
            ("recorded_at", timestamp(&self.recorded_at)),
        ])
    }
}

impl Provenance {
    pub fn provenance_id(&self) -> &str {
        self.provenance_id.as_str()
    }
    pub fn subject_ids(&self) -> &[Identifier] {
        &self.subject_ids
    }
    pub fn source_artifact_ids(&self) -> &[Identifier] {
        &self.source_artifact_ids
    }
    pub fn output_artifact_ids(&self) -> &[Identifier] {
        &self.output_artifact_ids
    }
    pub fn tool_call_ids(&self) -> &[Identifier] {
        &self.tool_call_ids
    }
    pub fn domain_package_digests(&self) -> &[Sha256Digest] {
        &self.domain_package_digests
    }
    pub fn parent_provenance_ids(&self) -> &[Identifier] {
        &self.parent_provenance_ids
    }
    pub fn transformation_ids(&self) -> &[Identifier] {
        &self.transformation_ids
    }
    pub fn model_transcript_digest(&self) -> Option<Sha256Digest> {
        self.model_transcript_digest
    }
    pub fn seed(&self) -> Option<&NonNegativeInteger> {
        self.seed.as_ref()
    }
    pub fn recorded_at(&self) -> &Timestamp {
        &self.recorded_at
    }
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

fn text(value: &str) -> WireValue {
    WireValue::Text(Text::new(value))
}

fn text_value(value: &Text) -> WireValue {
    WireValue::Text(value.clone())
}

fn texts(values: &[Text]) -> WireValue {
    WireValue::Array(values.iter().map(text_value).collect())
}

fn identifiers(values: &[Identifier]) -> WireValue {
    WireValue::Array(values.iter().map(|value| text(value.as_str())).collect())
}

fn digest(value: Sha256Digest) -> WireValue {
    text(&value.to_hex())
}

fn digests(values: &[Sha256Digest]) -> WireValue {
    WireValue::Array(values.iter().map(|value| digest(*value)).collect())
}

fn opt_digest(value: &Option<Sha256Digest>) -> WireValue {
    value.map_or(WireValue::Null, digest)
}

fn opt_identifier(value: &Option<Identifier>) -> WireValue {
    value
        .as_ref()
        .map_or(WireValue::Null, |value| text(value.as_str()))
}

fn opt_text(value: &Option<Text>) -> WireValue {
    value.as_ref().map_or(WireValue::Null, text_value)
}

fn float(value: FiniteNonNegativeFloat) -> WireValue {
    WireValue::Float(value.get())
}

fn opt_float(value: Option<FiniteNonNegativeFloat>) -> WireValue {
    value.map_or(WireValue::Null, float)
}

fn integer(value: &NonNegativeInteger) -> WireValue {
    WireValue::Integer(value.as_bigint().clone())
}

fn timestamp(value: &Timestamp) -> WireValue {
    text(&value.to_wire_string())
}

fn opt_timestamp(value: Option<&Timestamp>) -> WireValue {
    value.map_or(WireValue::Null, timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    #[test]
    fn requirement_defaults_and_sorting_match_python() {
        let requirement = Requirement::from_json_str(
            r#"{"id":" req ","title":" T ","kind":"functional","tags":["b","a"]}"#,
        )
        .unwrap();
        assert_eq!(requirement.id(), "req");
        assert_eq!(requirement.title().as_str(), Some("T"));
        assert_eq!(requirement.priority(), RequirementPriority::Medium);
        assert_eq!(requirement.status(), RequirementStatus::Draft);
        assert_eq!(
            requirement
                .tags()
                .iter()
                .map(Identifier::as_str)
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn run_budget_defaults_and_decimal_scale() {
        let budget = RunBudget::from_json_str(r#"{"max_cost":"1.00"}"#).unwrap();
        assert_eq!(budget.max_model_calls().to_u64(), Some(32));
        assert_eq!(budget.max_cost().to_python_string(), "1.00");
        assert_eq!(
            budget.to_wire_json_ascii(),
            r#"{"max_model_calls":32,"max_tool_calls":64,"max_total_tokens":100000,"max_artifact_bytes":1000000000,"max_duration_seconds":3600.0,"max_cost":"1.00","max_replanning_attempts":3}"#
        );
    }

    #[test]
    fn cross_field_invariants_are_enforced() {
        assert!(
            ToolResult::from_json_str(
                r#"{"call_id":"c","tool_id":"t","status":"failed","result_json":"{\"a\":1}"}"#
            )
            .is_err()
        );
        assert!(ApprovalRequest::from_json_str(&format!(
            r#"{{"request_id":"a","checkpoint_type":"egress","status":"pending","workflow_id":"w","action_digest":"{}","state_digest":"{}","options":["approve"],"selected_option_id":"approve","created_at":"2026-01-02T03:04:05Z"}}"#,
            digest('a'),
            digest('b')
        ))
        .is_err());
    }
}
