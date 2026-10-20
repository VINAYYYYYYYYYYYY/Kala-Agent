# Cell report — native bounded process supervision and sandbox boundary

> **Repair round 2 (2026-10-19):** all five `REVIEW-1.md` items are repaired.
> The round-1 text below is retained as the prior worker response and as the
> record of the crate that Sol reviewed. It is superseded where the round-2
> section at the end reports stricter validation, different status mapping,
> new tests or corrected counts. In particular, round 1's **38 tests** are now
> **46 tests** on Linux; the Windows stdio direction, Unix stdin bootstrap
> bound, aggregate environment validation, Python signal status and I/O
> liveness gap are fixed.

- Cell: `.mece/waves/rust-backend-20261019/cells/process`
- Brief: [`TASK.md`](TASK.md)
- Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**.
  T3 task
  `node:delegated-task:command%3Amcp%3A84bb7a8e-861f-4322-aff7-0fcabb65179a%3Adelegate-task%3Arust-backend-20261019-process-r1-go-continue49`,
  child thread
  `thread:delegated-task:command%3Amcp%3A84bb7a8e-861f-4322-aff7-0fcabb65179a%3Adelegate-task%3Arust-backend-20261019-process-r1-go-continue49`
  (`LAUNCH.md`).
- Reviewer: GPT-6.1-Sol (conduct/review; not run by this worker).
- Date: 2026-10-19.
- No delegation, no child turns, no commit, no push, no deploy, no model or
  provider switch, and no packaging cutover were performed.

## Delivered

A new standalone crate `backend/crates/kala-process/**` plus this cell's
`REPORT.md` and `SESSION.md`. Nothing else was edited. The crate has its own
`[workspace]` and local `Cargo.lock`; it does **not** touch
`backend/Cargo.toml`, `backend/Cargo.lock`, accepted crate sources/manifests,
Studio, contracts, shared fixtures, Python product code, packaging, frontend or
user files. No `kala-json` dependency was needed.

| Path | Purpose |
|---|---|
| `backend/crates/kala-process/Cargo.toml` | standalone workspace; local dependency set (`thiserror`, `serde_json`, `tempfile`, Unix `libc`, Windows `windows-sys`) |
| `backend/crates/kala-process/Cargo.lock` | local resolved lock; all final commands used `--locked` |
| `src/lib.rs` | crate documentation, re-exports, compatibility constants |
| `src/process.rs` | `ProcessSpec`, `CancelToken`, supervisor entry point |
| `src/outcome.rs` | typed outcomes, exit status, bounded captures, cleanup evidence, Python status mapping |
| `src/limits.rs` | wall/bootstrap/termination deadlines and stdin/stdout/stderr bounds |
| `src/environment.rs` | explicit child environment, protected keys, exact Python transitional helper |
| `src/workspace.rs` | confined relative path and working-directory resolution, symlink refusal |
| `src/error.rs` | typed `ProcessError`, workspace/environment/sandbox errors |
| `src/unix.rs` | raw `pipe2`/`fork`/`execve`, process groups, non-blocking drain, bootstrap handshake, reaping |
| `src/windows.rs` | suspended `CreateProcessW`, explicit handle list, Job Object containment, resume after assignment |
| `src/sandbox.rs` | `IsolationProfile`/`SandboxSpec`/`SandboxProbe`/`SandboxVerdict`, controls, probe, judge, sanitizer |
| `src/sandbox_plan.rs` | Linux mount/namespace/chroot plan applied in the probe helper and in the post-fork child |
| `src/bin/kala-process-probe.rs` | native strict-sandbox probe helper |
| `tests/fixtures/kala_process_fixture.rs` | native process fixture (`echo`, exits, sleep, floods, stdin, stderr, descendants, escapee) |
| `tests/process.rs` | process/supervision/deadline/cleanup/concurrency tests |
| `tests/sandbox.rs` | payloads, gating, probe, judge, strict jail adversarial tests |
| `tests/python_oracle.rs` + `tests/python_oracle.py` | test-only live Python source oracle |
| `tests/windows_pending.rs` | Windows compile-checked tests, all marked execution pending |
| `README.md` | public API, containment model, status mapping, probe evidence, gaps |

