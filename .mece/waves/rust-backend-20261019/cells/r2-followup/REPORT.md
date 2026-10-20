# r2-followup — one STEP import limit, and the worker's CAD provider route

Cell: `r2-followup`. Specs: ADR-0010 "Rust Control Plane; Geometry Stays in
the CAD Kernel", ADR-0011 "External CAD Builds, OpenCascade Views (Builds
Only on Request)"; the internal worker route contract from
`r1-worker-mode/REPORT.md` and the CAD provider model from
`../design-turns-20261019/cells/b2-providers/REPORT.md`.

Status: **done** for the two items the cell task names. The Rust front door
and the Python worker now share one STEP import ceiling, and the worker
answers `GET /internal/v1/providers/cad`, so the live differential has **0
known gaps** (it had 1).

No commit, no push, no delegation, no credential reads, no runtime cutover,
no Windows-runtime claim, no speed claim.

---

## 1. What this cell delivered

| Concern | Before | After |
|---|---|---|
| The STEP import ceiling | the Rust front door hard-coded 16 MiB, the worker used 64 MiB via `KALA_STEP_IMPORT_MAX_BYTES` | **one** setting, `KALA_STEP_IMPORT_MAX_BYTES`, default 64 MiB, resolved once by the front door with the worker's own parsing, enforced by the front door and handed to the worker process |
| `GET /api/studio/providers/cad` | the front door proxied the worker's 404; the public Python server probed the host | the worker mounts the public server's own handler; the front door's answer equals the public server's, byte for byte |

The live differential now reports:

```
differential cases: 289
mismatches: 0
known gaps: 0
```

---

## 2. Changed paths

Only the owned paths were touched.

**Python (the worker route and its tests)**

| Path | Change |
|---|---|
| `src/kala_agent/studio/internal_api.py` | the `GET /internal/v1/providers/cad` route: the public server's `cad_providers` handler mounted unchanged on the internal router, with the `CadProvidersResponse` model; the module docstring now says what the route probes and what the worker still does not expose |
| `tests/unit/studio_worker/test_worker_http.py` | the exact route set now includes `/internal/v1/providers/cad`; the "no public routes" test is renamed and now also proves that `/api/studio/providers/cad` is **not** mounted on the worker; five new tests (handler reuse, the live probe document, the bearer gate, the Host/Origin gates, and equality with the public server) |

**Rust (`backend/crates/kala-studio/**`)**

| Path | Change |
|---|---|
| `src/step_limit.rs` | **new file** — the exact port of the Python worker's `resolve_step_import_max_bytes`: the variable, the default, the bounds, the two refusal messages and the CPython `int()` grammar |
| `src/lib.rs` | the new module |
| `src/server.rs` | `StudioConfig.import_max_bytes`; the resolved ceiling in `State`; the import route's body limit is the resolved ceiling; `StudioService::import_max_bytes()`; `GET /api/studio/providers/cad` now carries the source's route-level bearer gate; the `MAX_IMPORT_BODY_BYTES` constant is gone |
| `src/worker.rs` | the supervisor carries the resolved ceiling and hands it to the child as `KALA_STEP_IMPORT_MAX_BYTES`; `GeometryWorker::new` takes it as a fourth argument; `step_import_max_bytes()` |
| `src/cli.rs` | the ceiling is resolved **before** anything is started; an unusable value refuses startup (exit 2); the resolved value goes to both the worker and the service |
| `tests/step_import_limit.rs` | **new file** — the default, the override, the unusable value, a body at the limit and one byte over it (declared and streamed), and the shared limit on the other routes |
| `tests/worker_lifecycle.rs` | **new test** — the real binary with the real worker: the worker's `/proc/<pid>/environ` carries the resolved value, and the front door's import route accepts the limit and refuses limit+1 |
| `tests/cli_launch.rs` | **new test** — an unusable `KALA_STEP_IMPORT_MAX_BYTES` exits 2 with the worker's message |
| `tests/security_boundaries.rs` | **new test** — the read-only routes (journal, journal/verify, providers/cad) carry their own bearer gate |
| `tests/common/mod.rs` | the stub backend answers the import route with the byte count, so a test can prove a body reached the worker; the `StudioConfig` literal carries the new field |
| `tests/settings_token.rs` | the `StudioConfig` literals carry the new field |
| `tests/worker_differential.py` | the `providers/cad` case is no longer a known gap; the public server is built with the same environment view the supervised worker runs under, so both sides probe the same host the same way |
| `README.md` | the new ceiling section, the route's bearer gate, the removed known gap, the new verification commands |

