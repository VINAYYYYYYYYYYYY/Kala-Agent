# api-routes-r1 — the core API routes and the native server

**Cell:** `api-routes-r1` · **Worker:** Rust worker `api-routes-r1` (Opus
conductor, wave `rust-backend-20261019`) · **Date:** 2026-10-20

## 1. What was built

The routes of the local Kala API are ported into the accepted crate
`backend/crates/kala-api`, and a native server that serves them over the
accepted composition root is added next to it.

| Python module | Rust module | what it carries |
| --- | --- | --- |
| `src/kala_agent/api/app.py` | `backend/crates/kala-api/src/routes.rs` | every route, the route table, the answers, the lifespan |
| `src/kala_agent/api/dependencies.py` | `backend/crates/kala-api/src/routes.rs` | the dependency wiring and the bounds of the path and query parameters |
| `src/kala_agent/api/app.py` (the body layer) | `backend/crates/kala-api/src/jsoninput.rs` | the reader of a request body, with the position of a decoding failure |
| `src/kala_agent/api/app.py` (the answers) | `backend/crates/kala-api/src/wire.rs` | the bytes of an answer, in the form the live API writes them |
| `src/kala_agent/cli/main.py` (`serve`) | `backend/crates/kala-api/server/src/main.rs` | the `kala-api-server` binary and its launch contract |
| `src/kala_agent/application.py` (the views) | `backend/crates/kala-api/server/src/lib.rs` | the adapter that binds the composition root to the routes |

Every route of the Python source is here: the health route of the root
router, the domain packages, the evaluations, the revisions, the prepared
revision commits with their approve and reject decisions, the mechanical
reference workflow, the workflow checkpoints with their approve and reject
decisions, and the journal. The dependency wiring, the startup and the
shutdown, and the root router are here as well.

The live differential drives the **live** Python FastAPI application and the
**native** server with the very same request script and compares the status,
the headers that matter and the exact body bytes of **96 cases**: **0
unexplained mismatches**.

### The changed paths

```
backend/crates/kala-api/Cargo.toml                   (serde_json, the number format of the answers)
backend/crates/kala-api/Cargo.lock                   (serde_json)
backend/crates/kala-api/README.md                    (the modules, the counts, the routes)
backend/crates/kala-api/src/lib.rs                   (the new modules and the re-exports)
backend/crates/kala-api/src/jsoninput.rs             (new: the reader of a request body)
backend/crates/kala-api/src/routes.rs                (new: the routes and the port they call)
backend/crates/kala-api/src/wire.rs                  (new: the wire form of the answers)
backend/crates/kala-api/src/schemas.rs               (the three Default impls and two constants)
backend/crates/kala-api/server/Cargo.toml            (new: the standalone workspace of the server)
backend/crates/kala-api/server/Cargo.lock            (new)
backend/crates/kala-api/server/src/lib.rs            (new: the composition adapter)
backend/crates/kala-api/server/src/main.rs           (new: the kala-api-server binary)
backend/crates/kala-api/server/tests/oracle_routes.py       (new: the live Python oracle)
backend/crates/kala-api/server/tests/routes_differential.rs (new: the live differential)
backend/crates/kala-api/server/tests/launch_contract.rs      (new: the launch contract)
.mece/waves/rust-backend-20261019/cells/api-routes-r1/REPORT.md   (this file)
```

Nothing else was touched: no Python file, no test of the repository, no other
crate, no root manifest or lock, no `studio/**`, no shared fixture. The
Python source was read as it is in the working tree, including the
uncommitted changes of `api/__init__.py`, `api/envelope.py`, `api/middleware.py`
and `api/schemas.py`. No commit, no push, no delegation, no credential read.

## 2. The one structural problem, and how it is solved

The brief asks the routes to be built **on** `kala-application` and to use
its DTOs. That direction is a **package cycle**, and Cargo refuses it:

```
error: cyclic package dependency: package `a v0.1.0` depends on itself. Cycle:
package `a v0.1.0 (...)
    ... which satisfies path dependency `a` of package `b v0.1.0 (...)
    ... which satisfies path dependency `b` of package `a` v0.1.0 (...)
```

