# Solid Edge on Windows

> Status: **first milestone implemented and checked on a Linux development
> host. This is still not a production CAD capability.** The contract, the
> bridge foundation, and the Windows-local client exist and pass their
> test/lint/type checks, but there is no production native build executor,
> no Windows probe, and no calibration. No Windows host was available, no
> Solid Edge edition was probed, and no native build, calibration, or
> feature verification has been performed.

## What this is

Kala and Solid Edge run on the **same Windows machine**. Kala is the
agent and workspace; Solid Edge is the licensed desktop CAD seat that
performs the feature modelling. The integration is a local one: a small
authenticated service on loopback, plus a client that compiles a Kala
construction script into a bounded build request.

Nothing here requires a second machine, a remote shell, or a port that is
reachable from the network. Both processes run in the same interactive
Windows session. Development and the test suite may run on Linux, but the
**target runtime is Windows only**. Any Linux-side code is a development
and CI convenience, not a supported runtime path.

The pieces:

1. **A versioned wire contract** — the `kala-solidedge-bridge`
   distribution's `kala_solidedge_bridge.protocol` module. It is pure
   Pydantic v2, imports no Windows module, and can be installed and
   tested anywhere. It defines the only requests and results that cross
   the boundary.
2. **A local bridge** — the same distribution's runtime: a small
   authenticated HTTP service, a probe, and one long-lived COM worker
   process. It runs in the logged-in Windows desktop session and owns
   one Solid Edge automation instance.
3. **A local client** — distribution `kala-agent-solidedge`, command
   `kala-solidedge`. It validates an existing Kala `ConstructionScript`,
   compiles it into bridge operations, refuses anything unsupported
   *before* it opens a socket, submits the build, polls, and downloads
   artifacts into a caller-owned output directory.

The dependency direction is deliberate: the bridge is a standalone
service that does not need the Kala kernel to run. The client may import
the narrow core contracts it needs (the construction script and its
errors). Kala core does not import either package, does not discover
them, and does not register a tool for them. Installing this
integration does not change `kala-agent init`, the core HTTP API, or the
core CLI.

## Architecture

```text
Windows desktop session (one machine)
  |
  |  Kala Studio / Kala core (local)
  |    - the agent and workspace
  |    - no Solid Edge COM in the core process
  |
  |  kala-solidedge (local client)
  |    - validates ConstructionScript, rejects unsupported features
  |    - compiles to bridge protocol v1 operations
  |    - bearer-authenticated HTTP over loopback
  v
127.0.0.1:8797
  |
  |  kala-se-bridge serve
  |    - stdlib HTTP server bound to loopback only
  |    - auth, request validation, job ledger, artifact manifests
  |    - no COM call ever runs on an HTTP thread
  v
one spawned worker process
  |    - explicit STA: CoInitializeEx(COINIT_APARTMENTTHREADED = 2)
  |    - idle message pump while attached
  |    - lazy pywin32 import, only in the session module
  v
Solid Edge desktop application (interactive session)
```

The properties that matter:

- **Everything is local.** The bridge binds `127.0.0.1` only. There is
  no inbound port on the network interface, no remote transport, and no
  configuration that exposes the bridge to another host. A loopback
  bind is a security property, not an implementation detail.
- **The bridge is started from the desktop.** It must run in the
  logged-in interactive session, not as a Windows service and not as a
  scheduled task. Those have no interactive window station, and Solid
  Edge automation needs one.
- **One COM apartment, one worker process.** COM calls happen in a
  single-threaded apartment with a pumping message loop. HTTP request
  threads never call COM directly; they enqueue work and read ledger
  state.
- **The bridge stays kernel-free.** It is a small service with a small
  dependency set. It does not import the Kala application, the Studio
  backend, or the core executor. That keeps the Windows service
  reviewable and independent of a kernel release.

## Milestone scope

Version 1 of the contract is deliberately small.

**Supported, when the native executor exists and the host is calibrated:**