## Public API

```rust
// process.rs
pub struct CancelToken;                  // Clone, Default
impl CancelToken { pub fn new() -> Self; pub fn cancel(&self); pub fn is_cancelled(&self) -> bool; }

pub struct ProcessSpec;
impl ProcessSpec {
    pub fn new(executable: impl Into<PathBuf>, working_directory: impl Into<PathBuf>)
        -> Result<Self, ProcessError>;
    pub fn arg(self, argument: impl Into<OsString>) -> Self;
    pub fn args<I, S>(self, arguments: I) -> Self where I: IntoIterator<Item = S>, S: Into<OsString>;
    pub fn with_environment(self, environment: ChildEnvironment) -> Self;
    pub fn with_limits(self, limits: ProcessLimits) -> Self;
    pub fn with_sandbox(self, sandbox: SandboxSpec) -> Result<Self, ProcessError>;
    pub fn executable(&self) -> &Path;
    pub fn arguments(&self) -> &[OsString];
    pub fn working_directory(&self) -> &Path;
    pub fn environment(&self) -> &ChildEnvironment;
    pub fn limits(&self) -> &ProcessLimits;
    pub fn sandbox(&self) -> Option<&SandboxSpec>;
}
pub fn supervise(spec: &ProcessSpec, stdin: &[u8], cancel: &CancelToken)
    -> Result<ProcessOutcome, ProcessError>;

// outcome.rs
pub struct CapturedOutput;               // as_bytes, len, is_empty, truncated,
                                         // discarded_bytes, as_str, to_string_lossy
pub struct ExitStatus { pub code: i64, pub signal: Option<i32> }
impl ExitStatus { pub fn exited(code: i64) -> Self; pub fn signalled(signal: i32) -> Self;
                  pub fn python_exit_code(&self) -> i64; }
pub enum TimeoutStage { Bootstrap, WallClock }
pub enum CancelStage { BeforeLaunch, Bootstrap, Running }
pub enum OutputStream { Stdout, Stderr }
pub enum ProcessEnd { Exited(ExitStatus), TimedOut(TimeoutStage),
                      Cancelled(CancelStage), OutputLimitExceeded(OutputStream) }
pub struct TreeCleanup { pub direct_reaped: bool, pub tree_terminated: bool,
                         pub tree_empty: bool, pub pipes_drained: bool,
                         pub termination_timed_out: bool }
impl TreeCleanup { pub fn is_complete(&self) -> bool; }
pub struct ProcessOutcome {
    pub end: ProcessEnd, pub exit: Option<ExitStatus>,
    pub stdout: CapturedOutput, pub stderr: CapturedOutput,
    pub stderr_summary: String, pub elapsed: Duration,
    pub process_id: Option<u32>, pub cleanup: TreeCleanup,
}
impl ProcessOutcome {
    pub fn succeeded(&self) -> bool;
    pub fn stdout_str(&self) -> Result<&str, Utf8Error>;
    pub fn stderr_summary(&self) -> &str;
    pub fn compatibility(&self) -> Compatibility;
}
pub enum CompatibilityStatus { Succeeded, Failed, Denied, TimedOut, Cancelled }
pub struct Compatibility { pub status: CompatibilityStatus,
                           pub error_code: Option<&'static str>, pub message: String }

// limits.rs
pub struct ProcessLimits { /* wall/bootstrap/termination deadlines, stdin/stdout/stderr caps,
                              enforce_resource_limits, max_artifact_bytes */ }
impl ProcessLimits { pub fn python_defaults() -> Self; pub fn validate(&self) -> Result<(), String>;
                     pub fn with_wall_timeout(..) -> Self; pub fn with_bootstrap_timeout(..) -> Self;
                     pub fn with_termination_timeout(..) -> Self;
                     pub fn with_output_limits(..) -> Self; }
// exported constants: DEFAULT_*, MAX_*, MAX_STDIN_BYTES

// environment.rs
pub struct EnvironmentEntry { pub key: String, pub value: String }
pub struct ChildEnvironment;             // from_entries, minimal, legacy_python_worker,
                                         // with_extra, with_allowlisted_extra, entries, get, len, is_empty
pub fn build_child_environment(workspace: &Path, import_roots: &[PathBuf],
                               extra: &[(String, String)]) -> Result<ChildEnvironment, EnvironmentError>;
pub fn is_protected(key: &str) -> bool;
pub const PROTECTED_CHILD_ENVIRONMENT_KEYS: &[&str];
pub const CHILD_ENVIRONMENT_KEYS: &[&str];
pub const TOOL_PROTOCOL_VERSION: &str; pub const PROCESS_PROTOCOL_VERSION: &str;

// workspace.rs
pub fn resolve_workspace_path(workspace: &Path, relative: &str) -> Result<PathBuf, WorkspaceError>;
pub fn resolve_workspace_directory(workspace: &Path, relative: &str) -> Result<PathBuf, WorkspaceError>;
pub fn canonical_directory(path: &Path) -> Result<PathBuf, WorkspaceError>;

// sandbox.rs
pub enum IsolationProfile { Portable, StrictLinux }
pub struct SandboxProbe { pub available: bool, pub platform: String, pub controls: Vec<String>,
                          pub detail: String, pub elapsed_ms: f64 }
impl SandboxProbe { pub fn to_payload(&self) -> serde_json::Value;
                    pub fn from_payload(&serde_json::Value) -> Result<Self, SandboxProbeError>; }
pub struct SandboxSpec { pub controls: BTreeSet<String>, pub root: PathBuf, pub workspace: PathBuf,
                         pub readonly_bindings: Vec<String>, pub writable_bindings: Vec<String>,
                         pub device_bindings: Vec<String>, pub scratch_paths: Vec<String>,
                         pub scratch_bytes: i64 }
impl SandboxSpec { pub fn to_payload(&self) -> Value;
                   pub fn from_payload(&Value) -> Result<Self, SandboxSetupError>;
                   pub fn controls(&self) -> Vec<String>;
                   pub fn has_required_controls(&self) -> bool;
                   pub fn validate_strict(&self) -> Result<(), SandboxSetupError>; }
pub struct SandboxVerdict { pub allowed: bool, pub reason: String, pub controls: Vec<String> }
impl SandboxVerdict { pub fn to_payload(&self) -> Value; }
pub struct SandboxProbeOptions { pub helper: PathBuf, pub worker_roots: Vec<PathBuf>,
                                 pub scratch_bytes: i64, pub timeout: Duration }
impl SandboxProbeOptions { pub fn new(helper: impl Into<PathBuf>) -> Self;
                           pub fn discover() -> Option<Self>; }
pub fn platform_name() -> String;
pub fn resolve_system_bindings(worker_roots: &[PathBuf]) -> Vec<String>;
pub fn build_sandbox_spec(controls: &[String], root: &Path, workspace: &Path,
                          worker_roots: &[PathBuf], scratch_bytes: i64)
    -> Result<SandboxSpec, SandboxSetupError>;
pub fn portable_controls(limits: &ProcessLimits) -> Vec<&'static str>;
pub fn enforced_controls(profile: IsolationProfile, probe: Option<&SandboxProbe>,
                         limits: &ProcessLimits) -> Vec<String>;
pub fn judge_sandbox(profile: IsolationProfile, probe: Option<&SandboxProbe>) -> SandboxVerdict;
pub fn isolate_process(spec: &SandboxSpec) -> Result<Vec<String>, SandboxSetupError>;
pub fn verify_controls(spec: &SandboxSpec, canary: &str, workspace: &str,
                       established: &[String]) -> (Vec<String>, Vec<String>);
pub fn probe_sandbox(options: &SandboxProbeOptions) -> SandboxProbe;
pub fn run_probe(spec_payload: &str, canary_payload: &str) -> i32;
pub fn sanitize_child_stderr(payload: &[u8], replacement: Option<&str>) -> String;
// probe helpers: canary_is_invisible, workspace_is_writable, system_bindings_are_readonly,
// procfs_is_namespaced, scratch_is_writable, devices_are_bound, device_list_is_minimal,
// network_is_isolated, unshadowed_scratch_paths
// constants: STRICT_REQUIRED_CONTROLS, STRICT_OPTIONAL_CONTROLS, CLONE_*, MS_*,
// ISOLATION_FAILURE_EXIT=121, PROBE_FAILURE_EXIT=122, PROBE_TIMEOUT_SECONDS=30.0

// Windows-only diagnostics
pub fn current_process_in_job() -> Option<bool>;
pub fn windows_handle_count() -> Option<u32>;
```

