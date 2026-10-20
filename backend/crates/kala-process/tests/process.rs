use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kala_process::{
    CancelStage, CancelToken, ChildEnvironment, CompatibilityStatus, EnvironmentEntry,
    EnvironmentError, ExitStatus, OutputStream, ProcessEnd, ProcessError, ProcessLimits,
    ProcessSpec, TimeoutStage, WorkspaceError, build_child_environment, resolve_workspace_path,
    sanitize_child_stderr, supervise,
};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_kala-process-fixture"))
}

fn workspace() -> (TempDir, PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("kala-process-test-")
        .tempdir()
        .expect("temporary directory");
    let path = std::fs::canonicalize(directory.path()).expect("canonical workspace");
    (directory, path)
}

fn spec(workspace: &Path, arguments: &[&str]) -> ProcessSpec {
    ProcessSpec::new(fixture(), workspace)
        .expect("fixture specification")
        .args(arguments.iter().copied())
        .with_limits(ProcessLimits {
            wall_timeout: Duration::from_secs(10),
            bootstrap_timeout: Duration::from_secs(5),
            termination_timeout: Duration::from_secs(2),
            ..ProcessLimits::default()
        })
}

#[test]
fn success_reports_exit_status_and_stdout() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["echo", "hello"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert_eq!(outcome.stdout.as_bytes(), b"hello");
    assert!(outcome.stdout_str().is_ok());
    assert!(outcome.succeeded());
    assert!(outcome.cleanup.is_complete());
    assert!(outcome.process_id.is_some());
    assert_eq!(
        outcome.compatibility().status,
        CompatibilityStatus::Succeeded
    );
    assert_eq!(outcome.compatibility().error_code, None);
}

#[test]
fn exit_codes_are_reported_exactly() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["exit", "7"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(7)));
    assert!(!outcome.succeeded());
    let compatibility = outcome.compatibility();
    assert_eq!(compatibility.status, CompatibilityStatus::Failed);
    assert_eq!(compatibility.error_code, Some("tool_crashed"));
    assert!(compatibility.message.contains("status 7"));
}

#[cfg(unix)]
#[test]
fn signal_death_uses_python_popen_status_not_the_shell_status() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["self-signal", "9"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    let status = outcome.exit.expect("signal status");
    eprintln!(
        "signal regression: exit={:?} python={} shell={} compatibility={:?}",
        status,
        status.python_exit_code(),
        status.shell_exit_code(),
        outcome.compatibility()
    );
    assert_eq!(status, ExitStatus::signalled(9));
    assert_eq!(status.python_exit_code(), -9);
    assert_eq!(status.shell_exit_code(), 137);
    assert_eq!(outcome.end, ProcessEnd::Exited(status));
    assert!(!outcome.succeeded());
    let compatibility = outcome.compatibility();
    assert_eq!(compatibility.status, CompatibilityStatus::Failed);
    assert_eq!(compatibility.error_code, Some("tool_crashed"));
    assert!(
        compatibility.message.contains("status -9"),
        "unexpected compatibility message: {}",
        compatibility.message
    );
}

#[test]
fn interior_nul_arguments_are_refused_before_execution() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &[]).arg("bad\0argument");
    assert!(matches!(
        supervise(&process, &[], &CancelToken::new()),
        Err(ProcessError::InvalidSpec(_))
    ));
}

#[test]
fn argv_is_never_interpreted_by_a_shell() {
    let (_guard, workspace) = workspace();
    let process = spec(
        &workspace,
        &[
            "arg-echo",
            "; exit 9",
            "$(echo hacked)",
            "a b",
            "quote\"inside",
        ],
    );
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert_eq!(
        String::from_utf8_lossy(outcome.stdout.as_bytes()),
        "; exit 9\u{1f}$(echo hacked)\u{1f}a b\u{1f}quote\"inside"
    );
}

