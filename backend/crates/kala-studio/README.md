# kala-studio

Native Kala Studio backend. This crate is the Studio **front door**: a
real Axum-free Hyper/Tokio binary and library that owns the HTTP boundary,
the settings and token stores, the static UI serving, the deterministic local
chat provider and the fixed-endpoint remote OpenAI-compatible provider.

It is the seventh member of the root `backend/` workspace; `backend/Cargo.lock`
is authoritative.

## The geometry front door (ADR-0010 phase 2, ADR-0011)

Geometry is never re-implemented in Rust. The front door calls the **real
OpenCascade** through the narrow C++ adapter of `kala-cad`, and it owns the
design pipeline, the session journal and the CAD provider connections.

The default build links:

| Crate | What it owns |
|---|---|
| `kala-cad` | the OpenCascade adapter: compile, measure, tessellate, STEP write and read, hidden-line drawings and exact-area sections |
| `kala-cad-providers` | the CAD provider connections: `FreeCADCmd` out of process, the Solid Edge loopback bridge |
| `kala-design-intent` | the `DesignIntent` contract, the part templates and the strict prompt parser |
| `kala-studio-journal` | the hash-chained session journal and the content-addressed artifact store (byte-compatible with the Python journal) |
| `kala-contracts` | the typed construction script and its canonical form |
| `kala-json`, `kala-persistence` | the canonical JSON and the lossless text rules |

**The Python geometry worker is the development fallback.** It is started
*only* when `KALA_STUDIO_GEOMETRY_WORKER` names one, so a default launch has
no Python process anywhere in the picture.

* The worker command comes from `KALA_STUDIO_GEOMETRY_WORKER`. The value is
  either a JSON argv array (`["/path/python", "-m", "kala_agent.studio",
  "--worker"]`) or a single executable path, in which case the fixed module
  arguments `-m kala_agent.studio --worker` are appended. Whitespace
  splitting is **not** performed: a value that names an executable *and*
  arguments is refused rather than silently re-parsed.
* The bearer token is 32 fresh random bytes rendered as 64 hexadecimal
  characters. It is passed **only** through `KALA_WORKER_TOKEN`, never in
  argv, and it is never logged.
* `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops when the front
  door closes its stdin: a dead front door can never orphan a worker.
* The worker is restarted with a capped backoff (250 ms to 5 s) and its state
  is published in `/health` as `geometry_worker {state, version,
  design_available}`.
* A worker that cannot start leaves the native routes working and the geometry
  routes answering 503 `geometry_worker_unavailable`.
* `KALA_STEP_IMPORT_MAX_BYTES` carries the **resolved** STEP import ceiling
  into the worker process, so the front door and the worker accept exactly
  the same body (see below).

The `GeometryBackend` trait (`src/geometry_backend.rs`) is the seam: one
async method per geometry route (preview, export, handoff, design, drawing,
section, import, intent_parse, intent_validate, intent_schema, journal_page,
journal_verify, journal_append, providers_cad). `NativeGeometryBackend`
(`src/native_geometry/`) is the production implementation; it answers with the
same bytes the Python worker answered with, and the front door forwards the
request body and the original content type unchanged.

## Run

Build from the root workspace or through the desktop staging helper:

```bash
cd backend
cargo build --release --locked -p kala-studio
./target/release/kala-studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist ../studio/dist

