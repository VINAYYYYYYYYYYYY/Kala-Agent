# sandbox-r1 — `kala-sandbox` (Rust port of `kala_agent.tools.sandbox`)

**Status:** complete for the named module. No runtime cutover is claimed, no
Windows runtime is claimed, no speed or readiness claim is made.

**Cell:** `sandbox-r1` · **Wave:** `rust-backend-20261019` · **Date:** 2026-10-20

---

## 1. What was built

A new standalone crate `backend/crates/kala-sandbox` that ports
`src/kala_agent/tools/sandbox.py` (844 lines: 20 names in `__all__`, 16
public definitions and 26 private helpers) completely, and that consumes the
accepted
`backend/crates/kala-process` read-only as a path dependency for everything the
Python module does through `subprocess` (process launching, the wall-clock
deadline, cancellation and the output bounds).

The crate owns:

| Area | Python original | Rust port |
|---|---|---|
| Constants | `CLONE_NEW*`, `MS_*`, `ISOLATION_FAILURE_EXIT`, `PROBE_FAILURE_EXIT`, `PROBE_TIMEOUT_SECONDS`, `STRICT_REQUIRED_CONTROLS`, `STRICT_OPTIONAL_CONTROLS` | `src/constants.rs` (same values) |
| Errors | `SandboxError`, `SandboxSetupError`, `SandboxProbeError` | `src/error.rs` (+ `FailureClass`, the CPython exception-name mapping the probe text needs) |
| Types | `IsolationProfile`, `SandboxProbe`, `SandboxSpec`, `SandboxVerdict` | `src/profile.rs`, `src/probe.rs`, `src/spec.rs` |
| Policy | `platform_name`, `child_resource_paths`, `resolve_system_bindings`, `build_sandbox_spec`, `judge_sandbox` | `src/lib.rs`, `src/resources.rs`, `src/spec.rs`, `src/probe.rs` |
| Jail | `isolate_process` and its 21 private helpers (`_unshare`, `_mount`, `_make_mountpoint`, `_bind_readonly`, `_bind_read_write`, `_bind_device`, `_unshadowed_scratch_paths`, `_mount_scratch`, `_mount_scratch_set`, `_enter_pid_namespace`, `_close_inherited_fds`, `_exit_code_from_status`, `_libc_error`, `_network_is_isolated`, `_device_list_is_minimal`, `_verify_controls`, `_canary_is_invisible`, `_workspace_is_writable`, `_system_bindings_are_readonly`, `_procfs_is_namespaced`, `_scratch_is_writable`) | `src/isolation.rs`, `src/probe.rs` (all public, so the same behaviour is testable) |
| Probe | `probe_sandbox`, `run_probe`, `_probe_entry`, `_emit_probe_result`, `_unavailable` | `src/probe_run.rs` + the native helper `src/bin/kala-sandbox-probe.rs` |
| Hook | `child_pre_exec` | `src/isolation.rs` (`child_pre_exec`, `child_pre_exec_or_exit`) |

### Public API

```
platform_name()                                   IsolationProfile
SandboxSpec  SandboxProbe  SandboxVerdict         SandboxError  SandboxSetupError
SandboxProbeError  FailureClass                  JsonValue  JsonParseError
build_sandbox_spec  build_sandbox_spec_with       resolve_system_bindings
resolve_system_bindings_with  child_resource_paths InterpreterLayout
isolate_process  child_pre_exec  child_pre_exec_or_exit
unshare  mount  make_mountpoint  bind_readonly  bind_read_write  bind_device
mount_scratch  mount_scratch_set  unshadowed_scratch_paths
enter_pid_namespace  close_inherited_fds  exit_code_from_status  libc_error
os_reason  start_recording  take_recorded_calls
probe_sandbox  ProbeOptions  run_probe  probe_entry  emit_probe_result
discover_helper  probe_paths  judge
judge_sandbox  verify_controls  verify_controls_with
canary_is_invisible  workspace_is_writable  system_bindings_are_readonly
procfs_is_namespaced  scratch_is_writable  device_list_is_minimal
network_is_isolated  truncate_detail  unavailable_probe  control_is_known
native_sandbox_spec  native_supported_controls
parse_json  escape_text  dump_float  dump_string_list  round_three
posix_text  native_text  resolve_strict_false  realpath
CLONE_NEW*  MS_*  ISOLATION_FAILURE_EXIT  PROBE_FAILURE_EXIT
PROBE_TIMEOUT_SECONDS  STRICT_REQUIRED_CONTROLS  STRICT_OPTIONAL_CONTROLS
UNSHARE_ORDER  NAMESPACE_CONTROLS  DEVICE_NODES  SCRATCH_PATHS
OPTIONAL_SYSTEM_DIRECTORIES  OPTIONAL_SYSTEM_FILES  CANARY_PAYLOAD
MIN_DEVICE_BYTES  MAX_PROBE_DETAIL  all_controls  is_known_control
namespace_flag
```

