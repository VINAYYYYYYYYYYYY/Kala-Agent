# kala-process

Native bounded process supervision and strict Linux sandbox boundary for the
Kala Agent backend. This is a **standalone crate** with its own workspace and
`Cargo.lock`; it is not a member of the accepted six-crate root workspace and
does not edit that workspace, Studio, contracts or any Python/product file.

This crate stops at the process/sandbox boundary. Request envelopes, JSON
schemas, policy, budgets, tool registries, worker pools, HTTP, artifact
classification, Solid Edge and packaging are out of scope.

## Public API

```rust
use std::time::Duration;
use kala_process::{
    CancelToken, ChildEnvironment, ProcessEnd, ProcessLimits, ProcessSpec,
    resolve_workspace_path, supervise,
};

let workspace = std::fs::canonicalize("run/calls/call-1/workspace")?;
let process = ProcessSpec::new("/usr/local/bin/kala-worker", &workspace)?
    .args(["--operation", "beam_volume"])
    .with_environment(ChildEnvironment::minimal(&workspace)?)
    .with_limits(ProcessLimits {
        wall_timeout: Duration::from_secs(30),
        bootstrap_timeout: Duration::from_secs(5),
        termination_timeout: Duration::from_secs(5),
        ..ProcessLimits::default()
    });
let cancel = CancelToken::new();
let outcome = supervise(&process, b"request bytes", &cancel)?;
```

Core types and functions:

- `ProcessSpec` — explicit absolute executable, argv (no shell/PATH lookup),
  canonical working directory, explicit `ChildEnvironment`, `ProcessLimits` and
  optional `SandboxSpec`.
- `supervise(&ProcessSpec, stdin, &CancelToken) -> Result<ProcessOutcome, ProcessError>`
- `ProcessOutcome` — `ProcessEnd`, exit status, bounded `stdout`/`stderr`,
  `stderr_summary`, elapsed time, pid and `TreeCleanup`.
- `ProcessEnd` — `Exited`, `TimedOut(Bootstrap|WallClock)`,
  `Cancelled(BeforeLaunch|Bootstrap|Running)`, `OutputLimitExceeded`.
- `ExitStatus` — `code` is the conventional shell status (`128 + signal`),
  `python_exit_code()` is the `Popen.returncode` view (`-signal` on a Unix
  signal death), and `shell_exit_code()` names the shell view explicitly.
- `ProcessOutcome::compatibility()` — exact `ToolRunStatus`/`ToolErrorCode`
  mapping for the future native tools executor.
- `CancelToken` — cloneable cooperative cancellation.
- `ChildEnvironment::minimal`, `build_child_environment` (Python-compatible
  transitional helper), `PROTECTED_CHILD_ENVIRONMENT_KEYS`.
- `resolve_workspace_path`, `resolve_workspace_directory` — relative-path
  confinement; **not** a filesystem jail.
- `SandboxSpec`, `SandboxProbe`, `SandboxVerdict`, `IsolationProfile` typed
  payloads matching `kala_agent.tools.sandbox`.
- `build_sandbox_spec`, `probe_sandbox`, `judge_sandbox`,
  `isolate_process`, `run_probe`, `verify_controls`, `enforced_controls`,
  `portable_controls`, `sanitize_child_stderr`.

`SandboxSpec::to_payload`/`from_payload`, `SandboxProbe::to_payload`/
`from_payload` and `SandboxVerdict::to_payload` use the Python field names and
validation rules.

## Native containment

### Unix (Linux development path)

`pipe2(O_CLOEXEC)` + raw `fork` + `execve`. The child performs only
async-signal-safe work after `fork`: `dup2`, `fcntl`, `close`/`close_range`,
`setrlimit`, signal reset, raw sandbox syscalls and `execve`; no allocation or
locking happens in that window. The worker is placed in its own process group;
the supervisor kills the group with `SIGKILL`, observes the direct child with
`waitid(..., WNOWAIT)`, drains both pipes non-blockingly up to the termination
deadline, and only then reaps. A descendant that creates a new session/group
leaves the portable boundary; that limitation is reported rather than hidden.
stdin, stdout and stderr never use blocking reader threads on Unix.

The parent-side stdio descriptors are made non-blocking immediately after
`fork`, **before** the exec handshake, so cleanup after an early launch failure
cannot block on a pipe held by a live process. The bootstrap deadline bounds
both the exec handshake and delivery/closure of stdin: a worker that never
reads a large request reports `TimedOut(Bootstrap)` even when the wall deadline
is later. Each output read consumes at most 256 KiB and each stdin pump writes
at most 256 KiB before the supervisor re-checks cancellation, the bootstrap
deadline, the wall deadline and the output cap. This keeps a continuously ready
producer or reader from monopolising a supervision or cleanup iteration; the
regression fixture is a native writer that never stops.

### Windows

`CreateProcessW` with `CREATE_SUSPENDED`, an explicit
`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` containing only the three stdio handles, a
Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assignment while the child
is still suspended, and `ResumeThread` only after assignment succeeds. Job
creation/configuration or assignment failure terminates the suspended child and
never resumes an uncontrolled worker. The parent's own job is queried:
nested-job failure is explicit, and `CREATE_BREAKAWAY_FROM_JOB` is used only
when the parent job has `JOB_OBJECT_LIMIT_BREAKAWAY_OK`.
`JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK` is recognized but needs no flag.

