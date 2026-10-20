//! Cross-reader tests for legacy Python-exported kernel state.

mod support;

use kala_contracts::ContractModel;
use kala_json::{CanonicalValue, parse_strict};
use kala_kernel::policy::PolicyEngine;
use kala_kernel::workflow::WorkflowEngine;
use support::{
    array, assert_exact_oracle, get, int, object, policy_native, request_value, run_oracle,
    simple_definition, text, workflow_native,
};

const NOW: &str = "2026-01-02T03:04:05Z";

fn last_result(output: &CanonicalValue) -> CanonicalValue {
    let results = get(output, "results").expect("results");
    match results {
        CanonicalValue::Array(values) => values.last().expect("result").clone(),
        _ => panic!("results array"),
    }
}

fn object_map(
    value: &CanonicalValue,
) -> std::collections::BTreeMap<kala_json::JsonText, CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.clone(),
        _ => panic!("object"),
    }
}

fn with_key(value: &CanonicalValue, key: &str, replacement: CanonicalValue) -> CanonicalValue {
    let mut object = object_map(value);
    object.insert(kala_json::JsonText::new(key), replacement);
    CanonicalValue::Object(object)
}

fn without_key(value: &CanonicalValue, key: &str) -> CanonicalValue {
    let mut object = object_map(value);
    object.remove(&kala_json::JsonText::new(key));
    CanonicalValue::Object(object)
}

