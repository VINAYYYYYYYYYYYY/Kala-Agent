//! Independent review probes 4 and 5 for policy: concurrency/torn-state and
//! resource bounds (tracking/pruning, arbitrary limits, deep JSON, huge ids).

mod support;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use kala_contracts::Text;
use kala_json::CanonicalValue;
use kala_kernel::policy::{
    DecisionContext, GrantSpec, PolicyDecisionStatus, PolicyEngine, PolicyRequest, PolicyState,
};
use kala_kernel::{ApprovalKind, KernelTimestamp, SideEffectLevel};
use num_bigint::BigInt;
use support::{array, int, object, text};

const NOW: &str = "2026-01-02T03:04:05Z";

fn at(value: &str) -> KernelTimestamp {
    KernelTimestamp::parse_isoformat(value).expect("timestamp")
}

fn request(
    request_id: &str,
    action_digest: &str,
    capabilities: &[&str],
    side_effect: SideEffectLevel,
    checkpoint: Option<ApprovalKind>,
) -> PolicyRequest {
    PolicyRequest::from_parts(
        Text::new(request_id),
        Text::new("state-1"),
        Text::new(action_digest),
        capabilities.iter().map(|value| Text::new(value)),
        side_effect,
        false,
        false,
        None,
        checkpoint,
    )
    .expect("request")
}

fn read_request(id: &str) -> PolicyRequest {
    request(id, "action-read", &["read"], SideEffectLevel::None, None)
}

fn egress_request(id: &str, action: &str) -> PolicyRequest {
    request(
        id,
        action,
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
    )
}

fn request_value(
    request_id: &str,
    action_digest: &str,
    capabilities: &[&str],
    side_effect: &str,
) -> CanonicalValue {
    object(vec![
        ("action_digest", text(action_digest)),
        ("checkpoint_type", CanonicalValue::Null),
        ("destructive", CanonicalValue::Bool(false)),
        ("egress", CanonicalValue::Bool(side_effect == "egress")),
        ("request_id", text(request_id)),
        (
            "requested_capabilities",
            array(capabilities.iter().map(|value| text(value)).collect()),
        ),
        ("scope", CanonicalValue::Null),
        ("side_effect_level", text(side_effect)),
        ("state_digest", text("state-1")),
    ])
}

fn pending_approval(request_id: &str, action_digest: &str) -> CanonicalValue {
    object(vec![
        ("request_id", text(request_id)),
        ("state_digest", text("state-1")),
        ("action_digest", text(action_digest)),
        ("status", text("pending")),
        ("actor_id", CanonicalValue::Null),
        ("decided_at", CanonicalValue::Null),
        ("reason", CanonicalValue::Null),
    ])
}

fn tool_class(action_digest: &str) -> CanonicalValue {
    object(vec![
        ("action_digest", text(action_digest)),
        ("capabilities", array(vec![text("read")])),
        ("scope", CanonicalValue::Null),
    ])
}

#[test]
fn tracking_and_tool_class_bounds_hold_under_churn() {
    const LIMIT: usize = 6;
    let engine = PolicyEngine::fixed(at(NOW))
        .with_grants([GrantSpec::from("read"), GrantSpec::from("network.egress")])
        .expect("grants")
        .with_max_tracked_requests(LIMIT.into())
        .expect("limit");
    let context = DecisionContext::new()
        .with_actor_id(Text::new("load"))
        .with_now(at(NOW));

    for index in 0..240u32 {
        let egress = egress_request(
            &format!("egress-{index}"),
            &format!("egress-action-{index}"),
        );
        engine
            .decide(&egress, Some(at(NOW)))
            .expect("egress decision");
        engine
            .request_approval(&egress, Some(at(NOW)))
            .expect("pending approval");
        if index % 3 == 0 {
            let _ = engine.approve(&egress, &context);
        }
        if index % 5 == 0 {
            let _ = engine.reject(&egress, &context);
        }

        let tool = request(
            &format!("tool-{index}"),
            &format!("tool-action-{index}"),
            &["read"],
            SideEffectLevel::None,
            Some(ApprovalKind::ToolClassChange),
        );
        engine.decide(&tool, Some(at(NOW))).expect("tool decide");
        if index % 2 == 0 {
            let _ = engine.approve(&tool, &context);
        }

        assert!(
            engine.tracked_request_ids().len() <= LIMIT,
            "tracked requests grew past {LIMIT} at {index}"
        );
        assert!(
            engine.approvals().len() <= LIMIT,
            "approvals grew past {LIMIT} at {index}"
        );
        assert!(
            engine.approved_tool_classes().len() <= LIMIT,
            "tool-class cache grew past {LIMIT} at {index}"
        );
    }

    let exported = engine.export_state();
    exported.validate().expect("validated export");
    let json = exported.to_json().expect("json");
    let restored = PolicyEngine::from_state_json(&json, Some(at(NOW)), None).expect("restore");
    assert_eq!(restored.export_state(), exported);
    assert_eq!(restored.max_tracked_requests(), LIMIT.into());
}

