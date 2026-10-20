# cli-serve — `kala-agent serve` over the accepted native API server

Cell: `cli-serve` of the `rust-backend-20261019` wave. Owner: the Rust worker
`cli-serve`. Status: **complete**. `kala-agent serve` now starts the accepted
native `kala-api` routes over the accepted `kala-application` composition root,
with the launch contract of the Python command line. The live differential
against the repository Python CLI is green with **zero unexplained
mismatches** (96 request cases plus every launch/shutdown case in §4), and
every Rust gate that can run here passes.

No commit, no push, no deployment, no delegation and no credential read. No
user data was touched: every shell exported
`RUSTUP_HOME`/`CARGO_HOME` and a temporary `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`, and every child process was given
its own temporary data directory. `/home/v9i/.local/share/kala-agent` was never
an input to any command, test or probe.

---

## 1. What was built

The `serve` command was the one gap left by `cli-r1`: it validated the target
and then refused because no accepted server entry point existed. The
`api-routes-r1` cell has since landed `backend/crates/kala-api/server` with
`LocalApplication` and the `kala-api-server` binary. This cell:

1. exposes the launch of that crate as a **library entry point**
   (`kala_api_server::launch`), with no behaviour change to the
   `kala-api-server` binary (its accepted launch contract still passes);
2. wires `kala-agent serve` to it, reproducing the Python command's order,
   token file, summary, loopback rule, exit codes and signal handling;
3. proves the wiring with a live differential that starts **both command
   lines** and drives both with the `api-routes-r1` request corpus, plus the
   launch and shutdown cases.

### The launch path

`kala-agent serve` follows the Python command step for step:

| step | Python `kala_agent.cli.main` | native `kala-agent serve` |
| --- | --- | --- |
| parse | argparse | the ported argparse-compatible parser |
| settings | `Settings.from_env` | `kala_api::config::settings_from_env` |
| open root | `KalaApplication(settings)` | `KalaApplication::open(settings)` — **before** the command validates the target, as Python does |
| target | `build_serve_target(host, port)` — host, then port | `build_serve_target` — same checks and messages |
| server lookup | `load_uvicorn()` | `load_uvicorn()` (always `Ok`: the server is compiled in) |
| plan | `build_serve_plan` — target again, mint token, build app | `build_serve_launch` — target again, mint token, keep the value privately |
| bind/serve | `uvicorn.run(app, host, port, log_level="warning", access_log=False)` | `kala_api_server::launch(application, host, port, token)` |
| stop | summary on a normal return | summary on `SIGINT`; signal termination on `SIGTERM` (§2.4) |
| close | `finally: resolved.close()` | the launch lifespan closes the root; the command closes again (idempotent) |

### Changed paths (the only ones I touched)

```
backend/crates/kala-cli/Cargo.toml                      path dep on kala-api/server, signal-hook (unix), libc (dev)
backend/crates/kala-cli/Cargo.lock                      kala-api-server, signal-hook, signal-hook-registry
backend/crates/kala-cli/src/lib.rs                      serve wiring, plan/token, uvicorn error line, SIGTERM
backend/crates/kala-cli/tests/differential.rs           one comment: the serve cases are no longer a gap
backend/crates/kala-cli/tests/public_api.rs             the removed gap constant; load_uvicorn is Ok
backend/crates/kala-cli/tests/security.rs               serve validation/refusal tests replace the gap test
backend/crates/kala-cli/tests/serve_differential.rs     new: live differential (96 cases + launch/shutdown)
backend/crates/kala-cli/tests/serve_live.rs             new: Python-free live launch contract (6 cases)
backend/crates/kala-api/server/src/lib.rs               new public launch entry point (no behaviour change)
backend/crates/kala-api/server/src/main.rs              the binary calls the shared entry point
.mece/waves/rust-backend-20261019/cells/cli-serve/REPORT.md   this file
```

Nothing else: no other crate, no root manifest or lock, no Python source or
test, no shared fixture, no adapter, no `studio/**`. `cargo fmt` touched only
`.rs` files of the two crates named above; the server tests and manifests were
last written by `api-routes-r1` at 05:25–05:46, before this cell started.

The new dependencies are `kala-api-server` (the accepted sibling workspace, by
path) and, on Unix only, `signal-hook` (a safe wrapper used to restore the
default disposition of `SIGTERM` before re-raising it). The server crate needed
no new dependency: it already had `tokio` for its binary.

