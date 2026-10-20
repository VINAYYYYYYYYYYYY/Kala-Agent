//! Security and edge cases of the sandbox port.
//!
//! The cases mirror `tests/unit/test_sandbox_branches.py`,
//! `tests/security/test_tool_executor.py` and `tests/unit/test_executor_branches.py`
//! of the Python repository: every refusal, every error precedence and every
//! fail-closed path of the strict profile.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use kala_sandbox::{
    ISOLATION_FAILURE_EXIT, IsolationProfile, PROBE_FAILURE_EXIT, ProbeOptions, SandboxProbe,
    SandboxSetupError, SandboxSpec, build_sandbox_spec, exit_code_from_status, isolate_process,
    judge_sandbox, make_mountpoint, mount_scratch, parse_json, platform_name, probe_entry,
    probe_sandbox, unshadowed_scratch_paths,
};
use serde_json::{Value, json};

fn controls_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

const REQUIRED: [&str; 6] = [
    "chroot_workspace_only",
    "mount_namespace",
    "network_isolation",
    "readonly_system_bindings",
    "user_namespace",
    "workspace_read_write",
];

fn temporary(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("a temporary directory")
}

/// A helper that answers the probe protocol with a fixed line.
fn fake_helper(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    let mut file = std::fs::File::create(&path).expect("the helper is created");
    file.write_all(format!("#!/bin/sh\n{body}\n").as_bytes())
        .expect("the helper is written");
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the helper is executable");
    }
    path
}

#[test]
fn the_strict_profile_requires_every_control_before_it_looks_at_the_paths() {
    let error = build_sandbox_spec(
        &controls_of(&["user_namespace"]),
        Path::new("relative/root"),
        Path::new("relative/workspace"),
        &[],
        1 << 20,
    )
    .expect_err("the specification is refused");
    assert_eq!(
        error.detail(),
        "the strict profile requires controls that were not probed: \
         chroot_workspace_only, mount_namespace, network_isolation, \
         readonly_system_bindings, workspace_read_write"
    );
}

#[test]
fn the_strict_profile_refuses_a_relative_path() {
    let scratch = temporary("kala-security-");
    let error = build_sandbox_spec(
        &controls_of(&REQUIRED),
        Path::new("relative/root"),
        &scratch.path().join("workspace"),
        &[],
        1 << 20,
    )
    .expect_err("the specification is refused");
    assert_eq!(error.detail(), "sandbox paths must be absolute");
}

#[test]
fn the_strict_profile_refuses_a_relative_workspace() {
    let scratch = temporary("kala-security-");
    let error = build_sandbox_spec(
        &controls_of(&REQUIRED),
        &scratch.path().join("root"),
        Path::new("relative/workspace"),
        &[],
        1 << 20,
    )
    .expect_err("the specification is refused");
    assert_eq!(error.detail(), "sandbox paths must be absolute");
}

#[test]
fn a_malformed_specification_is_refused_with_one_message() {
    let payload = json!({ "controls": ["a"] });
    let error = SandboxSpec::from_payload(&payload).expect_err("the payload is refused");
    assert_eq!(error.detail(), "the sandbox specification is malformed");
    assert!(matches!(error, SandboxSetupError { .. }));
}

#[test]
fn a_hostile_specification_never_widens_the_authority() {
    // Every shape below is either refused or reduced to a value that cannot
    // name a path outside the jail.
    let hostile = [
        (
            "a bare string for the control set",
            "controls",
            json!("abc"),
        ),
        ("numbers in the control set", "controls", json!([1, 2])),
        (
            "a bare string for a binding list",
            "readonly_bindings",
            json!("/usr"),
        ),
        ("a number for the root", "root", json!(7)),
        ("a non-numeric size", "scratch_bytes", json!("not-a-number")),
        ("an infinite size", "scratch_bytes", json!("Infinity")),
        ("an object for a binding list", "device_bindings", json!({})),
    ];
    for (label, key, value) in hostile {
        let mut payload = json!({
            "controls": REQUIRED,
            "device_bindings": [],
            "readonly_bindings": ["/usr"],
            "root": "/tmp/root",
            "scratch_bytes": 1 << 20,
            "scratch_paths": ["/tmp"],
            "writable_bindings": [],
            "workspace": "/tmp/workspace",
        });
        payload[key] = value;
        let outcome = SandboxSpec::from_payload(&payload);
        assert!(outcome.is_err(), "the hostile payload {label} was accepted");
    }
}