cd ../studio
npm run build:native
backend-native/kala-studio --host 127.0.0.1 --port 8765
```

The staging helper copies `studio/dist` beside the binary as `studio_dist/`,
which is the layout used by the packaged Electron app. See
`docs/rust-studio.md` for the desktop resolver and packaging contract.

`KALA_STUDIO_TOKEN` supplies the bearer token (no token file is written when
it is set); otherwise the process mints a `studio-token` file with owner-only
mode where supported. `KALA_STUDIO_DIST` can override the static UI root.
The process prints one sorted JSON launch banner before binding and exits 0
on SIGTERM/Ctrl-C, 1 on startup failure, and 2 on usage errors.

## Routes

All routes live under `/api/studio`.

**Rust-native:** `GET health`, `GET integrations`, `GET providers`,
`GET settings`, `POST settings`, `POST chat`.

**Served by the native geometry backend:** `POST preview`, `POST export`,
`POST handoff`, `POST design`, `GET design/{id}/drawing`,
`GET design/{id}/section`, `GET journal`, `GET journal/verify`,
`GET providers/cad`, `POST import`. The same routes are served by the
supervised Python worker when `KALA_STUDIO_GEOMETRY_WORKER` names one.

The middleware order is the security policy and matches the Python source:
security headers, loopback Host guard (421), declared/streamed body cap
(413), same-origin guard (403), bearer token on DELETE/PATCH/POST/PUT (401 +
`Bearer realm="kala-studio"`), static files, router. The read-only design,
journal and CAD provider routes carry their own bearer gate (401 + `Bearer`),
like the source's `_require_bearer`. Forwarded headers are never trusted.
Static serving is GET/HEAD only, `/api` is reserved, missing paths fall back
to `index.html`, and lookup canonicalizes the target and refuses path or
symlink escapes.

`POST /api/studio/import` (the view-only STEP import) has its own body
limit instead of the shared Studio limit. The limit is the shared STEP
import ceiling (see below), and the worker keeps its own gate: the front
door never raises the worker's ceiling, and the worker's refusal is
forwarded unchanged.

## The STEP import ceiling (ADR-0011)

A provider-built STEP assembly is much larger than an ordinary Studio
request, so the view-only import has its own ceiling. The setting is one
variable, `KALA_STEP_IMPORT_MAX_BYTES`, with the worker's own semantics
(`src/step_limit.rs` is the exact port of the Python worker's resolution):

| Value | Result |
|---|---|
| absent or blank | the 64 MiB default |
| an integer in `1 ..= 512 MiB` | that value |
| anything else | a startup refusal, with the worker's own message |

The front door resolves the value **once**, before the worker is launched:

* the public import route enforces the resolved value;
* the worker process is launched with the same value in its environment, so
  the two sides always accept exactly the same body;
* an unusable value refuses startup, exactly like the Python worker.

The CPython `int()` grammar is ported, including single underscores between
digits (`1_048_576` is 1 MiB). One documented difference: CPython also
accepts non-ASCII decimal digits, and this port refuses them with the same
"must be an integer number of bytes" message.

## Chat with designs

`POST /api/studio/chat` may carry a design.

* **Local provider:** the worker parses the prompt with `intent/parse`. A
  prompt that is not a design request keeps the ordinary analytic reply; a
  design prompt gets the strict reading, and the reply text is built from the
  parsed family, the missing values and the parser notes.
* **Remote providers:** the front door adds the DesignIntent JSON Schema
  (fetched from `intent/schema`) to the prompt, extracts the fenced JSON from
  the reply and validates it with `intent/validate`. One repair round-trip is
  allowed.
* **The CAD provider is never chosen automatically** (ADR-0011). The request
  carries an optional `cad_provider`; without it the turn asks the operator
  which provider to use and compiles nothing. A selected provider
  (`opencascade`, `freecad` or `solidedge`) is passed to the worker unchanged:
  the worker decides whether it can build on this host, a refusal is relayed
  in the source's own wording (the provider, its reason, the providers that
  can build here and "No design was built."), and another provider is never
  tried in its place.
* Every turn appends a `chat.turn` event through the worker's
  `journal/append`. The payload never carries the prompt text, only its
  digest and length.

## The native geometry backend

`src/native_geometry/` is the default front door's geometry implementation.

* `service.rs` is the port of the Python `DesignService`: the intent
  resolution, the design identity (`sha256(canonical script || 0x00 ||
  provider)`), the provider-required rule, the view-is-the-provider's-export
  rule, the provider-measured checks, the journal events, the bounded LRU
  (32) and the cold reload from the journal.
* `providers.rs` is the closed ADR-0011 provider set: the opt-in
  `opencascade` builder (the real kernel), `freecad` (out of process) and
  `solidedge` (the Windows loopback bridge).
* `documents.rs` builds the response documents in Pydantic field order with
  pydantic-core's float text.
* `wire.rs` renders the construction script in its two forms: the canonical
  bytes that feed the design identity and the model dump the response
  carries.

The primitive preview, export and handoff routes keep the reviewed analytic
geometry crates (`kala-geometry`, `kala-step`) behind the `native-primitives`
feature, which the `native-geometry` feature enables.

```bash
cargo build -p kala-studio                              # the native default
cargo build -p kala-studio --no-default-features         # the portable surface
```

## Compatibility notes

- Request bodies follow CPython `json.loads(bytes)` encoding detection
  (UTF-8, UTF-8 BOM, UTF-16LE/BE and UTF-32LE/BE, with or without a BOM).
  CPython's `NaN`/`Infinity` constants keep their source validation branch,
  and `json_invalid` offsets are mapped back to the original decoded
  document so a constant replacement never shifts a later error. A
  CPython-scanner-compatible walk (`src/json_diagnostics.rs`) reproduces the
  source `body.<pos>` for partial literals, invalid escapes, malformed
  numbers, unterminated strings and trailing commas. On the default path the
  body is parsed by the worker, which is the source's own parser.
- Studio success bodies are ordered UTF-8 JSON. FastAPI's `response_model`
  path is serialized by pydantic-core, whose float text is the Rust
  `serde_json` form (`0.0000912...`, `1e-6`, `1e+16`), not CPython `repr`.
  The frozen preview digests prove this; `tests/float_vectors.rs` checks
  2,000 Python-generated bit patterns. Route/transport refusals use the
  `json.dumps(..., sort_keys=True)` shape and Pydantic-style validation uses
  the ordered envelope.
- The health document carries the ADR-0009 `design_available` flag and the
  ADR-0010 `geometry_worker` state. The Python public server has no worker,
  so the live differential compares the health document without that one
  field.
- Remote chat uses fixed OpenAI/OpenRouter endpoints, a 30-second timeout,
  no redirects, a 1,000,000-byte response cap, and only the documented
  environment credential. The `HttpTransport` trait is the isolated test
  seam for wire/error assertions; no live provider is called in tests.
  Provider replies accept the same byte encodings as CPython
  `json.loads(bytes)` (UTF-8 BOM, UTF-16, UTF-32), accept `NaN`/`Infinity`,
  and are bounded by the `ChatResponse` DTO: a reply model longer than 256
  characters is the source's sanitized 500, not a widened body.
- Proxy environment handling reproduces `urllib.request.getproxies_environment`
  and `proxy_bypass_environment` (`src/proxy.rs`): lowercase `*_proxy`
  variables win over uppercase spellings, an empty lowercase value removes
  the uppercase one, `ALL_PROXY` is ignored for HTTPS, uppercase
  `HTTP_PROXY` is suppressed under CGI (`REQUEST_METHOD`), and `NO_PROXY`
  uses CPython's suffix/leading-dot/port rules. The fixed endpoint URLs are
  never overridable.
- TLS uses the source's default trust model. On Unix/macOS the reviewed
  `rustls-tls-native-roots` default is kept (OS store and OpenSSL defaults
  plus `SSL_CERT_FILE`/`SSL_CERT_DIR`). On Windows the client replaces the
  built-in set with the current-user and local-machine `CA` and `ROOT`
  stores through the supported `schannel` CryptoAPI binding, plus
  `SSL_CERT_FILE`, mirroring `SSLContext.load_default_certs()`; certificate
  and hostname verification stay enabled. `tests/tls_trust.rs` proves
  trusted/untrusted CA, hostname mismatch, explicit-DER-root assembly and
  the proxy rules against a synthetic local HTTPS server.
- The local chat provider reproduces the Python keyword/suite tables,
  lookbehind behaviour, operation order, `:.3f`/`:.3g` formatting and
  1,024-code-point cap; 106 Python oracle vectors and a live differential
  prompt matrix pass byte-for-byte.
- Settings files and token files keep the Python formats (`indent=2`,
  sorted keys, ASCII escapes, trailing newline, `0600`; atomic replacement).
  Stored keys override environment keys and are never echoed. There is no
  file-size cap. `Path.read_text`-style failures reproduce the source class
  exactly: a non-UTF-8 file is a `UnicodeDecodeError` (sanitized 500 reads;
  422 codec detail on a write), and an integer literal beyond CPython's
  4,300-digit conversion limit is a `ValueError` on reads (sanitized 500) and
  a 422 carrying the exact CPython message on a write, leaving the file
  untouched. Values are stored as `kala_json::JsonText` code points and the
  reader walks the document with CPython's `json.loads` rules: duplicate
  object keys are last-wins at every level, escaped lone surrogates survive
  in keys and values, `NaN`/`Infinity` are values, and a document with
  duplicates plus an unrelated surrogate still yields the valid keys. Writes
  re-emit `json.dumps(..., ensure_ascii=True)` bytes; reads report
  `configured=true`/`source=stored`, and a chat that would place a surrogate
  value in a header is the source's sanitized 500.

## Known gaps / limits

- The native build links the **system** OpenCascade (7.9.3 here), while the
  Python reference imports the `cadquery-ocp` binding (8.0.1.1, OCCT 8.0).
  Two facts follow and both are declared in the live differentials: the
  reported `opencascade` provider version names a different kernel binding,
  and the two readers assemble the same STEP file with the faces in a
  different order, so a display mesh has the same vertices and the same
  triangles in a different order. The differentials compare the mesh
  geometry without an order.
- The design document the native backend returns carries the journal head of
  its own chain. Two processes never share a chain digest, so the live
  differential replaces the volatile journal fields before comparing.
- The native design cache holds the plain view data, not the kernel handle,
  so a drawing or a section re-imports the stored STEP artifact instead of
  reusing a cached shape. The result is identical; the work is not.
- `serde_json` rejects lone-surrogate `\uD800` escapes that Python accepts
  for HTTP request bodies; requests still fail 422, but the detail
  (`json_invalid` vs Pydantic `string_unicode`/`extra_forbidden`) differs.
  Provider replies are handled (500 when the surrogate reaches
  `text`/`model`, 200 in an unused field), and stored settings values are
  fully preserved (see above). For settings files the reader is iterative,
  so it has no parser depth cap of its own; CPython's C scanner can raise
  `RecursionError` (sanitized 500) beyond its process-dependent stack
  threshold, which the native reader does not reproduce.
- The design request validation reproduces the Pydantic error *kind* for the
  cases the corpus and the differentials exercise (a missing provider, an
  unknown provider). A field-level intent error reports the reviewed
  `kala-design-intent` message with the `value_error` kind, where Pydantic
  would name a more specific kind (`greater_than`, `string_type`, ...); the
  message text and the location are the same.
- Static MIME types use a host-independent subset of Python's host database;
  exotic extensions fall back to `application/octet-stream` (which the Python
  middleware also forces for non-`text/*`/`application/*` types). A symlinked
  directory `index.html` that escapes the root is refused here, while the
  Python source could follow it.
- Remote network error class names and HTTP reason phrases are approximated
  where reqwest/rustls do not expose urllib's exact names. On Windows,
  CPython also enumerates the enterprise/group-policy/services/`USERS`
  store locations and reads `SSL_CERT_DIR`; this build loads the
  current-user and local-machine `CA`/`ROOT` stores and `SSL_CERT_FILE`
  only. No Windows host was available, so that enumeration is
  compile-gated and compile-checked, not executed.
- Windows code paths are behind `cfg` (mode bits, atomic replacement,
  home-directory discovery, certificate stores) but were not executed on a
  Windows host. Tests are run on Linux; no Windows/COM or packaging cutover
  is claimed. The worker lifecycle (readiness, the token, the restart and
  the stdin lifeline) is proven on Linux only.
- The `clap` help text differs from argparse. An invalid token exits 1 with
  a sanitized message instead of a Python traceback.

## Verification

Run these from `backend/` (the workspace root), or add `-p kala-studio` to
the package-specific commands:

```bash
cargo fmt --all --check
cargo test -p kala-studio --locked                        # the native default
cargo test -p kala-studio --locked --all-features         # every feature
cargo test -p kala-studio --locked --release
cargo clippy -p kala-studio --all-targets --all-features --locked -- -D warnings
cargo build --release -p kala-studio --locked
python3 crates/kala-studio/tests/python_differential.py     # the nine-route corpus
python3 crates/kala-studio/tests/worker_differential.py     # corpus + design routes
python3 crates/kala-studio/tests/python_provider_dto_oracle.py  # monkeypatched provider cases
python3 crates/kala-studio/tests/replay_binary_corpus.py      # fixture cases on release bin
```

The four live differentials run against the **native** backend by default;
`tests/worker_differential.py --backend worker` runs the same matrix against
the supervised Python worker.

The STEP import ceiling has its own tests: `tests/step_import_limit.rs`
covers the default, the override, the unusable value and the body limit at
the limit and one byte over it, and `tests/worker_lifecycle.rs` proves that
the real worker process carries the resolved value.

The live/OCP oracles default to the repository `.venv`; on Windows set
`KALA_STUDIO_OCP_PYTHON` to an absolute interpreter path.

`tests/step_export_ocp.rs` re-imports the real HTTP STEP bodies with the
test-only OpenCascade oracle (`tests/step_ocp_oracle.py`) and compares
volume/area/bounds in millimetres against the Python exporter.
`tests/native_geometry.rs` proves the native backend over a real socket: the
design route, the provider refusals, the journal routes, the intent routes,
the view-only import, the drawing and section routes, and the cold reload of
a **Python-written** journal. The Python scripts are test oracles only; the
binary and library never invoke Python except through the supervised geometry
worker.
