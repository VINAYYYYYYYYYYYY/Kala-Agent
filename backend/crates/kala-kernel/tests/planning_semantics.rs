//! Native plan normalization, validation precedence and dependency tests.

use kala_contracts::{ContractModel, PlanStep, Text, WorkflowDefinition};
use kala_json::CanonicalValue;
use kala_kernel::planning::{
    PlanConfigError, PlanValidationCode, PlanValidationError, PlanValidator, capability_set,
    combine_capability_sets, find_cycle, topological_order,
};
use num_bigint::BigInt;

fn definition() -> WorkflowDefinition {
    WorkflowDefinition::from_json_str(
        r#"{"definition_id":"workflow-1","version":"1.0.0","name":"Test workflow","initial_state":"start","states":["start","done"],"transitions":[{"transition_id":"transition-1","from_state":"start","to_state":"done","step_id":"step-1","checkpoint_type":null}],"available_step_ids":["step-1","step-2","step-3"],"max_replanning_attempts":2}"#,
    )
    .expect("definition")
}

fn step(
    step_id: &str,
    catalog_step_id: &str,
    depends_on: &[&str],
    capabilities: &[&str],
    max_attempts: i64,
) -> PlanStep {
    let depends = depends_on
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(",");
    let caps = capabilities
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>()
        .join(",");
    PlanStep::from_json_str(&format!(
        r#"{{"step_id":"{step_id}","catalog_step_id":"{catalog_step_id}","intent":"Perform a declared step.","depends_on":[{depends}],"required_capabilities":[{caps}],"max_attempts":{max_attempts}}}"#
    ))
    .expect("step")
}

fn validator(
    allowed: Option<&[&str]>,
    max_steps: i64,
    max_attempts: i64,
    total: Option<i64>,
) -> PlanValidator {
    let allowed = allowed.map(|values| {
        capability_set(values.iter().map(|value| Text::new(value))).expect("capability set")
    });
    PlanValidator::from_config(
        definition(),
        allowed,
        None,
        BigInt::from(max_steps),
        BigInt::from(max_attempts),
        total.map(BigInt::from),
    )
    .expect("validator")
}

fn validate(
    validator: &PlanValidator,
    steps: &[PlanStep],
    allowed: Option<&[&str]>,
    granted: Option<&[&str]>,
    replanning: i64,
) -> Result<kala_kernel::planning::NormalizedPlan, PlanValidationError> {
    let allowed = allowed
        .map(|values| capability_set(values.iter().map(|value| Text::new(value))).expect("set"));
    let granted = granted
        .map(|values| capability_set(values.iter().map(|value| Text::new(value))).expect("set"));
    validator.validate(
        steps,
        allowed.as_ref(),
        granted.as_ref(),
        &BigInt::from(replanning),
    )
}

fn code(error: PlanValidationError) -> PlanValidationCode {
    error.code()
}

#[test]
fn constructor_validates_bounds() {
    assert!(matches!(
        PlanValidator::from_config(
            definition(),
            None,
            None,
            BigInt::from(0),
            BigInt::from(1),
            None
        ),
        Err(PlanConfigError::Value(_))
    ));
    assert!(matches!(
        PlanValidator::from_config(
            definition(),
            None,
            None,
            BigInt::from(1),
            BigInt::from(0),
            None
        ),
        Err(PlanConfigError::Value(_))
    ));
    assert!(matches!(
        PlanValidator::from_config(
            definition(),
            None,
            None,
            BigInt::from(1),
            BigInt::from(1),
            Some(BigInt::from(0))
        ),
        Err(PlanConfigError::Value(_))
    ));
}

#[test]
fn capability_sets_strip_and_reject_empty_or_non_strings() {
    let set = capability_set([Text::new(" read "), Text::new("write")]).expect("set");
    assert_eq!(
        set.iter().map(Text::to_string_lossy).collect::<Vec<_>>(),
        ["read", "write"]
    );
    assert!(matches!(
        capability_set([Text::new("read"), Text::new("  ")]),
        Err(PlanConfigError::Value(_))
    ));
    assert_eq!(combine_capability_sets(None, None), None);
    assert_eq!(
        combine_capability_sets(
            Some(capability_set([Text::new("read"), Text::new("write")]).expect("set")),
            Some(capability_set([Text::new("read")]).expect("set"))
        )
        .expect("set")
        .len(),
        1
    );
}