### Changed paths

Only new files, all inside the cell's crate and its cell folder:

```
backend/crates/kala-sandbox/Cargo.toml            (new, standalone workspace)
backend/crates/kala-sandbox/Cargo.lock            (new, own lock)
backend/crates/kala-sandbox/README.md             (new)
backend/crates/kala-sandbox/src/lib.rs            (new)
backend/crates/kala-sandbox/src/constants.rs      (new)
backend/crates/kala-sandbox/src/error.rs          (new)
backend/crates/kala-sandbox/src/isolation.rs      (new)
backend/crates/kala-sandbox/src/json.rs           (new)
backend/crates/kala-sandbox/src/parse.rs          (new)
backend/crates/kala-sandbox/src/path.rs           (new)
backend/crates/kala-sandbox/src/probe.rs          (new)
backend/crates/kala-sandbox/src/probe_run.rs      (new)
backend/crates/kala-sandbox/src/profile.rs        (new)
backend/crates/kala-sandbox/src/resources.rs      (new)
backend/crates/kala-sandbox/src/spec.rs           (new)
backend/crates/kala-sandbox/src/bin/kala-sandbox-probe.rs    (new, native probe child)
backend/crates/kala-sandbox/src/bin/kala-sandbox-inspect.rs  (new, test-only inspector)
backend/crates/kala-sandbox/tests/differential.rs (new)
backend/crates/kala-sandbox/tests/security.rs     (new)
backend/crates/kala-sandbox/tests/python_oracle.py(new, test-only oracle)
.mece/waves/rust-backend-20261019/cells/sandbox-r1/REPORT.md (new)
```

No other file in the repository was edited: no root manifest, no root or
sibling lock, no Python source or test, no shared fixture, no adapter, no
`studio/**`. The accepted `backend/Cargo.toml` still resolves
(`cargo metadata --no-deps` and `cargo check --workspace` pass) and the new
crate is not a member of it.

---

## 2. Test counts and differential results

### Rust gates (all green)

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean (no warnings) |
| `cargo test --locked` (debug) | **48 passed, 0 failed** (15 unit + 4 differential + 29 security) |
| `cargo test --locked --release` | **48 passed, 0 failed** |
| `cargo build --release` | ok |
| `cargo check --target x86_64-pc-windows-gnu` | ok (target installed on this host; **no Windows execution claimed**) |

### Live Python differential

The oracle is the repository's own Python, run **test-only** with
`/home/v9i/Projects/Kala-Agent/.venv/bin/python -I` (isolated interpreter) and
`PYTHONPATH=src`; the oracle installs `src` on `sys.path` itself because `-I`
ignores `PYTHONPATH` by design. The Rust crate never invokes Python.

**182 cases, 0 unexplained mismatches.** The corpus mixes hand-written valid
and hostile cases with a deterministic fixed-seed generator (an LCG seeded
`0x5A17C0DE12345678`) that produces 48 specification builds, 48 hostile
specification payloads and 24 probe payloads.

What the corpus compares, byte for byte:

| Area | Cases | Compared |
|---|---|---|
| `platform_name` | 1 | the string |
| `interpreter_layout` | 1 | the interpreter facts the live Python reports (the *input* of the two cases below, not a comparison) |
| `child_resource_paths` | 1 | the ordered, de-duplicated path list, computed by the Rust port from those facts |
| `resolve_system_bindings` | 1 | the ordered binding list for a real import root |
| `build_sandbox_spec` | 59 | the payload **bytes** (key order included) or the refusal message |
| `SandboxSpec.from_payload` | 59 | the payload bytes or `the sandbox specification is malformed` |
| `SandboxProbe.to_payload` | 35 | the payload bytes, including the three-decimal rounding of `elapsed_ms` |
| `SandboxVerdict.to_payload` | 2 | the payload bytes |
| `judge_sandbox` | 8 | the payload bytes and the refusal reason |
| `_unshadowed_scratch_paths` | 1 | the surviving scratch list |
| `_exit_code_from_status` | 8 | the integer |
| `os.strerror` | 1 | the operating-system reason of 13 error codes (`EPERM`, `ENOENT`, `EIO`, `EBADF`, `EACCES`, `EBUSY`, `EEXIST`, `EINVAL`, `ENOSPC`, `EPIPE`, `ENOTSUP`), so the refusal texts name the same reasons as CPython |
| `_probe_entry` | 3 | the decoded specification and the canary, or the malformed error |
| `run_probe` | 2 | the exit code and the exact stdout line |

