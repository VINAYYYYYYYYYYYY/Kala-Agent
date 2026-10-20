# kala-sandbox

The strict Linux sandbox jail, the preflight probe that decides whether the
host can enforce it, the production worker jail that runs a worker inside a
PID namespace, and the refusal policy that guards side effects.

This crate is a complete port of `src/kala_agent/tools/sandbox.py`. It is a
**standalone workspace** with its own `Cargo.lock`; the accepted root workspace
under `backend/` and the accepted sibling crates are consumed read-only through
path dependencies. Nothing outside `backend/crates/kala-sandbox/` is edited.

## What it owns

* The namespace/mount/chroot jail (`isolate_process` and its helpers).
* The **production worker jail**: the native init helper (`kala-sandbox-init`)
  that performs the PID-1 transition, runs the worker inside the namespace and
  tears the namespace down with the jail (`WorkerJail`).
* The strict-profile probe (`probe_sandbox`, `run_probe` and the native
  `kala-sandbox-probe` helper).
* The refusal policy (`judge_sandbox`, `SandboxVerdict`).
* The payload shapes (`SandboxSpec`, `SandboxProbe`) with the exact Python
  bytes.

Request envelopes, JSON schemas, policy, budgets, tool registries, HTTP, worker
pools, artifact classification and Solid Edge are **not** here: they belong to
the tools executor.

## Public API

```rust
use std::collections::BTreeSet;
use kala_sandbox::{
    IsolationProfile, SandboxSpec, build_sandbox_spec, judge_sandbox, platform_name,
    probe_sandbox, ProbeOptions,
};

// The host description, the same string the Python module reports.
assert_eq!(platform_name(), "linux-x86_64");

// A specification is refused unless every required control was probed.
let controls: BTreeSet<String> = kala_sandbox::STRICT_REQUIRED_CONTROLS
    .iter()
    .map(|control| (*control).to_owned())
    .collect();
let spec = build_sandbox_spec(
    &controls,
    std::path::Path::new("/run/jail/root"),
    std::path::Path::new("/run/jail/workspace"),
    &[],
    1 << 20,
)?;

// The strict profile is advertised only after a probe verified it.
let probe = probe_sandbox(&ProbeOptions::new("/opt/kala/bin/kala-sandbox-probe"));
let verdict = judge_sandbox(IsolationProfile::StrictLinux, Some(&probe));
assert!(verdict.allowed || !verdict.reason.is_empty());
```

Core types and functions:

- `SandboxSpec` — the resolved jail: controls, root, workspace, read-only and
  read-write bindings, device bindings, scratch paths and the scratch size.
  `to_payload`/`to_json` reproduce the Python payload **byte for byte**,
  including the key order; `from_payload` refuses every malformed document
  with `the sandbox specification is malformed`.
- `SandboxProbe` — the outcome of a probe: availability, platform, the
  verified controls, a sanitised detail and the elapsed milliseconds.
- `SandboxVerdict` — the decision whether a side effect may proceed.
- `IsolationProfile` — `portable` or `strict_linux`.
- `build_sandbox_spec`, `build_sandbox_spec_with` — the specification builder
  with the exact Python error precedence (missing controls before relative
  paths).
- `resolve_system_bindings`, `child_resource_paths`, `InterpreterLayout` — the
  host paths a jailed worker must reach.
- `isolate_process` — build the jail in the calling process.
- `unshare_namespaces`, `enter_pid_namespace`, `enter_pid_namespace_with_role`,
  `build_jail`, `PidRole` — the three steps of the jail, separately.
- `child_pre_exec`, `child_pre_exec_or_exit` — the pre-exec hook, with the
  Python diagnostic and the exit code `121`.
- `WorkerJail`, `WorkerCommand` — the production worker jail: the
  specification, the init helper and the command the worker runs.
- `InitRequest`, `InitRefusal`, `parse_init_arguments`, `run_init`,
  `run_worker`, `run_worker_in_jail` — the protocol of the native init helper.
- `probe_sandbox`, `ProbeOptions`, `run_probe`, `probe_entry`,
  `emit_probe_result` — the probe protocol.
- `judge_sandbox` — the refusal policy, with the Python refusal texts.
- `verify_controls`, `verify_controls_with`, and the individual probes
  (`canary_is_invisible`, `workspace_is_writable`, `system_bindings_are_readonly`,
  `procfs_is_namespaced`, `scratch_is_writable`, `device_list_is_minimal`,
  `network_is_isolated`).