#[test]
fn empty_and_too_many_steps_precede_replanning_checks() {
    let validator = validator(None, 1, 1, None);
    assert_eq!(
        code(validate(&validator, &[], None, None, 99).expect_err("empty")),
        PlanValidationCode::EmptyPlan
    );
    let steps = [
        step("a", "step-1", &[], &[], 1),
        step("b", "step-2", &[], &[], 1),
    ];
    let error = validate(&validator, &steps, None, None, 99).expect_err("too many");
    assert_eq!(error.code(), PlanValidationCode::TooManySteps);
    assert_eq!(
        error.details(),
        &[
            ("count".to_owned(), "2".to_owned()),
            ("maximum".to_owned(), "1".to_owned())
        ]
    );
    assert!(error.to_string().contains("maximum is 1"));
}

#[test]
fn replanning_is_bounded_by_the_definition() {
    let validator = validator(None, 256, 1, None);
    assert_eq!(
        code(
            validate(
                &validator,
                &[step("a", "step-1", &[], &[], 1)],
                None,
                None,
                -1
            )
            .expect_err("negative")
        ),
        PlanValidationCode::ReplanningLimit
    );
    assert_eq!(
        code(
            validate(
                &validator,
                &[step("a", "step-1", &[], &[], 1)],
                None,
                None,
                3
            )
            .expect_err("over")
        ),
        PlanValidationCode::ReplanningLimit
    );
    assert!(
        validate(
            &validator,
            &[step("a", "step-1", &[], &[], 1)],
            None,
            None,
            2
        )
        .is_ok()
    );
}

#[test]
fn duplicate_catalog_dependency_and_cycle_codes_are_exact() {
    let validator = validator(None, 256, 2, None);
    let duplicate = validate(
        &validator,
        &[
            step("a", "step-1", &[], &[], 1),
            step("a", "step-2", &[], &[], 1),
        ],
        None,
        None,
        0,
    )
    .expect_err("duplicate");
    assert_eq!(duplicate.code(), PlanValidationCode::DuplicateStepId);
    assert_eq!(
        duplicate.step_id().map(Text::to_string_lossy),
        Some("a".to_owned())
    );

    let unknown = validate(
        &validator,
        &[step("a", "absent", &[], &[], 1)],
        None,
        None,
        0,
    )
    .expect_err("catalog");
    assert_eq!(unknown.code(), PlanValidationCode::UnknownCatalogStep);
    assert_eq!(
        unknown.details(),
        &[("catalog_step_id".to_owned(), "absent".to_owned())]
    );

    let dependency = validate(
        &validator,
        &[step("a", "step-1", &["absent"], &[], 1)],
        None,
        None,
        0,
    )
    .expect_err("dependency");
    assert_eq!(dependency.code(), PlanValidationCode::UnknownDependency);
    assert_eq!(
        dependency.details(),
        &[("dependency".to_owned(), "absent".to_owned())]
    );

    let cycle = validate(
        &validator,
        &[
            step("a", "step-1", &["b"], &[], 1),
            step("b", "step-2", &["a"], &[], 1),
        ],
        None,
        None,
        0,
    )
    .expect_err("cycle");
    assert_eq!(cycle.code(), PlanValidationCode::DependencyCycle);
    assert_eq!(
        cycle.step_id().map(Text::to_string_lossy),
        Some("a".to_owned())
    );
    assert_eq!(
        cycle.details(),
        &[("cycle".to_owned(), "a -> b -> a -> a".to_owned())]
    );
}

