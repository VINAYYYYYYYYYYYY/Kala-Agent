//! Native policy grants, approvals and decision state machine.
//!
//! This is a complete port of `kala_agent.kernel.policy`. The Python engine
//! keys approvals on the exact `(request_id, state_digest, action_digest)`
//! triple, keeps an approved tool-class cache scoped by action digest and
//! scope, invalidates stale/conflicting approvals, and exports/imports a
//! validated state document. All mutations are serialized by an internal
//! mutex; recursive engine calls are implemented as locked helpers so no user
//! callback can re-enter a held lock.

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use indexmap::IndexMap;
use kala_contracts::{ApprovalKind, ApprovalStatus, SideEffectLevel, Text};
use kala_json::CanonicalValue;
use num_bigint::BigInt;

use crate::json_input::parse_model_json;
use crate::time::{IsoformatError, KernelTimestamp};
use crate::value::{
    canonical_state_json, display_text, has_control_character, is_python_whitespace, value_bool,
    value_integer, value_text,
};

/// Python `POLICY_STATE_VERSION`.
pub const POLICY_STATE_VERSION: u64 = 1;

/// Python `DEFAULT_MAX_TRACKED_REQUESTS`.
pub const DEFAULT_MAX_TRACKED_REQUESTS: u64 = 512;

/// Python `MAX_NOTE_LENGTH`.
pub const MAX_NOTE_LENGTH: usize = 512;

/// The Python `ApprovalKey` tuple.
pub type ApprovalKey = (Text, Text, Text);

/// The Python `ToolClassKey` tuple.
pub type ToolClassKey = (Text, Option<Text>);

/// Base policy error, mapping the Python exception hierarchy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    /// `PolicyError` (the base `RuntimeError` subclass).
    Base(String),
    /// `PolicyTransitionError` (`PolicyError`, `ValueError`).
    Transition(String),
    /// `PolicyRequestError` (`PolicyError`, `ValueError`).
    Request(String),
    /// `PolicyStateError` (`PolicyError`, `ValueError`).
    State(String),
    /// Python `KeyError` from a state entry that indexes a required key.
    Key(String),
}

impl PolicyError {
    /// The Python exception class this variant corresponds to.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Base(_) => "PolicyError",
            Self::Transition(_) => "PolicyTransitionError",
            Self::Request(_) => "PolicyRequestError",
            Self::State(_) => "PolicyStateError",
            Self::Key(_) => "KeyError",
        }
    }

    /// The Python exception message.
    pub fn message(&self) -> &str {
        match self {
            Self::Base(message)
            | Self::Transition(message)
            | Self::Request(message)
            | Self::State(message)
            | Self::Key(message) => message,
        }
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PolicyError {}

fn transition(message: impl Into<String>) -> PolicyError {
    PolicyError::Transition(message.into())
}

fn request_error(message: impl Into<String>) -> PolicyError {
    PolicyError::Request(message.into())
}

fn state_error(message: impl Into<String>) -> PolicyError {
    PolicyError::State(format!("policy state is invalid: {}", message.into()))
}

/// A policy configuration or state-loading failure from an engine factory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicySetupError {
    /// Configuration (`TypeError`/`ValueError`).
    Config(PolicyConfigError),
    /// Dynamic state parsing/validation.
    Policy(PolicyError),
}

impl PolicySetupError {
    /// The Python exception class this variant corresponds to.
    pub const fn python_class_name(&self) -> &'static str {
        match self {
            Self::Config(error) => error.python_class_name(),
            Self::Policy(error) => error.python_class_name(),
        }
    }
    /// The exception message.
    pub fn message(&self) -> &str {
        match self {
            Self::Config(error) => error.message(),
            Self::Policy(error) => error.message(),
        }
    }
}

impl fmt::Display for PolicySetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PolicySetupError {}

impl From<PolicyConfigError> for PolicySetupError {
    fn from(error: PolicyConfigError) -> Self {
        Self::Config(error)
    }
}

impl From<PolicyError> for PolicySetupError {
    fn from(error: PolicyError) -> Self {
        Self::Policy(error)
    }
}

/// A Python `TypeError` or `ValueError` from policy engine configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyConfigError {
    /// Python `TypeError`.
    Type(String),
    /// Python `ValueError`.
    Value(String),
}

impl PolicyConfigError {
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

impl fmt::Display for PolicyConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PolicyConfigError {}

/// The Python `PolicyDecisionStatus` enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PolicyDecisionStatus {
    /// `allowed`.
    Allowed,
    /// `denied`.
    Denied,
    /// `pending`.
    Pending,
    /// `invalidated`.
    Invalidated,
    /// `rejected`.
    Rejected,
    /// `aborted`.
    Aborted,
}

impl PolicyDecisionStatus {
    /// Every accepted value in Python declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Allowed,
        Self::Denied,
        Self::Pending,
        Self::Invalidated,
        Self::Rejected,
        Self::Aborted,
    ];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Pending => "pending",
            Self::Invalidated => "invalidated",
            Self::Rejected => "rejected",
            Self::Aborted => "aborted",
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

impl fmt::Display for PolicyDecisionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The Python `PolicyReason` enum.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PolicyReason {
    /// `allowed`.
    Allowed,
    /// `approval_required`.
    ApprovalRequired,
    /// `capability_not_granted`.
    CapabilityNotGranted,
    /// `capability_expired`.
    CapabilityExpired,
    /// `scope_mismatch`.
    ScopeMismatch,
    /// `stale_approval`.
    StaleApproval,
    /// `rejected`.
    Rejected,
    /// `aborted`.
    Aborted,
    /// `invalid_request`.
    InvalidRequest,
}

impl PolicyReason {
    /// Every accepted value in Python declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Allowed,
        Self::ApprovalRequired,
        Self::CapabilityNotGranted,
        Self::CapabilityExpired,
        Self::ScopeMismatch,
        Self::StaleApproval,
        Self::Rejected,
        Self::Aborted,
        Self::InvalidRequest,
    ];

    /// The Python string value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::ApprovalRequired => "approval_required",
            Self::CapabilityNotGranted => "capability_not_granted",
            Self::CapabilityExpired => "capability_expired",
            Self::ScopeMismatch => "scope_mismatch",
            Self::StaleApproval => "stale_approval",
            Self::Rejected => "rejected",
            Self::Aborted => "aborted",
            Self::InvalidRequest => "invalid_request",
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

impl fmt::Display for PolicyReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

const APPROVAL_REQUIRED_EFFECTS: &[SideEffectLevel] = &[
    SideEffectLevel::LocalWrite,
    SideEffectLevel::Network,
    SideEffectLevel::Egress,
];

const TERMINAL_APPROVAL_STATUSES: &[ApprovalStatus] = &[
    ApprovalStatus::Rejected,
    ApprovalStatus::Aborted,
    ApprovalStatus::Invalidated,
    ApprovalStatus::Expired,
];

const REFUSED_APPROVAL_STATUSES: &[ApprovalStatus] =
    &[ApprovalStatus::Rejected, ApprovalStatus::Aborted];

fn unapproved_outcome(status: ApprovalStatus) -> (PolicyDecisionStatus, PolicyReason) {
    match status {
        ApprovalStatus::Pending => (
            PolicyDecisionStatus::Pending,
            PolicyReason::ApprovalRequired,
        ),
        ApprovalStatus::Rejected => (PolicyDecisionStatus::Rejected, PolicyReason::Rejected),
        ApprovalStatus::Aborted => (PolicyDecisionStatus::Aborted, PolicyReason::Aborted),
        ApprovalStatus::Invalidated | ApprovalStatus::Expired => (
            PolicyDecisionStatus::Invalidated,
            PolicyReason::StaleApproval,
        ),
        ApprovalStatus::Approved => (PolicyDecisionStatus::Allowed, PolicyReason::Allowed),
    }
}

fn safe_identifier(value: &Text, field_name: &str) -> Result<Text, PolicyError> {
    let normalized = value.python_strip();
    if normalized.is_empty() || normalized.len() > 256 {
        return Err(request_error(format!(
            "{field_name} must be nonempty and bounded"
        )));
    }
    if normalized
        .code_points()
        .iter()
        .copied()
        .any(|point| is_python_whitespace(point) || point < 32)
    {
        return Err(request_error(format!(
            "{field_name} contains unsafe characters"
        )));
    }
    Ok(normalized)
}

fn digest(value: &Text, field_name: &str) -> Result<Text, PolicyError> {
    safe_identifier(value, field_name)
}

fn note(value: Option<&Text>, field_name: &str) -> Result<Option<Text>, PolicyError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let normalized = value.python_strip();
    if normalized.is_empty() || normalized.len() > MAX_NOTE_LENGTH {
        return Err(request_error(format!(
            "{field_name} must be nonempty and bounded"
        )));
    }
    if has_control_character(&normalized) {
        return Err(request_error(format!(
            "{field_name} contains unsafe characters"
        )));
    }
    Ok(Some(normalized))
}

fn normalize_capabilities<I>(values: I) -> Result<Vec<Text>, PolicyError>
where
    I: IntoIterator<Item = Text>,
{
    let mut result = BTreeSet::new();
    for value in values {
        result.insert(safe_identifier(&value, "capability")?);
    }
    Ok(result.into_iter().collect())
}

fn text_value(value: &Text) -> CanonicalValue {
    CanonicalValue::Text(value.as_json_text().clone())
}

fn opt_text_value(value: Option<&Text>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, text_value)
}

fn opt_timestamp_value(value: Option<KernelTimestamp>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |value| {
        CanonicalValue::Text(kala_json::JsonText::new(&value.isoformat()))
    })
}

/// One scoped/expiring capability grant.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct CapabilityGrant {
    capability: Text,
    scope: Option<Text>,
    expires_at: Option<KernelTimestamp>,
}

impl CapabilityGrant {
    /// Build a grant and validate every field.
    pub fn from_parts(
        capability: Text,
        scope: Option<Text>,
        expires_at: Option<KernelTimestamp>,
    ) -> Result<Self, PolicyError> {
        let capability = safe_identifier(&capability, "capability")?;
        if capability == Text::new("*") {
            return Err(request_error(
                "wildcard capability grants are not permitted",
            ));
        }
        let scope = match scope {
            Some(scope) => Some(safe_identifier(&scope, "scope")?),
            None => None,
        };
        Ok(Self {
            capability,
            scope,
            expires_at,
        })
    }