Nothing else was edited. No other Rust crate, no shared fixture, no adapter,
no `studio/**` file, no `backend/Cargo.toml` and no `backend/Cargo.lock`.

---

## 3. Public API

### `step_limit` (new)

```rust
pub const STEP_IMPORT_MAX_BYTES_ENV: &str = "KALA_STEP_IMPORT_MAX_BYTES";
pub const DEFAULT_MAX_STEP_BYTES: usize = 64 * 1024 * 1024;
pub const MIN_STEP_IMPORT_BYTES: usize = 1;
pub const MAX_STEP_IMPORT_BYTES: usize = 512 * 1024 * 1024;

pub const STEP_IMPORT_LIMIT_TYPE_MESSAGE: &str =
    "the STEP import limit must be an integer number of bytes";
pub const STEP_IMPORT_LIMIT_RANGE_MESSAGE: &str =
    "the STEP import limit must be between 1 and 536870912 bytes";
pub const STEP_IMPORT_ENV_TYPE_MESSAGE: &str =
    "KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes";

pub fn validate_step_import_max_bytes(value: i64) -> Result<usize, String>;
pub fn resolve_step_import_max_bytes(environ: &BTreeMap<String, String>) -> Result<usize, String>;
pub fn parse_python_integer(text: &str) -> Option<i64>;
```

### `server` (extended)

```rust
pub struct StudioConfig {
    ..,
    /// `None` resolves `KALA_STEP_IMPORT_MAX_BYTES` from the environment
    /// view; `Some` is the embedding seam.
    pub import_max_bytes: Option<usize>,
}

impl StudioService {
    /// The STEP import ceiling this service enforces.
    pub fn import_max_bytes(&self) -> usize;
}
```

`server::MAX_IMPORT_BODY_BYTES` (16 MiB) is **removed**: the ceiling is
now the resolved setting, not a constant. `StudioConfig` gained a field, so
every `StudioConfig { .. }` literal needs `import_max_bytes`.

### `worker` (extended)

```rust
impl GeometryWorker {
    pub fn new(
        command: WorkerCommand,
        workspace: PathBuf,
        import_roots: Vec<PathBuf>,
        step_import_max_bytes: usize,
    ) -> Arc<Self>;
    pub fn step_import_max_bytes(&self) -> usize;
}
```

---

## 4. The one STEP import limit

### The rule

| Input | Result | Python | Rust |
|---|---|---|---|
| absent or blank | the 64 MiB default | `67108864` | `67108864` |
| an integer in `1 ..= 536870912` | that value | same | same |
| not an integer | `KALA_STEP_IMPORT_MAX_BYTES must be an integer number of bytes` | same | same |
| out of range (including a value above `i64`) | `the STEP import limit must be between 1 and 536870912 bytes` | same | same |

The CPython `int()` grammar is ported, including single underscores between
digits (`1_048_576` is 1 MiB) and a leading sign. The table in
`step_limit::tests::the_resolution_matches_the_python_worker_case_by_case`
records the **Python worker's own answers** for 29 inputs; the Rust answers
are asserted against them.

### Where it is resolved

The front door resolves the value **once**, in `cli::main`, *before* the
worker is launched:

1. `cli::main` resolves it from the process environment. An unusable value
   prints the worker's message and exits 2 (a usage error), so no worker is
   ever started with a value the front door could not resolve.
2. The resolved value is handed to `GeometryWorker::new`, which puts it in
   the child environment as `KALA_STEP_IMPORT_MAX_BYTES` through
   `ChildEnvironment::with_extra`. The variable is not one of
   `kala-process`'s protected keys, so the accepted child-environment
   contract is used unchanged.
3. The same resolved value is handed to `StudioService` through
   `StudioConfig.import_max_bytes`, and the public import route enforces it.

The worker reads the variable with its own `resolve_step_import_max_bytes`,
so the two sides apply the same rule to the same number.

An embedder that builds a `StudioService` without the field gets the Python
behaviour: the value is resolved from the configured environment view
(`StudioConfig.environ`, or the process environment when it is `None`).

### The enforced bound

