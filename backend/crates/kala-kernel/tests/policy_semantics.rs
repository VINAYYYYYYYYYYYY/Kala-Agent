//! Native policy state-machine, security and concurrency tests.

use std::sync::{Arc, Barrier};
use std::thread;

use kala_contracts::{ApprovalKind, ApprovalStatus, SideEffectLevel, Text};
use kala_json::CanonicalValue;
use kala_kernel::KernelTimestamp;
use kala_kernel::policy::{
    CapabilityGrant, DecisionContext, GrantSpec, PolicyDecisionStatus, PolicyEngine, PolicyError,
    PolicyReason, PolicyRequest,
};
use num_bigint::BigInt;

const NOW: &str = "2026-01-02T03:04:05Z";
const LATER: &str = "2026-01-02T03:05:05Z";

fn at(value: &str) -> KernelTimestamp {
    KernelTimestamp::parse_isoformat(value).expect("timestamp")
}

fn request(
    request_id: &str,
    capabilities: &[&str],
    side_effect: SideEffectLevel,
    scope: Option<&str>,
    checkpoint: Option<ApprovalKind>,
) -> PolicyRequest {
    PolicyRequest::from_parts(
        Text::new(request_id),
        Text::new("state-1"),
        Text::new("action-1"),
        capabilities
            .iter()
            .map(|value| Text::new(value))
            .collect::<Vec<Text>>(),
        side_effect,
        false,
        false,
        scope.map(Text::new),
        checkpoint,
    )
    .expect("request")
}

fn egress(request_id: &str) -> PolicyRequest {
    request(
        request_id,
        &["network.egress"],
        SideEffectLevel::Egress,
        None,
        None,
    )
}

fn context(now: &str) -> DecisionContext {
    DecisionContext::new().with_now(at(now))
}

fn allowed(engine: &PolicyEngine, request: &PolicyRequest, now: &str) -> bool {
    engine
        .decide(request, Some(at(now)))
        .expect("decide")
        .allowed()
}

fn status(engine: &PolicyEngine, request: &PolicyRequest, now: &str) -> PolicyDecisionStatus {
    engine
        .decide(request, Some(at(now)))
        .expect("decide")
        .status()
}

#[test]
fn egress_and_network_are_denied_without_grants() {
    let engine = PolicyEngine::fixed(at(NOW));
    let decision = engine
        .decide(&egress("request-egress"), Some(at(NOW)))
        .expect("decide");
    assert!(!decision.allowed());
    assert_eq!(decision.status(), PolicyDecisionStatus::Denied);
    assert_eq!(decision.reason(), PolicyReason::CapabilityNotGranted);
    assert_eq!(
        decision
            .missing_capabilities()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>(),
        ["network.egress"]
    );
    assert!(decision.requires_approval());

    let network = request(
        "request-network",
        &["network"],
        SideEffectLevel::Network,
        None,
        None,
    );
    assert!(!allowed(&engine, &network, NOW));
    let read = request("request-read", &["read"], SideEffectLevel::None, None, None);
    assert!(!allowed(&engine, &read, NOW));
}

#[test]
fn egress_without_capabilities_is_never_approvable() {
    let engine = PolicyEngine::fixed(at(NOW));
    let bare = request("request-bare", &[], SideEffectLevel::Egress, None, None);
    assert!(!allowed(&engine, &bare, NOW));
    assert_eq!(
        engine
            .request_approval(&bare, Some(at(NOW)))
            .expect("pending")
            .status(),
        ApprovalStatus::Pending
    );
    assert!(
        !engine
            .approve(&bare, &context(NOW))
            .expect("decision")
            .allowed()
    );
}

#[test]
fn explicit_egress_approval_allows_and_is_repeatable() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-approved");
    assert_eq!(
        status(&engine, &request, NOW),
        PolicyDecisionStatus::Pending
    );
    assert_eq!(
        engine
            .request_approval(&request, Some(at(NOW)))
            .expect("pending")
            .status(),
        ApprovalStatus::Pending
    );
    let decision = engine
        .approve(
            &request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("approve");
    assert!(decision.allowed());
    assert_eq!(
        decision.actor_id().map(Text::to_string_lossy),
        Some("reviewer-1".to_owned())
    );
    assert_eq!(decision.decided_at(), Some(at(NOW)));
    assert!(allowed(&engine, &request, NOW));
}