    /// Build an unscoped, non-expiring grant.
    pub fn new(capability: Text) -> Result<Self, PolicyError> {
        Self::from_parts(capability, None, None)
    }

    /// Build a grant from a Rust string.
    pub fn parse(capability: &str) -> Result<Self, PolicyError> {
        Self::new(Text::new(capability))
    }

    /// The granted capability.
    pub fn capability(&self) -> &Text {
        &self.capability
    }

    /// The optional scope.
    pub fn scope(&self) -> Option<&Text> {
        self.scope.as_ref()
    }

    /// The optional expiry instant.
    pub fn expires_at(&self) -> Option<KernelTimestamp> {
        self.expires_at
    }

    /// Return a copy with a different scope.
    pub fn with_scope(self, scope: Option<Text>) -> Result<Self, PolicyError> {
        Self::from_parts(self.capability, scope, self.expires_at)
    }

    /// Return a copy with a different expiry.
    pub fn with_expires_at(self, expires_at: Option<KernelTimestamp>) -> Result<Self, PolicyError> {
        Self::from_parts(self.capability, self.scope, expires_at)
    }

    /// Whether this grant authorizes `capability` in `scope` at `now`.
    pub fn applies_to(
        &self,
        capability: &Text,
        scope: Option<&Text>,
        now: KernelTimestamp,
    ) -> bool {
        if &self.capability != capability {
            return false;
        }
        if self.expires_at.is_some_and(|expires_at| expires_at <= now) {
            return false;
        }
        self.scope.is_none() || self.scope.as_ref() == scope
    }

    /// The Python `as_dict()` shape.
    pub fn as_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `as_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("capability", text_value(&self.capability)),
            ("expires_at", opt_timestamp_value(self.expires_at)),
            ("scope", opt_text_value(self.scope.as_ref())),
        ])
    }
}

/// A request for policy authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyRequest {
    request_id: Text,
    state_digest: Text,
    action_digest: Text,
    requested_capabilities: Vec<Text>,
    side_effect_level: SideEffectLevel,
    destructive: bool,
    egress: bool,
    scope: Option<Text>,
    checkpoint_type: Option<ApprovalKind>,
}

impl PolicyRequest {
    /// Build a normalized request and validate every field.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        request_id: Text,
        state_digest: Text,
        action_digest: Text,
        requested_capabilities: impl IntoIterator<Item = Text>,
        side_effect_level: SideEffectLevel,
        destructive: bool,
        egress: bool,
        scope: Option<Text>,
        checkpoint_type: Option<ApprovalKind>,
    ) -> Result<Self, PolicyError> {
        let request_id = safe_identifier(&request_id, "request_id")?;
        let state_digest = digest(&state_digest, "state_digest")?;
        let action_digest = digest(&action_digest, "action_digest")?;
        let requested_capabilities = normalize_capabilities(requested_capabilities)?;
        let scope = match scope {
            Some(scope) => Some(safe_identifier(&scope, "scope")?),
            None => None,
        };
        let egress = egress || side_effect_level == SideEffectLevel::Egress;
        Ok(Self {
            request_id,
            state_digest,
            action_digest,
            requested_capabilities,
            side_effect_level,
            destructive,
            egress,
            scope,
            checkpoint_type,
        })
    }

    /// Build a request with Python's defaults.
    pub fn new(
        request_id: Text,
        state_digest: Text,
        action_digest: Text,
    ) -> Result<Self, PolicyError> {
        Self::from_parts(
            request_id,
            state_digest,
            action_digest,
            Vec::new(),
            SideEffectLevel::None,
            false,
            false,
            None,
            None,
        )
    }

    /// Revalidate and normalize this request.
    pub fn revalidated(&self) -> Result<Self, PolicyError> {
        Self::from_parts(
            self.request_id.clone(),
            self.state_digest.clone(),
            self.action_digest.clone(),
            self.requested_capabilities.clone(),
            self.side_effect_level,
            self.destructive,
            self.egress,
            self.scope.clone(),
            self.checkpoint_type,
        )
    }

    /// The normalized request identifier.
    pub fn request_id(&self) -> &Text {
        &self.request_id
    }

    /// The normalized state digest.
    pub fn state_digest(&self) -> &Text {
        &self.state_digest
    }

    /// The normalized action digest.
    pub fn action_digest(&self) -> &Text {
        &self.action_digest
    }

    /// Sorted, deduplicated requested capabilities.
    pub fn requested_capabilities(&self) -> &[Text] {
        &self.requested_capabilities
    }

    /// The declared side-effect level.
    pub fn side_effect_level(&self) -> SideEffectLevel {
        self.side_effect_level
    }

    /// Whether the request is destructive.
    pub fn destructive(&self) -> bool {
        self.destructive
    }

    /// Whether the request can leave the machine.
    pub fn egress(&self) -> bool {
        self.egress
    }

    /// The optional scope.
    pub fn scope(&self) -> Option<&Text> {
        self.scope.as_ref()
    }

    /// The optional checkpoint kind.
    pub fn checkpoint_type(&self) -> Option<ApprovalKind> {
        self.checkpoint_type
    }

    /// Whether an explicit approval checkpoint is required.
    pub fn requires_approval(&self) -> bool {
        self.checkpoint_type.is_some()
            || self.destructive
            || self.egress
            || APPROVAL_REQUIRED_EFFECTS.contains(&self.side_effect_level)
    }

    /// Whether this is a tool-class change request.
    pub fn tool_class_change(&self) -> bool {
        self.checkpoint_type == Some(ApprovalKind::ToolClassChange)
    }

    /// Python's `capabilities` property.
    pub fn capabilities(&self) -> &[Text] {
        &self.requested_capabilities
    }

    /// Python's `side_effect` property.
    pub fn side_effect(&self) -> SideEffectLevel {
        self.side_effect_level
    }

    /// The Python `as_dict()` shape.
    pub fn as_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `as_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("action_digest", text_value(&self.action_digest)),
            (
                "checkpoint_type",
                self.checkpoint_type.map_or(CanonicalValue::Null, |kind| {
                    CanonicalValue::Text(kala_json::JsonText::new(kind.as_str()))
                }),
            ),
            ("destructive", CanonicalValue::Bool(self.destructive)),
            ("egress", CanonicalValue::Bool(self.egress)),
            ("request_id", text_value(&self.request_id)),
            (
                "requested_capabilities",
                CanonicalValue::Array(self.requested_capabilities.iter().map(text_value).collect()),
            ),
            ("scope", opt_text_value(self.scope.as_ref())),
            (
                "side_effect_level",
                CanonicalValue::Text(kala_json::JsonText::new(self.side_effect_level.as_str())),
            ),
            ("state_digest", text_value(&self.state_digest)),
        ])
    }
}

/// A policy decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    request_id: Text,
    status: PolicyDecisionStatus,
    reason: PolicyReason,
    state_digest: Text,
    action_digest: Text,
    allowed: bool,
    requires_approval: bool,
    missing_capabilities: Vec<Text>,
    actor_id: Option<Text>,
    decided_at: Option<KernelTimestamp>,
}

impl PolicyDecision {
    #[allow(clippy::too_many_arguments)]
    fn new(
        request: &PolicyRequest,
        status: PolicyDecisionStatus,
        reason: PolicyReason,
        allowed: bool,
        missing_capabilities: Vec<Text>,
        actor_id: Option<Text>,
        decided_at: Option<KernelTimestamp>,
    ) -> Self {
        Self {
            request_id: request.request_id.clone(),
            status,
            reason,
            state_digest: request.state_digest.clone(),
            action_digest: request.action_digest.clone(),
            allowed,
            requires_approval: request.requires_approval(),
            missing_capabilities,
            actor_id,
            decided_at,
        }
    }

    /// The request identifier.
    pub fn request_id(&self) -> &Text {
        &self.request_id
    }
    /// The decision status.
    pub fn status(&self) -> PolicyDecisionStatus {
        self.status
    }
    /// The reason.
    pub fn reason(&self) -> PolicyReason {
        self.reason
    }
    /// The bound state digest.
    pub fn state_digest(&self) -> &Text {
        &self.state_digest
    }
    /// The bound action digest.
    pub fn action_digest(&self) -> &Text {
        &self.action_digest
    }
    /// Whether the request is allowed.
    pub fn allowed(&self) -> bool {
        self.allowed
    }
    /// Python's `approved` property.
    pub fn approved(&self) -> bool {
        self.allowed
    }
    /// Python's `denied` property.
    pub fn denied(&self) -> bool {
        !self.allowed
    }
    /// Whether approval was required.
    pub fn requires_approval(&self) -> bool {
        self.requires_approval
    }
    /// Capabilities that caused a denial.
    pub fn missing_capabilities(&self) -> &[Text] {
        &self.missing_capabilities
    }
    /// The deciding actor, when a resolved approval produced the decision.
    pub fn actor_id(&self) -> Option<&Text> {
        self.actor_id.as_ref()
    }
    /// The decision instant, when a resolved approval produced the decision.
    pub fn decided_at(&self) -> Option<KernelTimestamp> {
        self.decided_at
    }
    /// Python truthiness: true exactly when allowed.
    pub fn as_bool(&self) -> bool {
        self.allowed
    }