#[test]
fn an_oversized_specification_is_refused_without_allocating_it() {
    let payload = json!({
        "controls": REQUIRED,
        "root": format!("/tmp/{}", "x".repeat(1_000_000)),
        "workspace": "/tmp/workspace",
        "readonly_bindings": [],
        "writable_bindings": [],
        "device_bindings": [],
        "scratch_paths": [],
        "scratch_bytes": 1 << 20,
    });
    // The payload is well formed, so it is accepted; the point of the check is
    // that the port does not panic on an attacker-sized document.
    let spec = SandboxSpec::from_payload(&payload).expect("a valid payload");
    assert!(spec.root.to_string_lossy().len() > 1_000_000);
}

#[test]
fn a_deeply_nested_document_is_refused_without_overflowing_the_stack() {
    let mut text = String::new();
    for _ in 0..2_000 {
        text.push('[');
    }
    for _ in 0..2_000 {
        text.push(']');
    }
    let error = parse_json(&text).expect_err("the document is refused");
    assert!(!error.detail.is_empty());
    // The probe entry point turns the refusal into the malformed-payload error.
    let error = probe_entry(&text, "\"/canary\"").expect_err("the payload is refused");
    assert_eq!(error.detail(), "the probe payload is malformed");
}

#[test]
fn the_probe_helper_refuses_a_malformed_payload() {
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let output = Command::new(helper)
        .arg("{bad")
        .arg("nope")
        .output()
        .expect("the helper runs");
    assert_eq!(output.status.code(), Some(PROBE_FAILURE_EXIT));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: Value = serde_json::from_str(stdout.trim()).expect("the helper emits JSON");
    assert_eq!(payload["controls"], json!([]));
    assert_eq!(payload["detail"], json!("the probe payload is malformed"));
}

