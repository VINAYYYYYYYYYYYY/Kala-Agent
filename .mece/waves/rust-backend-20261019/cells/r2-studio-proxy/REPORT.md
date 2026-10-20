# r2-studio-proxy — Rust kala-studio front door + supervised geometry worker

Cell: `r2-studio-proxy`. Specs: ADR-0010 "Rust Control Plane; Geometry Stays
in the CAD Kernel" and ADR-0011 "CAD Provider Model: External CAD Builds,
OpenCascade Views (Builds Only on Request)"; the internal worker route
contract from `r1-worker-mode/REPORT.md` (binding).

Status: **done for the Rust front door**. The `GeometryBackend` seam, the
supervised Python geometry worker, the proxied routing, the parked native
geometry and chat-with-design are implemented, gated and differentially
proven against the live Python reference.

No commit, no push, no delegation, no credential reads, no runtime cutover,
no Windows-runtime claim, no speed claim.

---

## 1. What this cell delivered

The Rust `kala-studio` is now the **front door** of the Studio API. It owns
the HTTP boundary, the settings and token stores, the static UI, the chat
providers and the CAD provider routing. It does **not** compute geometry: a
supervised Python/OCP geometry worker does, over an authenticated loopback
connection.

| Concern | Owner after this cell |
|---|---|
| HTTP boundary, Host/Origin/bearer/body limits | Rust |
| `/health`, `/integrations`, `/providers`, `/settings`, `/chat` | Rust |
| `/preview`, `/export`, `/handoff`, `/design*`, `/journal*`, `/providers/cad`, `/import` | Python geometry worker (proxied) |
| Geometry, measurement, drawing, section, STEP import, the strict intent parser, the journal | Python geometry worker |
| CAD provider selection and the "never default to opencascade" rule | Rust |
| The native primitive geometry (`kala-geometry`, `kala-step`) | **parked** behind the non-default `native-primitives` feature |

---

## 2. Changed paths

Only the owned paths were touched.

**New**

| Path | Purpose |
|---|---|
| `backend/crates/kala-studio/src/worker.rs` | The worker command grammar, the per-launch token, the readiness line, the supervision loop, the restart backoff, the stdin lifeline |
| `backend/crates/kala-studio/src/geometry_backend.rs` | The `GeometryBackend` trait (one async method per worker route) and `PythonWorkerBackend` |
| `backend/crates/kala-studio/src/chat_design.rs` | Chat-with-design: the strict reading, the remote schema/validate/repair flow, the CAD provider rule, the `chat.turn` journal event |
| `backend/crates/kala-studio/src/native_backend.rs` | The **parked** native primitive backend (feature `native-primitives`) |
| `backend/crates/kala-studio/tests/worker_lifecycle.rs` | Worker readiness, the token, the crash, the restart, the stdin lifeline |
| `backend/crates/kala-studio/tests/chat_with_design.rs` | Chat-with-design against the real worker |
| `backend/crates/kala-studio/tests/worker_differential.py` | The live differential: the corpus plus the ADR-0009/0011 design routes |

**Modified**

