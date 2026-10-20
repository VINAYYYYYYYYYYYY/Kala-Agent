//! Domain packages: declaration validation, evaluator identity binding,
//! manifests, and untyped registry dispatch.

use kala_contracts::{
    ApprovalKind, ContractModel, DomainPackageManifest, EvaluationResult, WorkflowDefinition,
};
use kala_json::CanonicalValue;

use crate::error::{DomainCallError, DomainError};
use crate::parameters::ParameterModel;
use crate::provenance::EvaluatorImplementation;
use crate::spec::{
    CatalogStep, canonical_digest, is_python_blank, require_capabilities, require_digest,
    require_identifier,
};
use crate::{EvaluationContext, context::parse_parameters};

/// The common evaluator interface implemented by every domain package.
pub trait DomainEvaluator<P>: Send + Sync {
    /// The catalog step this evaluator is bound to.
    fn step_id(&self) -> &str;
    /// The capabilities this evaluator declares.
    fn required_capabilities(&self) -> &[String];
    /// The identified native implementation behind this evaluator.
    fn implementation(&self) -> Result<EvaluatorImplementation, DomainError>;
    /// Evaluate one parameter model.
    fn evaluate(
        &self,
        context: &EvaluationContext,
        parameters: &P,
    ) -> Result<EvaluationResult, DomainError>;
}

/// The declarative inputs to a new domain package.
pub struct DomainPackageParts<P> {
    /// Package identifier.
    pub package_id: String,
    /// Package version.
    pub version: String,
    /// Package description.
    pub description: String,
    /// Package disclaimer.
    pub disclaimer: String,
    /// The declared workflow state machine.
    pub workflow: WorkflowDefinition,
    /// Catalog steps in workflow order.
    pub steps: Vec<CatalogStep>,
    /// One evaluator per catalog step.
    pub evaluators: Vec<Box<dyn DomainEvaluator<P>>>,
    /// Parameter-schema digests.
    pub schema_digests: Vec<String>,
    /// Package-level required capabilities.
    pub required_capabilities: Vec<String>,
}

/// A versioned, inspectable domain package declaration.
pub struct DomainPackage<P> {
    package_id: String,
    version: String,
    description: String,
    disclaimer: String,
    workflow: WorkflowDefinition,
    steps: Vec<CatalogStep>,
    evaluators: Vec<Box<dyn DomainEvaluator<P>>>,
    schema_digests: Vec<String>,
    required_capabilities: Vec<String>,
    evaluator_implementations: Vec<EvaluatorImplementation>,
    step_ids: Vec<String>,
    package_payload: CanonicalValue,
    package_digest: String,
    manifest: DomainPackageManifest,
}

impl<P: ParameterModel> DomainPackage<P> {
    /// Validate a package declaration and bind evaluator identities.
    pub fn new(parts: DomainPackageParts<P>) -> Result<Self, DomainError> {
        let package_id = require_identifier(&parts.package_id, "package_id")?;
        if is_python_blank(&parts.version) {
            return Err(DomainError::Package(
                "a domain package must declare a version".to_owned(),
            ));
        }
        if is_python_blank(&parts.description) || is_python_blank(&parts.disclaimer) {
            return Err(DomainError::Package(
                "a domain package must be described and disclaimed".to_owned(),
            ));
        }
        let required_capabilities = require_capabilities(
            parts.required_capabilities.iter().map(String::as_str),
            "package required capability",
        )?;
        validate_workflow(&parts.workflow)?;
        if parts.steps.is_empty() || parts.evaluators.is_empty() {
            return Err(DomainError::Package(
                "a domain package must declare steps and evaluators".to_owned(),
            ));
        }
        let step_ids: Vec<String> = parts
            .steps
            .iter()
            .map(|step| step.step_id().to_owned())
            .collect();
        let unique_steps: std::collections::BTreeSet<&str> =
            step_ids.iter().map(String::as_str).collect();
        if unique_steps.len() != step_ids.len() {
            return Err(DomainError::Package(
                "catalog step ids must be unique".to_owned(),
            ));
        }
        let available: std::collections::BTreeSet<&str> = parts
            .workflow
            .available_step_ids()
            .iter()
            .map(|value| value.as_str())
            .collect();
        if unique_steps != available {
            return Err(DomainError::Package(
                "catalog steps must match the workflow available step ids exactly".to_owned(),
            ));
        }
        let evaluator_ids: Vec<&str> = parts
            .evaluators
            .iter()
            .map(|evaluator| evaluator.step_id())
            .collect();
        let unique_evaluators: std::collections::BTreeSet<&str> =
            evaluator_ids.iter().copied().collect();
        if unique_evaluators.len() != evaluator_ids.len() {
            return Err(DomainError::Package(
                "each catalog step needs exactly one evaluator".to_owned(),
            ));
        }
        if unique_evaluators != unique_steps {
            return Err(DomainError::Package(
                "every catalog step must have an evaluator".to_owned(),
            ));
        }
        if parts.schema_digests.is_empty() {
            return Err(DomainError::Package(
                "a domain package must publish schema digests".to_owned(),
            ));
        }
        let mut schema_digests = Vec::with_capacity(parts.schema_digests.len());
        for digest in &parts.schema_digests {
            schema_digests.push(require_digest(digest, "schema digest")?);
        }
        validate_checkpoints(&parts.steps, &parts.workflow)?;
        let evaluator_implementations =
            identify_evaluators(&package_id, &parts.steps, &parts.evaluators)?;

        let package_payload = build_package_payload(
            &package_id,
            &parts.version,
            &parts.description,
            &parts.disclaimer,
            &parts.workflow,
            &parts.steps,
            &evaluator_implementations,
            &schema_digests,
            &required_capabilities,
        )?;
        let package_digest = canonical_digest(&package_payload)?;
        let manifest_schema_digests =
            manifest_schema_digests(&schema_digests, &evaluator_implementations);
        let manifest = build_manifest(
            &package_id,
            &parts.version,
            &parts.description,
            &parts.workflow,
            &manifest_schema_digests,
            &required_capabilities,
            &package_digest,
        )?;

        Ok(Self {
            package_id,
            version: parts.version,
            description: parts.description,
            disclaimer: parts.disclaimer,
            workflow: parts.workflow,
            steps: parts.steps,
            evaluators: parts.evaluators,
            schema_digests,
            required_capabilities,
            evaluator_implementations,
            step_ids,
            package_payload,
            package_digest,
            manifest,
        })
    }