### Differential of the real jail, not only of the policy

Two further tests compare the **effect** of the jail, not only its decisions:

1. **`live_python_and_the_port_build_the_same_jail`** — the Python
   implementation is driven with a recording stand-in for the C library, so its
   exact operating-system call sequence is observable without changing the
   oracle's namespaces. The Rust port records the same calls through a
   diagnostic recorder (`start_recording` / `take_recorded_calls`) while it
   builds the *real* jail in a child process. The two sequences are compared
   element by element: the same calls, in the same order, with the same
   targets and the same flags — including `MS_BIND | MS_REC`,
   `MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV` for the read-only
   bindings, `MS_BIND | MS_REMOUNT | MS_NOSUID | MS_NODEV` for the writable
   ones, `MS_REC | MS_PRIVATE` for the private root and `size=<n>` for the
   scratch tmpfs. **Identical.**
2. **`live_python_and_the_port_probe_the_same_host`** — the live Python
   `probe_sandbox()` and this crate's `probe_sandbox()` run against the same
   host. Both report `available=True`, `platform=linux-x86_64` and the **same
   eleven verified controls**:
   `chroot_workspace_only, device_nodes_minimal, ipc_namespace,
   mount_namespace, network_isolation, pid_namespace, procfs,
   readonly_system_bindings, user_namespace, uts_namespace,
   workspace_read_write`.

### The repository's own Python tests (unchanged, still green)

`tests/unit/test_sandbox_branches.py` (104 tests) and, with the executor
suites that consume the sandbox (`tests/security/test_tool_executor.py`,
`tests/unit/test_executor_branches.py`): **277 passed** on the repository
sources, which this cell did not touch.

---

## 3. Compatibility and security evidence

### Every refusal is preserved

| Python refusal | Rust port | Evidence |
|---|---|---|
| `the strict profile requires controls that were not probed: …` | identical text, identical precedence (missing controls are reported **before** a relative path) | differential (58 build cases) + `the_strict_profile_requires_every_control_before_it_looks_at_the_paths` |
| `sandbox paths must be absolute` | identical text and literal `startswith("/")` test (not `Path::is_absolute`) | differential + `the_strict_profile_refuses_a_relative_path` |
| `the sandbox specification is malformed` | identical text for every missing key, wrong type and non-integer size | differential (60 payload cases) + `a_malformed_specification_is_refused_with_one_message` |
| `the probe payload is malformed` | identical text, exit code `122` | differential (3 probe-entry cases) + `the_probe_helper_refuses_a_malformed_payload` |
| `the strict profile is not available on this host, so side effects are refused[: <detail>]` | identical text, including the `detail` suffix rule | differential (12 judge cases) + `the_strict_profile_is_refused_without_a_probe` |
| `the portable profile confines a process, not the filesystem` | identical text, empty control tuple | differential + `the_portable_profile_never_claims_a_filesystem_control` |
| `the strict profile is unavailable on this host` | identical text for a non-zero probe exit | `the_probe_refuses_a_helper_that_fails` |
| `the sandbox probe did not finish` | identical text for the deadline | `the_probe_gives_up_when_the_helper_overruns_the_deadline` |
| `the sandbox probe returned no result` | identical text for an unreadable result | `the_probe_refuses_a_helper_that_reports_nothing`, `the_probe_bounds_the_output_of_the_helper` |
| `a required sandbox control was not verified` | identical text | `the_probe_refuses_a_helper_that_verifies_too_little` |
| `the sandbox probe could not start: <class>` | identical text and the same class names (`OSError`, `ValueError`, `SandboxSetupError`) | `FailureClass` + the probe code path |
| `unshare(<name>) failed with errno <n> (<reason>)` | identical text, identical `strerror` wording (the Rust `(os error n)` suffix is stripped) | `libc_error` / `os_reason` |
| `mount(<target>) failed with errno <n> (<reason>)` | identical text | `mount` |
| `chroot failed with errno …`, `chdir to the sandbox root failed with errno …`, `chdir into the workspace failed with errno …` | identical text | `isolate_process` |
| `the read-only binding <p> does not exist`, `the writable binding <p> is not a directory` | identical text | `bind_readonly`, `bind_read_write` |
| `the sandbox root cannot hold <p>: <reason> (<file>)` | identical text | `make_mountpoint` |
| `sandbox_setup_failed: SandboxSetupError` on stderr, exit `121` | identical diagnostic and exit code | `child_pre_exec_or_exit` + `the_pre_exec_hook_exits_with_the_isolation_failure_code` |