| Path | Change |
|---|---|
| `backend/crates/kala-studio/Cargo.toml` | `default = []`; `native-primitives` (and the `native-step` alias) are non-default; `kala-geometry`/`kala-step` became optional; `kala-process` added as a path dependency; the `tokio` `process` feature added |
| `backend/crates/kala-studio/src/lib.rs` | The new modules and the re-exports |
| `backend/crates/kala-studio/src/server.rs` | The route table, the proxied routes, the health document, the chat design hook, the per-route body limit, the route-level bearer gate |
| `backend/crates/kala-studio/src/schemas.rs` | The native DTOs and validators moved behind `native-primitives`; `cad_provider` added to the chat request |
| `backend/crates/kala-studio/src/geometry.rs` | The native mesh/STEP code moved behind `native-primitives`; the typed capability seam stays in every build |
| `backend/crates/kala-studio/src/json.rs` | `Json::Raw` — pre-rendered JSON embedded verbatim (the worker's own design document) |
| `backend/crates/kala-studio/src/cli.rs` | The worker is started and stopped with the service |
| `backend/crates/kala-studio/README.md` | The new architecture, the routes, the chat-with-design rules, the parking and the verification commands |
| `backend/crates/kala-studio/tests/common/mod.rs` | A deterministic `StubBackend` for the offline tests |
| `backend/crates/kala-studio/tests/{chat_and_remote,cli_launch,corpus_fixture,geometry_exports,json_encoding,settings_token,step_export_ocp}.rs` | Adapted to the backend seam; the native-only files are gated behind `native-primitives` |
| `backend/crates/kala-studio/tests/python_differential.py` | The binary path now points at the root workspace target; the harness waits for the geometry worker and ignores the ADR-0010 worker state in the health document |
| `backend/crates/kala-studio/tests/replay_binary_corpus.py` | The same, plus the ADR-0009/0010 document normalisation |
| `backend/crates/kala-studio/tests/python_provider_dto_oracle.py` | The expected chat bodies carry the ADR-0009 `design` field |
| `backend/Cargo.lock` | `kala-process` added as a dependency of `kala-studio` (11 lines) |

`backend/Cargo.toml` is **unchanged**: `kala-process` stays excluded from the
root workspace's member list, and a path dependency on an excluded crate
resolves normally. Nothing outside `backend/crates/kala-studio/**` and
`backend/Cargo.lock` was edited. No Python source, no Python test, no shared
fixture, no adapter and no `studio/**` file was touched.

---

## 3. Public API

### `worker` (new)

```rust
pub const WORKER_COMMAND_ENV_VAR: &str = "KALA_STUDIO_GEOMETRY_WORKER";
pub const WORKER_TOKEN_ENV_VAR: &str = "KALA_WORKER_TOKEN";
pub const WORKER_LIFELINE_ENV_VAR: &str = "KALA_WORKER_STDIN_LIFELINE";
pub const WORKER_READY_PREFIX: &str = "KALA_WORKER_READY";
pub const WORKER_HOST: &str = "127.0.0.1";
pub const WORKER_TOKEN_BYTES: usize = 32;
pub const WORKER_START_TIMEOUT: Duration;          // 30 s
pub const WORKER_RESTART_MIN_DELAY: Duration;       // 250 ms
pub const WORKER_RESTART_MAX_DELAY: Duration;       // 5 s
pub const WORKER_INTERNAL_PREFIX: &str = "/internal/v1";
pub const FIXED_WORKER_ARGUMENTS: &[&str];          // ["-m", "kala_agent.studio", "--worker"]

pub enum WorkerCommandError { Empty, NotAnArgvArray, InvalidArgvArray,
                              WhitespaceNotAllowed, NotAbsolute }
pub struct WorkerCommand { .. }
impl WorkerCommand {
    pub fn new(executable, arguments) -> Result<Self, WorkerCommandError>;
    pub fn executable(&self) -> &Path;
    pub fn arguments(&self) -> &[String];
    pub fn parse(value: &str) -> Result<Self, WorkerCommandError>;
    pub fn development_default() -> Option<Self>;
    pub fn from_environment(environ: &BTreeMap<String, String>) -> Option<Self>;
}
pub fn development_import_roots() -> Vec<PathBuf>;
pub fn mint_worker_token() -> Result<String, String>;
pub struct WorkerReadiness { pub port: u16, pub version: String, pub design_available: bool }
pub fn parse_readiness_line(line: &str) -> Result<WorkerReadiness, String>;
pub enum WorkerState { Starting, Ready, Restarting, Stopped, Failed }
pub struct WorkerStatus { pub state: WorkerState, pub version: Option<String>,
                          pub design_available: bool }
pub struct WorkerEndpoint { pub port: u16, pub token: Arc<str> }
pub struct GeometryWorker { .. }
impl GeometryWorker {
    pub fn new(command, workspace, import_roots) -> Arc<Self>;
    pub fn command(&self) -> &WorkerCommand;
    pub async fn status(&self) -> WorkerStatus;
    pub async fn endpoint(&self) -> Option<WorkerEndpoint>;
    pub async fn start(self: &Arc<Self>);
    pub async fn shutdown(&self);
}
```

### `geometry_backend` (new)

```rust
pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub const WORKER_REQUEST_TIMEOUT: Duration;      // 120 s
pub const MAX_WORKER_RESPONSE_BYTES: usize;      // 64 MiB
pub const FORWARDED_RESPONSE_HEADERS: &[&str];

pub struct WorkerResponse { pub status: u16, pub headers: Vec<(String, String)>,
                            pub body: Vec<u8> }
pub struct BackendHealth { pub state: String, pub version: Option<String>,
                           pub design_available: bool, pub step_available: bool }
pub enum BackendError { Unavailable, Timeout, Transport, TooLarge, Unsupported }

pub trait GeometryBackend: Send + Sync {
    fn health(&self) -> BackendFuture<'_, BackendHealth>;
    fn preview<'a>(&'a self, body: &'a [u8], content_type: Option<&'a str>)
        -> BackendFuture<'a, Result<WorkerResponse, BackendError>>;
    fn export<'a>(..); fn handoff<'a>(..); fn design<'a>(..);
    fn drawing<'a>(&'a self, design_id: &'a str, query: Option<&'a str>) -> ..;
    fn section<'a>(&'a self, design_id: &'a str, query: Option<&'a str>) -> ..;
    fn import_step<'a>(..);
    fn intent_parse<'a>(..); fn intent_validate<'a>(..); fn intent_schema<'a>(&'a self) -> ..;
    fn journal_page<'a>(&'a self, query: Option<&'a str>) -> ..;
    fn journal_verify<'a>(&'a self) -> ..;
    fn journal_append<'a>(..);
    fn providers_cad<'a>(&'a self) -> ..;
}

pub struct PythonWorkerBackend { .. }
impl PythonWorkerBackend {
    pub fn new(worker: Arc<GeometryWorker>) -> Result<Arc<Self>, String>;
    pub fn worker(&self) -> &Arc<GeometryWorker>;
    pub async fn design_schema(&self) -> Option<Arc<DesignSchema>>;
}
pub struct DesignSchema { .. }   // the DesignIntent property names
```

### `chat_design` (new)

```rust
pub const DESIGN_INTENT_INSTRUCTION: &str;
pub const CAD_PROVIDER_OPENCASCADE: &str = "opencascade";
pub const JOURNAL_AGGREGATE_ID: &str = "studio";
pub const CHAT_TURN_EVENT_TYPE: &str = "chat.turn";

pub struct DesignTurn { pub text: String, pub design: Option<Value>,
                        pub design_id: Option<String>, pub design_requested: bool,
                        pub missing: Vec<String>, pub model: Option<String> }
pub struct ParsedIntent { pub family: Option<String>, pub intent: Option<Value>,
                          pub missing: Vec<String>, pub notes: Vec<String> }
pub enum DesignTurnError { Backend(BackendError), Provider(ProviderFailure) }

pub async fn parse_intent(backend, prompt) -> Result<ParsedIntent, BackendError>;
pub fn design_reply_text(parsed: &ParsedIntent) -> String;
pub fn design_prompt(prompt: &str, schema_text: &str) -> String;
pub fn repair_prompt(prompt: &str, error: &str) -> String;
pub fn strip_design_json_block(text: &str) -> String;
pub fn measured_summary(design: &Value) -> String;
pub fn join_text(parts: &[&str]) -> String;
pub async fn available_cad_providers(backend) -> Vec<String>;
pub fn provider_choice_message(providers: &[String]) -> String;
pub async fn compile_design(backend, intent, cad_provider)
    -> Result<Result<Value, String>, BackendError>;
pub async fn local_design_turn(backend, parsed, cad_provider) -> DesignTurn;
pub async fn remote_design_turn<F, Fut>(backend, prompt, cad_provider, call) -> Result<DesignTurn, DesignTurnError>;
pub async fn append_chat_turn(backend, provider, model, offline, prompt, design_id,
                               design_requested, cad_provider, missing) -> Result<(), BackendError>;
```

### `server` (extended)

```rust
pub const MAX_IMPORT_BODY_BYTES: usize;   // 16 MiB, the STEP import's own limit
pub const ROUTE_BEARER_CHALLENGE: &str;   // "Bearer"
pub const GEOMETRY_UNAVAILABLE_CODE: &str;
pub const GEOMETRY_UNAVAILABLE_MESSAGE: &str;

pub struct StudioConfig { .., pub geometry_backend: Option<Arc<dyn GeometryBackend>> }
impl StudioService {
    pub fn geometry_backend(&self) -> Option<Arc<dyn GeometryBackend>>;
}
```

### `native_backend` (new, feature `native-primitives`)

```rust
pub struct NativePrimitivesBackend { .. }
impl NativePrimitivesBackend {
    pub fn new(exporter: Arc<dyn StepExporter>, workdir: PathBuf) -> Self;
}
```

---

## 4. The launch contract

* **Command.** `KALA_STUDIO_GEOMETRY_WORKER` is either a JSON argv array
  (`["/path/python","-m","kala_agent.studio","--worker"]`) or a single
  executable path, in which case the fixed module arguments are appended.
  Whitespace splitting is **not** performed: a value that names an
  executable *and* arguments is refused with
  `WorkerCommandError::WhitespaceNotAllowed`. The development default is
  `<repo>/.venv/bin/python -m kala_agent.studio --worker`, resolved from the
  repository that contains the crate.
* **Token.** 32 fresh random bytes rendered as 64 hexadecimal characters,
  passed **only** through `KALA_WORKER_TOKEN`. It is never in argv and never
  logged.
* **Lifeline.** `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops
  when the front door closes its stdin.
* **Readiness.** Exactly one line, `KALA_WORKER_READY {json}`, is read with a
  bounded 30-second start timeout. The port, the version and the
  `design_available` flag are taken from it; the port is validated.
* **Restart.** The worker is restarted with a capped exponential backoff
  (250 ms → 5 s). The state is `starting`, `ready`, `restarting`, `stopped`
  or `failed`.
* **Health.** `/health` publishes
  `geometry_worker {state, version, design_available}` next to the ADR-0009
  `design_available` flag. `step_available` reports the *effective*
  capability of the front door: the worker's OCP kernel provides both the
  design compiler and the STEP writer, so one flag answers both.

### The launch implementation and `kala-process`

The command, the working directory and the child environment are built with
the accepted `kala-process` crate: `ProcessSpec` (absolute executable,
canonical working directory) and `ChildEnvironment::legacy_python_worker`
(the transitional Python worker environment with `PYTHONPATH`,
`PYTHONNOUSERSITE`, `PYTHONSAFEPATH`, a fixed `PATH` and the workspace as
`HOME`/`TMPDIR`). The token and the lifeline are added through
`ChildEnvironment::with_extra`, which refuses the protected keys.

**Honest limitation:** `kala-process::supervise` supervises a process *to
completion*; it has no streaming or long-lived API, and its stdin contract
closes the pipe. A persistent worker that must keep its stdin open for the
whole lifetime and hand over a readiness line therefore cannot be launched
through it. This crate uses the reviewed `ProcessSpec`/`ChildEnvironment`
contract for the launch and implements the persistent spawn, the readiness
line, the restart loop and the shutdown itself. This is stated in the module
documentation of `src/worker.rs`; no `kala-process` API was patched.

---

## 5. Routing

| Path | Method | Answered by |
|---|---|---|
| `/api/studio/health` | GET | Rust |
| `/api/studio/integrations` | GET | Rust |
| `/api/studio/providers` | GET | Rust |
| `/api/studio/settings` | GET, POST | Rust |
| `/api/studio/chat` | POST | Rust (the design hook reaches the worker) |
| `/api/studio/preview` | POST | worker |
| `/api/studio/export` | POST | worker |
| `/api/studio/handoff` | POST | worker |
| `/api/studio/design` | POST | worker |
| `/api/studio/design/{id}/drawing` | GET | worker |
| `/api/studio/design/{id}/section` | GET | worker |
| `/api/studio/journal` | GET | worker |
| `/api/studio/journal/verify` | GET | worker |
| `/api/studio/providers/cad` | GET | worker |
| `/api/studio/import` | POST | worker |

**Body limits.** The shared Studio limit (256 KiB) applies to every route
except the STEP import, which has its own large dedicated limit
(`MAX_IMPORT_BODY_BYTES`, 16 MiB). The limit is chosen from the path
*before* the body is read, so the declared length and the streamed body are
both checked against the route's own bound. The worker keeps its own gate
and its refusal is forwarded unchanged: the front door never raises the
worker's ceiling.

**Response and time limits.** A worker answer is capped at 64 MiB and a
request at 120 s. A worker that is down answers 503
`geometry_worker_unavailable`; a timeout answers 504; an oversized answer
answers 502; a route the backend does not implement answers 501. The native
routes keep working while the worker is down.

**Bearer gate.** The mutating methods carry the middleware gate (401 +
`Bearer realm="kala-studio"`). The read-only design and journal routes carry
the source's own route-level gate (401 + `Bearer`). The method check runs
before the route-level gate, exactly like the source (a `POST` to a GET-only
route is 405, not 401).

**Payload forwarding.** The request body and the **original content type**
are forwarded unchanged, so the worker applies the source's own validation
(a `text/plain` body is the source's `model_attributes_type` 422, and a
malformed body is the source's `json_invalid` offset). The worker's answer
is forwarded byte for byte, with the content type, the download name, the
`x-studio-*` headers and the `www-authenticate` challenge preserved.

---

## 6. Chat with designs

* **Local provider.** The front door asks the worker to read the prompt
  (`intent/parse`). A prompt that is not a design request keeps the ordinary
  analytic reply. A design prompt gets the strict reading: the family, the
  missing values and the parser notes, rendered in the source's own wording
  (`Local helper. I can draft a L-bracket, but I need these declared values:
  …` / `Local helper. Design intent parsed from declared values. …`).
* **Remote providers.** The front door fetches the DesignIntent JSON Schema
  from `intent/schema`, adds it to the prompt, calls the provider, extracts
  the fenced JSON and validates it with `intent/validate`. **One** repair
  round-trip is allowed; a second failure is reported honestly
  (`No validated design was produced: …`).
* **The CAD provider is never chosen automatically** (ADR-0011). The chat
  request carries an optional `cad_provider`. Without it the turn asks the
  operator which provider to use and compiles nothing. A provider the studio
  cannot serve is refused; it is never silently replaced by another one.
  The provider is added to the worker's design request (the ADR-0011
  `provider` field); a worker whose design request does not know the field
  refuses it as an extra input, and the front door then retries once
  without it.
* **The journal turn.** Every turn appends a `chat.turn` event through the
  worker's `journal/append`. The payload never carries the prompt text,
  only its SHA-256 digest and its length, and it carries the provider, the
  model, the offline flag, the design id, the design-requested flag and the
  CAD provider — the same fields the source records.

A refused journal append is reported locally and the answer is still
returned: a worker that is down must not take the native chat route down
with it.

---

## 7. The parked native geometry

ADR-0010 parks `kala-geometry` and `kala-step`. The default build links
**no** native geometry crate: both became optional dependencies behind the
non-default `native-primitives` feature, and the native mesh/STEP code, the
native DTOs and the native validators are compiled out. Nothing was deleted:
the parked backend (`src/native_backend.rs`) serves the three analytic
primitive routes from the reviewed crates in the `native-primitives` build,
and the frozen 48-case corpus replay runs there.

`native-step` is kept as an alias of `native-primitives` so the former build
recipe still works. Neither feature is a default feature.

---

## 8. Test results

All commands are run from `backend/`.

| Gate | Result |
|---|---|
| `cargo fmt -p kala-studio -- --check` | **pass** |
| `cargo clippy -p kala-studio --all-targets --locked -- -D warnings` (default build) | **pass**, 0 warnings |
| `cargo clippy -p kala-studio --all-targets --all-features --locked -- -D warnings` | **pass**, 0 warnings |
| `cargo test -p kala-studio --locked` (default build, debug) | **125 passed**, 0 failed |
| `cargo test -p kala-studio --locked --all-features` (debug) | **141 passed**, 0 failed |
| `cargo test -p kala-studio --locked --release` (default build) | **125 passed**, 0 failed |
| `cargo build --release -p kala-studio --locked` | **pass** |
| `cargo build --release --workspace --locked` | **pass** |
| `cargo test --workspace --locked --no-run` | **compiles** |
| `cargo check -p kala-studio --target x86_64-pc-windows-gnu` | **blocked**: the `ring` build script needs `x86_64-w64-mingw32-gcc`, which is not installed on this host. `x86_64-pc-windows-msvc` is blocked the same way (`lib.exe` missing). The blocker is the missing C toolchain, not this crate, and it was reported by the accepted studio review as well. No Windows compilation or runtime claim is made. |

Per-target counts (default build): 86 lib unit tests, 7 `chat_and_remote`,
5 `chat_with_design`, 5 `cli_launch`, 4 `security_boundaries`,
12 `settings_token`, 4 `worker_lifecycle`, 1 `float_vectors`, 1 `tls_trust`.
The `native-primitives` build adds 6 lib unit tests, 1 `corpus_fixture`,
6 `geometry_exports`, 2 `json_encoding` and 1 `step_export_ocp`.

### The live differential (the parity gate)

`tests/worker_differential.py` launches the **real release binary with the
real supervised Python worker** and replays the corpus and the ADR-0009/0011
design routes against the live Python public server (and, for the routes the
public server does not mount, against a directly launched worker).

```
differential cases: 289
mismatches: 0
known gaps: 1
```

The 289 cases are the 268-case corpus plus 21 ADR-0009/0011 cases: the
design request (with and without a provider, an empty body, a bad template,
two sources at once), the journal page and the journal verification, the
read-only journal routes without a token, the drawing and section routes
with a bad and with an unknown design id, the CAD provider route, a method
mismatch, the chat turn with a chosen CAD provider (which compiles the
design and is compared byte for byte against the live Python answer) and the
two refused `cad_provider` values.

* `tests/python_differential.py` (the nine-route corpus, unchanged): **268
  cases, 0 mismatches**. The same harness against the pre-change binary
  reports **30 mismatches** — the 30 known mismatches are closed. They were
  the ADR-0009 `design` field on `ChatResponse` (18 cases) and the
  ADR-0009 `design_available` field on `HealthResponse` (12 cases).
* `tests/replay_binary_corpus.py` (the frozen 48-case binary corpus):
  **44 exact + 2 structural, 0 mismatches** (2 cases are skipped by the
  harness for a library body-limit configuration).
* `tests/python_provider_dto_oracle.py`: **20 cases, 0 failures**.

The comparison is status + exact body bytes + the stable and security
headers. Two documented normalisations are used and nothing else:

1. **The health document** loses the Rust-only `geometry_worker` field
   (the Python public server has no worker). This is the ADR-0010
   requirement itself.
2. **The journal documents** lose `occurred_at`, the payload digest, the
   chain digests and the journal head. Two processes never share a chain
   digest, so those fields are replaced by a placeholder on both sides. The
   event sequence, the event types, the aggregates and the payloads are
   still compared exactly — which is how the `chat.turn` payload (including
   the ADR-0011 `cad_provider` field) is proven equal.

The STEP downloads are compared structurally (a real ISO 10303-21 file with
one manifold solid and the stable headers), because OpenCascade's writer
embeds a timestamp and its own entity numbering.

### The worker lifecycle (`tests/worker_lifecycle.rs`, 4 tests)

* A worker that cannot start leaves the native routes working and the
  geometry routes answering 503 `geometry_worker_unavailable` with a
  message that names the worker.
* A worker that is killed is restarted and the geometry route answers again;
  the token is **absent from `/proc/<pid>/cmdline`** and from the worker's
  argv, and it is absent from the process's stdout and stderr.
* A dead front door makes the worker exit: the stdin lifeline closes, and
  the worker process is gone within 30 s.
* The health document publishes the worker state, the version and the
  `design_available` flag.

### Chat with designs (`tests/chat_with_design.rs`, 5 tests)

* A design prompt **without** a `cad_provider` asks the operator which
  provider to use and compiles nothing.
* A design prompt **with** `cad_provider: "opencascade"` compiles the
  design: the same design id
  (`9ec29435f7b95f19641f548c1d29e398f0cbb4115e9e5e38c5d8ef07c83f4bf8`),
  the same measured volume (28 439.999999999993 mm³), the same surface
  area (11 148.0 mm²) and the same verdict as the live Python reference.
* A design prompt with an **unavailable** provider (`freecad`) is refused
  honestly and does **not** fall back to another provider.
* A design prompt with **missing values** asks for exactly those values.
* The chat turn is journalled through the worker: one `chat.turn` event,
  the aggregate `studio`, the prompt digest and never the prompt.

The live differential also compares the chat turn **with** a chosen
`cad_provider` against the live Python public server: the reply text and
the whole design document match byte for byte (the journal head is the only
normalised field).

---

## 9. Security evidence

* **The token never reaches argv.** The token is 32 random bytes rendered
  as hexadecimal, passed only through `KALA_WORKER_TOKEN`. The lifecycle
  test reads `/proc/<worker-pid>/cmdline` and asserts the token and the
  variable name are absent, and asserts the token is absent from the
  process's stdout and stderr.
* **The worker is authenticated and not browser-facing.** The front door
  sends `Authorization: Bearer <per-launch token>` on every worker request
  and never sends an `Origin` header. The worker enforces the token, the
  `Host: 127.0.0.1:<port>` and the no-Origin policy itself.
* **The front door's own boundary is unchanged and still first.** Host
  (421) → body size (413) → Origin (403) → bearer (401) → static → router.
  The proxied routes are only reached after the front door's own gate; the
  worker is a second, independent gate behind it.
* **A worker that is down cannot be confused with a working one.** The
  geometry routes answer 503 `geometry_worker_unavailable`; the native
  routes keep working; the health document says the worker is not ready. No
  mesh, no STEP file and no measurement is ever fabricated.
* **Path safety.** The design identifier is validated as a 64-character
  lowercase sha256 **before** it is interpolated into the worker URL, so a
  crafted path segment cannot reach the worker's router. The query string
  is forwarded verbatim, after the path.
* **Bounded resources.** The request body is bounded per route (256 KiB,
  16 MiB for the STEP import), the worker response by 64 MiB, the request
  by 120 s and the start by 30 s. The worker's own body gate is forwarded
  unchanged and is never raised.
* **The journal never receives the prompt.** Only the SHA-256 digest and
  the length are journalled.

---

## 10. Intentional differences

1. **The health document has one extra field.** `geometry_worker
   {state, version, design_available}` is the ADR-0010 requirement. The
   Python public server has no worker, so the field cannot exist there.
2. **The `ChatResponse` carries `design`.** The ADR-0009 DTO. `null` when
   the turn compiled nothing.
3. **`/api/studio/import` is a new public path.** The ADR-0011 view-only
   STEP import. The Python public server does not mount it yet; the
   differential compares the front door against the worker's own handler.
4. **The chat request's `cad_provider` is a closed enum.** The front door
   accepts exactly `opencascade`, `freecad` or `solidedge` and refuses
   anything else with the source's own enum detail
   (`body.cad_provider: enum: Input should be 'opencascade', 'freecad' or
   'solidedge'`). The two refused values are in the live differential.
5. **The front door never chooses a CAD provider.** The Python reference
   (before ADR-0011) built a design with the implicit `opencascade` provider.
   The front door asks the operator instead, exactly like the current
   Python reference does. The two answers agree on the wording and on the
   provider list; they differ in the *availability* list, because the front
   door can only prove the built-in OpenCascade builder (it has no FreeCAD
   or Solid Edge adapter) while the Python reference probes the host. This is
   the one place where the front door is deliberately *less* capable than the
   reference, and it never fabricates a provider.
6. **The front door cannot build with `freecad` or `solidedge`.** The
   ADR-0011 provider clients are separate cells (`r4-solidedge-client`, the
   ADR-0007 FreeCAD adapter). Until they exist, the front door refuses
   those providers honestly ("The freecad CAD provider is not available in
   this studio build. No design was built.") instead of silently building
   with OpenCascade. The reference would build with FreeCAD on this host.
7. **The worker command is not whitespace-split.** ADR-0010 shows
   `KALA_STUDIO_GEOMETRY_WORKER="<python> -m kala_agent.studio --worker"`;
   the cell task forbids whitespace splitting. The implementation follows
   the cell task: a JSON argv array or a single executable path. The
   ADR's single-string form is refused with a clear message rather than
   silently re-parsed.
8. **The STEP import has a 16 MiB front-door limit** while the worker
   still applies its own 256 KiB gate (r1 gap 3). The front door's bound
   is an outer limit; it never raises the worker's ceiling, and the
   worker's refusal is forwarded unchanged.
9. **The native geometry is parked.** The default build links no native
   geometry crate; the primitive routes are answered by the worker.

---

## 11. Remaining gaps

1. **`GET /api/studio/providers/cad` answers the worker's 404.** The
   accepted worker contract (r1) has **13** routes and none of them is
   `providers/cad`; the Python public server has the route and probes the
   host's CAD providers. The front door proxies the route as the cell task
   requires and forwards the worker's answer unchanged, so the two answers
   differ. This is the single known gap of the live differential
   (1 of 289 cases). **Fix (Python side):** add `providers/cad` to the
   worker's internal router; the front door needs no change. The same route
   is what lets the front door report the exact provider availability in
   the chat turn's "choose a provider" message, so closing it also closes
   the availability-list difference in section 10.5.
2. **No Windows evidence.** The Windows cross-check is blocked by the
   missing mingw-w64/MSVC C toolchain that `ring` needs, and no Windows
   host was available. The worker lifecycle, the `/proc` assertions and the
   restart behaviour are Linux-only proofs. No Windows runtime, COM or
   packaging claim is made.
3. **`kala-process` has no long-lived process API.** The persistent spawn,
   the readiness line and the restart loop are implemented in this crate
   (section 4). A `kala-process` extension would move them, but that crate
   is owned elsewhere and was not patched.
4. **The frozen corpus predates the design-turns wave.** The 48-case
   `backend/fixtures/corpus/v1/studio_http.json` artifact was generated for
   the nine-route slice; the current Python adds `design_available` to the
   health document and `design` to the chat response, and the Rust front
   door adds `geometry_worker`. The three harnesses compare the documents
   without exactly those fields and byte-compare everything else. The
   fixture itself is a shared artifact and was **not** edited.
5. **The stale crate-local build directory.** `backend/crates/kala-studio/target/`
   holds artifacts from before the crate joined the root workspace,
   including a superseded `release/kala-studio` binary. Both differential
   harnesses now point at `backend/target/release/kala-studio` (the root
   workspace's target directory) and no longer fall back to the stale
   binary. The stale directory was left in place (it is a gitignored build
   artifact); a reviewer who runs the harnesses must use the root target
   directory or pass `--binary`.
6. **Every chat turn asks the worker to read the prompt.** The front door
   has no local copy of the strict parser (by design: the worker owns it),
   so an ordinary chat turn costs one extra loopback request. A worker
   that is down degrades the turn to the ordinary analytic reply.
7. **The `chat.turn` journal append is best effort.** A refused append is
   logged and the answer is still returned, so the native route keeps
   working when the worker is down. The Python reference answers 500 in
   that case; the difference only appears while the worker is down.
8. **A frozen worker executable is supported by the argv-array form only.**
   The single-path form always appends the Python module arguments.
9. **`backend/README.md` and `docs/rust-studio.md` still describe the old
   nine-route native surface.** Both are outside this cell's ownership, so
   they were not edited. The crate's own `README.md` describes the new
   architecture.

---

## 12. Honesty notes

* No runtime cutover was performed. The Python Studio is untouched and
  remains the reference oracle.
* No speed, benchmark or readiness claim is made. The front door adds one
  loopback hop for every geometry route and one `intent/parse` hop per
  chat turn.
* No Windows compilation, COM, packaging or installer claim is made.
* The geometry is measured by the worker's OCP kernel, never by Rust. The
  Rust side never fabricates a mesh, a STEP file, a measurement or a
  design.
* The `native-primitives` build is a **parked alternative**, not a second
  production path. Nothing in the default build links the native geometry
  crates.
* No commit, push, deploy or delegation was performed. No credential was
  read; the only credential-shaped strings in the tree are the synthetic
  corpus values that were already there.
