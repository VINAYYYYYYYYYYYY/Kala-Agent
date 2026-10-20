# sandbox-r2 — restore the PID namespace in the production worker jail (security repair)

**Status:** the security regression is repaired inside `backend/crates/kala-sandbox`.
No runtime cutover is claimed, no Windows runtime is claimed, no speed or
readiness claim is made, no commit was made.

**Cell:** `sandbox-r2` · **Wave:** `rust-backend-20261019` · **Date:** 2026-10-20
· **Predecessor:** `sandbox-r1` (the accepted port, whose difference 11 recorded
the regression)

---

## 1. The finding and the repair in one paragraph

Opus' review of `sandbox-r1` found that the **production** worker jail had no
PID namespace: the accepted `kala-process` launcher builds the jail after
`fork` and before `execve` and has no PID-1 transition, so `native_sandbox_spec`
dropped the `pid_namespace` control and a production worker shared the host
process table. The Python module isolates PIDs, so this was a security
regression that must not ship.

The repair adds the missing transition **without touching the accepted
`kala-process` crate**: a tiny native init helper (`kala-sandbox-init`) that
the accepted supervisor launches *instead of* the worker. The helper performs
the Python transition, runs the worker inside the new PID namespace and tears
the namespace down with the jail. The accepted crate is consumed read-only
through its path dependency, exactly as before.

---

## 2. The design

### 2.1 The chain

```
supervisor (kala-process)  ──▶  kala-sandbox-init  ──▶  namespace init (PID 1)  ──▶  worker
        (level -1)                    (level 0)                (level 1)            (level 2)
```

| Level | Process | Role |
|---|---|---|
| −1 | the accepted `kala_process::supervise` parent | the deadline, the cancellation token, the output caps, the process group, the reaping of level 0 |
| 0 | `kala-sandbox-init` | the process the supervisor started: unshares the namespaces, unshares the PID namespace, forks, waits for level 1, ends exactly as the worker ended |
| 1 | the namespace init | PID 1 of the new PID namespace: builds the jail (mounts, the `/proc` of the new namespace, the `chroot`, the working directory), forks the worker, forwards the termination signals to it, reaps it and every orphan |
| 2 | the worker | the process the caller asked for; a child of PID 1, so it is PID 2 in the namespace and an orphan is reparented to the init |

### 2.2 The order of the operations is the Python order

The jail is built by the *same* functions the port already had, split into
three pieces so that both the probe and the production path use them:

| Function | Step |
|---|---|
| `unshare_namespaces(spec)` | the namespaces, in the Python order `user`, `network`, `mount`, `ipc`, `uts` |
| `enter_pid_namespace_with_role()` | `unshare(CLONE_NEWPID)` and the fork that makes a child PID 1 |
| `build_jail(spec)` | `/` made private, the device nodes, the read-only and read-write bindings, the scratch tmpfs, **`/proc`**, the `chroot` and the working directory |

`isolate_process(spec)` is the composition of the three and stays the exact
Python port: the recorded call sequence, the error precedence and the
established-control order are unchanged (the differential compares them).

The `/proc` of the jail is mounted **after** the PID transition, by PID 1
itself, exactly like the Python module. That is what makes the jail's `/proc`
show the namespace processes instead of the host ones: a procfs is bound to
the PID namespace of the task that mounted it.

### 2.3 Why the init helper and not the launcher

The launcher cannot be used for the PID namespace:

* `kala_process::SandboxSpec::validate_strict()` **refuses** a specification
  that requests `pid_namespace` (`"pid_namespace is not implemented by the
  native launcher; the PID-1 double-fork transition is pending"`), and the
  accepted crate must not be edited;
* the launcher mounts `/proc` *before* the PID namespace exists, so the jail
  would keep the host's procfs and the worker would still see the host process
  table.

The init helper owns the whole jail, so the launcher keeps its own job (the
launch, the deadlines, the cancellation, the containment boundary) and the
helper is the child it supervises.

### 2.4 The status channel, and a kernel fact worth writing down

The init of a PID namespace **cannot be killed by a fatal signal sent from
inside its namespace** — the kernel ignores it (only `SIGKILL` from an
ancestor namespace is delivered). So PID 1 cannot repeat the death of a worker
that was killed by a signal, and the status would be flattened to
`128 + signal`.

