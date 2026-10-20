# kala-api — the Kala core API connection layer

`kala-api` is the transport's part of the local Kala API. It is a native Rust
port of the Python modules `kala_agent.config`, `kala_agent.api.boundary`,
`kala_agent.api.envelope`, `kala_agent.api.middleware`,
`kala_agent.api.errors`, `kala_agent.api.schemas`, `kala_agent.api.app` and
`kala_agent.api.dependencies`, and of the parts of `kala_agent.api` they rely
on.

It is a **standalone workspace** with its own `Cargo.lock`; the parent
`backend/` workspace is owned by another cell and is not touched. The
accepted `kala-json` and `kala-contracts` crates are consumed read-only, as
path dependencies.

The crate holds two layers:

* the **connection layer**: the boundary gates, the body limit, the security
  headers, the sanitized error envelope and the models of the API;
* the **routes**: the port of `kala_agent.api.app` and
  `kala_agent.api.dependencies`, with the route table, the validation of the
  path parameters, of the query parameters and of the body, and the answers.

The routes reach the composition root through the `routes::Application` port,
so this crate does not depend on the composition root and the composition root
does not depend on the routes. The `server/` crate is the other side of that
port: it binds the accepted `kala-application` root to the routes, and it holds
the `kala-api-server` binary.

```
backend/crates/kala-api
├── Cargo.toml            standalone workspace, members = []
├── Cargo.lock            this crate's own lock
├── src
│   ├── lib.rs            the crate surface
│   ├── boundary.rs       the Host, bearer token and origin gates
│   ├── config.rs         the local settings and their validation
│   ├── datetime.rs       the timezone aware timestamps, as Pydantic reads them
│   ├── envelope.rs       the sanitized error envelope
│   ├── errors.rs         the error handlers and the failure taxonomy
│   ├── jsoninput.rs      the JSON reader of a request body
│   ├── middleware.rs     the body limit and the security headers
│   ├── patterns.rs       the patterns the request models declare
│   ├── probe.rs          the minimal router the layer is exercised with
│   ├── pytext.rs         Python `str.strip` / `str.isspace` semantics
│   ├── routes.rs         the routes of the API and the port they call
│   ├── schemas.rs        the request and response models
│   ├── server.rs         the reusable connection layer and the hyper server
│   ├── validation.rs     the Pydantic-compatible failure vocabulary
│   ├── views.rs          the validating form of the application views
│   └── wire.rs           the wire form of the answers
├── server                the native server: the routes over the composition root
│   ├── Cargo.toml        standalone workspace, members = []
│   ├── Cargo.lock        that crate's own lock
│   ├── src
│   │   ├── lib.rs        the composition adapter of the routes
│   │   └── main.rs       the `kala-api-server` binary
│   └── tests
│       ├── launch_contract.rs    the launch contract of the binary
│       ├── routes_differential.rs the live differential of every route
│       └── oracle_routes.py      the live Python oracle (test only)
└── tests
    ├── differential.rs           the live Python HTTP differential
    ├── config_differential.rs    the live Python configuration differential
    ├── socket_differential.rs    the live Python differential over a socket
    ├── schemas_differential.rs   the live Python schema differential
    ├── schemas_security.rs       the schema security and edge cases
    ├── security_boundaries.rs    the boundary security tests
    ├── oracle_http.py            the live Python oracle (test only)
    ├── oracle_config.py          the live Python oracle (test only)
    ├── oracle_schemas.py         the live Python oracle (test only)
    └── oracle_socket.py          the live Python oracle (test only)
```

## What it does

Three controls guard every request, and nothing else in this crate reaches the
network, inspects a payload for a business rule, or writes a row.

1. **The loopback Host guard** refuses a request whose `Host` header does not
   name the loopback interface, and refuses a request that declares more than
   one `Host`. A browser resolves the name it sends, so a rebound DNS name
   reaches this process carrying an attacker-chosen header; comparing that
   header against loopback is what stops it.
2. **The per-launch bearer token** is required on every mutating method
   (`POST`, `PUT`, `PATCH`, `DELETE`). It is minted once per launch, stored in
   the local data directory with owner-only permissions, and never logged,
   never returned in a response and never echoed in an error.
3. **The cross-origin guard** refuses every request that carries an `Origin`
   header. This process serves one local operator and no browser, so nothing
   legitimate sends `Origin`, and no CORS header is granted to anyone.

