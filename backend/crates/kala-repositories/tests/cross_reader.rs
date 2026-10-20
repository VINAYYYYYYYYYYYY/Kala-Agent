//! Live Python -> Rust and Rust -> Python repository cross-readers.
//!
//! Python is the product oracle only. The native library never invokes it.

mod support;

use kala_repositories::{
    ApprovalRequestRepository, CanonicalJsonObject, ContractModel, DesignRevisionRepository,
    DesignSpecRepository, MemoryGetOptions, MemoryListOptions, MemoryRepository,
    ToolCallRepository, ToolResultRepository,
};
use serde_json::Value;
use support::{model, run_oracle, timestamp};
use tempfile::TempDir;

fn raw<'a>(payload: &'a Value, table: &str) -> &'a Vec<Value> {
    payload["rows"][table]
        .as_array()
        .unwrap_or_else(|| panic!("rows.{table}"))
}

fn wire<'a>(payload: &'a Value, kind: &str) -> Vec<&'a str> {
    payload["wire"][kind]
        .as_array()
        .unwrap_or_else(|| panic!("wire.{kind}"))
        .iter()
        .map(|value| value.as_str().expect("wire string"))
        .collect()
}

fn raw_model<'a>(payload: &'a Value, table: &str, key: &str, id: &str) -> &'a str {
    raw(payload, table)
        .iter()
        .find(|row| row[key].as_str() == Some(id))
        .unwrap_or_else(|| panic!("{table} {id}"))["model_json"]
        .as_str()
        .expect("model_json")
}

