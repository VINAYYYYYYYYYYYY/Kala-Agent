# api-errors-fix — the latent `HTTP_500_BAD_REQUEST` bug in the API error handler

**Cell:** `api-errors-fix` · **Worker:** Rust worker `api-errors-fix` (Opus
conductor, wave `rust-backend-20261019`) · **Date:** 2026-10-20

## 1. What was fixed

`src/kala_agent/api/errors.py` compared the status of an `HTTPException`
against `status.HTTP_500_BAD_REQUEST`, a name Starlette does not define
(verified: `'HTTP_500_BAD_REQUEST' in dir(starlette.status)` is `False`; the
constant is `HTTP_500_INTERNAL_SERVER_ERROR`). Reading the missing name
raises an `AttributeError` **inside** the handler, so for every status of 400
or more that is not 404, 405 or 413 the handler never answers, Starlette's
server error middleware answers instead, and the caller receives a generic
`500 internal_error` instead of the status it asked for.

The whole fix is one comparison:

```diff
-    elif status.HTTP_400_BAD_REQUEST <= error.status_code < status.HTTP_500_BAD_REQUEST:
+    elif status.HTTP_400_BAD_REQUEST <= error.status_code < status.HTTP_500_INTERNAL_SERVER_ERROR:
```

Nothing else in that file was touched, and **no other `api/*` file was
touched** — `api/__init__.py`, `api/middleware.py`, `api/schemas.py` and the
new `api/envelope.py` still carry the user's uncommitted working-tree changes,
untouched.

### The observable effect of the fix

| raised status | before the fix | after the fix |
| --- | --- | --- |
| 400, 401, 403, 409, 418, 429, 499 | `500 internal_error` | the same status, `request_rejected`, detail cut to 128 code points |
| 404 | `404 not_found` | unchanged |
| 405 | `405 method_not_allowed` | unchanged |
| 413 | `413 payload_too_large` | unchanged |
| 500 | `500 internal_error` | unchanged (byte-identical) |
| 501 … 599 | `500 internal_error` (the status was mangled by the fallback) | the same status, `internal_error` |
| 301 and below | the same status, `internal_error` | unchanged |

The 500 case is byte-identical before and after, which is why the defect hid
so well: the fallback produced the same answer for 500 and a *wrong* answer
for everything else.

## 2. Changed paths

```
src/kala_agent/api/errors.py                              (1 line: the comparison)
tests/unit/test_api_error_statuses.py                     (new: 19 tests)
backend/crates/kala-api/src/errors.rs                     (the 4xx branch + docs + 2 unit tests)
backend/crates/kala-api/tests/oracle_http.py              (the corpus: 9 → 20 framework-error cases)
backend/crates/kala-api/README.md                         (the defect section replaced, the counts updated)
.mece/waves/rust-backend-20261019/cells/api-errors-fix/REPORT.md   (this file)
```

No commit, no push, no delegation, and no credential read. The other
`api/*` files, the other crates, the studio tree and the root manifests were
not touched.

## 3. The Python regression tests

`tests/unit/test_api_error_statuses.py` is new. It builds a minimal FastAPI
application that carries the **real** error handlers
(`install_error_handlers`) and one route that raises the exception, and drives
it with `TestClient(raise_server_exceptions=False)`, so the whole handler path
is exercised: the framework dispatch, the status, the code and the bounded
message. A socket is never opened (an autouse fixture blocks
`socket.socket.connect`).

| group | cases | assertion |
| --- | --- | --- |
| rejected requests | 400, 401, 403, 409, 418, 429, 499 | the status is kept, the code is `request_rejected`, the message is the detail |
| the truncation | 1 | the detail is cut to exactly 128 characters |
| the empty detail | 1 | the message is `the request was rejected` |
| the routing statuses | 404, 405, 413 | their own code and their own message, the detail is not echoed |
| the server statuses | 500, 501, 502, 503, 504 | the status is kept, the code is `internal_error`, the detail is not echoed |
| a status below 400 | 301 | the status is kept, the code is `internal_error` |
| the shape of a server failure | 1 | no traceback, no exception text, an empty `details` |

**19 tests, all passing.**

### The tests fail on the old code

The regression was checked by reverting the one line and re-running the file:
**13 of the 19 tests fail** on the pre-fix source and all 19 pass on the
fixed source. The six that pass on both are the 404/405/413 cases, the 500
case and the 301 case — the branches the defect did not touch.

## 4. The Rust port now follows the fixed behaviour

`backend/crates/kala-api/src/errors.rs`:

* `handle_http_exception` answers a status from 400 up to (but not including)
  500 with the `request_rejected` code and the detail cut to 128 code points,
  and a detail that carries nothing with the generic
  `the request was rejected` — exactly the fixed Python behaviour. The
  previous `code if code >= 400 => handle_unexpected_error()` arm (the
  deliberate reproduction of the defect) is gone.