#[test]
fn oversized_imported_state_is_pruned_and_arbitrary_limits_survive() {
    let count = 1_200u32;
    let requests: Vec<CanonicalValue> = (0..count)
        .map(|index| {
            request_value(
                &format!("r-{index}"),
                &format!("a-{index}"),
                &["read"],
                "none",
            )
        })
        .collect();
    let tools: Vec<CanonicalValue> = (0..count)
        .map(|index| tool_class(&format!("a-{index}")))
        .collect();
    let state = object(vec![
        ("version", int(1)),
        ("grants", array(vec![])),
        ("requests", array(requests)),
        ("approvals", array(vec![])),
        ("tool_classes", array(tools)),
        ("max_tracked_requests", int(5)),
    ]);
    let engine = PolicyEngine::from_state(&state).expect("oversized import");
    assert_eq!(engine.max_tracked_requests(), 5.into());
    assert!(engine.tracked_request_ids().len() <= 5);
    assert!(engine.approvals().is_empty());
    // Source parity: Python's import prunes requests/approvals only; an
    // imported tool-class cache is not truncated until the next remember.
    assert_eq!(engine.approved_tool_classes().len(), count as usize);
    let exported = engine.export_state();
    exported.validate().expect("validated export");
    let json = exported.to_json().expect("json");
    let restored = PolicyEngine::from_state_json(&json, None, None).expect("restore");
    assert_eq!(restored.export_state(), exported);

    // A subsequent tool-class approval applies the cache bound and evicts the
    // oldest entries.
    engine.grant_capability(Text::new("read")).expect("grant");
    let tool_request = request(
        "trim",
        "new-tool-action",
        &["read"],
        SideEffectLevel::None,
        Some(ApprovalKind::ToolClassChange),
    );
    let context = DecisionContext::new()
        .with_actor_id(Text::new("reviewer"))
        .with_now(at(NOW));
    engine
        .decide(&tool_request, Some(at(NOW)))
        .expect("pending tool");
    let decision = engine
        .approve(&tool_request, &context)
        .expect("approve tool");
    assert!(decision.allowed());
    assert!(engine.approved_tool_classes().len() <= 5);
    assert!(
        !engine.approved_tool_classes().contains(&Text::new("a-0")),
        "the oldest tool-class entry should be evicted"
    );
    assert!(
        engine
            .approved_tool_classes()
            .contains(&Text::new("new-tool-action"))
    );

    // Requests *and* approvals are pruned to the imported limit.
    let approvals: Vec<CanonicalValue> = (0..50u32)
        .map(|index| pending_approval(&format!("p-{index}"), &format!("pa-{index}")))
        .collect();
    let pruned_requests: Vec<CanonicalValue> = (0..50u32)
        .map(|index| {
            request_value(
                &format!("p-{index}"),
                &format!("pa-{index}"),
                &["read"],
                "none",
            )
        })
        .collect();
    let pruned_state = object(vec![
        ("version", int(1)),
        ("grants", array(vec![])),
        ("requests", array(pruned_requests)),
        ("approvals", array(approvals)),
        ("tool_classes", array(vec![])),
        ("max_tracked_requests", int(5)),
    ]);
    let pruned = PolicyEngine::from_state(&pruned_state).expect("pruned import");
    assert!(pruned.tracked_request_ids().len() <= 5);
    assert!(pruned.approvals().len() <= 5);

    // Arbitrary-precision positive limits are accepted and do not prune.
    let huge = BigInt::from(10).pow(40);
    let small_requests: Vec<CanonicalValue> = (0..40)
        .map(|index| {
            request_value(
                &format!("h-{index}"),
                &format!("ha-{index}"),
                &["read"],
                "none",
            )
        })
        .collect();
    let huge_state = object(vec![
        ("version", int(1)),
        ("grants", array(vec![])),
        ("requests", array(small_requests)),
        ("approvals", array(vec![])),
        ("tool_classes", array(vec![])),
        (
            "max_tracked_requests",
            CanonicalValue::Integer(huge.clone()),
        ),
    ]);
    let huge_engine = PolicyEngine::from_state(&huge_state).expect("huge limit");
    assert_eq!(huge_engine.max_tracked_requests(), huge);
    assert_eq!(huge_engine.tracked_request_ids().len(), 40);

    // Oversized identifiers are rejected without touching the engine.
    let baseline = huge_engine.export_state();
    let giant = "x".repeat(1_000_000);
    let bad = object(vec![
        ("version", int(1)),
        (
            "requests",
            array(vec![request_value(&giant, "a", &["read"], "none")]),
        ),
    ]);
    assert!(huge_engine.import_state(&bad).is_err());
    assert_eq!(huge_engine.export_state(), baseline);
    assert!(PolicyState::parse(&bad, &BigInt::from(4)).is_err());
}