---

## 2. The Python contract, including what the live server actually does

The source of truth is the repository Python as installed in `.venv`
(uvicorn 0.54.0, FastAPI 0.141.1). Three behaviours were measured rather than
assumed:

### 2.1 The pre-bind order

`KalaApplication(settings)` runs **before** `build_serve_target`, so a refused
address still leaves the data directory opened/created and mints **no** token.
The native command reproduces this: the differential compares the data
directory and the token file for every refused launch (all 18 cases).

Inside the command the order is target → server lookup → plan (target again +
token) → bind. The native order is the same. A host/port refusal is
`application_error`, exit `1`. A malformed flag is a parser error, exit `2`.

### 2.2 The token

The token is minted to `<data-dir>/api-token` with mode `0600`, 43 URL-safe
characters and one trailing newline (44 bytes), **before** the socket exists.
It is never printed by `serve`, never logged and never in the summary; the
summary carries only its path. `kala-agent token` reads it while the server
runs and after it stops. The differential asserts the mode, length, alphabet
and exact file bytes on both sides.

When the token cannot be stored (the path is a directory), Python logs
`the launch token was not stored in the local data directory: IsADirectoryError`
and still serves with the in-memory token. The native command reproduces the
line, the class name and the in-memory enforcement; a write without the token
is `401` on both sides.

### 2.3 A failed bind

Uvicorn logs one error line to stderr, writes nothing to stdout, and exits
with `STARTUP_FAILURE` (`3`). The token is already minted. The line is:

```
ERROR:    [Errno 98] error while attempting to bind on address ('127.0.0.1', <port>): [errno 98] address already in use
```

For IPv6 it is `('::1', <port>, 0, 0)`. The native command reproduces the line
byte for byte (including the four-space prefix and the lowercased system
message), exits `3`, and keeps the token. Both the IPv4 and the IPv6 variants
are in the differential.

### 2.4 Signals — the measured contract

This surprised the source reading, so it is stated explicitly:

| signal | Python result (3/3 runs) | native result (differential) |
| --- | --- | --- |
| `SIGTERM` | graceful shutdown, then uvicorn **restores the default handler and re-raises `SIGTERM`**. The process is **terminated by signal 15**, exit status is signal 15 (shell `$? == 143`), stdout and stderr are **empty**. The summary is not printed. | same: tokio stops the server, then `signal-hook` restores the default disposition and raises `SIGTERM`; the test asserts `ExitStatus::signal() == Some(15)` and empty streams |
| `SIGINT` | graceful shutdown, re-raises `SIGINT` through asyncio's handler, `uvicorn.run` returns, the summary is printed, exit `0` | same: the summary is printed through the CLI's `json.dumps(indent=2, sort_keys=True)` writer, exit `0` |

The native re-raise needs `signal-hook::low_level::emulate_default_handler`
because tokio's signal driver installs a handler and never unregisters it. The
`kala-api-server` binary keeps its own accepted contract (summary + exit `0` on
either signal); only `kala-agent serve` follows the Python CLI.

### 2.5 The summary

On `SIGINT` (and on any normal return) both sides print exactly:

```json
{
  "command": "serve",
  "data": {
    "api_token_path": "<data-dir>/api-token",
    "authentication": "bearer",
    "host": "127.0.0.1",
    "loopback_only": true,
    "port": <port>,
    "url": "http://127.0.0.1:<port>"
  },
  "status": "stopped"
}
```

Byte-identical after normalizing the two run-specific facts (data directory and
port). IPv6 brackets the authority: `http://[::1]:<port>`.

---

## 3. Public API

### 3.1 `kala_api_server` (new entry point, no behaviour change)

```rust
use kala_api_server::{
    LaunchError, LaunchOutcome, LaunchSummary, StopSignal, launch,
};

pub fn launch(
    application: Arc<KalaApplication>,
    host: &str,
    port: u16,
    api_token: &str,
) -> Result<LaunchOutcome, LaunchError>;

pub struct LaunchSummary { pub host: String, pub port: u16, pub token_path: PathBuf }
impl LaunchSummary { pub fn url(&self) -> String }

pub struct LaunchOutcome { pub summary: LaunchSummary, pub stopped_by: StopSignal }
pub enum StopSignal { Interrupt, Terminate }

pub enum LaunchError {
    Address(String),
    State(String),
    Runtime(String),
    Bind { host: String, port: u16, source: std::io::Error },
    Server(String),
}
impl LaunchError { pub fn code(&self) -> &'static str; pub fn message(&self) -> String }
```