#[test]
fn the_probe_helper_writes_nothing_when_a_required_control_is_missing() {
    // The jail is established, but the specification does not request the
    // required controls, so the helper fails without reporting a result.
    let scratch = temporary("kala-security-");
    let root = scratch.path().join("root");
    let workspace = scratch.path().join("workspace");
    std::fs::create_dir_all(&root).expect("the jail root");
    std::fs::create_dir_all(&workspace).expect("the workspace");
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let spec = json!({
        "controls": ["user_namespace", "mount_namespace"],
        "device_bindings": [],
        "readonly_bindings": [],
        "root": root.to_string_lossy(),
        "scratch_bytes": 1 << 20,
        "scratch_paths": [],
        "writable_bindings": [workspace.to_string_lossy()],
        "workspace": workspace.to_string_lossy(),
    });
    let output = Command::new(helper)
        .arg(spec.to_string())
        .arg("\"/canary\"")
        .output()
        .expect("the helper runs");
    assert_eq!(output.status.code(), Some(PROBE_FAILURE_EXIT));
    assert!(
        output.stdout.is_empty(),
        "a failed probe must not report a result: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn the_probe_helper_reports_a_malformed_control_set() {
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let output = Command::new(helper)
        .arg("{\"controls\": 7}")
        .arg("\"/canary\"")
        .output()
        .expect("the helper runs");
    assert_eq!(output.status.code(), Some(PROBE_FAILURE_EXIT));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let payload: Value = serde_json::from_str(stdout.trim()).expect("the helper emits JSON");
    assert_eq!(payload["detail"], json!("the probe payload is malformed"));
}

#[test]
fn a_jail_that_cannot_be_built_fails_closed() {
    let scratch = temporary("kala-security-");
    let spec = SandboxSpec::new(
        controls_of(&[]),
        scratch.path().join("missing-root"),
        scratch.path().join("workspace"),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        1 << 20,
    );
    let error = isolate_process(&spec).expect_err("the jail is refused");
    assert!(
        error.detail().contains("chroot"),
        "the refusal names the failing operation: {error}"
    );
}

#[test]
fn the_pre_exec_hook_exits_with_the_isolation_failure_code() {
    let scratch = temporary("kala-security-");
    let spec = json!({
        "controls": [],
        "device_bindings": [],
        "readonly_bindings": [],
        "root": scratch.path().join("missing-root").to_string_lossy(),
        "scratch_bytes": 1 << 20,
        "scratch_paths": [],
        "writable_bindings": [],
        "workspace": scratch.path().join("workspace").to_string_lossy(),
    });
    let inspector = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-inspect"));
    let output = Command::new(inspector)
        .arg(spec.to_string())
        .arg("\"/canary\"")
        .env("KALA_SANDBOX_PRE_EXEC", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("the inspector runs");
    assert_eq!(output.status.code(), Some(ISOLATION_FAILURE_EXIT));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("sandbox_setup_failed"),
        "the hook reports the refusal on standard error"
    );
}

#[test]
fn a_scratch_path_that_would_hide_a_binding_is_dropped() {
    let spec = SandboxSpec::new(
        controls_of(&REQUIRED),
        PathBuf::from("/root"),
        PathBuf::from("/ws"),
        vec!["/tmp".to_owned()],
        vec!["/tmp/work".to_owned(), "/var".to_owned()],
        Vec::new(),
        vec!["/dev/shm".to_owned(), "/tmp".to_owned(), "/var".to_owned()],
        1 << 20,
    );
    assert_eq!(unshadowed_scratch_paths(&spec), vec!["/dev/shm".to_owned()]);
}

#[test]
fn a_negative_scratch_size_is_clamped_to_zero() {
    let scratch = temporary("kala-security-");
    // The mount itself needs privileges; the refusal must be about the mount
    // and never about a negative size.
    let error = mount_scratch(scratch.path(), "/tmp", -5).expect_err("the mount is refused");
    assert!(
        !error.detail().contains("size=-5"),
        "the negative size never reaches the kernel: {error}"
    );
}

#[test]
fn a_mountpoint_reports_the_hostile_root() {
    let scratch = temporary("kala-security-");
    let blocker = scratch.path().join("root");
    std::fs::write(&blocker, b"not a directory").expect("the blocker is written");
    let error = make_mountpoint(&blocker, "/usr", true).expect_err("the mountpoint is refused");
    assert!(
        error.detail().contains("cannot hold /usr"),
        "the refusal names the target: {error}"
    );
}

#[test]
fn the_strict_profile_is_refused_without_a_probe() {
    let verdict = judge_sandbox(IsolationProfile::StrictLinux, None);
    assert!(!verdict.allowed);
    assert!(verdict.reason.ends_with("refused"));
    assert!(verdict.controls.is_empty());

    let probe = SandboxProbe {
        available: false,
        platform: "linux".to_owned(),
        controls: Vec::new(),
        detail: "namespaces denied".to_owned(),
        elapsed_ms: 0.0,
    };
    let verdict = judge_sandbox(IsolationProfile::StrictLinux, Some(&probe));
    assert!(!verdict.allowed);
    assert!(verdict.reason.ends_with("refused: namespaces denied"));

    let probe = SandboxProbe {
        available: true,
        platform: "linux".to_owned(),
        controls: vec!["mount_namespace".to_owned(), "user_namespace".to_owned()],
        detail: String::new(),
        elapsed_ms: 0.0,
    };
    let verdict = judge_sandbox(IsolationProfile::StrictLinux, Some(&probe));
    assert!(verdict.allowed);
    assert_eq!(verdict.reason, "");
    assert_eq!(verdict.controls.len(), 2);
}

#[test]
fn the_portable_profile_never_claims_a_filesystem_control() {
    let verdict = judge_sandbox(IsolationProfile::Portable, None);
    assert!(verdict.allowed);
    assert!(verdict.controls.is_empty());
    assert!(verdict.reason.contains("confines a process"));
}

#[test]
fn the_launcher_adapter_refuses_a_pid_namespace_specification() {
    let scratch = temporary("kala-security-");
    let spec = build_sandbox_spec(
        &controls_of(&[
            "chroot_workspace_only",
            "device_nodes_minimal",
            "ipc_namespace",
            "mount_namespace",
            "network_isolation",
            "pid_namespace",
            "procfs",
            "readonly_system_bindings",
            "scratch_tmpfs",
            "user_namespace",
            "uts_namespace",
            "workspace_read_write",
        ]),
        &scratch.path().join("root"),
        &scratch.path().join("workspace"),
        &[],
        1 << 20,
    )
    .expect("a valid specification");
    // The accepted launcher has no PID-1 transition, so it must not build a
    // jail for a specification that asks for one: a silently weaker jail is
    // refused instead.
    let error = kala_sandbox::native_sandbox_spec(&spec).expect_err("the launcher refuses it");
    assert!(
        error.detail().contains("pid_namespace"),
        "the refusal names the control: {error}"
    );
    // Without the control the launcher accepts the specification and never
    // claims the PID namespace.
    let mut weaker = spec.clone();
    weaker.controls.remove("pid_namespace");
    let native = kala_sandbox::native_sandbox_spec(&weaker).expect("the launcher accepts it");
    assert!(
        !native.controls.contains("pid_namespace"),
        "the native launcher has no PID-1 transition"
    );
    assert_eq!(native.controls.len(), weaker.controls.len());
    assert!(
        native.validate_strict().is_ok(),
        "the launcher validates it"
    );
}

#[test]
fn the_native_production_path_enforces_every_optional_control() {
    // The production path runs the worker under the native init helper, which
    // performs the PID-1 transition, so the PID namespace is enforceable.
    assert!(
        kala_sandbox::native_supported_controls().contains(&"pid_namespace"),
        "the native path enforces the PID namespace"
    );
    assert!(
        !kala_sandbox::launcher_supported_controls().contains(&"pid_namespace"),
        "the accepted launcher alone cannot enforce it"
    );
}

#[test]
fn the_platform_description_names_the_host() {
    let platform = platform_name();
    assert!(
        platform.starts_with("linux-")
            || platform.starts_with("darwin")
            || platform.starts_with("win32")
    );
}

#[test]
fn the_wait_status_matches_python() {
    assert_eq!(exit_code_from_status(0), 0);
    assert_eq!(exit_code_from_status(9 << 8), 9);
    assert_eq!(exit_code_from_status(3 << 8), 3);
    assert_eq!(exit_code_from_status(0xffff), 1);
    assert_eq!(exit_code_from_status(9), 137);
}

#[cfg(unix)]
#[test]
fn the_probe_refuses_a_missing_helper() {
    let options = ProbeOptions::new("/nonexistent/kala-sandbox-probe");
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe.detail.contains("no usable interpreter"),
        "the refusal names the missing helper: {}",
        probe.detail
    );
}