The init therefore writes the wait status of the worker into a pipe
(`O_CLOEXEC`, so the worker never keeps it open) and exits with the
conventional code. The process the supervisor started — which is *not* the
init of a namespace — reads the status and repeats the signal, so the
supervisor observes `ExitStatus::signalled(S)`, exactly as it did when it
launched the worker directly. The pipe is also the reason the init can report
a signal death at all: the intermediate code path is unchanged.

### 2.5 The containment rules

* The worker is a child of PID 1, so an orphan is reparented to the namespace
  init and reaped by it (`waitpid(-1, …)`).
* The kernel tears the whole namespace down when PID 1 dies, so no descendant
  can outlive the jail.
* PID 1 and the worker arm `PR_SET_PDEATHSIG = SIGKILL` after their own
  `fork`, so the namespace dies with the process the supervisor started. The
  classic "the parent died before the signal was armed" race cannot be closed
  for PID 1, because its parent lives in the parent PID namespace and
  `getppid()` is not comparable across the boundary (see the limits). It *is*
  closed for the worker, whose parent is the namespace init.
* Every level of the chain stays in the process group of the direct child, so
  the supervisor's `SIGKILL` (its deadline and its cancellation) reaches all
  of them.
* `SIGTERM`, `SIGINT`, `SIGHUP` and `SIGQUIT` are forwarded down the chain
  instead of killing the receiver, so a worker can shut down in an orderly
  way. `SIGKILL` cannot be caught and always reaches every level.

### 2.6 Failing closed

The production path refuses to hand a worker a jail that is weaker than the
specification:

* a specification that does not request `pid_namespace` is refused by
  `WorkerJail::process_spec` (`the production worker jail requires the
  pid_namespace control`);
* a control that was requested but could not be established refuses the worker
  (`the production worker jail could not establish the <control> control`).
  The Python implementation continues without such a control and reports it;
  the production path does not, because the control that hides the host
  process table (`procfs`) is exactly the one that can fail softly;
* a jail that cannot be built at all exits with `121` and the Python
  diagnostic `sandbox_setup_failed: SandboxSetupError` on standard error.

`native_sandbox_spec`, the adapter to the accepted launcher, now **refuses** a
specification that requests `pid_namespace` instead of silently dropping the
control: the launcher must not build a jail that is weaker than the caller
asked for. A specification without the control is still accepted by the
launcher, which is the explicit way to ask for a jail without a PID
namespace.

---

## 3. Public API

### Added

```
WorkerCommand                       the executable and the argv of a jailed worker
WorkerJail                          the production worker jail (init helper, command, spec)
InitRequest                         the two requests of the init helper (probe, worker)
InitRefusal                         the message and the exit code of a refused request
parse_init_arguments                the command line of the init helper
run_init                            the entry point of the init helper
run_worker                          build the jail of a payload and run the worker in it
run_worker_in_jail                  build the jail of a specification and run the worker in it
arm_parent_death_signal             the parent-death signal of the jail
PidRole                             the side of the PID transition the caller is on
unshare_namespaces                  the first step of the jail
enter_pid_namespace_with_role       the second step of the jail
build_jail                          the third step of the jail
launcher_supported_controls         the controls the accepted launcher can enforce alone
```

### Changed

| Item | Before (r1) | After (r2) |
|---|---|---|
| `native_supported_controls()` | every optional control **except** `pid_namespace` | every optional control, `pid_namespace` included: the native production path enforces it |
| `native_sandbox_spec(spec)` | dropped `pid_namespace` and returned the weaker launcher jail | **refuses** a specification that requests `pid_namespace`; a specification without it is still accepted |
| `enter_pid_namespace()` | `Result<(), SandboxSetupError>` | unchanged signature, unchanged behaviour; the role-aware variant is new |
| `isolate_process(spec)` | one function | the same function, composed of the three new steps; the recorded call sequence, the error precedence and the established-control order are unchanged |

The crate is not wired into the backend yet (no runtime cutover), so the API
change is contained in this crate.

---

## 4. Changed paths

Only the cell's crate and this cell's folder were touched:

```
backend/crates/kala-sandbox/Cargo.toml             (modified: the kala-sandbox-init bin target)
backend/crates/kala-sandbox/README.md              (rewritten)
backend/crates/kala-sandbox/src/isolation.rs       (modified: the three steps, PidRole)
backend/crates/kala-sandbox/src/lib.rs             (modified: the exports, the control lists, the adapter)
backend/crates/kala-sandbox/src/probe_run.rs       (modified: the helper discovery)
backend/crates/kala-sandbox/src/worker.rs          (new: the production worker jail)
backend/crates/kala-sandbox/src/bin/kala-sandbox-init.rs (new: the native init helper)
backend/crates/kala-sandbox/tests/differential.rs  (modified: one new differential test)
backend/crates/kala-sandbox/tests/security.rs      (modified: one rewritten test, one new test)
backend/crates/kala-sandbox/tests/pid_namespace.rs (new: the production-path tests)
.mece/waves/rust-backend-20261019/cells/sandbox-r2/REPORT.md (new)
```

`Cargo.lock` is unchanged: the repair adds no dependency. No other file in the
repository was edited: no root manifest, no root or sibling lock, no Python
source or test, no shared fixture, no adapter, no `studio/**`, and **no file
of the accepted `kala-process` crate**.

---

## 5. Test counts and results

### Rust gates (all green)

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean (no warnings) |
| `cargo test --locked` (debug) | **60 passed, 0 failed** (15 unit + 5 differential + 10 PID-namespace + 30 security) |
| `cargo test --locked --release` | **60 passed, 0 failed** |
| `cargo build --release` | ok |
| `cargo check --target x86_64-pc-windows-gnu` | ok (the target is installed on this host; **no Windows execution is claimed**) |

### The live Python differential

The oracle is the repository's own Python, run **test-only** with
`/home/v9i/Projects/Kala-Agent/.venv/bin/python -I` (isolated interpreter) and
`PYTHONPATH=src`; the oracle installs `src` on `sys.path` itself because `-I`
ignores `PYTHONPATH` by design. The Rust crate never invokes Python.

**182 cases, 0 unexplained mismatches.** The corpus is unchanged from r1: a
hand-written valid and hostile corpus plus a deterministic fixed-seed
generator (an LCG seeded `0x5A17C0DE12345678`).

| Area | Cases | Compared |
|---|---|---|
| `platform_name` | 1 | the string |
| `interpreter_layout` | 1 | the interpreter facts the live Python reports |
| `child_resource_paths` | 1 | the ordered, de-duplicated path list |
| `resolve_system_bindings` | 1 | the ordered binding list |
| `build_sandbox_spec` | 59 | the payload **bytes** (key order included) or the refusal message |
| `SandboxSpec.from_payload` | 59 | the payload bytes or `the sandbox specification is malformed` |
| `SandboxProbe.to_payload` | 35 | the payload bytes, including the three-decimal rounding |
| `SandboxVerdict.to_payload` | 2 | the payload bytes |
| `judge_sandbox` | 8 | the payload bytes and the refusal reason |
| `_unshadowed_scratch_paths` | 1 | the surviving scratch list |
| `_exit_code_from_status` | 8 | the integer |
| `os.strerror` | 1 | the operating-system reason of 13 error codes |
| `_probe_entry` | 3 | the decoded specification and the canary, or the malformed error |
| `run_probe` | 2 | the exit code and the exact stdout line |

### New differential evidence: the PID-namespace jail is the Python jail

`live_python_and_the_port_build_the_same_pid_namespace_jail` builds the *same*
specification in both implementations — the same controls (including
`pid_namespace`), the same bindings, the same workspace — and compares the
fingerprint each of them observes from the inside of its jail:

| Compared | Result |
|---|---|
| the established controls | identical |
| the verified controls | identical |
| the failed controls | identical |
| the entries of the jail root | identical |
| `getpid() == 1` | true on both sides |

The two earlier jail comparisons still hold: the same calls in the same order
with the same flags (`live_python_and_the_port_build_the_same_jail`) and the
same eleven verified controls on the same host
(`live_python_and_the_port_probe_the_same_host`).

### The repository's own Python tests (unchanged, still green)

`tests/unit/test_sandbox_branches.py` (104 tests) and, with the executor
suites that consume the sandbox (`tests/security/test_tool_executor.py`,
`tests/unit/test_executor_branches.py`): **277 passed** on the repository
sources, which this cell did not touch.

---

## 6. Security evidence

### 6.1 The namespace exists and the host process table is invisible