The public import route's body limit is the resolved ceiling, exactly like
the worker's route-aware body middleware: **not** the maximum of the ceiling
and the shared Studio limit. A ceiling below the shared limit therefore
applies to the import route only, and every other route keeps the 256 KiB
Studio limit. The two sides enforce the same number, so the front door's
413 and the worker's 413 agree; the front door never raises the worker's
ceiling, and the worker's answer is forwarded unchanged whenever the front
door lets a request through.

---

## 5. The worker's CAD provider route

`GET /internal/v1/providers/cad` is the **public server's own handler**
(`kala_agent.studio.server.cad_providers`), mounted on the internal router
with the public `CadProvidersResponse` model. There is no second
implementation and no duplicated probe logic: the route is the ADR-0011
probe, and the worker and the public server therefore report the same live
facts.

The route is behind the worker's existing gate stack — security headers →
Host (421) → body size (413) → Origin (403) → bearer (401) — and the
handler's own `_require_bearer` check runs as well, so the route is gated
twice. The worker probes only the **local** CAD providers: an OpenCascade
import check, a bounded `FreeCADCmd --version` subprocess and the ADR-0008
loopback bridge health probe. It never calls a chat provider and never
mounts an `/api/*` route.

The front door's `GET /api/studio/providers/cad` now carries the source's
route-level bearer gate (401 + `Bearer`), like the journal and design
routes. The public server has always required the launch bearer on that
route ("so a same-origin page cannot enumerate a Windows bridge it does not
own"), so this closes a real parity and security difference that r2 left
open.

### The live proof

On this host the front door now answers:

```json
{"providers":[
  {"id":"opencascade","name":"OpenCascade (opt-in)","available":true,"version":"8.0.1.1","reason":null,"measured_by":"OpenCascade (opt-in)","mode":"opt-in"},
  {"id":"freecad","name":"FreeCAD","available":true,"version":"1.1.4","reason":null,"measured_by":"FreeCAD","mode":"external"},
  {"id":"solidedge","name":"Solid Edge","available":false,"version":null,"reason":"Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here","measured_by":"Solid Edge","mode":"bridge"}]}
```

and the chat turn's provider-choice message now names the same available
providers the Python reference names:

```
front door : … Choose a CAD provider before I build this design: opencascade (opt-in), freecad (external), solidedge (bridge). Available on this host: opencascade, freecad. The studio never selects a provider automatically. No design was built.
reference  : … Choose a CAD provider before I build this design: opencascade (opt-in), freecad (external), solidedge (bridge). Available on this host: opencascade, freecad. No design was built.
```

The **availability list** now agrees (it did not before: the front door
could only prove its own OpenCascade builder). The one remaining wording
difference — the front door's extra sentence "The studio never selects a
provider automatically." — is pre-existing and unchanged by this cell.

---

## 6. Test results

### Rust (all commands from `backend/`)

| Gate | Result |
|---|---|
| `cargo fmt -p kala-studio -- --check` | **pass** |
| `cargo clippy -p kala-studio --all-targets --locked -- -D warnings` (default build) | **pass**, 0 warnings |
| `cargo clippy -p kala-studio --all-targets --all-features --locked -- -D warnings` | **pass**, 0 warnings |
| `cargo test -p kala-studio --locked` (default build, debug) | **144 passed**, 0 failed |
| `cargo test -p kala-studio --locked --all-features` (debug) | **160 passed**, 0 failed |
| `cargo test -p kala-studio --locked --release` | **144 passed**, 0 failed |
| `cargo build --release -p kala-studio --locked` | **pass** |
| `cargo build --release --workspace --locked` | **pass** |
| `cargo test --workspace --locked --no-run` | **compiles** |
| `cargo check -p kala-studio --target x86_64-pc-windows-gnu` | **blocked**: the `ring` build script needs `x86_64-w64-mingw32-gcc`, which is not installed on this host. The blocker is the missing C toolchain, not this crate, and it is the same blocker the r2 cell recorded. No Windows compilation or runtime claim is made. |

The counts grew from r2's 125/141 to 144/160: 19 new tests (8 `step_limit`
unit tests, 1 `worker` unit test, 1 `server` unit test, 6
`step_import_limit`, 1 `cli_launch`, 1 `security_boundaries`, 1
`worker_lifecycle`).

### The live differential (the parity gate)

