# tools-executor-r1 — native tool executor (SECURITY-CRITICAL)

Date: 2026-10-20
Cell: `tools-executor-r1` (wave `rust-backend-20261019`)
Crate: `backend/crates/kala-tools` (module `executor`)
Implementer: OpenCode Go **MiniMax M3.1 Flash (Free)**
Status: implemented and validated on this Linux host; ready for independent review.

No commit, push, publish, deploy, delegation, credential read, runtime cutover,
Windows runtime execution, speed claim or readiness claim.

---

## 1. Scope and ownership

Owned and changed only:

```
backend/crates/kala-tools/Cargo.toml
backend/crates/kala-tools/Cargo.lock
backend/crates/kala-tools/README.md
backend/crates/kala-tools/src/lib.rs
backend/crates/kala-tools/src/executor/mod.rs
backend/crates/kala-tools/src/executor/artifact.rs
backend/crates/kala-tools/src/executor/dispatch.rs
backend/crates/kala-tools/src/executor/error.rs
backend/crates/kala-tools/src/executor/paths.rs
backend/crates/kala-tools/src/executor/profile.rs
backend/crates/kala-tools/src/executor/report.rs
backend/crates/kala-tools/src/bin/kala-tools-sandbox-init.rs
backend/crates/kala-tools/src/bin/kala-tools-sandbox-probe.rs
backend/crates/kala-tools/src/registry/mod.rs          (two additive members, no behaviour change)
backend/crates/kala-tools/tests/oracle.py               (one new `executor` mode)
backend/crates/kala-tools/tests/support/mod.rs          (three new helpers)
backend/crates/kala-tools/tests/executor_branches.rs
backend/crates/kala-tools/tests/executor_differential.rs
backend/crates/kala-tools/tests/executor_security.rs
.mece/waves/rust-backend-20261019/cells/tools-executor-r1/REPORT.md
```

Not touched: the root `backend/Cargo.toml` and the root `backend/Cargo.lock`,
any accepted crate source or manifest (`kala-json`, `kala-contracts`,
`kala-budget`, `kala-kernel`, `kala-process`, `kala-sandbox`, …), the Python
product or its tests, `adapters/`, `studio/**`, the other wave cells.

The crate keeps its own standalone workspace and its own `Cargo.lock`; the new
path dependencies were resolved into that lockfile only.

---

## 2. What was delivered

A complete native port of `src/kala_agent/tools/executor.py` (1,915 lines) into
a new `executor` module of the accepted `kala-tools` crate (3,694 lines of
Rust across seven files).

### Public API

