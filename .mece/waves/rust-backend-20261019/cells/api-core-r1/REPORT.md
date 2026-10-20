# api-core-r1 — the Kala core API connection layer (config, boundary gates, error envelope, middleware)

**Cell:** `api-core-r1` · **Worker:** Rust worker `api-core-r1` (Opus conductor,
wave `rust-backend-20261019`) · **Date:** 2026-10-20

## 1. What was built

A new standalone crate `backend/crates/kala-api` that ports the core API's
connection layer from the repository's Python source:

| Python module | Rust module | public surface |
| --- | --- | --- |
| `src/kala_agent/config.py` | `src/config.rs` | `Settings`, `SettingValue`, `SettingsError`, `settings_from_env`, `settings_from_values`, `ENVIRONMENT_FIELDS`, `error_kind::*` |
| `src/kala_agent/api/boundary.py` | `src/boundary.rs` | `Boundary`, `Refusal`, `is_loopback_host`, `split_authority`, `is_permitted_authority`, `validate_api_token`, `issue_api_token`, `read_api_token`, `store_api_token`, `mint_api_token`, `presented_credential`, `token_matches`, `is_mutating_method`, and the module constants |
| `src/kala_agent/api/envelope.py` | `src/envelope.rs` | `ErrorDetail`, `ErrorResponse`, `error_response`, `truncate_chars`, `MAX_ERROR_DETAILS`, `MAX_ERROR_MESSAGE_LENGTH` |
| `src/kala_agent/api/middleware.py` | `src/middleware.rs` | `validate_max_body_bytes`, `refusal_body`, `header_values`, `header_value`, `raw_headers`, `decode_header_value`, `apply_security_headers`, `SECURITY_HEADERS`, `STRIPPED_RESPONSE_HEADERS`, `MAX_REQUEST_BODY_BYTES`, `MIN_REQUEST_BODY_BYTES`, `MAX_HEADER_VALUE_LENGTH` |
| `src/kala_agent/api/errors.py` | `src/errors.rs` | `ApplicationError`, `ApplicationErrorKind`, `ValidationFailure`, `handle_request_validation`, `handle_application_error`, `handle_http_exception`, `handle_unexpected_error`, `validation_details`, and the message constants |
| `src/kala_agent/api/__init__.py` (the parts they rely on) | `src/lib.rs`, `src/server.rs`, `src/probe.rs` | `ApiState`, `Router`, `handle_request`, `serve`, `refusal_response`, `error_response`, `internal_error_response`, `ProbeRouter` |
| — (support) | `src/pytext.rs` | Python `str.strip` / `str.isspace` / `str.lower` semantics |

The crate is a **standalone workspace** (`[workspace] members = []`) with its
own `Cargo.lock`. It consumes the accepted `kala-json` crate read-only as a
path dependency. It adds **no second HTTP framework**: it uses the same stack
as the accepted `kala-studio` crate — hyper 1 with `http1`/`server`, hyper-util
0.1 with `http1`/`server`/`server-graceful`/`tokio`, and a Tokio reactor.

### Changed paths

Only new files inside this crate, plus this report:

```
backend/crates/kala-api/Cargo.toml
backend/crates/kala-api/Cargo.lock
backend/crates/kala-api/README.md
backend/crates/kala-api/src/lib.rs
backend/crates/kala-api/src/boundary.rs
backend/crates/kala-api/src/config.rs
backend/crates/kala-api/src/envelope.rs
backend/crates/kala-api/src/errors.rs
backend/crates/kala-api/src/middleware.rs
backend/crates/kala-api/src/probe.rs
backend/crates/kala-api/src/pytext.rs
backend/crates/kala-api/src/server.rs
backend/crates/kala-api/tests/config_differential.rs
backend/crates/kala-api/tests/differential.rs
backend/crates/kala-api/tests/oracle_config.py
backend/crates/kala-api/tests/oracle_http.py
backend/crates/kala-api/tests/oracle_socket.py
backend/crates/kala-api/tests/security_boundaries.rs
backend/crates/kala-api/tests/socket_differential.rs
.mece/waves/rust-backend-20261019/cells/api-core-r1/REPORT.md
```