## Implemented semantics

### Process launch and I/O

- Explicit absolute executable plus argv; no shell, PATH lookup, argument
  splitting or Python bootstrap. Windows argv is quoted with the MSVCRT rules
  and `lpApplicationName` is set explicitly.
- Canonical absolute working directory; `resolve_workspace_path` ports the
  Python relative-path rules (NUL/backslash/absolute/traversal/symlink/length
  refusal) but is documented as **cwd confinement, not a filesystem jail**.
- The default child environment is built from scratch:
  `HOME`, `TMPDIR` (and Windows `USERPROFILE`/`TMP`/`TEMP`), `LANG`,
  `PATH`, `KALA_PROCESS_PROTOCOL`; no inherited secrets. The exact Python
  `build_child_environment` key set is retained as a transitional test/oracle
  helper.
- stdin, stdout and stderr are bounded. stdin is non-blocking on Unix; stdout
  and stderr are drained by `poll` + `O_NONBLOCK` without reader threads, so
  simultaneous floods and descendant pipe holders cannot deadlock the
  supervisor. Windows uses `PeekNamedPipe`/`ReadFile` on the main supervisor
  thread and a bounded stdin writer thread.
- stdout is exposed as raw bytes plus a strict UTF-8 view; invalid UTF-8 is
  preserved and rejected by `stdout_str`. stderr is exposed raw plus
  `sanitize_child_stderr`, matching the Python first/last-line, control-strip,
  whitespace-collapse, workspace-redaction and 512-character rules.