| Python member | Rust member |
| --- | --- |
| `MAX_WORKERS`, `MAX_DEFAULT_TIMEOUT_SECONDS`, `MINIMUM_TIMEOUT_SECONDS`, `MAX_ERROR_MESSAGE_LENGTH`, `MAX_QUEUE_SECONDS`, `MINIMUM_QUEUE_SECONDS` | `kala_tools::executor::{MAX_WORKERS, …}` |
| `_APPROVAL_BINDING_VERSION` | `APPROVAL_BINDING_VERSION` |
| `_EXTRA_ENVIRONMENT_KEYS`, `_NETWORK_CAPABILITIES`, `_EGRESS_CAPABILITIES`, `_FORBIDDEN_CAPABILITIES` | `EXTRA_ENVIRONMENT_KEYS`, `NETWORK_CAPABILITIES`, `EGRESS_CAPABILITIES`, `FORBIDDEN_CAPABILITIES` |
| `_PORTABLE_CONTROLS` | `PORTABLE_CONTROLS` |
| `TEXTUAL_MEDIA_SUFFIXES`, `_BINARY_SIGNATURES`, `_CLASSIFICATION_HEADER_BYTES` | `TEXTUAL_MEDIA_SUFFIXES`, `BINARY_SIGNATURES`, `CLASSIFICATION_HEADER_BYTES` |
| `ToolExecutorError`, `ToolAuthorityError`, `ToolIsolationError` | `ToolExecutorError` with the variants `Executor`, `Authority`, `Isolation` and `Workspace`; `python_class_name()` reports the Python class |
| `IsolationMode` (`AUTO`, `SUBPROCESS`, `TRUSTED_IN_PROCESS_EVALUATOR`) | `IsolationMode` (`Auto`, `Subprocess`, `TrustedInProcessEvaluator`) |
| `SandboxProfile` (13 members, `__post_init__`, `enforced_controls`, `timeout_for`) | `SandboxProfile` with the same members, `validated()`, `enforced_controls()`, `timeout_for()` |
| `ToolExecutionReport` (12 members, `to_payload`) | `ToolExecutionReport` with the same members and `to_payload()` |
| `ToolExecutor` (21 public members) | `ToolExecutor` with the same members, plus `approval_digests()` and the public `verify_response()` / `verify_artifacts()` |
| `ToolExecutor.__init__` keyword arguments | `ExecutorConfig` |
| `ToolExecutor.execute(..., timeout_seconds=, cancel=)` | `ToolExecutor.execute(&call, &ExecutionOptions)` |
| `_ApprovalBinding` | the private `ApprovalBinding` |
| `_CallRejected` | the private `CallRejected` |
| `_derive_classification`, `_classify_header`, `_probe_is_textual` | `derive_classification`, `classify_header` |
| `_verify_artifacts`, `_read_artifact_once`, `_confine` | `verify_artifacts`, `read_artifact_once`, `confine` |
| `_call_component`, `_ensure_directory`, `_prepare_run_directory` | `call_component`, `ensure_directory`, `prepare_run_directory` |
| `_normalize_capabilities`, `_normalize_import_roots` | `normalize_capabilities`, `normalize_import_roots` |
| `_child_limits`, `_child_pre_exec`, `_BoundedStreamReader`, `_terminate_process_group`, `_has_exited`, `_reap`, `_send_request` | the accepted `kala-process` supervisor (read-only) — the bounds, the process group, the reaping and the descriptor close are the accepted implementation |
| `_dispatch_in_process`, `_dispatch_subprocess`, `_supervise` | `executor::dispatch::{dispatch_in_process, dispatch_subprocess, classify_outcome}` |

The crate root re-exports `ExecutionOptions`, `ExecutorConfig`, `IsolationMode`,
`SandboxProfile`, `ToolExecutionReport`, `ToolExecutor` and `ToolExecutorError`.

### The accepted crates are used, not re-implemented

* `kala-json` owns the canonical bytes, the digests and the CPython float
  formatting (`python_float` uses `kala_json::format_float`, so a deadline is
  reported as `2.0` and not as `2`).
* `kala-contracts` owns the typed call, descriptor, artifact, result and
  timestamp models.
* `kala-process` owns the bounded supervisor: the launch, the wall-clock and
  bootstrap deadlines, the cancellation, the output caps, the process group,
  the `SIGKILL` containment, the reaping and the child resource limits
  (`RLIMIT_CORE 0/0`, `RLIMIT_CPU n/n+1`, `RLIMIT_FSIZE = max_artifact_bytes`,
  `RLIMIT_NOFILE 256/256`, umask `0077` — the same four limits and the same
  umask as the Python `_child_limits`).
* `kala-sandbox` owns the strict jail, the preflight probe and the refusal
  verdict; the executor consumes `judge_sandbox`, `probe_sandbox`,
  `build_sandbox_spec_with`, `child_resource_paths` and `WorkerJail`.
* `kala-budget` owns the ledger (`reserve`, `commit_with`, `release`).
* `kala-kernel` owns the policy engine (`PolicyRequest::from_parts`,
  `PolicyEngine::decide`).

### Two thin launchers

`kala-tools` ships two binary targets that are **launchers only**:

* `kala-tools-sandbox-probe` calls `kala_sandbox::run_probe`;
* `kala-tools-sandbox-init` calls `kala_sandbox::run_init`.

