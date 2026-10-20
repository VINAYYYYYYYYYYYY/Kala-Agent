//! Independent review probe 1 and 2: policy state import/from_state boundaries
//! and approval replay/finality.
//!
//! These tests are adversarial gates against `kala_kernel::policy`. They are
//! deliberately independent of the implementer's tests: every malformed
//! document must be rejected, the previous engine must stay byte-identical,
//! and no failure path may leave a grant, approval, or tool-class cache behind.

mod support;

use kala_contracts::Text;
use kala_json::CanonicalValue;
use kala_kernel::policy::{
    DecisionContext, GrantSpec, PolicyDecisionStatus, PolicyEngine, PolicyRequest,
};
use kala_kernel::{ApprovalKind, ApprovalStatus, KernelTimestamp, SideEffectLevel};
use support::{array, assert_exact_oracle, get, int, object, policy_native, run_oracle, text};

const NOW: &str = "2026-01-02T03:04:05Z";
const LATER: &str = "2026-01-02T03:04:30Z";

fn at(value: &str) -> KernelTimestamp {
    KernelTimestamp::parse_isoformat(value).expect("timestamp")
}

fn request(
    request_id: &str,
    state_digest: &str,
    action_digest: &str,
    capabilities: &[&str],
    side_effect: SideEffectLevel,
    scope: Option<&str>,
    checkpoint: Option<ApprovalKind>,
) -> PolicyRequest {
    PolicyRequest::from_parts(
        Text::new(request_id),
        Text::new(state_digest),
        Text::new(action_digest),
        capabilities.iter().map(|value| Text::new(value)),
        side_effect,
        false,
        false,
        scope.map(Text::new),
        checkpoint,
    )
    .expect("request")
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

fn first(value: &CanonicalValue) -> CanonicalValue {
    match value {
        CanonicalValue::Array(values) => values.first().expect("nonempty").clone(),
        _ => panic!("array"),
    }
}

fn grant_value(capability: &str, scope: Option<&str>, expires_at: Option<&str>) -> CanonicalValue {
    object(vec![
        ("capability", text(capability)),
        ("scope", scope.map_or(CanonicalValue::Null, text)),
        ("expires_at", expires_at.map_or(CanonicalValue::Null, text)),
    ])
}

fn request_value(
    request_id: &str,
    state_digest: &str,
    action_digest: &str,
    capabilities: &[&str],
    side_effect: &str,
    scope: Option<&str>,
    checkpoint: Option<&str>,
) -> CanonicalValue {
    object(vec![
        ("action_digest", text(action_digest)),
        (
            "checkpoint_type",
            checkpoint.map_or(CanonicalValue::Null, text),
        ),
        ("destructive", CanonicalValue::Bool(false)),
        ("egress", CanonicalValue::Bool(side_effect == "egress")),
        ("request_id", text(request_id)),
        (
            "requested_capabilities",
            array(capabilities.iter().map(|value| text(value)).collect()),
        ),
        ("scope", scope.map_or(CanonicalValue::Null, text)),
        ("side_effect_level", text(side_effect)),
        ("state_digest", text(state_digest)),
    ])
}

fn approval_value(
    request_id: &str,
    state_digest: &str,
    action_digest: &str,
    status: &str,
    actor: Option<&str>,
    decided_at: Option<&str>,
) -> CanonicalValue {
    object(vec![
        ("request_id", text(request_id)),
        ("state_digest", text(state_digest)),
        ("action_digest", text(action_digest)),
        ("status", text(status)),
        ("actor_id", actor.map_or(CanonicalValue::Null, text)),
        ("decided_at", decided_at.map_or(CanonicalValue::Null, text)),
        ("reason", CanonicalValue::Null),
    ])
}

fn base_state() -> CanonicalValue {
    object(vec![
        ("version", int(1)),
        (
            "grants",
            array(vec![grant_value(
                "network.egress",
                Some("project-1"),
                Some(LATER),
            )]),
        ),
        (
            "requests",
            array(vec![request_value(
                " request-1 ",
                "state-1",
                "action-1",
                &[" network.egress ", "network.egress"],
                "egress",
                Some(" project-1 "),
                None,
            )]),
        ),
        (
            "approvals",
            array(vec![approval_value(
                "request-1",
                "state-1",
                "action-1",
                "expired",
                Some("reviewer"),
                Some("2026-01-02T03:04:05+05:30"),
            )]),
        ),
        (
            "tool_classes",
            array(vec![object(vec![
                ("action_digest", text("action-1")),
                ("capabilities", array(vec![text("read"), text(" read ")])),
                ("scope", CanonicalValue::Null),
            ])]),
        ),
        ("max_tracked_requests", int(4)),
    ])
}

/// Every document here is rejected by the Python reference (verified by the
/// exact-oracle test below, which includes the same builders).
fn rejected_states() -> Vec<(&'static str, CanonicalValue)> {
    let base = base_state();
    let grants = get(&base, "grants").expect("grants").clone();
    let requests = get(&base, "requests").expect("requests").clone();
    let approvals = get(&base, "approvals").expect("approvals").clone();
    let tools = get(&base, "tool_classes").expect("tools").clone();
    let grant = first(&grants);
    let request = first(&requests);
    let approval = first(&approvals);
    let tool = first(&tools);

    vec![
        ("not-an-object", text("{}")),
        ("array", array(vec![])),
        ("null", CanonicalValue::Null),
        ("unknown-top-key", with_key(&base, "extra", int(1))),
        ("missing-version", without_key(&base, "version")),
        (
            "version-bool",
            with_key(&base, "version", CanonicalValue::Bool(true)),
        ),
        ("version-string", with_key(&base, "version", text("1"))),
        (
            "version-float",
            with_key(&base, "version", CanonicalValue::Float(1.0)),
        ),
        ("version-negative", with_key(&base, "version", int(-1))),
        ("version-unsupported", with_key(&base, "version", int(2))),
        (
            "limit-zero",
            with_key(&base, "max_tracked_requests", int(0)),
        ),
        (
            "limit-negative",
            with_key(&base, "max_tracked_requests", int(-1)),
        ),
        (
            "limit-bool",
            with_key(&base, "max_tracked_requests", CanonicalValue::Bool(true)),
        ),
        (
            "limit-string",
            with_key(&base, "max_tracked_requests", text("4")),
        ),
        (
            "limit-null",
            with_key(&base, "max_tracked_requests", CanonicalValue::Null),
        ),
        (
            "grants-not-array",
            with_key(&base, "grants", object(vec![])),
        ),
        (
            "grant-not-object",
            with_key(&base, "grants", array(vec![int(1)])),
        ),
        (
            "grant-missing-capability",
            with_key(
                &base,
                "grants",
                array(vec![without_key(&grant, "capability")]),
            ),
        ),
        (
            "grant-wildcard",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "capability", text("*"))]),
            ),
        ),
        (
            "grant-unsafe-capability",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "capability", text("a b"))]),
            ),
        ),
        (
            "grant-long-capability",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "capability", text(&"a".repeat(257)))]),
            ),
        ),
        (
            "grant-unsafe-scope",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "scope", text("a b"))]),
            ),
        ),
        (
            "grant-naive-expiry",
            with_key(
                &base,
                "grants",
                array(vec![with_key(
                    &grant,
                    "expires_at",
                    text("2026-01-02T03:04:05"),
                )]),
            ),
        ),
        (
            "grant-bad-expiry",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "expires_at", text("never"))]),
            ),
        ),
        (
            "grant-extra-key",
            with_key(
                &base,
                "grants",
                array(vec![with_key(&grant, "extra", int(1))]),
            ),
        ),
        (
            "request-not-array",
            with_key(&base, "requests", text("request")),
        ),
        (
            "request-extra-key",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "extra", int(1))]),
            ),
        ),
        (
            "request-missing-id",
            with_key(
                &base,
                "requests",
                array(vec![without_key(&request, "request_id")]),
            ),
        ),
        (
            "request-empty-id",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "request_id", text(""))]),
            ),
        ),
        (
            "request-long-digest",
            with_key(
                &base,
                "requests",
                array(vec![with_key(
                    &request,
                    "state_digest",
                    text(&"s".repeat(257)),
                )]),
            ),
        ),
        (
            "request-capabilities-not-array",
            with_key(
                &base,
                "requests",
                array(vec![with_key(
                    &request,
                    "requested_capabilities",
                    text("network.egress"),
                )]),
            ),
        ),
        (
            "request-capability-non-string",
            with_key(
                &base,
                "requests",
                array(vec![with_key(
                    &request,
                    "requested_capabilities",
                    array(vec![int(1)]),
                )]),
            ),
        ),
        (
            "request-side-effect-bool",
            with_key(
                &base,
                "requests",
                array(vec![with_key(
                    &request,
                    "side_effect_level",
                    CanonicalValue::Bool(true),
                )]),
            ),
        ),
        (
            "request-side-effect-unknown",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "side_effect_level", text("root"))]),
            ),
        ),
        (
            "request-destructive-int",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "destructive", int(1))]),
            ),
        ),
        (
            "request-destructive-null",
            with_key(
                &base,
                "requests",
                array(vec![with_key(
                    &request,
                    "destructive",
                    CanonicalValue::Null,
                )]),
            ),
        ),
        (
            "request-egress-string",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "egress", text("true"))]),
            ),
        ),
        (
            "request-scope-int",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "scope", int(1))]),
            ),
        ),
        (
            "request-checkpoint-unknown",
            with_key(
                &base,
                "requests",
                array(vec![with_key(&request, "checkpoint_type", text("root"))]),
            ),
        ),
        (
            "request-duplicate-normalized-id",
            with_key(
                &base,
                "requests",
                array(vec![
                    request.clone(),
                    with_key(&request, "request_id", text("request-1")),
                ]),
            ),
        ),
        ("approval-not-array", with_key(&base, "approvals", int(1))),
        (
            "approval-extra-key",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "extra", int(1))]),
            ),
        ),
        (
            "approval-orphan",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "request_id", text("ghost"))]),
            ),
        ),
        (
            "approval-missing-status",
            with_key(
                &base,
                "approvals",
                array(vec![without_key(&approval, "status")]),
            ),
        ),
        (
            "approval-status-unknown",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "status", text("APPROVED"))]),
            ),
        ),
        (
            "approval-pending-with-decision",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "status", text("pending"))]),
            ),
        ),
        (
            "approval-approved-without-actor",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "actor_id", CanonicalValue::Null)]),
            ),
        ),
        (
            "approval-empty-reason",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(&approval, "reason", text(""))]),
            ),
        ),
        (
            "approval-naive-time",
            with_key(
                &base,
                "approvals",
                array(vec![with_key(
                    &approval,
                    "decided_at",
                    text("2026-01-02T03:04:05"),
                )]),
            ),
        ),
        (
            "approval-duplicate",
            with_key(&base, "approvals", array(vec![approval.clone(), approval])),
        ),
        (
            "tool-not-array",
            with_key(&base, "tool_classes", object(vec![])),
        ),
        (
            "tool-extra-key",
            with_key(
                &base,
                "tool_classes",
                array(vec![with_key(&tool, "extra", int(1))]),
            ),
        ),
        (
            "tool-missing-digest",
            with_key(
                &base,
                "tool_classes",
                array(vec![without_key(&tool, "action_digest")]),
            ),
        ),
        (
            "tool-caps-not-array",
            with_key(
                &base,
                "tool_classes",
                array(vec![with_key(&tool, "capabilities", text("read"))]),
            ),
        ),
        (
            "tool-cap-unsafe",
            with_key(
                &base,
                "tool_classes",
                array(vec![with_key(
                    &tool,
                    "capabilities",
                    array(vec![text("a b")]),
                )]),
            ),
        ),
        (
            "tool-scope-int",
            with_key(
                &base,
                "tool_classes",
                array(vec![with_key(&tool, "scope", int(1))]),
            ),
        ),
        (
            "tool-duplicate",
            with_key(&base, "tool_classes", array(vec![tool.clone(), tool])),
        ),
    ]
}

