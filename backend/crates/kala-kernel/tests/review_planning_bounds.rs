//! Independent review probe 5 for planning: huge plans/cycles, arbitrary
//! attempt bounds, and the exact configured step limit.

mod support;

use kala_contracts::{ContractModel, PlanStep, WorkflowDefinition};
use kala_kernel::planning::{PlanValidationCode, PlanValidator, find_cycle, topological_order};
use num_bigint::BigInt;
use support::{array, object, text};

fn make_definition(count: usize) -> WorkflowDefinition {
    let available: Vec<kala_json::CanonicalValue> = (0..count)
        .map(|index| text(&format!("catalog-{index}")))
        .collect();
    let value = object(vec![
        ("available_step_ids", array(available)),
        ("definition_id", text("huge-workflow")),
        ("initial_state", text("start")),
        ("max_replanning_attempts", support::int(1)),
        ("name", text("Huge workflow")),
        ("states", array(vec![text("start"), text("done")])),
        (
            "transitions",
            array(vec![object(vec![
                ("checkpoint_type", kala_json::CanonicalValue::Null),
                ("from_state", text("start")),
                ("step_id", text("catalog-0")),
                ("to_state", text("done")),
                ("transition_id", text("transition-1")),
            ])]),
        ),
        ("version", text("1.0.0")),
    ]);
    WorkflowDefinition::from_value(&value).expect("definition")
}