They own no jail logic. The jail, the PID-1 transition and the control
verification stay in the accepted `kala-sandbox` crate, so the control the
worker is given is exactly the control the accepted production worker jail
establishes. The accepted crate's own binaries are not built by this crate's
build, so the executor needs a reachable helper; the executor looks for it next
to the running program and `ExecutorConfig` can name it explicitly.

### The two additive members of the registry

`RegistryHandler::subprocess_handler()` (a defaulted method) and
`RegisteredTool::handler_handle()` (an owned handle) were added so the
executor can build the worker command from the entry point and can dispatch a
handler on another thread. No existing behaviour changed: the default of
`subprocess_handler` is `None` and the protocol, registry and simulator
suites are unchanged and green.

---

## 3. The security controls and their evidence

No control was weakened. Where the accepted crates lacked an API, the
executor refuses the run instead of running unsandboxed.

| Control | How it is enforced | Test |
| --- | --- | --- |
| Capability authorization against the registry | `require_authority` compares the required capabilities with the granted set and refuses the tool with `ToolAuthorityError`; `authorize` refuses a call that requests an ungranted or an undeclared capability | `executor_branches::a_missing_capability_is_denied_before_the_workspace_is_created`, `executor_branches::an_undeclared_capability_is_denied`, `executor_security::the_executor_refuses_a_tool_that_claims_ungranted_authority` |
| The default refusal of the network, the egress and the privileged capabilities | `check_capability_policy` and `check_side_effect_policy`, with the Python messages | `executor_security::the_network_and_the_egress_capabilities_are_denied_by_default`, the `call-network` and `call-egress` differential cases |
| The side effect must match the descriptor | `authorize` refuses a mismatch with `side_effect_mismatch` | `executor_branches::the_side_effect_must_match_the_descriptor` |
| The isolation mode never runs tool code inside the kernel by accident | `select_isolation` refuses an in-kernel handler under `Auto` and under `Subprocess`; `TrustedInProcessEvaluator` needs a registered first-party pure evaluator | `executor_branches::the_default_mode_never_runs_a_tool_inside_the_kernel`, `executor_branches::the_trusted_mode_runs_a_registered_pure_evaluator` |
| The strict jail is refused when the host cannot establish it | `sandbox_unavailable` consults `kala_sandbox::judge_sandbox`; a side-effecting tool is refused with `sandbox_unavailable` | `executor_security::the_strict_profile_runs_the_worker_in_the_jail_it_verified` (both branches) |
| The worker runs inside a PID namespace with a PID 1 that reaps its orphans | the strict path runs the worker through `kala_sandbox::WorkerJail`, whose init helper performs the PID-1 transition and refuses a jail that could not establish a requested control | `executor_security::the_strict_profile_runs_the_worker_in_the_jail_it_verified` (asserts `pid_namespace` and `procfs` in the verified and in the enforced controls), the `call-strict` differential case |
| The preflight probe is the only source of a claimed control | `SandboxProfile::enforced_controls` adds the probe controls only when the profile is strict **and** the probe is available | `executor_security::the_portable_profile_never_claims_a_filesystem_control` |
| The portable profile never claims a filesystem control | the portable probe reports `available = false`, `platform = "not-probed"` | `executor_security::the_portable_profile_never_claims_a_filesystem_control` |
| The wall-clock deadline | the deadline is the smaller of the call deadline and the descriptor deadline, capped by the profile; the accepted supervisor enforces it | the `call-hang` differential case (both sides report `timeout`), `executor_branches::the_deadline_is_the_smallest_one_and_is_capped_by_the_profile` |
| The cancellation | the token is observed before the launch, during the launch and while the worker runs | `executor_branches::a_cancelled_call_never_dispatches`, the `call-cancelled` differential case |
| The process-group containment | the accepted supervisor kills the process group on **every** outcome, so a descendant of a worker that ended cannot outlive the call | `executor_differential::a_timed_out_tool_leaves_no_descendant_that_still_writes` (the stray child never writes), the `call-stray` and `call-lingering` differential cases |
| The host process table is clean after a run | the differential reads the worker pid of every report and checks `/proc/<pid>` | `executor_differential::the_native_executor_agrees_with_the_repository_python_on_every_case` |
| The output bound | the accepted supervisor caps stdout at `max_output_bytes` and the executor refuses an in-process envelope above the bound | the `call-flood` differential case (both sides report `output_too_large`), `executor_security::the_output_bound_is_enforced` |
| The argument bound | the canonical arguments are re-validated after the approval | the `call-oversized` differential case (both sides report `invalid_arguments`) |
| The artifact bound | the artifact is read through one descriptor, so the size check, the digest check and the classification see the same bytes | `executor_security::an_artifact_outside_the_workspace_is_refused`, `executor_security::a_symlinked_artifact_is_refused`, `executor_security::a_duplicate_artifact_identifier_is_refused` |
| The artifact confinement | the path may not be relative, may not leave the workspace and may not traverse a symlink; the final component is opened with `O_NOFOLLOW` | the same three tests and the `call-escape` differential case (both sides report `artifact_rejected`) |
| The kernel classifies the artifact, not the tool | the declared classification can never downgrade the verdict of the kernel | `executor_security::an_unrecognised_artifact_format_is_classified_as_a_design_binary` (eleven cases), the `call-liar` differential case |
| A tool may not report a decision only the kernel can make | `normalize_tool_error_code` maps an unknown or a kernel-only code to the internal error | `executor_security::the_response_envelope_of_a_failed_run_cannot_report_an_error_code_the_kernel_does_not_know`, the `call-unrecognized` differential case |
| The child environment is minimal and workspace scoped | `build_child_environment` with the protected keys of the protocol layer | `executor_security::the_child_environment_is_minimal_and_workspace_scoped`, `executor_security::the_environment_keys_the_executor_owns_cannot_be_overridden` |
| The worker concurrency cap | the executor holds at most `max_workers` slots and refuses a call with `worker_capacity_exhausted` when the pool is saturated and the queueing wait expired | `executor_security::the_worker_pool_is_capped_and_a_saturated_pool_denies`, the `call-saturated` differential case |
| A call identifier cannot address a run directory component | `call_component` refuses an empty, an oversized, a `.`/`..`, a separated, a null-bearing and a control-character-bearing identifier | `executor_security::a_call_identifier_cannot_address_a_run_directory_component` |
| The approval is bound to the exact request | the binding covers the argument digest, the workspace, the effective capabilities and the tool, descriptor and schema digests | the differential compares the three digests of every case |
| The provenance record carries no secret | the twelve members of the report payload | `executor_branches::the_report_payload_carries_no_secret` |
| The child resource limits | the accepted supervisor applies the four POSIX limits and the `0077` umask | the accepted `kala-process` suite; the executor enables them through `ProcessLimits::enforce_resource_limits` |