#[test]
fn read_only_requests_are_allowed_within_the_grant() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("read")])
        .expect("engine")
        .with_now(at(NOW));
    let request = request("request-read", &["read"], SideEffectLevel::None, None, None);
    let decision = engine.decide(&request, Some(at(NOW))).expect("decide");
    assert!(decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::Allowed);
    assert!(!decision.requires_approval());
}

#[test]
fn expired_grants_deny_even_after_approval() {
    let engine = PolicyEngine::from_grants([GrantSpec::Grant(
        CapabilityGrant::from_parts(
            Text::new("network.egress"),
            None,
            Some(at("2026-01-02T03:04:35Z")),
        )
        .expect("grant"),
    )])
    .expect("engine")
    .with_now(at(NOW));
    let request = egress("request-expiring");
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    let later = engine.decide(&request, Some(at(LATER))).expect("decide");
    assert!(!later.allowed());
    assert_eq!(later.reason(), PolicyReason::CapabilityExpired);

    let boundary = PolicyEngine::from_grants([GrantSpec::Grant(
        CapabilityGrant::from_parts(Text::new("network.egress"), None, Some(at(NOW)))
            .expect("grant"),
    )])
    .expect("engine")
    .with_now(at(NOW));
    assert_eq!(
        boundary
            .decide(&egress("request-boundary"), Some(at(NOW)))
            .expect("decide")
            .reason(),
        PolicyReason::CapabilityExpired
    );
}

#[test]
fn stale_state_digest_invalidates_and_allows_reapproval() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-stale");
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    let changed = PolicyRequest::from_parts(
        request.request_id().clone(),
        Text::new("state-2"),
        request.action_digest().clone(),
        request.requested_capabilities().to_vec(),
        request.side_effect_level(),
        request.destructive(),
        request.egress(),
        request.scope().cloned(),
        request.checkpoint_type(),
    )
    .expect("changed");
    let invalidated = engine.decide(&changed, Some(at(NOW))).expect("decide");
    assert_eq!(invalidated.status(), PolicyDecisionStatus::Invalidated);
    assert_eq!(invalidated.reason(), PolicyReason::StaleApproval);
    let record = engine
        .get_approval(request.request_id(), None, None)
        .expect("lookup")
        .expect("record");
    assert_eq!(record.status(), ApprovalStatus::Invalidated);
    assert_eq!(record.state_digest(), request.state_digest());
    assert!(
        engine
            .approve(&changed, &context(NOW))
            .expect("approve")
            .allowed()
    );
    assert!(allowed(&engine, &changed, NOW));
}

#[test]
fn approval_with_mismatched_digests_is_invalidated() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-mismatched");
    engine
        .request_approval(&request, Some(at(NOW)))
        .expect("pending");
    let decision = engine
        .approve(
            &request,
            &DecisionContext::new()
                .with_state_digest(Text::new("state-other"))
                .with_now(at(NOW)),
        )
        .expect("approve");
    assert_eq!(decision.status(), PolicyDecisionStatus::Invalidated);
    assert_eq!(
        engine
            .get_approval(request.request_id(), None, None)
            .expect("lookup")
            .expect("record")
            .status(),
        ApprovalStatus::Invalidated
    );
}

#[test]
fn explicit_invalidation_requires_the_exact_pair() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-invalidated");
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    let decision = engine
        .invalidate(
            &request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("invalidate");
    assert_eq!(decision.status(), PolicyDecisionStatus::Invalidated);
    assert_eq!(
        decision.actor_id().map(Text::to_string_lossy),
        Some("reviewer-1".to_owned())
    );
    assert!(!allowed(&engine, &request, NOW));
    let error = engine
        .invalidate(
            &request,
            &DecisionContext::new()
                .with_state_digest(Text::new("state-other"))
                .with_now(at(NOW)),
        )
        .expect_err("mismatch");
    assert!(matches!(error, PolicyError::Transition(_)));
}