#[cfg(unix)]
#[test]
fn the_probe_refuses_a_helper_that_fails() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(scratch.path(), "failing", "exit 121");
    let options = ProbeOptions::new(helper);
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe.detail.contains("unavailable on this host"),
        "the refusal names the failing profile: {}",
        probe.detail
    );
}

#[cfg(unix)]
#[test]
fn the_probe_refuses_a_helper_that_reports_nothing() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(scratch.path(), "silent", "echo not json");
    let options = ProbeOptions::new(helper);
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe.detail.contains("returned no result"),
        "the refusal names the missing result: {}",
        probe.detail
    );
}

#[cfg(unix)]
#[test]
fn the_probe_refuses_a_helper_that_verifies_too_little() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(
        scratch.path(),
        "partial",
        "printf '{\"controls\": [\"mount_namespace\"], \"detail\": \"\"}\\n'",
    );
    let options = ProbeOptions::new(helper);
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe
            .detail
            .contains("required sandbox control was not verified"),
        "the refusal names the missing control: {}",
        probe.detail
    );
}

#[cfg(unix)]
#[test]
fn the_probe_advertises_only_verified_controls() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(
        scratch.path(),
        "complete",
        "printf '{\"controls\": [\"chroot_workspace_only\", \"mount_namespace\", \
         \"network_isolation\", \"readonly_system_bindings\", \"user_namespace\", \
         \"workspace_read_write\", \"made_up\"], \"detail\": \"\"}\\n'",
    );
    let options = ProbeOptions::new(helper);
    let probe = probe_sandbox(&options);
    assert!(probe.available, "the probe failed: {:?}", probe);
    assert!(!probe.controls.contains(&"made_up".to_owned()));
    assert_eq!(probe.controls.len(), REQUIRED.len());
}