- Wall-clock, bootstrap and termination deadlines are enforced. Cancellation is
  checked before launch, during the exec handshake, while writing stdin/reading
  output and while waiting. stdout cap exceeds maps to
  `OutputLimitExceeded(Stdout)` even if the worker exits before the last bytes
  are drained; stderr cap truncates without failing a clean exit.

### Unix containment

- `pipe2(O_CLOEXEC)` + raw `fork` + `execve`. The parent closes every
  child-side descriptor; the child closes every descriptor above fd 3 except a
  bootstrap pipe, then execs. No uncontrolled child is created.
- Each worker has its own process group (`setpgid(0,0)`). The supervisor
  observes the direct child with `waitid(..., WNOWAIT)`, kills the process
  group with `SIGKILL`, closes stdin, drains both pipes non-blockingly until
  EOF or the termination deadline, then reaps. A descendant that deliberately
  creates a new session/group is outside the documented boundary; the outcome
  reports incomplete pipe/tree cleanup rather than claiming success.
- The post-fork child performs only async-signal-safe raw calls: `dup2`,
  `fcntl`, `close`/`close_range`, `setrlimit`, `sigprocmask`, `signal`,
  `unshare`, `mount`, `chroot`, `chdir`, `write`, `execve`. All sandbox paths,
  argv, envp and namespace map contents are precomputed before `fork`; no
  allocation, formatting, environment access or locking occurs in that window.