`launch` validates the loopback address and a non-zero port, builds the
`Routes` over `LocalApplication`, builds the `ApiState` with the caller's
token, installs `SIGINT`/`SIGTERM` handlers **before** the socket exists, binds,
serves, drains the in-flight connections, closes the root through the
`Lifespan`, and returns the summary and the signal that stopped it.

The binary `kala-api-server` now calls this function. Its flags, environment,
token path/mode, JSON summary, exit codes and SIGTERM/SIGINT behaviour are
unchanged: its 8 launch-contract tests and its 96-case route differential still
pass. The only new failure mode is a signal-registration error, which is
reported as a sanitized `local_failure` instead of panicking; it is unreachable
in ordinary operation.

### 3.2 `kala-cli`

Unchanged signatures:

```rust
pub fn build_serve_target(host: Option<&str>, port: Option<&BigInt>)
    -> Result<ServeTarget, ApplicationError>;
pub fn build_serve_plan(application: &KalaApplication,
    host: Option<&str>, port: Option<&BigInt>) -> Result<ServePlan, ApplicationError>;
```

`build_serve_plan` now mints the token with `mint_api_token` +
`store_api_token` (one mint, as before) so the command can keep the value for
the server while the public plan still carries only the path. Its Debug form
still never shows a token. `load_uvicorn()` now always returns `Ok(())`: the
native server is compiled in, and the port keeps the Python call order.

New/changed constants:

* `EXIT_SERVE_STARTUP_FAILURE = 3` — uvicorn's startup-failure code;
* `SERVE_INJECTED_APPLICATION_MESSAGE` — the native refusal for a borrowed root;
* removed `SERVE_ROUTE_LAYER_GAP_MESSAGE` (the gap no longer exists);
* `UVICORN_MODULE_NAME`, `SERVE_LOG_LEVEL` and `SERVE_UNAVAILABLE_MESSAGE`
  remain as the ported Python constants, but the native path never uses them.

`main` now dispatches `serve` before the borrowing commands because the native
server needs an owned `Arc<KalaApplication>`. The binary always opens its own
root, so this is invisible to `kala-agent serve`; see §5.4 for the
library-only injected-root case.

---

## 4. The live differential

`backend/crates/kala-cli/tests/serve_differential.rs` starts the **real**
Python command line and the **real** native binary, each on its own temporary
data directory and HOME, waits for both sockets, and drives both with the
96-case request script of the accepted `api-routes-r1` cell (the same corpus
as `backend/crates/kala-api/server/tests/routes_differential.rs`).

### 4.1 The request corpus

96 cases: every route (health, domains, evaluations, revisions, prepared
commits with approve/reject, the mechanical workflow with its checkpoints and
decisions, the journal), every validation failure, the conflicts, the unknown
resources, the pagination, and the request boundary (missing/wrong/non-bearer
token, refused/rebound Host, refused Origin, oversized body, duplicate Host,
non-UTF-8 body, bodies with the wrong content type, repeated keys, non-finite
numbers, large integers).

Comparison: status, `content-type`, `content-length`, `location`, and the
exact body bytes. Normalizations, and only these:

1. the data directory and the port (the facts of each run);
2. ISO-8601 aware timestamps (the two runs happen at different times);
3. runs of 16+ hexadecimal digits (the declared native identity boundary of
   `kala-domains`/`kala-application`).

One case is a declared parser refusal: `host-repeated` is `400` from uvicorn's
HTTP parser and `421 invalid_host` from the native application, exactly as
declared by `api-core-r1`/`api-routes-r1`. The test asserts that shape (95
strict comparisons + 1 declared).

Result: **96/96, 0 unexplained mismatches**, identical in debug and release.
The measured line is:

```
the live serve differential compared 96 cases (95 of them strictly, 1 declared parser refusals)
```

### 4.2 The launch and shutdown cases