#[test]
fn attempt_bounds_have_their_own_codes_and_details() {
    let validator = validator(None, 256, 2, None);
    assert_eq!(
        code(
            validate(
                &validator,
                &[step("a", "step-1", &[], &[], 0)],
                None,
                None,
                0
            )
            .expect_err("zero")
        ),
        PlanValidationCode::InvalidAttempts
    );
    let too_many = validate(
        &validator,
        &[step("a", "step-1", &[], &[], 3)],
        None,
        None,
        0,
    )
    .expect_err("too many");
    assert_eq!(too_many.code(), PlanValidationCode::TooManyAttempts);
    assert_eq!(
        too_many.details(),
        &[
            ("requested".to_owned(), "3".to_owned()),
            ("maximum".to_owned(), "2".to_owned())
        ]
    );
}

#[test]
fn capability_intersection_and_deny_all_behavior() {
    let unrestricted = PlanValidator::new(definition());
    assert!(unrestricted.unrestricted());
    assert!(unrestricted.allowed_capabilities().is_none());
    assert!(
        validate(
            &unrestricted,
            &[step("a", "step-1", &[], &["read"], 1)],
            None,
            None,
            0
        )
        .is_ok()
    );

    let denied_all = validator(Some(&[]), 256, 1, None);
    assert!(!denied_all.unrestricted());
    let error = validate(
        &denied_all,
        &[step("a", "step-1", &[], &["read"], 1)],
        None,
        None,
        0,
    )
    .expect_err("deny all");
    assert_eq!(error.code(), PlanValidationCode::CapabilityNotGranted);
    assert_eq!(
        error.details(),
        &[("capability".to_owned(), "read".to_owned())]
    );
    assert!(
        validate(
            &denied_all,
            &[step("a", "step-1", &[], &[], 1)],
            None,
            None,
            0
        )
        .is_ok()
    );

    let narrowed = validator(Some(&["read", "write"]), 256, 1, None);
    assert!(
        validate(
            &narrowed,
            &[step("a", "step-1", &[], &["read"], 1)],
            None,
            Some(&["write"]),
            0
        )
        .is_err()
    );
    assert!(
        validate(
            &narrowed,
            &[step("a", "step-1", &[], &["read"], 1)],
            None,
            Some(&["read"]),
            0
        )
        .is_ok()
    );
}

#[test]
fn aggregate_attempts_are_checked_after_capabilities() {
    let validator = validator(Some(&["read"]), 256, 2, Some(2));
    let steps = [
        step("a", "step-1", &[], &["write"], 2),
        step("b", "step-2", &[], &[], 2),
    ];
    let error = validate(&validator, &steps, None, None, 0).expect_err("capability first");
    assert_eq!(error.code(), PlanValidationCode::CapabilityNotGranted);
    let steps = [
        step("a", "step-1", &[], &[], 2),
        step("b", "step-2", &[], &[], 2),
    ];
    let error = validate(&validator, &steps, None, None, 0).expect_err("aggregate");
    assert_eq!(error.code(), PlanValidationCode::TooManyAttempts);
    assert_eq!(
        error.details(),
        &[
            ("requested".to_owned(), "4".to_owned()),
            ("maximum".to_owned(), "2".to_owned())
        ]
    );
}

#[test]
fn first_error_precedence_within_a_step_matches_the_source() {
    let validator = validator(None, 256, 1, None);
    // Duplicate is checked before the second step's unknown catalog entry.
    let error = validate(
        &validator,
        &[
            step("a", "step-1", &[], &[], 1),
            step("a", "absent", &[], &[], 1),
        ],
        None,
        None,
        0,
    )
    .expect_err("duplicate");
    assert_eq!(error.code(), PlanValidationCode::DuplicateStepId);
    // Unknown catalog is checked before invalid attempts.
    let error = validate(
        &validator,
        &[step("a", "absent", &[], &[], 0)],
        None,
        None,
        0,
    )
    .expect_err("catalog");
    assert_eq!(error.code(), PlanValidationCode::UnknownCatalogStep);
    // Unknown dependency is checked before the cycle walk.
    let error = validate(
        &validator,
        &[
            step("a", "step-1", &["b", "absent"], &[], 1),
            step("b", "step-2", &["a"], &[], 1),
        ],
        None,
        None,
        0,
    )
    .expect_err("dependency");
    assert_eq!(error.code(), PlanValidationCode::UnknownDependency);
}