    /// The Python `as_dict()` shape.
    pub fn as_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `as_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("action_digest", text_value(&self.action_digest)),
            ("actor_id", opt_text_value(self.actor_id.as_ref())),
            ("allowed", CanonicalValue::Bool(self.allowed)),
            ("decided_at", opt_timestamp_value(self.decided_at)),
            (
                "missing_capabilities",
                CanonicalValue::Array(self.missing_capabilities.iter().map(text_value).collect()),
            ),
            (
                "reason",
                CanonicalValue::Text(kala_json::JsonText::new(self.reason.as_str())),
            ),
            ("request_id", text_value(&self.request_id)),
            (
                "requires_approval",
                CanonicalValue::Bool(self.requires_approval),
            ),
            ("state_digest", text_value(&self.state_digest)),
            (
                "status",
                CanonicalValue::Text(kala_json::JsonText::new(self.status.as_str())),
            ),
        ])
    }
}

/// One approval record, keyed by request and exact state/action digests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRecord {
    request_id: Text,
    state_digest: Text,
    action_digest: Text,
    status: ApprovalStatus,
    actor_id: Option<Text>,
    decided_at: Option<KernelTimestamp>,
    reason: Option<Text>,
}

impl ApprovalRecord {
    /// Build and validate an approval record.
    pub fn from_parts(
        request_id: Text,
        state_digest: Text,
        action_digest: Text,
        status: ApprovalStatus,
        actor_id: Option<Text>,
        decided_at: Option<KernelTimestamp>,
        reason: Option<Text>,
    ) -> Result<Self, PolicyError> {
        let request_id = safe_identifier(&request_id, "request_id")?;
        let state_digest = digest(&state_digest, "state_digest")?;
        let action_digest = digest(&action_digest, "action_digest")?;
        let actor_id = match actor_id {
            Some(actor_id) => Some(safe_identifier(&actor_id, "actor_id")?),
            None => None,
        };
        let reason = note(reason.as_ref(), "reason")?;
        if status == ApprovalStatus::Pending {
            if actor_id.is_some() || decided_at.is_some() || reason.is_some() {
                return Err(request_error("pending approvals cannot contain a decision"));
            }
        } else if actor_id.is_none() || decided_at.is_none() {
            return Err(request_error(
                "resolved approvals require actor and decision time",
            ));
        }
        Ok(Self {
            request_id,
            state_digest,
            action_digest,
            status,
            actor_id,
            decided_at,
            reason,
        })
    }

    /// Build a pending approval.
    pub fn pending(
        request_id: Text,
        state_digest: Text,
        action_digest: Text,
    ) -> Result<Self, PolicyError> {
        Self::from_parts(
            request_id,
            state_digest,
            action_digest,
            ApprovalStatus::Pending,
            None,
            None,
            None,
        )
    }

    /// The request identifier.
    pub fn request_id(&self) -> &Text {
        &self.request_id
    }
    /// The bound state digest.
    pub fn state_digest(&self) -> &Text {
        &self.state_digest
    }
    /// The bound action digest.
    pub fn action_digest(&self) -> &Text {
        &self.action_digest
    }
    /// The approval status.
    pub fn status(&self) -> ApprovalStatus {
        self.status
    }
    /// The deciding actor.
    pub fn actor_id(&self) -> Option<&Text> {
        self.actor_id.as_ref()
    }
    /// The decision instant.
    pub fn decided_at(&self) -> Option<KernelTimestamp> {
        self.decided_at
    }
    /// The decision note.
    pub fn reason(&self) -> Option<&Text> {
        self.reason.as_ref()
    }
    /// Python's `action_key` property.
    pub fn action_key(&self) -> (Text, Text) {
        (self.state_digest.clone(), self.action_digest.clone())
    }
    /// Python's `key` property.
    pub fn key(&self) -> ApprovalKey {
        (
            self.request_id.clone(),
            self.state_digest.clone(),
            self.action_digest.clone(),
        )
    }
    /// Python's `resolved` property.
    pub fn resolved(&self) -> bool {
        self.status != ApprovalStatus::Pending
    }

    /// The Python `as_dict()` shape.
    pub fn as_dict(&self) -> CanonicalValue {
        self.as_value()
    }

    /// The Python `as_dict()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("action_digest", text_value(&self.action_digest)),
            ("actor_id", opt_text_value(self.actor_id.as_ref())),
            ("decided_at", opt_timestamp_value(self.decided_at)),
            ("reason", opt_text_value(self.reason.as_ref())),
            ("request_id", text_value(&self.request_id)),
            ("state_digest", text_value(&self.state_digest)),
            (
                "status",
                CanonicalValue::Text(kala_json::JsonText::new(self.status.as_str())),
            ),
        ])
    }
}

/// Python aliases retained as Rust type aliases.
pub type PolicyApprovalRequest = PolicyRequest;
/// Python alias.
pub type PolicyApproval = ApprovalRecord;
/// Python alias.
pub type ApprovalRequest = PolicyRequest;
/// Python alias.
pub type PolicyRequestModel = PolicyRequest;
/// Python alias.
pub type DecisionStatus = PolicyDecisionStatus;
/// Python alias.
pub type PolicyStatus = PolicyDecisionStatus;
/// Python alias.
pub type ApprovalDecision = ApprovalRecord;
/// Python alias for the transition exception class.
pub type PolicyTransitionError = PolicyError;
/// Python alias for the request exception class.
pub type PolicyRequestError = PolicyError;
/// Python alias for the state exception class.
pub type PolicyStateError = PolicyError;

/// A remembered tool-class approval cache entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyToolClass {
    action_digest: Text,
    capabilities: Vec<Text>,
    scope: Option<Text>,
}

impl PolicyToolClass {
    /// Build and validate a tool-class cache entry.
    pub fn from_parts(
        action_digest: Text,
        capabilities: impl IntoIterator<Item = Text>,
        scope: Option<Text>,
    ) -> Result<Self, PolicyError> {
        let action_digest = digest(&action_digest, "action_digest")?;
        let normalized_capabilities = normalize_capabilities(capabilities)?;
        let scope = match scope {
            Some(scope) => Some(safe_identifier(&scope, "scope")?),
            None => None,
        };
        Ok(Self {
            action_digest,
            capabilities: normalized_capabilities,
            scope,
        })
    }

    /// The action digest.
    pub fn action_digest(&self) -> &Text {
        &self.action_digest
    }
    /// The exact remembered capability set.
    pub fn capabilities(&self) -> &[Text] {
        &self.capabilities
    }
    /// The optional scope.
    pub fn scope(&self) -> Option<&Text> {
        self.scope.as_ref()
    }

    fn key(&self) -> ToolClassKey {
        (self.action_digest.clone(), self.scope.clone())
    }

    /// The exported JSON shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("action_digest", text_value(&self.action_digest)),
            (
                "capabilities",
                CanonicalValue::Array(self.capabilities.iter().map(text_value).collect()),
            ),
            ("scope", opt_text_value(self.scope.as_ref())),
        ])
    }
}

/// A complete policy state document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyState {
    version: u64,
    grants: Vec<CapabilityGrant>,
    requests: Vec<PolicyRequest>,
    approvals: Vec<ApprovalRecord>,
    tool_classes: Vec<PolicyToolClass>,
    max_tracked_requests: BigInt,
}