#[test]
fn policy_python_exported_state_round_trips_exactly() {
    let approved = request_value(
        "request-approved",
        "state-1",
        "action-1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let rejected = request_value(
        "request-rejected",
        "state-1",
        "action-1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let source = object(vec![
        ("grants", array(vec![text("network.egress"), text("read")])),
        ("now", text(NOW)),
        (
            "ops",
            array(vec![
                object(vec![
                    ("actor_id", text("reviewer-1")),
                    ("now", text(NOW)),
                    ("op", text("approve")),
                    ("request", approved.clone()),
                ]),
                object(vec![
                    ("actor_id", text("reviewer-1")),
                    ("now", text(NOW)),
                    ("op", text("reject")),
                    ("request", rejected.clone()),
                ]),
                object(vec![("op", text("export_state"))]),
            ]),
        ),
    ]);
    let oracle_source = run_oracle("policy", &source);
    let parsed = parse_strict(
        oracle_source.as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle output");
    let python_state = last_result(&parsed);

    // The native import accepts the live Python export and reproduces it.
    let native_engine = PolicyEngine::new();
    let request = support::policy_request(&approved).expect("request");
    native_engine
        .import_state(&python_state)
        .expect("import Python state");
    let native_export = native_engine.export_state().as_value();
    assert_eq!(
        support::canonical(&native_export),
        support::canonical(&python_state)
    );
    let exported_json = native_engine.export_state_json().expect("json");
    assert_eq!(exported_json, support::canonical(&python_state));
    assert!(
        native_engine
            .decide(&request, None)
            .expect("decide")
            .allowed()
    );

    // And the oracle imports the same document again, yielding the same bytes.
    let import_doc = object(vec![
        ("now", text(NOW)),
        (
            "ops",
            array(vec![
                object(vec![("op", text("import_state")), ("state", python_state)]),
                object(vec![("op", text("export_state"))]),
                object(vec![
                    ("now", text(NOW)),
                    ("op", text("decide")),
                    ("request", approved),
                ]),
            ]),
        ),
    ]);
    let oracle = run_oracle("policy", &import_doc);
    let native = policy_native(&import_doc);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn workflow_python_exported_state_restores_and_resumes() {
    let source = object(vec![
        ("clock", text(NOW)),
        ("definition", simple_definition(true)),
        (
            "ops",
            array(vec![
                object(vec![
                    ("op", text("start")),
                    ("workflow_id", text("workflow-1")),
                ]),
                object(vec![("op", text("run"))]),
                object(vec![
                    ("actor_id", text("reviewer")),
                    ("now", text(NOW)),
                    ("op", text("approve_checkpoint")),
                ]),
                object(vec![("op", text("serialize"))]),
            ]),
        ),
    ]);
    let oracle_source = run_oracle("workflow", &source);
    let parsed = parse_strict(
        oracle_source.as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle output");
    let serialized = get(&last_result(&parsed), "result")
        .and_then(|value| value.as_text())
        .expect("serialized state")
        .to_string_lossy();

    let definition = kala_contracts::WorkflowDefinition::from_value(
        get(&source, "definition").expect("definition"),
    )
    .expect("definition");
    let engine = WorkflowEngine::new(definition);
    let restored = engine
        .restore_json(&serialized)
        .expect("restore Python state");
    assert_eq!(
        restored.to_json().expect("json"),
        serialized,
        "restored Python state re-serializes byte-identically"
    );
    let resumed = engine.resume(None).expect("resume");
    assert_eq!(resumed.status().as_str(), "completed");

    let import_doc = object(vec![
        ("clock", text(NOW)),
        (
            "definition",
            get(&source, "definition").expect("definition").clone(),
        ),
        (
            "ops",
            array(vec![
                object(vec![("op", text("restore")), ("state", text(&serialized))]),
                object(vec![("op", text("resume"))]),
                object(vec![("op", text("serialize"))]),
            ]),
        ),
    ]);
    let oracle = run_oracle("workflow", &import_doc);
    let native = workflow_native(&import_doc);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn native_policy_export_is_accepted_by_python() {
    let engine =
        PolicyEngine::from_grants([kala_kernel::policy::GrantSpec::from("network.egress")])
            .expect("engine");
    let request = support::policy_request(&request_value(
        "request-native",
        "state-1",
        "action-1",
        &["network.egress"],
        "egress",
        None,
        None,
    ))
    .expect("request");
    engine
        .approve(
            &request,
            &kala_kernel::policy::DecisionContext::new()
                .with_actor_id(kala_contracts::Text::new("native-reviewer"))
                .with_now(kala_kernel::KernelTimestamp::parse_isoformat(NOW).expect("timestamp")),
        )
        .expect("approve");
    let state = engine.export_state().as_value();
    let document = object(vec![
        ("now", text(NOW)),
        (
            "ops",
            array(vec![
                object(vec![("op", text("import_state")), ("state", state)]),
                object(vec![("op", text("export_state"))]),
            ]),
        ),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn native_workflow_export_is_accepted_by_python() {
    let definition = kala_contracts::WorkflowDefinition::from_json_str(&support::canonical(
        &simple_definition(false),
    ))
    .expect("definition");
    let engine = WorkflowEngine::new(definition);
    engine
        .start(&kala_contracts::Text::new("workflow-1"), None)
        .expect("start");
    let serialized = engine.serialize(None).expect("serialize");
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", simple_definition(false)),
        (
            "ops",
            array(vec![
                object(vec![("op", text("restore")), ("state", text(&serialized))]),
                object(vec![("op", text("run"))]),
                object(vec![("op", text("serialize"))]),
            ]),
        ),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[allow(clippy::vec_init_then_push)]
#[test]
fn policy_state_import_rejection_matrix_matches_python() {
    let base_request = request_value(
        "request-valid",
        "state-1",
        "action-1",
        &["read"],
        "none",
        None,
        Some("plan_approval"),
    );
    let source = object(vec![
        ("grants", array(vec![text("read")])),
        ("now", text(NOW)),
        (
            "ops",
            array(vec![
                object(vec![
                    ("now", text(NOW)),
                    ("op", text("approve")),
                    ("request", base_request),
                ]),
                object(vec![("op", text("export_state"))]),
            ]),
        ),
    ]);
    let parsed = parse_strict(
        run_oracle("policy", &source).as_bytes(),
        &kala_contracts::model_json_limits(),
    )
    .expect("oracle output");
    let valid = last_result(&parsed);
    let request = match get(&valid, "requests").expect("requests") {
        CanonicalValue::Array(values) => values[0].clone(),
        _ => panic!("requests"),
    };
    let approval = match get(&valid, "approvals").expect("approvals") {
        CanonicalValue::Array(values) => values[0].clone(),
        _ => panic!("approvals"),
    };
    let grant = match get(&valid, "grants").expect("grants") {
        CanonicalValue::Array(values) => values[0].clone(),
        _ => panic!("grants"),
    };

    let mut variants: Vec<(&str, CanonicalValue)> = Vec::new();
    variants.push(("valid", valid.clone()));
    variants.push(("version-99", with_key(&valid, "version", int(99))));
    variants.push(("version-string", with_key(&valid, "version", text("1"))));
    variants.push((
        "version-bool",
        with_key(&valid, "version", CanonicalValue::Bool(true)),
    ));
    variants.push((
        "unknown-key",
        with_key(&valid, "unexpected", CanonicalValue::Bool(true)),
    ));
    variants.push(("grants-shape", with_key(&valid, "grants", text("none"))));
    variants.push((
        "grant-extra",
        with_key(
            &valid,
            "grants",
            array(vec![with_key(&grant, "extra", int(1))]),
        ),
    ));
    variants.push((
        "grant-wildcard",
        with_key(
            &valid,
            "grants",
            array(vec![object(vec![("capability", text("*"))])]),
        ),
    ));
    variants.push((
        "request-extra",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(&request, "extra", int(1))]),
        ),
    ));
    variants.push((
        "request-unsafe",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(&request, "request_id", text("request id"))]),
        ),
    ));
    variants.push((
        "request-bool",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(&request, "destructive", int(1))]),
        ),
    ));
    variants.push((
        "request-side-effect",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(
                &request,
                "side_effect_level",
                text("unknown"),
            )]),
        ),
    ));
    variants.push((
        "request-checkpoint",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(&request, "checkpoint_type", text("unknown"))]),
        ),
    ));
    variants.push((
        "request-caps-null",
        with_key(
            &valid,
            "requests",
            array(vec![with_key(
                &request,
                "requested_capabilities",
                CanonicalValue::Null,
            )]),
        ),
    ));
    variants.push((
        "request-missing-id",
        with_key(
            &valid,
            "requests",
            array(vec![without_key(&request, "request_id")]),
        ),
    ));
    variants.push((
        "approval-status",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(&approval, "status", text("unknown"))]),
        ),
    ));
    variants.push((
        "approval-naive-time",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T03:04:05"),
            )]),
        ),
    ));
    variants.push((
        "approval-bad-time",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(&approval, "decided_at", text("not-a-time"))]),
        ),
    ));
    variants.push((
        "approval-actor",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(&approval, "actor_id", text("bad actor"))]),
        ),
    ));
    variants.push((
        "approval-pending-actor",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &with_key(&approval, "status", text("pending")),
                "actor_id",
                text("reviewer"),
            )]),
        ),
    ));
    variants.push((
        "approval-resolved-actor-null",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(&approval, "actor_id", CanonicalValue::Null)]),
        ),
    ));
    variants.push((
        "orphan-approval",
        with_key(&valid, "requests", array(Vec::new())),
    ));
    variants.push((
        "duplicate-approval",
        with_key(
            &valid,
            "approvals",
            array(vec![approval.clone(), approval.clone()]),
        ),
    ));
    variants.push((
        "duplicate-request",
        with_key(
            &valid,
            "requests",
            array(vec![request.clone(), request.clone()]),
        ),
    ));
    variants.push((
        "tool-class-action",
        with_key(
            &valid,
            "tool_classes",
            array(vec![object(vec![
                ("action_digest", text("action 1")),
                ("capabilities", array(vec![text("read")])),
            ])]),
        ),
    ));
    variants.push((
        "tool-class-duplicate",
        with_key(
            &valid,
            "tool_classes",
            array(vec![
                object(vec![
                    ("action_digest", text("action-1")),
                    ("capabilities", array(vec![text("read")])),
                    ("scope", CanonicalValue::Null),
                ]),
                object(vec![
                    ("action_digest", text("action-1")),
                    ("capabilities", array(vec![text("read")])),
                    ("scope", CanonicalValue::Null),
                ]),
            ]),
        ),
    ));
    variants.push((
        "tool-class-capability",
        with_key(
            &valid,
            "tool_classes",
            array(vec![object(vec![
                ("action_digest", text("action-1")),
                ("capabilities", array(vec![text("bad cap")])),
                ("scope", CanonicalValue::Null),
            ])]),
        ),
    ));
    variants.push(("max-zero", with_key(&valid, "max_tracked_requests", int(0))));
    variants.push((
        "max-string",
        with_key(&valid, "max_tracked_requests", text("4")),
    ));
    variants.push(("no-max", without_key(&valid, "max_tracked_requests")));
    variants.push(("no-grants", without_key(&valid, "grants")));
    variants.push(("no-requests", without_key(&valid, "requests")));
    variants.push(("no-tool-classes", without_key(&valid, "tool_classes")));
    variants.push(("missing-version", without_key(&valid, "version")));
    variants.push((
        "grant-missing-capability",
        with_key(
            &valid,
            "grants",
            array(vec![without_key(&grant, "capability")]),
        ),
    ));
    variants.push((
        "request-missing-side-effect",
        with_key(
            &valid,
            "requests",
            array(vec![without_key(&request, "side_effect_level")]),
        ),
    ));
    variants.push((
        "approval-missing-status",
        with_key(
            &valid,
            "approvals",
            array(vec![without_key(&approval, "status")]),
        ),
    ));
    variants.push((
        "approval-pending-default",
        with_key(
            &valid,
            "approvals",
            array(vec![without_key(
                &without_key(&without_key(&approval, "status"), "actor_id"),
                "decided_at",
            )]),
        ),
    ));
    variants.push((
        "approval-missing-actor",
        with_key(
            &valid,
            "approvals",
            array(vec![without_key(&approval, "actor_id")]),
        ),
    ));
    variants.push((
        "time-24h",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T24:00:00+00:00"),
            )]),
        ),
    ));
    variants.push((
        "time-offset-hour",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T08:34:05+05"),
            )]),
        ),
    ));
    variants.push((
        "time-offset-basic",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T08:34:05+0530"),
            )]),
        ),
    ));
    variants.push((
        "time-offset-seconds",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T08:34:05+05:30:15.5"),
            )]),
        ),
    ));
    variants.push((
        "time-comma",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T03:04:05,123456Z"),
            )]),
        ),
    ));
    variants.push((
        "time-week",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-W01-4T00:00:00Z"),
            )]),
        ),
    ));
    variants.push((
        "time-basic",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("20260102T030405Z"),
            )]),
        ),
    ));
    variants.push((
        "time-date-only",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(&approval, "decided_at", text("2026-01-02"))]),
        ),
    ));
    variants.push((
        "time-lowercase-z",
        with_key(
            &valid,
            "approvals",
            array(vec![with_key(
                &approval,
                "decided_at",
                text("2026-01-02T03:04:05z"),
            )]),
        ),
    ));
    variants.push(("not-a-mapping", array(Vec::new())));

    let mut ops = Vec::new();
    for (_, state) in &variants {
        ops.push(object(vec![
            ("op", text("import_state")),
            ("state", state.clone()),
        ]));
        ops.push(object(vec![("op", text("export_state"))]));
    }
    let document = object(vec![
        ("grants", array(vec![text("read")])),
        ("now", text(NOW)),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}