### The sandbox control the accepted crates did not offer

The accepted `kala-process` launcher builds the jail after `fork` and before
`execve` and has **no PID-1 transition**, so a worker it launches shares the
host process table. `kala_sandbox::native_sandbox_spec` refuses a
specification that requests `pid_namespace` for exactly that reason.

The Python executor applies `child_pre_exec(spec)` in the forked child, and
`child_pre_exec` performs the PID namespace transition, so the Python worker
runs as PID 1 of a new namespace.

The port therefore uses `kala_sandbox::WorkerJail`, which launches the accepted
native init helper instead of the worker: the helper performs the PID-1
transition, the worker becomes a child of the namespace init, an orphan is
reparented to PID 1 and reaped there, and the kernel tears the namespace down
when PID 1 dies. **The control is not weakened**; it is the control the
`sandbox-r2` cell repaired.

---

## 4. The task requirements, one by one

| Requirement | Where it is preserved | Evidence |
| --- | --- | --- |
| The request/response envelope flow | `approval_binding` builds the request envelope, `dispatch_subprocess` writes it to the worker's standard input, the answer is read back and rebuilt by `verify_response` | the whole differential corpus |
| Capability authorization against the registry | `require_authority`, `authorize`, `check_capability_policy`, `check_side_effect_policy` | the `call-missing-capability`, `call-network` and `call-egress` cases; `executor_security` |
| Argument and result validation | `prepare_dispatch` re-validates the arguments against the input schema and the canonical byte bound; `verify_response` validates the result against the output schema | the `call-oversized` and `call-liar` cases |
| Worker bootstrap and child environment | the accepted `WORKER_BOOTSTRAP` and `build_child_environment` of the protocol layer; the environment is minimal, the home and the temporary directory are the workspace and the protected keys cannot be replaced | `executor_security::the_child_environment_is_minimal_and_workspace_scoped`, `executor_security::the_environment_keys_the_executor_owns_cannot_be_overridden` |
| Deadlines and cancellation | `SandboxProfile::timeout_for`, the accepted supervisor's wall-clock deadline, the cancellation token | the `call-hang`, `call-stray` and `call-cancelled` cases |
| Output bounds and stderr sanitization | `max_output_bytes` on the supervisor, the 65,536 byte stderr sample, `sanitize_child_stderr` | the `call-flood` and `call-absent-import` cases |
| Crash handling | the exit status of the worker, `tool_crashed` with the sanitized stderr | the `call-crash` and `call-absent-import` cases |
| Hang handling | the wall-clock deadline, `timeout` | the `call-hang` case |
| Fork-bomb handling | the process group of the worker is killed on **every** outcome, so a worker that forks cannot outlive the call | the `call-lingering` case (the grandchild never writes) and the process-table check of the whole corpus |
| Stray-child handling | the same process-group kill | `executor_differential::a_timed_out_tool_leaves_no_descendant_that_still_writes` |
| Oversize-output handling | the stdout cap of the supervisor, `output_too_large` | the `call-flood` case |
| Malformed-response handling | `ToolResponseEnvelope::from_json`, `protocol_error` with the class of the failure and the sanitized stderr | the `call-absent-import` case (a worker that never answers) |
| Provenance and digest records | `ToolExecutionReport`, the three approval digests | every case of the differential compares the record and the digests |
| Every refusal message and error precedence | the decision order of `execute` and the messages of every refusal | the decision table of section 5 and the `executor_branches` suite |

