//! The security tests of the native tool executor.
//!
//! The tests cover the controls the executor enforces, the refusals it
//! reports and the boundaries it never crosses. Every control that the
//! Python implementation claims is claimed here too, and every control that
//! the host could not establish is never claimed.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kala_contracts::{ArtifactClassification, ArtifactRef, ContractModel, ToolRunStatus};
use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::executor::{
    ExecutionOptions, ExecutorConfig, IsolationMode, SandboxProfile, TEXTUAL_MEDIA_SUFFIXES,
    ToolExecutor,
};
use kala_tools::protocol::{
    PROTECTED_CHILD_ENVIRONMENT_KEYS, ToolErrorCode, ToolResponseEnvelope, build_child_environment,
};
use kala_tools::registry::ToolRegistry;
use kala_tools::simulator::{
    SIMULATOR_CAPABILITIES, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION, input_schema, output_schema,
    register_simulator,
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
fn executor(root: &Path, granted: BTreeSet<String>, profile: SandboxProfile) -> ToolExecutor {
    let registry = Arc::new(ToolRegistry::new());
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator is registered");
    let mut config = ExecutorConfig::new(registry, root.join("run"));
    config.granted_capabilities = granted;
    config.profile = profile;
    config.import_roots = vec![product_source_root()];
    config.python_executable = Some(python_executable().to_string_lossy().into_owned());
    config.sandbox_probe_helper = Some(PathBuf::from(env!(
        "CARGO_BIN_EXE_kala-tools-sandbox-probe"
    )));
    config.sandbox_init = Some(PathBuf::from(env!("CARGO_BIN_EXE_kala-tools-sandbox-init")));
    config.interpreter_layout = Some(support::python_interpreter_layout());
    ToolExecutor::new(config).expect("the executor is built")
}

/// The simulator call of the tests.
fn simulator_call(call_id: &str) -> kala_contracts::ToolCall {
    tool_call(
        call_id,
        SIMULATOR_TOOL_ID,
        SIMULATOR_TOOL_VERSION,
        "{\"operation\": \"beam_volume\", \"write_report\": true}",
        &SIMULATOR_CAPABILITIES,
        "local_write",
        20.0,
    )
}

#[test]
fn the_child_environment_is_minimal_and_workspace_scoped() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let workspace = root.path().join("child");
    std::fs::create_dir_all(&workspace).expect("the workspace");
    let environment = build_child_environment(&workspace, &[&product_source_root()], None)
        .expect("the child environment");
    let mut keys: Vec<&str> = environment.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "HOME",
            "KALA_TOOL_PROTOCOL",
            "LANG",
            "PATH",
            "PYTHONDONTWRITEBYTECODE",
            "PYTHONHASHSEED",
            "PYTHONIOENCODING",
            "PYTHONNOUSERSITE",
            "PYTHONPATH",
            "PYTHONSAFEPATH",
            "TMPDIR",
        ]
    );
    assert_eq!(environment.get("HOME"), environment.get("TMPDIR"));
    assert!(!environment.contains_key("SECRET"));
}

#[test]
fn the_environment_keys_the_executor_owns_cannot_be_overridden() {
    for key in PROTECTED_CHILD_ENVIRONMENT_KEYS {
        let mut extra = BTreeMap::new();
        extra.insert((*key).to_owned(), "/etc".to_owned());
        let error = build_child_environment(Path::new("/tmp"), &[], Some(&extra))
            .expect_err("the key is refused");
        assert!(error.to_string().contains("protected"), "{key}: {error}");
    }
}

#[test]
fn the_portable_profile_never_claims_a_filesystem_control() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let controls = executor.enforced_controls();
    for confinement in [
        "chroot_workspace_only",
        "filesystem_confinement",
        "network_isolation",
        "user_namespace",
        "mount_namespace",
        "workspace_read_write",
        "pid_namespace",
        "procfs",
    ] {
        assert!(!controls.contains(&confinement.to_owned()), "{confinement}");
    }
    assert!(!executor.sandbox_probe().available);
    assert_eq!(executor.sandbox_probe().platform, "not-probed");
}