- Basic parts: a solid built from a rectangle or circle profile on a
  named plane, followed by finite or through-all cuts. Additive
  features may extend a part that already has a solid. The measured
  corpus exercises same-plane holes, and the compiler makes no broader
  claim about a cut drawn on another plane.
- Rectangular profiles revolved about the component-local `Y` axis on
  `XY` or `YZ`. A circle "revolve" is lowered to an extrusion (see
  [construction-semantics.md](construction-semantics.md)).
- Positioned assemblies: each finished part placed once with a
  translation, saved as `.asm`, and exported to STEP.
- Measurement, `.par` save, and STEP export per part.

**Not supported, and rejected by the compiler before any socket is
opened:**

- Helix, threads, and any de-threading or thread-approximation trick.
- Fillets, chamfers, drafts, shells, patterns, and other feature edits.
- Fuse, compound, and any boolean assembly operation.
- FEA, meshing, materials, loads, solvers, and results.
- An in-Solid-Edge add-in, ribbon command, or macro surface.
- Arbitrary scripts, arbitrary paths, arbitrary document names, or
  caller-supplied code of any kind.
- Touching a document the operator already has open.

**Deferred beyond this milestone:**

- A native production geometry executor. The bridge can carry a
  procedure and report honest status, but no production handler that
  drives real Solid Edge features has been live-verified.
- Windows calibration and feature verification. Calibration is recorded
  only from an actual probe on the actual host; it is not a constant.
- Kala Studio launcher integration and shared-CAD extraction. Both are
  gated on deterministic real-native evidence, not on the existence of
  these packages.

## Wire contract

`kala_solidedge_bridge.protocol` is the single source of truth. Its
docstring is the contract; this section is a summary.

A request is a `BuildRequest` with `protocol_version = 1`, a bounded
`request_id`, an optional `calibration_id`, a short `summary`, a
`procedure` of 1–600 operations, a `timeout_seconds` in `[1, 900]`
(default 120), and an optional `density_kg_m3`. There is no field for a
file path, a script, an executable, a URL, or a document identifier.

The operation set is a discriminated union:

- `part.new`, `assembly.new`
- `sketch.rectangle`, `sketch.circle`
- `solid.extrude`, `solid.cut`, `solid.revolve`
- `part.measure`, `part.save`, `part.export_step`
- `assembly.place`, `assembly.save`, `assembly.export_step`

The procedure validator refuses a sketch that is not consumed, a cut
before a solid, a measure/save/export before a solid, a part with no
solid, more than eight parts, duplicate part names, an assembly that
references an unfinished or missing part, a component placed twice, and
any operation after `assembly.new` that is not an assembly operation.
The part being operated on is implicit: the most recent `part.new`. No
operation can name a document.

Results are `BuildAccepted`, `BuildStatus`, `BuildResult`, `Health`, or
`ErrorResponse`. A `BuildResult` carries local part metrics (volume,
surface area, a local range box), optional world assembly bounds,
artifacts, a bounded step trace, warnings, and a typed error. An
`Artifact` is `name`, `format` (`par` | `asm` | `step`), `media_type`,
`size_bytes`, and `sha256` — never a disk path, absolute or relative.

`BuildRequest.canonical_request_bytes()` is the deterministic digest
input: UTF-8, sorted keys, compact separators, ASCII escapes, and
`request_id` removed. `request_id` is transport correlation only, so a
retry of the same semantic request has the same digest and the bridge
can detect a same-ID/different-payload conflict.

Real-valued fields reject booleans, numeric strings, `NaN`, and
infinities; plain JSON integers are accepted where a real number is
expected. Sizes are inclusive `0.001`–`5000` mm, offsets are
`-5000`–`5000` mm, angles are `(0, 360]` degrees, and density is
`(0, 22000]` kg/m³.

## Installing