#[test]
fn concurrent_decisions_and_imports_never_tear_state() {
    let engine = Arc::new(
        PolicyEngine::fixed(at(NOW))
            .with_grants([GrantSpec::from("read")])
            .expect("grants")
            .with_max_tracked_requests(8.into())
            .expect("limit"),
    );

    // Phase 1: many threads decide the same read request. Python's outcome is
    // deterministic here: every decision is allowed.
    let barrier = Arc::new(Barrier::new(32));
    let failures = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..32 {
        let engine = engine.clone();
        let barrier = barrier.clone();
        let failures = failures.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..200 {
                let decision = engine
                    .decide(&read_request("shared-read"), None)
                    .expect("decision");
                if !decision.allowed() || decision.status() != PolicyDecisionStatus::Allowed {
                    failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(failures.load(Ordering::Relaxed), 0);

    // Phase 2: a writer alternates complete state documents while readers
    // export concurrently. Every export must be a complete, valid state.
    let empty = object(vec![("version", int(1))]);
    let granted = object(vec![
        ("version", int(1)),
        (
            "requests",
            array(vec![request_value("tracked", "action", &["read"], "none")]),
        ),
        (
            "grants",
            array(vec![object(vec![
                ("capability", text("read")),
                ("expires_at", CanonicalValue::Null),
                ("scope", CanonicalValue::Null),
            ])]),
        ),
    ]);
    let done = Arc::new(AtomicBool::new(false));
    let writer_engine = engine.clone();
    let writer_done = done.clone();
    let writer_empty = empty.clone();
    let writer_granted = granted.clone();
    let writer = thread::spawn(move || {
        for index in 0..300 {
            let state = if index % 2 == 0 {
                &writer_empty
            } else {
                &writer_granted
            };
            writer_engine.import_state(state).expect("import");
        }
        writer_done.store(true, Ordering::Release);
    });

    let validations = Arc::new(AtomicUsize::new(0));
    let mut readers = Vec::new();
    for _ in 0..4 {
        let engine = engine.clone();
        let done = done.clone();
        let validations = validations.clone();
        readers.push(thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let exported = engine.export_state();
                exported.validate().expect("torn export");
                exported.to_json().expect("torn serialization");
                validations.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    writer.join().expect("writer");
    for reader in readers {
        reader.join().expect("reader");
    }
    assert!(validations.load(Ordering::Relaxed) > 0);

    // Phase 3: concurrent approvals of one request. Exactly one transition
    // may win; the rest must report the record already resolved.
    let approval_engine = Arc::new(
        PolicyEngine::fixed(at(NOW))
            .with_grants([GrantSpec::from("network.egress")])
            .expect("grants"),
    );
    let request = egress_request("single", "single-action");
    approval_engine
        .decide(&request, Some(at(NOW)))
        .expect("pending");
    let successes = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(48));
    let mut handles = Vec::new();
    for _ in 0..48 {
        let engine = approval_engine.clone();
        let barrier = barrier.clone();
        let successes = successes.clone();
        let request = request.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            let context = DecisionContext::new()
                .with_actor_id(Text::new("racer"))
                .with_now(at(NOW));
            if engine.approve(&request, &context).is_ok() {
                successes.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(successes.load(Ordering::Relaxed), 1);
    let record = approval_engine
        .get_approval(
            &Text::new("single"),
            Some(&Text::new("state-1")),
            Some(&Text::new("single-action")),
        )
        .expect("lookup")
        .expect("record");
    assert_eq!(record.status(), kala_kernel::ApprovalStatus::Approved);
}

#[test]
fn deep_json_and_oversized_values_are_refused_without_panicking() {
    let shallow = format!("{}0{}", "[".repeat(480), "]".repeat(480));
    assert!(
        kala_kernel::parse_model_json(&shallow).is_ok(),
        "a 480-deep document should parse"
    );
    for depth in [600usize, 5_000, 50_000] {
        let deep = format!("{}0{}", "[".repeat(depth), "]".repeat(depth));
        assert!(
            kala_kernel::parse_model_json(&deep).is_err(),
            "a {depth}-deep document must be refused"
        );
        let engine = PolicyEngine::new();
        let baseline = engine.export_state();
        assert!(engine.import_state_json(&deep).is_err());
        assert_eq!(engine.export_state(), baseline);
    }
}

#[test]
fn imported_tool_class_cache_matches_python() {
    let tools: Vec<CanonicalValue> = (0..64)
        .map(|index| tool_class(&format!("a-{index}")))
        .collect();
    let state = object(vec![
        ("version", int(1)),
        ("grants", array(vec![])),
        ("requests", array(vec![])),
        ("approvals", array(vec![])),
        ("tool_classes", array(tools)),
        ("max_tracked_requests", int(5)),
    ]);
    let document = object(vec![
        ("now", text(NOW)),
        (
            "ops",
            array(vec![
                object(vec![("op", text("import_state")), ("state", state)]),
                object(vec![("op", text("approved_tool_classes"))]),
                object(vec![("op", text("export_state"))]),
            ]),
        ),
    ]);
    let oracle = support::run_oracle("policy", &document);
    let native = support::policy_native(&document);
    support::assert_exact_oracle(&native, &oracle);
}

#[test]
fn imported_state_cannot_forge_an_allowed_tool_class_without_grants() {
    // A trusted state document may contain tool-class cache entries; that is
    // source behaviour. It must still not bypass the capability grant check:
    // the decision is denied for the missing capability before the cache is
    // consulted, and a mismatched cached capability set never matches.
    let state = object(vec![
        ("version", int(1)),
        ("grants", array(vec![])),
        (
            "requests",
            array(vec![request_value(
                "tool",
                "tool-action",
                &["read"],
                "none",
            )]),
        ),
        ("approvals", array(vec![])),
        ("tool_classes", array(vec![tool_class("tool-action")])),
    ]);
    let engine = PolicyEngine::from_state(&state).expect("trusted state");
    let tool = request(
        "tool",
        "tool-action",
        &["read"],
        SideEffectLevel::None,
        Some(ApprovalKind::ToolClassChange),
    );
    let decision = engine.decide(&tool, Some(at(NOW))).expect("decision");
    assert_eq!(decision.status(), PolicyDecisionStatus::Denied);
    assert!(!decision.allowed());
}