#[test]
fn the_strict_profile_runs_the_worker_in_the_jail_it_verified() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let profile =
        SandboxProfile::new().with_isolation_profile(kala_sandbox::IsolationProfile::StrictLinux);
    let executor = executor(root.path(), simulator_capabilities(), profile);
    let probe = executor.sandbox_probe();
    if !probe.available {
        // The host cannot establish the jail: the executor refuses the side
        // effect instead of running the worker anyway.
        let result = executor.execute(&simulator_call("call-strict"), &ExecutionOptions::new());
        assert_eq!(result.status(), ToolRunStatus::Denied);
        assert_eq!(result.error_code(), Some("sandbox_unavailable"));
        assert!(executor.reports().is_empty());
        return;
    }
    for control in [
        "chroot_workspace_only",
        "mount_namespace",
        "network_isolation",
        "pid_namespace",
        "procfs",
        "user_namespace",
        "workspace_read_write",
    ] {
        assert!(
            probe.controls.iter().any(|value| value == control),
            "{control} is not verified"
        );
        assert!(
            executor
                .enforced_controls()
                .iter()
                .any(|value| value == control),
            "{control} is not enforced"
        );
    }
    let result = executor.execute(&simulator_call("call-strict"), &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Succeeded, "{result:?}");
    let report = &executor.reports()[0];
    assert_eq!(report.status, ToolRunStatus::Succeeded);
    assert_eq!(report.enforced_controls, executor.enforced_controls());
    // The jail is removed after the run.
    let sandbox = executor.run_directory().join("sandbox");
    if sandbox.exists() {
        let mut stack = vec![sandbox.clone()];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    assert!(
                        path.file_name().map(|name| name != "root").unwrap_or(true),
                        "the sandbox root survived the run"
                    );
                    stack.push(path);
                }
            }
        }
    }
}

#[test]
fn an_artifact_outside_the_workspace_is_refused() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let workspace = executor
        .workspace_for("call-artifact")
        .expect("the workspace");
    let payload = b"verified";
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, payload).expect("the outside file");
    let artifact = ArtifactRef::from_value(&CanonicalValue::Object({
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("artifact_id"),
            CanonicalValue::text("artifact-1"),
        );
        entries.insert(
            JsonText::new("uri"),
            CanonicalValue::text(format!("file://{}", outside.to_string_lossy())),
        );
        entries.insert(
            JsonText::new("digest"),
            CanonicalValue::text(kala_tools::protocol::sha256_hex(payload)),
        );
        entries.insert(
            JsonText::new("media_type"),
            CanonicalValue::text("text/plain"),
        );
        entries.insert(
            JsonText::new("size_bytes"),
            CanonicalValue::Integer(num_bigint::BigInt::from(payload.len())),
        );
        entries
    }))
    .expect("the artifact");
    let error = executor
        .verify_artifacts(&[artifact], &workspace)
        .expect_err("the artifact is refused");
    assert!(
        error.to_string().contains("escapes the call workspace"),
        "{error}"
    );
    assert!(outside.is_file());
}

#[test]
fn a_symlinked_artifact_is_refused() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let workspace = executor
        .workspace_for("call-symlink")
        .expect("the workspace");
    let target = workspace.join("payload.txt");
    std::fs::write(&target, b"payload").expect("the payload");
    let link = workspace.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("the link");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &link).expect("the link");
    let artifact = ArtifactRef::from_value(&CanonicalValue::Object({
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("artifact_id"),
            CanonicalValue::text("artifact-1"),
        );
        entries.insert(
            JsonText::new("uri"),
            CanonicalValue::text(format!("file://{}", link.to_string_lossy())),
        );
        entries.insert(
            JsonText::new("digest"),
            CanonicalValue::text(kala_tools::protocol::sha256_hex(b"payload")),
        );
        entries.insert(
            JsonText::new("media_type"),
            CanonicalValue::text("text/plain"),
        );
        entries.insert(
            JsonText::new("size_bytes"),
            CanonicalValue::Integer(num_bigint::BigInt::from(7)),
        );
        entries
    }))
    .expect("the artifact");
    let error = executor
        .verify_artifacts(&[artifact], &workspace)
        .expect_err("the artifact is refused");
    assert!(error.to_string().contains("symlink"), "{error}");
}

