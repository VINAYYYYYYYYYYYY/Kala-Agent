# r2-datadir-fix — the geometry worker uses the front door's data dir

Status: implemented and verified on Linux (Rust + Python). No commit, push,
deploy, delegation or credential read was performed. No owned code or test
reads, writes or deletes anything under `/home/v9i/.local/share/kala-agent`.
One pre-existing, non-owned Python test was executed with the ambient
environment before the audit identified it — see "Audit findings" and
"Caveat" below.

## Root cause

`kala-studio/src/cli.rs::start_geometry_worker` launched
`python -m kala_agent.studio --worker` with a cwd workspace and the token,
lifeline and STEP-limit environment, but **no data directory**. In worker
mode `kala_agent.studio.cli._resolve_data_dir` then used
`Settings.from_env()` (the home directory, or the shared temp folder on
failure), so:

* the worker's journal/artifacts did not live in the front door's
  `--data-dir`;
* every worker with the same implicit fallback shared one journal, which is
  the 3-events-instead-of-1 flake in
  `the_chat_turn_is_journalled_through_the_worker`;
* a Python reference server and the Rust front door on the same explicit
  data dir did not share one journal.

The Python worker test helper had the matching bug from the other side:
`tests/unit/studio_worker/test_worker_process.py` set
`KALA_STUDIO_DATA_DIR`, but the worker CLI ignored it.

## Changes

### Rust front door (`backend/crates/kala-studio`, owned)

* `src/worker.rs`
  * `GeometryWorker` now carries the front door's resolved `data_dir`.
  * `GeometryWorker::new(command, data_dir, workspace, import_roots,
    step_import_max_bytes)` — `data_dir` is a new explicit parameter.
  * `GeometryWorker::launch_arguments()` appends `--data-dir <data_dir>` as
    the **last** argv pair, so a stale `--data-dir` in
    `KALA_STUDIO_GEOMETRY_WORKER` cannot win (argparse last-wins).
  * `GeometryWorker::data_dir()` accessor; `WORKER_DATA_DIR_FLAG` constant.
  * The path is carried as an `OsString`, so a non-UTF-8 directory is not
    altered by lossy conversion.
  * Module docs describe the new launch contract.
* `src/cli.rs`
  * `start_geometry_worker` passes the already-resolved front-door
    `data_dir` into `GeometryWorker::new`; the `<data-dir>/geometry-worker`
    workspace is unchanged.