### Hostile-input refusal parity

* A malformed document is refused with one message and never partially
  applied (`a_malformed_specification_is_refused_with_one_message`,
  `a_hostile_specification_never_widens_the_authority`).
* A document nested 2 000 levels deep is refused **without a stack overflow**
  (`a_deeply_nested_document_is_refused_without_overflowing_the_stack`): the
  parser is bounded and iterative.
* A 1 MiB path in a specification is handled without a panic
  (`an_oversized_specification_is_refused_without_allocating_it`).
* The probe helper writes **nothing** when a required control is missing
  (`the_probe_helper_writes_nothing_when_a_required_control_is_missing`),
  exactly like the Python helper.
* The probe's stdout is bounded by the accepted supervisor: a helper that
  floods standard output is stopped at the cap and the result is refused
  (`the_probe_bounds_the_output_of_the_helper`).
* The probe honours the wall-clock deadline through the accepted supervisor
  (`the_probe_gives_up_when_the_helper_overruns_the_deadline`).
* The strict profile never claims a control it did not establish: the
  `native_sandbox_spec` adapter drops `pid_namespace` instead of claiming it
  (`the_native_launcher_never_claims_the_pid_namespace`).

### Unix / Windows distinctions stay explicit

* The strict profile is Linux only. `probe_sandbox` reports
  `the strict profile requires Linux namespaces` and `judge_sandbox` refuses
  it everywhere else.
* `build_sandbox_spec` keeps the Python **literal** `str(path).startswith("/")`
  test, so a Windows path (`C:\work`) is refused exactly as CPython refuses
  it.
* `x86_64-pc-windows-gnu` compiles. The Windows path has no `unshare`, no
  `chroot` and no `mount`; the stubs refuse with the non-Linux reason. **No
  Windows execution is claimed.**

### The jail that is actually built

* The order, the flags and the targets of the jail are compared against the
  live Python implementation (section 2, first test).
* The `kala-sandbox-probe` helper builds the jail in a real child process and
  verifies it from the inside; on this host it verifies all eleven controls,
  including the PID namespace (the child forks again and becomes PID 1) and
  `/proc` (the jail has its own procfs).
* The production worker path uses the accepted `kala-process` launcher, which
  builds the jail after `fork` and before `execve`; the port's
  `isolate_process` is used by the single-threaded probe helper.

---

## 4. Intentional differences (complete list)

1. **The probe child is native, not an interpreter.** The Python module runs
   `python -I -B -c "<entry point>"`; this crate runs the native
   `kala-sandbox-probe` helper. The refusal texts keep the Python wording
   verbatim ("no usable interpreter is available for the sandbox probe") so a
   decision recorded before the migration still explains itself after it. The
   probe child gets the native minimal environment (`HOME`, `LANG`, `PATH`,
   `TMPDIR`, `KALA_PROCESS_PROTOCOL`) instead of the Python interpreter
   variables, which a native helper does not need.
2. **The interpreter facts are supplied, not discovered.**
   `child_resource_paths` takes an `InterpreterLayout`; the native default
   (`InterpreterLayout::current()`) describes the running executable instead
   of `sys.executable`/`sysconfig`. The differential proves the *algorithm* is
   identical by feeding the live interpreter's own facts through it.
   `build_sandbox_spec_with` and `resolve_system_bindings_with` exist for the
   same reason.
3. **The child paths of a native specification are the native ones.** A
   specification built by `build_sandbox_spec` binds the paths of the native
   worker, not the paths of a Python interpreter. The differential builds both
   specifications with the same child paths, so the *rule* is compared exactly.
4. **Specification fields are type checked.** CPython's
   `SandboxSpec.from_payload` coerces `controls` with `frozenset(...)` and the
   binding lists with `tuple(...)`, which accept any iterable of any hashable
   value: a bare string becomes a tuple of characters, an object becomes a
   tuple of its keys, and a number inside a list is carried through to the
   payload. The Rust port requires a JSON array of strings and refuses
   everything else with `the sandbox specification is malformed`. CPython
   accepts those payloads and then fails later, with an uncaught `TypeError`,
   when the jail is built. The five shapes are pinned by
   `the_documented_differences_are_the_only_accepted_hostile_payloads`, which
   asserts both sides: CPython accepts them, the port refuses them.