#[test]
fn python_seed_is_read_and_reserialized_byte_identically() {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("python-seed.sqlite3");
    let payload = run_oracle(&["seed", path.to_str().expect("path")]);
    let database = kala_repositories::Database::open(&path).expect("open");

    let specs = DesignSpecRepository::new(&database);
    let spec = specs
        .get("spec-python")
        .expect("spec")
        .expect("python spec");
    assert_eq!(
        spec.to_wire_json().expect("wire"),
        raw_model(&payload, "design_specs", "id", "spec-python")
    );
    assert_eq!(spec.name().to_utf8().expect("name"), "Café ☕ design");
    assert_eq!(
        spec.attributes_json().as_str(),
        r#"{"a":{"nested":[1,true,null,"\u03ba"]},"z":2}"#
    );

    let revisions = DesignRevisionRepository::new(&database);
    let listed = revisions
        .list_for_design("design-python")
        .expect("revisions");
    assert_eq!(listed.len(), 2);
    for revision in &listed {
        assert_eq!(
            revision.to_wire_json().expect("wire"),
            raw_model(
                &payload,
                "design_revisions",
                "revision_id",
                revision.revision_id()
            )
        );
    }
    assert_eq!(listed[0].sequence().to_u64(), Some(0));
    assert_eq!(listed[1].sequence().to_u64(), Some(1));
    assert_eq!(
        revisions
            .latest_for_design("design-python")
            .expect("latest")
            .expect("row")
            .revision_id(),
        "revision-python-1"
    );
    assert_eq!(
        listed[0].created_at().to_wire_string(),
        "2026-01-02T03:04:05.123456Z"
    );
    let raw_created = raw(&payload, "design_revisions")
        .iter()
        .map(|row| row["created_at"].as_str().expect("created"))
        .collect::<Vec<_>>();
    assert_eq!(
        raw_created,
        vec!["2026-01-02T03:04:05.123456Z", "2026-01-02T03:05:05.123456Z"]
    );

    let approvals = ApprovalRequestRepository::new(&database);
    let stored = approvals
        .get("approval-python")
        .expect("approval")
        .expect("row");
    assert_eq!(stored.status(), kala_repositories::ApprovalStatus::Approved);
    assert_eq!(
        stored.to_wire_json().expect("wire"),
        raw_model(
            &payload,
            "approval_requests",
            "request_id",
            "approval-python"
        )
    );
    assert_eq!(
        raw(&payload, "approval_requests")[0]["created_at"]
            .as_str()
            .expect("created"),
        "2026-01-02T03:04:05.000000Z"
    );

    let calls = ToolCallRepository::new(&database);
    let call = calls
        .list_for_tool("tool-python")
        .expect("calls")
        .into_iter()
        .next()
        .expect("call");
    assert_eq!(
        call.to_wire_json().expect("wire"),
        raw_model(&payload, "tool_calls", "call_id", "call-python")
    );
    assert_eq!(
        raw(&payload, "tool_calls")[0]["issued_at"]
            .as_str()
            .expect("issued"),
        "2026-01-02T03:04:05.000000Z"
    );
    let result = ToolResultRepository::new(&database)
        .get("call-python")
        .expect("result")
        .expect("row");
    assert_eq!(
        result.to_wire_json().expect("wire"),
        raw_model(&payload, "tool_results", "call_id", "call-python")
    );

    let memory = MemoryRepository::new(&database);
    let records = memory
        .list_with_options(
            &MemoryListOptions::default()
                .with_now(timestamp("2026-01-02T03:04:05Z"))
                .with_include_expired(true),
        )
        .expect("memory");
    assert_eq!(records.len(), 3);
    for (record, expected) in records.iter().zip(wire(&payload, "memories")) {
        assert_eq!(record.to_wire_json().expect("wire"), expected);
    }
    assert_eq!(
        records
            .iter()
            .map(|record| record.memory_id())
            .collect::<Vec<_>>(),
        vec![
            "memory-python-expired",
            "memory-python-active",
            "memory-python-offset"
        ]
    );

    let expired = memory
        .get_with_options(
            "memory-python-expired",
            &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:04:05Z")),
        )
        .expect("expired");
    assert_eq!(expired, None);
    let expired = memory
        .get_with_options(
            "memory-python-expired",
            &MemoryGetOptions::default()
                .with_now(timestamp("2026-01-02T03:04:05Z"))
                .with_include_expired(true),
        )
        .expect("expired")
        .expect("row");
    assert_eq!(
        expired.to_wire_json().expect("wire"),
        raw_model(
            &payload,
            "memory_entries",
            "memory_id",
            "memory-python-expired"
        )
    );

    let offset = memory
        .get_with_options(
            "memory-python-offset",
            &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:04:05Z")),
        )
        .expect("offset")
        .expect("row");
    assert_eq!(
        offset.created_at().to_wire_string(),
        "2026-01-02T08:34:05+05:30"
    );
    assert_eq!(
        offset.expires_at().expect("expiry").to_wire_string(),
        "2026-01-02T09:34:05+05:30"
    );
    let row = raw(&payload, "memory_entries")
        .iter()
        .find(|row| row["memory_id"].as_str() == Some("memory-python-offset"))
        .expect("offset row");
    assert_eq!(
        row["created_at"].as_str(),
        Some("2026-01-02T03:04:05.000000Z")
    );
    assert_eq!(
        row["expires_at"].as_str(),
        Some("2026-01-02T04:04:05.000000Z")
    );

    assert_eq!(wire(&payload, "active_memories").len(), 2);
}