#[test]
fn wall_clock_timeout_kills_the_worker_and_returns_bounded() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["sleep", "30000"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_millis(150),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_secs(2),
        ..ProcessLimits::default()
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::TimedOut(TimeoutStage::WallClock));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(outcome.cleanup.is_complete());
    assert!(outcome.exit.is_some());
    let compatibility = outcome.compatibility();
    assert_eq!(compatibility.status, CompatibilityStatus::TimedOut);
    assert_eq!(compatibility.error_code, Some("timeout"));
    let pid = outcome.process_id.expect("pid");
    assert!(!pid_exists(pid));
}

#[test]
fn cancellation_before_launch_never_spawns() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["echo", "never"]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let outcome = supervise(&process, &[], &cancel).expect("supervised");
    assert_eq!(
        outcome.end,
        ProcessEnd::Cancelled(CancelStage::BeforeLaunch)
    );
    assert_eq!(outcome.process_id, None);
    assert!(outcome.cleanup.is_complete());
    assert_eq!(
        outcome.compatibility().status,
        CompatibilityStatus::Cancelled
    );
    assert_eq!(outcome.compatibility().error_code, None);
}

#[test]
fn cancellation_during_execution_is_observed() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["sleep", "30000"]);
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(120));
        trigger.cancel();
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &cancel).expect("supervised");
    timer.join().expect("timer");
    assert_eq!(outcome.end, ProcessEnd::Cancelled(CancelStage::Running));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(outcome.cleanup.is_complete());
    assert_eq!(
        outcome.compatibility().status,
        CompatibilityStatus::Cancelled
    );
    assert_eq!(outcome.compatibility().error_code, None);
}

#[test]
fn stdout_flood_is_bounded_and_maps_to_output_too_large() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["flood", "4194304", "stdout"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(10),
        max_stdout_bytes: 4_096,
        ..ProcessLimits::default()
    });
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(
        outcome.end,
        ProcessEnd::OutputLimitExceeded(OutputStream::Stdout)
    );
    assert_eq!(outcome.stdout.len(), 4_096);
    assert!(outcome.stdout.truncated());
    assert!(outcome.stdout.discarded_bytes() > 0);
    assert!(outcome.cleanup.is_complete());
    let compatibility = outcome.compatibility();
    assert_eq!(compatibility.status, CompatibilityStatus::Failed);
    assert_eq!(compatibility.error_code, Some("output_too_large"));
}

#[test]
fn simultaneous_stdout_and_stderr_floods_do_not_deadlock() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["flood", "2097152", "both"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(10),
        max_stdout_bytes: 2_048,
        max_stderr_bytes: 2_048,
        ..ProcessLimits::default()
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        outcome.end,
        ProcessEnd::OutputLimitExceeded(OutputStream::Stdout)
    );
    assert!(outcome.stdout.truncated());
    assert!(outcome.stderr.truncated());
    assert!(outcome.cleanup.is_complete());
}

#[test]
fn a_continuously_ready_output_producer_cannot_starve_the_wall_deadline() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["flood-forever"]).with_limits(ProcessLimits {
        // Short enough that the maximum legal capture cap is not reached
        // before the wall deadline; the point is that the deadline is still
        // observed while output is continuously ready.
        wall_timeout: Duration::from_millis(5),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_secs(2),
        max_stdout_bytes: 64 * 1_048_576,
        ..ProcessLimits::default()
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::TimedOut(TimeoutStage::WallClock));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "continuous output delayed the deadline decision"
    );
    assert!(outcome.cleanup.is_complete());
}

#[test]
fn stderr_flood_truncates_without_failing_a_successful_exit() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["flood", "1048576", "stderr"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(10),
        max_stderr_bytes: 1_024,
        ..ProcessLimits::default()
    });
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert!(outcome.stderr.truncated());
    assert_eq!(outcome.stderr.len(), 1_024);
    assert!(outcome.succeeded());
}

#[test]
fn invalid_utf8_stdout_is_preserved_and_rejected_by_the_utf8_view() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["invalid-utf8"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(
        outcome.stdout.as_bytes(),
        &[0xff, 0xfe, 0x00, b'b', b'a', b'd']
    );
    assert!(outcome.stdout_str().is_err());
    assert!(outcome.stdout.to_string_lossy().contains('\u{fffd}'));
}