5. **`NaN`, `Infinity` and `-Infinity` are accepted by the reader.** CPython's
   `json` accepts them; the reader rewrites them to `null`. Every field is
   type checked afterwards, so the outcome is the same malformed-payload
   refusal that `int(nan)` produces in CPython.
6. **An unpaired UTF-16 surrogate escape is refused.** CPython accepts it and
   produces a lone surrogate; the port refuses the document.
7. **A document nested deeper than the parser limit is refused.** CPython
   raises `RecursionError` at roughly 1 000 levels, which `_probe_entry` does
   not catch, so the Python probe child dies with exit status 1. The port
   refuses the payload and exits with `122` — the safer outcome, and no stack
   is used.
8. **`round(value, 3)` is exact.** The port formats the double with three
   fractional digits, which applies CPython's round-half-to-even to the exact
   binary value. (`kala-process` uses a floor-based approximation that differs
   from CPython for values such as `8.835`; the port does not copy it.)
9. **The network probe does not block.** CPython opens a blocking socket; the
   port opens a non-blocking one and treats an in-flight connect as "not
   isolated". The verdict is the same and the probe cannot stall.
10. **The namespace controls are verified against the jail that was built.**
    `verify_controls` is the exact CPython rule (a requested namespace
    control holds). The probe additionally uses `verify_controls_with`, which
    requires the control to appear in the list that `isolate_process` returns,
    so a namespace is never reported as verified unless the `unshare` really
    succeeded.
11. **The production worker jail has no PID namespace.** The accepted
    `kala-process` launcher has no PID-1 double-fork transition, so
    `native_sandbox_spec` drops `pid_namespace` and the control is never
    advertised for a production worker. The probe helper *does* perform the
    transition and verifies it, exactly like the Python module.
12. **The output of the probe is bounded.** CPython's `subprocess.run` reads
    the whole output; the port stops the child at the supervisor's capture cap
    and refuses the result. The refusal text differs from CPython's for that
    one case (`the sandbox probe returned no result` instead of a successful
    parse of an unbounded document).
13. **`os.strerror` wording.** The port uses the Rust `io::Error` text with
    the `(os error n)` suffix removed. The differential compares thirteen error
    codes against CPython's `os.strerror` and the wording is identical.
14. **No import-time C library load.** The Python module loads `libc` through
    `ctypes` at import time and raises
    `the C library could not be loaded for namespace isolation` when it cannot.
    The port links the C library at build time, so that failure mode does not
    exist; the namespace calls report their own `errno` instead.

## 5. Remaining gaps

* **No runtime cutover.** The Python executor still calls
  `kala_agent.tools.sandbox`; this crate is not wired into the backend yet and
  no readiness is claimed.
* **No Windows runtime.** `x86_64-pc-windows-gnu` compiles; the Windows
  behaviour of the strict profile is "unavailable", and no Windows execution
  was performed.
* **The PID namespace is not available to the production worker** (difference
  11). The probe verifies it; the accepted launcher cannot build it.
* **The strict profile is Linux only.** macOS and Windows always report it
  unavailable, exactly like the Python module.
* **`isolate_process` must run in a single-threaded child.** The port keeps
  the CPython contract. The production path therefore uses the accepted
  `kala-process` launcher, which builds the jail after `fork` with no
  allocation in that window.
* **The differential is host-dependent.** The live probe test asserts that
  both implementations report the same controls on *this* host. On a host
  that cannot create a user namespace, both implementations report the profile
  as unavailable and the test asserts that neither advertises a control. The
  jail-plan comparison needs a host that can create a user namespace; on a
  host that cannot, the test fails loudly instead of silently passing.
* **The `kala-sandbox-inspect` helper is a test fixture**, not a product
  entry point. The library never requires it to exist.
* **The oracle's `jail_plan` case replaces the C library** with a recorder, so
  the Python side of that comparison observes the *plan* while the Rust side
  observes the *real* jail. The two agree on this host; a host where a mount
  fails would make the Rust sequence shorter, and the test would report the
  difference instead of hiding it.
* **The scope stops at the sandbox boundary.** Request envelopes, JSON
  schemas, policy, budgets, tool registries, HTTP, worker pools, artifact
  classification and Solid Edge are not in this crate.