**No Python file was created, edited or deleted.** `src/kala_agent/api/*`
still carries the user's uncommitted working-tree changes (`api/__init__.py`,
`api/middleware.py`, `api/schemas.py`, and the new `api/envelope.py`); the
port reads the current working tree exactly. No commit, no push, no
delegation, no credential read, and no edit outside this crate and this
report.

## 2. Fidelity notes that mattered

### 2.1 The two error layouts are different byte sequences

The Python source writes the same envelope in two different orders:

* a **transport refusal** (`middleware.send_refusal` →
  `json.dumps(payload, separators=(",", ":"), sort_keys=True)`) is
  `{"details":[...],"error":{"code":...,"message":...}}`;
* a **framework or routing error** (`errors._json` → Starlette's
  `JSONResponse`, which is `json.dumps(..., ensure_ascii=False,
  separators=(",", ":"))`) is
  `{"error":{"code":...,"message":...},"details":[...]}`.

The two differ in key order **and** in escaping: the refusal path escapes
non-ASCII as `\uXXXX`, the error-handler path writes it verbatim. Both are
reproduced byte for byte. The sorted layout is exactly what the accepted
`kala-json` crate emits, so it is produced through that crate; the ordered
layout is not canonical JSON (`kala-json` sorts keys), so it is encoded in
this crate with CPython's own escaping rules.

### 2.2 CPython 3.14's `ipaddress` is stricter than Rust's `IpAddr`

`boundary.is_loopback_host` calls `ipaddress.ip_address`. On the interpreter
of this repository (CPython 3.14.8):

* the short IPv4 forms (`127.1`, `127.0.1`) and leading zeros (`0177.0.0.1`)
  are **rejected** — Python 3.14 requires exactly four octets with no leading
  zero. Rust's `Ipv4Addr::from_str` has the same strictness, so the two agree;
* an IPv6 address may carry a **scope identifier** (`::1%eth0`), which Rust's
  `Ipv6Addr::from_str` rejects. The port therefore strips and validates a
  scope suffix the way CPython's `_split_scope_id` does;
* `IPv6Address.is_loopback` and `is_unspecified` **delegate to the mapped IPv4
  address** when the address is IPv4-mapped, so `::ffff:127.0.0.1` *is*
  loopback and `::ffff:0.0.0.0` *is* unspecified. Rust's `Ipv6Addr` has no
  such rule, so the mapping is reproduced explicitly.

Each of these was confirmed against the live interpreter before it was
ported; the corpus contains `::ffff:127.0.0.1`, `[::1%eth0]:80`, `::1%eth0`,
`127.1.2.3` and the wildcard forms.

### 2.3 Python's `str.strip` is not Rust's `trim`

`str.strip()` treats `U+001C..U+001F` as whitespace; Rust's `trim()` uses the
Unicode `White_Space` property, which does not. The boundary trims the
presented credential and the configured provider name, so `src/pytext.rs`
implements CPython's set. (Pydantic's own `strip_whitespace` uses the Rust
property; the port keeps the two apart, exactly as `kala-studio` does.)

### 2.4 `model_endpoint` uses the same URL parser pydantic-core uses

Pydantic 2.13 validates `AnyHttpUrl` with the WHATWG `url` crate. This crate
depends on the same `url` crate, so the parsing, the IDNA normalization, the
path percent-encoding, the default-port removal and the parse-error wording
(`empty host`, `invalid international domain name`, `invalid port number`,
`invalid IPv4 address`, `invalid IPv6 address`, `relative URL without a base`)
match. The credential, query and fragment checks reproduce Pydantic's
falsy-empty semantics: `http://example.com#` and `http://example.com/?` are
accepted, because an empty query or fragment is falsy in Python.

### 2.5 The configuration reports Pydantic's own error kinds