The accepted composition root already depends on the accepted connection
layer — `backend/crates/kala-application/Cargo.toml:18` declares
`kala-api = { path = "../kala-api" }` for its `Settings` — and
`kala-application` is read-only for this cell. A `kala-api` that depends on
`kala-application` therefore cannot be built, and neither can a crate that
depends on both.

The cycle is broken with the **port** the Python source already uses:

* `kala_api::routes::Application` is the native form of the
  `ApplicationDependency` of `api/dependencies.py`: the router holds the
  composition root, and the handlers call it. The trait answers the document
  of a route, or the failure the composition root reports.
* The routes live in `kala-api` (as the brief asks) and hold **no**
  composition root: they are transport only.
* The other side of the port lives in `backend/crates/kala-api/server`, the
  nested crate that binds the accepted `kala-application` root to the routes
  and holds the binary.

The result is a crate that **both** `kala-application` and the accepted
`kala-cli` crate can depend on, which the cycle would have made impossible.

The `server/` crate is a **new standalone workspace** (`[workspace]
members = []`) with its own `Cargo.lock`, nested inside `backend/crates/kala-api`
so that every path this cell touches stays inside the boundary the brief
sets. It consumes `kala-api`, `kala-application`, `kala-contracts` and
`kala-json` read-only, by path.

## 3. The public API

### 3.1 `kala_api::routes`

```rust
use kala_api::routes::{
    API_PREFIX, API_VERSION, Application, Created, EventQuery, LOCAL_DISCLAIMER,
    Lifespan, MechanicalRun, Routes,
};
```

* `Routes::new(Arc<dyn Application>)` builds the router of one launch, and
  `Routes` implements the `server::Router` trait of the connection layer.
* `trait Application` is the port the routes call:
  `health`, `list_domains`, `read_domain`, `evaluate_domain`,
  `create_revision`, `prepare_revision_commit`, `read_revision_commit`,
  `approve_revision_commit`, `reject_revision_commit`, `list_revisions`,
  `read_revision`, `run_mechanical_reference`, `read_workflow_state`,
  `list_checkpoints`, `approve_checkpoint`, `reject_checkpoint`,
  `list_events`, `closed` and `close`.
* `struct Created { document, identifier }` is a created resource: its
  document and the identity its `location` header names.
* `struct MechanicalRun { document, workflow_id, started }` is a run: its
  document, the workflow it belongs to, and whether this call started it (the
  `201` of a started run against the `200` of a resumed one).
* `struct EventQuery { after_sequence, limit, workflow_id, event_type,
  aggregate_id }` is the query of the journal route.
* `struct Lifespan { application, owns }` is the lifecycle of one launch:
  `Lifespan::new(application, owns)` and `Lifespan::shutdown()` close the
  root the launch created and leave a root it was given alone, exactly as
  `build_lifespan` does in the Python source.
* The constants of the module are the ones of the Python source: `API_PREFIX`
  (`/api/v1`), `API_VERSION` (`1.0.0`), `LOCAL_DISCLAIMER`, `LOOPBACK_SERVER`,
  `TAGS`, and the bounds of the query parameters.

### 3.2 `kala_api::jsoninput`

* `pub fn parse(text: &str) -> Result<CanonicalValue, JsonError>` reads a
  request body the way CPython's `json.loads` reads one.
* `struct JsonError { position }` carries the character index at which the
  scanner of CPython stopped, which is the index the detail line of a
  rejected body carries (`body.29: json_invalid: JSON decode error`).

### 3.3 `kala_api::wire`

* `pub fn encode_answer(&WireValue) -> Result<String, ContractError>` writes an
  answer the way the live API writes it (see §7.1).

### 3.4 `kala_api::schemas`

* `Default` is implemented for `EvaluateDomainRequest`,
  `RunMechanicalWorkflowRequest` and `CheckpointDecisionRequest`, with the
  defaults the Python models declare, so a route whose body is absent can
  build the request the Python source builds.
* `DEFAULT_MECHANICAL_DESIGN_ID` and `DEFAULT_ACTOR_ID` name those defaults.

