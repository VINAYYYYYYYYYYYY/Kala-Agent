//! Deterministic reference workflow execution.

use kala_contracts::{EvaluationResult, StepStatus, WorkflowStatus};

use crate::capability::{CapabilityGrantPolicy, enforce_capability_request};
use crate::context::EvaluationContext;
use crate::error::DomainError;
use crate::package::DomainPackage;
use crate::parameters::ParameterModel;

/// The outcome of one catalog step inside a reference workflow run.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOutcome {
    /// The catalog step id.
    pub step_id: String,
    /// The step status.
    pub status: StepStatus,
    /// The evaluation produced by the step, when it ran.
    pub evaluation: Option<EvaluationResult>,
    /// The failure reason for a hard-constraint stop.
    pub reason: Option<String>,
}

/// A deterministic, bounded run of a package's reference workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceWorkflowRun {
    /// The package the run belongs to.
    pub package_id: String,
    /// The workflow definition id.
    pub definition_id: String,
    /// The workflow definition version.
    pub definition_version: String,
    /// The run status.
    pub status: WorkflowStatus,
    /// The requested step order.
    pub ordered_step_ids: Vec<String>,
    /// The steps that completed before any stop.
    pub completed_step_ids: Vec<String>,
    /// One outcome per executed step.
    pub outcomes: Vec<StepOutcome>,
    /// The step that stopped the run, when one did.
    pub blocked_step_id: Option<String>,
}

impl ReferenceWorkflowRun {
    /// Every evaluation produced by the run, in execution order.
    pub fn evaluations(&self) -> Vec<&EvaluationResult> {
        self.outcomes
            .iter()
            .filter_map(|outcome| outcome.evaluation.as_ref())
            .collect()
    }

    /// Whether every declared step satisfied its hard constraints.
    pub fn passed(&self) -> bool {
        self.status == WorkflowStatus::Completed
    }

    /// Every finding produced by the run, in execution order.
    pub fn findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        for evaluation in self.evaluations() {
            for finding in evaluation.findings() {
                findings.push(finding.to_string());
            }
        }
        findings
    }

    /// The mean bounded score of the run, or `None` when unscored.
    pub fn score(&self) -> Option<f64> {
        let scores: Vec<f64> = self
            .evaluations()
            .iter()
            .filter_map(|evaluation| evaluation.score())
            .collect();
        if scores.is_empty() {
            return None;
        }
        // Python's builtin `sum` uses Neumaier compensated summation for
        // floats (CPython 3.12+). A naive left-to-right accumulator is not
        // equivalent: `sum([0.356107, 0.301885, 0.49567, 0.042336]) / 4`
        // rounds to 0.299 in Python but to 0.298999 with a naive accumulator.
        // Mirror `CompensatedSum::cs_add`/`cs_to_double` exactly, starting
        // from the integer zero that also collapses a leading negative zero.
        let total = cpython_sum_floats(&scores);
        crate::spec::clamp_unit_interval(total / scores.len() as f64).ok()
    }
}

/// CPython's compensated float summation (`sum()` in 3.12 and later).
fn cpython_sum_floats(values: &[f64]) -> f64 {
    let mut hi = 0.0;
    let mut lo = 0.0;
    for value in values {
        let total = hi + value;
        if hi.abs() >= value.abs() {
            lo += (hi - total) + value;
        } else {
            lo += (value - total) + hi;
        }
        hi = total;
    }
    // `cs_to_double`: keep the high part when the compensation is zero or
    // non-finite, so an overflowed total cannot become NaN.
    if lo != 0.0 && lo.is_finite() {
        hi + lo
    } else {
        hi
    }
}

/// Run declared steps in order and stop at the first hard-constraint failure.
pub fn run_reference_workflow<P: ParameterModel>(
    package: &DomainPackage<P>,
    context: &EvaluationContext,
    parameters: &P,
    step_ids: &[String],
    capability_policy: Option<&dyn CapabilityGrantPolicy>,
) -> Result<ReferenceWorkflowRun, DomainError> {
    if step_ids.is_empty() {
        return Err(DomainError::Parameter(
            "a reference workflow must declare at least one step".to_owned(),
        ));
    }
    let unique: std::collections::BTreeSet<&str> = step_ids.iter().map(String::as_str).collect();
    if unique.len() != step_ids.len() {
        return Err(DomainError::Parameter(
            "reference workflow steps must be unique".to_owned(),
        ));
    }
    enforce_capability_request(package, step_ids, capability_policy)?;

    let mut outcomes = Vec::with_capacity(step_ids.len());
    let mut completed = Vec::new();
    let mut blocked_step_id = None;
    let mut status = WorkflowStatus::Completed;
    for step_id in step_ids {
        let evaluation = package.evaluate(step_id, context, parameters)?;
        if evaluation.passed() {
            outcomes.push(StepOutcome {
                step_id: step_id.clone(),
                status: StepStatus::Succeeded,
                evaluation: Some(evaluation),
                reason: None,
            });
            completed.push(step_id.clone());
            continue;
        }
        let reason = format!(
            "hard constraint failure reported by {}",
            evaluation.evaluation_id()
        );
        outcomes.push(StepOutcome {
            step_id: step_id.clone(),
            status: StepStatus::Failed,
            evaluation: Some(evaluation),
            reason: Some(reason),
        });
        blocked_step_id = Some(step_id.clone());
        status = WorkflowStatus::Failed;
        break;
    }
    Ok(ReferenceWorkflowRun {
        package_id: package.package_id().to_owned(),
        definition_id: package.workflow().definition_id().to_owned(),
        definition_version: package.workflow().version().to_string(),
        status,
        ordered_step_ids: step_ids.to_vec(),
        completed_step_ids: completed,
        outcomes,
        blocked_step_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensated_sum_matches_cpython_sum_bits() {
        // Expected values were produced by the repository Python 3.14.8
        // interpreter (`sum(values)` and `struct.pack`).
        for (values, expected) in [
            (
                vec![0.356107, 0.301885, 0.49567, 0.042336],
                "3ff322cecc814d73",
            ),
            (vec![0.1; 10], "3ff0000000000000"),
            (vec![1e16, 1.0, -1e16], "3ff0000000000000"),
            (vec![1e16, -1e16, 1.0], "3ff0000000000000"),
            (vec![0.1, 0.2, 0.3], "3fe3333333333333"),
            (vec![-0.0, 0.0, -0.0], "0000000000000000"),
            (vec![5e-324, 5e-324, 5e-324], "0000000000000003"),
        ] {
            assert_eq!(
                format!("{:016x}", cpython_sum_floats(&values).to_bits()),
                expected,
                "{values:?}"
            );
        }
    }
}
