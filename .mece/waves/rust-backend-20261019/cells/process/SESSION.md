# Session log — process cell

- Cell: `.mece/waves/rust-backend-20261019/cells/process`
- Brief: [`TASK.md`](TASK.md)
- Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**.
  T3 task
  `node:delegated-task:command%3Amcp%3A84bb7a8e-861f-4322-aff7-0fcabb65179a%3Adelegate-task%3Arust-backend-20261019-process-r1-go-continue49`
  and child thread
  `thread:delegated-task:command%3Amcp%3A84bb7a8e-861f-4322-aff7-0fcabb65179a%3Adelegate-task%3Arust-backend-20261019-process-r1-go-continue49`.
- Reviewer: GPT-6.1-Sol (not run by this worker).
- No delegation, no child turns, no commit, no push, no deploy, no model or
  provider switch, and no packaging cutover.
- Working directory: `/home/v9i/Projects/Kala-Agent`.

## Scope and ownership

Read `WAVE.md`, `SYNTHESIS.md`, `src/kala_agent/tools/sandbox.py`,
`src/kala_agent/tools/executor.py`, `src/kala_agent/tools/protocol.py`,
`src/kala_agent/config.py`, the sandbox/executor/protocol tests, and the
accepted `backend` workspace README/manifests. Implementation was confined to
the new `backend/crates/kala-process/**` plus this cell's `REPORT.md` and
`SESSION.md`. No root manifest/lock, other crate, Studio, contracts, contracts
fixture, Python product, packaging, frontend, adapter or user file was edited.
The crate is a standalone `[workspace]` with a local `Cargo.lock`; no
`kala-json` path dependency was required.

## Implementation sequence

1. Built the crate skeleton with typed limits, outcomes, errors, environment,
   workspace confinement and public re-exports.
2. Implemented the Unix raw launch/supervision path (`pipe2`, `fork`, `execve`,
   process group, bootstrap handshake, `poll` I/O, `waitid(WNOWAIT)`, reaping).
3. Implemented `PreparedSandbox` and the Linux strict plan (user/net/mount/ipc/
   uts namespaces, uid/gid maps, bind mounts, tmpfs/procfs, chroot/chdir,
   async-signal-safe raw child application).
4. Implemented the separate native probe helper, runtime verification, typed
   probe/spec/verdict payloads and judge/control gating.
5. Implemented the Windows suspended Job Object path and explicit handle list.
6. Added the native fixture, process/sandbox tests, live Python oracle and
   Windows-pending tests.
7. Ran final `fmt`, locked debug/release tests, clippy, release build and
   Windows MSVC/GNU all-target checks.

## Focused failures found and repaired

These were distinct implementation defects found by the tests/source review,
not repeated blind attempts. No blocker remains.

1. **Bootstrap never observed EOF** — the parent retained the child-side pipe
   descriptors after `fork`, keeping the bootstrap writer open. Fixed by
   explicitly closing all child-side descriptors in the parent. A single test
   then completed in 0.02 s.
2. **Concurrent descriptor leak** — normal EOF handling set pipe options to
   `None` without closing the descriptors; the concurrency test observed
   `12 -> 76` open descriptors. Fixed by closing on EOF in the run loop and
   `finish`; the test then returned to the baseline.
3. **Timeout outcome had no exit status** — `finish` reaped with a null status.
   Fixed by decoding `waitpid` status; timeout/escape outcomes now carry the
   observed status where one exists.
4. **Probe argument encoding** — `probe_sandbox` passed the canary as a raw path
   while the helper expects a JSON string, producing `the probe payload is
   malformed` and exit 122. Fixed by JSON-encoding the canary; the strict probe
   then verified all required controls.
5. **Optional `procfs` was fatal** — the helper's procfs mount returns EPERM on
   this host (user namespace without PID namespace), which initially made the
   whole strict profile unavailable. Optional device/scratch/procfs steps are
   now non-fatal in probe mode, removed from advertised controls on failure,
   and remain fail-closed in production. Final probe detail:
   `not enforced: procfs`.
6. **Windows assignment order** — the first Windows draft resumed the suspended
   child before assigning it to the Job Object. Fixed to assign first and only
   then `ResumeThread`; assignment/resume failures terminate the child.