* `tests/chat_with_design.rs`
  * `start_studio` now creates an empty `<temp>/home` and points the front
    door's `HOME` at it.
  * `the_chat_turn_is_journalled_through_the_worker` asserts exactly one
    `studio-journal.sqlite3` below the test temp root and that it is
    `<temp>/data/studio-journal.sqlite3` (the front door's `--data-dir`),
    and that the empty `HOME` stays empty.
* `tests/worker_lifecycle.rs`
  * `a_killed_worker_is_restarted_and_never_sees_the_token_in_argv` now also
    asserts that the worker's `/proc/<pid>/cmdline` carries `--data-dir` and
    the front door's data path (the token assertions are unchanged).
* `tests/worker_differential.py`
  * `start_worker` now launches the Python worker with
    `--data-dir <its data_dir>`; without it the fail-closed worker refuses
    to start, and the differential cannot silently reach the home store.

### Python worker (`src/kala_agent/studio`, owned: worker-mode data-dir only)

* `cli.py`
  * New `STUDIO_DATA_DIR_ENV_VAR = "KALA_STUDIO_DATA_DIR"` and
    `WORKER_DATA_DIR_REQUIRED_MESSAGE`.
  * New `_resolve_worker_data_dir(arguments)`: `--data-dir` wins, then
    `KALA_STUDIO_DATA_DIR`; a missing/blank value raises the clear message.
  * `main()` resolves the worker data dir **before** uvicorn and the worker
    start, prints the message to stderr, and exits `2` (usage) on refusal.
  * `--data-dir` help names the worker requirement.
  * The non-worker path (`_resolve_data_dir`) is byte-for-byte unchanged:
    the normal Studio keeps its current home/temp default.
* `worker.py`
  * Module docs now state the mandatory explicit data dir and the absence of
    a home/temp fallback. No behaviour change in this file.
* `tests/unit/studio_worker/test_worker_process.py`
  * `test_worker_refuses_startup_without_an_explicit_data_dir`: missing,
    empty and whitespace-only `KALA_STUDIO_DATA_DIR` each exit `2`, print a
    message naming both `--data-dir` and `KALA_STUDIO_DATA_DIR`, print no
    readiness line, and leave a fresh empty `HOME` empty.
  * `test_the_journal_lives_in_the_explicit_data_dir`: a real worker started
    through `KALA_STUDIO_DATA_DIR` journals a design turn at
    `<data-dir>/studio-journal.sqlite3`, and a fresh empty `HOME` stays
    empty.
  * The spawn helpers accept an optional `home` so the isolation is explicit
    in the test process environment.

## Public API summary

Rust (`kala_studio::worker`):

```rust
pub const WORKER_DATA_DIR_FLAG: &str = "--data-dir";
impl GeometryWorker {
    pub fn new(command: WorkerCommand, data_dir: PathBuf, workspace: PathBuf,
               import_roots: Vec<PathBuf>, step_import_max_bytes: usize) -> Arc<Self>;
    pub fn data_dir(&self) -> &Path;
    pub fn launch_arguments(&self) -> Vec<std::ffi::OsString>;
    // existing API unchanged
}
```

Python (`kala_agent.studio.cli`):

```python
STUDIO_DATA_DIR_ENV_VAR = "KALA_STUDIO_DATA_DIR"
WORKER_DATA_DIR_REQUIRED_MESSAGE = "--worker requires an explicit --data-dir or ..."
```

No `/api/studio` route, JSON shape, status code, header, canonical JSON or
digest changed. The only new failure surface is the deliberate worker-mode
usage refusal.

## Verification (exact results)

All Rust commands were run in `backend/crates/kala-studio`; all Python
commands from the repository root with `.venv/bin/python`.

| Gate | Result |
|---|---|
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --locked` (debug, default) | **149 passed, 0 failed** |
| `cargo test --locked --all-features` (debug) | **165 passed, 0 failed** |
| `cargo test --locked --release` | **149 passed, 0 failed** |
| `cargo build --release --locked` | success |
| `pytest tests/unit/studio_worker` | **42 passed** (28 HTTP, 6 import, 8 process) |
| `pytest tests/unit/test_studio_*.py tests/security/test_studio_*.py tests/unit/studio_design tests/unit/studio_worker` | **453 passed** |
| `ruff check` / `ruff format --check` (changed Python files and worker tests) | clean |
| `mypy --strict src/kala_agent/studio` | success, 41 files |

### Isolation with an empty HOME

Each run used a fresh empty directory as `HOME` and asserted it was still
empty afterwards (`find "$EMPTY" -mindepth 1 | wc -l` = `0`).

| Run | Result | HOME after |
|---|---|---|
| Full Rust suite (`cargo test --locked`, `CARGO_HOME`/`RUSTUP_HOME` pointed at the existing toolchain caches, `HOME` empty) | 149 passed | **0 entries** |
| Python worker suite (`pytest tests/unit/studio_worker`) | 42 passed | **0 entries** |
| Python design/security/worker selection (`pytest tests/unit/studio_design tests/unit/studio_worker tests/security/test_studio_security.py`) | 293 passed | **0 entries** |
| Manual `env -i HOME=<empty> ... python -m kala_agent.studio --worker` without a data dir | exit 2, clear message, no readiness line | **0 entries** |

The flaky test was run 20 consecutive times under the test binary's default
parallelism (5 tests at once):

```
for i in $(seq 1 20); do cargo test --locked --test chat_with_design; done
→ 20/20 runs: test result: ok. 5 passed; 0 failed
```

Every run also asserted the journal location and the empty HOME described
above.

### Live differentials

| Oracle | Result |
|---|---|
| `python_differential.py` | **268 cases, 0 mismatches**, exit 0 |
| `worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps**, exit 0 |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures**, exit 0 |
| `replay_binary_corpus.py` | **48 cases: 44 exact, 2 STEP-structural, 2 library-limit skips, 0 mismatches**, exit 0 |

Run as `cd backend && ../.venv/bin/python crates/kala-studio/tests/<oracle>.py`.

## Security and compatibility evidence

* **Fail closed, before any socket**: the missing-data-dir refusal is a CLI
  usage error (exit 2), prints to stderr, prints no readiness line and
  creates nothing in `HOME`; `run_worker` is never entered, so no listener
  is bound.
* **One journal per data dir**: the worker opens
  `<front-door data-dir>/studio-journal.sqlite3`. The flaky test asserts
  that exactly one journal file exists anywhere below its temp root, so a
  reintroduced fallback (for example `<data>/geometry-worker/.local/...`)
  fails the test.
* **Token hygiene unchanged**: the worker bearer token remains
  `KALA_WORKER_TOKEN`-only; the lifecycle test still proves it never appears
  in the worker's argv. `--data-dir` is a non-secret path.
* **No argument smuggling**: the data dir is a separate argv element, not a
  shell string; `kala-process` validated the executable/cwd environment as
  before.
* **Override order**: the front door's `--data-dir` pair is appended last
  and the Python CLI resolves the flag before the env var, so neither a
  stale configured argv nor a stale environment value can redirect the
  journal.
* **Parity**: the Python reference server started with the same
  `--data-dir` now shares the journal with the Rust front door; the live
  differentials (including settings-file byte comparisons) stay at 0
  mismatches.
* **No change to the non-worker Studio**: `_resolve_data_dir` and the Rust
  `default_studio_data_dir` are untouched, so the documented home/temp
  default for the normal server remains.

## Audit findings (default data dir uses)

Owned and fixed:

* `backend/crates/kala-studio/tests/worker_differential.py::start_worker`
  launched the worker without a data dir (now passes `--data-dir`).
* `tests/unit/studio_worker/test_worker_process.py` set
  `KALA_STUDIO_DATA_DIR` which the worker CLI ignored (now honoured and
  covered by tests).

Not owned, reported only:

1. `tests/unit/test_studio_server.py::test_token_is_never_echoed_in_any_response`
   builds `create_studio_app(data_dir=None)`, which resolves
   `KALA_STUDIO_DATA_DIR` or `Settings.from_env()` (the real default store).
   This is outside the cell's owned paths. The full-studio Python selection
   should be run with `HOME` pointed at a scratch directory until it is
   fixed.
2. `packaging/native_studio_smoke.py` and `packaging/studio_smoke.py` set
   `KALA_STUDIO_DATA_DIR` but start the backend without `--data-dir`. The
   Python CLI and the Rust front door both ignore that variable when
   `--data-dir` is absent, so those smokes use the home/temp default rather
   than the smoke's temporary `data_dir`.
3. `studio/test/nativeBackendPackaging.test.ts` sets
   `KALA_STUDIO_DATA_DIR` on the native front door; the Rust CLI does not
   read it (same reason as 2). That test is in `studio/**`, which this cell
   must not edit.
4. `backend/compat/corpuslib/http_cases.py` and
   `tests/unit/studio_design/test_design_api.py` set
   `KALA_STUDIO_DATA_DIR` for the **public Python app**
   (`create_studio_app`), which honours it when `data_dir` is `None`; these
   are not worker launches and are safe.
5. Other Rust crates' `HOME` tests (`kala-process`, `kala-tools`,
   `kala-cad-providers`) are supervisor environment fixtures/assertions,
   not data-dir fallbacks; `kala-process` already proves the child `HOME`
   is the workspace. No fix is needed there.