---

## 5. The live differential

`tests/executor_differential.rs` runs every case twice on this host: once
through the repository's own Python executor (the `executor` mode of
`tests/oracle.py`) and once through the native executor. Both sides use the
**same run directory and the same call identifier**, so the workspace path, and
therefore the approval digests, are comparable byte for byte.

The comparison covers, field by field:

* the run status;
* the error code;
* the sanitized diagnostic (the error message);
* the canonical result payload;
* the artifacts (identifier, uri, digest, media type, size, classification);
* the provenance record (the twelve members of the payload; the process
  identifier is compared by its presence, because a pid is different on every
  run);
* the three approval digests (the argument digest, the state digest and the
  action digest);
* the preflight probe (availability, verified controls, platform);
* the enforced controls.

The host process table is checked after every run: the worker pid of every
report is read from `/proc/<pid>/stat` and must be gone.

### The corpus (23 cases)

| Group | Cases |
| --- | --- |
| The reference simulator | `call-success` (with a report and an artifact), `call-read` |
| The authorization stage | `call-absent` (unknown tool), `call-missing-capability`, `call-side-effect` (mismatch), `call-network`, `call-egress` |
| **Every misbehaviour entry point** | `call-crash` (`crash_invoke`), `call-flood` (`flood_invoke`), `call-liar` (`liar_invoke`), `call-escape` (`escape_invoke`), `call-unrecognized` (`unrecognized_code_invoke`), `call-hang` (`hang_invoke`), `call-stray` (`stray_child_invoke`), `call-lingering` (`lingering_child_invoke`) |
| The worker that cannot be started | `call-absent-import` (an entry point that does not exist) |
| The lifecycle | `call-cancelled` (cancelled before the dispatch) |
| The trusted in-process evaluator | `call-evaluator` |
| The bounds | `call-oversized` (the argument bound) |
| The pool | `call-saturated` (one worker, a 50 ms queueing wait, a hanging tool) |
| The policy approval | `call-approval-required` (the approval was not recorded), `call-approved` (the same call after the approval) |
| The strict profile | `call-strict` (the strict Linux jail) |