`config::SettingsError` carries the Pydantic error `type` (`value_error`,
`string_too_short`, `string_too_long`, `greater_than_equal`,
`less_than_equal`, `url_parsing`, `url_scheme`, `url_type`,
`extra_forbidden`, `string_type`), the field, and the message in Pydantic's
wording. The failures are reported in the order the Python model declares its
fields, with the extra keys last, exactly as Pydantic collects them.

### 2.6 The judgement order is the policy

The Python stack is `SecurityHeaders → HostGuard → MaxBodySize →
BearerToken → CrossOriginGuard → router`. The native layer runs the same
judgements in the same order, and the corpus contains cases that pin the
order down (`order.host.before.token`, `order.body.before.token`,
`order.token.before.origin`, …).

### 2.7 A latent defect in the Python `errors.py`, reproduced deliberately

`src/kala_agent/api/errors.py` line 146 reads

```python
elif status.HTTP_400_BAD_REQUEST <= error.status_code < status.HTTP_500_BAD_REQUEST:
```

**Starlette has no `HTTP_500_BAD_REQUEST`** — the constant is
`HTTP_500_INTERNAL_SERVER_ERROR`. Reading the missing name raises an
`AttributeError` *inside* the handler, so for **every status of 400 or more
that is not 404, 405 or 413** the handler never answers, the server error
middleware answers instead, and the client receives:

```
500 {"error":{"code":"internal_error","message":"the request could not be completed"},"details":[]}
```

The intended `request_rejected` branch — the one that would echo the
framework detail, cut to 128 code points — is therefore **dead code in the
live application**. A status below 400 (for example `301`) does not raise,
because the chained comparison short-circuits before the missing name is
read, and answers `internal_error` with its own status.

Reproduced from the live interpreter:

```
400 -> RAISED AttributeError module 'starlette.status' has no attribute 'HTTP_500_BAD_REQUEST'
418 -> RAISED AttributeError ...
500 -> RAISED AttributeError ...
501 -> RAISED AttributeError ...
404 -> 404 {"error":{"code":"not_found",...}}
405 -> 405 {"error":{"code":"method_not_allowed",...}}
413 -> 413 {"error":{"code":"payload_too_large",...}}
301 -> 301 {"error":{"code":"internal_error",...}}
```

**This port reproduces the observable behaviour exactly** — the native
`errors::handle_http_exception` answers the sanitized 500 for every status of
400 or more that is not 404, 405 or 413 — because the live Python application
is the specification of this cell and the brief asks for zero unexplained
mismatches. The nine-case live differential pins the behaviour down on both
sides.

**This is a defect in the Python source, not in the port, and this cell is not
allowed to edit that file** (`src/kala_agent/api/errors.py` is the user's
source). The conductor may want to fix the Python source; the fix would be
`status.HTTP_500_INTERNAL_SERVER_ERROR`, and the Rust side would then have to
follow. The Rust comment in `errors::handle_http_exception` says so at the
place where it matters.

## 3. Tests

### 3.1 Exact counts

| Suite | Tests | Result |
| --- | --- | --- |
| unit tests (`src/`) | 37 | all pass |
| `tests/differential.rs` | 2 | all pass |
| `tests/config_differential.rs` | 2 | all pass |
| `tests/security_boundaries.rs` | 14 | all pass |
| `tests/socket_differential.rs` | 1 | all pass |
| doc tests | 0 | — |
| **total** | **56** | **56 pass, 0 fail** |

The same 56 tests pass in the debug and the release profile.

### 3.2 The live Python differential

Python is a **test oracle only**. The crate has no production dependency on
an interpreter, and none of the oracle scripts is reachable from the library.

| Suite | Cases | Mismatches |
| --- | --- | --- |
| HTTP connection layer (ASGI level, `oracle_http.py`) | **393** | **0** |
| Validation details of the error handlers | **17** | **0** |
| Status of every composition root failure | **17** | **0** |
| Framework errors of the error handlers | **9** | **0** |
| Real loopback socket (uvicorn vs hyper, `oracle_socket.py`) | **31** | **0** |
| Configuration precedence and validation messages | **113** | **0** |
| **total live differential** | **580** | **0** |