* The new public constant `MAX_REJECTED_DETAIL_LENGTH` (128) names the
  truncation the Python source writes as `str(error.detail)[:128]`.
* The doc comment at the function no longer claims to reproduce a defect; it
  describes the contract.
* The unit test `http_exceptions_map_onto_the_python_codes` now asserts the
  seven rejected statuses, and two new unit tests pin the truncation
  boundary and the empty-detail fallback.

`backend/crates/kala-api/tests/oracle_http.py`: the framework-error corpus grew
from 9 to 20 cases — 401, 403, 409, 429 and 499 added, a 503 added, and the
detail corpus widened with the exact-length (128), the one-over-length (129),
the non-ASCII (`café refusé`), the over-length non-ASCII (`é` × 129) and the
quoted-and-escaped detail. The corpus is what makes the differential
byte-exact rather than status-exact.

`backend/crates/kala-api/README.md`: the "One deliberate reproduction of a
defect" section is replaced by "The status of a rejected request", the test
count is updated (56 → 58) and the corpus description names the new cases.

### The Rust port is the fixed behaviour, and the differential proves it

The live differential was run against the **fixed** Python. It reports
**0 mismatches**:

| differential | cases | mismatches |
| --- | --- | --- |
| HTTP connection layer (ASGI level, `oracle_http.py`) | **393** | **0** |
| validation details of the error handlers | **17** | **0** |
| status of every composition root failure | **17** | **0** |
| framework errors of the error handlers | **20** | **0** |
| real loopback socket (uvicorn vs hyper, `oracle_socket.py`) | **31** | **0** |
| configuration precedence and validation messages | **113** | **0** |
| **total live differential** | **591** | **0** |

**The differential is sensitive.** Patching the Rust function back to the
pre-fix arm and re-running it against the *fixed* Python produces **15
mismatches** (one for every 4xx case and for the 503), so the green result is
a real comparison and not a vacuous one.

The live Python answers the new corpus like this (extracted from the oracle's
own output):

```
http.client          status=400 body={"error":{"code":"request_rejected","message":"the request was rejected"},"details":[]}
http.client.empty    status=400 body={"error":{"code":"request_rejected","message":"the request was rejected"},"details":[]}
http.client.long     status=400 body={"error":{"code":"request_rejected","message":"ddd…(128)"},"details":[]}
http.client.exact    status=400 body={"error":{"code":"request_rejected","message":"eee…(128)"},"details":[]}
http.client.oneOver  status=400 body={"error":{"code":"request_rejected","message":"eee…(128)"},"details":[]}
http.client.unicode  status=400 body={"error":{"code":"request_rejected","message":"café refusé"},"details":[]}
http.client.quoted   status=400 body={"error":{"code":"request_rejected","message":"a \"quoted\" \\ detail"},"details":[]}
http.unauthorized    status=401 body={"error":{"code":"request_rejected","message":"a valid token is required"},"details":[]}
http.forbidden       status=403 body={"error":{"code":"request_rejected","message":"the request is refused"},"details":[]}
http.conflict        status=409 body={"error":{"code":"request_rejected","message":"the resource is in conflict"},"details":[]}
http.tooMany         status=429 body={"error":{"code":"request_rejected","message":"too many requests"},"details":[]}
http.clientClosed    status=499 body={"error":{"code":"request_rejected","message":"the client closed the request"},"details":[]}
http.server          status=500 body={"error":{"code":"internal_error","message":"the request could not be completed"},"details":[]}
http.notfound        status=404 body={"error":{"code":"not_found",…},"details":[]}
http.notallowed      status=405 body={"error":{"code":"method_not_allowed",…},"details":[]}
http.tooLarge        status=413 body={"error":{"code":"payload_too_large",…},"details":[]}
```

## 5. The gates

### Python

| Gate | Command | Result |
| --- | --- | --- |
| unit tests | `pytest tests/unit -q` | **1552 passed**, 0 failed |
| security tests | `pytest tests/security -q` | **189 passed**, 0 failed |
| the new file | `pytest tests/unit/test_api_error_statuses.py -q` | **19 passed**, 0 failed |
| lint | `ruff check` on both changed files | clean |
| format | `ruff format --check` on both changed files | clean |

`mypy` could not be run as a gate: the interpreter's `numpy 2.5.4` stubs use
the `type` statement, which the repository's `python_version = "3.11"` setting
makes mypy 2.3.1 reject, so it aborts inside
`.venv/lib/python3.14/site-packages/numpy/__init__.pyi` before it checks
anything. This is **pre-existing and environmental**: the same failure happens
on the untouched `tests/unit/test_api.py` and `tests/unit/test_studio_errors.py`.
`mypy src/kala_agent/api/errors.py` alone passes.

### Rust