### The result

**Zero unexplained mismatches.** Every one of the 23 cases produced the same
status, the same error code, the same diagnostic, the same result payload,
the same artifacts, the same provenance record, the same three digests, the
same probe verdict and the same enforced controls on the two implementations.

The observed decisions are the ones the Python implementation produces:

| Case | Status | Error code |
| --- | --- | --- |
| `call-success` | `succeeded` | — |
| `call-read` | `succeeded` | — |
| `call-absent` | `denied` | `unknown_tool` |
| `call-missing-capability` | `denied` | `capability_denied` |
| `call-side-effect` | `denied` | `side_effect_mismatch` |
| `call-network` | `denied` | `network_denied` |
| `call-egress` | `denied` | `egress_denied` |
| `call-crash` | `failed` | `tool_crashed` |
| `call-flood` | `failed` | `output_too_large` |
| `call-liar` | `failed` | `invalid_result` |
| `call-escape` | `failed` | `artifact_rejected` |
| `call-unrecognized` | `failed` | `tool_error` |
| `call-absent-import` | `failed` | `tool_crashed` |
| `call-hang` | `timed_out` | `timeout` |
| `call-stray` | `timed_out` | `timeout` |
| `call-lingering` | `succeeded` | — |
| `call-cancelled` | `cancelled` | — |
| `call-evaluator` | `succeeded` | — |
| `call-oversized` | `failed` | `invalid_arguments` |
| `call-saturated` | `timed_out` | `timeout` |
| `call-approval-required` | `denied` | `approval_required` |
| `call-approved` | `succeeded` | — |
| `call-strict` | `succeeded` | — |

The decisions are asserted by
`executor_differential::the_corpus_reports_the_expected_decisions`, so a
change of a decision is a test failure and not a silent drift.

The `call-lingering` case is the proof that the containment boundary is the
process group: the worker answers at once and leaves a grandchild behind; the
grandchild never writes its side effect, because the supervisor kills the
group on the successful exit as well
(`executor_differential::a_successful_tool_leaves_no_descendant_that_still_writes`).

The `call-strict` case is the proof that the strict jail is the Python jail:
the two implementations report the same verified controls
(`chroot_workspace_only`, `device_nodes_minimal`, `ipc_namespace`,
`mount_namespace`, `network_isolation`, `pid_namespace`, `procfs`,
`readonly_system_bindings`, `user_namespace`, `uts_namespace`,
`workspace_read_write`) and the same enforced controls, and the run succeeds
inside the jail on both sides.

The `call-absent-import` case is the proof that the sanitized diagnostic is the
Python one: both sides report `tool_crashed` with the message
`the tool process exited with status 1: Traceback (most recent call last): | ...
| AttributeError: module 'kala_agent.tools.simulator' has no attribute
'absent_invoke'` — the first line, the ellipsis and the last line of the
traceback, exactly as `sanitize_child_stderr` keeps them.

### The timing classes

The deadline and the crash are distinguished exactly as the Python
implementation distinguishes them: the `call-hang` and the `call-stray` cases
are `timed_out` with the code `timeout` on both sides, and the `call-crash`
and the `call-absent-import` cases are `failed` with the code `tool_crashed`
on both sides.

---

## 6. The test counts and the gates

From `backend/crates/kala-tools`:

| Suite | Tests |
| --- | --- |
| `unittests src/lib.rs` | 18 |
| `tests/protocol_branches.rs` | 45 |
| `tests/protocol_differential.rs` | 13 |
| `tests/protocol_pattern_differential.rs` | 3 |
| `tests/protocol_security.rs` | 10 |
| `tests/registry_branches.rs` | 38 |
| `tests/registry_differential.rs` | 3 |
| `tests/registry_security.rs` | 9 |
| `tests/simulator_branches.rs` | 40 |
| `tests/simulator_differential.rs` | 3 |
| `tests/simulator_security.rs` | 10 |
| `tests/executor_branches.rs` | 14 (new) |
| `tests/executor_differential.rs` | 6 (new) |
| `tests/executor_security.rs` | 17 (new) |
| **Total** | **229** (192 before this cell, 37 new; 23 differential cases) |