fn step(index: usize, depends_on: Option<usize>, max_attempts: &str) -> PlanStep {
    let dependencies =
        depends_on.map_or_else(String::new, |dependency| format!(r#""step-{dependency}""#));
    PlanStep::from_json_str(&format!(
        r#"{{"step_id":"step-{index}","catalog_step_id":"catalog-{index}","intent":"Do the step.","depends_on":[{dependencies}],"required_capabilities":[],"max_attempts":{max_attempts}}}"#
    ))
    .expect("step")
}

fn validator(
    definition: WorkflowDefinition,
    max_steps: BigInt,
    max_attempts: BigInt,
    max_total_attempts: Option<BigInt>,
) -> PlanValidator {
    PlanValidator::from_config(
        definition,
        None,
        None,
        max_steps,
        max_attempts,
        max_total_attempts,
    )
    .expect("validator")
}

#[test]
fn huge_acyclic_chain_and_cycle_do_not_overflow_the_stack() {
    const COUNT: usize = 12_000;
    let definition = make_definition(COUNT);
    let chain: Vec<PlanStep> = (0..COUNT)
        .map(|index| step(index, index.checked_sub(1), "1"))
        .collect();
    assert!(find_cycle(&chain).is_none());
    let order = topological_order(&chain).expect("acyclic order");
    assert_eq!(order.len(), COUNT);
    assert_eq!(order.first().expect("first").to_string_lossy(), "step-0");
    assert_eq!(
        order.last().expect("last").to_string_lossy(),
        format!("step-{}", COUNT - 1)
    );

    let validator = validator(
        definition.clone(),
        BigInt::from(COUNT + 1),
        BigInt::from(1),
        None,
    );
    let plan = validator
        .validate(&chain, None, None, &BigInt::from(0))
        .expect("12k-step chain must validate");
    assert_eq!(plan.steps().len(), COUNT);
    assert_eq!(plan.step_ids().len(), COUNT);
    assert!(plan.required_capabilities().is_empty());

    // Close the chain into one 12k-node cycle. Cycle discovery is iterative
    // and must return a deterministic closed path, not overflow.
    let mut cycle: Vec<PlanStep> = (0..COUNT)
        .map(|index| step(index, index.checked_sub(1), "1"))
        .collect();
    cycle[0] = step(0, Some(COUNT - 1), "1");
    let path = find_cycle(&cycle).expect("cycle");
    assert_eq!(path.len(), COUNT + 1);
    assert_eq!(path.first(), path.last());
    assert_eq!(path.first().expect("start").to_string_lossy(), "step-0");
    assert!(topological_order(&cycle).is_none());

    let error = validator
        .validate(&cycle, None, None, &BigInt::from(0))
        .expect_err("cyclic plan must be rejected");
    assert_eq!(error.code(), PlanValidationCode::DependencyCycle);
    assert!(error.details().iter().any(|(key, value)| key == "cycle"
        && value.starts_with("step-0 -> ")
        && value.ends_with(" -> step-0")));
}

#[test]
fn attempt_and_step_bounds_use_arbitrary_precision_and_exact_limits() {
    let definition = make_definition(2);
    let huge_attempts = BigInt::from(10).pow(100);
    let huge_hint = huge_attempts.to_string();
    let huge_step = step(0, None, &huge_hint);

    // A per-step attempt value larger than the configured bound is rejected
    // without float conversion or overflow.
    let lower = validator(
        definition.clone(),
        BigInt::from(10),
        BigInt::from(10).pow(99),
        None,
    );
    let error = lower
        .validate(
            std::slice::from_ref(&huge_step),
            None,
            None,
            &BigInt::from(0),
        )
        .expect_err("too many attempts");
    assert_eq!(error.code(), PlanValidationCode::TooManyAttempts);
    assert!(
        error
            .details()
            .iter()
            .any(|(key, value)| key == "requested" && value == &huge_hint)
    );

    // An arbitrary-precision bound above the step value accepts it and is
    // preserved in the normalized plan.
    let upper_bound = BigInt::from(10).pow(101);
    let upper = validator(
        definition.clone(),
        BigInt::from(10),
        upper_bound.clone(),
        None,
    );
    let plan = upper
        .validate(&[huge_step], None, None, &BigInt::from(0))
        .expect("huge bound");
    assert_eq!(plan.max_attempts(), &upper_bound);

    // Aggregate attempt sums are arbitrary precision as well.
    let fifty = BigInt::from(10).pow(50);
    let fifty_hint = fifty.to_string();
    let steps = vec![step(0, None, &fifty_hint), step(1, Some(0), &fifty_hint)];
    let aggregate_ok = validator(
        definition.clone(),
        BigInt::from(10),
        fifty.clone(),
        Some(BigInt::from(10).pow(99)),
    );
    aggregate_ok
        .validate(&steps, None, None, &BigInt::from(0))
        .expect("aggregate fits");
    let aggregate_small = validator(
        definition.clone(),
        BigInt::from(10),
        fifty.clone(),
        Some(fifty.clone()),
    );
    let error = aggregate_small
        .validate(&steps, None, None, &BigInt::from(0))
        .expect_err("aggregate too large");
    assert_eq!(error.code(), PlanValidationCode::TooManyAttempts);
    assert!(error.details().iter().any(|(key, _)| key == "maximum"));

    // Zero attempts is invalid at the step level.
    let zero = step(0, None, "0");
    let error = lower
        .validate(&[zero], None, None, &BigInt::from(0))
        .expect_err("zero attempts");
    assert_eq!(error.code(), PlanValidationCode::InvalidAttempts);

    // Exactly 256 steps are allowed by the default bound; 257 are not, and
    // the payload reports the exact count and maximum.
    let wide_definition = make_definition(257);
    let steps: Vec<PlanStep> = (0..256).map(|index| step(index, None, "1")).collect();
    let default_bound = validator(wide_definition, BigInt::from(256), BigInt::from(1), None);
    let plan = default_bound
        .validate(&steps, None, None, &BigInt::from(0))
        .expect("256 steps");
    assert_eq!(plan.steps().len(), 256);
    let too_many: Vec<PlanStep> = (0..257).map(|index| step(index, None, "1")).collect();
    let error = default_bound
        .validate(&too_many, None, None, &BigInt::from(0))
        .expect_err("257 steps");
    assert_eq!(error.code(), PlanValidationCode::TooManySteps);
    assert!(
        error
            .details()
            .iter()
            .any(|(key, value)| key == "count" && value == "257")
    );
    assert!(
        error
            .details()
            .iter()
            .any(|(key, value)| key == "maximum" && value == "256")
    );
}
