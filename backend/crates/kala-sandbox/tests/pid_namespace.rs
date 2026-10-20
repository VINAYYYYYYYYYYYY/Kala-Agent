//! The production worker jail: the PID namespace, the exit status, the
//! deadline, the cancellation and the absence of orphans.
//!
//! The production path launches the native init helper through the accepted
//! `kala-process` supervisor. The helper builds the jail and runs the worker
//! inside it, so these cases exercise the *real* containment boundary: the
//! worker is a child of the namespace init, the init is PID 1 of the new
//! namespace, and the whole namespace dies with the jail.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use kala_process::{CancelToken, ProcessEnd, ProcessLimits};
use kala_sandbox::{
    PROBE_TIMEOUT_SECONDS, ProbeOptions, STRICT_OPTIONAL_CONTROLS, STRICT_REQUIRED_CONTROLS,
    SandboxSpec, WorkerCommand, WorkerJail, build_sandbox_spec, probe_sandbox,
};

/// The controls the strict profile may establish.
fn every_control() -> BTreeSet<String> {
    STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .map(|control| (*control).to_owned())
        .collect()
}

/// The controls of a jail without the PID namespace.
fn without_pid_namespace() -> BTreeSet<String> {
    STRICT_REQUIRED_CONTROLS
        .iter()
        .chain(STRICT_OPTIONAL_CONTROLS.iter())
        .filter(|control| **control != "pid_namespace")
        .map(|control| (*control).to_owned())
        .collect()
}

/// A temporary jail root and workspace.
struct Fixture {
    /// The scratch directory, kept alive for the lifetime of the fixture.
    _scratch: tempfile::TempDir,
    spec: SandboxSpec,
}

impl Fixture {
    fn new(controls: BTreeSet<String>) -> Self {
        let scratch = tempfile::Builder::new()
            .prefix("kala-pid-")
            .tempdir()
            .expect("a temporary directory");
        let root = scratch.path().join("root");
        let workspace = scratch.path().join("workspace");
        std::fs::create_dir_all(&root).expect("the jail root");
        std::fs::create_dir_all(&workspace).expect("the workspace");
        let spec = build_sandbox_spec(&controls, &root, &workspace, &[], 1 << 20)
            .expect("a valid specification");
        Self {
            _scratch: scratch,
            spec,
        }
    }

    fn jail(&self, command: WorkerCommand) -> WorkerJail {
        WorkerJail {
            init: PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-init")),
            command,
            spec: self.spec.clone(),
            environment: None,
            limits: None,
        }
    }
}

/// A marker that is unique to one test run.
fn marker(label: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("kala-pid-marker-{label}-{}-{counter}", std::process::id())
}

/// The host processes whose command line carries the marker.
fn processes_with_marker(marker: &str) -> Vec<u32> {
    let mut found: Vec<u32> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        let Ok(command) = std::fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let text = String::from_utf8_lossy(&command);
        if text.contains(marker) {
            found.push(name.parse().expect("a process id"));
        }
    }
    found.sort_unstable();
    found
}

/// Wait until no process carries the marker any more.
fn wait_until_no_process(marker: &str) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found = processes_with_marker(marker);
    while !found.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        found = processes_with_marker(marker);
    }
    found
}

/// A shell command that runs inside the jail.
fn shell(script: &str, marker: &str) -> WorkerCommand {
    WorkerCommand::new(
        "/bin/sh",
        vec![
            OsString::from("-c"),
            OsString::from(script),
            OsString::from(marker),
        ],
    )
}

#[test]
fn the_production_worker_runs_in_a_pid_namespace() {
    let fixture = Fixture::new(every_control());
    let marker = marker("visible");
    let script = "read -r pid comm state ppid rest < /proc/self/stat; \
                  echo \"pid=$pid ppid=$ppid\"; \
                  for entry in /proc/[0-9]*; do echo \"entry=$entry\"; done";
    let jail = fixture.jail(shell(script, &marker));
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    let stdout = outcome.stdout.to_string_lossy().into_owned();
    assert!(
        matches!(outcome.end, ProcessEnd::Exited(status) if status.code == 0),
        "the worker failed: {outcome:?}"
    );
    assert!(
        stdout.contains("ppid=1"),
        "the worker is a child of the namespace init: {stdout}"
    );
    assert!(
        stdout.contains("pid=2"),
        "the worker is the second process of the namespace: {stdout}"
    );
    let entries: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("entry="))
        .collect();
    assert_eq!(
        entries,
        vec!["/proc/1", "/proc/2"],
        "the jail sees the namespace processes only: {stdout}"
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.contains(&std::process::id().to_string())),
        "the host process table is invisible: {stdout}"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the jail leaves no process behind"
    );
}

#[test]
fn the_production_worker_reports_its_exit_status() {
    let fixture = Fixture::new(every_control());
    let marker = marker("exit");
    let jail = fixture.jail(shell("exit 7", &marker));
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    assert!(
        matches!(outcome.end, ProcessEnd::Exited(status) if status.code == 7),
        "the exit code is the worker's: {outcome:?}"
    );
    let exit = outcome.exit.expect("an exit status");
    assert_eq!(exit.python_exit_code(), 7);
    assert_eq!(exit.signal, None);
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the jail leaves no process behind"
    );
}