#[test]
fn stderr_is_sanitized_and_workspace_paths_are_redacted() {
    let (_guard, workspace) = workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let process = spec(&workspace, &["stderr-lines", workspace_text.as_str()]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    let summary = outcome.stderr_summary();
    assert!(summary.contains("first line"));
    assert!(summary.contains("..."));
    assert!(summary.contains("last line"));
    assert!(!summary.contains(&workspace.to_string_lossy().to_string()));
    assert!(summary.contains("<workspace>"));
    assert!(!summary.contains('\n'));

    let process = spec(&workspace, &["binary-stderr"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(
        sanitize_child_stderr(outcome.stderr.as_bytes(), None),
        "binary\u{fffd}"
    );
    assert_eq!(sanitize_child_stderr(b"", None), "");
    assert_eq!(sanitize_child_stderr(b"\x00\x01only", None), "only");
    assert_eq!(
        sanitize_child_stderr(b"first\nsecond", None),
        "first | second"
    );
}

#[test]
fn stdin_is_delivered_exactly() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["echo-stdin"]);
    let payload = b"hello\x00world\xff\n";
    let outcome = supervise(&process, payload, &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert_eq!(outcome.stdout.as_bytes(), payload);
}

#[test]
fn stdin_backpressure_is_bounded_by_the_wall_clock() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["stdin-hold", "30000"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_millis(200),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_secs(2),
        max_stdin_bytes: 1_048_576,
        ..ProcessLimits::default()
    });
    let payload = vec![b'x'; 1_048_576];
    let started = Instant::now();
    let outcome = supervise(&process, &payload, &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::TimedOut(TimeoutStage::WallClock));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(outcome.cleanup.is_complete());
}

#[test]
fn pending_stdin_is_bounded_by_the_bootstrap_deadline_not_only_the_handshake() {
    let (_guard, workspace) = workspace();
    // This is the independent-review reproduction: the worker runs but never
    // reads stdin, the bootstrap deadline is much shorter than the wall
    // deadline, and the payload cannot fit in the pipe.
    let process = spec(&workspace, &["stdin-hold", "30000"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_millis(600),
        bootstrap_timeout: Duration::from_millis(50),
        termination_timeout: Duration::from_secs(2),
        max_stdin_bytes: 1_048_576,
        ..ProcessLimits::default()
    });
    let payload = vec![b'x'; 1_048_576];
    let started = Instant::now();
    let outcome = supervise(&process, &payload, &CancelToken::new()).expect("supervised");
    eprintln!(
        "bootstrap-stdin regression: end={:?} elapsed={:?} cleanup={:?}",
        outcome.end, outcome.elapsed, outcome.cleanup
    );
    assert_eq!(
        outcome.end,
        ProcessEnd::TimedOut(TimeoutStage::Bootstrap),
        "pending stdin must expire with the bootstrap stage"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the bootstrap deadline was not enforced while stdin was pending"
    );
    assert!(outcome.cleanup.is_complete());
}

#[test]
fn a_lingering_descendant_holding_pipes_is_killed() {
    let (_guard, workspace) = workspace();
    let marker = workspace.join("late-marker.txt");
    let marker_text = marker.to_string_lossy().into_owned();
    let process = spec(&workspace, &["linger", "1500", marker_text.as_str()]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert!(outcome.cleanup.is_complete());
    std::thread::sleep(Duration::from_millis(1_800));
    assert!(!marker.exists(), "a descendant escaped tree cleanup");
}

#[test]
fn a_timed_out_worker_leaves_no_grandchild_side_effect() {
    let (_guard, workspace) = workspace();
    let marker = workspace.join("late-timeout-marker.txt");
    let marker_text = marker.to_string_lossy().into_owned();
    let process = spec(
        &workspace,
        &["spawn-holder", "1200", marker_text.as_str(), "30000"],
    )
    .with_limits(ProcessLimits {
        wall_timeout: Duration::from_millis(150),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_secs(2),
        ..ProcessLimits::default()
    });
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::TimedOut(TimeoutStage::WallClock));
    assert!(outcome.cleanup.is_complete());
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(!marker.exists(), "a grandchild survived the timeout");
}

