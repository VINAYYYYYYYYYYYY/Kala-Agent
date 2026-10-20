//! The branch tests of the native tool executor.
//!
//! The tests cover the decision order of the executor, the refusals it
//! reports and the provenance records it writes. The live differential
//! against the repository Python is in `executor_differential.rs`.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kala_contracts::ToolRunStatus;
use kala_json::{CanonicalValue, JsonText};
use kala_tools::executor::{
    ExecutionOptions, ExecutorConfig, IsolationMode, SandboxProfile, ToolExecutor,
};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    EVALUATOR_CAPABILITIES, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION, SIMULATOR_CAPABILITIES,
    SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION, register_reference_evaluator, register_simulator,
};
use support::{product_source_root, python_executable, tool_call};

/// The capabilities of the simulator.
fn simulator_capabilities() -> BTreeSet<String> {
    SIMULATOR_CAPABILITIES
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

/// An executor with the simulator registered.
fn executor(root: &std::path::Path, granted: BTreeSet<String>) -> ToolExecutor {
    let registry = Arc::new(ToolRegistry::new());
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator is registered");
    let mut config = ExecutorConfig::new(registry, root.join("run"));
    config.granted_capabilities = granted;
    config.import_roots = vec![product_source_root()];
    config.python_executable = Some(python_executable().to_string_lossy().into_owned());
    ToolExecutor::new(config).expect("the executor is built")
}

/// The simulator call of the tests.
fn simulator_call(call_id: &str) -> kala_contracts::ToolCall {
    tool_call(
        call_id,
        SIMULATOR_TOOL_ID,
        SIMULATOR_TOOL_VERSION,
        "{\"operation\": \"beam_volume\"}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        20.0,
    )
}

#[test]
fn the_simulator_runs_out_of_process_and_reports_its_provenance() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let result = executor.execute(&simulator_call("call-1"), &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Succeeded, "{result:?}");
    assert!(result.error_code().is_none());
    let reports = executor.reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].call_id, "call-1");
    assert_eq!(reports[0].isolation, IsolationMode::Subprocess);
    assert!(reports[0].process_id.is_some());
    assert_eq!(reports[0].status, ToolRunStatus::Succeeded);
    assert_eq!(
        reports[0].granted_capabilities,
        simulator_capabilities().into_iter().collect::<Vec<_>>()
    );
    assert!(reports[0].envelope_digest.len() == 64);
}

#[test]
fn an_unknown_tool_is_denied_before_the_workspace_is_created() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let call = tool_call(
        "call-absent",
        "test.absent",
        SIMULATOR_TOOL_VERSION,
        "{}",
        &[],
        "read",
        5.0,
    );
    let result = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Denied);
    assert_eq!(result.error_code(), Some("unknown_tool"));
    assert!(executor.reports().is_empty());
    assert!(!executor.run_directory().join("calls").exists());
}

#[test]
fn an_unknown_version_is_denied() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let call = tool_call(
        "call-version",
        SIMULATOR_TOOL_ID,
        "9.9.9",
        "{}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        5.0,
    );
    let result = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Denied);
    assert_eq!(result.error_code(), Some("version_not_found"));
}

#[test]
fn a_missing_capability_is_denied_before_the_workspace_is_created() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), BTreeSet::from(["design.read".to_owned()]));
    let result = executor.execute(&simulator_call("call-capability"), &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Denied);
    assert_eq!(result.error_code(), Some("capability_denied"));
    assert!(executor.reports().is_empty());
    assert!(!executor.run_directory().join("calls").exists());
}

#[test]
fn the_side_effect_must_match_the_descriptor() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let call = tool_call(
        "call-side-effect",
        SIMULATOR_TOOL_ID,
        SIMULATOR_TOOL_VERSION,
        "{}",
        &SIMULATOR_CAPABILITIES,
        "read",
        5.0,
    );
    let result = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Denied);
    assert_eq!(result.error_code(), Some("side_effect_mismatch"));
}

#[test]
fn an_undeclared_capability_is_denied() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let mut granted = simulator_capabilities();
    granted.insert("artifact.archive".to_owned());
    let executor = executor(root.path(), granted);
    let call = tool_call(
        "call-undeclared",
        SIMULATOR_TOOL_ID,
        SIMULATOR_TOOL_VERSION,
        "{}",
        &[
            "artifact.archive",
            "artifact.read",
            "artifact.write",
            "design.read",
            "tool.execute",
        ],
        "local_write",
        5.0,
    );
    let result = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Denied);
    assert_eq!(result.error_code(), Some("capability_denied"));
}

#[test]
fn the_default_mode_never_runs_a_tool_inside_the_kernel() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let registry = Arc::new(ToolRegistry::new());
    register_reference_evaluator(&registry, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
        .expect("the evaluator is registered");
    let mut config = ExecutorConfig::new(registry, root.path().join("run"));
    config.granted_capabilities = BTreeSet::new();
    config.python_executable = Some(python_executable().to_string_lossy().into_owned());
    let executor = ToolExecutor::new(config).expect("the executor is built");
    let call = tool_call(
        "call-evaluator",
        EVALUATOR_TOOL_ID,
        EVALUATOR_TOOL_VERSION,
        "{\"operation\": \"beam_volume\"}",
        &EVALUATOR_CAPABILITIES,
        "none",
        10.0,
    );
    let refused = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(refused.status(), ToolRunStatus::Denied);
    assert_eq!(refused.error_code(), Some("isolation_required"));
    assert!(executor.reports().is_empty());
}