Around them the transport bounds the request body (`262144` bytes by default,
on every method, with or without a declared length) and adds a fixed set of
security headers to every response, including every refusal.

The order of the judgements is the policy:

```
security headers  →  Host guard  →  body limit  →  bearer token  →  origin  →  router
```

The Host guard runs before a body is read, so a rebound name costs nothing.
The size limit runs before the token is judged, so the limit answers
identically for every method and for an unauthenticated caller. The token is
judged before routing, so a mutating method can never reach a handler
unauthenticated.

## The two error layouts

The Python source writes two different bodies for the same envelope, and this
crate reproduces both byte for byte:

| producer | layout | encoder |
| --- | --- | --- |
| a transport refusal | `{"details":[..],"error":{"code":..,"message":..}}` | `json.dumps(..., sort_keys=True)` |
| a framework or routing error | `{"error":{"code":..,"message":..},"details":[..]}` | `json.dumps(..., ensure_ascii=False)` |

The sorted layout is exactly what the accepted `kala-json` crate produces, so
it is emitted through that crate. The ordered layout is *not* canonical JSON
(`kala-json` sorts keys and the Python error handlers do not), so it is
encoded in this crate with the same CPython escaping rules.

### The status of a rejected request

`errors::handle_http_exception` answers a `404`, a `405` and a `413` with
their own codes, a status from `400` up to (but not including) `500` with the
`request_rejected` code and the detail cut to 128 code points, and every
other status with the `internal_error` code and the status the failure
carried.

The Python source used to read `status.HTTP_500_BAD_REQUEST`, a name Starlette
does not define, so for every status of 400 or more that is not 404, 405 or
413 its handler raised an `AttributeError` and the server error middleware
answered a generic 500. The Python source is fixed now, and this port follows
the fixed behaviour: the status of a rejected request is part of the contract
of this API, so a `409` stays a `409` and a `429` stays a `429`.

The detail of a rejected request is answered as it was raised, cut to 128 code
points, and a detail that carries nothing answers with the generic
`the request was rejected`.

## The public surface

```rust
use kala_api::boundary::{issue_api_token, read_api_token, validate_api_token};
use kala_api::config::{settings_from_env, SettingValue};
use kala_api::errors::{handle_application_error, handle_http_exception};
use kala_api::server::{handle_request, serve, ApiState, Router};
```

* `config::settings_from_env` / `config::settings_from_values` build the local
  settings from the seven allowlisted environment variables, or from an
  explicit set of values. The environment is the only source: the Python
  module has no file source either.
* `config::SettingsError` carries the Pydantic error kind, the field and the
  message, so a caller can tell a rejected path from a rejected timeout
  without parsing prose.
* `boundary::Boundary` holds the request boundary of one launch: the expected
  token and the body limit. `boundary::Refusal` is one refusal, with the
  status, the code, the message, the details and the extra headers.
* `server::ApiState` binds a boundary to a `Router`. `server::handle_request`
  is the reusable layer; `server::serve` is the hyper 1 server over
  hyper-util's Tokio adapter, the same HTTP stack the accepted `kala-studio`
  crate uses.
* `probe::ProbeRouter` is the minimal router the layer is exercised with: one
  read, one echo, and the sanitized error envelope for everything else.
* `routes::Routes` is the router of the API: it matches the request, validates
  it and calls the composition root through the `routes::Application` port.
  `routes::Lifespan` closes the root the launch created and leaves a root it
  was given alone.
* `jsoninput::parse` reads a request body the way CPython's `json.loads`
  reads one, and reports the position at which the scanner stopped, which is
  the position the detail line of a rejected body carries.
* `wire::encode_answer` writes an answer the way the live API writes it: the
  JSON serializer of the Python framework is written in Rust, and it writes a
  floating point number in the fixed form whenever the fixed form is the
  shorter one.

## The request and response models

`schemas` is the native form of `kala_agent.api.schemas`: the five request
models, the six response models, the `DomainName` enumeration and every
bound the Python module declares.

* Every model is read with `ApiModel::from_value`, which answers the failures
  the Python source reports: the kind, the location, the message, the input
  and the context. `ApiModel::from_body` reads a request body the way the
  body layer of FastAPI reads it.