### 3.5 `kala_api_server`

* `struct LocalApplication` binds an `Arc<KalaApplication>` to the
  `Application` port: `LocalApplication::new(application)`.
* The binary `kala-api-server` is the native counterpart of the `serve`
  command of the Python command line adapter.

## 4. The launch contract of the binary

```
kala-api-server [--data-dir DATA_DIR] [--host HOST] [--port PORT]
```

* `--data-dir` names the local data directory and overrides
  `KALA_AGENT_DATA_DIR` for this invocation only. The other six environment
  variables of the configuration are read as they are.
* `--host` names the loopback address to bind (the default is `127.0.0.1`).
  An address that is not a loopback interface is refused **before a socket
  exists**, with the message of the Python source.
* `--port` names the bounded TCP port (the default is `8000`); a port outside
  `1..=65535` is refused.
* The launch token is minted and written to `<data-dir>/api-token` with
  owner-only permissions before the socket exists, and is never printed,
  never logged and never returned.
* The launch serves until `SIGINT` or `SIGTERM`, and then prints one JSON
  document on stdout — the summary of the Python source: the command, the
  data directory of the token, the authentication, the host, the port, the
  URL and the status.
* Every failure is one JSON document on stderr. The exit codes are the ones
  of the Python source: `0` the server stopped, `1` the launch could not be
  completed, `2` a usage error.

## 5. Fidelity

The whole of `app.py` and `dependencies.py` is ported: every route, the
dependency wiring, the startup and the shutdown, the root router, the
validation of every path parameter, of every query parameter and of every
body, the statuses, the `location` headers, the sanitized error envelope and
the status table of the composition root failures.

Details that mattered:

* **The route table and its order.** The routes are matched in the order the
  Python source registers them, the first route that matches the path and
  the method answers, and the first route that matches only the path answers
  a `405`. FastAPI does not add `HEAD` to a `GET` route, so `HEAD` on a read
  is a `405` as well.
* **The trailing slash.** A path that ends with a slash, or that misses one,
  is answered with a `307` and the absolute URL of the other spelling, which
  is built from the `Host` header of the request and quoted the way
  `urllib.parse.quote` quotes it.
* **The body is read as JSON only when the content type names it.** FastAPI
  0.141 reads a body as JSON when the content type is `application/json` or
  `*+json`; a request that carries no content type hands its raw bytes to
  the model, and the model refuses them because they are not an object.
* **The position of a decoding failure.** The detail line of a rejected body
  carries the character index at which the scanner of CPython stopped, which
  is not always the end of the document (`{"a": 1,}` is reported at the
  comma, an unterminated string at its opening quote, a document that carries
  something after its value where that something starts). `jsoninput` is an
  explicit-stack state machine that reproduces those positions; 47 of them are
  pinned by the unit cases of the module, and the live differential reads the
  position the live API writes.
* **The integer of a query parameter.** Pydantic reads the surrounding
  whitespace away, ignores an underscore between two digits, accepts a
  decimal spelling without a fractional part, refuses a value with more
  digits than CPython converts (`int_parsing_size`) and compares the value
  as an arbitrary precision integer, so a 30-digit value is refused by the
  bound and not by the reader.
* **Every failure of a request is collected.** The framework answers a
  request that carries a bad path parameter, a bad query parameter and a bad
  body with all of the failures, in that order. The routes collect them the
  same way.
* **The path is decoded before it is matched**, exactly as the server of the
  Python source decodes it: `%2F` is a slash, and the route does not match.
* **A closed composition root is refused before any route runs**, with the
  `application_closed` code and the `503` of the Python source.
* **The answers carry the DTOs of the composition root.** The route answers
  are built from the DTOs of `kala-application` and not from a second model
  of the views: the adapter of the `server/` crate writes
  `DomainPackageView`, `DomainEvaluationReport`, `RevisionRecord`,
  `RevisionCommitView`, `MechanicalRunResult`, `WorkflowStateView`,
  `CheckpointView`, `EventPage` and the rest with their own wire form.

## 6. The live differential