| case | comparison | result |
| --- | --- | --- |
| initial launch with explicit `--data-dir`/`--host`/`--port` | token file mode/length/content, health, token gate, summary, token command after stop, database table row counts | equal |
| `SIGINT` | exit `0`, empty stderr, summary bytes after path/port normalization | equal |
| `SIGTERM` | `ExitStatus::signal() == 15`, stdout and stderr empty, token present | equal |
| 18 refused launches (wildcard, `::`, routable, hostname, blank host, ports 0/-1/65536/1000000/huge, `--port=0`, `--host=0.0.0.0`, non-integer, missing values, unknown flag, ambiguous `--h`) | exit, stdout, stderr, data-directory existence, token absence | equal |
| IPv4 port in use | exit `3`, empty stdout, exact uvicorn line, token present | equal |
| IPv6 port in use | same on `::1` (`('::1', port, 0, 0)`) | equal |
| a privileged port the user cannot bind (port 80) | exact `[Errno 13]` line, exit `3` (skipped for root) | equal |
| unstorable token (`api-token` is a directory) | warning line, in-memory token gate, summary, exit `0` | equal |
| IPv6 loopback launch | both serve `/health` on `::1`, summary host/URL bracketing | equal |

Python's **request-level** log lines for the two declared transport cases
(`WARNING:  Invalid HTTP request received.` from the duplicate-Host case and
Starlette's deprecation warning when the 413 handler first runs) are removed
before the launch stderr comparison; the native server has no request logger.
Nothing else is filtered, and native stderr is asserted empty. This is
declared in §5.3.

### 4.3 The Python-free live contract

`backend/crates/kala-cli/tests/serve_live.rs` (6 cases) starts only the native
binary and pins the same contract without an oracle: token mode/length, health,
token gate, the `token` command, the `SIGINT` summary, `SIGTERM` termination by
signal, the exact uvicorn bind line and exit `3`, the pre-token refusals, the
unstorable-token warning, and the IPv6 bind.

---

## 5. Intentional differences

Each is measured or inherited, not hidden.

### 5.1 A hostname binds one resolved address

Python's asyncio binds **every** address `getaddrinfo` returns for a hostname.
On this machine `--host localhost` makes the Python command listen on both
`127.0.0.1` and `::1`; the native launch binds the first resolved address only
(`::1` here), so a client connecting over IPv4 to the native launch is refused.
The advertised hosts are the literals `127.0.0.1` and `::1`, where the two
commands are identical (including the IPv4/IPv6 bind-failure lines). A
multi-listener bind would have required changing the accepted single-listener
`kala_api::server::serve`, which this cell may not do.

### 5.2 Uvicorn's terminal colours

Uvicorn colourises its `ERROR:` line when stderr is a terminal. The native line
is the plain form that uvicorn writes to a pipe; the differential compares
pipes. No content difference otherwise.

### 5.3 Request-level server logging

The Python ASGI server logs request-level warnings (duplicate Host) and Starlette
emits a deprecation warning the first time the 413 handler runs. The native
transport has no request logger. These lines are removed from the Python launch
stderr before comparison as declared artifacts of the request corpus; the
launch's own stderr is empty.

### 5.4 An injected composition root cannot be served

`kala_cli::main(argv, Some(&application), ...)` with `serve` is refused with
`application_error` and `SERVE_INJECTED_APPLICATION_MESSAGE`; the Python
`main` serves an injected application and leaves it open. The native route port
requires a `'static` `Arc`, so a borrowed root cannot be shared with it. The
binary never takes this path (it always opens its own root). This is the same
difference `cli-r1` declared; the refusal now happens after the target check,
so a refused address has the same precedence as Python.

### 5.5 A fatal accept-loop error

Python's command line maps an `OSError`/`RuntimeError` from the server to
`local_failure` with the exception class name. The native launch reports
`local_failure` with its sanitized message. The code is the same; the text is
the native failure's. This path is beyond a normal launch (transient accept
errors are retried).

### 5.6 Inherited from the accepted cells

The following differences are unchanged from `cli-r1`, `application-r1`,
`api-routes-r1` and `api-core-r1`, and are not re-opened here: constructor
failure typing, the parameter-validation wording/aggregation of
`kala-domains`, deep JSON past CPython's recursion limit, non-UTF-8 argv,
terminal width detection, and the OpenAPI/documentation routes
(`/openapi.json`, `/docs`, `/docs/oauth2-redirect`) which the native routes do
not serve (§6).

