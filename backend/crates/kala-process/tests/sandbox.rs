use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use kala_process::{
    IsolationProfile as _IsolationProfile, ProcessLimits, STRICT_REQUIRED_CONTROLS, SandboxProbe,
    SandboxProbeOptions, SandboxSpec, SandboxVerdict, judge_sandbox, portable_controls,
    probe_sandbox, unshadowed_scratch_paths,
};
use serde_json::json;
use tempfile::TempDir;

#[cfg(target_os = "linux")]
use kala_process::build_sandbox_spec;

fn controls() -> Vec<String> {
    STRICT_REQUIRED_CONTROLS
        .iter()
        .map(|item| (*item).to_owned())
        .collect()
}

#[test]
fn probe_and_spec_payloads_have_the_python_shape() {
    let probe = SandboxProbe {
        available: true,
        platform: "linux-x86_64".to_owned(),
        controls: vec!["mount_namespace".to_owned(), "user_namespace".to_owned()],
        detail: "fine".to_owned(),
        elapsed_ms: 1.23456,
    };
    let payload = probe.to_payload();
    assert_eq!(payload["elapsed_ms"], json!(1.235));
    assert_eq!(
        payload["controls"],
        json!(["mount_namespace", "user_namespace"])
    );
    let decoded = SandboxProbe::from_payload(&payload).expect("decoded");
    assert_eq!(decoded.to_payload(), payload);

    let spec = SandboxSpec {
        controls: controls().into_iter().collect::<BTreeSet<_>>(),
        root: PathBuf::from("/root"),
        workspace: PathBuf::from("/workspace"),
        readonly_bindings: vec!["/usr".to_owned()],
        writable_bindings: vec!["/workspace".to_owned()],
        device_bindings: vec!["/dev/null".to_owned()],
        scratch_paths: vec!["/tmp".to_owned()],
        scratch_bytes: 1 << 20,
    };
    let payload = spec.to_payload();
    assert_eq!(payload["controls"], json!(controls()));
    assert_eq!(payload["root"], "/root");
    assert_eq!(payload["scratch_bytes"], 1 << 20);
    assert_eq!(SandboxSpec::from_payload(&payload).expect("decoded"), spec);

    let malformed = json!({"controls": []});
    assert!(SandboxSpec::from_payload(&malformed).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn strict_specs_require_all_required_controls() {
    let (_guard, workspace) = temp_workspace();
    let root = workspace.join("root");
    let mut missing = controls();
    missing.retain(|control| control != "user_namespace");
    let error =
        build_sandbox_spec(&missing, &root, &workspace, &[], 1 << 20).expect_err("missing control");
    assert!(error.detail().contains("user_namespace"));

    let mut relative = controls();
    relative.push("procfs".to_owned());
    let relative_root = workspace.join("relative-root");
    let spec = build_sandbox_spec(&relative, &relative_root, &workspace, &[], 1 << 20)
        .expect("absolute paths");
    assert!(spec.has_required_controls());
    assert!(
        spec.writable_bindings
            .contains(&workspace.to_string_lossy().into_owned())
    );

    let mut pid = controls();
    pid.push("pid_namespace".to_owned());
    let spec = SandboxSpec {
        controls: pid.into_iter().collect(),
        root,
        workspace,
        readonly_bindings: Vec::new(),
        writable_bindings: Vec::new(),
        device_bindings: Vec::new(),
        scratch_paths: Vec::new(),
        scratch_bytes: 1 << 20,
    };
    let error = spec
        .validate_strict()
        .expect_err("pid namespace is pending");
    assert!(error.detail().contains("pid_namespace"));
}

#[test]
fn scratch_paths_shadowed_by_bindings_are_dropped() {
    let spec = SandboxSpec {
        controls: controls().into_iter().collect(),
        root: PathBuf::from("/root"),
        workspace: PathBuf::from("/ws"),
        readonly_bindings: vec!["/tmp".to_owned()],
        writable_bindings: vec!["/tmp/work".to_owned(), "/var".to_owned()],
        device_bindings: Vec::new(),
        scratch_paths: vec!["/dev/shm".to_owned(), "/tmp".to_owned(), "/var".to_owned()],
        scratch_bytes: 1 << 20,
    };
    assert_eq!(unshadowed_scratch_paths(&spec), vec!["/dev/shm"]);
}

#[test]
fn judging_never_advertises_an_unprobed_strict_profile() {
    let portable = judge_sandbox(_IsolationProfile::Portable, None);
    assert!(portable.allowed);
    assert!(portable.controls.is_empty());
    assert!(portable.reason.contains("confines a process"));

    let absent = judge_sandbox(_IsolationProfile::StrictLinux, None);
    assert!(!absent.allowed);
    assert!(absent.reason.ends_with("refused"));

    let unavailable = SandboxProbe {
        available: false,
        platform: "linux-x86_64".to_owned(),
        controls: Vec::new(),
        detail: "namespaces denied".to_owned(),
        elapsed_ms: 0.0,
    };
    let refused = judge_sandbox(_IsolationProfile::StrictLinux, Some(&unavailable));
    assert!(!refused.allowed);
    assert!(refused.reason.ends_with("refused: namespaces denied"));

    let available = SandboxProbe {
        available: true,
        platform: "linux-x86_64".to_owned(),
        controls: vec!["mount_namespace".to_owned(), "user_namespace".to_owned()],
        detail: String::new(),
        elapsed_ms: 1.0,
    };
    let allowed = judge_sandbox(_IsolationProfile::StrictLinux, Some(&available));
    assert!(allowed.allowed);
    assert_eq!(allowed.controls, available.controls);
    assert_eq!(
        allowed.to_payload(),
        json!({"allowed": true, "controls": ["mount_namespace", "user_namespace"], "reason": ""})
    );
}

#[test]
fn verdict_payload_is_stable() {
    let payload = SandboxVerdict {
        allowed: false,
        reason: "no".to_owned(),
        controls: vec!["a".to_owned()],
    }
    .to_payload();
    assert_eq!(
        payload,
        json!({"allowed": false, "controls": ["a"], "reason": "no"})
    );
}

#[test]
fn portable_controls_name_only_enforced_behaviour() {
    let limits = ProcessLimits::default();
    let controls = portable_controls(&limits);
    assert!(controls.contains(&"cwd_confined"));
    assert!(controls.contains(&"environment_minimal"));
    assert!(controls.contains(&"wall_clock_timeout"));
    assert!(controls.contains(&"output_size_limit"));
    assert!(!controls.contains(&"chroot_workspace_only"));
    assert!(!controls.contains(&"worker_concurrency_cap"));
    assert!(!controls.contains(&"artifact_digest_verification"));
    #[cfg(unix)]
    {
        assert!(controls.contains(&"file_descriptor_close"));
        assert!(controls.contains(&"process_group_kill"));
        assert!(controls.contains(&"resource_limits"));
    }
    #[cfg(windows)]
    {
        assert!(controls.contains(&"job_object_containment"));
        assert!(controls.contains(&"explicit_handle_inheritance"));
        assert!(!controls.contains(&"process_group_kill"));
        assert!(!controls.contains(&"resource_limits"));
    }
    let mut sorted = controls.clone();
    sorted.sort_unstable();
    assert_eq!(controls, sorted);
}

#[test]
fn probe_reports_truthfully_and_an_opt_in_strict_run_fails_closed() {
    let (_guard, workspace) = temp_workspace();
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-process-probe"));
    let options = SandboxProbeOptions {
        helper,
        worker_roots: vec![workspace.clone()],
        scratch_bytes: 1 << 20,
        timeout: Duration::from_secs(30),
    };
    let probe = probe_sandbox(&options);
    eprintln!("strict probe payload: {}", probe.to_payload());
    assert_eq!(probe.platform, kala_process::platform_name());
    if probe.available {
        for control in STRICT_REQUIRED_CONTROLS {
            assert!(
                probe.controls.iter().any(|value| value == control),
                "required control {control} was not verified"
            );
        }
        assert!(probe.detail.is_empty());
        let verdict = judge_sandbox(_IsolationProfile::StrictLinux, Some(&probe));
        assert!(verdict.allowed);
    } else {
        assert!(probe.controls.is_empty());
        assert!(!probe.detail.is_empty());
        let verdict = judge_sandbox(_IsolationProfile::StrictLinux, Some(&probe));
        assert!(!verdict.allowed);
        assert!(probe.elapsed_ms >= 0.0);
    }
    if std::env::var_os("KALA_STRICT_SANDBOX").is_some() && !probe.available {
        panic!(
            "KALA_STRICT_SANDBOX was requested but the strict probe is unavailable: {}",
            probe.detail
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_available_strict_jail_actually_contains_a_native_worker() {
    let (_guard, workspace) = temp_workspace();
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-process-probe"));
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_kala-process-fixture"));
    let fixture_root = fixture.parent().expect("fixture parent").to_path_buf();
    let options = SandboxProbeOptions {
        helper,
        worker_roots: vec![fixture_root.clone(), workspace.clone()],
        scratch_bytes: 1 << 20,
        timeout: Duration::from_secs(30),
    };
    let probe = probe_sandbox(&options);
    if !probe.available {
        eprintln!(
            "strict namespace execution unavailable on this host: {}",
            probe.detail
        );
        return;
    }
    let sandbox_root = workspace.join("jail-root");
    let sandbox_workspace = workspace.join("jail-workspace");
    std::fs::create_dir(&sandbox_workspace).expect("jail workspace");
    let canary = workspace.join("outside-canary");
    std::fs::write(&canary, b"kala-sandbox-canary").expect("canary");
    let spec = build_sandbox_spec(
        &probe.controls,
        &sandbox_root,
        &sandbox_workspace,
        &[fixture_root],
        1 << 20,
    )
    .expect("strict specification");
    let run = |arguments: &[&str]| {
        let process = kala_process::ProcessSpec::new(&fixture, &sandbox_workspace)
            .expect("fixture")
            .args(arguments.iter().copied())
            .with_sandbox(spec.clone())
            .expect("sandbox");
        kala_process::supervise(&process, &[], &kala_process::CancelToken::new())
            .expect("supervised")
    };

    let inside = run(&["write", "inside.txt", "hello"]);
    assert_eq!(
        inside.end,
        kala_process::ProcessEnd::Exited(kala_process::ExitStatus::exited(0))
    );
    assert!(sandbox_workspace.join("inside.txt").is_file());
    assert!(inside.cleanup.is_complete());

    let canary_result = run(&["read", canary.to_string_lossy().as_ref()]);
    assert_eq!(
        canary_result.end,
        kala_process::ProcessEnd::Exited(kala_process::ExitStatus::exited(42)),
        "the host canary was visible inside the jail"
    );

    let network = run(&["connect"]);
    assert_eq!(
        network.end,
        kala_process::ProcessEnd::Exited(kala_process::ExitStatus::exited(43)),
        "the worker could reach the external network"
    );

    let readonly = run(&["write", "/usr/kala-process-probe", "nope"]);
    assert_eq!(
        readonly.end,
        kala_process::ProcessEnd::Exited(kala_process::ExitStatus::exited(7)),
        "the read-only system binding was writable"
    );
}

fn temp_workspace() -> (TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("kala-sandbox-test-")
        .tempdir()
        .expect("temporary directory");
    let path = std::fs::canonicalize(directory.path()).expect("canonical workspace");
    (directory, path)
}

#[test]
fn a_missing_probe_helper_is_reported_truthfully() {
    let (_guard, workspace) = temp_workspace();
    let mut options = SandboxProbeOptions::new(workspace.join("absent-probe"));
    options.worker_roots = vec![workspace];
    let probe = probe_sandbox(&options);
    assert!(!probe.available);
    assert!(probe.controls.is_empty());
    assert!(probe.detail.contains("probe helper"));
    assert!(!judge_sandbox(_IsolationProfile::StrictLinux, Some(&probe)).allowed);
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_preparation_fails_before_the_worker_is_launched() {
    let (_guard, workspace) = temp_workspace();
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_kala-process-fixture"));
    let root = workspace.join("missing-bind-root");
    let missing = workspace.join("not-here");
    let spec = SandboxSpec {
        controls: controls().into_iter().collect(),
        root,
        workspace: workspace.clone(),
        readonly_bindings: vec![missing.to_string_lossy().into_owned()],
        writable_bindings: vec![workspace.to_string_lossy().into_owned()],
        device_bindings: Vec::new(),
        scratch_paths: Vec::new(),
        scratch_bytes: 1 << 20,
    };
    let process = kala_process::ProcessSpec::new(&fixture, &workspace)
        .expect("fixture")
        .with_sandbox(spec)
        .expect("valid specification");
    let error = kala_process::supervise(&process, &[], &kala_process::CancelToken::new())
        .expect_err("preparation must fail");
    assert!(matches!(error, kala_process::ProcessError::SandboxSetup(_)));
}
