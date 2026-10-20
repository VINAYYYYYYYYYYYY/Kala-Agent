//! The security tests of the reference simulator.
//!
//! The simulator publishes a file inside the workspace of a call, so the
//! publication has to be confined, atomic and free of a symlink that was
//! planted before the call. The misbehaviour entry points are the ones the
//! executor's security tests use to prove that a hostile tool is contained.

mod support;

use kala_contracts::ArtifactClassification;
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::ToolHandler;
use kala_tools::simulator::{
    MISBEHAVIOR_ESCAPE_NAME, MISBEHAVIOR_STRAY_CHILD_NAME, SimulatorParameters,
    SimulatorToolHandler, compute_metrics, escape_invoke, write_report, write_workspace_file,
};

use support::{request, symlink_directory};

/// The parameters of the tests.
fn parameters() -> SimulatorParameters {
    SimulatorParameters::new("beam_volume", 120.0, 20.0, 10.0, 2_700.0, false)
}

#[test]
fn the_report_stays_inside_the_workspace() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (artifact, relative) = write_report(
        workspace.path(),
        &parameters(),
        &compute_metrics(&parameters()),
        "call-1",
    )
    .expect("report");
    assert_eq!(relative, "outputs/reference-report.json");
    let uri = artifact.uri().as_text().to_string_lossy();
    let root = workspace.path().canonicalize().expect("root");
    let target = root.join("outputs").join("reference-report.json");
    assert_eq!(uri, format!("file://{}", target.to_string_lossy()));
    assert!(target.is_file());
    assert!(target.starts_with(&root));
}

#[test]
fn the_report_directory_is_not_a_symlink_after_a_write() {
    let workspace = tempfile::tempdir().expect("workspace");
    write_report(
        workspace.path(),
        &parameters(),
        &compute_metrics(&parameters()),
        "call-1",
    )
    .expect("report");
    let outputs = workspace.path().join("outputs");
    let metadata = std::fs::symlink_metadata(&outputs).expect("metadata");
    assert!(!metadata.file_type().is_symlink());
    assert!(metadata.is_dir());
}

#[test]
fn a_write_leaves_no_temporary_file_behind() {
    let workspace = tempfile::tempdir().expect("workspace");
    write_report(
        workspace.path(),
        &parameters(),
        &compute_metrics(&parameters()),
        "call-1",
    )
    .expect("report");
    let outputs = workspace.path().join("outputs");
    let leftovers: Vec<String> = std::fs::read_dir(&outputs)
        .expect("directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_write_publishes_the_whole_payload() {
    let workspace = tempfile::tempdir().expect("workspace");
    let target = workspace.path().join("outputs").join("report.json");
    write_workspace_file(&target, b"first").expect("written");
    assert_eq!(std::fs::read(&target).expect("target"), b"first");
    write_workspace_file(&target, b"second").expect("written");
    assert_eq!(std::fs::read(&target).expect("target"), b"second");
    let leftovers: Vec<String> = std::fs::read_dir(workspace.path().join("outputs"))
        .expect("directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_write_refuses_a_symlinked_directory() {
    let workspace = tempfile::tempdir().expect("workspace");
    let real = workspace.path().join("real");
    std::fs::create_dir(&real).expect("directory");
    let linked = workspace.path().join("linked");
    symlink_directory(&real, &linked).expect("symlink");
    let error = write_workspace_file(&linked.join("report.json"), b"{}").expect_err("refused");
    assert!(error.to_string().contains("must be a real directory"));
    // Nothing was written through the link.
    assert!(!real.join("report.json").exists());
}

#[test]
fn a_write_refuses_a_directory_that_cannot_be_created() {
    let workspace = tempfile::tempdir().expect("workspace");
    let blocker = workspace.path().join("blocked");
    std::fs::write(&blocker, "x").expect("blocker");
    let error = write_workspace_file(&blocker.join("nested").join("report.json"), b"{}")
        .expect_err("refused");
    assert!(error.to_string().contains("could not be created"));
}

#[test]
fn the_artifact_classification_is_text() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (artifact, _) = write_report(
        workspace.path(),
        &parameters(),
        &compute_metrics(&parameters()),
        "call-1",
    )
    .expect("report");
    assert_eq!(artifact.classification(), ArtifactClassification::Text);
    assert_eq!(artifact.media_type(), "application/json");
    let bytes = std::fs::read(
        workspace
            .path()
            .join("outputs")
            .join("reference-report.json"),
    )
    .expect("report");
    assert_eq!(
        artifact.digest().to_hex(),
        kala_tools::protocol::sha256_hex(&bytes)
    );
    assert_eq!(
        artifact.size_bytes().as_bigint().to_string(),
        bytes.len().to_string()
    );
}

#[test]
fn the_escape_writes_outside_the_workspace() {
    let outer = tempfile::tempdir().expect("workspace");
    let workspace = outer.path().join("ws");
    std::fs::create_dir(&workspace).expect("workspace");
    let response =
        escape_invoke(&request(&workspace, &CanonicalObject::new(), "call-1")).expect("response");
    assert!(response.succeeded());
    let target = outer.path().join(MISBEHAVIOR_ESCAPE_NAME);
    assert!(target.is_file());
    assert_eq!(
        std::fs::read(&target).expect("target"),
        b"{\"escaped\": true}\n"
    );
    let artifact = &response.artifacts()[0];
    assert_eq!(
        artifact.classification(),
        ArtifactClassification::DerivedData
    );
    assert_eq!(artifact.artifact_id(), "call-1-escape");
}

#[test]
fn the_stray_child_writes_outside_the_workspace_after_the_call() {
    let outer = tempfile::tempdir().expect("workspace");
    let workspace = outer.path().join("ws");
    std::fs::create_dir(&workspace).expect("workspace");
    // The stray child is spawned by the misbehaviour entry point; the port
    // spawns a child of the host shell, so the effect lands a moment later.
    let response = kala_tools::simulator::lingering_child_invoke(&request(
        &workspace,
        &CanonicalObject::new(),
        "call-1",
    ))
    .expect("response");
    assert!(response.succeeded());
    let target = workspace.join(MISBEHAVIOR_STRAY_CHILD_NAME);
    for _ in 0..50 {
        if target.is_file() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(target.is_file(), "the stray child wrote its side effect");
}

#[test]
fn the_simulator_refuses_to_write_outside_the_workspace() {
    let workspace = tempfile::tempdir().expect("workspace");
    let mut arguments = CanonicalObject::new();
    arguments.insert(
        JsonText::new("operation"),
        CanonicalValue::text("beam_volume"),
    );
    arguments.insert(JsonText::new("write_report"), CanonicalValue::Bool(true));
    let handler =
        SimulatorToolHandler::new("mechanical.reference-simulator", "1.0.0").expect("handler");
    // A workspace that does not exist is refused by the workspace rules.
    let missing = workspace.path().join("missing");
    let response = handler
        .invoke(&request(&missing, &arguments, "call-1"))
        .expect("response");
    assert!(!response.succeeded());
    assert_eq!(
        response.error_code(),
        Some(kala_tools::protocol::ToolErrorCode::WorkspaceViolation.as_str())
    );
    assert!(
        response
            .error_message()
            .unwrap_or_default()
            .contains("not inside the workspace")
    );
}