- `native_sandbox_spec` — the adapter to the accepted `kala-process` launcher.
  It **refuses** a specification that requests `pid_namespace`, because the
  launcher has no PID-1 transition and must not build a weaker jail silently.
- `native_supported_controls` — the controls the native production path
  enforces; `launcher_supported_controls` — the controls the accepted launcher
  can enforce on its own.

## The jail

`isolate_process` establishes the requested controls in the calling process:

1. the namespaces, in the order `user`, `network`, `mount`, `ipc`, `uts`;
2. the PID transition, when `pid_namespace` is requested: the child forks
   again, becomes PID 1 inside the new namespace and the intermediate process
   mirrors its exit status;
3. `/` is made private when the mount namespace is established;
4. the device nodes are bound when `device_nodes_minimal` is requested;
5. the read-only and read-write bindings are established;
6. the scratch tmpfs is mounted when `scratch_tmpfs` is requested, and it is
   unmounted again when it turns out not to be writable;
7. `/proc` is mounted when `procfs` is requested;
8. the root is `chroot`ed, the working directory is moved to `/` and then to
   the workspace.

A failure raises `SandboxSetupError` with a short, sanitised reason that names
the failing operation and the operating-system reason, for example
`unshare(user_namespace) failed with errno 1 (Operation not permitted)`. The
caller then fails closed: the worker is never executed.

`isolate_process` changes the calling process, so it must run in a freshly
forked, single-threaded child or in the single-threaded probe helper.

## The production worker jail

The accepted `kala-process` launcher builds a jail after `fork` and before
`execve`, but it has no PID-1 transition: a worker it launches shares the host
process table. The production path therefore launches the **native init helper**
(`kala-sandbox-init`) instead of the worker, through the same accepted
supervisor:

```
supervisor ──▶ kala-sandbox-init ──▶ namespace init (PID 1) ──▶ worker
```

* The helper unshares the namespaces of the specification in the Python order,
  unshares the PID namespace and forks, so that the child becomes PID 1 of the
  new namespace.
* PID 1 builds the jail — the mounts, the `/proc` **of the new namespace**, the
  `chroot` and the working directory — exactly like the Python
  `isolate_process`, and then forks the worker, forwards the termination
  signals to it and reaps it and every orphan it leaves behind.
* The process the supervisor started waits for PID 1 and ends exactly as the
  worker ended, so the supervisor observes the same exit status it observed
  when it launched the worker directly.
* The worker is a child of PID 1, so an orphan is reparented to the namespace
  init; the kernel tears the whole namespace down when PID 1 dies; every level
  of the chain stays in the process group of the direct child, so the
  supervisor's `SIGKILL` reaches all of them.
* The production path fails closed: a specification that does not request the
  `pid_namespace` control is refused, and a control that could not be
  established refuses the worker instead of running it in a weaker jail.

The deadline, the cancellation token and the output bounds are the ones the
accepted supervisor enforces; the init helper inherits them end to end.

## The probe

`probe_sandbox` runs the native `kala-sandbox-probe` helper through the
accepted `kala-process` supervisor, so the launch, the wall-clock deadline
(`PROBE_TIMEOUT_SECONDS`), the cancellation token and the output bounds are the
ones the rest of the backend already uses. The helper builds exactly the jail
an executor would build and then verifies, from the inside, that the network
is unreachable, the host filesystem is invisible and the workspace is
writable. Only the controls the probe observed are returned.

The probe is a *preflight*: it proves that the host can enforce the strict
profile. It is not a runtime guard for a specific call.

## Platform

The strict profile is Linux only. On every other platform `probe_sandbox`
reports the profile as unavailable and `judge_sandbox` refuses it. The
portable profile is unaffected and keeps its explicit wording: `the portable
profile confines a process, not the filesystem`.

`x86_64-pc-windows-gnu` compiles; no Windows execution is claimed.

## Tests

* `cargo test --locked` — 60 tests: 15 unit, 5 differential, 10 PID-namespace,
  30 security.
* The differential test runs the live Python implementation from `src` as an
  oracle over 182 cases and compares the payload bytes, the refusal messages,
  the error precedence, the jail plan, the PID-namespace jail fingerprint and
  the probe answer.
* The PID-namespace tests run a real worker inside the production jail and
  check the namespace, the exit status, the deadline, the cancellation, the
  output bounds and the absence of orphans.
* `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --locked` (debug and release), `cargo build --release` and
  `cargo check --target x86_64-pc-windows-gnu` all pass.

The Python implementation is never executed by the library; it is used only
by the test suite as an oracle.