#[test]
fn rejection_and_abort_are_terminal() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let rejected_request = egress("request-rejected");
    let rejected = engine
        .reject(
            &rejected_request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("reject");
    assert_eq!(rejected.status(), PolicyDecisionStatus::Rejected);
    assert_eq!(
        status(&engine, &rejected_request, NOW),
        PolicyDecisionStatus::Rejected
    );
    assert!(matches!(
        engine.approve(&rejected_request, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));

    let aborted_request = egress("request-aborted");
    let aborted = engine
        .abort(
            &aborted_request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("abort");
    assert_eq!(aborted.status(), PolicyDecisionStatus::Aborted);
    assert!(matches!(
        engine.abort(&aborted_request, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));
}

#[test]
fn a_rejection_survives_invalidation_and_stale_digests() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-reject-then-invalidate");
    engine
        .reject(
            &request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("reject");
    assert_eq!(
        engine
            .invalidate(&request, &context(NOW))
            .expect("invalidate")
            .status(),
        PolicyDecisionStatus::Rejected
    );
    assert_eq!(
        engine
            .approve(
                &request,
                &DecisionContext::new()
                    .with_state_digest(Text::new("state-bogus"))
                    .with_now(at(NOW)),
            )
            .expect("approve")
            .status(),
        PolicyDecisionStatus::Rejected
    );
    assert_eq!(
        engine
            .request_approval(&request, Some(at(NOW)))
            .expect("pending")
            .status(),
        ApprovalStatus::Rejected
    );
    assert!(matches!(
        engine.approve(&request, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));
}

#[test]
fn eviction_forgets_approvals_before_rejections() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW))
        .with_max_tracked_requests(BigInt::from(2))
        .expect("limit");
    let rejected = egress("request-evicted-rejection");
    engine.reject(&rejected, &context(NOW)).expect("reject");
    for index in 0..3 {
        engine
            .approve(
                &egress(&format!("request-evicted-approved-{index}")),
                &context(NOW),
            )
            .expect("approve");
    }
    assert_eq!(
        status(&engine, &rejected, NOW),
        PolicyDecisionStatus::Rejected
    );
    assert!(matches!(
        engine.approve(&rejected, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));
}

#[test]
fn revoked_grants_deny_previously_allowed_requests() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-revoked");
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    assert!(engine.revoke(&Text::new("network.egress")).expect("revoke"));
    assert!(!engine.revoke(&Text::new("network.egress")).expect("revoke"));
    assert_eq!(
        engine
            .decide(&request, Some(at(NOW)))
            .expect("decide")
            .reason(),
        PolicyReason::CapabilityNotGranted
    );
}

#[test]
fn scoped_grants_deny_outside_scope_but_allow_inside() {
    let engine = PolicyEngine::from_grants([GrantSpec::Grant(
        CapabilityGrant::from_parts(Text::new("read"), Some(Text::new("project-1")), None)
            .expect("grant"),
    )])
    .expect("engine")
    .with_now(at(NOW));
    for (request_id, scope) in [
        ("request-unscoped", None),
        ("request-other-scope", Some("project-2")),
    ] {
        let request = request(request_id, &["read"], SideEffectLevel::None, scope, None);
        let decision = engine.decide(&request, Some(at(NOW))).expect("decide");
        assert!(!decision.allowed());
        assert_eq!(decision.reason(), PolicyReason::ScopeMismatch);
    }
    let matching = request(
        "request-matching-scope",
        &["read"],
        SideEffectLevel::None,
        Some("project-1"),
        None,
    );
    assert!(allowed(&engine, &matching, NOW));
}

#[test]
fn scoped_side_effects_require_approval_within_scope() {
    let engine = PolicyEngine::from_grants([GrantSpec::Grant(
        CapabilityGrant::from_parts(Text::new("local.write"), Some(Text::new("project-1")), None)
            .expect("grant"),
    )])
    .expect("engine")
    .with_now(at(NOW));
    let request = request(
        "request-scoped-write",
        &["local.write"],
        SideEffectLevel::LocalWrite,
        Some("project-1"),
        None,
    );
    assert_eq!(
        status(&engine, &request, NOW),
        PolicyDecisionStatus::Pending
    );
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    let unscoped = PolicyRequest::from_parts(
        Text::new("request-scoped-write-unscoped"),
        request.state_digest().clone(),
        request.action_digest().clone(),
        request.requested_capabilities().to_vec(),
        request.side_effect_level(),
        request.destructive(),
        request.egress(),
        None,
        request.checkpoint_type(),
    )
    .expect("request");
    assert!(!allowed(&engine, &unscoped, NOW));
}

#[test]
fn every_checkpoint_type_requires_approval() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("read")])
        .expect("engine")
        .with_now(at(NOW));
    for (index, checkpoint) in ApprovalKind::ALL.iter().copied().enumerate() {
        let request = request(
            &format!("request-checkpoint-{index}"),
            &["read"],
            SideEffectLevel::Read,
            None,
            Some(checkpoint),
        );
        assert!(request.requires_approval());
        assert_eq!(
            status(&engine, &request, NOW),
            PolicyDecisionStatus::Pending
        );
        assert!(
            engine
                .approve(&request, &context(NOW))
                .expect("approve")
                .allowed()
        );
    }
}