impl PolicyState {
    /// Validate a state document with an explicit fallback request limit.
    pub fn parse(value: &CanonicalValue, default_limit: &BigInt) -> Result<Self, PolicyError> {
        let payload = state_mapping(value, "state")?;
        require_keys(payload, POLICY_STATE_KEYS, "state")?;
        let version = state_int(
            payload.get(&kala_json::JsonText::new("version")),
            "state.version",
        )?;
        let version_value = version.clone();
        let version_u64 = {
            use num_traits::ToPrimitive;
            version_value
                .to_u64()
                .ok_or_else(|| state_error(format!("unsupported version {version_value}")))?
        };
        if version_u64 != POLICY_STATE_VERSION {
            return Err(state_error(format!("unsupported version {version_value}")));
        }
        let limit = match payload.get(&kala_json::JsonText::new("max_tracked_requests")) {
            Some(value) => state_positive_int(Some(value), "state.max_tracked_requests")?,
            None => default_limit.clone(),
        };
        let mut grants = Vec::new();
        for item in state_sequence(payload.get(&kala_json::JsonText::new("grants")), "grants")? {
            grants.push(grant_from_state(item)?);
        }
        let mut requests: IndexMap<Text, PolicyRequest> = IndexMap::new();
        for item in state_sequence(
            payload.get(&kala_json::JsonText::new("requests")),
            "requests",
        )? {
            let request = request_from_state(item)?;
            if requests.contains_key(request.request_id()) {
                return Err(state_error(format!(
                    "duplicate request {}",
                    display_text(request.request_id())
                )));
            }
            requests.insert(request.request_id().clone(), request);
        }
        let mut approvals: IndexMap<ApprovalKey, ApprovalRecord> = IndexMap::new();
        for item in state_sequence(
            payload.get(&kala_json::JsonText::new("approvals")),
            "approvals",
        )? {
            let record = approval_from_state(item)?;
            if approvals.contains_key(&record.key()) {
                return Err(state_error(format!(
                    "duplicate approval for {}",
                    display_text(record.request_id())
                )));
            }
            approvals.insert(record.key(), record);
        }
        let orphaned: Vec<Text> = {
            let mut values = BTreeSet::new();
            for key in approvals.keys() {
                if !requests.contains_key(&key.0) {
                    values.insert(key.0.clone());
                }
            }
            values.into_iter().collect()
        };
        if !orphaned.is_empty() {
            return Err(state_error(format!(
                "approvals without a tracked request: {}",
                orphaned
                    .iter()
                    .map(display_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let mut tool_classes: IndexMap<ToolClassKey, PolicyToolClass> = IndexMap::new();
        for item in state_sequence(
            payload.get(&kala_json::JsonText::new("tool_classes")),
            "tool_classes",
        )? {
            let entry = tool_class_from_state(item)?;
            if tool_classes.contains_key(&entry.key()) {
                return Err(state_error(format!(
                    "duplicate tool class {}",
                    display_text(entry.action_digest())
                )));
            }
            tool_classes.insert(entry.key(), entry);
        }
        Ok(Self {
            version: version_u64,
            grants,
            requests: requests.into_values().collect(),
            approvals: approvals.into_values().collect(),
            tool_classes: tool_classes.into_values().collect(),
            max_tracked_requests: limit,
        })
    }

    /// Validate a state document from ordinary JSON text.
    pub fn parse_json(text: &str, default_limit: &BigInt) -> Result<Self, PolicyError> {
        let value = parse_model_json(text)
            .map_err(|error| state_error(format!("value must be valid JSON: {error}")))?;
        Self::parse(&value, default_limit)
    }

    /// The state version (always 1 when valid).
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Grants in export order.
    pub fn grants(&self) -> &[CapabilityGrant] {
        &self.grants
    }
    /// Requests in export order.
    pub fn requests(&self) -> &[PolicyRequest] {
        &self.requests
    }
    /// Approvals in export order.
    pub fn approvals(&self) -> &[ApprovalRecord] {
        &self.approvals
    }
    /// Tool-class cache entries in export order.
    pub fn tool_classes(&self) -> &[PolicyToolClass] {
        &self.tool_classes
    }
    /// The imported/exported request limit.
    pub fn max_tracked_requests(&self) -> &BigInt {
        &self.max_tracked_requests
    }

    /// Validate cross-record invariants for a typed state.
    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.version != POLICY_STATE_VERSION {
            return Err(state_error(format!("unsupported version {}", self.version)));
        }
        if self.max_tracked_requests < BigInt::from(1) {
            return Err(state_error("state.max_tracked_requests must be positive"));
        }
        let mut request_ids = HashSet::new();
        for request in &self.requests {
            if !request_ids.insert(request.request_id().clone()) {
                return Err(state_error(format!(
                    "duplicate request {}",
                    display_text(request.request_id())
                )));
            }
        }
        let mut approval_keys = HashSet::new();
        let mut orphaned = BTreeSet::new();
        for record in &self.approvals {
            if !approval_keys.insert(record.key()) {
                return Err(state_error(format!(
                    "duplicate approval for {}",
                    display_text(record.request_id())
                )));
            }
            if !request_ids.contains(record.request_id()) {
                orphaned.insert(record.request_id().clone());
            }
        }
        if !orphaned.is_empty() {
            return Err(state_error(format!(
                "approvals without a tracked request: {}",
                orphaned
                    .iter()
                    .map(display_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let mut tool_keys = HashSet::new();
        for entry in &self.tool_classes {
            if !tool_keys.insert(entry.key()) {
                return Err(state_error(format!(
                    "duplicate tool class {}",
                    display_text(entry.action_digest())
                )));
            }
        }
        Ok(())
    }

    /// The Python `export_state()` shape.
    pub fn as_value(&self) -> CanonicalValue {
        CanonicalValue::object([
            (
                "approvals",
                CanonicalValue::Array(
                    self.approvals
                        .iter()
                        .map(ApprovalRecord::as_value)
                        .collect(),
                ),
            ),
            (
                "grants",
                CanonicalValue::Array(self.grants.iter().map(CapabilityGrant::as_value).collect()),
            ),
            (
                "max_tracked_requests",
                CanonicalValue::Integer(self.max_tracked_requests.clone()),
            ),
            (
                "requests",
                CanonicalValue::Array(self.requests.iter().map(PolicyRequest::as_value).collect()),
            ),
            (
                "tool_classes",
                CanonicalValue::Array(
                    self.tool_classes
                        .iter()
                        .map(PolicyToolClass::as_value)
                        .collect(),
                ),
            ),
            (
                "version",
                CanonicalValue::Integer(BigInt::from(self.version)),
            ),
        ])
    }

    /// Canonical ASCII JSON bytes, matching a sorted `json.dumps` of the
    /// Python `export_state()` document.
    pub fn to_json(&self) -> Result<String, PolicyError> {
        canonical_state_json(&self.as_value())
            .map_err(|error| state_error(format!("state cannot be serialized: {error}")))
    }
}

/// One grant construction input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrantSpec {
    /// A bare capability.
    Capability(Text),
    /// A fully validated grant.
    Grant(CapabilityGrant),
    /// A capability restricted to a scope.
    Scoped { capability: Text, scope: Text },
}

impl From<&str> for GrantSpec {
    fn from(value: &str) -> Self {
        Self::Capability(Text::new(value))
    }
}

impl From<Text> for GrantSpec {
    fn from(value: Text) -> Self {
        Self::Capability(value)
    }
}

impl From<CapabilityGrant> for GrantSpec {
    fn from(value: CapabilityGrant) -> Self {
        Self::Grant(value)
    }
}

impl GrantSpec {
    fn into_grant(self) -> Result<CapabilityGrant, PolicyError> {
        match self {
            Self::Capability(capability) => CapabilityGrant::new(capability),
            Self::Grant(grant) => Ok(grant),
            Self::Scoped { capability, scope } => {
                CapabilityGrant::from_parts(capability, Some(scope), None)
            }
        }
    }
}

/// Extra arguments shared by approve/reject/abort/invalidate.
#[derive(Clone, Debug)]
pub struct DecisionContext {
    /// Optional replacement state digest; defaults to the request's digest.
    pub state_digest: Option<Text>,
    /// Optional replacement action digest; defaults to the request's digest.
    pub action_digest: Option<Text>,
    /// The deciding actor.
    pub actor_id: Text,
    /// The decision note for reject/abort (validated only when a record is
    /// created, exactly as Python does).
    pub reason: Option<Text>,
    /// Optional decision time.
    pub now: Option<KernelTimestamp>,
}

impl Default for DecisionContext {
    fn default() -> Self {
        Self {
            state_digest: None,
            action_digest: None,
            actor_id: Text::new("local-user"),
            reason: None,
            now: None,
        }
    }
}

impl DecisionContext {
    /// Default context (`local-user`, no digest overrides).
    pub fn new() -> Self {
        Self::default()
    }
    /// Override the state digest.
    pub fn with_state_digest(mut self, value: Text) -> Self {
        self.state_digest = Some(value);
        self
    }
    /// Override the action digest.
    pub fn with_action_digest(mut self, value: Text) -> Self {
        self.action_digest = Some(value);
        self
    }
    /// Override the actor.
    pub fn with_actor_id(mut self, value: Text) -> Self {
        self.actor_id = value;
        self
    }
    /// Set a decision note.
    pub fn with_reason(mut self, value: Text) -> Self {
        self.reason = Some(value);
        self
    }
    /// Set the decision time.
    pub fn with_now(mut self, value: KernelTimestamp) -> Self {
        self.now = Some(value);
        self
    }
}

struct Inner {
    grants: IndexMap<Text, Vec<CapabilityGrant>>,
    requests: IndexMap<Text, PolicyRequest>,
    approvals: IndexMap<ApprovalKey, ApprovalRecord>,
    tool_classes: IndexMap<ToolClassKey, Vec<Text>>,
    max_tracked_requests: BigInt,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            grants: IndexMap::new(),
            requests: IndexMap::new(),
            approvals: IndexMap::new(),
            tool_classes: IndexMap::new(),
            max_tracked_requests: BigInt::from(DEFAULT_MAX_TRACKED_REQUESTS),
        }
    }
}

/// Thread-safe policy engine.
pub struct PolicyEngine {
    inner: Mutex<Inner>,
    clock: Option<KernelTimestamp>,
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PolicyEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PolicyEngine")
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl PolicyEngine {
    /// Create an empty engine with the Python default limit.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            clock: None,
        }
    }

    /// Create an engine with an injectable fixed clock.
    pub fn fixed(now: KernelTimestamp) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            clock: Some(now),
        }
    }

    /// Set or clear the injectable clock.
    pub fn with_clock(mut self, clock: Option<KernelTimestamp>) -> Self {
        self.clock = clock;
        self
    }

    /// Set a fixed injectable clock.
    pub fn with_now(mut self, now: KernelTimestamp) -> Self {
        self.clock = Some(now);
        self
    }

    /// Set the tracked-request limit.
    pub fn with_max_tracked_requests(self, limit: BigInt) -> Result<Self, PolicyConfigError> {
        if limit < BigInt::from(1) {
            return Err(PolicyConfigError::Value(
                "max_tracked_requests must be positive".to_owned(),
            ));
        }
        self.lock().max_tracked_requests = limit;
        Ok(self)
    }

    /// Add grants from any supported grant specification.
    pub fn with_grants<I>(self, grants: I) -> Result<Self, PolicyError>
    where
        I: IntoIterator<Item = GrantSpec>,
    {
        for spec in grants {
            let grant = spec.into_grant()?;
            self.add_grant(grant);
        }
        Ok(self)
    }

    /// Create an engine from grants.
    pub fn from_grants<I>(grants: I) -> Result<Self, PolicyError>
    where
        I: IntoIterator<Item = GrantSpec>,
    {
        Self::new().with_grants(grants)
    }

    /// Create an engine from bare capability names.
    pub fn from_capabilities<I>(capacities: I) -> Result<Self, PolicyError>
    where
        I: IntoIterator<Item = Text>,
    {
        let specs = capacities
            .into_iter()
            .map(GrantSpec::Capability)
            .collect::<Vec<_>>();
        Self::new().with_grants(specs)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The fixed clock, when one is configured.
    pub fn clock(&self) -> Option<KernelTimestamp> {
        self.clock
    }

    /// The set of granted capability names.
    pub fn grants(&self) -> BTreeSet<Text> {
        self.lock().grants.keys().cloned().collect()
    }

    /// Python alias for `grants`.
    pub fn capability_grants(&self) -> BTreeSet<Text> {
        self.grants()
    }

    /// The tracked-request limit.
    pub fn max_tracked_requests(&self) -> BigInt {
        self.lock().max_tracked_requests.clone()
    }

    /// Tracked request identifiers in eviction-recency order.
    pub fn tracked_request_ids(&self) -> Vec<Text> {
        self.lock().requests.keys().cloned().collect()
    }

    /// Approval records in insertion order.
    pub fn approvals(&self) -> Vec<ApprovalRecord> {
        self.lock().approvals.values().cloned().collect()
    }

    /// Distinct approved tool-class action digests, sorted.
    pub fn approved_tool_classes(&self) -> Vec<Text> {
        let inner = self.lock();
        let mut values: BTreeSet<Text> = BTreeSet::new();
        for (action_digest, _) in inner.tool_classes.keys() {
            values.insert(action_digest.clone());
        }
        values.into_iter().collect()
    }

    /// Add a grant, deduplicating exact records.
    pub fn grant(&self, grant: CapabilityGrant) -> CapabilityGrant {
        self.add_grant(grant.clone());
        grant
    }

    /// Build and add a bare capability grant.
    pub fn grant_capability(&self, capability: Text) -> Result<CapabilityGrant, PolicyError> {
        let grant = CapabilityGrant::new(capability)?;
        Ok(self.grant(grant))
    }

    fn add_grant(&self, grant: CapabilityGrant) {
        let mut inner = self.lock();
        let existing = inner.grants.entry(grant.capability.clone()).or_default();
        if !existing.contains(&grant) {
            existing.push(grant);
        }
    }

    /// Remove every grant for a capability.
    pub fn revoke(&self, capability: &Text) -> Result<bool, PolicyError> {
        let normalized = safe_identifier(capability, "capability")?;
        let mut inner = self.lock();
        Ok(inner.grants.shift_remove(&normalized).is_some())
    }

    /// Python alias.
    pub fn revoke_capability(&self, capability: &Text) -> Result<bool, PolicyError> {
        self.revoke(capability)
    }

    fn resolve_now(&self, value: Option<KernelTimestamp>) -> KernelTimestamp {
        value.or(self.clock).unwrap_or_else(KernelTimestamp::now)
    }

    /// Evaluate a request.
    pub fn decide(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = request.revalidated()?;
        let current_time = self.resolve_now(now);
        let mut inner = self.lock();
        self.decide_locked(&mut inner, &request, current_time)
    }

    /// Python alias.
    pub fn evaluate(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        self.decide(request, now)
    }
    /// Python alias.
    pub fn check(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        self.decide(request, now)
    }
    /// Python alias.
    pub fn authorize(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        self.decide(request, now)
    }
    /// Python alias.
    pub fn request(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        self.decide(request, now)
    }
    /// Python alias.
    pub fn evaluate_request(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        self.decide(request, now)
    }

    fn decide_locked(
        &self,
        inner: &mut Inner,
        request: &PolicyRequest,
        current_time: KernelTimestamp,
    ) -> Result<PolicyDecision, PolicyError> {
        self.track_request_locked(inner, request);
        if request.requires_approval() && request.requested_capabilities().is_empty() {
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                PolicyDecisionStatus::Denied,
                PolicyReason::CapabilityNotGranted,
                false,
                Vec::new(),
                None,
                None,
            ));
        }
        let (missing, expired, scope_mismatch) =
            self.capability_state_locked(inner, request, current_time);
        if !missing.is_empty() || !expired.is_empty() || !scope_mismatch.is_empty() {
            let reason = if !missing.is_empty() {
                PolicyReason::CapabilityNotGranted
            } else if !expired.is_empty() {
                PolicyReason::CapabilityExpired
            } else {
                PolicyReason::ScopeMismatch
            };
            let mut listed = if !expired.is_empty() {
                expired
            } else if !scope_mismatch.is_empty() {
                scope_mismatch
            } else {
                missing
            };
            listed.sort();
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                PolicyDecisionStatus::Denied,
                reason,
                false,
                listed,
                None,
                None,
            ));
        }
        if !self.requires_approval_locked(inner, request) {
            self.remember_tool_class_locked(inner, request);
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                PolicyDecisionStatus::Allowed,
                PolicyReason::Allowed,
                true,
                Vec::new(),
                None,
                None,
            ));
        }
        let key = approval_key(request);
        let approval = inner.approvals.get(&key).cloned();
        let Some(approval) = approval else {
            if let Some(conflict) = self.conflicting_approval_locked(inner, request) {
                return self.invalidate_record_locked(
                    inner,
                    request,
                    &conflict,
                    conflict.actor_id().cloned(),
                    conflict.decided_at(),
                );
            }
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                PolicyDecisionStatus::Pending,
                PolicyReason::ApprovalRequired,
                false,
                Vec::new(),
                None,
                None,
            ));
        };
        if approval.status() != ApprovalStatus::Approved {
            let (status, reason) = unapproved_outcome(approval.status());
            let actor_id = approval.actor_id().cloned();
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                status,
                reason,
                false,
                Vec::new(),
                actor_id,
                None,
            ));
        }
        if approval.action_key()
            != (
                request.state_digest().clone(),
                request.action_digest().clone(),
            )
        {
            return self.invalidate_record_locked(
                inner,
                request,
                &approval,
                approval.actor_id().cloned(),
                approval.decided_at(),
            );
        }
        self.remember_tool_class_locked(inner, request);
        self.prune_locked(inner);
        Ok(PolicyDecision::new(
            request,
            PolicyDecisionStatus::Allowed,
            PolicyReason::Allowed,
            true,
            Vec::new(),
            approval.actor_id().cloned(),
            approval.decided_at(),
        ))
    }

    /// Request (or return the existing) approval record for a request.
    pub fn request_approval(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<ApprovalRecord, PolicyError> {
        let request = request.revalidated()?;
        let mut inner = self.lock();
        self.track_request_locked(&mut inner, &request);
        self.resolve_now(now);
        let key = approval_key(&request);
        if let Some(current) = inner.approvals.get(&key).cloned() {
            if current.status() != ApprovalStatus::Invalidated {
                self.prune_locked(&mut inner);
                return Ok(current);
            }
        }
        let record = self.store_pending_locked(
            &mut inner,
            request.request_id().clone(),
            request.state_digest().clone(),
            request.action_digest().clone(),
        )?;
        self.prune_locked(&mut inner);
        Ok(record)
    }

    /// Python alias.
    pub fn create_approval(
        &self,
        request: &PolicyRequest,
        now: Option<KernelTimestamp>,
    ) -> Result<ApprovalRecord, PolicyError> {
        self.request_approval(request, now)
    }

    fn supplied_action_key(
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<(Text, Text), PolicyError> {
        let state_digest = match &context.state_digest {
            Some(value) => digest(value, "state_digest")?,
            None => request.state_digest().clone(),
        };
        let action_digest = match &context.action_digest {
            Some(value) => digest(value, "action_digest")?,
            None => request.action_digest().clone(),
        };
        Ok((state_digest, action_digest))
    }

    fn resolve_request(&self, request: &PolicyRequest) -> Result<PolicyRequest, PolicyError> {
        request.revalidated()
    }

    fn resolve_request_id(&self, request_id: &Text) -> Result<PolicyRequest, PolicyError> {
        let normalized = safe_identifier(request_id, "request_id")?;
        let inner = self.lock();
        inner
            .requests
            .get(&normalized)
            .cloned()
            .ok_or_else(|| request_error(format!("unknown request: {}", display_text(&normalized))))
    }

    /// Approve a request (and create a pending record when absent).
    pub fn approve(
        &self,
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request(request)?;
        self.approve_locked(&request, context)
    }

    /// Approve by tracked request identifier.
    pub fn approve_by_id(
        &self,
        request_id: &Text,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request_id(request_id)?;
        self.approve_locked(&request, context)
    }

    fn approve_locked(
        &self,
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let action_key = Self::supplied_action_key(request, context)?;
        let actor = safe_identifier(&context.actor_id, "actor_id")?;
        let mut inner = self.lock();
        self.track_request_locked(&mut inner, request);
        let approval = inner
            .approvals
            .get(&(
                request.request_id().clone(),
                action_key.0.clone(),
                action_key.1.clone(),
            ))
            .cloned();
        let approval = match approval {
            Some(approval) => approval,
            None => {
                if let Some(stale) = self.stale_approval_locked(&inner, request, &action_key) {
                    return self.invalidate_record_locked(
                        &mut inner,
                        request,
                        &stale,
                        Some(actor),
                        Some(self.resolve_now(context.now)),
                    );
                }
                self.store_pending_locked(
                    &mut inner,
                    request.request_id().clone(),
                    action_key.0.clone(),
                    action_key.1.clone(),
                )?
            }
        };
        if approval.action_key() != action_key {
            return self.invalidate_record_locked(
                &mut inner,
                request,
                &approval,
                Some(actor),
                Some(self.resolve_now(context.now)),
            );
        }
        if approval.status() != ApprovalStatus::Pending {
            return Err(transition(format!(
                "approval for {} is already {}",
                display_text(request.request_id()),
                approval.status().as_str()
            )));
        }
        let record = ApprovalRecord::from_parts(
            approval.request_id().clone(),
            approval.state_digest().clone(),
            approval.action_digest().clone(),
            ApprovalStatus::Approved,
            Some(actor),
            Some(self.resolve_now(context.now)),
            None,
        )?;
        self.store_approval_locked(&mut inner, record);
        self.prune_locked(&mut inner);
        self.decide_locked(&mut inner, request, self.resolve_now(context.now))
    }

    /// Reject a request (and create a pending record when absent).
    pub fn reject(
        &self,
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request(request)?;
        self.resolve_non_approval_locked(&request, ApprovalStatus::Rejected, context)
    }

    /// Reject by tracked request identifier.
    pub fn reject_by_id(
        &self,
        request_id: &Text,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request_id(request_id)?;
        self.resolve_non_approval_locked(&request, ApprovalStatus::Rejected, context)
    }

    /// Abort a request (and create a pending record when absent).
    pub fn abort(
        &self,
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request(request)?;
        self.resolve_non_approval_locked(&request, ApprovalStatus::Aborted, context)
    }

    /// Abort by tracked request identifier.
    pub fn abort_by_id(
        &self,
        request_id: &Text,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request_id(request_id)?;
        self.resolve_non_approval_locked(&request, ApprovalStatus::Aborted, context)
    }

    fn resolve_non_approval_locked(
        &self,
        request: &PolicyRequest,
        status: ApprovalStatus,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let action_key = Self::supplied_action_key(request, context)?;
        let actor = safe_identifier(&context.actor_id, "actor_id")?;
        let mut inner = self.lock();
        self.track_request_locked(&mut inner, request);
        let approval = inner
            .approvals
            .get(&(
                request.request_id().clone(),
                action_key.0.clone(),
                action_key.1.clone(),
            ))
            .cloned();
        let approval = match approval {
            Some(approval) => approval,
            None => {
                if let Some(stale) = self.stale_approval_locked(&inner, request, &action_key) {
                    return self.invalidate_record_locked(
                        &mut inner,
                        request,
                        &stale,
                        Some(actor),
                        Some(self.resolve_now(context.now)),
                    );
                }
                self.store_pending_locked(
                    &mut inner,
                    request.request_id().clone(),
                    action_key.0.clone(),
                    action_key.1.clone(),
                )?
            }
        };
        if approval.action_key() != action_key {
            return self.invalidate_record_locked(
                &mut inner,
                request,
                &approval,
                Some(actor),
                Some(self.resolve_now(context.now)),
            );
        }
        if approval.status() != ApprovalStatus::Pending {
            return Err(transition(format!(
                "approval for {} is already {}",
                display_text(request.request_id()),
                approval.status().as_str()
            )));
        }
        inner
            .tool_classes
            .shift_remove(&(approval.action_digest().clone(), request.scope().cloned()));
        let record = ApprovalRecord::from_parts(
            approval.request_id().clone(),
            approval.state_digest().clone(),
            approval.action_digest().clone(),
            status,
            Some(actor),
            Some(self.resolve_now(context.now)),
            context.reason.clone(),
        )?;
        self.store_approval_locked(&mut inner, record);
        self.prune_locked(&mut inner);
        self.decide_locked(&mut inner, request, self.resolve_now(context.now))
    }

    /// Explicitly invalidate an approval.
    pub fn invalidate(
        &self,
        request: &PolicyRequest,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request(request)?;
        let action_key = Self::supplied_action_key(&request, context)?;
        let actor = safe_identifier(&context.actor_id, "actor_id")?;
        let mut inner = self.lock();
        self.track_request_locked(&mut inner, &request);
        let approval = inner
            .approvals
            .get(&(
                request.request_id().clone(),
                action_key.0.clone(),
                action_key.1.clone(),
            ))
            .cloned();
        let Some(approval) = approval else {
            return Err(transition(format!(
                "approval for {} does not match the supplied digests",
                display_text(request.request_id())
            )));
        };
        if approval.action_key() != action_key {
            return Err(transition(format!(
                "approval for {} does not match the supplied digests",
                display_text(request.request_id())
            )));
        }
        self.prune_locked(&mut inner);
        self.invalidate_record_locked(
            &mut inner,
            &request,
            &approval,
            Some(actor),
            Some(self.resolve_now(context.now)),
        )
    }

    /// Explicitly invalidate by tracked request identifier.
    pub fn invalidate_by_id(
        &self,
        request_id: &Text,
        context: &DecisionContext,
    ) -> Result<PolicyDecision, PolicyError> {
        let request = self.resolve_request_id(request_id)?;
        self.invalidate(&request, context)
    }

    /// Look up an approval, optionally by exact digests.
    pub fn get_approval(
        &self,
        request_id: &Text,
        state_digest: Option<&Text>,
        action_digest: Option<&Text>,
    ) -> Result<Option<ApprovalRecord>, PolicyError> {
        let normalized = safe_identifier(request_id, "request_id")?;
        let inner = self.lock();
        if state_digest.is_some() != action_digest.is_some() {
            return Err(request_error(
                "supply both state_digest and action_digest or neither",
            ));
        }
        if let (Some(state_digest), Some(action_digest)) = (state_digest, action_digest) {
            let key = (
                normalized,
                digest(state_digest, "state_digest")?,
                digest(action_digest, "action_digest")?,
            );
            return Ok(inner.approvals.get(&key).cloned());
        }
        for (key, record) in inner.approvals.iter().rev() {
            if key.0 == normalized {
                return Ok(Some(record.clone()));
            }
        }
        Ok(None)
    }

    /// Forget remembered tool classes for an action digest.
    pub fn forget_tool_class(
        &self,
        action_digest: &Text,
        scope: Option<&Text>,
    ) -> Result<bool, PolicyError> {
        let normalized = digest(action_digest, "action_digest")?;
        let wanted_scope = match scope {
            Some(scope) => Some(safe_identifier(scope, "scope")?),
            None => None,
        };
        let mut inner = self.lock();
        let keys: Vec<ToolClassKey> = inner
            .tool_classes
            .keys()
            .filter(|key| {
                key.0 == normalized
                    && wanted_scope
                        .as_ref()
                        .is_none_or(|wanted| key.1.as_ref() == Some(wanted))
            })
            .cloned()
            .collect();
        let mut removed = false;
        for key in keys {
            inner.tool_classes.shift_remove(&key);
            removed = true;
        }
        Ok(removed)
    }

    /// Export a validated copy of the complete state.
    pub fn export_state(&self) -> PolicyState {
        let inner = self.lock();
        let grants = inner
            .grants
            .values()
            .flat_map(|grants| grants.iter().cloned())
            .collect();
        PolicyState {
            version: POLICY_STATE_VERSION,
            grants,
            requests: inner.requests.values().cloned().collect(),
            approvals: inner.approvals.values().cloned().collect(),
            tool_classes: inner
                .tool_classes
                .iter()
                .map(|((action_digest, scope), capabilities)| PolicyToolClass {
                    action_digest: action_digest.clone(),
                    capabilities: capabilities.clone(),
                    scope: scope.clone(),
                })
                .collect(),
            max_tracked_requests: inner.max_tracked_requests.clone(),
        }
    }

    /// Export canonical ASCII JSON bytes.
    pub fn export_state_json(&self) -> Result<String, PolicyError> {
        self.export_state().to_json()
    }

    /// Import a dynamic state document. The engine is unchanged on failure.
    pub fn import_state(&self, state: &CanonicalValue) -> Result<(), PolicyError> {
        let default_limit = self.lock().max_tracked_requests.clone();
        let parsed = PolicyState::parse(state, &default_limit)?;
        self.install_state(parsed);
        Ok(())
    }

    /// Import ordinary JSON state text. The engine is unchanged on failure.
    pub fn import_state_json(&self, state: &str) -> Result<(), PolicyError> {
        let default_limit = self.lock().max_tracked_requests.clone();
        let parsed = PolicyState::parse_json(state, &default_limit)?;
        self.install_state(parsed);
        Ok(())
    }

    /// Import an already typed state, revalidating cross-record invariants.
    pub fn import_state_typed(&self, state: &PolicyState) -> Result<(), PolicyError> {
        state.validate()?;
        self.install_state(state.clone());
        Ok(())
    }

    fn install_state(&self, state: PolicyState) {
        let mut inner = self.lock();
        inner.grants.clear();
        for grant in state.grants {
            let existing = inner.grants.entry(grant.capability.clone()).or_default();
            if !existing.contains(&grant) {
                existing.push(grant);
            }
        }
        inner.requests = state
            .requests
            .into_iter()
            .map(|request| (request.request_id().clone(), request))
            .collect();
        inner.approvals = state
            .approvals
            .into_iter()
            .map(|record| (record.key(), record))
            .collect();
        inner.tool_classes = state
            .tool_classes
            .into_iter()
            .map(|entry| (entry.key(), entry.capabilities))
            .collect();
        inner.max_tracked_requests = state.max_tracked_requests;
        self.prune_locked(&mut inner);
    }

    /// Recreate an engine from a dynamic state document.
    pub fn from_state(state: &CanonicalValue) -> Result<Self, PolicySetupError> {
        Self::from_state_with(state, None, None)
    }

    /// Recreate an engine with an injectable clock and fallback limit.
    pub fn from_state_with(
        state: &CanonicalValue,
        now: Option<KernelTimestamp>,
        max_tracked_requests: Option<BigInt>,
    ) -> Result<Self, PolicySetupError> {
        let mut engine = Self::new();
        if let Some(now) = now {
            engine = PolicyEngine::fixed(now);
        }
        if let Some(limit) = max_tracked_requests {
            engine = engine.with_max_tracked_requests(limit)?;
        }
        engine.import_state(state)?;
        Ok(engine)
    }

    /// Recreate an engine from ordinary JSON state text.
    pub fn from_state_json(
        state: &str,
        now: Option<KernelTimestamp>,
        max_tracked_requests: Option<BigInt>,
    ) -> Result<Self, PolicySetupError> {
        let value = parse_model_json(state)
            .map_err(|error| state_error(format!("value must be valid JSON: {error}")))?;
        Self::from_state_with(&value, now, max_tracked_requests)
    }

    fn capability_state_locked(
        &self,
        inner: &Inner,
        request: &PolicyRequest,
        now: KernelTimestamp,
    ) -> (Vec<Text>, Vec<Text>, Vec<Text>) {
        let mut missing = Vec::new();
        let mut expired = Vec::new();
        let mut scope_mismatch = Vec::new();
        for capability in request.requested_capabilities() {
            let grants = inner.grants.get(capability);
            let Some(grants) = grants else {
                missing.push(capability.clone());
                continue;
            };
            if grants.is_empty() {
                missing.push(capability.clone());
                continue;
            }
            let active: Vec<&CapabilityGrant> = grants
                .iter()
                .filter(|grant| grant.expires_at().is_none_or(|expires_at| expires_at > now))
                .collect();
            if active.is_empty() {
                expired.push(capability.clone());
                continue;
            }
            if !active
                .iter()
                .any(|grant| grant.applies_to(capability, request.scope(), now))
            {
                scope_mismatch.push(capability.clone());
            }
        }
        (missing, expired, scope_mismatch)
    }

    fn requires_approval_locked(&self, inner: &Inner, request: &PolicyRequest) -> bool {
        if !request.requires_approval() {
            return false;
        }
        !self.tool_class_approved_locked(inner, request)
    }

    fn tool_class_approved_locked(&self, inner: &Inner, request: &PolicyRequest) -> bool {
        if !request.tool_class_change() {
            return false;
        }
        let remembered = inner
            .tool_classes
            .get(&(request.action_digest().clone(), request.scope().cloned()));
        remembered.is_some_and(|capabilities| capabilities == request.requested_capabilities())
    }

    fn remember_tool_class_locked(&self, inner: &mut Inner, request: &PolicyRequest) {
        if !request.tool_class_change() {
            return;
        }
        let key = (request.action_digest().clone(), request.scope().cloned());
        if inner.tool_classes.get(&key) == Some(&request.requested_capabilities) {
            return;
        }
        inner
            .tool_classes
            .insert(key, request.requested_capabilities().to_vec());
        while BigInt::from(inner.tool_classes.len()) > inner.max_tracked_requests {
            let first = inner.tool_classes.keys().next().cloned();
            let Some(first) = first else {
                break;
            };
            inner.tool_classes.shift_remove(&first);
        }
    }

    fn prune_locked(&self, inner: &mut Inner) {
        let limit = inner.max_tracked_requests.clone();
        if BigInt::from(inner.approvals.len()) <= limit
            && BigInt::from(inner.requests.len()) <= limit
        {
            return;
        }
        let tracked: HashSet<Text> = inner.requests.keys().cloned().collect();
        inner.approvals.retain(|key, _| tracked.contains(&key.0));
        if BigInt::from(inner.approvals.len()) <= limit
            && BigInt::from(inner.requests.len()) <= limit
        {
            return;
        }
        let recency: std::collections::HashMap<Text, usize> = inner
            .requests
            .keys()
            .enumerate()
            .map(|(index, request_id)| (request_id.clone(), index))
            .collect();
        let mut ranked: Vec<Text> = inner.requests.keys().cloned().collect();
        ranked.sort_by_key(|request_id| {
            (
                self.eviction_tier_locked(inner, request_id),
                recency.get(request_id).copied().unwrap_or(0),
            )
        });
        for request_id in ranked {
            if BigInt::from(inner.requests.len()) <= limit
                && BigInt::from(inner.approvals.len()) <= limit
            {
                break;
            }
            self.discard_request_locked(inner, &request_id);
        }
    }

    /// Rank a request for eviction. Forgetting a lower tier grants no
    /// authority: pending approvals are kept longest, then refused decisions,
    /// then approvals, then terminal non-refusals.
    fn eviction_tier_locked(&self, inner: &Inner, request_id: &Text) -> u8 {
        let mut has_pending = false;
        let mut has_refused = false;
        let mut has_approved = false;
        for (key, record) in &inner.approvals {
            if &key.0 != request_id {
                continue;
            }
            if !record.resolved() {
                has_pending = true;
            }
            if REFUSED_APPROVAL_STATUSES.contains(&record.status()) {
                has_refused = true;
            }
            if !TERMINAL_APPROVAL_STATUSES.contains(&record.status()) {
                has_approved = true;
            }
        }
        if has_pending {
            3
        } else if has_refused {
            2
        } else if has_approved {
            1
        } else {
            0
        }
    }

    fn discard_request_locked(&self, inner: &mut Inner, request_id: &Text) {
        inner.requests.shift_remove(request_id);
        let keys: Vec<ApprovalKey> = inner
            .approvals
            .keys()
            .filter(|key| &key.0 == request_id)
            .cloned()
            .collect();
        for key in keys {
            inner.approvals.shift_remove(&key);
        }
    }

    fn track_request_locked(&self, inner: &mut Inner, request: &PolicyRequest) {
        inner.requests.shift_remove(request.request_id());
        inner
            .requests
            .insert(request.request_id().clone(), request.clone());
    }

    fn store_pending_locked(
        &self,
        inner: &mut Inner,
        request_id: Text,
        state_digest: Text,
        action_digest: Text,
    ) -> Result<ApprovalRecord, PolicyError> {
        let record = ApprovalRecord::pending(request_id, state_digest, action_digest)?;
        self.store_approval_locked(inner, record.clone());
        Ok(record)
    }

    fn store_approval_locked(&self, inner: &mut Inner, record: ApprovalRecord) {
        inner.approvals.shift_remove(&record.key());
        inner.approvals.insert(record.key(), record);
    }

    fn conflicting_approval_locked(
        &self,
        inner: &Inner,
        request: &PolicyRequest,
    ) -> Option<ApprovalRecord> {
        for (key, record) in inner.approvals.iter().rev() {
            if key.0 != *request.request_id() {
                continue;
            }
            if &key.1 == request.state_digest() && &key.2 == request.action_digest() {
                continue;
            }
            if record.status() == ApprovalStatus::Pending
                || record.status() == ApprovalStatus::Approved
            {
                return Some(record.clone());
            }
        }
        None
    }

    fn stale_approval_locked(
        &self,
        inner: &Inner,
        request: &PolicyRequest,
        action_key: &(Text, Text),
    ) -> Option<ApprovalRecord> {
        if let Some(current) = inner.approvals.get(&approval_key(request)) {
            if &current.action_key() != action_key {
                return Some(current.clone());
            }
        }
        if let Some(conflict) = self.conflicting_approval_locked(inner, request) {
            if &conflict.action_key() != action_key {
                return Some(conflict);
            }
        }
        None
    }

    fn invalidate_record_locked(
        &self,
        inner: &mut Inner,
        request: &PolicyRequest,
        approval: &ApprovalRecord,
        actor_id: Option<Text>,
        decided_at: Option<KernelTimestamp>,
    ) -> Result<PolicyDecision, PolicyError> {
        let resolved_actor = actor_id.unwrap_or_else(|| Text::new("local-user"));
        let resolved_time = decided_at.unwrap_or_else(|| self.resolve_now(None));
        inner
            .tool_classes
            .shift_remove(&(approval.action_digest().clone(), request.scope().cloned()));
        if REFUSED_APPROVAL_STATUSES.contains(&approval.status()) {
            let (status, reason) = unapproved_outcome(approval.status());
            self.prune_locked(inner);
            return Ok(PolicyDecision::new(
                request,
                status,
                reason,
                false,
                Vec::new(),
                approval.actor_id().cloned(),
                approval.decided_at(),
            ));
        }
        let record = ApprovalRecord::from_parts(
            approval.request_id().clone(),
            approval.state_digest().clone(),
            approval.action_digest().clone(),
            ApprovalStatus::Invalidated,
            Some(resolved_actor.clone()),
            Some(resolved_time),
            Some(Text::new("state or action digest changed")),
        )?;
        self.store_approval_locked(inner, record);
        self.prune_locked(inner);
        Ok(PolicyDecision::new(
            request,
            PolicyDecisionStatus::Invalidated,
            PolicyReason::StaleApproval,
            false,
            Vec::new(),
            Some(resolved_actor),
            Some(resolved_time),
        ))
    }
}

fn approval_key(request: &PolicyRequest) -> ApprovalKey {
    (
        request.request_id().clone(),
        request.state_digest().clone(),
        request.action_digest().clone(),
    )
}

// ---------------------------------------------------------------------------
// Dynamic state parsing.
// ---------------------------------------------------------------------------

const REQUEST_STATE_KEYS: &[&str] = &[
    "request_id",
    "state_digest",
    "action_digest",
    "requested_capabilities",
    "side_effect_level",
    "destructive",
    "egress",
    "scope",
    "checkpoint_type",
];

const APPROVAL_STATE_KEYS: &[&str] = &[
    "request_id",
    "state_digest",
    "action_digest",
    "status",
    "actor_id",
    "decided_at",
    "reason",
];

const GRANT_STATE_KEYS: &[&str] = &["capability", "scope", "expires_at"];

const TOOL_CLASS_STATE_KEYS: &[&str] = &["action_digest", "capabilities", "scope"];

const POLICY_STATE_KEYS: &[&str] = &[
    "version",
    "grants",
    "requests",
    "approvals",
    "tool_classes",
    "max_tracked_requests",
];

fn state_mapping<'a>(
    value: &'a CanonicalValue,
    field_name: &str,
) -> Result<&'a kala_json::CanonicalObject, PolicyError> {
    match value {
        CanonicalValue::Object(object) => Ok(object),
        _ => Err(state_error(format!("{field_name} must be a mapping"))),
    }
}