#[test]
fn dynamic_values_reject_non_sequences_invalid_steps_and_raw_self_dependency() {
    let validator = PlanValidator::new(definition());
    let error = validator
        .validate_values(
            &CanonicalValue::text("not a plan"),
            None,
            None,
            &BigInt::from(0),
        )
        .expect_err("sequence");
    assert_eq!(error.code(), PlanValidationCode::InvalidStep);

    let error = validator
        .validate_values(
            &CanonicalValue::Array(vec![CanonicalValue::text("not a step")]),
            None,
            None,
            &BigInt::from(0),
        )
        .expect_err("entry");
    assert_eq!(error.code(), PlanValidationCode::InvalidStep);

    let raw = CanonicalValue::object([
        ("catalog_step_id", CanonicalValue::text("step-1")),
        (
            "depends_on",
            CanonicalValue::Array(vec![CanonicalValue::text("a")]),
        ),
        ("intent", CanonicalValue::text("Do it.")),
        ("step_id", CanonicalValue::text("a")),
    ]);
    let error = validator
        .validate_values(
            &CanonicalValue::Array(vec![raw]),
            None,
            None,
            &BigInt::from(0),
        )
        .expect_err("self dependency");
    assert_eq!(error.code(), PlanValidationCode::SelfDependency);
    assert_eq!(
        error.step_id().map(Text::to_string_lossy),
        Some("a".to_owned())
    );

    let invalid_contract = CanonicalValue::object([
        ("catalog_step_id", CanonicalValue::text(" ")),
        ("depends_on", CanonicalValue::Array(Vec::new())),
        ("intent", CanonicalValue::text("Do it.")),
        ("step_id", CanonicalValue::text("a")),
    ]);
    let error = validator
        .validate_values(
            &CanonicalValue::Array(vec![invalid_contract]),
            None,
            None,
            &BigInt::from(0),
        )
        .expect_err("invalid contract");
    assert_eq!(error.code(), PlanValidationCode::InvalidStep);
    assert_eq!(
        error.details(),
        &[("reason".to_owned(), "ValidationError".to_owned())]
    );
}