#[test]
fn the_trusted_mode_runs_a_registered_pure_evaluator() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let registry = Arc::new(ToolRegistry::new());
    register_reference_evaluator(&registry, EVALUATOR_TOOL_ID, EVALUATOR_TOOL_VERSION)
        .expect("the evaluator is registered");
    let mut config = ExecutorConfig::new(registry, root.path().join("run"));
    config.granted_capabilities = BTreeSet::new();
    config.isolation = IsolationMode::TrustedInProcessEvaluator;
    config.python_executable = Some(python_executable().to_string_lossy().into_owned());
    let executor = ToolExecutor::new(config).expect("the executor is built");
    let call = tool_call(
        "call-evaluator",
        EVALUATOR_TOOL_ID,
        EVALUATOR_TOOL_VERSION,
        "{\"operation\": \"beam_volume\"}",
        &EVALUATOR_CAPABILITIES,
        "none",
        10.0,
    );
    let result = executor.execute(&call, &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Succeeded, "{result:?}");
    let reports = executor.reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].isolation,
        IsolationMode::TrustedInProcessEvaluator
    );
    assert!(reports[0].process_id.is_none());
}

#[test]
fn a_cancelled_call_never_dispatches() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let cancel = kala_process::CancelToken::new();
    cancel.cancel();
    let options = ExecutionOptions::new().with_cancel(Some(&cancel));
    let result = executor.execute(&simulator_call("call-cancelled"), &options);
    assert_eq!(result.status(), ToolRunStatus::Cancelled);
    assert!(result.error_code().is_none());
    assert!(executor.reports().is_empty());
}

#[test]
fn the_profile_refuses_the_values_it_cannot_honour() {
    let profile = SandboxProfile::new().with_max_workers(0);
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(error.message(), "max_workers must be between 1 and 8");

    let profile = SandboxProfile::new().with_profile_name("   ");
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(error.message(), "profile_name must be a nonempty string");

    let profile = SandboxProfile::new().with_default_timeout_seconds(0.0);
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(
        error.message(),
        "default_timeout_seconds must be within the supported bounds"
    );

    let profile = SandboxProfile::new().with_max_queue_seconds(1_000.0);
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(
        error.message(),
        "max_queue_seconds must be within the supported bounds"
    );

    let profile = SandboxProfile::new().with_max_output_bytes(0);
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(
        error.message(),
        "max_output_bytes must be a positive integer"
    );
}

#[test]
fn the_profile_refuses_an_environment_key_it_does_not_own() {
    let profile = SandboxProfile::new().with_extra_environment(BTreeMap::from([(
        "SECRET_TOKEN".to_owned(),
        "x".to_owned(),
    )]));
    let error = profile.validated().expect_err("the profile is refused");
    assert!(
        error
            .message()
            .contains("extra_environment only accepts non-authority keys"),
        "{error}"
    );

    let profile = SandboxProfile::new()
        .with_extra_environment(BTreeMap::from([("HOME".to_owned(), "/etc".to_owned())]));
    let error = profile.validated().expect_err("the profile is refused");
    assert_eq!(
        error.message(),
        "extra_environment cannot override the protected key HOME"
    );
}

#[test]
fn the_deadline_is_the_smallest_one_and_is_capped_by_the_profile() {
    let profile = SandboxProfile::new().with_default_timeout_seconds(30.0);
    assert_eq!(profile.timeout_for(Some(5.0), 20.0), 5.0);
    assert_eq!(profile.timeout_for(Some(60.0), 20.0), 20.0);
    assert_eq!(profile.timeout_for(Some(0.0), 20.0), 20.0);
    assert_eq!(profile.timeout_for(None, 0.0), 30.0);
    let profile = SandboxProfile::new().with_default_timeout_seconds(0.001);
    assert_eq!(profile.timeout_for(None, 0.0), 0.01);
}

#[test]
fn the_portable_profile_never_claims_a_filesystem_control() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    let controls = executor.enforced_controls();
    assert_eq!(controls, {
        let mut sorted = controls.clone();
        sorted.sort();
        sorted
    });
    assert!(controls.contains(&"process_isolation".to_owned()));
    assert!(controls.contains(&"process_group_kill".to_owned()));
    assert!(controls.contains(&"worker_concurrency_cap".to_owned()));
    for confinement in [
        "chroot_workspace_only",
        "filesystem_confinement",
        "network_isolation",
        "user_namespace",
        "mount_namespace",
        "workspace_read_write",
    ] {
        assert!(!controls.contains(&confinement.to_owned()), "{confinement}");
    }
    assert!(!executor.sandbox_probe().available);
}

#[test]
fn the_report_payload_carries_no_secret() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities());
    executor.execute(&simulator_call("call-report"), &ExecutionOptions::new());
    let payload = executor.reports()[0].to_payload();
    let CanonicalValue::Object(entries) = &payload else {
        panic!("the payload is not an object");
    };
    assert_eq!(entries.len(), 12);
    assert_eq!(
        entries
            .get(&JsonText::new("status"))
            .and_then(|value| match value {
                CanonicalValue::Text(text) => Some(text.to_string_lossy().to_owned()),
                _ => None,
            }),
        Some("succeeded".to_owned())
    );
    assert_eq!(
        entries
            .get(&JsonText::new("sandbox_profile"))
            .and_then(|value| match value {
                CanonicalValue::Text(text) => Some(text.to_string_lossy().to_owned()),
                _ => None,
            }),
        Some("portable-minimum".to_owned())
    );
}
