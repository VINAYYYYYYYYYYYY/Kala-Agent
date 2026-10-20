//! The domain bindings the composition root evaluates.
//!
//! Every binding names one accepted package from `kala-domains` and the
//! reference parameters, step order and subject identifier the composition
//! root evaluates it with. The accepted crate publishes exactly three
//! packages, so a binding can only name one of those three; an arbitrary
//! package would need to be accepted into `kala-domains` first.

use kala_contracts::EvaluationResult;
use kala_domains::{
    DomainCallError, DomainName, DomainPackageApi, DomainParameters, ParameterModel,
    building::{self, BUILDING_PACKAGE, BuildingParameters},
    mechanical::{self, BracketParameters, MECHANICAL_PACKAGE},
    wing::{self, WING_PACKAGE, WingParameters},
};
use kala_json::CanonicalValue;

use crate::dto::DomainEvaluationReport;
use crate::error::{ApplicationError, ApplicationResult};
use crate::value::{MAX_IDENTIFIER_LENGTH, canonical_digest, require_identifier};

/// `MAX_ATTRIBUTE_COUNT`: the bounded parameter-entry count.
pub const MAX_ATTRIBUTE_COUNT: usize = 4_096;

/// Everything the composition root needs to evaluate one domain package.
#[derive(Clone)]
pub struct DomainBinding {
    domain: DomainName,
    package: &'static dyn DomainPackageApi,
    reference_parameters: DomainParameters,
    reference_step_ids: Vec<String>,
    subject_id: String,
}

impl std::fmt::Debug for DomainBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DomainBinding")
            .field("domain", &self.domain.as_str())
            .field("package_id", &self.package.package_id())
            .field("reference_step_ids", &self.reference_step_ids)
            .field("subject_id", &self.subject_id)
            .finish()
    }
}

impl DomainBinding {
    /// Bind one accepted package with its declared reference defaults.
    ///
    /// The reference subject and step order come from the accepted package
    /// module, exactly as `_default_domains()` reads them.
    #[must_use]
    pub fn new(domain: DomainName) -> Self {
        match domain {
            DomainName::Building => Self {
                domain,
                package: &*BUILDING_PACKAGE,
                reference_parameters: DomainParameters::Building(BuildingParameters::default()),
                reference_step_ids: building::REFERENCE_WORKFLOW_STEP_IDS.to_vec(),
                subject_id: building::REFERENCE_SUBJECT_ID.to_owned(),
            },
            DomainName::Mechanical => Self {
                domain,
                package: &*MECHANICAL_PACKAGE,
                reference_parameters: DomainParameters::Mechanical(BracketParameters::default()),
                reference_step_ids: mechanical::REFERENCE_WORKFLOW_STEP_IDS.to_vec(),
                subject_id: mechanical::REFERENCE_SUBJECT_ID.to_owned(),
            },
            DomainName::Wing => Self {
                domain,
                package: &*WING_PACKAGE,
                reference_parameters: DomainParameters::Wing(WingParameters::default()),
                reference_step_ids: wing::REFERENCE_WORKFLOW_STEP_IDS.to_vec(),
                subject_id: wing::REFERENCE_SUBJECT_ID.to_owned(),
            },
        }
    }

    /// The three declared bindings, in the application's stable order.
    #[must_use]
    pub fn defaults() -> Vec<Self> {
        DomainName::ALL.into_iter().map(Self::new).collect()
    }

    /// Replace the reference parameters, validating they belong to the domain.
    ///
    /// # Errors
    ///
    /// Refuses parameters of another domain.
    pub fn with_reference_parameters(
        mut self,
        parameters: DomainParameters,
    ) -> ApplicationResult<Self> {
        if parameters.domain() != self.domain {
            return Err(ApplicationError::invalid_parameters(format!(
                "the reference parameters of {} must belong to {}",
                self.domain.as_str(),
                self.domain.as_str()
            )));
        }
        self.reference_parameters = parameters;
        Ok(self)
    }

    /// Replace the reference step order.
    pub fn with_reference_step_ids(mut self, step_ids: Vec<String>) -> ApplicationResult<Self> {
        for step_id in &step_ids {
            require_identifier(step_id, "step_id", MAX_IDENTIFIER_LENGTH)?;
        }
        self.reference_step_ids = step_ids;
        Ok(self)
    }

    /// Replace the reference subject identifier.
    pub fn with_subject_id(mut self, subject_id: &str) -> ApplicationResult<Self> {
        self.subject_id = require_identifier(subject_id, "subject_id", MAX_IDENTIFIER_LENGTH)?;
        Ok(self)
    }

    /// The registry domain name.
    #[must_use]
    pub const fn domain(&self) -> DomainName {
        self.domain
    }