#[test]
fn rust_writes_are_read_and_accepted_by_the_python_repositories() {
    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("rust-seed.sqlite3");
    let database = kala_repositories::Database::open(&path).expect("open");

    let spec: kala_repositories::DesignSpec = model(
        r#"{"id":"spec-rust","name":"Rust ☕ design","summary":"A \"native\" summary.","parameter_schema_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","attributes_json":"{\"z\":2,\"a\":{\"nested\":[1,true,null,\"κ\"]}}","requirement_ids":["requirement-2","requirement-1"],"constraints":["Preserve data."]}"#,
    );
    let revision: kala_repositories::DesignRevision = model(&format!(
        r#"{{"revision_id":"revision-rust-0","design_id":"design-rust","sequence":0,"spec":{spec_json},"parent_revision_id":null,"artifacts":[],"domain_packages":[],"provenance_id":"provenance-rust","created_at":"2026-01-02T08:34:05.123456+05:30"}}"#,
        spec_json = spec.to_wire_json().expect("spec"),
    ));
    let approval_request: kala_repositories::ApprovalRequest = model(
        r#"{"request_id":"approval-rust","checkpoint_type":"egress","status":"approved","workflow_id":"workflow-rust","step_id":null,"action_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","options":["approve","deny"],"selected_option_id":"approve","requested_capabilities":[],"artifact_ids":[],"created_at":"2026-01-02T03:04:05Z","decided_at":"2026-01-02T03:05:05Z","actor_id":"reviewer-rust","modification_digest":null}"#,
    );
    let call: kala_repositories::ToolCall = model(
        r#"{"call_id":"call-rust","tool_id":"tool-rust","tool_version":"1.0.0","arguments_json":"{\"value\":2}","requested_capabilities":["compute.local"],"side_effect_level":"read","timeout_seconds":30.0,"issued_at":"2026-01-02T09:34:05+06:30"}"#,
    );
    let result: kala_repositories::ToolResult = model(
        r#"{"call_id":"call-rust","tool_id":"tool-rust","status":"succeeded","result_json":"{\"value\":4}","error_code":null,"error_message":null,"artifacts":[],"started_at":"2026-01-02T03:04:05Z","finished_at":"2026-01-02T03:04:05.025000Z","duration_ms":25.0}"#,
    );
    let metadata = CanonicalJsonObject::parse(
        r#"{"priority":"high","labels":["local","db"],"nested":{"κ":"☕"}}"#,
    )
    .expect("metadata");
    let active = kala_repositories::MemoryRecord::new(
        "memory-rust-active",
        "Mémoire ☕",
        "conversation-rust",
        Some("provenance-rust"),
        metadata,
        timestamp("2026-01-02T03:04:05Z"),
        Some(timestamp("2026-01-02T04:04:05Z")),
    )
    .expect("memory");
    let offset = kala_repositories::MemoryRecord::new(
        "memory-rust-offset",
        "offset",
        "conversation-rust",
        None,
        CanonicalJsonObject::empty(),
        timestamp("2026-01-02T08:34:05+05:30"),
        Some(timestamp("2026-01-02T09:34:05+05:30")),
    )
    .expect("offset");
    let expired = kala_repositories::MemoryRecord::new(
        "memory-rust-expired",
        "expired",
        "conversation-rust",
        None,
        CanonicalJsonObject::empty(),
        timestamp("2026-01-02T01:04:05Z"),
        Some(timestamp("2026-01-02T02:04:05Z")),
    )
    .expect("expired");

    DesignSpecRepository::new(&database)
        .save(&spec)
        .expect("spec");
    DesignRevisionRepository::new(&database)
        .commit_revision(&revision, None)
        .expect("revision");
    ApprovalRequestRepository::new(&database)
        .save(&approval_request)
        .expect("approval");
    ToolCallRepository::new(&database)
        .save(&call)
        .expect("call");
    ToolResultRepository::new(&database)
        .save(&result)
        .expect("result");
    let memory = MemoryRepository::new(&database);
    memory.put(&active).expect("active");
    memory.put(&offset).expect("offset");
    memory.put(&expired).expect("expired");

    let payload = run_oracle(&["inspect", path.to_str().expect("path")]);
    assert_eq!(payload["raw_round_trip"], Value::Bool(true));
    assert_eq!(payload["raw_round_trip_count"], Value::from(8));

    assert_eq!(
        wire(&payload, "specs"),
        vec![spec.to_wire_json().expect("wire").as_str()]
    );
    assert_eq!(
        wire(&payload, "revisions"),
        vec![revision.to_wire_json().expect("wire").as_str()]
    );
    assert_eq!(
        wire(&payload, "approvals"),
        vec![approval_request.to_wire_json().expect("wire").as_str()]
    );
    assert_eq!(
        wire(&payload, "calls"),
        vec![call.to_wire_json().expect("wire").as_str()]
    );
    assert_eq!(
        wire(&payload, "results"),
        vec![result.to_wire_json().expect("wire").as_str()]
    );
    let expected_memory = [
        expired.to_wire_json().expect("wire"),
        active.to_wire_json().expect("wire"),
        offset.to_wire_json().expect("wire"),
    ];
    assert_eq!(
        wire(&payload, "memories"),
        expected_memory
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        wire(&payload, "active_memories"),
        vec![
            active.to_wire_json().expect("wire").as_str(),
            offset.to_wire_json().expect("wire").as_str()
        ]
    );

    assert_eq!(
        raw_model(
            &payload,
            "design_revisions",
            "revision_id",
            "revision-rust-0"
        ),
        revision.to_wire_json().expect("wire")
    );
    assert_eq!(
        raw(&payload, "design_revisions")[0]["created_at"].as_str(),
        Some("2026-01-02T03:04:05.123456Z")
    );
    assert_eq!(
        raw(&payload, "tool_calls")[0]["issued_at"].as_str(),
        Some("2026-01-02T03:04:05.000000Z")
    );
    let memory_rows = raw(&payload, "memory_entries");
    let offset_row = memory_rows
        .iter()
        .find(|row| row["memory_id"].as_str() == Some("memory-rust-offset"))
        .expect("offset row");
    assert_eq!(
        offset_row["created_at"].as_str(),
        Some("2026-01-02T03:04:05.000000Z")
    );
    assert_eq!(
        offset_row["expires_at"].as_str(),
        Some("2026-01-02T04:04:05.000000Z")
    );
    assert_eq!(
        specs_after(&database, "spec-rust")
            .to_wire_json()
            .expect("wire"),
        spec.to_wire_json().expect("wire")
    );
}

