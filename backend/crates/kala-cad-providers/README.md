# kala-cad-providers

Native CAD provider **connections** for the Kala Agent backend, implementing
the provider model of ADR-0011:

- the `CadProvider` contract, `Availability`, `MeasuredBounds`,
  `MeasuredProperties`, `ProviderBuild`, `ProviderStatus`, the closed
  `CadProviderId` set, and the provider refusal errors;
- `FreeCadProvider` — a real `FreeCADCmd` subprocess, supervised by the
  accepted `kala-process` crate;
- `SolidEdgeProvider` — a loopback-bridge probe and an honest build refusal,
  using the accepted `kala-solidedge-client`.

This is a **standalone crate** with its own `[workspace]` and `Cargo.lock`. It
is not a member of the root backend workspace. It consumes `kala-contracts`,
`kala-json`, `kala-process` and `kala-solidedge-client` read-only through path
dependencies and edits none of them.

## What this crate does not do

It computes **no geometry**. There is no volume formula, no bound, no surface
area, no STEP writer and no tessellation here. Every number a build returns is
the provider's own number, parsed from the same envelope the Python provider
reads. There is intentionally no `opencascade` provider: the OpenCascade
compiler and viewer stay in the geometry worker.

Nothing in this crate is a runtime cutover, a Windows-runtime proof, or a
readiness claim. The FreeCAD path is proven against a real FreeCAD 1.1.4 on the
development host; the Solid Edge path is proven only against the live Python
provider on Linux plus an in-process mock bridge.

## Public API

```rust
use kala_cad_providers::{CadProvider, FreeCadProvider, SolidEdgeProvider};

let mut freecad = FreeCadProvider::with_defaults()?;
let availability = freecad.probe();          // available, version?, reason?
let build = freecad.build(&script)?;         // STEP bytes + provider facts

let mut solidedge = SolidEdgeProvider::with_defaults()?;
let status = solidedge.status();             // availability + static identity
```

The Python protocol also accepts a `DesignIntent`; both providers delete it
without reading it (`del intent`), so the Rust connection takes the already
resolved `ConstructionScript` and nothing else. `probe`/`build` take
`&mut self` because the probe result is cached, like the Python instances.

The full surface:

- `base` — `CadProviderId` (`OpenCascade`, `FreeCad`, `SolidEdge`),
  `ALLOWED_PROVIDER_IDS`, `Availability`, `MeasuredBounds`,
  `MeasuredProperties`, `ProviderBuild`, `ProviderStatus`, `CadValueError`,
  `CadProviderConfigError`, `CadProviderUnavailableError`,
  `CadProviderBuildError`, `CadProviderError`, the `CadProvider` trait, and the
  four bound constants (`MAX_REASON_LENGTH` 512, `MAX_VERSION_LENGTH` 64,
  `MAX_WARNING_LENGTH` 256, `MAX_WARNINGS` 16).
- `freecad` — `FreeCadProvider`, `FreeCadProviderOptions`,
  `canonical_request_json`, `resolve_step_import_max_bytes`,
  `validate_step_import_max_bytes`, `DRIVER_SOURCE`, and the protocol
  constants (`FREECAD_EXECUTABLE_NAMES`, `DEFAULT_TIMEOUT_SECONDS` 120,
  `MIN_TIMEOUT_SECONDS` 5, `MAX_TIMEOUT_SECONDS` 600,
  `DEFAULT_PROBE_CACHE_SECONDS` 30, `VERSION_PROBE_TIMEOUT_SECONDS` 20,
  `NATIVE_MAX_BYTES` 64 MiB, `MAX_RESPONSE_BYTES` 4 MiB, the file and
  environment names, and `UNAVAILABLE_REASON`).
- `solidedge` — `SolidEdgeProvider`, `SolidEdgeProviderOptions`,
  `DEFAULT_BRIDGE_URL`, `BRIDGE_URL_ENV_VAR`, `TOKEN_FILE_ENV_VAR`,
  `TOKEN_FILENAME`, `DATA_DIR_PARTS`, `HEALTH_PATH`,
  `DEFAULT_TIMEOUT_SECONDS` 5, `MAX_HEALTH_BYTES` 64 KiB,
  `MIN_TOKEN_LENGTH` 16, `MAX_TOKEN_LENGTH` 256,
  `DEFAULT_PROBE_CACHE_SECONDS` 5, `NOT_REACHABLE_MESSAGE`,
  `UNAVAILABLE_REASON`.

## FreeCAD connection

The provider writes the **same driver script** as
`kala_agent.studio.design.providers.freecad` (`src/driver.py`, embedded byte
for byte) plus the canonical request JSON, then runs:

```text
<absolute FreeCADCmd> <temporary driver.py>
    cwd        = the temporary provider directory
    stdin      = empty
    stdout     = bounded capture (64 MiB)
    stderr     = bounded capture (16 MiB)
    env        = credential-scrubbed snapshot + the KALA_FREECAD_* protocol
    shell      = never
    wall time  = 5..600 s (120 s default)
```

The driver lowers the script with the adapter's official
`worker.invoke_design` entry point, builds in a FreeCAD document, exports STEP
and FCStd, reopens the FCStd, and reports FreeCAD's own `BoundBox` and
`Solids`. The adapter owns the export target, so a placed part and a
place-only assembly export the geometry they actually describe; a place-only
assembly exports as one STEP assembly with its placed occurrences. The
provider reads the envelope, verifies every artifact path is inside the build
workspace, re-hashes every artifact with SHA-256, and only then returns a
`ProviderBuild`. A missing response, a refusal, a crash, a timeout or a bad
artifact is a typed `CadProviderBuildError`; an adapter refusal that does not
name the failing fuse/compound operation gets the same ` (op: …)` suffix the
Python provider appends.

Credential-shaped environment keys (`TOKEN`, `API_KEY`, `SECRET`, `PASSWORD`,
`CREDENTIAL`) are removed before the child starts, and `PYTHONPATH` is removed,
so FreeCAD never inherits a worker token or a chat API key.

`FreeCadProviderOptions::sys_path` is the adapter search path handed to the
driver. When it is absent the provider discovers the repository layout from
the current directory and the executable's ancestors:
`adapters/freecad/src`, `src`, and each `.venv/lib/python*/site-packages`. A
deployment with a different layout passes the path explicitly.

When the adapter ships per-ABI dependency directories beside `src`
(`site-packages-cp311` … `site-packages-cp314`), the provider runs a small
Python-identity probe in `FreeCADCmd`, caches the result with the availability
probe (`DEFAULT_PROBE_CACHE_SECONDS`), and adds exactly the matching
`site-packages-<abi>` directory after `src`. An unsupported Python is an
unavailable provider whose reason names the interpreter version (for example
`Python 3.10.14 (cp310)`); no other ABI's compiled extension is loaded. A
development checkout with no `site-packages-*` directories keeps the single
path behaviour and spawns no extra process.

## Solid Edge connection

The provider resolves `KALA_SOLIDEDGE_BRIDGE_URL` (default
`http://127.0.0.1:8797`) and the token file exactly as the Python provider
does: explicit argument, then `KALA_SOLIDEDGE_TOKEN_FILE`, then
`%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`. It accepts only `http` on
`127.0.0.1`, `localhost` or `::1` with an explicit port (`1..=65535`), no user
information, no path (except `/`), no query and no fragment. The token is read
as ASCII, trimmed, and must be 16–256 characters. The authenticated
`GET /v1/health` call is made through `kala-solidedge-client`; proxies are
never used and redirects are refused.

`available` is true only when the bridge reports both `available` and
`supported`. A reachable bridge still refuses a build:

```text
the Solid Edge bridge is reachable but this Studio worker has no verified
native build executor; no Solid Edge build was attempted          (op: bridge.build)
```

On Linux the probe returns the exact ADR-0011 text:

```text
Solid Edge build requires the Windows bridge (not reachable); this host is
linux, so the ADR-0008 bridge cannot run here
```

and a build is a `CadProviderUnavailableError` with the message
`Solid Edge build requires the Windows bridge (not reachable)`. No token is
read and no socket is opened in that path.

## Intentional differences from the Python providers

FreeCAD:

1. **Child environment validation.** `kala-process` accepts keys matching
   `[A-Za-z_][A-Za-z0-9_]{0,63}`; entries with other key shapes (for example
   exported shell functions) and non-UTF-8 names/values are skipped. Python
   forwards any key without `=` or NUL.
2. **Adapter search path.** Python asks its interpreter for `sysconfig`'s
   `purelib`. Rust has no Python interpreter, so it discovers the repository
   layout (including the repository `.venv`) or takes an explicit path.
3. **Response parsing.** `kala-json`'s strict parser rejects duplicate object
   keys and non-finite numbers; such an envelope is reported as "did not
   report a result" rather than partially read. A successful FreeCAD build
   emits neither. Integer values that overflow `f64` are dropped rather than
   raising `OverflowError`.
4. **Stderr tail.** The reviewer sees the bounded capture retained by
   `kala-process` (up to 16 MiB); above that cap the earliest bytes are
   retained and the tail is unavailable. Python reads the last 1 600 bytes of
   an unbounded log file.