`backend/crates/kala-api/server/tests/oracle_routes.py` drives the **live**
FastAPI application of `kala_agent.api` — the real middleware stack, the real
routers, the real error handlers and the real composition root — over a
loopback socket, and writes a transcript of the answers.
`backend/crates/kala-api/server/tests/routes_differential.rs` drives the
**native** server with the very same request script and compares the answers.

| compared | cases | mismatches |
| --- | ---: | ---: |
| every route of the API, over a socket | **96** | **0** |

95 of the 96 cases are compared strictly: the status, the `content-type`,
the `content-length` and the `location` header, and the exact body bytes.
The 96th case is the request that carries a duplicated `Host` header, which
the two parsers answer differently (§7.4).

The corpus covers:

* **every route**: the health route, the domain packages, one package, the
  evaluations of a package, the revisions, one revision, the prepared commits,
  the read of a commit, the approval of a commit, the rejection of a commit,
  the mechanical reference workflow, the state of a workflow, the checkpoints
  of a workflow, the approval and the rejection of a checkpoint, and the
  journal;
* **the success answers**: the created revision, the prepared commit, the
  approved commit, the rejected commit, the started and the resumed run, the
  approved checkpoint, and the pages of the journal;
* **the validation failures**: a missing field, an undeclared field, a
  refused enum, a refused identifier, a refused digest, a body that is not
  an object, an empty body, a body that is not JSON (with the position of the
  scanner), a body that is not UTF-8, a body without a content type, a body
  with a content type that names something else, a repeated key, a number
  that is not finite, a large integer, a request that carries two failures at
  once;
* **the conflicts**: a stale digest, a stale parent, the approval of a
  commit that is already decided, the rejection of a commit that is already
  decided, the second approval of a checkpoint, the rejection of a workflow
  that is finished;
* **the unknown resources**: an unknown domain package, an unknown revision,
  an unknown commit, an unknown workflow, an unknown route;
* **the finality of an approval**: the same commit approved twice, the same
  commit rejected after it was approved, the same checkpoint approved twice;
* **the pagination**: the journal with a limit of one, the journal after a
  sequence, the journal after the last sequence, the journal with a
  repeated parameter, the revisions with a limit;
* **the request boundary**: a missing token, a wrong token, a token that is
  not a bearer, a refused host, a rebound host, a refused origin, an origin
  on a read, an oversized body, a repeated host header.

Two normalizations are applied to the answers, and only two:

1. **the wall clock**, because the two runs happen at different times: every
   aware timestamp of the ISO 8601 form becomes `<timestamp>`;
2. **the native identity of the domain packages**, because the accepted
   `kala-domains` crate publishes native Rust digests and not the historical
   Python source digests: every run of sixteen hexadecimal digits or more
   becomes `<digest>`. That covers the package digests, the parameter schema
   digests, the report artifact digests, the specification digests, the state
   and action digests, the journal event digests and the commit identity
   that carries two of them.

Two facts of the launch are normalized as well, because they are the facts of
the two runs and not of the answers: the data directory (the health route
reports it) and the port (the redirect names it).

The differential is **sensitive**: it compares the exact bytes of the answers,
and every detail of the port (the position of a decoding failure, the
message of a refused value, the number format of a floating point number) is
visible in it.

## 7. Intentional differences

### 7.1 The number format of a floating point number

The live API writes its answers with the JSON serializer of Pydantic, which
is written in Rust, and not with the JSON serializer of CPython. The two
write the same bytes for everything but the floating point numbers: the Rust
serializer writes a number in the fixed form whenever the fixed form is the
shorter one, and CPython writes the scientific form. `2.36e-05` is
`0.0000236` in the answer of the live API and `2.36e-05` in the answer of
`json.dumps`.

`kala_api::wire::encode_answer` reproduces the form of the live API, and the
differential pins it down (`material.thermal_expansion` is the value that
carries it). The accepted `kala-contracts` crate writes the CPython form,
because the *application* layer compares against `json.dumps`; the two forms
are both correct for their own layer, and the API layer writes the form of
the API.

### 7.2 The JSON reader of a body