fn specs_after(database: &kala_repositories::Database, id: &str) -> kala_repositories::DesignSpec {
    DesignSpecRepository::new(database)
        .get(id)
        .expect("get")
        .expect("row")
}

#[test]
fn deep_and_large_memory_metadata_cross_reads_in_both_directions() {
    let directory = TempDir::new().expect("temp dir");

    // Python writes metadata beyond both the 48-depth and 1 MiB tool bounds.
    let python_path = directory.path().join("python-large.sqlite3");
    let payload = run_oracle(&["seed-large", python_path.to_str().expect("path")]);
    let python_database = kala_repositories::Database::open(&python_path).expect("open");
    let python_memory = MemoryRepository::new(&python_database)
        .get_with_options(
            "memory-python-large",
            &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:04:05Z")),
        )
        .expect("memory")
        .expect("row");
    let raw_large = raw_model(
        &payload,
        "memory_entries",
        "memory_id",
        "memory-python-large",
    );
    assert!(raw_large.len() > 1_048_576);
    assert_eq!(python_memory.to_wire_json().expect("wire"), raw_large);
    assert!(python_memory.metadata_json().as_str().len() > 1_048_576);
    assert!(
        python_memory
            .metadata_json()
            .as_str()
            .contains(&"7".repeat(1_000))
    );

    // Rust writes the same domain and Python reads it back byte-identically.
    let rust_path = directory.path().join("rust-large.sqlite3");
    let rust_database = kala_repositories::Database::open(&rust_path).expect("open");
    let mut deep = "0".to_owned();
    for _ in 0..80 {
        deep = format!("[{deep}]");
    }
    let large_metadata = CanonicalJsonObject::parse(&format!(
        r#"{{"big":{},"deep":{deep},"text":"{}"}}"#,
        "7".repeat(1_000),
        "a".repeat(1_100_000),
    ))
    .expect("large metadata");
    let rust_memory = kala_repositories::MemoryRecord::new(
        "memory-rust-large",
        "large",
        "conversation-rust",
        None,
        large_metadata,
        timestamp("2026-01-02T03:04:05Z"),
        Some(timestamp("2026-01-02T04:04:05Z")),
    )
    .expect("memory");
    MemoryRepository::new(&rust_database)
        .put(&rust_memory)
        .expect("put");
    let inspected = run_oracle(&["inspect", rust_path.to_str().expect("path")]);
    assert_eq!(inspected["raw_round_trip"], Value::Bool(true));
    assert_eq!(inspected["raw_round_trip_count"], Value::from(1));
    assert_eq!(
        wire(&inspected, "memories"),
        vec![rust_memory.to_wire_json().expect("wire").as_str()]
    );
    assert_eq!(
        raw_model(
            &inspected,
            "memory_entries",
            "memory_id",
            "memory-rust-large"
        ),
        rust_memory.to_wire_json().expect("wire")
    );
    let rust_read = MemoryRepository::new(&rust_database)
        .get_with_options(
            "memory-rust-large",
            &MemoryGetOptions::default().with_now(timestamp("2026-01-02T03:04:05Z")),
        )
        .expect("read")
        .expect("row");
    assert_eq!(rust_read, rust_memory);
    assert!(rust_read.metadata_json().as_str().len() > 1_048_576);
}