fn require_keys(
    payload: &kala_json::CanonicalObject,
    allowed: &[&str],
    field_name: &str,
) -> Result<(), PolicyError> {
    let mut unexpected: Vec<kala_json::JsonText> = payload
        .keys()
        .filter(|key| {
            key.to_utf8()
                .map_or(true, |text| !allowed.contains(&text.as_str()))
        })
        .cloned()
        .collect();
    if unexpected.is_empty() {
        return Ok(());
    }
    unexpected.sort();
    Err(state_error(format!(
        "{field_name} has unexpected keys: {}",
        unexpected
            .iter()
            .map(|key| key.to_string_lossy())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn state_sequence<'a>(
    value: Option<&'a CanonicalValue>,
    field_name: &str,
) -> Result<Vec<&'a CanonicalValue>, PolicyError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        CanonicalValue::Array(values) => Ok(values.iter().collect()),
        _ => Err(state_error(format!("{field_name} must be a sequence"))),
    }
}

fn state_str(value: Option<&CanonicalValue>, field_name: &str) -> Result<Text, PolicyError> {
    value
        .and_then(value_text)
        .ok_or_else(|| state_error(format!("{field_name} must be a string")))
}

fn indexed_state_str(
    payload: &kala_json::CanonicalObject,
    key: &str,
    field_name: &str,
) -> Result<Text, PolicyError> {
    match payload.get(&kala_json::JsonText::new(key)) {
        Some(value) => state_str(Some(value), field_name),
        None => Err(PolicyError::Key(format!("'{key}'"))),
    }
}

fn state_opt_str(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<Option<Text>, PolicyError> {
    match value {
        None | Some(CanonicalValue::Null) => Ok(None),
        Some(value) => Ok(Some(state_str(Some(value), field_name)?)),
    }
}

fn state_bool(value: Option<&CanonicalValue>, field_name: &str) -> Result<bool, PolicyError> {
    value
        .and_then(value_bool)
        .ok_or_else(|| state_error(format!("{field_name} must be a boolean")))
}

fn state_int<'a>(
    value: Option<&'a CanonicalValue>,
    field_name: &str,
) -> Result<&'a BigInt, PolicyError> {
    value
        .and_then(value_integer)
        .ok_or_else(|| state_error(format!("{field_name} must be an integer")))
}