5. **Probe decoding.** Probe output is decoded with replacement; Python
   raises `UnicodeDecodeError` on invalid text. A capture-limit exit is a
   failed probe rather than a successful one.
6. **Relative executable paths.** An explicit relative executable is made
   absolute once and used absolutely. Python passes the relative string to
   both the probe and the build, so its build (with `cwd` in the temporary
   directory) would fail to find it.
7. **Timeout stage.** The native supervisor has a bootstrap deadline; any
   deadline maps to the same Python timeout message.

Solid Edge:

1. **Health document strictness.** The accepted client parses the protocol-v1
   `Health` model. A wrong protocol version, an unknown field, a missing
   required field, an over-long `se_version`/`detail`, or an over-long body is
   "did not answer a valid health probe". Python reads any JSON object and
   looks up four fields. The real bridge document parses in both.
2. **`localhost` transport.** The client accepts literal loopback addresses,
   so `localhost` is transported as `127.0.0.1`; the diagnostic URL still
   says `localhost`.
3. **Out-of-range/non-numeric ports.** Python's `urlparse().port` raises
   `ValueError` outside its `try`, so its probe raises for
   `http://127.0.0.1:65536` or `:http`. The native provider classifies them as
   an invalid URL and returns the standard unavailable reason.
4. **Missing home directory.** If `HOME`/`USERPROFILE` is absent, the default
   token path is unknown and the probe returns the token-file reason; Python's
   `Path.home()` can raise `RuntimeError`.
5. **Health body cap.** Python stops at 64 KiB; the client stops at 1 MiB. A
   valid protocol document is far below both.

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked                 # debug
cargo test --locked --release
cargo build --locked --release
cargo check --target x86_64-pc-windows-gnu --all-targets
cargo check --target x86_64-pc-windows-msvc --all-targets
```

- `tests/freecad_differential.rs` — **live real FreeCAD 1.1.4** against the
  repository Python provider. Builds all 28 cases of
  `tests/fixtures/construction_parity.json` and 15 deterministic randomized
  cases — 43 builds, 0 refusals with the current adapter — comparing the
  measured volume/area/bounds, warnings, validity, solid count, provider
  version, native artifact presence, refusal messages and `op` when a refusal
  occurs, the exact canonical request bytes, and the embedded driver source.
  Two of the corpus cases are place-only assemblies that now export as STEP
  assemblies; the randomized set includes pads, revolves, pockets, placed
  parts and fuse assemblies. Every STEP produced
  by Rust is imported by the Python importer (`import_step_bytes`) and its
  bounds and solid count are compared with the bounds of the STEP the Python
  provider produced. The test skips only when no `FreeCADCmd` is installed.
- `tests/freecad_unit.rs` — 23 tests with a **fake** `FreeCADCmd`: probe
  caching/version parsing, the subprocess hand-off, credential scrubbing, the
  protocol environment, artifact containment (including a symlink escape),
  size/digest integrity, response errors and the bounded stderr tail, the
  exact timeout, refusal parsing and op naming, a place-only assembly reaching
  FreeCAD instead of being pre-refused, unreported-value honesty, bounds
  validation, truncation, constructor validation, STEP-limit resolution, the
  provider status row, the closed provider set, contract-model validation,
  an executable path containing spaces, cp311-cp314 selection (exactly one
  matching `site-packages` directory on `sys.path`), and the honest
  unavailable reason for an unsupported Python (cp310 / 3.10.14).
- `tests/solidedge_differential.rs` — 3 live differentials against the
  repository Python provider: availability and reasons (27 cases including
  URL, token-file, `LOCALAPPDATA`, detail and health-status cases), build
  refusals (Linux, reachable with no executor, no capability, unreachable),
  and a URL-resolution corpus (39 fixed plus 180 deterministic generated URLs)
  with no network. Cases where Python raises `ValueError` from
  `urlparse().port` are asserted as the documented native difference.
- `tests/solidedge_mock_bridge.rs` — 13 tests against an in-process mock
  bridge: authentication, the health route, no redirects, the token never
  appearing in a message, token-source precedence, caching, non-protocol
  documents, the documented strictness differences, URL validation, and the
  Linux refusal that opens no socket.

`KALA_PYTHON` selects a different interpreter for the differential tests; the
default is the repository `.venv/bin/python`.

## Evidence boundary

The FreeCAD differential is real: FreeCAD 1.1.4 on this Linux host built every
case and the Python importer verified the STEP bytes. The Solid Edge
differential is real for the live Python provider on Linux and for the health
contract against a mock bridge. There is no Windows host, no Solid Edge
installation and no native Solid Edge build in this repository; no test here is
evidence about any of them.