Every case compares the **status**, **every response header** and the **exact
body bytes**; the configuration suite compares every field value and every
`(kind, field, message)` failure triple.

What the corpus covers:

* **bearer**: absent, empty, `Bearer`, `Bearer `, `Basic …`, a wrong token, a
  token with one character too many or too few, a non-ASCII token, a token
  with an inner space, two `Authorization` headers, the scheme in lower, upper
  and mixed case, one and two spaces after the scheme, a trailing tab;
* **Host**: the permitted forms (`127.0.0.1`, `127.0.0.1:8000`, `localhost`,
  `LOCALHOST`, `LocalHost:80`, `[::1]`, `[::1]:8000`, `127.0.0.2`,
  `127.0.0.255`, `[::ffff:127.0.0.1]:80`, `[::1%eth0]:80`, a zero-padded
  port, `127.1.2.3`) and the refused ones (a rebound name, a userinfo
  section, a path, a trailing dot, `0.0.0.0`, `::`, `[::]`, `[::0]`,
  `0:0:0:0:0:0:0:0`, `*`, a link-local address, a private address, port `0`,
  port `65536`, port `port`, an unclosed bracket, an empty authority, a
  leading and a trailing space, an IDNA/punycode name, a non-breaking space,
  an over-long authority, two `Host` headers, no `Host` header at all);
* **Origin**: absent, empty, `null`, a loopback origin, a private origin, an
  IDNA origin, a comma-separated origin, two `Origin` headers;
* **bodies**: over the limit on every method, over the limit chunked without
  a declared length, exactly at the limit, a declared length over the limit, a
  declared length that is not a number, a negative declared length, and a
  declared length that lies about a body that is over the limit;
* **payloads**: malformed JSON (an unclosed object, plain text, trailing
  data, a trailing comma), an array, a number, `NaN`, `1e400`, invalid UTF-8,
  a deeply nested document, whitespace only, an empty body;
* **routing**: an unknown path, an unknown path with a mutating method, and
  every unsupported method on the probe path;
* **framework errors**: a `400`, an empty `400` detail, a `400` detail longer
  than 128 code points, a `418`, a `500`, a `501`, a `404`, a `405`, a `413`
  and an unexpected exception — these pin the behaviour described in §2.7;
* **ordering**: the six combinations that pin the order of the judgements.

The randomized part of the corpus is deterministic: a fixed seed
(`20261019`) over 240 cases drawn from the host, origin, credential, method,
path and body-size pools.

### 3.3 How the differential is driven, and why

The HTTP differential drives the **live** Python FastAPI application of
`kala_agent.api` — the real `build_middleware_stack` middleware stack and the
real `install_error_handlers` handlers — at the **ASGI boundary**, with a
hand-built scope. That is what makes the comparison exact: a duplicated
`Host`, an empty `Origin`, a chunked body without a declared length and a
body that lies about its length all reach both applications unchanged, with
no HTTP parser in between. The HTTP/1.1 framing layer is not part of this
cell (the routes and the composition are later cells), and the two parsers
genuinely differ: uvicorn's `httptools` rejects a duplicated `Host` and an
absurd `Content-Length` with a `400` before the application ever sees them.

The socket differential covers that gap for well-formed requests: it starts
the live Python application on a loopback port with **uvicorn** and the native
server on another loopback port, and replays **31 raw HTTP/1.1 requests**
against both. The status, the body bytes and the application headers are
compared; the framing headers the two servers add on their own (`date`,
`server`, `connection`) are excluded, because they belong to the servers and
not to the application. 31 cases, 0 mismatches.

The configuration differential builds the corpus in Python and replays it in
Rust over the same paths, including a temporary directory, a symlink, a
parent traversal, a file instead of a directory, a NUL byte, `~` and `~/…`.

### 3.4 Security evidence