#[test]
fn tool_class_cache_is_exact_and_scoped() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("read"), GrantSpec::from("simulate")])
        .expect("engine")
        .with_now(at(NOW));
    let first = request(
        "request-tool-class-first",
        &["simulate"],
        SideEffectLevel::None,
        None,
        Some(ApprovalKind::ToolClassChange),
    );
    assert_eq!(status(&engine, &first, NOW), PolicyDecisionStatus::Pending);
    assert!(engine.approved_tool_classes().is_empty());
    assert!(
        engine
            .approve(&first, &context(NOW))
            .expect("approve")
            .allowed()
    );
    assert_eq!(engine.approved_tool_classes().len(), 1);

    let repeat = request(
        "request-tool-class-repeat",
        &["simulate"],
        SideEffectLevel::None,
        None,
        Some(ApprovalKind::ToolClassChange),
    );
    assert!(allowed(&engine, &repeat, NOW));

    let wider = request(
        "request-tool-class-wider",
        &["simulate", "read"],
        SideEffectLevel::None,
        None,
        Some(ApprovalKind::ToolClassChange),
    );
    assert_eq!(status(&engine, &wider, NOW), PolicyDecisionStatus::Pending);

    let renamed = PolicyRequest::from_parts(
        Text::new("request-tool-class-renamed"),
        Text::new("state-1"),
        Text::new("action-2"),
        vec![Text::new("simulate")],
        SideEffectLevel::None,
        false,
        false,
        None,
        Some(ApprovalKind::ToolClassChange),
    )
    .expect("request");
    assert_eq!(
        status(&engine, &renamed, NOW),
        PolicyDecisionStatus::Pending
    );

    assert!(
        engine
            .forget_tool_class(first.action_digest(), None)
            .expect("forget")
    );
    assert!(
        !engine
            .forget_tool_class(first.action_digest(), None)
            .expect("forget")
    );
    assert!(engine.approved_tool_classes().is_empty());
    assert_eq!(status(&engine, &repeat, NOW), PolicyDecisionStatus::Pending);

    let rebound = PolicyRequest::from_parts(
        first.request_id().clone(),
        first.state_digest().clone(),
        Text::new("action-2"),
        first.requested_capabilities().to_vec(),
        first.side_effect_level(),
        first.destructive(),
        first.egress(),
        first.scope().cloned(),
        first.checkpoint_type(),
    )
    .expect("request");
    assert_eq!(
        status(&engine, &rebound, NOW),
        PolicyDecisionStatus::Invalidated
    );
}

#[test]
fn an_approval_cannot_be_replayed_onto_a_different_action() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-replay");
    assert!(
        engine
            .approve(&request, &context(NOW))
            .expect("approve")
            .allowed()
    );
    let rebound = PolicyRequest::from_parts(
        request.request_id().clone(),
        request.state_digest().clone(),
        Text::new("action-2"),
        request.requested_capabilities().to_vec(),
        request.side_effect_level(),
        request.destructive(),
        request.egress(),
        request.scope().cloned(),
        request.checkpoint_type(),
    )
    .expect("request");
    let replayed = engine.approve(&rebound, &context(NOW)).expect("approve");
    assert_eq!(replayed.status(), PolicyDecisionStatus::Invalidated);
    assert!(!allowed(&engine, &request, NOW));
    assert!(matches!(
        engine.approve(&request, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));
    assert_eq!(
        status(&engine, &rebound, NOW),
        PolicyDecisionStatus::Pending
    );
    assert!(
        engine
            .approve(&rebound, &context(NOW))
            .expect("approve")
            .allowed()
    );
}

