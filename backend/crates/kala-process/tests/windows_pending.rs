//! Windows-only contract and concurrency tests.
//!
//! These tests are compiled for `x86_64-pc-windows-msvc` but marked ignored:
//! this repository has no Windows host, and no Windows runtime execution is
//! claimed. Run them on a Windows host with `cargo test -- --ignored` after
//! reviewing the Job Object behaviour.

#![cfg(windows)]

use std::path::PathBuf;
use std::time::Duration;

use kala_process::{
    CancelToken, ChildEnvironment, CompatibilityStatus, ExitStatus, ProcessEnd, ProcessLimits,
    ProcessSpec, SandboxSetupError, current_process_in_job, platform_name, supervise,
    windows_handle_count,
};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kala-process-fixture"))
}

fn workspace() -> (TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("kala-windows-test-")
        .tempdir()
        .expect("temporary directory");
    let path = std::fs::canonicalize(directory.path()).expect("canonical workspace");
    (directory, path)
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_platform_name_is_the_python_name() {
    assert_eq!(platform_name(), "win32");
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_job_context_is_queryable() {
    if let Some(in_job) = current_process_in_job() {
        eprintln!("current process in job: {in_job}");
    }
    assert!(windows_handle_count().is_some());
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_job_object_contains_the_child_tree() {
    let (_guard, workspace) = workspace();
    let marker = workspace.join("late-marker.txt");
    let process = ProcessSpec::new(fixture(), &workspace)
        .expect("fixture")
        .args(["linger", "1500", marker.to_string_lossy().as_ref()])
        .with_environment(ChildEnvironment::minimal(&workspace).expect("environment"))
        .with_limits(ProcessLimits {
            wall_timeout: Duration::from_secs(10),
            bootstrap_timeout: Duration::from_secs(5),
            termination_timeout: Duration::from_secs(5),
            ..ProcessLimits::default()
        });
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert!(outcome.cleanup.is_complete());
    std::thread::sleep(Duration::from_millis(1800));
    assert!(!marker.exists(), "a descendant escaped the Job Object");
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_timeout_and_cancellation_are_bounded() {
    let (_guard, workspace) = workspace();
    let process = ProcessSpec::new(fixture(), &workspace)
        .expect("fixture")
        .args(["sleep", "30000"])
        .with_limits(ProcessLimits {
            wall_timeout: Duration::from_millis(200),
            bootstrap_timeout: Duration::from_secs(5),
            termination_timeout: Duration::from_secs(2),
            ..ProcessLimits::default()
        });
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert!(matches!(
        outcome.end,
        ProcessEnd::TimedOut(kala_process::TimeoutStage::WallClock)
    ));
    assert!(outcome.cleanup.is_complete());

    let process = ProcessSpec::new(fixture(), &workspace)
        .expect("fixture")
        .args(["sleep", "30000"]);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        trigger.cancel();
    });
    let outcome = supervise(&process, &[], &cancel).expect("supervised");
    timer.join().expect("timer");
    assert!(matches!(
        outcome.end,
        ProcessEnd::Cancelled(kala_process::CancelStage::Running)
    ));
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_handles_do_not_grow_under_concurrency() {
    let (_guard, workspace) = workspace();
    let before = windows_handle_count().unwrap_or(0);
    let mut workers = Vec::new();
    for _ in 0..8 {
        let workspace = workspace.clone();
        workers.push(std::thread::spawn(move || {
            for _ in 0..4 {
                let process = ProcessSpec::new(fixture(), &workspace)
                    .expect("fixture")
                    .args(["echo", "parallel"]);
                let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
                assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
                assert_eq!(outcome.stdout.as_bytes(), b"parallel");
            }
        }));
    }
    for worker in workers {
        worker.join().expect("worker");
    }
    let after = windows_handle_count().unwrap_or(0);
    assert!(after <= before + 8, "handle count {before} -> {after}");
}

#[test]
#[ignore = "requires a Windows host; runtime proof is pending"]
fn windows_refuses_the_linux_strict_profile() {
    let (_guard, workspace) = workspace();
    let spec = kala_process::SandboxSpec {
        controls: kala_process::STRICT_REQUIRED_CONTROLS
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        root: workspace.join("root"),
        workspace: workspace.clone(),
        readonly_bindings: Vec::new(),
        writable_bindings: Vec::new(),
        device_bindings: Vec::new(),
        scratch_paths: Vec::new(),
        scratch_bytes: 1 << 20,
    };
    let process = ProcessSpec::new(fixture(), &workspace)
        .expect("fixture")
        .with_sandbox(spec);
    assert!(process.is_err());
    let _ = SandboxSetupError::new("no Windows strict profile");
    let _ = CompatibilityStatus::Failed;
}