`tests/security_boundaries.rs` is the Rust counterpart of
`tests/security/test_api_boundary.py` and of the boundary cases in
`tests/unit/test_api.py`. 14 tests, all passing:

* a rebound `Host` never reaches a route — 17 refused authorities, each
  answered `421 invalid_host` with the sanitized body;
* two `Host` headers are refused; a request without a `Host` is refused;
* every mutating method needs the launch token; the refusal carries
  `www-authenticate: Bearer realm="kala-agent"`, and neither the expected
  token nor the submitted secret appears in the body;
* `DELETE`, `PATCH`, `POST` and `PUT` are the mutating methods, and no other
  method is judged as one;
* the body limit answers **every** method, including the ones that carry no
  payload, and carries the `nosniff` header;
* a wildcard address is never loopback, in any of its spellings;
* a cross-origin request is refused with `403 cross_origin_denied` and **no**
  `access-control-*` header is ever granted;
* every response carries the full security header set, and neither `server`
  nor `x-powered-by` is present;
* an error never echoes a submitted value, a credential-shaped value or a
  traceback;
* the launch token is stored in the local data directory, read back, rotated
  on every launch, and **never** returned in a response;
* the stored token file is owner-only (`0600`) on Unix;
* a missing, over-long, non-UTF-8 or malformed token store reads as absent;
* a token that cannot be stored (the path is a directory) still guards the
  process: the mutating request is still `401`.

Implementation facts behind those tests: the token comparison is
constant-time over equal-length byte strings; the token file is opened with
`O_NOFOLLOW` so a planted symlink cannot redirect the write, and its mode is
forced with `fchmod` after the open; the token is only ever written to that
file and is never logged, echoed or returned; and the refusal bodies are
built from the envelope, which carries nothing but a code, a bounded message
and bounded details.