- Optional POSIX limits: `umask(0077)`, `RLIMIT_CORE=0`,
  `RLIMIT_CPU=ceil(wall)+1`, `RLIMIT_FSIZE=max_artifact_bytes`,
  `RLIMIT_NOFILE=256`.

### Windows containment

- The worker is created with `CREATE_SUSPENDED`, a
  `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` containing exactly stdin/stdout/stderr,
  and an explicit Unicode environment block. Parent pipe ends are
  non-inheritable and all other handles are excluded.
- A Job Object is configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; the
  suspended child is assigned **before** `ResumeThread`. If job creation,
  handle-list setup, assignment or resume fails, the job/child is terminated
  and no uncontrolled worker is resumed.
- The parent's own job is queried. `CREATE_BREAKAWAY_FROM_JOB` is used only for
  `JOB_OBJECT_LIMIT_BREAKAWAY_OK`; `SILENT_BREAKAWAY_OK` is recognized without
  that flag. Nested-job assignment failure is an explicit containment error.
- The supervisor terminates the job after every outcome, drains pipes to EOF or
  the termination deadline, waits for the direct process, and reports active
  job-process cleanup evidence. No Windows runtime execution is claimed.

### Strict Linux sandbox

- `probe_sandbox` executes the native `kala-process-probe` helper (built from
  this crate). The helper is a fresh, single-threaded process that applies the
  same `PreparedSandbox` plan used by production, verifies namespace inode
  changes, canary invisibility, network unreachability, workspace writes,
  read-only system mounts, device identity, scratch writes and optional procfs,
  and prints only verified controls.
- Required controls are gated by `build_sandbox_spec`/`validate_strict`;
  unknown controls are refused; `pid_namespace` is refused with a precise
  pending message. Optional probe failures are reported as unenforced, not
  silently advertised. A production worker built from a probe specification
  fails closed if any requested control cannot be established.
- The Python interpreter bootstrap is superseded: no Python process is started,
  no Python is used for isolation, and `build_child_environment` is
  compatibility-only.

## Evidence

Host: `Linux v9i 7.2.8-9-omarchy #1 SMP PREEMPT_DYNAMIC ... x86_64 GNU/Linux`;
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`; `cargo 1.99.1 (5f94df478 2026-08-27)`.
Targets installed: `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
`x86_64-pc-windows-gnu`.

| Check | Result |
|---|---|
| `cargo fmt --check` | passed |
| `cargo test --locked` | **38 passed, 0 failed, 0 ignored** (27 process + 1 live Python oracle + 10 sandbox; doc-tests 0) |
| `cargo test --locked --release` | **38 passed, 0 failed, 0 ignored** |
| `cargo clippy --locked --all-targets -- -D warnings` | passed |
| `cargo build --locked --release` | passed |
| `cargo check --locked --target x86_64-pc-windows-msvc --all-targets` | passed |
| `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings` | passed |
| `cargo check --locked --target x86_64-pc-windows-gnu --all-targets` | passed |
| `cargo clippy --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings` | passed |
| `KALA_STRICT_SANDBOX=1 cargo test ... probe...` | passed on this host |
| `cargo doc --locked --no-deps` | passed |

Strict probe payload observed on this host:

```json
{"available":true,
 "controls":["chroot_workspace_only","device_nodes_minimal","ipc_namespace",
             "mount_namespace","network_isolation","readonly_system_bindings",
             "scratch_tmpfs","user_namespace","uts_namespace","workspace_read_write"],
 "detail":"","elapsed_ms":22.514,"platform":"linux-x86_64"}
```

The raw probe helper with all supported optional controls reported
`not enforced: procfs`; `procfs` is therefore **not** advertised. `pid_namespace`
is not requested and is explicitly pending. The adversarial strict test used
the actual probe controls to run a native fixture and observed: workspace write
succeeded; host canary read exited 42; external `8.8.8.8:53` connect exited 43;
write to `/usr/...` exited 7.