/// Documents that Python accepts because the source state loader supplies
/// defaults or normalizes values. Native import must accept them too.
fn accepted_states() -> Vec<(&'static str, CanonicalValue)> {
    let mut states = vec![
        ("minimal", object(vec![("version", int(1))])),
        ("base", base_state()),
        ("no-grants", without_key(&base_state(), "grants")),
        (
            "no-requests",
            without_key(&without_key(&base_state(), "requests"), "approvals"),
        ),
        ("no-approvals", without_key(&base_state(), "approvals")),
        ("no-tools", without_key(&base_state(), "tool_classes")),
        (
            "no-limit",
            without_key(&base_state(), "max_tracked_requests"),
        ),
    ];
    let base = base_state();
    let grants = get(&base, "grants").expect("grants").clone();
    let grant = first(&grants);
    states.push((
        "duplicate-grant-normalized",
        with_key(
            &base,
            "grants",
            array(vec![
                grant.clone(),
                with_key(&grant, "capability", text(" network.egress ")),
            ]),
        ),
    ));
    states
}

#[test]
fn import_boundary_matches_python_and_never_mutates_on_failure() {
    let mut ops = Vec::new();
    for (_, state) in rejected_states() {
        ops.push(object(vec![("op", text("import_state")), ("state", state)]));
        ops.push(object(vec![("op", text("export_state"))]));
        ops.push(object(vec![("op", text("grants"))]));
        ops.push(object(vec![("op", text("approvals"))]));
        ops.push(object(vec![("op", text("approved_tool_classes"))]));
        ops.push(object(vec![("op", text("tracked_request_ids"))]));
    }
    for (_, state) in accepted_states() {
        ops.push(object(vec![("op", text("import_state")), ("state", state)]));
        ops.push(object(vec![("op", text("export_state"))]));
    }
    let oracle = run_oracle(
        "policy",
        &object(vec![("now", text(NOW)), ("ops", array(ops.clone()))]),
    );
    let native = policy_native(&object(vec![("now", text(NOW)), ("ops", array(ops))]));
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn failed_imports_leave_the_engine_byte_identical() {
    let engine = PolicyEngine::new()
        .with_grants([GrantSpec::from("read")])
        .expect("grants")
        .with_max_tracked_requests(4.into())
        .expect("limit");
    let tracked = request(
        "tracked",
        "state-tracked",
        "action-tracked",
        &["read"],
        SideEffectLevel::None,
        None,
        None,
    );
    engine
        .decide(&tracked, None)
        .expect("decision")
        .allowed()
        .then_some(())
        .expect("baseline read allowed");
    let pending = request(
        "pending",
        "state-pending",
        "action-pending",
        &["read"],
        SideEffectLevel::LocalWrite,
        None,
        None,
    );
    engine
        .request_approval(&pending, Some(at(NOW)))
        .expect("pending");

    let baseline = engine.export_state();
    let baseline_json = baseline.to_json().expect("json");
    let baseline_grants = engine.grants();
    let baseline_ids = engine.tracked_request_ids();
    let baseline_approvals = engine.approvals();

    for (label, state) in rejected_states() {
        let outcome = engine.import_state(&state);
        assert!(outcome.is_err(), "{label} was accepted");
        assert_eq!(
            engine.export_state(),
            baseline,
            "{label} mutated the engine state"
        );
        assert_eq!(
            engine.export_state_json().expect("json"),
            baseline_json,
            "{label} changed the exported bytes"
        );
        assert_eq!(engine.grants(), baseline_grants, "{label} changed grants");
        assert_eq!(
            engine.tracked_request_ids(),
            baseline_ids,
            "{label} changed tracked requests"
        );
        assert_eq!(
            engine.approvals(),
            baseline_approvals,
            "{label} changed approvals"
        );
    }

    // `from_state` must reject the same corpus and must not somehow build a
    // widened fresh engine.
    for (label, state) in rejected_states() {
        assert!(
            PolicyEngine::from_state(&state).is_err(),
            "{label} was accepted by from_state"
        );
        let json = support::canonical(&state);
        assert!(
            PolicyEngine::from_state_json(&json, None, None).is_err(),
            "{label} was accepted by from_state_json"
        );
    }

    // The engine is still fully usable after every failed import.
    assert!(engine.decide(&tracked, None).expect("decision").allowed());
}

#[test]
fn accepted_imports_restore_normalized_values_and_limits() {
    for (label, state) in accepted_states() {
        let engine = PolicyEngine::from_state(&state).unwrap_or_else(|error| {
            panic!("{label} was rejected: {}", error.message());
        });
        let exported = engine.export_state();
        exported
            .to_json()
            .unwrap_or_else(|error| panic!("{label} did not re-serialize: {}", error.message()));

        let json = support::canonical(&state);
        let via_json = PolicyEngine::from_state_json(&json, None, None)
            .unwrap_or_else(|error| panic!("{label} json import failed: {}", error.message()));
        assert_eq!(
            via_json.export_state(),
            exported,
            "{label} diverged by path"
        );
    }

    // The state's own limit wins over the factory fallback, exactly as the
    // Python `from_state(...)` factory does.
    let state = base_state();
    let engine = PolicyEngine::from_state_with(&state, None, Some(2.into())).expect("restore");
    assert_eq!(engine.max_tracked_requests(), 4.into());
    let without_limit = without_key(&state, "max_tracked_requests");
    let engine =
        PolicyEngine::from_state_with(&without_limit, None, Some(2.into())).expect("restore");
    assert_eq!(engine.max_tracked_requests(), 2.into());
}

#[test]
fn rejected_type_confusion_never_widens_privileges() {
    let engine = PolicyEngine::new();
    let states = rejected_states();
    for (label, state) in &states {
        assert!(engine.import_state(state).is_err(), "{label} was accepted");
        assert!(engine.grants().is_empty(), "{label} left a grant behind");
        assert!(
            engine.approvals().is_empty(),
            "{label} left an approval behind"
        );
        assert!(
            engine.approved_tool_classes().is_empty(),
            "{label} left a tool-class cache behind"
        );
    }
    // No malformed document can turn an unprivileged engine into one that
    // allows egress or a read.
    let egress = request(
        "egress",
        "state",
        "action",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        Some(ApprovalKind::Egress),
    );
    let decision = engine.decide(&egress, Some(at(NOW))).expect("decision");
    assert_eq!(decision.status(), PolicyDecisionStatus::Denied);
    let read = request(
        "read",
        "state",
        "action",
        &["read"],
        SideEffectLevel::None,
        None,
        None,
    );
    assert!(!engine.decide(&read, None).expect("decision").allowed());

    // A default-shaped document is a valid import that clears privileges; it
    // must not be mistaken for a rejection and must not retain old grants.
    let privileged = PolicyEngine::from_grants([GrantSpec::from("read")]).expect("engine");
    privileged
        .import_state(&object(vec![("version", int(1))]))
        .expect("minimal import");
    assert!(privileged.grants().is_empty());
}

#[test]
fn approval_replay_matrix_matches_python() {
    let grant = grant_value("network.egress", None, Some(LATER));
    let replay = request_value(
        "replay",
        "s1",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let replay_changed = request_value(
        "replay",
        "s2",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let reject = request_value(
        "reject",
        "s1",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let reject_changed = request_value(
        "reject",
        "s2",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let expiry = request_value(
        "expiry",
        "s1",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let override_request = request_value(
        "override",
        "s1",
        "a1",
        &["network.egress"],
        "egress",
        None,
        None,
    );

    let expired_import = object(vec![
        ("version", int(1)),
        ("grants", array(vec![grant.clone()])),
        (
            "requests",
            array(vec![request_value(
                "imported-expired",
                "s1",
                "a1",
                &["network.egress"],
                "egress",
                None,
                None,
            )]),
        ),
        (
            "approvals",
            array(vec![approval_value(
                "imported-expired",
                "s1",
                "a1",
                "expired",
                Some("reviewer"),
                Some(NOW),
            )]),
        ),
        ("tool_classes", array(vec![])),
    ]);
    let mismatch_import = object(vec![
        ("version", int(1)),
        ("grants", array(vec![grant.clone()])),
        (
            "requests",
            array(vec![request_value(
                "imported-mismatch",
                "s1",
                "a1",
                &["network.egress"],
                "egress",
                None,
                None,
            )]),
        ),
        (
            "approvals",
            array(vec![approval_value(
                "imported-mismatch",
                "s9",
                "a9",
                "approved",
                Some("reviewer"),
                Some(NOW),
            )]),
        ),
        ("tool_classes", array(vec![])),
    ]);
    let exact_import = object(vec![
        ("version", int(1)),
        ("grants", array(vec![grant])),
        (
            "requests",
            array(vec![request_value(
                "imported-exact",
                "s1",
                "a1",
                &["network.egress"],
                "egress",
                None,
                None,
            )]),
        ),
        (
            "approvals",
            array(vec![approval_value(
                "imported-exact",
                "s1",
                "a1",
                "approved",
                Some("reviewer"),
                Some(NOW),
            )]),
        ),
        ("tool_classes", array(vec![])),
    ]);

    let approve = |request: &CanonicalValue| {
        object(
            [
                vec![("op", text("approve")), ("request", request.clone())],
                vec![("actor_id", text("reviewer")), ("now", text(NOW))],
            ]
            .concat(),
        )
    };
    let decide_at = |request: &CanonicalValue, now: &str| {
        object(vec![
            ("op", text("decide")),
            ("request", request.clone()),
            ("now", text(now)),
        ])
    };
    let ops = array(vec![
        decide_at(&replay, "2026-01-02T03:04:20Z"),
        approve(&replay),
        approve(&replay),
        decide_at(&replay_changed, "2026-01-02T03:04:20Z"),
        approve(&replay_changed),
        object(vec![
            ("op", text("reject")),
            ("request", reject.clone()),
            ("actor_id", text("reviewer")),
            ("reason", text("no")),
            ("now", text(NOW)),
        ]),
        approve(&reject),
        object(vec![
            ("op", text("invalidate")),
            ("request", reject.clone()),
            ("actor_id", text("reviewer")),
            ("now", text(NOW)),
        ]),
        decide_at(&reject, "2026-01-02T03:04:20Z"),
        decide_at(&reject_changed, "2026-01-02T03:04:20Z"),
        approve(&reject_changed),
        decide_at(&expiry, "2026-01-02T03:04:29Z"),
        object(vec![
            ("op", text("approve")),
            ("request", expiry.clone()),
            ("actor_id", text("reviewer")),
            ("now", text("2026-01-02T03:04:29Z")),
        ]),
        decide_at(&expiry, LATER),
        decide_at(&expiry, "2026-01-02T03:04:29Z"),
        decide_at(&override_request, "2026-01-02T03:04:20Z"),
        object(vec![
            ("op", text("approve")),
            ("request", override_request),
            ("state_digest", text("s9")),
            ("actor_id", text("reviewer")),
            ("now", text(NOW)),
        ]),
        object(vec![("op", text("export_state"))]),
        object(vec![
            ("op", text("import_state")),
            ("state", expired_import),
        ]),
        object(vec![
            ("op", text("decide")),
            (
                "request",
                request_value(
                    "imported-expired",
                    "s1",
                    "a1",
                    &["network.egress"],
                    "egress",
                    None,
                    None,
                ),
            ),
            ("now", text("2026-01-02T03:04:20Z")),
        ]),
        object(vec![
            ("op", text("import_state")),
            ("state", mismatch_import),
        ]),
        object(vec![
            ("op", text("decide")),
            (
                "request",
                request_value(
                    "imported-mismatch",
                    "s1",
                    "a1",
                    &["network.egress"],
                    "egress",
                    None,
                    None,
                ),
            ),
            ("now", text("2026-01-02T03:04:20Z")),
        ]),
        object(vec![("op", text("import_state")), ("state", exact_import)]),
        object(vec![
            ("op", text("decide")),
            (
                "request",
                request_value(
                    "imported-exact",
                    "s1",
                    "a1",
                    &["network.egress"],
                    "egress",
                    None,
                    None,
                ),
            ),
            ("now", text("2026-01-02T03:04:20Z")),
        ]),
        object(vec![("op", text("export_state"))]),
    ]);
    let document = object(vec![
        (
            "grants",
            array(vec![grant_value("network.egress", None, Some(LATER))]),
        ),
        ("now", text(NOW)),
        ("ops", ops),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn replayed_approval_never_authorizes_a_changed_binding() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")]).expect("engine");
    let first = request(
        "replay",
        "state-1",
        "action-1",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );
    let changed = request(
        "replay",
        "state-2",
        "action-1",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );

    let pending = engine.decide(&first, Some(at(NOW))).expect("decision");
    assert_eq!(pending.status(), PolicyDecisionStatus::Pending);
    let context = DecisionContext::new()
        .with_actor_id(Text::new("reviewer"))
        .with_now(at(NOW));
    let approved = engine.approve(&first, &context).expect("approve");
    assert_eq!(approved.status(), PolicyDecisionStatus::Allowed);
    assert_eq!(
        engine.approve(&first, &context),
        Err(kala_kernel::policy::PolicyError::Transition(
            "approval for replay is already approved".to_owned()
        ))
    );

    let invalidated = engine.decide(&changed, Some(at(NOW))).expect("decision");
    assert_eq!(invalidated.status(), PolicyDecisionStatus::Invalidated);
    assert!(!invalidated.allowed());
    let record = engine
        .get_approval(
            &Text::new("replay"),
            Some(&Text::new("state-1")),
            Some(&Text::new("action-1")),
        )
        .expect("lookup")
        .expect("record");
    assert_eq!(record.status(), ApprovalStatus::Invalidated);

    // The changed action is not allowed merely because some approval for the
    // request ID exists; it needs its own explicit approval and then it works.
    assert_eq!(
        engine
            .decide(&changed, Some(at(NOW)))
            .expect("decision")
            .status(),
        PolicyDecisionStatus::Pending
    );
    assert!(
        !engine
            .decide(&changed, Some(at(NOW)))
            .expect("decision")
            .allowed()
    );
    assert!(
        engine
            .approve(&changed, &context)
            .expect("approve")
            .allowed()
    );

    // The same request ID with a changed *action* digest is equally stale.
    let action_first = request(
        "replay-action",
        "state-1",
        "action-1",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );
    let action_changed = request(
        "replay-action",
        "state-1",
        "action-2",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );
    assert_eq!(
        engine
            .decide(&action_first, Some(at(NOW)))
            .expect("decision")
            .status(),
        PolicyDecisionStatus::Pending
    );
    assert!(
        engine
            .approve(&action_first, &context)
            .expect("approve")
            .allowed()
    );
    let action_invalidated = engine
        .decide(&action_changed, Some(at(NOW)))
        .expect("decision");
    assert_eq!(
        action_invalidated.status(),
        PolicyDecisionStatus::Invalidated
    );
    assert!(!action_invalidated.allowed());
    assert_eq!(
        engine
            .decide(&action_changed, Some(at(NOW)))
            .expect("decision")
            .status(),
        PolicyDecisionStatus::Pending
    );
    assert!(
        engine
            .approve(&action_changed, &context)
            .expect("approve")
            .allowed()
    );
}

/// Review finding D1 (failing-if-broken): Python's `decide()` returns
/// `actor_id=approval.actor_id` for every non-approved approval status
/// (`rejected`, `aborted`, `invalidated`, `expired`). The native
/// `decide_locked` passes `None` in that branch, so denial decisions lose the
/// accountable approver. This test must pass before the crate is accepted.
#[test]
fn denied_decisions_preserve_the_acting_approver() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")]).expect("engine");
    let denied_request = request(
        "denied",
        "state-1",
        "action-1",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );
    let context = DecisionContext::new()
        .with_actor_id(Text::new("reviewer"))
        .with_now(at(NOW));

    let rejected = engine.reject(&denied_request, &context).expect("reject");
    assert_eq!(rejected.status(), PolicyDecisionStatus::Rejected);
    assert_eq!(
        rejected.actor_id().map(Text::to_string_lossy),
        Some("reviewer".to_owned()),
        "rejected decisions must retain the deciding actor"
    );

    let repeated = engine
        .decide(&denied_request, Some(at(NOW)))
        .expect("decide");
    assert_eq!(repeated.status(), PolicyDecisionStatus::Rejected);
    assert_eq!(
        repeated.actor_id().map(Text::to_string_lossy),
        Some("reviewer".to_owned()),
        "a repeated terminal read must retain the deciding actor"
    );

    let expired_state = object(vec![
        ("version", int(1)),
        (
            "grants",
            array(vec![grant_value("network.egress", None, None)]),
        ),
        (
            "requests",
            array(vec![request_value(
                "expired",
                "state-1",
                "action-1",
                &["network.egress"],
                "egress",
                None,
                None,
            )]),
        ),
        (
            "approvals",
            array(vec![approval_value(
                "expired",
                "state-1",
                "action-1",
                "expired",
                Some("previous-reviewer"),
                Some(NOW),
            )]),
        ),
        ("tool_classes", array(vec![])),
    ]);
    let expired_engine = PolicyEngine::from_state(&expired_state).expect("state");
    let expired_request = request(
        "expired",
        "state-1",
        "action-1",
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    );
    let expired = expired_engine
        .decide(&expired_request, Some(at(NOW)))
        .expect("decide");
    assert_eq!(expired.status(), PolicyDecisionStatus::Invalidated);
    assert_eq!(
        expired.actor_id().map(Text::to_string_lossy),
        Some("previous-reviewer".to_owned()),
        "invalidated/expired decisions must retain the resolving actor"
    );
}
