//! Public-surface tests for the composition root: declarations, the mechanical
//! reference loop, revisions, journal pages and the report artifact, driven
//! through the same methods the later api-routes and cli cells call.

use std::collections::BTreeMap;
use std::path::Path;

use kala_api::config::{SettingValue, Settings, settings_from_values};
use kala_application::{
    DomainBinding, KERNEL_STATE_NAMESPACE, KalaApplication, LOCAL_ACTOR_ID, artifact_digest,
    kernel_state_aggregate, mechanical_step_catalog,
};
use kala_contracts::ApprovalStatus;
use kala_domains::DomainName;
use kala_json::{CanonicalValue, JsonText};

fn settings_for(data_dir: &Path) -> Settings {
    let mut values = BTreeMap::new();
    values.insert(
        "data_dir".to_owned(),
        SettingValue::Text(data_dir.to_string_lossy().into_owned()),
    );
    settings_from_values(&values).expect("valid settings")
}

fn open(data_dir: &Path) -> KalaApplication {
    KalaApplication::open(settings_for(data_dir)).expect("open")
}

#[test]
fn kernel_state_names_and_artifact_digests() {
    assert_eq!(
        kernel_state_aggregate("policy").expect("name"),
        format!("{KERNEL_STATE_NAMESPACE}policy")
    );
    assert!(kernel_state_aggregate("").is_err());
    let payload = "{\"a\":1}";
    let expected = kala_tools::protocol::sha256_hex(format!("{payload}\n").as_bytes());
    assert_eq!(artifact_digest(payload), expected);
}

#[test]
fn the_mechanical_step_catalog_is_the_declared_one() {
    let catalog = mechanical_step_catalog().expect("catalog");
    let ids = catalog.step_ids();
    assert_eq!(
        ids,
        vec![
            "mechanical.geometry-check",
            "mechanical.material-check",
            "mechanical.load-check",
            "mechanical.constraint-evaluation",
        ]
    );
    for step_id in ids {
        let step = catalog.require(&step_id).expect("step");
        assert_eq!(step.tool_id(), "mechanical.reference-simulator");
        assert_eq!(step.tool_version(), Some("1.0.0"));
        assert!(step.intent().contains(':'));
        assert!(
            step.required_capabilities()
                .contains(&"tool.execute".to_owned())
        );
    }
}

#[test]
fn domain_declarations_cover_all_three_packages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let application = open(&temp.path().join("data"));
    assert_eq!(
        application.domains(),
        vec!["building", "mechanical", "wing"]
    );
    let packages = application.list_domain_packages();
    assert_eq!(packages.len(), 3);
    assert_eq!(
        packages
            .iter()
            .map(|package| package.package_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "building.storey-envelope",
            "mechanical.bracket",
            "wing.parametric-tapered"
        ]
    );
    for package in &packages {
        assert_eq!(package.digest.len(), 64);
        assert!(
            package
                .schema_digests
                .iter()
                .all(|digest| digest.len() == 64)
        );
        assert!(!package.disclaimer.is_empty());
        assert!(!package.catalog_step_ids.is_empty());
    }
    assert_eq!(application.list_workflow_definitions().len(), 3);
    let detail = application.describe_domain("mechanical").expect("detail");
    assert_eq!(detail.catalog_steps.len(), 4);
    assert_eq!(
        detail
            .catalog_steps
            .iter()
            .filter(|step| step.checkpoint_kind.is_some())
            .count(),
        1
    );
    assert!(application.describe_domain("absent").is_err());
    for binding in DomainBinding::defaults() {
        assert_eq!(
            binding.domain(),
            DomainName::parse(binding.domain().as_str()).expect("domain")
        );
        assert!(!binding.reference_step_ids().is_empty());
        assert!(!binding.subject_id().is_empty());
    }
    application.close().expect("close");
}