#[test]
fn a_duplicate_artifact_identifier_is_refused() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let workspace = executor
        .workspace_for("call-duplicate")
        .expect("the workspace");
    let payload = b"payload";
    let path = workspace.join("report.json");
    std::fs::write(&path, payload).expect("the payload");
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("artifact_id"),
        CanonicalValue::text("artifact-1"),
    );
    entries.insert(
        JsonText::new("uri"),
        CanonicalValue::text(format!("file://{}", path.to_string_lossy())),
    );
    entries.insert(
        JsonText::new("digest"),
        CanonicalValue::text(kala_tools::protocol::sha256_hex(payload)),
    );
    entries.insert(
        JsonText::new("media_type"),
        CanonicalValue::text("application/json"),
    );
    entries.insert(
        JsonText::new("size_bytes"),
        CanonicalValue::Integer(num_bigint::BigInt::from(payload.len())),
    );
    let artifact = ArtifactRef::from_value(&CanonicalValue::Object(entries)).expect("the artifact");
    let error = executor
        .verify_artifacts(&[artifact.clone(), artifact], &workspace)
        .expect_err("the artifact is refused");
    assert!(
        error.to_string().contains("duplicate artifact identifiers"),
        "{error}"
    );
}

#[test]
fn an_unrecognised_artifact_format_is_classified_as_a_design_binary() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let workspace = executor
        .workspace_for("call-classification")
        .expect("the workspace");
    let cases: Vec<(&str, &[u8], ArtifactClassification, ArtifactClassification)> = vec![
        (
            "report.json",
            b"{\"ok\": true}",
            ArtifactClassification::Metadata,
            ArtifactClassification::Text,
        ),
        (
            "notes.md",
            b"plain text",
            ArtifactClassification::Text,
            ArtifactClassification::Text,
        ),
        (
            "unknown.payload",
            b"plain text",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "archive.bin",
            b"PK\x03\x04rest",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "mesh.stl",
            b"solid x\nendsolid x\n",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "part.step",
            b"ISO-10303-21;\n",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "report.json",
            b"{\"ok\": tru\x00e}",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "report.json",
            b"\xff\xfe\x00bad",
            ArtifactClassification::Text,
            ArtifactClassification::DesignBinary,
        ),
        (
            "report.json",
            b"{}",
            ArtifactClassification::DesignBinary,
            ArtifactClassification::DesignBinary,
        ),
        (
            "report.json",
            b"{}",
            ArtifactClassification::DerivedData,
            ArtifactClassification::Text,
        ),
    ];
    for (name, payload, declared, expected) in cases {
        let path = workspace.join(name);
        std::fs::write(&path, payload).expect("the payload");
        let mut entries = CanonicalObject::new();
        entries.insert(
            JsonText::new("artifact_id"),
            CanonicalValue::text(format!("artifact-{name}")),
        );
        entries.insert(
            JsonText::new("uri"),
            CanonicalValue::text(format!("file://{}", path.to_string_lossy())),
        );
        entries.insert(
            JsonText::new("digest"),
            CanonicalValue::text(kala_tools::protocol::sha256_hex(payload)),
        );
        entries.insert(
            JsonText::new("media_type"),
            CanonicalValue::text("application/octet-stream"),
        );
        entries.insert(
            JsonText::new("size_bytes"),
            CanonicalValue::Integer(num_bigint::BigInt::from(payload.len())),
        );
        entries.insert(
            JsonText::new("classification"),
            CanonicalValue::text(declared.as_str()),
        );
        let artifact =
            ArtifactRef::from_value(&CanonicalValue::Object(entries)).expect("the artifact");
        let verified = executor
            .verify_artifacts(&[artifact], &workspace)
            .expect("the artifact is verified");
        assert_eq!(verified[0].classification(), expected, "{name}");
    }
    for name in ["mesh.stl", "part.step"] {
        assert!(
            !TEXTUAL_MEDIA_SUFFIXES.contains(&name),
            "{name} must not be a textual suffix"
        );
    }
}