7. **stdout cap could be lost when exit won the race** — if the worker exited
   before the final drain, an `Exited` outcome could hide a truncated stdout.
   Fixed by reclassifying an `Exited` outcome with a truncated capped stdout as
   `OutputLimitExceeded(Stdout)` after `finish` on both platforms.
8. **Sandbox binding-path safety** — added absolute/non-root/traversal checks
   for mount targets and root/workspace host-root refusal during review.

Additional final hardening: a typed `ProcessError::Cleanup` is returned if the
direct child cannot be reaped within the termination window; Windows
pre-resume cancellation/bootstrap paths now report their actual termination
and reaping evidence; strict-only tests are compiled only on Linux so the
Windows all-target check remains warning-free; and the live Python oracle
reports an unavailable host instead of panicking when no usable interpreter is
present. No repeated repairs were needed after these corrections; the final
suites passed. No failure was left unreported.

## Final verification (actual)

Host: Linux 7.2.8-9-omarchy, x86_64; rustc 1.99.1; cargo 1.99.1. All commands
run in `backend/crates/kala-process`.

```sh
cargo fmt --check
# passed

cargo test --offline --locked
# 38 passed, 0 failed, 0 ignored
# 27 tests/process.rs + 1 tests/python_oracle.rs + 10 tests/sandbox.rs
# 0 lib/bin unit tests, 0 doc-tests, 0 Windows tests on Linux

cargo test --offline --locked --release
# 38 passed, 0 failed, 0 ignored (same split)

cargo clippy --offline --locked --all-targets -- -D warnings
# passed

cargo build --offline --locked --release
# passed

cargo check --offline --locked --target x86_64-pc-windows-msvc --all-targets
cargo clippy --offline --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo check --offline --locked --target x86_64-pc-windows-gnu --all-targets
cargo clippy --offline --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings
# all passed; six windows_pending tests compile but are #[ignore]d

KALA_STRICT_SANDBOX=1 cargo test --offline --locked --test sandbox \
  probe_reports_truthfully_and_an_opt_in_strict_run_fails_closed -- --exact --nocapture
# passed; probe available=true

cargo doc --offline --locked --no-deps
# passed
```

Observed strict probe:

```json
{"available":true,
 "controls":["chroot_workspace_only","device_nodes_minimal","ipc_namespace",
             "mount_namespace","network_isolation","readonly_system_bindings",
             "scratch_tmpfs","user_namespace","uts_namespace","workspace_read_write"],
 "detail":"","elapsed_ms":22.514,"platform":"linux-x86_64"}
```

The raw helper with all supported optional controls emitted
`{"controls":[...],"detail":"not enforced: procfs"}` and exit 0. The actual
strict jail test (not a payload mock) verified workspace write success, host
canary read refusal (exit 42), network connect refusal (exit 43) and `/usr`
write refusal (exit 7). The slow-launch cancellation test observed
`Cancelled(Bootstrap)` in five consecutive runs.

Windows has **no runtime proof**: both Windows targets were only compiled and
clippy-checked on this Linux host. `tests/windows_pending.rs` has six tests, all
explicitly `#[ignore = "requires a Windows host; runtime proof is pending"]`.

## Unfinished / handoff notes

- `pid_namespace` is pending the native double-fork PID-1 transition; it is
  rejected explicitly and never advertised.
- `procfs` is unenforced on this host and not advertised.
- Portable Unix cleanup is process-group based; escaped sessions/groups are
  reported as incomplete cleanup, not hidden. Windows Job Objects are the true
  tree boundary (runtime pending).
- `cwd_confined` is not a filesystem jail; only strict Linux provides
  namespace/mount/chroot confinement.
- No worker pool, artifact digest/classification controls, policy, budgets,
  registry, request protocol, HTTP or packaging work is included. No
  performance or full-migration claim.
- Root workspace/lock and all other cell paths remain untouched. No commit,
  push, deploy or cutover was performed.

---

# Repair round 2 session log

- Date: 2026-10-19.
- Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**.
- Trigger: `REVIEW-1.md` withheld acceptance for four concrete defects and one
  liveness guarantee gap.