#[cfg(unix)]
#[test]
fn concurrency_does_not_leak_descriptors() {
    let (_guard, workspace) = workspace();
    let before = descriptor_count();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let workspace = workspace.clone();
        workers.push(std::thread::spawn(move || {
            for _ in 0..4 {
                let process = spec(&workspace, &["echo", "parallel"]);
                let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
                assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
                assert_eq!(outcome.stdout.as_bytes(), b"parallel");
            }
        }));
    }
    for worker in workers {
        worker.join().expect("worker");
    }
    let after = descriptor_count();
    // Other tests in this binary run in parallel and may open short-lived
    // pipes. A real per-worker leak in this workload would add 64 descriptors;
    // this tolerance still catches that while ignoring harness noise.
    assert!(
        after <= before + 16,
        "descriptor count grew from {before} to {after}"
    );
}

#[cfg(unix)]
fn descriptor_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn workspace_paths_are_confined() {
    let (guard, workspace) = workspace();
    std::fs::create_dir(workspace.join("real")).expect("real directory");
    std::fs::write(workspace.join("real/nested.txt"), b"value").expect("nested");
    let outside = guard.path().join("outside.txt");
    std::fs::write(&outside, b"secret").expect("outside");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, workspace.join("link-outside")).expect("symlink");
        std::os::unix::fs::symlink(workspace.join("real"), workspace.join("real/link-inside"))
            .expect("nested symlink");
    }
    assert_eq!(
        resolve_workspace_path(&workspace, "real/nested.txt").expect("resolve"),
        workspace.join("real/nested.txt")
    );
    let long = "x".repeat(2_048);
    for candidate in [
        "../outside.txt",
        "real/../../outside.txt",
        "/etc/passwd",
        "..",
        ".",
        "",
        "back\\slash.txt",
        "null\0byte.txt",
        long.as_str(),
    ] {
        assert!(
            matches!(
                resolve_workspace_path(&workspace, candidate),
                Err(WorkspaceError::Empty
                    | WorkspaceError::NullOrBackslash
                    | WorkspaceError::TooLong
                    | WorkspaceError::Absolute
                    | WorkspaceError::SegmentCount
                    | WorkspaceError::UnsafeSegment)
            ),
            "candidate {candidate:?} should be refused"
        );
    }
    #[cfg(unix)]
    {
        assert_eq!(
            resolve_workspace_path(&workspace, "link-outside"),
            Err(WorkspaceError::Symlink)
        );
        assert_eq!(
            resolve_workspace_path(&workspace, "real/link-inside"),
            Err(WorkspaceError::Symlink)
        );
    }
}

#[test]
fn protected_environment_keys_cannot_be_overridden() {
    let (_guard, workspace) = workspace();
    let base = ChildEnvironment::minimal(&workspace).expect("minimal environment");
    for key in [
        "HOME",
        "PATH",
        "TMPDIR",
        "KALA_PROCESS_PROTOCOL",
        "PYTHONPATH",
    ] {
        assert!(matches!(
            base.clone()
                .with_extra(&[(key.to_owned(), "/etc".to_owned())]),
            Err(EnvironmentError::ProtectedKey(_))
        ));
    }
    assert!(matches!(
        base.clone()
            .with_extra(&[("BAD KEY".to_owned(), "1".to_owned())]),
        Err(EnvironmentError::InvalidKey)
    ));
    assert!(matches!(
        base.with_extra(&[("TZ".to_owned(), "Europe/\0".to_owned())]),
        Err(EnvironmentError::NullValue)
    ));
}