    /// The accepted package.
    #[must_use]
    pub const fn package(&self) -> &'static dyn DomainPackageApi {
        self.package
    }

    /// The reference parameter model.
    #[must_use]
    pub const fn reference_parameters(&self) -> &DomainParameters {
        &self.reference_parameters
    }

    /// The reference step order.
    #[must_use]
    pub fn reference_step_ids(&self) -> &[String] {
        &self.reference_step_ids
    }

    /// The reference subject identifier.
    #[must_use]
    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    /// `DomainBinding.parameters`: the reference model or validated overrides.
    ///
    /// # Errors
    ///
    /// Refuses a non-object, an over-large object, or parameters Pydantic
    /// would refuse, with the same summary text the Python root reports.
    pub fn parameters(
        &self,
        overrides: Option<&CanonicalValue>,
    ) -> ApplicationResult<DomainParameters> {
        let Some(overrides) = overrides else {
            return Ok(self.reference_parameters.clone());
        };
        let CanonicalValue::Object(entries) = overrides else {
            return Err(ApplicationError::invalid_parameters(format!(
                "{} parameters must be a JSON object",
                self.domain.as_str()
            )));
        };
        if entries.len() > MAX_ATTRIBUTE_COUNT {
            return Err(ApplicationError::invalid_parameters(format!(
                "{} parameters exceed the bounded entry count",
                self.domain.as_str()
            )));
        }
        DomainParameters::from_value(self.domain, overrides).map_err(|error| match error {
            DomainCallError::Parameters(error) => {
                ApplicationError::invalid_parameters(error.summary())
            }
            DomainCallError::Domain(error) => {
                ApplicationError::invalid_parameters(error.message().to_owned())
            }
        })
    }

    /// `_evaluate`: run the declared reference workflow over one model.
    pub(crate) fn evaluate(
        &self,
        subject_id: &str,
        parameters: &DomainParameters,
    ) -> ApplicationResult<DomainEvaluationReport> {
        let context = kala_domains::reference_context(subject_id).map_err(|error| {
            ApplicationError::application(format!("the reference context is invalid: {error}"))
        })?;
        match (self.domain, parameters) {
            (DomainName::Building, DomainParameters::Building(model)) => self.report(
                kala_domains::run_reference_workflow(
                    &BUILDING_PACKAGE,
                    &context,
                    model,
                    &self.reference_step_ids,
                    None,
                ),
                parameters,
                subject_id,
            ),
            (DomainName::Mechanical, DomainParameters::Mechanical(model)) => self.report(
                kala_domains::run_reference_workflow(
                    &MECHANICAL_PACKAGE,
                    &context,
                    model,
                    &self.reference_step_ids,
                    None,
                ),
                parameters,
                subject_id,
            ),
            (DomainName::Wing, DomainParameters::Wing(model)) => self.report(
                kala_domains::run_reference_workflow(
                    &WING_PACKAGE,
                    &context,
                    model,
                    &self.reference_step_ids,
                    None,
                ),
                parameters,
                subject_id,
            ),
            _ => Err(ApplicationError::invalid_parameters(format!(
                "{} parameters must belong to {}",
                self.domain.as_str(),
                self.domain.as_str()
            ))),
        }
    }

    fn report(
        &self,
        run: Result<kala_domains::ReferenceWorkflowRun, kala_domains::DomainError>,
        parameters: &DomainParameters,
        subject_id: &str,
    ) -> ApplicationResult<DomainEvaluationReport> {
        let run = run.map_err(|error| ApplicationError::application(error.message().to_owned()))?;
        let parameters_digest = canonical_digest(&parameters_value(parameters)?)?;
        Ok(DomainEvaluationReport {
            domain: self.domain.as_str().to_owned(),
            package_id: run.package_id.clone(),
            package_version: self.package.version().to_owned(),
            package_digest: self.package.digest().to_owned(),
            subject_id: subject_id.to_owned(),
            status: run.status,
            passed: run.passed(),
            score: run.score(),
            parameters_digest,
            completed_step_ids: run.completed_step_ids.clone(),
            blocked_step_id: run.blocked_step_id.clone(),
            evaluations: run.evaluations().into_iter().cloned().collect(),
            findings: run.findings(),
            disclaimer: self.package.disclaimer().to_owned(),
        })
    }

    /// `_evaluate_step`: run one declared mechanical catalog step.
    pub(crate) fn evaluate_step(
        &self,
        step_id: &str,
        subject_id: &str,
    ) -> ApplicationResult<EvaluationResult> {
        let context = kala_domains::reference_context(subject_id).map_err(|error| {
            ApplicationError::application(format!("the reference context is invalid: {error}"))
        })?;
        let step_ids = [step_id.to_owned()];
        match self.domain {
            DomainName::Building => {
                kala_domains::enforce_capability_request(&BUILDING_PACKAGE, &step_ids, None)
            }
            DomainName::Mechanical => {
                kala_domains::enforce_capability_request(&MECHANICAL_PACKAGE, &step_ids, None)
            }
            DomainName::Wing => {
                kala_domains::enforce_capability_request(&WING_PACKAGE, &step_ids, None)
            }
        }
        .map_err(|error| ApplicationError::application(error.message().to_owned()))?;
        self.package
            .evaluate_value(
                step_id,
                &context,
                &parameters_value(&self.reference_parameters)?,
            )
            .map_err(|error| match error {
                DomainCallError::Parameters(error) => {
                    ApplicationError::invalid_parameters(error.summary())
                }
                DomainCallError::Domain(error) => {
                    ApplicationError::application(error.message().to_owned())
                }
            })
    }
}

/// The `model_dump(mode="json")` value of a validated parameter model.
pub fn parameters_value(parameters: &DomainParameters) -> ApplicationResult<CanonicalValue> {
    parameters
        .to_wire_value()
        .to_canonical_value()
        .map_err(|error| ApplicationError::application(error.to_string()))
}

/// The parameter model class name, for diagnostics.
#[must_use]
pub fn parameter_model_name(domain: DomainName) -> &'static str {
    match domain {
        DomainName::Building => BuildingParameters::MODEL_NAME,
        DomainName::Mechanical => BracketParameters::MODEL_NAME,
        DomainName::Wing => WingParameters::MODEL_NAME,
    }
}