| Gate | Command | Result |
| --- | --- | --- |
| format | `cargo fmt --check` | clean |
| lint | `cargo clippy --all-targets --all-features -- -D warnings` | clean, no warnings |
| tests (debug) | `cargo test --locked` | **58 passed**, 0 failed |
| tests (release) | `cargo test --locked --release` | **58 passed**, 0 failed |
| release build | `cargo build --release --locked` | success |
| Windows compile | `cargo check --locked --target x86_64-pc-windows-gnu` | success (the target **is** installed) |

Rust test breakdown: 39 unit tests in `src/`, 2 in `tests/differential.rs`, 2
in `tests/config_differential.rs`, 14 in `tests/security_boundaries.rs`, 1 in
`tests/socket_differential.rs`, 0 doc tests — **58**.

Toolchain: `cargo 1.99.1`, `rustc 1.99.1`. The Windows target **compiles**
here; nothing was executed on Windows, and no Windows runtime claim is made.

## 6. Compatibility and security evidence

* **The status is now part of the contract.** A caller that asked for a 409
  gets a 409 with `request_rejected`; a caller that asked for a 429 gets a 429.
  Before the fix both got a 500, which told the caller nothing about why the
  request was refused.
* **The 4xx detail is echoed, cut to 128 code points.** This is the
  pre-existing, intended behaviour of `kala_agent.api.errors` and is unchanged
  by this cell — the `request_rejected` branch was simply dead code before.
  The exposure is latent, not live: no route of `kala_agent.api` raises an
  `HTTPException` today, and the framework's own 404, 405 and 413 take their
  own branches, so nothing reaches the echo yet. Note that the **studio**
  module (`kala_agent.studio.errors`) applies a stricter rule (`_safe_detail`
  drops a credential-shaped or over-long detail); the API module does not and
  did not. This cell did not change that policy; if the API module should
  refuse credential-shaped details too, that is a separate, larger change to
  the same handler, and the Rust port would have to follow it.
* **A server failure still echoes nothing.** 500 and above answer
  `internal_error` with the fixed message `the request could not be
  completed` and an empty `details`, and the raised detail never reaches the
  wire. The new test asserts the absence of a traceback and of the detail.
* **The 404, 405 and 413 answers are untouched**, including their own messages
  and the `payload_too_large` size-limit message.
* **The transport refusal path is untouched.** The boundary gates (Host,
  bearer token, origin, body limit) and the sorted refusal layout are not
  affected by this change; `tests/security_boundaries.rs` (14 tests) and the
  189 Python security tests still pass.
* **The response envelope is unchanged**: `{"error":{"code":…,"message":…},"details":[]}`
  with CPython's `ensure_ascii=False` escaping, reproduced byte for byte by
  the native layer (the corpus now pins the non-ASCII and the quoted detail
  on the wire, not just in the JSON model).

## 7. Intentional differences

1. **The Rust `handle_http_exception` takes the detail already rendered as a
   string.** The Python source writes `str(error.detail)[:128]`, so a detail
   that is not a string (`None` → `"None"`, `123` → `"123"`) is stringified
   by the source. The Rust signature takes `&str`, so that conversion is the
   caller's job. This gap is pre-existing and is not exercised by the live
   application: no route of `kala_agent.api` raises an `HTTPException`, and
   the framework's own 404 and 405 details are strings on the branches that
   use them.
2. **A status above 599 or a status that is not a valid HTTP status is
   answered as it was raised** (the `internal_error` code with that status),
   exactly as the fixed Python source does. The Rust port does not add a
   validity check the Python source does not have.
3. **The 4xx branch of the Python source is now reachable, so the `request_rejected`
   code and the `REQUEST_REJECTED_MESSAGE` constant are live** — they were
   exported but unreachable before.

## 8. Remaining gaps

* **The `api-core-r1` report is now stale in three places.** Its §2.7
  ("a latent defect in the Python `errors.py`, reproduced deliberately"), its
  intentional difference 13 ("the `request_rejected` branch is unreachable") and
  its last remaining gap describe the old behaviour. That cell's report was
  not edited here — it belongs to another cell — so this report supersedes
  those three statements. The `backend/crates/kala-api/README.md` **was**
  updated, because it is inside this cell's ownership.
* **The API module still echoes the detail of a 4xx.** The studio module does
  not. See §6; this cell did not change the policy.
* **The `mypy` gate is blocked by the environment** (the numpy stubs against
  `python_version = "3.11"`), independently of this change.
* **No Windows runtime was exercised**, no cutover was performed, and no
  speed, readiness or production claim is made. The Python reference remains
  in place; the Rust crate is not wired into any running service.
* **The routes, the dependencies and the composition root are not here.** They
  belong to later cells. This cell changed one comparison, added the
  regression tests, and made the native port follow the fixed behaviour.