#[test]
fn malformed_environment_entries_are_refused_at_the_aggregate_boundary() {
    // `EnvironmentEntry` exposes public fields, so direct construction must not
    // bypass the key/value rules enforced by the aggregate constructor.
    assert_eq!(
        ChildEnvironment::from_entries([EnvironmentEntry {
            key: "BAD=KEY".to_owned(),
            value: "value".to_owned(),
        }]),
        Err(EnvironmentError::InvalidKey)
    );
    assert_eq!(
        ChildEnvironment::from_entries([EnvironmentEntry {
            key: "GOOD_KEY".to_owned(),
            value: "bad\0value".to_owned(),
        }]),
        Err(EnvironmentError::NullValue)
    );
    assert_eq!(
        ChildEnvironment::from_entries([EnvironmentEntry {
            key: "BAD\0KEY".to_owned(),
            value: "value".to_owned(),
        }]),
        Err(EnvironmentError::InvalidKey)
    );
    assert!(
        ChildEnvironment::from_entries([EnvironmentEntry {
            key: "GOOD_KEY".to_owned(),
            value: "value".to_owned(),
        }])
        .is_ok()
    );
}

#[test]
fn the_legacy_child_environment_matches_the_python_key_set() {
    let (_guard, workspace) = workspace();
    let environment =
        build_child_environment(&workspace, std::slice::from_ref(&workspace), &[]).expect("built");
    let mut keys: Vec<&str> = environment
        .entries()
        .iter()
        .map(|entry| entry.key.as_str())
        .collect();
    keys.sort_unstable();
    let mut expected = vec![
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
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(
        environment.get("HOME"),
        Some(workspace.to_string_lossy().as_ref())
    );
    assert_eq!(
        environment.get("TMPDIR"),
        Some(workspace.to_string_lossy().as_ref())
    );
    assert!(matches!(
        build_child_environment(&workspace, &[], &[("HOME".to_owned(), "/etc".to_owned())]),
        Err(EnvironmentError::ProtectedKey(_))
    ));
}

#[test]
fn invalid_process_specs_are_refused_before_launch() {
    let (_guard, workspace) = workspace();
    assert!(matches!(
        ProcessSpec::new("relative-program", &workspace),
        Err(ProcessError::InvalidSpec(_))
    ));
    assert!(matches!(
        ProcessSpec::new(workspace.join("absent"), &workspace),
        Err(ProcessError::Spawn { .. })
    ));
    let process = spec(&workspace, &["echo", "x"]);
    assert!(matches!(
        supervise(&process, &vec![0u8; 2_000_000], &CancelToken::new()),
        Err(ProcessError::InvalidSpec(_))
    ));
}

#[test]
fn empty_stdin_is_closed_immediately() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["echo-stdin"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert!(outcome.stdout.is_empty());
}

#[cfg(unix)]
fn pid_exists(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
fn pid_exists(_pid: u32) -> bool {
    false
}

#[test]
fn exit_121_maps_to_sandbox_unavailable() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["exit", "121"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(121)));
    let compatibility = outcome.compatibility();
    assert_eq!(compatibility.status, CompatibilityStatus::Failed);
    assert_eq!(compatibility.error_code, Some("sandbox_unavailable"));
}

#[test]
fn the_child_working_directory_is_the_explicit_directory() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["cwd"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    assert_eq!(
        std::fs::canonicalize(String::from_utf8_lossy(outcome.stdout.as_bytes()).as_ref())
            .expect("reported cwd exists"),
        workspace
    );
}

#[test]
fn the_child_environment_contains_only_the_constructed_keys() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["env"]);
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    let text = String::from_utf8_lossy(outcome.stdout.as_bytes());
    let keys: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once('=').map(|(key, _)| key))
        .collect();
    assert!(keys.contains(&"HOME"));
    assert!(keys.contains(&"TMPDIR"));
    assert!(!keys.contains(&"SECRET_TOKEN"));
    assert!(!keys.contains(&"LD_PRELOAD"));
}

#[cfg(unix)]
#[test]
fn an_escaped_pipe_writer_that_is_always_ready_cannot_monopolize_cleanup() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["escapee-flood"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(10),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_millis(5),
        max_stdout_bytes: 64 * 1_048_576,
        ..ProcessLimits::default()
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a continuously ready escaped writer delayed cleanup past the bound"
    );
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    // The escaped writer is outside the process-group boundary. Cleanup is
    // bounded and reported honestly instead of waiting for EOF forever.
    assert!(!outcome.cleanup.pipes_drained);
    assert!(!outcome.cleanup.is_complete());
    assert!(!outcome.succeeded());
    assert_eq!(outcome.compatibility().error_code, Some("tool_crashed"));
}