    /// The package identifier.
    pub fn package_id(&self) -> &str {
        &self.package_id
    }
    /// The package version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// The package description.
    pub fn description(&self) -> &str {
        &self.description
    }
    /// The package disclaimer.
    pub fn disclaimer(&self) -> &str {
        &self.disclaimer
    }
    /// The declared workflow.
    pub fn workflow(&self) -> &WorkflowDefinition {
        &self.workflow
    }
    /// Catalog steps in workflow order.
    pub fn steps(&self) -> &[CatalogStep] {
        &self.steps
    }
    /// Declared catalog step ids in workflow order.
    pub fn step_ids(&self) -> &[String] {
        &self.step_ids
    }
    /// Parameter-schema digests in declaration order.
    pub fn schema_digests(&self) -> &[String] {
        &self.schema_digests
    }
    /// Canonical package-level capabilities.
    pub fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }
    /// Bound evaluator implementations, sorted by step id.
    pub fn evaluator_implementations(&self) -> &[EvaluatorImplementation] {
        &self.evaluator_implementations
    }
    /// Bound implementation digests in workflow order.
    pub fn evaluator_implementation_digests(&self) -> Vec<&str> {
        let mut digests = Vec::with_capacity(self.step_ids.len());
        for step_id in &self.step_ids {
            let implementation = self
                .evaluator_implementations
                .iter()
                .find(|item| item.step_id() == step_id)
                .expect("every step has a validated implementation");
            digests.push(implementation.digest());
        }
        digests
    }
    /// The canonical package payload.
    pub fn package_payload(&self) -> &CanonicalValue {
        &self.package_payload
    }
    /// The native package digest.
    pub fn digest(&self) -> &str {
        &self.package_digest
    }
    /// The contract manifest.
    pub fn manifest(&self) -> &DomainPackageManifest {
        &self.manifest
    }
    /// Manifest schema digests: schema digests plus implementation digests,
    /// sorted and duplicate-free.
    pub fn manifest_schema_digests(&self) -> Vec<String> {
        manifest_schema_digests(&self.schema_digests, &self.evaluator_implementations)
    }

    /// Return a declared catalog step.
    pub fn catalog_step(&self, step_id: &str) -> Result<&CatalogStep, DomainError> {
        self.steps
            .iter()
            .find(|step| step.step_id() == step_id)
            .ok_or_else(|| {
                DomainError::Step(format!("{step_id} is not declared by {}", self.package_id))
            })
    }

    /// Return the evaluator bound to a declared catalog step.
    pub fn evaluator(&self, step_id: &str) -> Result<&dyn DomainEvaluator<P>, DomainError> {
        self.evaluators
            .iter()
            .find(|evaluator| evaluator.step_id() == step_id)
            .map(|evaluator| evaluator.as_ref())
            .ok_or_else(|| {
                DomainError::Step(format!("{step_id} has no evaluator in {}", self.package_id))
            })
    }

    /// Run the evaluator bound to a declared catalog step.
    pub fn evaluate(
        &self,
        step_id: &str,
        context: &EvaluationContext,
        parameters: &P,
    ) -> Result<EvaluationResult, DomainError> {
        self.evaluator(step_id)?.evaluate(context, parameters)
    }
}