Tests specifically exercise: success/exit codes; argv without a shell; wall,
bootstrap and termination deadlines; cancellation before launch, during a slow
launch (actual `Cancelled(Bootstrap)` observed), and during execution;
simultaneous stdout/stderr floods; invalid UTF-8; stdout/stderr caps; stdin
backpressure; lingering descendant pipe holders; grandchild cleanup on timeout;
a deliberately escaped process group that holds pipes and is reported honestly;
concurrent workers without descriptor growth (the test allows a 16-descriptor
margin for parallel test-harness pipes while still detecting the 64-descriptor
per-worker leak regression); path/symlink/environment refusal;
strict payload/gating/judge; live Python oracle; probe helper; missing-helper
truthfulness; and strict adversarial jail behavior.

## Unfinished controls / explicit gaps

- **Windows runtime is not proven.** Windows source, tests and diagnostics were
  compiled and clippy-checked for MSVC and GNU, but no Windows host executed
  them. `tests/windows_pending.rs` contains six Windows-only tests, all
  `#[ignore]` with execution-pending reasons.
- **`pid_namespace` is pending.** The native launcher does not implement the
  Python double-fork PID-1 transition; requesting it fails with a precise
  message and it is never advertised.
- **`procfs` is host-unavailable.** The probe verified it cannot mount procfs in
  this host's user/mount namespace combination; it is reported as unenforced.
- **Portable Unix boundary is the process group.** A descendant that creates a
  new session/group is outside the documented portable boundary; Windows Job
  Objects provide the true child-tree boundary.
- **`cwd_confined` is not a filesystem jail.** It is path-string confinement and
  an explicit cwd; only the strict Linux profile provides mount/chroot
  confinement.
- This crate intentionally does **not** implement the executor worker pool,
  artifact digest/classification controls, policy, budgets, tool registry,
  request/schema protocol or HTTP. `enforced_controls` never claims those.
- No performance, optimization, full migration or Windows-readiness claim is
  made.

---

# Repair round 2 — REVIEW-1 disposition

Repair implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**. No
delegation, child turns, model/provider switch, commit, push, deploy or
packaging cutover. Scope remained `backend/crates/kala-process/**` plus this
`REPORT.md` and `SESSION.md`; no root manifest/lock, other crate, Python
product, Studio, contracts, fixture corpus, adapter, packaging or user file was
edited. `Cargo.toml` and `Cargo.lock` are unchanged from round 1.

## Review items