#[cfg(unix)]
#[test]
fn a_descendant_that_escapes_the_process_group_is_reported_not_hidden() {
    let (_guard, workspace) = workspace();
    let marker = workspace.join("escaped-marker.txt");
    let marker_text = marker.to_string_lossy().into_owned();
    let process =
        spec(&workspace, &["escapee", "500", marker_text.as_str()]).with_limits(ProcessLimits {
            wall_timeout: Duration::from_secs(10),
            bootstrap_timeout: Duration::from_secs(5),
            termination_timeout: Duration::from_millis(250),
            ..ProcessLimits::default()
        });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(outcome.end, ProcessEnd::Exited(ExitStatus::exited(0)));
    // The escaped descendant is outside the documented Unix process-group
    // boundary, so cleanup is honestly incomplete and the compatibility
    // mapping refuses to report success.
    assert!(!outcome.cleanup.pipes_drained);
    assert!(!outcome.cleanup.is_complete());
    assert_eq!(outcome.compatibility().error_code, Some("tool_crashed"));
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        marker.exists(),
        "the deliberately escaped descendant should have written"
    );
}

#[test]
fn the_bootstrap_deadline_bounds_the_exec_handshake() {
    let (_guard, workspace) = workspace();
    let process = spec(&workspace, &["echo", "should-not-run"]).with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(10),
        bootstrap_timeout: Duration::from_nanos(1),
        termination_timeout: Duration::from_secs(2),
        ..ProcessLimits::default()
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &CancelToken::new()).expect("supervised");
    assert_eq!(outcome.end, ProcessEnd::TimedOut(TimeoutStage::Bootstrap));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(outcome.cleanup.is_complete());
    assert_eq!(outcome.compatibility().error_code, Some("timeout"));
}

#[test]
fn cancellation_during_a_slow_launch_is_observed_before_the_worker_runs() {
    let (_guard, workspace) = workspace();
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-process-probe"));
    let fixture = fixture();
    let fixture_root = fixture.parent().expect("fixture parent").to_path_buf();
    let probe = kala_process::probe_sandbox(&kala_process::SandboxProbeOptions {
        helper,
        worker_roots: vec![fixture_root.clone()],
        scratch_bytes: 1 << 20,
        timeout: Duration::from_secs(30),
    });
    if !probe.available {
        eprintln!("strict launch-race test skipped: {}", probe.detail);
        return;
    }
    let sandbox_root = workspace.join("slow-root");
    let sandbox_workspace = workspace.join("slow-workspace");
    std::fs::create_dir(&sandbox_workspace).expect("workspace");
    let source = workspace.join("slow-source");
    std::fs::create_dir(&source).expect("source");
    let mut spec = kala_process::build_sandbox_spec(
        &probe.controls,
        &sandbox_root,
        &sandbox_workspace,
        &[fixture_root],
        1 << 20,
    )
    .expect("sandbox spec");
    for index in 0..2_000 {
        let target = workspace.join("slow").join(format!("{index:04}"));
        std::fs::create_dir_all(&target).expect("slow bind source");
        spec.readonly_bindings
            .push(target.to_string_lossy().into_owned());
    }
    let process = ProcessSpec::new(&fixture, &sandbox_workspace)
        .expect("fixture")
        .with_environment(ChildEnvironment::minimal(&sandbox_workspace).expect("environment"))
        .with_sandbox(spec)
        .expect("sandboxed process");
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2));
        trigger.cancel();
    });
    let started = Instant::now();
    let outcome = supervise(&process, &[], &cancel).expect("supervised");
    timer.join().expect("timer");
    assert!(started.elapsed() < Duration::from_secs(10));
    eprintln!("slow-launch cancellation stage: {:?}", outcome.end);
    assert!(
        matches!(
            outcome.end,
            ProcessEnd::Cancelled(CancelStage::Bootstrap | CancelStage::BeforeLaunch)
        ),
        "unexpected launch outcome: {:?}",
        outcome.end
    );
    assert!(outcome.cleanup.direct_reaped);
    assert!(outcome.cleanup.tree_empty);
}