#[test]
fn policy_state_round_trip_restores_everything() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-persisted");
    engine
        .approve(
            &request,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_now(at(NOW)),
        )
        .expect("approve");
    let rejected = egress("request-persisted-rejected");
    engine
        .reject(
            &rejected,
            &DecisionContext::new()
                .with_actor_id(Text::new("reviewer-1"))
                .with_reason(Text::new("no"))
                .with_now(at(NOW)),
        )
        .expect("reject");
    let state = engine.export_state();
    let json = state.to_json().expect("json");
    let restored = PolicyEngine::from_state_json(&json, Some(at(NOW)), None).expect("restore");
    assert_eq!(restored.grants(), engine.grants());
    assert!(allowed(&restored, &request, NOW));
    assert_eq!(
        status(&restored, &rejected, NOW),
        PolicyDecisionStatus::Rejected
    );
    assert_eq!(
        restored
            .get_approval(request.request_id(), None, None)
            .expect("lookup"),
        engine
            .get_approval(request.request_id(), None, None)
            .expect("lookup")
    );
    assert!(matches!(
        restored.approve(&request, &context(NOW)),
        Err(PolicyError::Transition(_))
    ));
}

#[test]
fn import_is_transactional_and_strict_about_dynamic_types() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("read")])
        .expect("engine")
        .with_now(at(NOW));
    let before = engine.export_state();
    let mut valid = before.clone().as_value();
    // Unknown key.
    if let CanonicalValue::Object(ref mut object) = valid {
        object.insert(
            kala_json::JsonText::new("unexpected"),
            CanonicalValue::Bool(true),
        );
    }
    assert!(engine.import_state(&valid).is_err());
    assert_eq!(engine.export_state(), before);

    // Bool is not an integer, and an integer is not a bool.
    let mut invalid_version = before.clone().as_value();
    if let CanonicalValue::Object(ref mut object) = invalid_version {
        object.insert(
            kala_json::JsonText::new("version"),
            CanonicalValue::Bool(true),
        );
    }
    assert!(engine.import_state(&invalid_version).is_err());
    assert_eq!(engine.export_state(), before);
}

#[test]
fn get_approval_requires_both_digests_or_neither() {
    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW));
    let request = egress("request-lookup");
    engine
        .request_approval(&request, Some(at(NOW)))
        .expect("pending");
    assert!(
        engine
            .get_approval(request.request_id(), Some(request.state_digest()), None)
            .is_err()
    );
    assert!(
        engine
            .get_approval(
                request.request_id(),
                Some(request.state_digest()),
                Some(request.action_digest())
            )
            .expect("lookup")
            .is_some()
    );
}

#[test]
fn configured_clock_is_used_when_now_is_omitted() {
    let engine = PolicyEngine::fixed(at(NOW));
    assert_eq!(engine.clock(), Some(at(NOW)));
    let decision = engine
        .decide(&egress("request-clock"), None)
        .expect("decide");
    assert_eq!(decision.status(), PolicyDecisionStatus::Denied);
}

#[test]
fn concurrent_decisions_and_approvals_are_atomic() {
    let engine = Arc::new(
        PolicyEngine::from_grants([GrantSpec::from("network.egress")])
            .expect("engine")
            .with_now(at(NOW)),
    );
    let barrier = Arc::new(Barrier::new(32));
    let mut handles = Vec::new();
    for index in 0..32 {
        let engine = engine.clone();
        let barrier = barrier.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            let request = egress(&format!("request-concurrent-{index}"));
            assert!(
                engine
                    .approve(&request, &context(NOW))
                    .expect("approve")
                    .allowed()
            );
            assert!(allowed(&engine, &request, NOW));
            request
        }));
    }
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(engine.approved_tool_classes().len(), 0);
    assert_eq!(engine.tracked_request_ids().len(), 32);
}