`the_production_worker_runs_in_a_pid_namespace` launches a real worker
(`/bin/sh`) through the production path and reads `/proc` **from inside the
jail**:

```
pid=2 ppid=1
entry=/proc/1
entry=/proc/2
```

* the worker's parent is PID 1 → the init of the namespace is PID 1;
* the worker is PID 2 → it is a child of the init, not the init itself;
* `/proc` contains **only** the namespace processes (1 and 2): the host
  process table is invisible, and the host pid of the test is not among them.

### 6.2 The whole namespace dies, with no orphans

| Test | What it does | Evidence |
|---|---|---|
| `the_deadline_terminates_the_whole_namespace` | a 700 ms wall deadline against a sleeping worker | `ProcessEnd::TimedOut`, and no process of the jail is left in the host process table |
| `the_cancellation_terminates_the_whole_namespace` | the token is cancelled while the worker runs | `ProcessEnd::Cancelled`, and no process of the jail is left |
| `an_orphan_of_the_worker_does_not_outlive_the_jail` | the worker leaves a grandchild behind and exits at once | the grandchild does not outlive the namespace |
| `the_output_of_the_worker_is_bounded` | the worker floods standard output | `ProcessEnd::OutputLimitExceeded(Stdout)` |

The "no orphan" check is a scan of `/proc/*/cmdline` for a unique marker that
every level of the chain carries, repeated until the marker is gone (10 s
budget). It therefore covers the supervisor's child, the namespace init, the
worker and any descendant the worker left behind.

### 6.3 The exit status is the pre-repair behaviour

| Worker | Pre-repair (the launcher execs the worker) | After the repair |
|---|---|---|
| `exit 7` | `Exited(7)`, `python_exit_code() == 7` | `Exited(7)`, `python_exit_code() == 7` |
| `kill -SEGV $$` | `Exited(signalled(11))`, `python_exit_code() == -11` | `Exited(signalled(11))`, `python_exit_code() == -11` |

The signal case is the reason for the status channel of section 2.4: the
namespace init cannot die from a fatal signal, so the signal travels through
the pipe and is repeated by the process the supervisor started.

The output bounds are the accepted supervisor's, unchanged: the capture caps,
the truncation and the `OutputLimitExceeded` decision are the ones the rest of
the backend already uses, and the init helper adds no output of its own.

### 6.4 The live probe reports the control for the production path

`the_live_probe_reports_the_pid_namespace_for_the_production_path` runs the
probe with the **production binary** as the helper (the init helper answers
the probe protocol), and the probe reports on this host:

```
SandboxProbe { available: true, platform: "linux-x86_64",
  controls: ["chroot_workspace_only", "device_nodes_minimal", "ipc_namespace",
    "mount_namespace", "network_isolation", "pid_namespace", "procfs",
    "readonly_system_bindings", "user_namespace", "uts_namespace",
    "workspace_read_write"], detail: "", elapsed_ms: 25.3 }
```

`pid_namespace` is verified, and the empty `detail` means that every control
the probe asked for was established. The live Python probe of the repository
reports the same eleven controls on the same host
(`live_python_and_the_port_probe_the_same_host`).

### 6.5 The refusals are preserved

Every refusal of r1 is unchanged, because the Python-facing code paths were
not touched: the malformed-specification message, the strict-profile control
precedence, the relative-path refusal, the probe messages, the exit codes
`121`/`122` and the `sandbox_setup_failed: SandboxSetupError` diagnostic.

Two new refusals were added, both fail-closed:

| Refusal | Where |
|---|---|
| `the production worker jail requires the pid_namespace control` | `WorkerJail::process_spec` |
| `the production worker jail could not establish the <control> control` | the init helper, before the worker runs |
| `the accepted launcher cannot enforce the pid_namespace control; use the native init helper for the production worker jail` | `native_sandbox_spec` |

---

## 7. Intentional differences (the complete list for this repair)

1. **The production worker jail is built by the init helper, not by the
   launcher.** The launcher has no PID-1 transition and mounts `/proc` before
   the namespace exists. The deadline, the cancellation and the output bounds
   are still the launcher's.
2. **The production path is stricter than the Python module.** A
   specification without `pid_namespace` is refused, and a control that could
   not be established refuses the worker. The Python module runs the worker
   without the controls it could not establish and reports them.