| Harness | Result |
|---|---|
| `tests/worker_differential.py` | **289 cases, 0 mismatches, 0 known gaps** (r2: 289 / 0 / 1) |
| `tests/python_differential.py` (the nine-route corpus) | **268 cases, 0 mismatches** |
| `tests/replay_binary_corpus.py` (the frozen 48-case corpus) | **44 exact + 2 structural, 0 mismatches** (2 skipped for a library body-limit configuration) |
| `tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |

The comparison is status + exact body bytes + the stable and security
headers, with the two documented normalisations of r2 (the Rust-only
`geometry_worker` health field and the volatile journal fields).

**One harness change, and why it is honest.** The public Python server in
the differential was built with an *empty* environment view. The FreeCAD
probe reads `PATH` and needs a writable `HOME` (`FreeCADCmd` creates its
user directory on the first run), so with an empty environment the public
server reported FreeCAD as unavailable while the supervised worker — which
runs under the `kala-process` child environment — reported it as available.
The public application is now built with the same environment view the
supervised worker runs under, so both sides probe the same host the same
way. The probe result depends only on `PATH` and a writable `HOME`; the
credential-shaped keys are not in that view, so nothing else changes.

### Python (all commands from the repository root)

| Gate | Result |
|---|---|
| `pytest tests/unit/studio_worker -q` | **40 passed**, 1 warning (r1: 35; this cell adds 5) |
| `pytest tests/unit -q` | **1517 passed**, 1 warning (`StarletteDeprecationWarning`, pre-existing). The count includes this cell's 5 new worker tests and one test a concurrent cell added to `tests/unit/studio_design/` while these gates ran. |
| `pytest tests/security -q` | **189 passed**, 1 warning |
| `ruff check src/kala_agent/studio/internal_api.py tests/unit/studio_worker/test_worker_http.py` | **pass** |
| `ruff format --check` (the same two files) | **pass** (2 files already formatted) |
| `mypy --python-version 3.14 <the same two files>` | **Success: no issues found in 2 source files** |

`mypy` with the repo config (`python_version = "3.11"`) fails before
checking any project file, with the pre-existing environment failure that
A1/A2/A3/B1/B2/r1 all recorded:

```
.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error: Type
statement is only supported in Python 3.12 and greater  [syntax]
```

The venv is CPython 3.14 and its numpy stubs use PEP 695 syntax, which
mypy refuses under the configured 3.11 target. With the matching target the
same files are clean. No project setting was loosened to hide it.

The Rust crate's Python oracles (`backend/crates/kala-studio/tests/*.py`)
are not inside the Python project's lint scope and were not ruff-clean
before this cell (six pre-existing findings: `ANN401`, `UP032`, `I001`,
`RUF059`). This cell added no new finding to them.

### What the new tests prove

* `step_limit`: the default, the override, an unusable value with the
  worker's own message, the bounds, the CPython integer grammar, and a
  29-case table of the **Python worker's recorded answers**.
* `step_import_limit`: the default is 64 MiB; an override is used; an
  unusable value refuses startup with the worker's message; a body of
  exactly the limit reaches the worker while limit+1 is refused with 413
  `payload_too_large` and the limit in the detail; the streamed body is
  bounded by the same limit; every other route keeps the shared limit.
* `cli_launch`: an unusable `KALA_STEP_IMPORT_MAX_BYTES` exits 2 with the
  worker's message.
* `worker_lifecycle`: the real worker process carries
  `KALA_STEP_IMPORT_MAX_BYTES=4096` in `/proc/<pid>/environ`, the front
  door accepts a 4096-byte body (the worker answers 422 `invalid_step`, so
  the body passed both gates) and refuses 4097 bytes with 413.
* `security_boundaries`: the read-only routes, including the CAD provider
  route, answer 401 + `Bearer` without the launch token.
* `test_worker_http.py`: the route set, the handler reuse, the live probe
  document, the bearer gate, the Host/Origin gates, and **equality with
  the public server's document** when both applications are built with the
  same environment view.

---

## 7. Security evidence

* **The worker is still not browser-facing.** The new route is behind the
  worker's unchanged gate stack: Host (421) → body size (413) → Origin
  (403) → bearer (401), and the handler's own `_require_bearer` check on
  top. The route probes local providers only; it opens no hosted-model
  connection.
* **The front door's CAD provider route now requires the launch token**,
  like the public server's. A same-origin page that never proved it holds
  the token can no longer enumerate the host's CAD providers through the
  front door.
* **The ceiling is a security boundary on both sides.** The front door
  refuses an unusable value at startup instead of silently changing it, and
  it never raises the worker's ceiling: both sides enforce the same number,
  and the worker's answer is forwarded unchanged whenever a request reaches
  it.
* **The ceiling cannot be set through the request.** It is read once, from
  the process environment, before the socket is bound; no request can
  change it.
* **The worker's token is unaffected.** The ceiling travels in the child's
  environment next to the token; it is a number, not a secret, and it is
  never logged. The lifecycle test still asserts that the token is absent
  from `/proc/<pid>/cmdline` and from the worker's stdout and stderr.
* **The bounded body is still bounded.** The import route keeps its own
  large ceiling, every other route keeps the 256 KiB Studio limit, the
  declared length is checked before the body is read, and the streamed body
  stops before it can cross the limit.

---

## 8. Intentional differences

1. **The route-level bearer gate on `GET /api/studio/providers/cad`.** The
   front door did not have it (r2); the public server has always had it.
   The front door now matches the source.
2. **The import route's limit is the resolved ceiling, not
   `max(ceiling, shared)`.** The worker's route-aware middleware applies
   the ceiling *instead of* the generic limit, so the front door does the
   same. With the default ceiling the two readings are identical; they
   differ only when an operator sets a ceiling below 256 KiB.3. **Non-ASCII decimal digits.** CPython's `int()` accepts them
   (`"١٢٣"` is 123); this port refuses them with the same "must be an
   integer number of bytes" message. The worker would accept such a value
   and the front door would refuse to start: a documented, deliberate
   difference in an operator error path.
4. **The reply wording of the provider-choice message** still differs by
   the front door's extra sentence "The studio never selects a provider
   automatically." (pre-existing; the availability list now agrees).
5. **The front door still cannot build with `freecad` or `solidedge`.**
   The new route makes the front door report the *host's* availability
   honestly, and a provider the studio build cannot serve is still refused
   ("The freecad CAD provider is not available in this studio build. No
   design was built."). The front door never fabricates a build.
6. **The worker is now the CAD provider probe for the front door.** Before
   this cell the worker made no provider call at all (r1's contract). The
   route probes local providers only; the r1 property "the worker makes no
   provider *network* call" is narrowed to "no chat provider call": the
   ADR-0008 bridge health probe is a loopback request on Windows only, and
   on Linux the provider refuses without opening a socket.

---

## 9. Remaining gaps

1. **No Windows evidence.** The cross-check is blocked by the missing
   mingw-w64/MSVC C toolchain that `ring` needs, and no Windows host was
   available. The worker lifecycle, the `/proc` assertions and the restart
   behaviour are Linux-only proofs. No Windows runtime, COM or packaging
   claim is made.
2. **The front door still cannot drive FreeCAD.** The route reports that
   FreeCAD is available on the host; building with it needs the ADR-0007
   adapter, which is a separate cell. The front door refuses that provider
   honestly instead of silently building with OpenCascade.
3. **`backend/README.md` and `docs/rust-studio.md` still describe the old
   nine-route native surface** and do not mention the ceiling. Both are
   outside this cell's ownership, so they were not edited. The crate's own
   `README.md` describes both.
4. **The worker-only routes the public server does not mount** are still
   only compared against a directly launched worker (`/import`,
   `journal/append`, `intent/*`).
5. **The stale crate-local build directory**
   (`backend/crates/kala-studio/target/`) is still there; both
   differential harnesses point at the root workspace's
   `backend/target/release/kala-studio`. It is a gitignored build artifact
   and was not touched.

---

## 10. Honesty notes

* No runtime cutover was performed. The Python Studio remains the reference
  oracle and the front door is still not the production server.
* No speed, benchmark or readiness claim is made. The ceiling resolution
  happens once at startup; the route gate is unchanged in cost.
* No Windows compilation, COM, packaging or installer claim is made.
* The geometry is still measured by the worker's OCP kernel, never by Rust.
  The Rust side never fabricates a mesh, a STEP file, a measurement, a
  design or a provider availability fact: the availability it reports is
  the worker's live probe, forwarded byte for byte.
* No commit, push, deploy or delegation was performed. No credential was
  read; the only credential-shaped strings in the tree are the synthetic
  corpus values that were already there.
* A concurrent cell was editing `src/kala_agent/studio/design/providers/`
  and `src/kala_agent/studio/design/service.py` while these gates ran. Two
  transient failures in one full-suite run came from that in-flight edit;
  the same suite is green on a clean re-run (**1517 passed**) and the
  failures were never in the files this cell owns.