The Windows machine needs a licensed Solid Edge installation and Python.
**Python 3.12 x64 is the recommended interpreter**; it is a
recommendation, not a hard requirement, and it is not a claim that
another bitness cannot work. COM here is out-of-process automation, so
the Python bitness and Solid Edge's bitness are independent in
principle; the probe on the actual host decides what works. No absolute
cross-bitness guarantee is made.

Run these in a normal PowerShell or terminal **in the interactive
desktop session**:

```powershell
py -3.12 -m venv .venv
.\.venv\Scripts\Activate.ps1

# The bridge service and the local client. The bridge does not need
# the Kala kernel; the client imports the narrow core contracts it uses.
pip install -e <path-to-repo>\adapters\solidedge-bridge
pip install -e <path-to-repo>\adapters\solidedge

# Confirm what this build actually exposes before trusting any flag.
kala-se-bridge --help
kala-solidedge --help
```

If the local client is delivered as part of a Kala Studio build rather
than as a separate install, the commands and the contract are the same;
only the packaging differs. The client package is present in this
checkout under `adapters/solidedge`; if it is absent from a checkout,
install the bridge only and expect `kala-solidedge` to be unavailable.
Do not assume a command exists because this document names it: run
`--help` on the installed build.

Do **not** register the bridge as a Windows service or a scheduled task.
A service that claims to work will fail at the first automation call,
and a scheduled task would run in a session the user cannot see.

`pywin32` is declared with a Windows platform marker and is imported
lazily by the session module. Importing the contract on a non-Windows
host never loads `pythoncom`, `win32com`, or `pywintypes`.

## Running a build

The public command names intended by this milestone are:

**Bridge** (`kala-se-bridge`):

```text
kala-se-bridge token  [--data-dir PATH]
kala-se-bridge probe  [--launch] [--output PATH] [--data-dir PATH]
kala-se-bridge serve  [--port 8797] [--data-dir PATH]
```

**Local client** (`kala-solidedge`):

```text
kala-solidedge doctor  [--bridge-url URL] [--token-file PATH]
kala-solidedge build --script FILE.json [--dry-run] [--yes]
                     [--calibration-id ID] [--timeout SECONDS]
                     [--bridge-url URL] [--token-file PATH] [--output-dir PATH]
kala-solidedge status BUILD_ID [connection flags]
kala-solidedge cancel BUILD_ID [connection flags]
```

`--dry-run` validates and prints the review summary without a token and
without a socket; it is the supported offline path on a development
host. A real `build` requires `--calibration-id` from `doctor`, is
refused when the live health says the host is unsupported, uncalibrated,
busy, or reports a different calibration id, and requires `--yes` when
stdin is not a terminal. Artifacts are downloaded under
`<output-dir>/builds/<build-id>/`, so two builds that produce the same
artifact name do not overwrite each other.

Actual native operations require Windows. `doctor`, `build`, `status`,
and `cancel` exist on another platform for development, and a build that
would reach Solid Edge fails usefully there (`exit 2`) rather than
pretend to succeed. `serve` refuses a non-Windows host the same way,
before it would create a data directory or token.

Environment and defaults:

- Bridge URL: `KALA_SOLIDEDGE_BRIDGE_URL`, default
  `http://127.0.0.1:8797`.
- Token file: `KALA_SOLIDEDGE_TOKEN_FILE`. On Windows the default is
  `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`; the installed
  client reports the path it actually used, and `--token-file` overrides
  it.