The gates, all run in `backend/crates/kala-tools`:

```
cargo fmt --check                                        # clean
cargo clippy --all-targets --all-features -- -D warnings # clean
cargo test --locked                                      # 229 passed, 0 failed
cargo test --locked --release                            # 229 passed, 0 failed
cargo build --locked --release                           # ok
cargo check --locked --target x86_64-pc-windows-gnu      # ok
cargo check --locked --target x86_64-pc-windows-msvc     # ok
```

The Windows targets are installed on this host and the cross-check of the
library, the two binaries and the tests **compiles**. **No Windows runtime is
executed and no Windows runtime claim is made**: the strict profile is Linux
only, and the portable profile on Windows uses the accepted
`kala-process` job object, whose runtime behaviour is the subject of the
`kala-process` cell, not of this one.

The repository Python is unchanged and green: `tests/security/test_tool_executor.py`
and `tests/unit/test_executor_branches.py` report **173 passed**.

---

## 7. The intentional differences

Every difference is listed here and in the crate README. None of them weakens
a control.

1. **The interpreter of the worker.** Python's executor starts the worker with
   `sys.executable`, which the language provides. Rust has no `sys.executable`,
   so the interpreter is a configured value
   (`ExecutorConfig::python_executable`). A subprocess dispatch without one is
   refused with an isolation error, and the strict profile additionally
   refuses a dispatch without an interpreter. The Python implementation falls
   back to `os.path.realpath(sys.executable)` in exactly that case.