#[test]
fn the_worker_pool_is_capped_and_a_saturated_pool_denies() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(
        root.path(),
        simulator_capabilities(),
        SandboxProfile::new().with_max_workers(1),
    );
    assert_eq!(executor.profile().max_workers(), 1);
    assert!(
        executor
            .enforced_controls()
            .iter()
            .any(|value| value == "worker_concurrency_cap")
    );
}

#[test]
fn a_call_identifier_cannot_address_a_run_directory_component() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    for call_id in [
        "",
        "   ",
        ".",
        "..",
        "a/b",
        "a\\b",
        "a\x00b",
        &"x".repeat(129),
    ] {
        let error = executor
            .workspace_path(call_id)
            .expect_err("the call identifier is refused");
        assert!(
            error.to_string().contains("call_id"),
            "{call_id:?}: {error}"
        );
    }
    let workspace = executor
        .workspace_path("call-valid")
        .expect("the workspace");
    assert!(workspace.ends_with("calls/call-valid/workspace"));
}

#[test]
fn the_response_envelope_of_a_failed_run_cannot_report_an_error_code_the_kernel_does_not_know() {
    let envelope = ToolResponseEnvelope::failed_envelope(
        "call-1",
        "mechanical.reference-simulator",
        "1.0.0",
        ToolRunStatus::Failed,
        Some("totally_fine_code"),
        Some("reported under a code the kernel does not recognise"),
        0.0,
    )
    .expect("the envelope");
    assert_eq!(envelope.error_code(), Some("totally_fine_code"));
    let executor = executor(
        tempfile::tempdir().expect("a temporary directory").path(),
        simulator_capabilities(),
        SandboxProfile::new(),
    );
    let workspace = executor.workspace_for("call-code").expect("the workspace");
    let verified = executor
        .verify_response(
            &envelope,
            &executor
                .registry()
                .require(SIMULATOR_TOOL_ID, None)
                .expect("the tool"),
            &tool_call(
                "call-1",
                SIMULATOR_TOOL_ID,
                SIMULATOR_TOOL_VERSION,
                "{}",
                &SIMULATOR_CAPABILITIES,
                "local_write",
                5.0,
            ),
            &workspace,
            0.0,
        )
        .expect("the envelope is verified");
    assert_eq!(verified.error_code(), Some("tool_error"));
}

#[test]
fn the_executor_refuses_a_tool_that_claims_ungranted_authority() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let registry = Arc::new(ToolRegistry::new());
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator is registered");
    let mut config = ExecutorConfig::new(registry, root.path().join("run"));
    config.granted_capabilities = BTreeSet::from(["design.read".to_owned()]);
    let executor = ToolExecutor::new(config).expect("the executor is built");
    let error = executor
        .require_authority(
            &executor
                .registry()
                .require(SIMULATOR_TOOL_ID, None)
                .expect("the tool"),
        )
        .expect_err("the tool is refused");
    assert_eq!(error.python_class_name(), "ToolAuthorityError");
    assert!(error.message().contains("artifact.write"), "{error}");
}