Each pipe is created with an explicit direction contract: stdin is
`(parent write, child read)` and stdout/stderr are `(parent read, child write)`.
The child end is inheritable, the parent end is not, and the child receives the
three named ends in `STARTUPINFOEXW`. A Windows-only unit test checks the
direction, inheritance flags and an actual round trip through each end; it is
compiled but marked ignored because no Windows runtime is available here.

**Windows runtime execution is not proven.** The Windows source/config is
cross-checked for both `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu`;
Windows-only tests are compiled but marked ignored until run on a real Windows
host.

## Strict Linux sandbox

`probe_sandbox` launches the native `kala-process-probe` helper (built from this
crate). That helper `exec`s into a fresh single-threaded process, builds the
jail and verifies it from the inside. Only verified controls are returned and
advertised by `judge_sandbox`/`enforced_controls`.

Verified controls on this development host:

```
available: true
controls: chroot_workspace_only, device_nodes_minimal, ipc_namespace,
          mount_namespace, network_isolation, readonly_system_bindings,
          scratch_tmpfs, user_namespace, uts_namespace, workspace_read_write
not advertised: procfs (mount denied), pid_namespace (pending native transition)
```

The probe requests the required controls plus all supported optional controls;
`procfs` failure is reported as unenforced rather than failing required
controls. The production worker plan fails closed if a control that the probe
verified later fails to establish.

`pid_namespace` is deliberately not implemented: the Python code's
double-fork PID-1 transition is pending a native launcher. A specification that
requests it fails with an explicit `pid_namespace is not implemented ...`
error.

`strict_linux` is unavailable on Windows; no Linux namespace is ever claimed
there.

## Status/error mapping

| Native outcome | Python `ToolRunStatus` | Python `ToolErrorCode` |
|---|---|---|
| exit 0 + complete cleanup | `SUCCEEDED` | none |
| wall/bootstrap deadline | `TIMED_OUT` | `timeout` |
| cancellation | `CANCELLED` | none |
| stdout cap | `FAILED` | `output_too_large` |
| exit 121 | `FAILED` | `sandbox_unavailable` |
| other non-zero exit | `FAILED` | `tool_crashed` |
| exit 0 + incomplete cleanup | `FAILED` | `tool_crashed` |

`ProcessError` maps sandbox preparation to `sandbox_unavailable`, launch/
containment failure to `isolation_unavailable`, cleanup failure to
`tool_crashed`, and caller/specification errors to `tool_error`.

Exit-status compatibility follows Python exactly: a worker killed by `SIGKILL`
has `Popen.returncode == -9`, so the `tool_crashed` message says `status -9`,
not the conventional shell `137`. `ExitStatus::shell_exit_code()` exposes the
shell view separately for callers that need it.

Environment blocks are aggregate-validated: `EnvironmentEntry` has public
fields, so `ChildEnvironment::from_entries` re-checks every key against
`[A-Za-z_][A-Za-z0-9_]{0,63}` and every value for embedded NULs. Windows also
rejects an interior NUL in an argument, executable or working directory before
`CreateProcessW`, and re-validates the environment before building the UTF-16
block.

## Python bootstrap transition

The Python executor's `-c` worker bootstrap reads a JSON request from stdin and
writes a JSON response. This crate does not run Python in production. A future
native tools executor supplies the native worker executable/argv and request
bytes directly. `build_child_environment` is retained only as an exact
compatibility port for transitional Python workers; `ChildEnvironment::minimal`
is the native-worker constructor and adds no interpreter variables.

## Tests

```sh
cargo fmt --check
cargo test --locked
cargo test --locked --release
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-msvc --all-targets
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

Integration tests use a native fixture executable for exits, signal deaths,
timeouts, cancellation, simultaneous floods, continuously ready producers and
escaped pipe writers, invalid UTF-8, output/stderr caps, stdin backpressure
bounded by the bootstrap deadline, descendant pipe holders, grandchild cleanup,
concurrency, malformed aggregate environments and path/argument refusal.
`tests/python_oracle.py` is a **test-only** live oracle against
`src/kala_agent/tools/sandbox.py` and `protocol.py`; it also runs the real
native fixture to death by `SIGKILL` and compares Python's `Popen.returncode`
with the Rust compatibility view. `KALA_STRICT_SANDBOX=1` makes the privileged
strict probe test fail instead of reporting an unavailable host.

## Honest gaps

- Windows runtime, Job Object behaviour, handle inheritance and nested jobs:
  compiled/linted only; execution pending a Windows host.
- `pid_namespace`: pending native double-fork bootstrap.
- Portable Unix confinement kills the process group, not descendants that
  deliberately leave it. Windows Job Objects provide the true tree boundary.
- `cwd_confined` confines path-string resolution and sets the worker's working
  directory; it is not a filesystem jail.
- The crate does not implement the executor's worker pool, artifact digest/
  classification controls, policy or request protocol. `enforced_controls`
  therefore never claims those.
- No performance or full-migration claim.