/// Object-safe package access for the domain-name registry.
pub trait DomainPackageApi: Send + Sync {
    /// The registry domain name (`mechanical`, `building`, `wing`).
    fn domain(&self) -> &'static str;
    /// The package identifier.
    fn package_id(&self) -> &str;
    /// The package version.
    fn version(&self) -> &str;
    /// The package description.
    fn description(&self) -> &str;
    /// The package disclaimer.
    fn disclaimer(&self) -> &str;
    /// The declared workflow.
    fn workflow(&self) -> &WorkflowDefinition;
    /// Declared catalog steps.
    fn steps(&self) -> &[CatalogStep];
    /// Declared step ids in workflow order.
    fn step_ids(&self) -> &[String];
    /// Parameter-schema digests.
    fn schema_digests(&self) -> &[String];
    /// Package-level required capabilities.
    fn required_capabilities(&self) -> &[String];
    /// Manifest schema digests: schema digests plus implementation digests.
    fn manifest_schema_digests(&self) -> Vec<String>;
    /// Bound evaluator implementations.
    fn evaluator_implementations(&self) -> &[EvaluatorImplementation];
    /// Native package digest.
    fn digest(&self) -> &str;
    /// Contract manifest.
    fn manifest(&self) -> &DomainPackageManifest;
    /// Canonical package payload.
    fn package_payload(&self) -> &CanonicalValue;
    /// Parse untyped JSON parameters and evaluate one step.
    fn evaluate_value(
        &self,
        step_id: &str,
        context: &EvaluationContext,
        parameters: &CanonicalValue,
    ) -> Result<EvaluationResult, DomainCallError>;
}

impl<P: ParameterModel> DomainPackage<P> {
    /// Parse an untyped JSON parameter value for this package.
    pub fn parameters_from_value(
        &self,
        value: &CanonicalValue,
    ) -> Result<P, crate::error::ParameterError> {
        parse_parameters::<P>(value)
    }
}

fn validate_workflow(workflow: &WorkflowDefinition) -> Result<(), DomainError> {
    // The accepted contract type is constructed only through validation, and
    // its wire form is the same shape Python re-validates in
    // `DomainPackage._validate_workflow`.
    let wire = workflow.to_canonical_value().map_err(|_| {
        DomainError::Package(
            "the declared workflow definition does not satisfy the contract".to_owned(),
        )
    })?;
    WorkflowDefinition::from_value(&wire).map_err(|_| {
        DomainError::Package(
            "the declared workflow definition does not satisfy the contract".to_owned(),
        )
    })?;
    Ok(())
}

fn validate_checkpoints(
    steps: &[CatalogStep],
    workflow: &WorkflowDefinition,
) -> Result<(), DomainError> {
    let mut transitions: std::collections::HashMap<&str, Option<ApprovalKind>> =
        std::collections::HashMap::new();
    for transition in workflow.transitions() {
        transitions.insert(transition.step_id(), transition.checkpoint_type());
    }
    for step in steps {
        let declared = step.checkpoint_kind();
        match transitions.get(step.step_id()) {
            None => {
                if declared.is_some() {
                    return Err(DomainError::Package(format!(
                        "{} declares a checkpoint that no workflow transition carries",
                        step.step_id()
                    )));
                }
            }
            Some(carried) => {
                if *carried != declared {
                    return Err(DomainError::Package(format!(
                        "{} declares a checkpoint kind that its workflow transition does not carry",
                        step.step_id()
                    )));
                }
            }
        }
    }
    Ok(())
}

fn identify_evaluators<P>(
    package_id: &str,
    steps: &[CatalogStep],
    evaluators: &[Box<dyn DomainEvaluator<P>>],
) -> Result<Vec<EvaluatorImplementation>, DomainError> {
    let mut implementations = Vec::with_capacity(evaluators.len());
    for evaluator in evaluators {
        let implementation = evaluator.implementation()?;
        let step = steps
            .iter()
            .find(|step| step.step_id() == implementation.step_id())
            .ok_or_else(|| {
                DomainError::Step(format!(
                    "{} is not declared by {package_id}",
                    implementation.step_id()
                ))
            })?;
        if step.required_capabilities() != implementation.required_capabilities() {
            return Err(DomainError::Package(format!(
                "{} must declare the same capabilities on its catalog step and on its evaluator",
                step.step_id()
            )));
        }
        implementations.push(implementation);
    }
    implementations.sort_by(|left, right| left.step_id().cmp(right.step_id()));
    Ok(implementations)
}

