//! Evaluation contexts and contract result construction.

use kala_contracts::{ContractModel, EvaluationMetric, EvaluationResult, Timestamp};
use kala_json::CanonicalValue;

use crate::error::DomainError;
use crate::parameters::ParameterModel;
use crate::spec::require_identifier;

/// The deterministic reference evaluation epoch, 2026-01-01T00:00:00Z.
pub fn reference_evaluation_epoch() -> Timestamp {
    Timestamp::new(2026, 1, 1, 0, 0, 0, 0, 0).expect("the reference epoch is a valid timestamp")
}

/// Bounded evaluation inputs that are not part of a parameter model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluationContext {
    subject_id: String,
    evaluated_at: Timestamp,
    evaluation_id_prefix: String,
}

impl EvaluationContext {
    /// Build a context and normalize the timestamp to UTC.
    pub fn new(subject_id: &str, evaluated_at: Timestamp) -> Result<Self, DomainError> {
        Self::with_prefix(subject_id, evaluated_at, "eval")
    }

    /// Build a context with an explicit evaluation-id prefix.
    pub fn with_prefix(
        subject_id: &str,
        evaluated_at: Timestamp,
        evaluation_id_prefix: &str,
    ) -> Result<Self, DomainError> {
        let subject_id = require_identifier(subject_id, "subject_id")?;
        let evaluation_id_prefix =
            require_identifier(evaluation_id_prefix, "evaluation_id_prefix")?;
        // Python's `astimezone(UTC)` preserves the instant and rewrites the
        // calendar fields/offset. `from_absolute_micros` does the same.
        let evaluated_at = Timestamp::from_absolute_micros(evaluated_at.absolute_micros())
            .map_err(|_| {
                DomainError::Parameter("evaluated_at must include a timezone".to_owned())
            })?;
        Ok(Self {
            subject_id,
            evaluated_at,
            evaluation_id_prefix,
        })
    }

    /// Build a context at the reference epoch.
    pub fn reference(subject_id: &str) -> Result<Self, DomainError> {
        Self::new(subject_id, reference_evaluation_epoch())
    }

    /// The normalized subject identifier.
    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }
    /// The UTC-normalized evaluation timestamp.
    pub fn evaluated_at(&self) -> &Timestamp {
        &self.evaluated_at
    }
    /// The evaluation-id prefix.
    pub fn evaluation_id_prefix(&self) -> &str {
        &self.evaluation_id_prefix
    }

    /// The deterministic evaluation identifier for a catalog step.
    pub fn evaluation_id(&self, step_id: &str) -> Result<String, DomainError> {
        Ok(format!(
            "{}.{}",
            self.evaluation_id_prefix,
            require_identifier(step_id, "step_id")?
        ))
    }

    /// Build a contract evaluation metric.
    pub fn metric(
        &self,
        name: &str,
        value: f64,
        unit: Option<&str>,
        passed: Option<bool>,
    ) -> Result<EvaluationMetric, DomainError> {
        let payload = CanonicalValue::object([
            ("name", CanonicalValue::text(name)),
            ("value", CanonicalValue::Float(value)),
            (
                "unit",
                unit.map_or(CanonicalValue::Null, CanonicalValue::text),
            ),
            (
                "passed",
                passed.map_or(CanonicalValue::Null, CanonicalValue::Bool),
            ),
        ]);
        EvaluationMetric::from_value(&payload)
            .map_err(|error| DomainError::Package(format!("EvaluationMetric: {error}")))
    }

    /// Build a contract evaluation result.
    pub fn result(
        &self,
        step_id: &str,
        passed: bool,
        score: Option<f64>,
        metrics: &[EvaluationMetric],
        findings: &[String],
    ) -> Result<EvaluationResult, DomainError> {
        let metrics = metrics
            .iter()
            .map(|metric| {
                metric
                    .to_canonical_value()
                    .map_err(|error| DomainError::Package(format!("EvaluationMetric: {error}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let payload = CanonicalValue::object([
            (
                "evaluation_id",
                CanonicalValue::text(self.evaluation_id(step_id)?),
            ),
            ("subject_id", CanonicalValue::text(self.subject_id.as_str())),
            ("requirement_id", CanonicalValue::Null),
            ("passed", CanonicalValue::Bool(passed)),
            (
                "score",
                score.map_or(CanonicalValue::Null, CanonicalValue::Float),
            ),
            ("metrics", CanonicalValue::Array(metrics)),
            (
                "findings",
                CanonicalValue::Array(
                    findings
                        .iter()
                        .map(|finding| CanonicalValue::text(finding.as_str()))
                        .collect(),
                ),
            ),
            ("artifact_ids", CanonicalValue::Array(Vec::new())),
            (
                "evaluated_at",
                CanonicalValue::text(self.evaluated_at.to_wire_string()),
            ),
        ]);
        EvaluationResult::from_value(&payload)
            .map_err(|error| DomainError::Package(format!("EvaluationResult: {error}")))
    }
}

/// Build a deterministic reference context for a subject.
pub fn reference_context(subject_id: &str) -> Result<EvaluationContext, DomainError> {
    EvaluationContext::reference(subject_id)
}

/// Parse a package's typed parameters from an untyped JSON value.
pub(crate) fn parse_parameters<P: ParameterModel>(
    value: &CanonicalValue,
) -> Result<P, crate::error::ParameterError> {
    P::from_value(value)
}