#[test]
fn concurrent_replay_of_one_approval_never_widens_privilege() {
    let engine = Arc::new(
        PolicyEngine::from_grants([GrantSpec::from("network.egress")])
            .expect("engine")
            .with_now(at(NOW)),
    );
    let request = egress("request-concurrent-replay");
    engine
        .request_approval(&request, Some(at(NOW)))
        .expect("pending");
    let barrier = Arc::new(Barrier::new(16));
    let mut handles = Vec::new();
    for _ in 0..16 {
        let engine = engine.clone();
        let request = request.clone();
        let barrier = barrier.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            engine.approve(&request, &context(NOW))
        }));
    }
    let mut allowed_count = 0;
    for handle in handles {
        match handle.join().expect("thread") {
            Ok(decision) if decision.allowed() => allowed_count += 1,
            Ok(_) => {}
            Err(PolicyError::Transition(_)) => {}
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
    assert_eq!(allowed_count, 1);
    assert!(allowed(&engine, &request, NOW));
    assert_eq!(
        engine
            .get_approval(request.request_id(), None, None)
            .expect("lookup")
            .expect("record")
            .status(),
        ApprovalStatus::Approved
    );
}

#[test]
fn identifiers_use_python_strip_and_lone_surrogates_are_lossless() {
    let padded = Text::new(" \u{1c}read\u{1f} ");
    let grant = CapabilityGrant::new(padded).expect("padded grant");
    assert_eq!(grant.capability().to_string_lossy(), "read");
    let interior = Text::new("re\u{1c}ad");
    assert!(CapabilityGrant::new(interior).is_err());

    let surrogate = Text::from_code_points(vec![u32::from(b'r'), 0xD800]).expect("surrogate");
    let request = PolicyRequest::from_parts(
        surrogate.clone(),
        Text::new("s"),
        Text::new("a"),
        Vec::new(),
        SideEffectLevel::None,
        false,
        false,
        None,
        None,
    )
    .expect("surrogate request");
    assert_eq!(request.request_id().code_points(), surrogate.code_points());
    let engine = PolicyEngine::fixed(at(NOW));
    let state = engine.export_state();
    state.to_json().expect("state serializes");
}

#[test]
fn bounded_maps_evict_in_the_source_tier_order() {
    let limit = 4;
    let engine = PolicyEngine::fixed(at(NOW))
        .with_max_tracked_requests(BigInt::from(limit))
        .expect("limit");
    let read = request("read", &["read"], SideEffectLevel::None, None, None);
    for index in 0..20 {
        let mut value = read.clone();
        value = PolicyRequest::from_parts(
            Text::new(&format!("request-bounded-{index}")),
            value.state_digest().clone(),
            value.action_digest().clone(),
            value.requested_capabilities().to_vec(),
            value.side_effect_level(),
            value.destructive(),
            value.egress(),
            value.scope().cloned(),
            value.checkpoint_type(),
        )
        .expect("request");
        engine.decide(&value, Some(at(NOW))).expect("decide");
    }
    let tracked = engine.tracked_request_ids();
    assert_eq!(tracked.len(), limit);
    assert_eq!(
        tracked
            .iter()
            .map(Text::to_string_lossy)
            .collect::<Vec<_>>(),
        [
            "request-bounded-16",
            "request-bounded-17",
            "request-bounded-18",
            "request-bounded-19"
        ]
    );

    let engine = PolicyEngine::fixed(at(NOW))
        .with_max_tracked_requests(BigInt::from(limit))
        .expect("limit");
    for index in 0..12 {
        engine
            .request_approval(
                &egress(&format!("request-bounded-egress-{index}")),
                Some(at(NOW)),
            )
            .expect("pending");
    }
    assert!(engine.approvals().len() <= limit);
    assert!(engine.tracked_request_ids().len() <= limit);

    let engine = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW))
        .with_max_tracked_requests(BigInt::from(limit))
        .expect("limit");
    for index in 0..12 {
        assert!(
            engine
                .approve(&egress(&format!("request-resolved-{index}")), &context(NOW))
                .expect("approve")
                .allowed()
        );
    }
    assert!(engine.approvals().len() <= limit);
    assert!(engine.tracked_request_ids().len() <= limit);

    let classes = PolicyEngine::from_grants([GrantSpec::from("network.egress")])
        .expect("engine")
        .with_now(at(NOW))
        .with_max_tracked_requests(BigInt::from(limit))
        .expect("limit");
    for index in 0..12 {
        let request = PolicyRequest::from_parts(
            Text::new(&format!("request-class-{index}")),
            Text::new("state-1"),
            Text::new(&format!("action-{index}")),
            vec![Text::new("network.egress")],
            SideEffectLevel::Egress,
            false,
            false,
            None,
            Some(ApprovalKind::ToolClassChange),
        )
        .expect("request");
        assert!(
            classes
                .approve(&request, &context(NOW))
                .expect("approve")
                .allowed()
        );
    }
    assert!(classes.approved_tool_classes().len() <= limit);
    assert!(classes.approvals().len() <= limit);
}