`jsoninput::parse` reads a body the way CPython's `json.loads` reads one:
a repeated object key is *last wins* and not a failure, and `NaN`,
`Infinity` and `-Infinity` are numbers and not failures. The reader is an
explicit-stack state machine: a document nested far beyond any ordinary
recursion limit is read on an ordinary thread stack, and a document that is
malformed at any depth fails closed without recursing.

A body that carries a lone surrogate is rendered with the replacement
character, because a Rust string cannot carry one. The Python source keeps
it. This is the same difference the `application-r1` cell declares for the
composition root.

### 7.3 A body that is deeper than the recursion limit of CPython

CPython's reader refuses a document that is nested deeper than its recursion
limit with a `RecursionError`, and the API answers that with a `500`. The
native reader accepts it and answers with the `422` of the model. The
`schemas-r1` cell already declares this difference for the body layer of the
schemas; the corpus of this cell does not carry such a body, because the
difference is declared and not a defect of this port.

### 7.4 A request that a parser refuses before the application

A request that carries a duplicated `Host` header is refused by the parser
of the Python server (`400 Invalid HTTP request received.`) before the
application sees it, and the parser of the native server hands it to the
application, which refuses it with its own answer (`421 invalid_host`). The
difference belongs to the parsers and not to the routes, and the `api-core-r1`
cell already declares it. The case is in the corpus and the test asserts the
documented shape of the difference.

### 7.5 The duplicated views of the application

The brief asks the duplicated application views of `kala-api/src/views.rs` to
be replaced by the DTOs of `kala-application`. The **answers of the routes**
use the DTOs of the composition root and not a second model of the views
(§5). The module `views.rs` itself stays, for two reasons that are both
outside this cell:

* the accepted response models of `kala-api/src/schemas.rs` embed it, and the
  accepted `schemas-r1` differential (560 cases) validates them against the
  live Pydantic models. A DTO of the composition root is *write only*: it is
  constructed by the root and cannot be read back from a document, so it
  cannot validate one;
* the response models cannot be moved either, because the accepted
  `schemas-r1` suite is part of the accepted state of `kala-api`.

So `views.rs` is now only the validating form of the accepted response
models, and the routes do not use it. Removing it is a change of the
`schemas-r1` cell, not of this one.

### 7.6 The OpenAPI document and the documentation pages

The live application also serves `/openapi.json`, `/docs` and
`/docs/oauth2-redirect`. Those are the routes of the framework, not of
`app.py`, and they are not ported: the OpenAPI document is generated by
FastAPI from the signatures of the handlers and the JSON schema of every
model, and porting it is a piece of work of its own. The corpus of this cell
does not carry them, and no claim is made about them.

### 7.7 The framing of the two servers

The two servers add their own framing headers (`date`, `server`,
`connection`). They belong to the servers and not to the application, and the
differential compares the headers of the application only. The `api-core-r1`
cell declares the same.

## 8. Compatibility and security evidence

* **The same bytes.** The differential compares the exact body bytes of
  every route. The answers are built from the DTOs of the composition root,
  so the field order, the escaping, the number format and the UTF-8 text are
  the bytes of the live API.
* **The same status table.** Every failure of the composition root is mapped
  onto the status the Python error handlers map it onto, with the code and
  the message of the failure: the lookup failures are a `404`, the conflicts
  are a `409`, the corrupt and the closed root are a `503`, the refused
  identifiers and parameters are a `422`, and the rest is a `400`.
* **The same refusal of a closed root.** A root that was closed is refused
  before any route runs, with `application_closed` and a `503`.
* **The request boundary is unchanged.** The gates of the connection layer
  (the Host guard, the per-launch token, the cross-origin guard, the body
  limit and the security headers) are the accepted ones, and the corpus
  exercises them on the routes: a missing token, a wrong token, a token that
  is not a bearer, a refused host, a rebound host, a refused origin and an
  oversized body are all refused by the native server exactly as they are by
  the live one.
* **The token never leaves.** The launch writes it to the local data
  directory with owner-only permissions (`0600`, asserted by the launch
  contract), it is never printed, never logged and never returned in a
  response, and a mutating request without it is refused.