- No delegation, model/provider switch, commit, push, deploy, cutover or
  unrelated install. Scope remained `backend/crates/kala-process/**` and this
  cell's `REPORT.md`/`SESSION.md`.

## Sequence

1. Re-read the original brief, round-1 report/session, `REVIEW-1.md`, the full
   Unix and Windows supervisors, environment/outcome/limits modules, fixtures,
   process/sandbox/oracle tests and the live Python executor contract.
2. Fixed the Windows stdio direction by replacing the ambiguous `own_pipe`
   tuple with an explicit `create_pipe(child_reads) -> (parent, child)` contract
   and direction-named assignments in `stdio_pipes`. Added a Windows-only unit
   test that inspects inheritance on all six handles and round-trips data
   through stdin, stdout and stderr. It is compiled for MSVC/GNU but ignored
   because no Windows host is available.
3. Made the Unix bootstrap deadline cover pending stdin: pass it into
   `supervise_loop`, check it alongside the wall deadline, and use
   `min(wall, bootstrap)` as the poll timeout while stdin remains open. Added
   the review's 1 MiB/50 ms/600 ms reproduction as a regression test.
4. Closed the public-field environment bypass: `from_entries` now validates
   every key/value, `validate()` runs before launch and before the Windows
   UTF-16 block, and Windows rejects interior-NUL executable/argv/cwd strings.
   Added aggregate-environment and NUL-argv tests.
5. Corrected signal compatibility: `python_exit_code()` is now `-signal` on
   signal death and `shell_exit_code()` carries the explicit `128+signal` view.
   Added a native `SIGKILL` test and extended the live Python oracle to compare
   real `Popen.returncode == -9`.
6. Bounded Unix I/O liveness: 256 KiB read budget, 256 KiB stdin write budget,
   non-blocking parent descriptors immediately after `fork` and before the
   handshake, defensive non-blocking conversion in `finish`, and `Drop` on
   `StdinState` to close a pending stdin descriptor on every early return.
7. Added `flood-forever` and `escapee-flood` fixture modes and two integration
   tests for a continuously ready producer and an escaped pipe writer, plus two
   Unix unit tests for the byte budgets.
8. Ran the full verification matrix and updated README plus this report.

## Focused failure and repair

After the first implementation pass, two new liveness tests failed because the
fast native flood producer reached the 1 MiB stdout cap before the 5/150 ms
deadline window, so the outcome was `OutputLimitExceeded` rather than
`TimedOut`/`Exited`. This was a test-bound selection error, not a supervisor
defect: the cap is the maximum legal 64 MiB and the deadline windows were
shortened to 5 ms so the cap could not be reached first (measured fixture
throughput is multi-GB/s). Both tests then passed in debug and release. No
other focused repair was needed; no blocker remains.

## Verification actually run

```sh
cargo fmt --all -- --check
cargo test --offline --locked
cargo test --offline --locked --release
cargo clippy --offline --locked --all-targets -- -D warnings
cargo build --offline --locked --release
cargo check --offline --locked --target x86_64-pc-windows-msvc --all-targets
cargo clippy --offline --locked --target x86_64-pc-windows-msvc --all-targets -- -D warnings
cargo check --offline --locked --target x86_64-pc-windows-gnu --all-targets
cargo clippy --offline --locked --target x86_64-pc-windows-gnu --all-targets -- -D warnings
KALA_STRICT_SANDBOX=1 cargo test --offline --locked --test sandbox -- --nocapture
cargo doc --offline --locked --no-deps
```

Results: debug and release each **46 passed, 0 failed, 0 ignored** (2 lib unit,
33 process, 1 live Python oracle, 10 sandbox); fmt, clippy, release build, both
Windows all-target check/clippy runs and rustdoc passed. Strict sandbox run:
10 passed, probe available with the same ten verified controls; `procfs` and
`pid_namespace` still unadvertised.

## Handoff

Windows runtime remains unproven, including the newly corrected pipe direction
and Job Object cleanup. `pid_namespace` and `procfs` remain explicit gaps.
Portable Unix still cannot contain a descendant that leaves the process group;
the escape is reported honestly and cleanup now returns within the termination
bound even while the escaped writer is always ready. No performance,
full-migration or Windows-readiness claim is made.