#[test]
fn the_full_mechanical_loop_reports_consistent_views() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = temp.path().join("data");
    let application = open(&data);

    let report = application
        .evaluate_domain_design("mechanical", None, Some("api-design"))
        .expect("evaluate");
    assert!(report.passed);
    assert_eq!(report.status.as_str(), "completed");
    assert_eq!(report.evaluations.len(), 4);
    assert_eq!(report.findings.len(), 5);
    assert_eq!(report.score.map(|score| score > 0.0), Some(true));

    let seeded = application
        .create_design_revision(
            "api-design",
            "mechanical",
            None,
            None,
            None,
            &[],
            &[],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        )
        .expect("seed");
    let artifact = &seeded.revision.artifacts()[0];
    let bytes = application
        .artifact_store()
        .get_bytes(&artifact.digest().to_hex())
        .expect("artifact bytes");
    assert_eq!(
        num_bigint::BigInt::from(bytes.len()),
        *artifact.size_bytes().as_bigint()
    );
    assert_eq!(
        kala_tools::protocol::sha256_hex(&bytes),
        artifact.digest().to_hex()
    );
    assert!(bytes.ends_with(b"\n"));
    let document = kala_persistence::parse_json_object(
        std::str::from_utf8(&bytes)
            .expect("utf8")
            .trim_end_matches('\n'),
    )
    .expect("report JSON");
    match &document {
        CanonicalValue::Object(map) => {
            assert_eq!(
                map.get(&JsonText::new("subject_id")),
                Some(&CanonicalValue::text("api-design"))
            );
            assert_eq!(
                map.get(&JsonText::new("passed")),
                Some(&CanonicalValue::Bool(true))
            );
            assert!(matches!(
                map.get(&JsonText::new("non_authoritative_finding")),
                Some(CanonicalValue::Text(value)) if value.to_utf8().expect("utf8").starts_with("Surrogate reference")
            ));
        }
        _ => panic!("report object"),
    }
    assert!(
        application
            .artifact_store()
            .contains(&artifact.digest().to_hex())
            .expect("contains")
    );

    let run = application
        .run_mechanical_reference_workflow(Some("workflow-api"), "api-design", "api-user")
        .expect("run");
    assert!(run.started);
    assert_eq!(run.status.as_str(), "waiting_for_approval");
    assert_eq!(run.evaluations.len(), 4);
    assert_eq!(run.planned_task_ids.len(), 4);
    let checkpoint = run.checkpoint.expect("checkpoint");
    assert_eq!(checkpoint.status, ApprovalStatus::Pending);
    assert_eq!(
        checkpoint.approval_id,
        "approval-workflow-api-mechanical.assess"
    );
    assert!(run.revision.is_none());

    let state = application
        .get_workflow_state("workflow-api")
        .expect("state");
    assert_eq!(state.completed_step_ids.len(), 3);
    assert_eq!(state.attempts.len(), 3);
    assert_eq!(state.history.len(), 3);
    assert!(state.pending_approval.is_some());

    let decided = application
        .approve_workflow_checkpoint(
            "workflow-api",
            "api-approver",
            Some(&checkpoint.state_digest),
            Some(&checkpoint.action_digest),
            None,
        )
        .expect("approve");
    assert_eq!(decided.status.as_str(), "completed");
    assert_eq!(decided.state.completed_step_ids.len(), 4);
    assert_eq!(decided.evaluations.len(), 4);
    let record = decided.revision.expect("revision");
    assert_eq!(record.revision.revision_id(), "rev-api-design-r1");
    assert_eq!(
        record.revision.parent_revision_id(),
        Some("rev-api-design-r0")
    );
    assert_eq!(
        record
            .provenance
            .transformation_ids()
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mechanical-reference-workflow",
            "reference-workflow-evaluation"
        ]
    );
    assert_eq!(record.artifact_digests.len(), 1);
    assert_eq!(
        record.artifact_digests[0],
        seeded.revision.artifacts()[0].digest().to_hex(),
        "the same report bytes are content-addressed once"
    );

    // Revisions are listed in sequence order and the limit is honoured.
    let listed = application
        .list_design_revisions("api-design", 1_000)
        .expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|record| record.revision.revision_id().to_owned())
            .collect::<Vec<_>>(),
        vec!["rev-api-design-r0", "rev-api-design-r1"]
    );
    assert_eq!(
        application
            .list_design_revisions("api-design", 1)
            .expect("limited")
            .len(),
        1
    );

    // Journal pages are stable and filterable.
    let first = application
        .list_events(0, 5, None, None, None)
        .expect("page 1");
    assert_eq!(first.count, 5);
    assert_eq!(first.after_sequence, 0);
    let second = application
        .list_events(first.next_after_sequence, 5, None, None, None)
        .expect("page 2");
    assert_eq!(second.after_sequence, first.next_after_sequence);
    assert!(second.events[0].sequence > first.events[4].sequence);
    let revision_events = application
        .list_events(0, 1_000, None, Some("revision.created"), None)
        .expect("filtered");
    assert_eq!(revision_events.count, 2);
    let workflow_events = application
        .list_events(0, 1_000, Some("workflow-api"), None, None)
        .expect("workflow events");
    assert!(workflow_events.count > 10);
    assert!(
        workflow_events
            .events
            .iter()
            .all(|event| event.workflow_id.as_deref() == Some("workflow-api"))
    );
    assert!(!first.payload_truncated);

    let verification = application.verify_chain().expect("verified");
    assert_eq!(
        verification.event_count as usize,
        application
            .list_events(0, 1_000, None, None, None)
            .expect("all")
            .count
    );
    application.close().expect("close");
}

#[test]
fn a_revision_with_explicit_provenance_inputs_is_stored_verbatim() {
    let temp = tempfile::tempdir().expect("tempdir");
    let application = open(&temp.path().join("data"));
    let record = application
        .create_design_revision(
            "provenance-design",
            "mechanical",
            Some("  explicit inputs  "),
            Some(&CanonicalValue::object([(
                JsonText::new("load_kn"),
                CanonicalValue::Float(4.5),
            )])),
            None,
            &["prov-external".to_owned()],
            &["zeta".to_owned(), "alpha".to_owned(), "zeta".to_owned()],
            None,
            LOCAL_ACTOR_ID,
            None,
            None,
        )
        .expect("create");
    assert_eq!(
        record.revision.spec().summary().as_str(),
        Some("explicit inputs")
    );
    assert_eq!(
        record
            .provenance
            .parent_provenance_ids()
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec!["prov-external"]
    );
    assert_eq!(
        record
            .provenance
            .transformation_ids()
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "reference-workflow-evaluation", "zeta"]
    );
    application.close().expect("close");
}