- Data directory: `%LOCALAPPDATA%\Kala\SolidEdgeBridge`.
- Output directory: `%LOCALAPPDATA%\Kala\SolidEdge\runs` on Windows, or
  an explicit `--output-dir`. Each build downloads into
  `<output-dir>\builds\<build-id>\`; the bridge never supplies a path.

A typical session, in order:

1. Create or read the token, in the desktop session:

   ```powershell
   kala-se-bridge token
   ```

   This is the one command that intentionally prints the token. It
   prints it to that terminal only; it does not log it elsewhere. The
   token file is private to the current user. Do not paste the token
   into chat, a ticket, a shell history line, or a committed file. If a
   token is exposed, regenerate it with `kala-se-bridge token` and
   restart `serve`.

2. Start the bridge in the same desktop session:

   ```powershell
   kala-se-bridge serve --port 8797
   ```

3. Check the path honestly:

   ```powershell
   kala-solidedge doctor
   ```

   `doctor` distinguishes unreachable, authentication failure,
   unsupported host, uncalibrated host, busy bridge, and protocol
   mismatch. It is a diagnostic, not a promise of readiness.

4. Compile and run a script:

   ```powershell
   kala-solidedge build --script plate_with_hole.json
   kala-solidedge status <build-id>
   ```

   The client preflights the script before opening a socket. A build
   that requires an unsupported feature fails locally with a message
   naming the feature; it never reaches the bridge.

5. If a build hangs, `kala-solidedge cancel <build-id>` asks the bridge
   to stop between operations. It is cooperative. It is not evidence
   that a COM call already in progress has stopped; see "Lifecycle" in
   [ADR-0008](adr/0008-solid-edge-windows-bridge.md).

## Security posture

The bridge runs outside Kala's sandbox. That is an explicit, recorded
exception, not an oversight: the kernel's sandbox profiles target
short-lived Linux subprocesses and are not being ported to Windows. A
COM client for a logged-in desktop application cannot be placed inside
that jail, and documenting a control that is not enforced would be
worse than recording the exception. ADR-0008 records it and the
compensating controls. In brief:

- **Bearer authentication on every route**, including `GET /v1/health`.
  There is no unauthenticated path.
- **Loopback bind only.** The server binds `127.0.0.1`; the Host
  authority and port are validated, and forwarded headers are not
  trusted.
- **No browser origin.** The bridge is not a browser API. Requests that
  carry an `Origin` header are rejected, so a web page cannot drive it
  through the loopback port.
- **No arbitrary paths or code.** Requests contain operations and
  dimensions only. Artifact names are resolved through completed
  manifest entries; a name that is not in the manifest is not served.
  There is no upload endpoint, no script endpoint, and no executable
  path field.
- **Bounded everything.** Request bodies, response bodies, JSON depth,
  procedure length, part count, artifact count, artifact size, trace
  length, and timeouts are all capped before work is admitted.
- **Token hygiene.** The token is random, compared in constant time,
  stored in a private file and Windows current-user storage, and never
  logged. The only command that prints it is the explicit `token`
  command.
- **Calibration gate.** A build is admitted only when the request's
  `calibration_id` is non-empty and exactly equal to the calibration the
  host currently reports for the Solid Edge version, template, binding,
  and probe version. A missing or stale id is a typed refusal before any
  job is created. An uncalibrated bridge reports honest health and
  refuses native builds.
- **Document ownership.** The bridge creates and tags its own documents,
  keeps a ledger of them, and closes only those. It never adopts
  `ActiveDocument`, never calls `Application.Quit`, and never overwrites
  a user file.

Probe and diagnostic JSON are private operator files. They contain host
and version details and must not contain the bearer token.

## Data, artifacts, and the ledger

The data directory defaults to
`%LOCALAPPDATA%\Kala\SolidEdgeBridge` and holds the token, the job
ledger, calibration records, and per-build private run directories. The
layout is owned by the bridge; operators should not edit it. A
user-edited calibration file cannot make a host ready, because readiness
is derived from a recorded probe, not from a JSON flag.

Artifacts are produced inside a private run directory, hashed, and
published atomically. A completed manifest entry is the only way to
resolve an artifact name. The client downloads into
`<output-dir>/builds/<build-id>/` under a safe local name, verifies the
declared size and SHA-256, and only then renames into place. A build id
that is not a safe single directory name (separators, an ADS colon, a
reserved device name, or a trailing dot/space) is refused rather than
written. Server-side paths never appear in a result and are never used
by the client. The journal itself stays at the output root, and each
build's artifacts stay in that build's directory, so two builds that
both produce `plate.step` do not overwrite each other.

The ledger is durable and stdlib-only. It records the normalized script, the
script digest, canonical request digest, normalized bridge target, request ID,
job ID, and terminal status. It does not record credentials. A request id is
reused only for an exact semantic request against the same bridge, so a changed
calibration cannot silently inherit an older job. If the journal holds an
unfinished entry from before exact request identity existed, the client refuses
to submit and tells the operator which id to reconcile instead of risking a
duplicate job. A known-different request (for example a new calibration) is a
new identity and submits normally.

## Doctor and failure modes

`kala-solidedge doctor` is expected to separate these conditions, and
the bridge's typed error codes keep them separate on the wire:

- **Unreachable** — the bridge is not running, or the port is wrong.
  Nothing was submitted.
- **Authentication failure** — a token mismatch. The token is never
  echoed back.
- **Unsupported host** — non-Windows runtime, no interactive session,
  Solid Edge not installed, COM attach refused, or no live-verified
  native build executor on this build (the current state everywhere).
- **Uncalibrated** — Solid Edge is present but this version, template,
  or binding has no matching probe record. Native builds are refused
  with `calibration_required`.
- **Busy** — another build owns the single COM worker. The bridge
  reports `busy` rather than queueing unbounded work.
- **Protocol mismatch** — the client and bridge disagree on
  `protocol_version`; the request is refused rather than reinterpreted.
- **Indeterminate** — a COM call exceeded its deadline, a submission
  acknowledgement was lost, or the worker crashed while the build was
  running. The bridge cannot prove whether the operation completed, so
  the job is marked `indeterminate` and further admission is blocked
  until reconciliation. The client must not treat the timeout as a
  cancellation. On restart, a build that was only queued becomes
  `interrupted` (nothing was submitted, so it is definite), while a
  build that was running becomes blocked `indeterminate`; neither is
  replayed automatically.

## Current implementation status

As of 2026-10-19, and stated without optimism:

- **The contract package is complete and checked on a development host.**
  The protocol, units, and error modules are present under
  `adapters/solidedge-bridge/src/kala_solidedge_bridge/`, with tests
  covering valid part and assembly procedures, malformed ordering,
  unknown fields, bounds, digest determinism, and unit conversion. The
  suite passes without `pywin32` installed. The development host is
  Linux; that does not make Linux a target runtime.
- **The bridge foundation is implemented and checked.** `serve`,
  `probe`, `token`, the STA session, the job ledger, the HTTP surface,
  the worker process, and the artifact manifests exist under the same
  package. The full bridge suite passes (186 passed, 1 gated live test
  skipped) with `ruff check`, `ruff format --check`, and strict `mypy`
  clean. All of that ran on the Linux development host; the live test
  only runs with `KALA_SOLIDEDGE_LIVE=1` on a real Windows desktop.
- **The Windows-local client is implemented and checked.**
  `kala-agent-solidedge` under `adapters/solidedge/` provides
  `kala-solidedge doctor` / `build` / `status` / `cancel`, the compiler,
  the closed-form oracle, and the loopback client. Its suite passes (138
  passed) with clean lint/format/strict mypy. The compile, `--dry-run`,
  help, and platform-refusal paths were exercised on Linux by the CLI
  itself, not only by tests.
- **Core import compatibility is in place.** Importing the shared
  construction contract no longer drags in the Linux executor or the
  strict sandbox. That is a narrow import fix, not a Windows port of the
  sandbox: the strict sandbox itself remains unported.
- **Boundary tests and CI exist.** `tests/unit/test_solidedge_boundaries.py`
  enforces the import directions and the lazy `pywin32` rule, and
  `.github/workflows/solidedge.yml` defines the Ubuntu/Windows checks.
  The hosted workflow has **not** been executed remotely; no Windows CI
  result is claimed here.
- **There is no production native geometry executor.** No handler has
  been written against live-verified Solid Edge signatures and no build
  has produced a real part. The bridge acknowledges a build and then
  fails it with `unavailable`; health never reports a native capability
  it cannot prove.
- **There is no Windows calibration and no feature verification.** No
  Windows host was available. The Solid Edge edition (including whether
  a Student Edition exposes the automation surface) is unknown until an
  actual probe runs.
- **Prompt/Studio wiring is deferred.** Kala Studio does not launch or
  drive this integration, and no shared-CAD tool extraction exists yet.
  There is no UI affordance implying a working native build.
- **Nothing in this document is a readiness claim.** The words
  "supported" and "available" in the health payload mean what the probe
  observed on that host, not what this document hopes. A `--dry-run`
  result is a compile result, not a native build.

### How to check the current state yourself

The commands below work on the checkout as it stands. On the Windows
target they run against a real host; on Linux they exercise the compiler,
the `--dry-run` path, and the honest refusals.

```powershell
# Bridge (from adapters/solidedge-bridge)
python -m pytest -q
python -m mypy
python -m kala_solidedge_bridge --help
python -m kala_solidedge_bridge probe --launch --data-dir "$env:LOCALAPPDATA\Kala\SolidEdgeBridge"
python -m kala_solidedge_bridge serve --port 8797