2. **The type validations of the profile.** Python validates the *types* of
   the profile members ("max_workers must be an integer", "allow_network must
   be a boolean", "extra_environment must map strings to strings"). Rust has
   no variant of those values, so those four messages exist in the port but
   cannot be reached. Every other validation message and its order is
   reproduced.
3. **The deadline override.** Python's `execute` accepts a `timeout_seconds`
   override and never reads it: the deadline is the smaller of the deadline
   the call declares and the deadline of the descriptor, capped by the
   profile. The port keeps the parameter and the behaviour, so a caller that
   relied on the override is not surprised by a different deadline.
4. **The internal failure of the approval binding.** Python lets a
   `JsonEnvelopeError` of the approval binding propagate out of `execute`. The
   port reports it as a failed run with the internal error code, because a
   `ToolResult` is the only value the port returns.
5. **The process group and the session.** Python starts the worker with
   `start_new_session=True`, so the worker leads a new session *and* a new
   process group. The accepted `kala-process` supervisor starts the worker
   with `setpgid(0, 0)`, so the worker leads a new process group inside the
   session of the supervisor. The boundary the executor relies on — the
   process group, which is killed on every outcome — is the same; the session
   of the worker is not.
6. **The result sink.** Python's `result_sink` is `Callable[[Path], str]` and
   its return value is discarded. The port drops the return value as well.
7. **The strict jail.** Python builds the jail in the forked child, before the
   worker is executed. The port runs the worker through the accepted native
   init helper, which performs the PID-1 transition the accepted launcher does
   not have. The control set is the one the preflight probe verified; a
   control the host could not establish is never requested and never claimed.
8. **The `ToolExecutorError` hierarchy.** Python raises three exception
   classes of its own and the protocol layer's `ToolWorkspaceError`. The port
   has one error enum with four variants; `python_class_name()` reports the
   class the Python implementation would have raised, so the differential can
   compare the class and the message.
9. **The `TypeError` of the constructor.** Python's constructor raises
   `TypeError` for a registry, a policy or a sink of the wrong type. Rust has
   no variant of those values, so those three messages cannot be reached.

---

## 8. The ADR conformance

The port keeps the decisions of ADR-0005 and ADR-0006 that the Python module
implements, and it does not weaken any of them:

* **ADR-0005 D1–D2** — the typed descriptor, the typed call and the typed
  result are the accepted `kala-contracts` models; the envelopes and their
  digests are the accepted protocol layer.
* **ADR-0005 D3–D5** — the capability closure, the least-privilege defaults
  (no network, no egress, no privileged capability), the separate
  authorization of the side effects and the `TOOL_CLASS_CHANGE` checkpoint are
  reproduced by `authorize`, `check_capability_policy`,
  `check_side_effect_policy` and `request_for`.
* **ADR-0005 D6** — tool code never runs in the kernel process: the default
  mode refuses an in-kernel handler, and the trusted in-process mode is
  restricted to a registered first-party pure evaluator.
* **ADR-0005 D7** — the sandboxing is layered and fail-closed: the portable
  profile keeps the process isolation, the minimal environment, the resource
  limits and the process-group kill; the strict profile is advertised only
  after a probe verified it from the inside, and a side effect the profile
  cannot back is refused with the reason.
* **ADR-0005 D8** — the bounds are enforced outside the tool: the deadline,
  the cancellation, the output cap, the artifact cap and the child resource
  limits are applied by the supervisor.
* **ADR-0005 D9** — the result is provisional until it is verified: the call
  id, the tool identity, the status, the result schema, the artifact
  references, the digests, the sizes and the classifications are all checked
  before the result is exposed.
* **ADR-0005 D10** — the worker concurrency is bounded and the saturation
  denies (`worker_capacity_exhausted`) instead of queueing without bound.
* **ADR-0005 D11** — the provenance record is written for every dispatch and
  carries no secret.
* **ADR-0006 D5** — the sandboxing and the provenance stay independent
  controls: the record names the controls the host enforced, and it never
  claims a control the probe did not verify.

---

## 9. The remaining gaps

1. **The interpreter of the product worker.** The port runs the *Python*
   worker, because the reference simulator and the misbehaviour entry points
   are Python modules and the differential compares the two implementations on
   the same worker. A native worker needs a native executable and a native
   entry point; the crate does not ship one, and no runtime cutover is
   claimed.
2. **The helpers of the strict profile.** The executor needs the native probe
   helper and the native init helper. They are shipped as two thin launchers
   of this crate over the accepted `kala-sandbox` entry points, and they have
   to be reachable at run time. A deployment that installs the product
   elsewhere has to install them next to the product or name them in
   `ExecutorConfig`.
3. **The interpreter layout of the strict jail.** The jail binds the host
   paths the worker interpreter needs read-only. Rust has no `sysconfig`, so
   the facts are supplied explicitly through
   `ExecutorConfig::interpreter_layout`; the default is the layout of the
   running program. The differential feeds the live interpreter's own facts
   through the same code, so the two jails hold the same paths.
4. **The Windows runtime.** The library compiles for
   `x86_64-pc-windows-gnu`; no Windows run was performed. The strict profile
   is Linux only, exactly as in the Python implementation.
5. **The dependency note on `kala-kernel`.** The `kernel-review` cell
   recommended a repair (D1: `PolicyEngine::decide` drops the accountable
   actor from a non-approved decision). The executor reads the status and the
   reason of a decision and never the actor, so D1 does not change an
   executor decision; the executor consumes the accepted crate read-only and
   did not patch it.
6. **`kala-budget` and `kala-kernel` are workspace members of the root
   `backend/` workspace.** They are consumed as path dependencies and their
   sources and manifests were not edited; the new entries landed in the
   `kala-tools` lockfile only.

---

## 10. The commands a reviewer can run

```bash
cd backend/crates/kala-tools
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo check --locked --target x86_64-pc-windows-gnu
```

The live differential needs the repository interpreter:

```bash
cd /home/v9i/Projects/Kala-Agent
.venv/bin/python -I -m pytest tests/security/test_tool_executor.py tests/unit/test_executor_branches.py -q
```

The oracle is test-only. The library itself never invokes Python and never
starts a subprocess other than the tool worker it supervises.