#[allow(clippy::too_many_arguments)]
fn build_package_payload(
    package_id: &str,
    version: &str,
    description: &str,
    disclaimer: &str,
    workflow: &WorkflowDefinition,
    steps: &[CatalogStep],
    evaluator_implementations: &[EvaluatorImplementation],
    schema_digests: &[String],
    required_capabilities: &[String],
) -> Result<CanonicalValue, DomainError> {
    Ok(CanonicalValue::object([
        ("description", CanonicalValue::text(description)),
        ("disclaimer", CanonicalValue::text(disclaimer)),
        (
            "evaluators",
            CanonicalValue::Array(
                evaluator_implementations
                    .iter()
                    .map(EvaluatorImplementation::as_canonical_value)
                    .collect(),
            ),
        ),
        ("package_id", CanonicalValue::text(package_id)),
        (
            "required_capabilities",
            CanonicalValue::Array(
                required_capabilities
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_str()))
                    .collect(),
            ),
        ),
        (
            "schema_digests",
            CanonicalValue::Array(
                schema_digests
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_str()))
                    .collect(),
            ),
        ),
        (
            "steps",
            CanonicalValue::Array(steps.iter().map(CatalogStep::as_canonical_value).collect()),
        ),
        ("version", CanonicalValue::text(version)),
        (
            "workflow",
            workflow
                .to_canonical_value()
                .map_err(|error| DomainError::Package(format!("workflow: {error}")))?,
        ),
    ]))
}

fn manifest_schema_digests(
    schema_digests: &[String],
    implementations: &[EvaluatorImplementation],
) -> Vec<String> {
    let mut digests: std::collections::BTreeSet<String> = schema_digests.iter().cloned().collect();
    digests.extend(
        implementations
            .iter()
            .map(|implementation| implementation.digest().to_owned()),
    );
    digests.into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn build_manifest(
    package_id: &str,
    version: &str,
    description: &str,
    workflow: &WorkflowDefinition,
    schema_digests: &[String],
    required_capabilities: &[String],
    package_digest: &str,
) -> Result<DomainPackageManifest, DomainError> {
    let payload = CanonicalValue::object([
        ("package_id", CanonicalValue::text(package_id)),
        ("version", CanonicalValue::text(version)),
        ("digest", CanonicalValue::text(package_digest)),
        ("description", CanonicalValue::text(description)),
        (
            "workflow_definition_ids",
            CanonicalValue::Array(vec![CanonicalValue::text(workflow.definition_id())]),
        ),
        ("tool_ids", CanonicalValue::Array(Vec::new())),
        (
            "schema_digests",
            CanonicalValue::Array(
                schema_digests
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_str()))
                    .collect(),
            ),
        ),
        (
            "required_capabilities",
            CanonicalValue::Array(
                required_capabilities
                    .iter()
                    .map(|value| CanonicalValue::text(value.as_str()))
                    .collect(),
            ),
        ),
        ("policy_overlay_digest", CanonicalValue::Null),
        ("signer_id", CanonicalValue::Null),
        ("signature_digest", CanonicalValue::Null),
    ]);
    DomainPackageManifest::from_value(&payload)
        .map_err(|error| DomainError::Package(format!("DomainPackageManifest: {error}")))
}

impl<P: ParameterModel> DomainPackageApi for DomainPackage<P> {
    fn domain(&self) -> &'static str {
        P::DOMAIN_NAME
    }
    fn package_id(&self) -> &str {
        &self.package_id
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn disclaimer(&self) -> &str {
        &self.disclaimer
    }
    fn workflow(&self) -> &WorkflowDefinition {
        &self.workflow
    }
    fn steps(&self) -> &[CatalogStep] {
        &self.steps
    }
    fn step_ids(&self) -> &[String] {
        &self.step_ids
    }
    fn schema_digests(&self) -> &[String] {
        &self.schema_digests
    }
    fn required_capabilities(&self) -> &[String] {
        &self.required_capabilities
    }
    fn manifest_schema_digests(&self) -> Vec<String> {
        manifest_schema_digests(&self.schema_digests, &self.evaluator_implementations)
    }
    fn evaluator_implementations(&self) -> &[EvaluatorImplementation] {
        &self.evaluator_implementations
    }
    fn digest(&self) -> &str {
        &self.package_digest
    }
    fn manifest(&self) -> &DomainPackageManifest {
        &self.manifest
    }
    fn package_payload(&self) -> &CanonicalValue {
        &self.package_payload
    }
    fn evaluate_value(
        &self,
        step_id: &str,
        context: &EvaluationContext,
        parameters: &CanonicalValue,
    ) -> Result<EvaluationResult, DomainCallError> {
        let parameters = parse_parameters::<P>(parameters).map_err(DomainCallError::Parameters)?;
        self.evaluate(step_id, context, &parameters)
            .map_err(DomainCallError::Domain)
    }
}