## Remaining gaps / intentional differences

* **Windows compile check**: `x86_64-pc-windows-gnu` is installed, but
  `cargo check --locked --target x86_64-pc-windows-gnu` stops in `ring`'s
  build script because `x86_64-w64-mingw32-gcc` is not installed in this
  environment (`failed to find tool "x86_64-w64-mingw32-gcc"`). This is a
  cross-toolchain gap, not a code error; no Windows runtime claim is made.
* **Normal Python Studio CLI still ignores `KALA_STUDIO_DATA_DIR`** when
  `--data-dir` is absent (pre-existing; the task asked to keep non-worker
  defaults unchanged). Only worker mode accepts the variable as the
  explicit alternative to the flag.
* **Packaging smokes** keep their pre-existing `KALA_STUDIO_DATA_DIR`
  mismatch (audit items 2–3); they are outside this cell's ownership.
* **Caveat about the audit run**: before the pre-existing default-data-dir
  test in item 1 was identified, the broader non-owned Python selection
  (`tests/unit/test_studio_*.py` …) was executed once with the ambient
  `HOME`. That test builds an app with `data_dir=None`, so it may have
  opened the default Studio store during that one run. No owned code,
  Rust test, worker test or differential does this, and no later run used
  the ambient `HOME`; subsequent isolation runs all used fresh empty home
  directories.
* No runtime-cutover, Windows-runtime, speed or readiness claims are made.
  The work proves the Linux worker/journal isolation only.