#[test]
fn normalized_plan_exposes_sorted_capabilities_and_configuration() {
    let validator = validator(Some(&["read", "write"]), 256, 2, None);
    let plan = validate(
        &validator,
        &[
            step("b", "step-2", &["a"], &["write"], 1),
            step("a", "step-1", &[], &["read"], 1),
        ],
        None,
        None,
        0,
    )
    .expect("plan");
    assert_eq!(
        plan.workflow_definition_id().to_string_lossy(),
        "workflow-1"
    );
    assert_eq!(
        plan.step_ids()
            .iter()
            .map(Text::to_string_lossy)
            .collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert_eq!(
        plan.required_capabilities()
            .iter()
            .map(Text::to_string_lossy)
            .collect::<Vec<_>>(),
        ["read", "write"]
    );
    assert_eq!(plan.max_attempts(), &BigInt::from(2));
    assert_eq!(plan.len(), 2);
    assert!(!plan.is_empty());
    assert_eq!(plan.iter().count(), 2);
}

#[test]
fn validate_or_raise_and_is_valid() {
    let validator = PlanValidator::new(definition());
    assert!(validator.is_valid(
        &[step("a", "step-1", &[], &[], 1)],
        None,
        None,
        &BigInt::from(0)
    ));
    assert!(!validator.is_valid(&[], None, None, &BigInt::from(0)));
    let error = validator
        .validate_or_raise(&[], None, None, &BigInt::from(0))
        .expect_err("rejected");
    assert_eq!(error.error().code(), PlanValidationCode::EmptyPlan);
    assert_eq!(error.to_string(), "a plan must contain at least one step");
}

#[test]
fn validation_payload_renders_like_python() {
    let error = PlanValidationError::new(
        PlanValidationCode::DuplicateStepId,
        "duplicate",
        Some(Text::new("s-1")),
        vec![("count".to_owned(), "2".to_owned())],
    );
    assert_eq!(error.to_string(), "duplicate (s-1)");
    assert_eq!(error.reason(), PlanValidationCode::DuplicateStepId);
    assert_eq!(
        error.field().map(Text::to_string_lossy),
        Some("s-1".to_owned())
    );
    assert_eq!(
        error.to_json(),
        r#"{"code":"duplicate_step_id","details":{"count":"2"},"message":"duplicate","step_id":"s-1"}"#
    );
}

#[test]
fn cycle_discovery_and_topological_order_are_deterministic() {
    let steps = [
        step("a", "step-1", &["c"], &[], 1),
        step("b", "step-2", &["a"], &[], 1),
        step("c", "step-3", &["b"], &[], 1),
    ];
    let cycle = find_cycle(&steps).expect("cycle");
    assert_eq!(
        cycle.iter().map(Text::to_string_lossy).collect::<Vec<_>>(),
        ["a", "c", "b", "a"]
    );
    assert!(topological_order(&steps).is_none());

    let acyclic = [
        step("a", "step-1", &[], &[], 1),
        step("b", "step-2", &["a"], &[], 1),
        step("c", "step-3", &["b"], &[], 1),
    ];
    assert!(find_cycle(&acyclic).is_none());
    assert_eq!(
        topological_order(&acyclic)
            .expect("order")
            .iter()
            .map(Text::to_string_lossy)
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
}

#[test]
fn deep_acyclic_plans_do_not_overflow_the_stack() {
    let count = 5_000usize;
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let depends = if index == 0 {
            Vec::new()
        } else {
            vec![format!("s{:05}", index - 1)]
        };
        values.push(CanonicalValue::object([
            ("catalog_step_id", CanonicalValue::text("step-1")),
            (
                "depends_on",
                CanonicalValue::Array(depends.iter().map(CanonicalValue::text).collect()),
            ),
            ("intent", CanonicalValue::text("Do it.")),
            ("max_attempts", CanonicalValue::Integer(BigInt::from(1))),
            ("parameters_json", CanonicalValue::text("{}")),
            ("step_id", CanonicalValue::text(format!("s{index:05}"))),
        ]));
    }
    let validator = PlanValidator::from_config(
        definition(),
        None,
        None,
        BigInt::from(6_000),
        BigInt::from(1),
        None,
    )
    .expect("validator");
    let plan = validator
        .validate_values(&CanonicalValue::Array(values), None, None, &BigInt::from(0))
        .expect("deep plan");
    assert_eq!(plan.len(), count);
}

#[test]
fn explicit_big_integer_attempts_are_preserved() {
    let validator = validator(None, 256, 10, None);
    let step = step("a", "step-1", &[], &[], 10);
    let plan = validate(&validator, &[step], None, None, 0).expect("plan");
    assert_eq!(
        plan.steps()[0].max_attempts().as_bigint(),
        &BigInt::from(10)
    );
}

#[test]
fn dynamic_replanning_counters_enforce_strict_integer_types() {
    let validator = PlanValidator::new(definition());
    let steps = CanonicalValue::Array(vec![CanonicalValue::object([
        ("catalog_step_id", CanonicalValue::text("step-1")),
        ("depends_on", CanonicalValue::Array(Vec::new())),
        ("intent", CanonicalValue::text("Do it.")),
        ("step_id", CanonicalValue::text("a")),
    ])]);
    for value in [
        CanonicalValue::Bool(true),
        CanonicalValue::text("1"),
        CanonicalValue::Null,
    ] {
        let error = validator
            .validate_values_request(&steps, None, None, &value)
            .expect_err("non-integer");
        assert_eq!(error.code(), PlanValidationCode::ReplanningLimit);
        assert_eq!(error.message(), "replanning_attempts must be an integer");
    }
}