# Client (from adapters/solidedge)
python -m pytest -q
python -m mypy
python -m kala_agent_solidedge --help
python -m kala_agent_solidedge build --script examples\plate_with_hole.json --dry-run
```

`probe` is the meaningful Windows diagnostic: it reports the platform,
attach state, `Application.Version`, installed type-library metadata,
resolved enum names, and — only when the installed type information
confirms every call — builds, measures, and hash-records a private 10 mm
cube. `probe` exits `0` only after a measured in-tolerance cube with a
signed calibration record; otherwise it exits `2` on a non-Windows host
and `1` for any other failure, and the JSON report lists
`missing_probe_work`. A `--dry-run` build opens no socket and needs no
token.

## Troubleshooting

- `Connection refused`: the bridge is not running in the desktop
  session, or it is serving a different port. Start `kala-se-bridge
  serve` from the interactive session first.
- `401` from every route: the token file is stale. Re-run
  `kala-se-bridge token`, replace the file, and restart `serve`.
- `503` with an unsupported/unavailable detail: the host is not
  attached to an interactive Solid Edge session. Check that Solid Edge
  is installed, licensed, and running in the same desktop session, then
  run `kala-se-bridge probe`.
- `calibration_required`: the probe has not been recorded for this exact
  host/version/template/binding. Run the probe; do not hand-edit a
  calibration file.
- `busy`: wait for the current build, or cancel it cooperatively. Do
  not start a second bridge against the same Solid Edge instance.
- `indeterminate`: stop and reconcile. Do not resubmit the same
  semantic request under a new ID; keep the original ID so the ledger
  can resolve it.
- `protocol mismatch`: install matching versions of the bridge and the
  client. The contract is versioned for this reason.

## References

- `adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py` —
  the versioned wire contract and its procedure invariants.
- `adapters/solidedge-bridge/src/kala_solidedge_bridge/units.py` — the
  millimetre/metre/degree conversions used at the boundary.
- `adapters/solidedge-bridge/src/kala_solidedge_bridge/errors.py` —
  typed failures mapped to stable wire codes.
- `adapters/solidedge/` — the local client (compiler, oracle, HTTP
  client, CLI), when present.
- [construction-semantics.md](construction-semantics.md) — planes,
  offsets, cuts, revolves, bounds, and the shared parity corpus.
- [ADR-0008](adr/0008-solid-edge-windows-bridge.md) — the architecture
  decision, the live-authority exception, and the lifecycle rules.
- [ADR-0007](adr/0007-optional-freecad-cad-cae-fem-adapter.md) — the
  FreeCAD adapter, whose measured geometry and separate-distribution
  pattern this integration follows.
- Siemens Solid Edge automation entry point (older SDK, architectural
  evidence only, not compatibility evidence for a current edition):
  <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>.