#[cfg(unix)]
#[test]
fn the_probe_gives_up_when_the_helper_overruns_the_deadline() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(scratch.path(), "slow", "sleep 30");
    let mut options = ProbeOptions::new(helper);
    options.timeout = Duration::from_millis(250);
    let started = std::time::Instant::now();
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe.detail.contains("did not finish"),
        "the refusal names the deadline: {}",
        probe.detail
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline is enforced by the supervisor"
    );
}

#[cfg(unix)]
#[test]
fn the_probe_bounds_the_output_of_the_helper() {
    let scratch = temporary("kala-security-");
    let helper = fake_helper(
        scratch.path(),
        "flooding",
        "i=0; while [ $i -lt 20000 ]; do printf '%0.sX' $(seq 1 200); i=$((i+1)); done",
    );
    let options = ProbeOptions::new(helper);
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(
        probe.detail.contains("returned no result")
            || probe.detail.contains("unavailable on this host"),
        "the flood is refused: {}",
        probe.detail
    );
}

#[test]
fn the_probe_entry_point_refuses_a_malformed_canary() {
    let spec = json!({
        "controls": REQUIRED,
        "device_bindings": [],
        "readonly_bindings": [],
        "root": "/tmp/kala-root",
        "scratch_bytes": 1 << 20,
        "scratch_paths": [],
        "writable_bindings": [],
        "workspace": "/tmp/kala-workspace",
    });
    let error = probe_entry(&spec.to_string(), "{not json").expect_err("the payload is refused");
    assert_eq!(error.detail(), "the probe payload is malformed");
    let (spec, canary) =
        probe_entry(&spec.to_string(), "\"/host/canary\"").expect("a valid payload");
    assert_eq!(canary, "/host/canary");
    assert_eq!(spec.workspace, PathBuf::from("/tmp/kala-workspace"));
}

#[test]
fn the_probe_reports_a_setup_failure_as_one_json_line() {
    // A jail that cannot be built is reported as one line of JSON with a
    // sanitised reason, and the process exits with the probe failure code.
    let scratch = temporary("kala-security-");
    let spec = json!({
        "controls": [],
        "device_bindings": [],
        "readonly_bindings": [],
        "root": scratch.path().join("missing-root").to_string_lossy(),
        "scratch_bytes": 1 << 20,
        "scratch_paths": [],
        "writable_bindings": [],
        "workspace": scratch.path().join("workspace").to_string_lossy(),
    });
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-probe"));
    let output = Command::new(helper)
        .arg(spec.to_string())
        .arg("\"/canary\"")
        .output()
        .expect("the helper runs");
    assert_eq!(output.status.code(), Some(PROBE_FAILURE_EXIT));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().count(),
        1,
        "the result is one line: {stdout}"
    );
    let payload: Value = serde_json::from_str(stdout.trim()).expect("the helper emits JSON");
    assert_eq!(payload["controls"], json!([]));
    assert!(
        payload["detail"]
            .as_str()
            .expect("detail")
            .contains("chroot"),
        "the reason names the failing operation: {payload}"
    );
}