3. **`native_sandbox_spec` refuses instead of weakening.** It used to drop the
   `pid_namespace` control; a caller that asks for the control now gets an
   error instead of a weaker jail.
4. **The supervisor sees the process the supervisor started.** The direct
   child is the init helper, not the worker. The exit status is the worker's,
   byte for byte, and the containment boundary is unchanged: the process group
   of the direct child holds every level of the chain.
5. **The setup failure of the production path is an exit status, not an
   error.** The launcher reported a jail that could not be built as
   `ProcessError::SandboxSetup`; the init helper reports it the Python way —
   `sandbox_setup_failed: SandboxSetupError` on standard error and the exit
   code `121` — which the accepted compatibility mapping turns into
   `sandbox_unavailable` with the same message. The *reason* of the failure is
   not written to standard error, exactly like the Python pre-exec hook, so no
   host path reaches the captured stderr of a worker.
6. **A worker that cannot be executed is reported differently.** The launcher
   reported `ProcessError::Spawn`; the init helper writes the reason to
   standard error and exits with `127`, because the `execve` happens two
   levels below the supervisor and the launcher cannot see it any more.
7. **The bootstrap deadline no longer covers the jail setup.** The launcher
   built the jail before the exec handshake, so a hang there was a bootstrap
   timeout; the jail is built after the handshake, so a hang there is a wall
   deadline. The total time is still bounded by the wall deadline.
8. **The PID-namespace jail has one more process.** The jail of r1 was the
   worker; the jail of r2 is the worker, its namespace init and the process
   the supervisor started.

The differences of r1 (the native probe helper, the supplied interpreter
facts, the native child paths, the type-checked specification fields, the
non-finite JSON values, the lone surrogate, the recursion limit, the exact
`round`, the non-blocking network probe, the `verify_controls_with` rule, the
bounded probe output, the `os.strerror` wording, the absence of an import-time
C library load) are unchanged and still listed in the r1 report.

---

## 8. Remaining gaps and limits

* **No runtime cutover.** The Python executor still calls
  `kala_agent.tools.sandbox`; this crate is not wired into the backend yet and
  no readiness is claimed.
* **No Windows runtime.** `x86_64-pc-windows-gnu` compiles; the Windows
  behaviour of the strict profile is "unavailable", and no Windows execution
  was performed.
* **The parent-death race of the namespace init is not closed.** PID 1 arms
  `PR_SET_PDEATHSIG` a few instructions after its `fork`; if the process the
  supervisor started dies inside that window, the init would survive it. The
  classic `getppid()` comparison cannot be used, because the parent of a
  namespace init lives in the parent PID namespace and `getppid()` is not
  comparable across the boundary. The window is a few microseconds wide, and
  every path the supervisor controls (its deadline, its cancellation, its
  cleanup) kills the whole process group, which reaches the init as well. The
  race **is** closed for the worker, whose parent is the namespace init.
* **A host without PID namespaces cannot run the production jail.** The
  production path requires the control, so a host whose probe does not verify
  it refuses the worker instead of running it in a jail that shares the host
  process table. The probe still reports the strict profile as available in
  that case, because `pid_namespace` is an optional control in the Python
  control table; the refusal happens at the production path, where it
  matters.
* **The reason of a setup failure is not in the captured stderr.** It is the
  Python behaviour, and it keeps host paths out of the worker's diagnostics.
  The exit code `121` and the class name are the signals.
* **`isolate_process` must still run in a single-threaded child.** The port
  keeps the CPython contract. The production path therefore uses the init
  helper, which builds the jail in a forked, single-threaded child.
* **The differential is host-dependent.** The live probe test asserts that
  both implementations report the same controls on *this* host. On a host that
  cannot create a user namespace, both implementations report the profile as
  unavailable and the test asserts that neither advertises a control. The
  jail-plan comparison needs a host that can create a user namespace; on a
  host that cannot, the test fails loudly instead of silently passing.
* **The `kala-sandbox-inspect` helper is a test fixture**, not a product
  entry point. The library never requires it to exist. The
  `kala-sandbox-init` helper is a product entry point of the production
  worker path, but the library never requires it to exist either: a caller
  that wants the jail asks for the path it installed.
* **The scope stops at the sandbox boundary.** Request envelopes, JSON
  schemas, policy, budgets, tool registries, HTTP, worker pools, artifact
  classification and Solid Edge are not in this crate.