fn state_positive_int(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<BigInt, PolicyError> {
    let parsed = state_int(value, field_name)?;
    if parsed < &BigInt::from(1) {
        return Err(state_error(format!("{field_name} must be positive")));
    }
    Ok(parsed.clone())
}

fn state_str_tuple(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<Vec<Text>, PolicyError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        CanonicalValue::Array(values) => values
            .iter()
            .map(|item| {
                value_text(item)
                    .ok_or_else(|| state_error(format!("{field_name} must be a string")))
            })
            .collect(),
        _ => Err(state_error(format!("{field_name} must be a sequence"))),
    }
}

fn state_datetime(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<KernelTimestamp, PolicyError> {
    let text = state_str(value, field_name)?;
    if text.has_surrogates() {
        return Err(state_error(format!(
            "{field_name} must be an ISO timestamp"
        )));
    }
    let source = text.to_string_lossy();
    KernelTimestamp::parse_isoformat(&source).map_err(|error| match error {
        IsoformatError::Invalid => state_error(format!("{field_name} must be an ISO timestamp")),
        IsoformatError::Naive => state_error(format!("{field_name} must include a timezone")),
    })
}

fn state_opt_datetime(
    value: Option<&CanonicalValue>,
    field_name: &str,
) -> Result<Option<KernelTimestamp>, PolicyError> {
    match value {
        None | Some(CanonicalValue::Null) => Ok(None),
        Some(value) => Ok(Some(state_datetime(Some(value), field_name)?)),
    }
}

fn request_from_state(value: &CanonicalValue) -> Result<PolicyRequest, PolicyError> {
    let payload = state_mapping(value, "request")?;
    require_keys(payload, REQUEST_STATE_KEYS, "request")?;
    let request_id = indexed_state_str(payload, "request_id", "request.request_id")?;
    let state_digest = indexed_state_str(payload, "state_digest", "request.state_digest")?;
    let action_digest = indexed_state_str(payload, "action_digest", "request.action_digest")?;
    let requested_capabilities = state_str_tuple(
        payload.get(&kala_json::JsonText::new("requested_capabilities")),
        "request.requested_capabilities",
    )?;
    let side_effect_text = match payload.get(&kala_json::JsonText::new("side_effect_level")) {
        None => Text::new(SideEffectLevel::None.as_str()),
        Some(value) => state_str(Some(value), "request.side_effect_level")?,
    };
    let side_effect_source = side_effect_text.to_string_lossy();
    let side_effect_level = SideEffectLevel::parse(&side_effect_source)
        .ok_or_else(|| state_error("request entry is invalid: side_effect_level is invalid"))?;
    let destructive = state_bool(
        payload.get(&kala_json::JsonText::new("destructive")),
        "request.destructive",
    )
    .or_else(|error| {
        if payload
            .get(&kala_json::JsonText::new("destructive"))
            .is_none()
        {
            Ok(false)
        } else {
            Err(error)
        }
    })?;
    let egress = state_bool(
        payload.get(&kala_json::JsonText::new("egress")),
        "request.egress",
    )
    .or_else(|error| {
        if payload.get(&kala_json::JsonText::new("egress")).is_none() {
            Ok(false)
        } else {
            Err(error)
        }
    })?;
    let scope = state_opt_str(
        payload.get(&kala_json::JsonText::new("scope")),
        "request.scope",
    )?;
    let checkpoint = state_opt_str(
        payload.get(&kala_json::JsonText::new("checkpoint_type")),
        "request.checkpoint_type",
    )?;
    let checkpoint_type = match checkpoint {
        Some(value) => {
            let source = value.to_string_lossy();
            Some(ApprovalKind::parse(&source).ok_or_else(|| {
                state_error("request entry is invalid: checkpoint_type is invalid")
            })?)
        }
        None => None,
    };
    PolicyRequest::from_parts(
        request_id,
        state_digest,
        action_digest,
        requested_capabilities,
        side_effect_level,
        destructive,
        egress,
        scope,
        checkpoint_type,
    )
    .map_err(|error| state_error(format!("request entry is invalid: {}", error.message())))
}

fn approval_from_state(value: &CanonicalValue) -> Result<ApprovalRecord, PolicyError> {
    let payload = state_mapping(value, "approval")?;
    require_keys(payload, APPROVAL_STATE_KEYS, "approval")?;
    let request_id = indexed_state_str(payload, "request_id", "approval.request_id")?;
    let state_digest = indexed_state_str(payload, "state_digest", "approval.state_digest")?;
    let action_digest = indexed_state_str(payload, "action_digest", "approval.action_digest")?;
    let status_text = match payload.get(&kala_json::JsonText::new("status")) {
        None => Text::new(ApprovalStatus::Pending.as_str()),
        Some(value) => state_str(Some(value), "approval.status")?,
    };
    let status_source = status_text.to_string_lossy();
    let status = ApprovalStatus::parse(&status_source)
        .ok_or_else(|| state_error("approval entry is invalid: approval status is invalid"))?;
    let actor_id = state_opt_str(
        payload.get(&kala_json::JsonText::new("actor_id")),
        "approval.actor_id",
    )?;
    let decided_at = state_opt_datetime(
        payload.get(&kala_json::JsonText::new("decided_at")),
        "approval.decided_at",
    )?;
    let reason = state_opt_str(
        payload.get(&kala_json::JsonText::new("reason")),
        "approval.reason",
    )?;
    ApprovalRecord::from_parts(
        request_id,
        state_digest,
        action_digest,
        status,
        actor_id,
        decided_at,
        reason,
    )
    .map_err(|error| state_error(format!("approval entry is invalid: {}", error.message())))
}

fn grant_from_state(value: &CanonicalValue) -> Result<CapabilityGrant, PolicyError> {
    let payload = state_mapping(value, "grant")?;
    require_keys(payload, GRANT_STATE_KEYS, "grant")?;
    let capability = state_str(
        payload.get(&kala_json::JsonText::new("capability")),
        "grant.capability",
    )?;
    let scope = state_opt_str(
        payload.get(&kala_json::JsonText::new("scope")),
        "grant.scope",
    )?;
    let expires_at = state_opt_datetime(
        payload.get(&kala_json::JsonText::new("expires_at")),
        "grant.expires_at",
    )?;
    CapabilityGrant::from_parts(capability, scope, expires_at)
        .map_err(|error| state_error(format!("grant entry is invalid: {}", error.message())))
}

fn tool_class_from_state(value: &CanonicalValue) -> Result<PolicyToolClass, PolicyError> {
    let payload = state_mapping(value, "tool_class")?;
    require_keys(payload, TOOL_CLASS_STATE_KEYS, "tool_class")?;
    let action_digest = state_str(
        payload.get(&kala_json::JsonText::new("action_digest")),
        "tool_class.action_digest",
    )?;
    let capabilities = state_str_tuple(
        payload.get(&kala_json::JsonText::new("capabilities")),
        "tool_class.capabilities",
    )?;
    let scope = state_opt_str(
        payload.get(&kala_json::JsonText::new("scope")),
        "tool_class.scope",
    )?;
    let action_digest = digest(&action_digest, "action_digest").map_err(|error| {
        state_error(format!("tool class entry is invalid: {}", error.message()))
    })?;
    let scope = match scope {
        Some(scope) => Some(safe_identifier(&scope, "scope").map_err(|error| {
            state_error(format!("tool class entry is invalid: {}", error.message()))
        })?),
        None => None,
    };
    let normalized_capabilities = normalize_capabilities(capabilities)?;
    Ok(PolicyToolClass {
        action_digest,
        capabilities: normalized_capabilities,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(text: &str) -> KernelTimestamp {
        KernelTimestamp::parse_isoformat(text).expect("timestamp")
    }

    fn request(id: &str) -> PolicyRequest {
        PolicyRequest::from_parts(
            Text::new(id),
            Text::new("state-1"),
            Text::new("action-1"),
            vec![Text::new("read")],
            SideEffectLevel::None,
            false,
            false,
            None,
            None,
        )
        .expect("request")
    }

    #[test]
    fn identifiers_use_python_strip_and_isspace() {
        assert_eq!(
            safe_identifier(&Text::new("  padded  "), "request_id")
                .expect("valid")
                .to_string_lossy(),
            "padded"
        );
        assert!(safe_identifier(&Text::new("has space"), "request_id").is_err());
        assert!(safe_identifier(&Text::new(&"x".repeat(257)), "request_id").is_err());
    }

    #[test]
    fn egrees_forces_the_egress_flag() {
        let request = PolicyRequest::from_parts(
            Text::new("r"),
            Text::new("s"),
            Text::new("a"),
            vec![Text::new("network.egress")],
            SideEffectLevel::Egress,
            false,
            false,
            None,
            None,
        )
        .expect("valid");
        assert!(request.egress());
        assert!(request.requires_approval());
    }

    #[test]
    fn exact_grant_duplicates_are_collapsed() {
        let engine = PolicyEngine::from_grants([GrantSpec::from("read"), GrantSpec::from("read")])
            .expect("engine");
        assert_eq!(engine.grants().len(), 1);
    }

    #[test]
    fn approval_keys_bind_exact_digests() {
        let engine = PolicyEngine::from_grants([GrantSpec::from("read")]).expect("engine");
        let first = request("r");
        let second = PolicyRequest::from_parts(
            Text::new("r"),
            Text::new("state-2"),
            Text::new("action-1"),
            vec![Text::new("read")],
            SideEffectLevel::None,
            false,
            false,
            None,
            Some(ApprovalKind::PlanApproval),
        )
        .expect("valid");
        engine
            .request_approval(&first, Some(timestamp("2026-01-02T03:04:05Z")))
            .expect("pending");
        let decision = engine
            .approve(
                &second,
                &DecisionContext::new().with_now(timestamp("2026-01-02T03:04:05Z")),
            )
            .expect("decision");
        assert_eq!(decision.status(), PolicyDecisionStatus::Invalidated);
    }

    #[test]
    fn state_round_trip_is_exact() {
        let now = timestamp("2026-01-02T03:04:05Z");
        let engine = PolicyEngine::fixed(now)
            .with_grants([GrantSpec::from("network.egress")])
            .expect("grants");
        let request = PolicyRequest::from_parts(
            Text::new("request-persisted"),
            Text::new("state-1"),
            Text::new("action-1"),
            vec![Text::new("network.egress")],
            SideEffectLevel::Egress,
            false,
            false,
            None,
            None,
        )
        .expect("valid");
        engine
            .approve(
                &request,
                &DecisionContext::new().with_actor_id(Text::new("reviewer-1")),
            )
            .expect("allowed");
        let state = engine.export_state();
        let json = state.to_json().expect("json");
        let restored = PolicyEngine::from_state_json(&json, Some(now), None).expect("restore");
        assert_eq!(restored.export_state(), state);
        assert!(
            restored
                .decide(&request, Some(now))
                .expect("decision")
                .allowed()
        );
    }

    #[test]
    fn failed_import_leaves_the_engine_unchanged() {
        let engine = PolicyEngine::from_grants([GrantSpec::from("read")]).expect("engine");
        let before = engine.export_state();
        let value = CanonicalValue::object([("version", CanonicalValue::from(2))]);
        assert!(engine.import_state(&value).is_err());
        assert_eq!(engine.export_state(), before);
    }
}