| Item | Repair | Regression evidence |
|---|---|---|
| 1. P1 Windows stdio direction reversed | `create_pipe` now returns an explicit `(parent_end, child_end)` pair; `child_reads=true` means stdin `(parent write, child read)`, `false` means stdout/stderr `(parent read, child write)`. `stdio_pipes` assigns each field by direction, sets the parent end non-inheritable and the child end inheritable explicitly. | Windows-only unit test `stdio_pipe_direction_inheritance_and_round_trip_are_correct` inspects `GetHandleInformation` on all six handles and performs a real write/read round trip for stdin, stdout and stderr through `StdioPipes`. Compiled and clippy-checked for MSVC and GNU; marked ignored because no Windows host executed it. |
| 2. P2 Unix stdin bootstrap bound absent | `bootstrap_deadline` is passed into `supervise_loop`; while stdin is pending, it is checked every loop iteration with the wall deadline and used as the `poll` timeout. Expiry returns `TimedOut(Bootstrap)`. | `pending_stdin_is_bounded_by_the_bootstrap_deadline_not_only_the_handshake` is the review reproduction: native fixture `stdin-hold 30000`, 1 MiB stdin, bootstrap 50 ms, wall 600 ms. It returns `TimedOut(Bootstrap)` in under 2 s. The existing `stdin_backpressure_is_bounded_by_the_wall_clock` (bootstrap 5 s, wall 200 ms) still returns `TimedOut(WallClock)`, so the stages are independently tested. |
| 3. P2 malformed environment entries bypass validation | `ChildEnvironment::from_entries` re-validates every public-field `EnvironmentEntry` key and value before sorting/deduplicating. `ChildEnvironment::validate` is called before launch and again before the Windows UTF-16 environment block. Windows rejects an interior NUL in executable, every argument and the working directory before `CreateProcessW`; Unix already rejected NUL argv through `CString`. | `malformed_environment_entries_are_refused_at_the_aggregate_boundary` covers `BAD=KEY`, a NUL value and a NUL key (all refused) plus a valid entry. `interior_nul_arguments_are_refused_before_execution` covers exact argv on Linux and the shared validation path. Windows unit test `interior_nul_argv_is_refused_before_create_process` is compile-checked and execution-pending. |
| 4. P2 Unix signal status not Python-compatible | `ExitStatus::python_exit_code()` now returns `-signal` on a signal death (the `Popen.returncode` view), while `ExitStatus::shell_exit_code()` explicitly exposes the conventional `128 + signal` status. `compatibility()` uses the Python view, so the message says `status -9`, not `137`. | `signal_death_uses_python_popen_status_not_the_shell_status` runs the native fixture through `self-signal 9` and asserts `-9`, shell `137`, `Exited(signalled(9))` and a `tool_crashed` message containing `status -9`. The live Python oracle runs the same fixture to death by `SIGKILL` and compares real `Popen.returncode == -9`. |
| 5. P2 I/O liveness / non-blocking cleanup | Unix output reads consume at most 256 KiB per call and stdin writes at most 256 KiB per call, so cancellation, bootstrap, wall-clock and output-cap decisions are revisited after every bounded operation. Parent stdio descriptors are made non-blocking immediately after `fork` and **before** the handshake; `finish` defensively re-applies non-blocking mode and closes a descriptor it cannot convert. `StdinState` now closes its descriptor on every early return. | Unit tests `read_capture_returns_after_its_byte_budget_under_a_ready_producer` and `stdin_pump_returns_after_its_byte_budget_under_a_fast_reader` assert the 256 KiB budgets. Integration test `a_continuously_ready_output_producer_cannot_starve_the_wall_deadline` runs `flood-forever` with a 5 ms wall deadline and returns `TimedOut(WallClock)` in bounded time. `an_escaped_pipe_writer_that_is_always_ready_cannot_monopolize_cleanup` runs an escaped pipe writer with a 5 ms termination window and returns promptly with `pipes_drained=false` and incomplete cleanup rather than waiting for EOF. |

Observed regression lines (`--nocapture`, this host):

```text
bootstrap-stdin regression: end=TimedOut(Bootstrap) elapsed=52.122275ms
  cleanup=TreeCleanup { direct_reaped: true, tree_terminated: true,
  tree_empty: true, pipes_drained: true, termination_timed_out: false }
signal regression: exit=ExitStatus { code: 137, signal: Some(9) }
  python=-9 shell=137 compatibility=Compatibility { status: Failed,
  error_code: Some("tool_crashed"),
  message: "the tool process exited with status -9" }
```

## Public API delta

Only one public method was added; no existing signature was removed or
renamed:

```rust
impl ExitStatus {
    /// Conventional shell status (`128 + signal` on a signal death).
    pub fn shell_exit_code(&self) -> i64;
}
```

Behavioral corrections in existing APIs:

- `ExitStatus::python_exit_code()` returns `-signal` for a signal death.
- `ChildEnvironment::from_entries` now rejects malformed public-field entries
  instead of only checking duplicates.
- Unix bootstrap deadlines now cover stdin delivery/closure.
- Windows stdin/stdout/stderr handle directions are correct.

## Changed paths (round 2)

