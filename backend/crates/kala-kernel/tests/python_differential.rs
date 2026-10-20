//! Live Python differential smoke tests for policy, workflow and planning.

mod support;

use kala_json::CanonicalValue;
use support::{
    array, assert_exact_oracle, int, object, policy_native, request_value, run_oracle,
    simple_definition, step_value, text, three_state_definition, workflow_native,
};

const NOW: &str = "2026-01-02T03:04:05Z";

fn policy_document(grants: Vec<CanonicalValue>, ops: Vec<CanonicalValue>) -> CanonicalValue {
    object(vec![
        ("grants", array(grants)),
        ("now", text(NOW)),
        ("ops", array(ops)),
    ])
}

fn decide_op(request: CanonicalValue, now: &str) -> CanonicalValue {
    object(vec![
        ("now", text(now)),
        ("op", text("decide")),
        ("request", request),
    ])
}

#[test]
fn policy_egress_approval_matches_python() {
    let request = request_value(
        "request-approved",
        "state-1",
        "action-1",
        &["network.egress"],
        "egress",
        None,
        None,
    );
    let ops = vec![
        decide_op(request.clone(), NOW),
        object(vec![
            ("op", text("request_approval")),
            ("now", text(NOW)),
            ("request", request.clone()),
        ]),
        object(vec![
            ("actor_id", text("reviewer-1")),
            ("now", text(NOW)),
            ("op", text("approve")),
            ("request", request.clone()),
        ]),
        decide_op(request.clone(), NOW),
        object(vec![("op", text("export_state"))]),
    ];
    let document = policy_document(vec![text("network.egress")], ops);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn policy_stale_binding_and_task_classes_match_python() {
    let first = request_value(
        "request-tool-class-first",
        "state-1",
        "action-1",
        &["simulate"],
        "none",
        None,
        Some("tool_class_change"),
    );
    let repeat = request_value(
        "request-tool-class-repeat",
        "state-1",
        "action-1",
        &["simulate"],
        "none",
        None,
        Some("tool_class_change"),
    );
    let wider = request_value(
        "request-tool-class-wider",
        "state-1",
        "action-1",
        &["simulate", "read"],
        "none",
        None,
        Some("tool_class_change"),
    );
    let rebound = request_value(
        "request-tool-class-first",
        "state-1",
        "action-2",
        &["simulate"],
        "none",
        None,
        Some("tool_class_change"),
    );
    let ops = vec![
        decide_op(first.clone(), NOW),
        object(vec![
            ("actor_id", text("reviewer-1")),
            ("now", text(NOW)),
            ("op", text("approve")),
            ("request", first.clone()),
        ]),
        object(vec![("op", text("approved_tool_classes"))]),
        decide_op(repeat, NOW),
        decide_op(wider, NOW),
        decide_op(rebound, NOW),
        object(vec![
            ("action_digest", text("action-1")),
            ("op", text("forget_tool_class")),
        ]),
        object(vec![("op", text("export_state"))]),
    ];
    let document = policy_document(vec![text("read"), text("simulate")], ops);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn policy_expiry_scope_and_eviction_match_python() {
    let grants = vec![
        object(vec![
            ("capability", text("read")),
            ("expires_at", text("2026-01-02T03:04:35+00:00")),
            ("scope", CanonicalValue::Null),
        ]),
        object(vec![
            ("capability", text("local.write")),
            ("expires_at", CanonicalValue::Null),
            ("scope", text("project-1")),
        ]),
    ];
    let read = request_value("read-request", "s", "a", &["read"], "read", None, None);
    let scoped = request_value(
        "scoped-request",
        "s",
        "a",
        &["local.write"],
        "local_write",
        Some("project-1"),
        None,
    );
    let unscoped = request_value(
        "unscoped-request",
        "s",
        "a",
        &["local.write"],
        "local_write",
        None,
        None,
    );
    let ops = vec![
        decide_op(read.clone(), NOW),
        decide_op(read.clone(), "2026-01-02T03:05:05Z"),
        decide_op(scoped, NOW),
        decide_op(unscoped, NOW),
        object(vec![("op", text("export_state"))]),
    ];
    let document = object(vec![
        ("grants", array(grants)),
        ("max_tracked_requests", int(4)),
        ("now", text(NOW)),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn policy_state_import_matches_python_and_rejects_tampering() {
    let request = request_value(
        "request-valid",
        "state-1",
        "action-1",
        &["read"],
        "none",
        None,
        Some("plan_approval"),
    );
    let ops = vec![
        decide_op(request, NOW),
        object(vec![
            ("actor_id", text("reviewer-1")),
            ("now", text(NOW)),
            ("op", text("approve")),
            (
                "request",
                request_value(
                    "request-valid",
                    "state-1",
                    "action-1",
                    &["read"],
                    "none",
                    None,
                    Some("plan_approval"),
                ),
            ),
        ]),
        object(vec![("op", text("export_state"))]),
    ];
    let document = policy_document(vec![text("read")], ops);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn workflow_checkpoint_and_resume_match_python() {
    let ops = vec![
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
        object(vec![("op", text("resume"))]),
        object(vec![("op", text("state_digest"))]),
        object(vec![("op", text("serialize"))]),
    ];
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", simple_definition(true)),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn workflow_retry_replan_cancel_and_events_match_python() {
    let ops = vec![
        object(vec![
            ("op", text("start")),
            ("workflow_id", text("workflow-1")),
        ]),
        object(vec![
            ("error_code", text("flaky")),
            ("op", text("record_step_failure")),
            ("step_id", text("step-1")),
        ]),
        object(vec![
            ("op", text("request_replan")),
            ("reason", text("different approach")),
        ]),
        object(vec![("op", text("cancel")), ("reason", text("stopped"))]),
        object(vec![("op", text("serialize"))]),
    ];
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", three_state_definition()),
        (
            "max_attempts",
            object(vec![("step-1", int(2)), ("step-2", int(1))]),
        ),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn workflow_reject_and_abort_match_python() {
    for op_name in ["reject_checkpoint", "abort_checkpoint"] {
        let ops = vec![
            object(vec![
                ("op", text("start")),
                ("workflow_id", text("workflow-1")),
            ]),
            object(vec![("op", text("run"))]),
            object(vec![("op", text(op_name)), ("reason", text("declined"))]),
            object(vec![("op", text("serialize"))]),
        ];
        let document = object(vec![
            ("clock", text(NOW)),
            ("definition", simple_definition(true)),
            ("ops", array(ops)),
        ]);
        let oracle = run_oracle("workflow", &document);
        let native = workflow_native(&document);
        assert_exact_oracle(&native, &oracle);
    }
}

#[test]
fn planning_validation_payloads_match_python() {
    let cycles = array(vec![
        step_value("a", "step-1", &["b"], &[], 1),
        step_value("b", "step-2", &["a"], &[], 1),
    ]);
    let cases = array(vec![
        object(vec![("steps", array(Vec::new()))]),
        object(vec![("steps", cycles)]),
        object(vec![(
            "steps",
            array(vec![step_value("a", "absent", &[], &[], 1)]),
        )]),
        object(vec![(
            "steps",
            array(vec![step_value("a", "step-1", &["absent"], &[], 1)]),
        )]),
        object(vec![(
            "steps",
            array(vec![step_value("a", "step-1", &[], &["write"], 1)]),
        )]),
        object(vec![(
            "steps",
            array(vec![
                step_value("a", "step-1", &[], &[], 1),
                step_value("a", "step-2", &[], &[], 1),
            ]),
        )]),
        object(vec![(
            "steps",
            array(vec![step_value("a", "step-1", &[], &[], 3)]),
        )]),
        object(vec![(
            "steps",
            array(vec![step_value("a", "step-1", &[], &["read"], 1)]),
        )]),
    ]);
    let document = object(vec![
        ("allowed_capabilities", array(vec![text("read")])),
        ("definition", three_state_definition()),
        ("max_attempts", int(2)),
        ("cases", cases),
    ]);
    let oracle = run_oracle("planning", &document);
    let native = support::planning_native(&document);
    assert_exact_oracle(&native, &oracle);
}

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn pick<'a, T>(&mut self, values: &'a [T]) -> &'a T {
        &values[(self.next() as usize) % values.len()]
    }
}

#[test]
fn randomized_policy_histories_match_python() {
    let mut random = Lcg::new(0x5eed_1234);
    let capability_names = ["read", "write", "network.egress", "simulate"];
    let mut grants = Vec::new();
    for capability in capability_names {
        if random.next() % 3 != 0 {
            let scope = if random.next() % 4 == 0 {
                Some(text(if random.next() % 2 == 0 {
                    "project-1"
                } else {
                    "project-2"
                }))
            } else {
                None
            };
            let expires = match random.next() % 5 {
                0 => Some(text("2026-01-02T03:04:30+00:00")),
                1 => Some(text("2026-01-02T03:10:00+00:00")),
                _ => None,
            };
            grants.push(object(vec![
                ("capability", text(capability)),
                ("expires_at", expires.unwrap_or(CanonicalValue::Null)),
                ("scope", scope.unwrap_or(CanonicalValue::Null)),
            ]));
        }
    }
    let request_ids = [
        "request-1",
        "request-2",
        "request-3",
        "request-4",
        "request-5",
    ];
    let digest_states = ["state-1", "state-2"];
    let digest_actions = ["action-1", "action-2", "action-3"];
    let mut requests = Vec::new();
    for request_id in request_ids {
        let capability_count = (random.next() % 3) as usize;
        let mut capabilities = Vec::new();
        for _ in 0..capability_count {
            capabilities.push(*random.pick(&capability_names));
        }
        let side_effect = *random.pick(&["none", "read", "local_write", "network", "egress"]);
        let scope = if random.next() % 3 == 0 {
            Some(*random.pick(&["project-1", "project-2"]))
        } else {
            None
        };
        let checkpoint = if side_effect == "egress" || random.next() % 4 == 0 {
            Some(*random.pick(&[
                "plan_approval",
                "tool_class_change",
                "egress",
                "pre_commit",
                "export",
                "provider_degraded",
            ]))
        } else {
            None
        };
        requests.push(request_value(
            request_id,
            random.pick(&digest_states),
            random.pick(&digest_actions),
            &capabilities,
            side_effect,
            scope,
            checkpoint,
        ));
    }
    let times = [
        "2026-01-02T03:04:05Z",
        "2026-01-02T03:10:00Z",
        "2026-01-02T04:00:00Z",
    ];
    let actors = ["local-user", "reviewer-1", "reviewer-2"];
    let mut ops = Vec::new();
    for _ in 0..220 {
        let request = random.pick(&requests).clone();
        let choice = random.next() % 14;
        let op = match choice {
            0 | 1 => object(vec![
                ("now", text(random.pick(&times))),
                ("op", text("decide")),
                ("request", request),
            ]),
            2 => object(vec![
                ("now", text(random.pick(&times))),
                ("op", text("request_approval")),
                ("request", request),
            ]),
            3 | 4 => object(vec![
                ("actor_id", text(random.pick(&actors))),
                ("now", text(random.pick(&times))),
                (
                    "op",
                    text(random.pick(&["approve", "reject", "abort", "invalidate"])),
                ),
                (
                    "reason",
                    text(random.pick(&["reviewed", "declined", "stopped"])),
                ),
                ("request", request),
            ]),
            5 => object(vec![
                ("action_digest", text(random.pick(&digest_actions))),
                ("op", text("forget_tool_class")),
                (
                    "scope",
                    if random.next() % 2 == 0 {
                        text("project-1")
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ]),
            6 => object(vec![
                ("op", text("get_approval")),
                ("request_id", text(random.pick(&request_ids))),
                (
                    "state_digest",
                    if random.next() % 3 == 0 {
                        text(random.pick(&digest_states))
                    } else {
                        CanonicalValue::Null
                    },
                ),
                (
                    "action_digest",
                    if random.next() % 3 == 0 {
                        text(random.pick(&digest_actions))
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ]),
            7 => object(vec![
                ("op", text("grant")),
                (
                    "grant",
                    object(vec![
                        ("capability", text(random.pick(&capability_names))),
                        (
                            "expires_at",
                            if random.next() % 4 == 0 {
                                text("2026-01-02T03:04:30+00:00")
                            } else {
                                CanonicalValue::Null
                            },
                        ),
                        (
                            "scope",
                            if random.next() % 4 == 0 {
                                text("project-1")
                            } else {
                                CanonicalValue::Null
                            },
                        ),
                    ]),
                ),
            ]),
            8 => object(vec![
                ("capability", text(random.pick(&capability_names))),
                ("op", text("revoke")),
            ]),
            9 => object(vec![("op", text("export_state"))]),
            10 => object(vec![("op", text("tracked_request_ids"))]),
            11 => object(vec![("op", text("approvals"))]),
            12 => object(vec![("op", text("approved_tool_classes"))]),
            _ => object(vec![
                ("now", text(random.pick(&times))),
                ("op", text("decide")),
                ("request", request),
            ]),
        };
        ops.push(op);
    }
    let document = object(vec![
        ("grants", array(grants)),
        ("max_tracked_requests", int(4)),
        ("now", text(NOW)),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn randomized_workflow_histories_match_python() {
    let mut random = Lcg::new(0xfeed_4321);
    let mut ops = vec![object(vec![
        ("op", text("start")),
        ("workflow_id", text("workflow-1")),
    ])];
    for _ in 0..180 {
        let choice = random.next() % 12;
        let op = match choice {
            0 | 1 => object(vec![
                ("op", text(random.pick(&["run"]))),
                (
                    "transition_id",
                    if random.next() % 3 == 0 {
                        text(random.pick(&["transition-1", "transition-2", "bogus"]))
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ]),
            2 => object(vec![
                ("op", text("run")),
                ("step_id", text(random.pick(&["step-1", "step-2", "bogus"]))),
                ("succeeded", CanonicalValue::Bool(random.next() % 2 == 0)),
                (
                    "transition_id",
                    if random.next() % 2 == 0 {
                        text(random.pick(&["transition-1", "transition-2"]))
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ]),
            3 => object(vec![
                ("op", text("record_step_success")),
                ("step_id", text(random.pick(&["step-1", "step-2"]))),
            ]),
            4 => object(vec![
                (
                    "error_code",
                    text(random.pick(&["flaky", "solver_diverged", "bad code"])),
                ),
                ("op", text("record_step_failure")),
                ("step_id", text(random.pick(&["step-1", "step-2"]))),
            ]),
            5 => object(vec![("op", text("resume"))]),
            6 => object(vec![
                ("op", text("request_replan")),
                ("reason", text("different approach")),
            ]),
            7 => object(vec![("op", text("serialize"))]),
            8 => object(vec![("op", text("state_digest"))]),
            9 => object(vec![("op", text("snapshot"))]),
            10 => object(vec![("op", text("state"))]),
            _ => object(vec![("op", text("run"))]),
        };
        ops.push(op);
    }
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", three_state_definition()),
        (
            "max_attempts",
            object(vec![("step-1", int(2)), ("step-2", int(2))]),
        ),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn randomized_checkpoint_histories_match_python() {
    let mut random = Lcg::new(0xc0ff_ee12);
    let mut ops = vec![object(vec![
        ("op", text("start")),
        ("workflow_id", text("workflow-1")),
    ])];
    ops.push(object(vec![("op", text("run"))]));
    let decisions = [
        "approve", "approved", "reject", "rejected", "abort", "aborted", "pending", "maybe",
    ];
    for _ in 0..160 {
        let choice = random.next() % 12;
        let now = text(random.pick(&[NOW, "2026-01-02T03:10:00Z"]));
        let op = match choice {
            0 | 1 => object(vec![
                ("actor_id", text(random.pick(&["local-user", "reviewer"]))),
                ("now", now),
                ("op", text("approve_checkpoint")),
                (
                    "state_digest",
                    if random.next() % 4 == 0 {
                        text(&"0".repeat(64))
                    } else {
                        CanonicalValue::Null
                    },
                ),
                (
                    "action_digest",
                    if random.next() % 4 == 0 {
                        text(&"1".repeat(64))
                    } else {
                        CanonicalValue::Null
                    },
                ),
            ]),
            2 => object(vec![
                ("now", now),
                ("op", text("reject_checkpoint")),
                ("reason", text("declined")),
            ]),
            3 => object(vec![
                ("now", now),
                ("op", text("abort_checkpoint")),
                ("reason", text("stopped")),
            ]),
            4 | 5 => object(vec![
                ("decision", text(random.pick(&decisions))),
                ("now", now),
                ("op", text("decide_checkpoint")),
                ("reason", text("decided")),
            ]),
            6 => object(vec![
                ("decision", text(random.pick(&decisions))),
                ("now", now),
                ("op", text("resolve_checkpoint")),
                ("reason", text("resolved")),
            ]),
            7 => object(vec![("op", text("resume"))]),
            8 => object(vec![("op", text("serialize"))]),
            9 => object(vec![("op", text("state_digest"))]),
            10 => object(vec![("op", text("cancel")), ("reason", text("cancelled"))]),
            _ => object(vec![("op", text("run"))]),
        };
        ops.push(op);
    }
    let document = object(vec![
        ("clock", text(NOW)),
        ("definition", simple_definition(true)),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("workflow", &document);
    let native = workflow_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn policy_offsets_unicode_and_c0_rules_match_python() {
    let surrogate_text = CanonicalValue::Text(
        kala_json::JsonText::from_code_points(vec![u32::from(b'r'), 0xD800, u32::from(b'x')])
            .expect("surrogate"),
    );
    let surrogate_request = object(vec![
        ("action_digest", text("action-1")),
        ("checkpoint_type", CanonicalValue::Null),
        ("destructive", CanonicalValue::Bool(false)),
        ("egress", CanonicalValue::Bool(true)),
        ("request_id", surrogate_text.clone()),
        (
            "requested_capabilities",
            array(vec![text("network.egress")]),
        ),
        ("scope", CanonicalValue::Null),
        ("side_effect_level", text("egress")),
        ("state_digest", text("state-1")),
    ]);
    let ops = vec![
        object(vec![
            ("actor_id", text("  reviewer  ")),
            ("now", text("2026-01-02T08:34:05+05:30")),
            ("op", text("approve")),
            ("request", surrogate_request.clone()),
            ("reason", text("ok")),
        ]),
        object(vec![
            ("now", text("2026-01-02T08:34:30+05:30:15.5")),
            ("op", text("decide")),
            ("request", surrogate_request),
        ]),
        object(vec![("op", text("export_state"))]),
    ];
    let document = object(vec![
        (
            "grants",
            array(vec![object(vec![
                ("capability", text("network.egress")),
                ("expires_at", text("2026-01-02T08:34:35+05:30:15.5")),
                ("scope", CanonicalValue::Null),
            ])]),
        ),
        ("now", text("2026-01-02T03:04:05Z")),
        ("ops", array(ops)),
    ]);
    let oracle = run_oracle("policy", &document);
    let native = policy_native(&document);
    assert_exact_oracle(&native, &oracle);
}

#[test]
fn planning_full_validation_matrix_matches_python() {
    let steps = |values: Vec<CanonicalValue>| object(vec![("steps", array(values))]);
    let cases = array(vec![
        steps(vec![
            step_value("a", "step-1", &[], &[], 1),
            step_value("b", "step-2", &[], &[], 1),
        ]),
        object(vec![
            ("steps", array(vec![step_value("a", "step-1", &[], &[], 1)])),
            ("replanning_attempts", int(-1)),
        ]),
        object(vec![
            ("steps", array(vec![step_value("a", "step-1", &[], &[], 1)])),
            ("replanning_attempts", text("1")),
        ]),
        object(vec![
            ("steps", array(vec![step_value("a", "step-1", &[], &[], 1)])),
            ("replanning_attempts", CanonicalValue::Bool(true)),
        ]),
        object(vec![
            ("steps", array(vec![step_value("a", "step-1", &[], &[], 1)])),
            ("replanning_attempts", CanonicalValue::Null),
        ]),
        object(vec![
            ("steps", array(vec![step_value("a", "step-1", &[], &[], 1)])),
            ("replanning_attempts", int(3)),
        ]),
        steps(vec![step_value("a", "step-1", &[], &[], 0)]),
        steps(vec![step_value("a", "step-1", &[], &[], 1)]),
        object(vec![
            (
                "steps",
                array(vec![step_value("a", "step-1", &[], &["read"], 1)]),
            ),
            ("allowed_capabilities", array(Vec::new())),
        ]),
        object(vec![
            (
                "steps",
                array(vec![step_value("a", "step-1", &[], &["write"], 1)]),
            ),
            ("allowed_capabilities", array(vec![text("read")])),
        ]),
        object(vec![
            (
                "steps",
                array(vec![
                    step_value("a", "step-1", &[], &[], 2),
                    step_value("b", "step-2", &[], &[], 2),
                ]),
            ),
            ("granted_capabilities", array(vec![text("read")])),
        ]),
        steps(vec![
            step_value("a", "step-1", &[], &[], 1),
            step_value("a", "absent", &[], &[], 1),
        ]),
        steps(vec![step_value("a", "absent", &[], &[], 0)]),
    ]);
    let document = object(vec![
        (
            "allowed_capabilities",
            array(vec![text("read"), text("write")]),
        ),
        ("definition", three_state_definition()),
        ("max_attempts", int(2)),
        ("max_steps", int(1)),
        ("max_total_attempts", int(2)),
        ("cases", cases),
    ]);
    let oracle = run_oracle("planning", &document);
    let native = support::planning_native(&document);
    assert_exact_oracle(&native, &oracle);
}