* `ApiModel::to_wire_value` and `ApiModel::to_wire_json` write the document
  the Python source writes: the declaration order of the fields, the compact
  separators and the UTF-8 text Starlette writes.
  `ApiModel::to_canonical_json` writes the sorted, ASCII escaped document the
  digests are taken over.
* `schemas::transport_failures` turns the failures of a rejected body into
  the sanitized detail lines the error handlers write, so a rejected body
  answers with the same bytes the Python source answers with.
* `schemas::request_body_limits` are the limits a body is parsed with. The
  Python transport parses a body with `json.loads`, which applies no protocol
  limit: the bounded JSON domain of the tool protocol is applied by the model
  that reads the body, and not by the parser.
* `views` is the native form of the application views the responses carry
  (`DomainPackageView`, `WorkflowDefinitionView`, `DomainEvaluationReport`,
  `CheckpointView`, `WorkflowStateView`, `BudgetUsageView`,
  `RevisionRecord`, `MechanicalRunResult`), with the enums they carry
  (`ApprovalKind`, `ApprovalStatus`, `WorkflowStatus`,
  `PolicyDecisionStatus`). The contract models they embed — `DesignRevision`,
  `Provenance`, `EvaluationResult` and `ApprovalRequest` — are the accepted
  `kala-contracts` types. The module is the *validating* form of the views:
  the DTOs of the composition root are constructed by the root and cannot be
  read back from a document, so they cannot validate one. The routes of this
  crate do not use this module: they are answered with the DTOs of the
  composition root, and the answers carry the same bytes.

The failures of a model are reported in the order the model declares its
fields, and the undeclared fields follow, in the order the value model keeps
them: the keys of an object are sorted by code point.

## The tests

The Python application is a **test oracle only**. The crate never starts an
interpreter, and it has no production dependency on Python.

```
cargo test --locked                        # 124 tests, including the live differential
cargo test --locked --release              # the same 124 tests
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --locked
cargo check --locked --target x86_64-pc-windows-gnu
```

The `server/` crate has its own gates and its own lock:

```
cd server
cargo test --locked                        # 10 tests: the launch contract and the live differential
cargo test --locked --release
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release --locked
```

The live differential drives the **live** Python FastAPI application of
`kala_agent.api` (the real boundary middleware stack and the real error
handlers) with a deterministic corpus, and compares the status, every response
header and the exact body bytes against the native layer. The corpus covers
good, bad and missing bearers, Host and Origin variants (IDNA, ports, IPv6,
case, scope identifiers, wildcards), oversized and chunked bodies, malformed
JSON, the routing errors, the validation details of the error handlers, the
status of every composition root failure, the status and the body of every
rejected request (400, 401, 403, 409, 418, 429, 499, 500, 503, 404, 405, 413,
including the empty, the exact-length, the over-length, the non-ASCII and the
quoted detail), and the configuration precedence and validation messages.

The schema differential drives the **live** Pydantic models of
`kala_agent.api.schemas` with a deterministic corpus of 558 payloads, and
compares the canonical JSON, the wire JSON, every rejection (the kind, the
location, the message, the input and the context) and the body of the 422 the
live error handlers answer.

The route differential of the `server/` crate drives the **live** Python
FastAPI application of `kala_agent.api` and the **native** server with the very
same request script, and compares the status, the headers that matter and the
exact body bytes of every route: the health route, the domain packages, the
evaluations, the revisions, the prepared commits and their decisions, the
mechanical reference workflow and its checkpoints, and the journal. The script
covers the good answers, the validation failures, the conflicts, the unknown
resources, the finality of an approval, the pagination and the request
boundary. Two normalizations are applied to the answers: the wall clock and
the native identity of the domain packages.

The oracle scripts can also be run on their own:

```
python3 tests/oracle_http.py   /tmp/corpus.json /tmp/results.json
python3 tests/oracle_config.py /tmp/corpus.json /tmp/results.json
python3 tests/oracle_schemas.py /tmp/corpus.json /tmp/results.json
python3 tests/oracle_socket.py /tmp/corpus.json /tmp/results.json <native-port>
python3 server/tests/oracle_routes.py /tmp/script.json /tmp/transcript.json /tmp/data-dir
```

Set `KALA_PYTHON` to point the tests at another interpreter. Without one the
differential tests report that they were skipped and pass.