#[test]
fn a_worker_that_dies_from_a_signal_reports_the_signal() {
    let fixture = Fixture::new(every_control());
    let marker = marker("signal");
    let jail = fixture.jail(shell("kill -SEGV $$", &marker));
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    let exit = outcome.exit.expect("an exit status");
    assert_eq!(
        exit.signal,
        Some(libc_sigsegv()),
        "the signal death is reported: {outcome:?}"
    );
    assert_eq!(
        exit.python_exit_code(),
        -i64::from(libc_sigsegv()),
        "the Python view of a signal death is negative"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the jail leaves no process behind"
    );
}

/// The number of `SIGSEGV`, without depending on the `libc` crate.
fn libc_sigsegv() -> i32 {
    11
}

#[test]
fn the_deadline_terminates_the_whole_namespace() {
    let fixture = Fixture::new(every_control());
    let marker = marker("deadline");
    let mut jail = fixture.jail(shell("sleep 30", &marker));
    jail.limits = Some(
        ProcessLimits::default()
            .with_wall_timeout(Duration::from_millis(700))
            .with_bootstrap_timeout(Duration::from_millis(700)),
    );
    let started = Instant::now();
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    assert!(
        matches!(outcome.end, ProcessEnd::TimedOut(_)),
        "the deadline ends the run: {outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the deadline is enforced"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the deadline leaves no orphan behind"
    );
}

#[test]
fn the_cancellation_terminates_the_whole_namespace() {
    let fixture = Fixture::new(every_control());
    let marker = marker("cancel");
    let jail = fixture.jail(shell("sleep 30", &marker));
    let token = CancelToken::new();
    let cancelling = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        cancelling.cancel();
    });
    let outcome = jail.run(&[], &token).expect("the worker runs");
    assert!(
        matches!(outcome.end, ProcessEnd::Cancelled(_)),
        "the cancellation ends the run: {outcome:?}"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the cancellation leaves no orphan behind"
    );
}

#[test]
fn the_output_of_the_worker_is_bounded() {
    let fixture = Fixture::new(every_control());
    let marker = marker("flood");
    let script = "i=0; while [ $i -lt 5000 ]; do printf '%0.sX' $(seq 1 200); i=$((i+1)); done";
    let mut jail = fixture.jail(shell(script, &marker));
    jail.limits = Some(ProcessLimits::default().with_output_limits(4_096, 4_096));
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    assert!(
        matches!(
            outcome.end,
            ProcessEnd::OutputLimitExceeded(kala_process::OutputStream::Stdout)
        ),
        "the flood is refused: {outcome:?}"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the flood leaves no orphan behind"
    );
}

#[test]
fn an_orphan_of_the_worker_does_not_outlive_the_jail() {
    let fixture = Fixture::new(every_control());
    let marker = marker("orphan");
    // The worker leaves a grandchild behind and exits at once.
    let script = "/bin/sh -c 'while :; do sleep 1; done' \"$0\" & exit 0";
    let jail = fixture.jail(shell(script, &marker));
    let outcome = jail.run(&[], &CancelToken::new()).expect("the worker runs");
    assert!(
        matches!(outcome.end, ProcessEnd::Exited(status) if status.code == 0),
        "the worker finished: {outcome:?}"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the grandchild does not outlive the namespace"
    );
}

#[test]
fn the_production_jail_requires_the_pid_namespace() {
    let fixture = Fixture::new(without_pid_namespace());
    let jail = fixture.jail(shell("exit 0", &marker("refused")));
    let error = jail.process_spec().expect_err("the jail is refused");
    assert!(
        error.to_string().contains("pid_namespace"),
        "the refusal names the missing control: {error}"
    );
    let outcome = jail.run(&[], &CancelToken::new());
    assert!(outcome.is_err(), "the worker never runs");
}

#[test]
fn the_production_jail_refuses_a_control_it_cannot_establish() {
    let fixture = Fixture::new(every_control());
    // A device node that does not exist cannot be bound, so the control
    // cannot be established. The Python implementation continues without it;
    // the production path refuses, because the worker would run in a jail
    // that is weaker than the specification.
    let mut spec = fixture.spec.clone();
    spec.device_bindings = vec!["/dev/kala-nonexistent-device".to_owned()];
    let marker = marker("unestablished");
    let mut jail = fixture.jail(shell("exit 0", &marker));
    jail.spec = spec;
    let outcome = jail
        .run(&[], &CancelToken::new())
        .expect("the worker is launched");
    assert!(
        matches!(outcome.end, ProcessEnd::Exited(status) if status.code == 121),
        "the jail is refused: {outcome:?}"
    );
    assert!(
        outcome.stderr_summary.contains("sandbox_setup_failed"),
        "the refusal is reported: {outcome:?}"
    );
    assert!(
        wait_until_no_process(&marker).is_empty(),
        "the refusal leaves no process behind"
    );
}

#[test]
fn the_live_probe_reports_the_pid_namespace_for_the_production_path() {
    // The production helper is the binary the supervisor launches, so the
    // probe of the production path is the probe of the same code.
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_kala-sandbox-init"));
    let mut options = ProbeOptions::new(helper);
    options.timeout = Duration::from_secs_f64(PROBE_TIMEOUT_SECONDS);
    let probe = probe_sandbox(&options);
    assert!(probe.available, "the probe failed: {probe:?}");
    assert!(
        probe.controls.contains(&"pid_namespace".to_owned()),
        "the production path verifies the PID namespace: {probe:?}"
    );
}