---

## 6. Tests and gates

### 6.1 `backend/crates/kala-cli`

| suite | tests |
| --- | ---: |
| `tests/differential.rs` (the 262-step command differential) | 2 |
| `tests/public_api.rs` | 7 |
| `tests/security.rs` | 14 |
| `tests/serve_differential.rs` | 6 |
| `tests/serve_live.rs` | 6 |
| unit tests in `src/` | 0 |
| **total** | **35** |

All 35 pass in debug and release, 0 failed, 0 ignored.

### 6.2 `backend/crates/kala-api/server`

| suite | tests |
| --- | ---: |
| unit tests in `src/` | 1 |
| `tests/launch_contract.rs` | 8 |
| `tests/routes_differential.rs` (96 cases) | 1 |
| **total** | **10** |

All 10 pass in debug and release, 0 failed, 0 ignored. The binary's preserved
behaviour is covered by these 10 cases.

### 6.3 Gates

Run in each crate directory. Toolchain: `cargo 1.99.1`, `rustc 1.99.1`.

| gate | `kala-cli` | `kala-api/server` |
| --- | --- | --- |
| `cargo fmt --check` | pass | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass, no warnings | pass, no warnings |
| `cargo test --locked` | 35 passed | 10 passed |
| `cargo test --locked --release` | 35 passed | 10 passed |
| `cargo build --locked --release` | pass | pass |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **cannot compile**: the target is installed, but the mingw C compiler is missing (`cc-rs: failed to find tool "x86_64-w64-mingw32-gcc"`, from the C dependencies of the accepted crates) | same |

The Windows check fails in a build script of the accepted C dependencies, not
in this cell's code. No Windows runtime claim is made.

The gates were re-run after the last source edit. Two consecutive debug runs and
two release runs produced the same results. The live differential needs
`/home/v9i/Projects/Kala-Agent/.venv/bin/python` (override with `KALA_PYTHON`)
and skips with a printed message if it cannot import the live command line; the
library and both binaries have no Python dependency.

---

## 7. Compatibility and security evidence

* **The same bytes.** The differential compares the exact body bytes of all 96
  route cases and the exact summary bytes after only the declared
  normalizations.
* **The same launch side effects.** The data directory is opened before the
  target check (both sides), the token is minted before the socket (both
  sides), and refused targets mint no token (both sides). The native launch's
  database tables and row counts match the Python launch after the corpus.
* **The token never leaves.** `serve` prints only the token path. The token file
  is `0600`, 43 URL-safe characters plus a newline, and a byte scan of the whole
  data directory after the 96-case corpus finds it **only** in `api-token`, on
  both sides. The served API refuses an unauthenticated write with `401`; the
  corpus exercises missing, wrong, non-bearer, refused-Host and refused-Origin
  requests. `kala-agent token` reads the token both while the server runs and
  after it stops.
* **Loopback only.** A wildcard, routable, hostname-resolved-non-loopback,
  blank or `::` address is refused before the token and before a socket. IPv6
  literals are bracketed in the URL.
* **A failed bind is sanitized.** The stderr line names the errno, the address
  and the OS message only; no token, no environment value, no traceback.
* **An unstorable token still guards the process.** The warning names the
  exception class only; the in-memory token is enforced; the summary still
  reports the path.
* **Signals.** `SIGTERM` terminates by signal with no output, exactly as the
  Python command; `SIGINT` prints the summary and exits `0`.
* **No credential reads.** No credential file was read, and no secret appears
  in the test or probe output.

---

## 8. Remaining gaps

* **A hostname binds one address** (§5.1). The literal loopback addresses are
  exact.
* **The framework routes are still absent** (`/openapi.json`, `/docs`,
  `/docs/oauth2-redirect`), inherited from `api-routes-r1` §7.6. The accepted
  request corpus does not carry them.
* **An injected composition root cannot be served** (§5.4).
* **Request-level logging is not reproduced** (§5.3).
* **Windows cross-check** needs the mingw C toolchain (§6.3); no Windows
  runtime is claimed.
* **No runtime cutover, packaging, speed or readiness claim.** The Python
  console script remains the installed entry point; this cell wires the native
  command and proves it against the live Python command.