* **The answers carry no secret.** Every failure is the sanitized envelope of
  the Python source: a code, a bounded message and the sanitized detail
  lines. A rejected body reports the kind, the place and the message of the
  failure, never the submitted value.
* **The reader fails closed.** The body reader is iterative: a document of
  two thousand levels is read without recursing, and a document that is
  malformed at any depth fails without recursing.
* **The launch binds the loopback interface only.** An address that is not a
  loopback interface is refused before a socket exists, and the launch
  reports it with the message of the Python source.

## 9. The tests

### 9.1 `backend/crates/kala-api`

| suite | tests |
| --- | ---: |
| unit tests in `src/` | 85 |
| `tests/differential.rs` | 2 |
| `tests/config_differential.rs` | 2 |
| `tests/schemas_differential.rs` | 1 |
| `tests/schemas_security.rs` | 19 |
| `tests/security_boundaries.rs` | 14 |
| `tests/socket_differential.rs` | 1 |
| doc tests | 0 |
| **total** | **124** |

The 100 tests the crate had before this cell are still green, and the 24 new
ones are the routes (16), the reader of a body (5) and the wire form (3) of
the new modules.

### 9.2 `backend/crates/kala-api/server`

| suite | tests |
| --- | ---: |
| unit tests in `src/lib.rs` | 1 |
| `tests/routes_differential.rs` | 1 (96 cases) |
| `tests/launch_contract.rs` | 8 |
| **total** | **10** |

All 134 tests pass in the debug profile and in the release profile.

## 10. The gates

### `backend/crates/kala-api`

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --check` | clean |
| lint | `cargo clippy --locked --all-targets --all-features -- -D warnings` | clean, no warnings |
| tests (debug) | `cargo test --locked` | **124 passed**, 0 failed |
| tests (release) | `cargo test --locked --release` | **124 passed**, 0 failed |
| release build | `cargo build --locked --release` | success |
| Windows compile | `cargo check --locked --target x86_64-pc-windows-gnu` | success (the target **is** installed) |

### `backend/crates/kala-api/server`

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --check` | clean |
| lint | `cargo clippy --locked --all-targets --all-features -- -D warnings` | clean, no warnings |
| tests (debug) | `cargo test --locked` | **10 passed**, 0 failed |
| tests (release) | `cargo test --locked --release` | **10 passed**, 0 failed |
| release build | `cargo build --locked --release` | success |
| Windows compile | `cargo check --locked --target x86_64-pc-windows-gnu` | **cannot run**: the target is installed, but the mingw C toolchain is not (`libsqlite3-sys` and `ring`, the C dependencies of the accepted `kala-persistence` and `kala-runtime` crates, fail in their build scripts with `failed to find tool "x86_64-w64-mingw32-gcc"`). This is a missing cross toolchain and not a code error; the `application-r1` cell reports the same for the same reason. No Windows runtime claim is made. |

Toolchain: `cargo 1.99.1`, `rustc 1.99.1`. Two consecutive debug runs and two
release runs produced the same result.

## 11. Remaining gaps

* **The OpenAPI document and the documentation pages are not ported** (§7.6).
* **The duplicated views are still in `kala-api/src/views.rs`** (§7.5): the
  routes do not use them, but the accepted response models of the
  `schemas-r1` cell do, and they cannot be replaced without a change of that
  cell.
* **A body that is nested deeper than the recursion limit of CPython** is
  answered differently (§7.3). The difference is declared by the
  `schemas-r1` cell already.
* **A request that a parser refuses before the application** is answered
  differently by the two servers (§7.4). The difference belongs to the
  parsers and is declared by the `api-core-r1` cell already.
* **The Windows cross-check of the server cannot run** (§10): the mingw C
  toolchain is absent here.
* **No runtime cutover was performed.** The Python reference is in place and
  untouched, the native server is not wired into any running service, and no
  speed, readiness or production claim is made.
* **The crate holds the routes and the connection layer; the composition
  root is bound to them by the nested `server/` crate** (§2). A cell that
  wants the routes and the root in one package has to break the cycle of
  §2 in the `kala-application` manifest, which is the accepted state and not
  this cell's to change.
