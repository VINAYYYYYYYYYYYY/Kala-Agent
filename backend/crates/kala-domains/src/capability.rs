//! Domain capability requests, policy grants, and denial before evaluation.

use crate::error::DomainError;
use crate::package::DomainPackage;
use crate::parameters::ParameterModel;
use crate::spec::{require_capabilities, require_identifier};

/// The domain capabilities one reference workflow run requires up front.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequest {
    package_id: String,
    step_ids: Vec<String>,
    required_capabilities: Vec<String>,
}

impl CapabilityRequest {
    /// Build and validate a capability request.
    pub fn new(
        package_id: &str,
        step_ids: &[String],
        required_capabilities: &[String],
    ) -> Result<Self, DomainError> {
        let package_id = require_identifier(package_id, "capability request package_id")?;
        if step_ids.is_empty() {
            return Err(DomainError::Parameter(
                "a capability request must declare at least one step".to_owned(),
            ));
        }
        let mut normalized_steps = Vec::with_capacity(step_ids.len());
        for step_id in step_ids {
            normalized_steps.push(require_identifier(step_id, "capability request step_id")?);
        }
        Ok(Self {
            package_id,
            step_ids: normalized_steps,
            required_capabilities: require_capabilities(
                required_capabilities.iter().map(String::as_str),
                "capability request capability",
            )?,
        })
    }

    /// The package the request belongs to.
    pub fn package_id(&self) -> &str {
        &self.package_id
    }
    /// The requested catalog steps, in request order.
    pub fn step_ids(&self) -> &[String] {
        &self.step_ids
    }
    /// The canonical sorted required capabilities.
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }
}

/// A domain-neutral decision point for the capabilities a run requires.
pub trait CapabilityGrantPolicy {
    /// Answer with the grants this policy allows for the request.
    fn granted_capabilities(&self, request: &CapabilityRequest) -> Vec<String>;
}

/// Refuse an evaluation unless its grants equal what the package declares.
pub fn enforce_capability_request<P: ParameterModel>(
    package: &DomainPackage<P>,
    step_ids: &[String],
    policy: Option<&dyn CapabilityGrantPolicy>,
) -> Result<CapabilityRequest, DomainError> {
    for step_id in step_ids {
        package.catalog_step(step_id)?;
    }
    let request = capability_request(package, step_ids)?;
    require_capability_grants(&request, policy)?;
    Ok(request)
}

fn capability_request<P: ParameterModel>(
    package: &DomainPackage<P>,
    step_ids: &[String],
) -> Result<CapabilityRequest, DomainError> {
    let mut required: std::collections::BTreeSet<String> =
        package.required_capabilities().iter().cloned().collect();
    for step_id in step_ids {
        required.extend(
            package
                .catalog_step(step_id)?
                .required_capabilities()
                .iter()
                .cloned(),
        );
    }
    let required: Vec<String> = required.into_iter().collect();
    CapabilityRequest::new(package.package_id(), step_ids, &required)
}

fn require_capability_grants(
    request: &CapabilityRequest,
    policy: Option<&dyn CapabilityGrantPolicy>,
) -> Result<(), DomainError> {
    let Some(policy) = policy else {
        if request.required_capabilities().is_empty() {
            return Ok(());
        }
        return Err(DomainError::Capability(format!(
            "{} declares capabilities that no policy granted: {}",
            request.package_id(),
            request.required_capabilities().join(", ")
        )));
    };
    let granted = require_capabilities(
        policy
            .granted_capabilities(request)
            .iter()
            .map(String::as_str),
        "granted capability",
    )?;
    let missing: Vec<&str> = request
        .required_capabilities()
        .iter()
        .filter(|capability| !granted.contains(capability))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        return Err(DomainError::Capability(format!(
            "{} declares capabilities that are not granted: {}",
            request.package_id(),
            missing.join(", ")
        )));
    }
    let extra: Vec<&str> = granted
        .iter()
        .filter(|capability| !request.required_capabilities().contains(capability))
        .map(String::as_str)
        .collect();
    if !extra.is_empty() {
        return Err(DomainError::Capability(format!(
            "the granted capabilities are not the capabilities {} declares: {}",
            request.package_id(),
            extra.join(", ")
        )));
    }
    Ok(())
}