#[test]
fn the_network_and_the_egress_capabilities_are_denied_by_default() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let registry = Arc::new(ToolRegistry::new());
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator is registered");
    let mut config = ExecutorConfig::new(registry, root.path().join("run"));
    config.granted_capabilities = BTreeSet::from(["network".to_owned()]);
    let executor = ToolExecutor::new(config).expect("the executor is built");
    let mut entries = CanonicalObject::new();
    entries.insert(
        JsonText::new("tool_id"),
        CanonicalValue::text("test.network"),
    );
    entries.insert(JsonText::new("version"), CanonicalValue::text("1.0.0"));
    entries.insert(JsonText::new("name"), CanonicalValue::text("Network"));
    entries.insert(
        JsonText::new("description"),
        CanonicalValue::text("A tool that needs the network."),
    );
    entries.insert(
        JsonText::new("input_schema_digest"),
        CanonicalValue::text(
            kala_tools::protocol::canonical_json_digest(&input_schema()).expect("digest"),
        ),
    );
    entries.insert(
        JsonText::new("output_schema_digest"),
        CanonicalValue::text(
            kala_tools::protocol::canonical_json_digest(&output_schema()).expect("digest"),
        ),
    );
    entries.insert(
        JsonText::new("side_effect_level"),
        CanonicalValue::text("network"),
    );
    entries.insert(
        JsonText::new("required_capabilities"),
        CanonicalValue::array([CanonicalValue::text("network")]),
    );
    entries.insert(JsonText::new("timeout_seconds"), CanonicalValue::Float(5.0));
    entries.insert(JsonText::new("destructive"), CanonicalValue::Bool(false));
    entries.insert(
        JsonText::new("supports_structured_output"),
        CanonicalValue::Bool(true),
    );
    entries.insert(JsonText::new("idempotent"), CanonicalValue::Bool(true));
    let descriptor = kala_contracts::ToolDescriptor::from_value(&CanonicalValue::Object(entries))
        .expect("descriptor");
    let handler = Arc::new(
        kala_tools::protocol::SubprocessToolHandler::new(
            descriptor.clone(),
            "kala_agent.tools.simulator:invoke",
            &["-B", "-s"],
        )
        .expect("the handler"),
    );
    let tool = executor
        .registry()
        .build(descriptor, handler, input_schema(), output_schema())
        .expect("the tool");
    let error = executor
        .require_authority(&tool)
        .expect_err("the network capability is refused");
    assert_eq!(error.python_class_name(), "ToolAuthorityError");
    assert!(
        error
            .message()
            .contains("network capabilities are denied by default"),
        "{error}"
    );
}

#[test]
fn the_isolation_mode_never_runs_a_subprocess_handler_inside_the_kernel() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let result = executor.execute(&simulator_call("call-isolation"), &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Succeeded);
    assert_eq!(executor.reports()[0].isolation, IsolationMode::Subprocess);
    assert!(executor.reports()[0].process_id.is_some());
}

#[test]
fn the_output_bound_is_enforced() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(
        root.path(),
        simulator_capabilities(),
        SandboxProfile::new().with_max_output_bytes(64),
    );
    let result = executor.execute(&simulator_call("call-output"), &ExecutionOptions::new());
    assert_eq!(result.status(), ToolRunStatus::Failed);
    assert_eq!(result.error_code(), Some("output_too_large"));
}

#[test]
fn the_error_codes_the_executor_reports_are_the_ones_the_kernel_knows() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let executor = executor(root.path(), simulator_capabilities(), SandboxProfile::new());
    let absent = tool_call("call-codes", "test.absent", "1.0.0", "{}", &[], "read", 5.0);
    let result = executor.execute(&absent, &ExecutionOptions::new());
    let code = result.error_code().expect("an error code");
    assert!(ToolErrorCode::parse(code).is_some(), "{code}");
    assert_eq!(code, "unknown_tool");
}

#[test]
fn the_schemas_of_the_simulator_are_the_ones_the_registry_accepts() {
    let registry = ToolRegistry::new();
    register_simulator(&registry, SIMULATOR_TOOL_ID, SIMULATOR_TOOL_VERSION)
        .expect("the simulator is registered");
    let tool = registry.require(SIMULATOR_TOOL_ID, None).expect("the tool");
    assert_eq!(tool.input_schema(), &input_schema());
    assert_eq!(tool.output_schema(), &output_schema());
}