```text
backend/crates/kala-process/src/environment.rs
backend/crates/kala-process/src/lib.rs
backend/crates/kala-process/src/outcome.rs
backend/crates/kala-process/src/process.rs
backend/crates/kala-process/src/unix.rs
backend/crates/kala-process/src/windows.rs
backend/crates/kala-process/tests/fixtures/kala_process_fixture.rs
backend/crates/kala-process/tests/process.rs
backend/crates/kala-process/tests/python_oracle.rs
backend/crates/kala-process/tests/python_oracle.py
backend/crates/kala-process/README.md
.mece/waves/rust-backend-20261019/cells/process/REPORT.md
.mece/waves/rust-backend-20261019/cells/process/SESSION.md
```

`tests/windows_pending.rs` was not changed; the new Windows tests are unit
tests inside `src/windows.rs` so they can inspect private pipe ownership. They
carry the same `#[ignore = "requires a Windows host; runtime proof is pending"]`
marker.

## Actual verification (round 2)

Host: `Linux v9i 7.2.8-9-omarchy #1 SMP PREEMPT_DYNAMIC ... x86_64 GNU/Linux`;
`rustc 1.99.1 (58cf8f9c3 2026-10-13)`; `cargo 1.99.1 (5f94df478 2026-08-27)`.
All commands ran in `backend/crates/kala-process` with `--offline --locked`.

| Command | Actual result |
|---|---|
| `cargo fmt --all -- --check` | passed |
| `cargo test --offline --locked` | **46 passed, 0 failed, 0 ignored** (2 lib unit + 33 process + 1 live Python oracle + 10 sandbox; 0 Windows tests executed on Linux; 0 doc-tests) |
| `cargo test --offline --locked --release` | **46 passed, 0 failed, 0 ignored** (same split) |
| `cargo clippy --offline --locked --all-targets -- -D warnings` | passed |
| `cargo build --offline --locked --release` | passed |
| `cargo check --offline --locked --target x86_64-pc-windows-msvc --all-targets` | passed |
| `cargo clippy --offline --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings` | passed, including the new ignored Windows unit tests |
| `cargo check --offline --locked --target x86_64-pc-windows-gnu --all-targets` | passed |
| `cargo clippy --offline --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings` | passed |
| `KALA_STRICT_SANDBOX=1 cargo test --offline --locked --test sandbox -- --nocapture` | **10 passed, 0 failed** |
| `cargo doc --offline --locked --no-deps` | passed |

The opt-in strict run reported the same host-verified controls as round 1:

```json
{"available":true,
 "controls":["chroot_workspace_only","device_nodes_minimal","ipc_namespace",
             "mount_namespace","network_isolation","readonly_system_bindings",
             "scratch_tmpfs","user_namespace","uts_namespace","workspace_read_write"],
 "detail":"","elapsed_ms":22.08,"platform":"linux-x86_64"}
```

The adversarial strict test again exercised the actual helper and observed
workspace write success, host canary invisibility (exit 42), network refusal
(exit 43) and `/usr` write refusal (exit 7). `procfs` remains unadvertised and
`pid_namespace` remains explicitly unimplemented.

## Remaining limitations (unchanged plus runtime-pending windows)

- **Windows runtime is still unproven.** MSVC and GNU all-target checks and
  clippy pass. The corrected pipe direction, Job Object containment, handle
  inheritance, NUL rejection and cleanup evidence are compiled but not executed
  here; the Windows-only tests remain ignored with execution-pending reasons.
- **`pid_namespace` is pending** and never advertised.
- **`procfs` is unenforced on this host** and never advertised.
- **Portable Unix boundary is the process group.** Escaped sessions/groups are
  reported as incomplete cleanup (now with a bounded return under a
  continuously ready escaped writer); Windows Job Objects are the intended true
  tree boundary, runtime pending.
- **`cwd_confined` is not a filesystem jail**; strict Linux is the only mounted
  confinement.
- No executor worker pool, policy, budget, registry, request protocol, HTTP,
  artifact digesting or packaging is implemented here. No performance,
  optimization, full-migration or Windows-readiness claim is made.