### 3.5 The Rust gates

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --check` | clean |
| lint | `cargo clippy --all-targets --all-features -- -D warnings` | clean, no warnings |
| tests (debug) | `cargo test --locked` | 56 passed, 0 failed |
| tests (release) | `cargo test --locked --release` | 56 passed, 0 failed |
| release build | `cargo build --release --locked` | success |
| Windows compile | `cargo check --locked --target x86_64-pc-windows-gnu` | success (the target is installed) |

Toolchain: `cargo 1.99.1`, `rustc 1.99.1`. The Windows target **compiles**
here; nothing was executed on Windows, and no Windows runtime claim is made.

## 4. Intentional differences

Every one of these is deliberate and none of them is a silent behaviour
change on a path the Python source exercises.

1. **The Python `bool` branch of `validate_max_body_bytes` is unreachable in
   Rust.** Python counts `bool` as an `int`, so the source refuses a boolean
   with `"max_body_bytes must be an integer"`. The Rust signature takes an
   `i64`, so the type system removes the mistake; the range message
   (`"max_body_bytes must be between 1024 and 262144"`) is reproduced exactly.
2. **`issue_api_token` returns a `Result`.** `secrets.token_urlsafe` raises
   when the entropy source fails, and the Python source propagates that
   exception, so the Rust port propagates it as an error instead of minting a
   token that is not random. The "the directory is unusable" path is
   unchanged: the token still guards the process in memory and the failure is
   reported by kind.
3. **`Path.expanduser()` raising `RuntimeError` becomes a validation
   error.** When the home directory cannot be determined and the path starts
   with `~`, CPython raises a `RuntimeError` out of the validator, which is
   not a `ValidationError`. The Rust port reports
   `"data_dir cannot be resolved"` instead.
4. **The token store on Windows.** The Python source calls `os.fchmod`, which
   does not exist on Windows, so the Python launch fails there. The Rust port
   skips the POSIX mode on Windows (where the mode is not meaningful) and
   keeps the `O_NOFOLLOW` open where the platform has it.
5. **The probe router does not implement Starlette's `redirect_slashes`.**
   `GET /probe/` is a `307` in the live application and a `404` in the probe
   router. Routing is a later cell; the connection layer does not route. The
   randomized corpus therefore stays free of trailing-slash paths, and this
   is called out rather than hidden.
6. **FastAPI does not add `HEAD` to a `GET` route** (Starlette's plain
   `Route` does, `APIRoute` does not). The probe router matches FastAPI:
   `HEAD /probe` is `405 method_not_allowed`. The token gate itself is
   unaffected: `HEAD` is not a mutating method.
7. **The two servers differ in their framing.** uvicorn adds `date` and
   `server`; hyper does not. The security header set removes `server` from the
   *application* response, but a server that adds it below the application
   keeps it on the wire. This is why the socket differential compares the
   application's headers and excludes the servers' own framing headers.
8. **`Settings` is a plain immutable struct in Rust.** The Python model is
   `frozen=True` and `extra="forbid"`; the Rust port has no mutating method
   and refuses an unknown field with the `extra_forbidden` error, but the
   immutability is by construction rather than by the type system.
9. **`install_error_handlers` has no Rust counterpart as a registration
   call.** The Python source registers the handlers on a FastAPI application.
   The Rust port exposes the same decisions as functions
   (`handle_request_validation`, `handle_application_error`,
   `handle_http_exception`, `handle_unexpected_error`); the transport calls
   them, and a later cell wires them to its own router.
10. **`ApplicationError` is an enumeration, not a class hierarchy.** The
    Python source raises a class per failure and the error handlers map the
    class onto a status with `isinstance`. The Rust port carries the same
    taxonomy, the same codes and the same statuses in
    `errors::ApplicationErrorKind`; the 17-case differential compares every
    one of them against the live `_status_for`.
11. **`ValidationFailure` carries the location as strings.** The Python
    source joins the parts of `loc` with `str(part)`, so an integer part
    becomes its decimal form. The Rust type takes the parts already
    rendered, which is the same text.
12. **The configuration has no file source.** Neither does the Python
    module: `Settings.from_env` reads the seven allowlisted variables and
    leaves everything else, including every secret, untouched. The Rust port
    reads the same seven variables and retains nothing else.
13. **The `request_rejected` branch is unreachable, as in the Python
    source.** See §2.7: the Python handler raises before it can answer, and
    the Rust port reproduces that observable behaviour rather than the
    intended one. The `request_rejected` code and the
    `REQUEST_REJECTED_MESSAGE` constant are still exported, so a fixed
    Python source has an obvious counterpart.

## 5. Remaining gaps

* **The routes, the dependencies and the application composition are not
  here.** `api/app.py`, `api/dependencies.py` and `api/schemas.py` belong to
  the later cells named in the brief. This crate exposes the gates and the
  middleware as a reusable layer plus a minimal test router, which is what
  the brief asked for.
* **The composition root's own failures are represented, not raised.** The
  taxonomy and its statuses are ported and differentially checked, but the
  Rust crate cannot raise a Python `ApplicationError`, so the mapping is
  checked against the live `_status_for` rather than through a live failure.
* **The HTTP/1.1 framing layer is not compared exhaustively.** The
  ASGI-level differential is exact; the socket differential covers 31
  well-formed requests. Requests that a parser rejects before the
  application (a duplicated `Host`, an absurd `Content-Length`) are answered
  differently by the two parsers — by the parser, not by this crate.
* **The Pydantic request schemas are not ported.** The validation-detail
  formatting of `errors.py` is ported and differentially checked against 17
  real Pydantic failures, but the schemas themselves are a later cell.
* **No Windows runtime was exercised.** The crate compiles for
  `x86_64-pc-windows-gnu`; the token store, the file modes and the server
  were not run there.
* **No speed, readiness or cutover claim is made.** This crate is not wired
  into any running service, and the Python reference remains in place and
  untouched.
* **The `request_rejected` branch of the framework-error handler is dead in
  the live application** (§2.7). The Rust port reproduces that, so a fix to
  the Python source has to be mirrored here. This cell cannot make that fix:
  `src/kala_agent/api/errors.py` is the user's source.
