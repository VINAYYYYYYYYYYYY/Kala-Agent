# ADR-0008: Local Windows Solid Edge Automation Through a Loopback Bridge

| Field | Value |
|---|---|
| Status | **Proposed** — implementation was explicitly authorized by the user, but this record does not declare accepted policy. A Proposed decision stays Proposed until a stakeholder accepts it. |
| Date | 2026-10-19 |
| Last reviewed | 2026-10-19 |
| Scope | A Windows-local agent driving a Windows-only commercial CAD application through a separate, authenticated, out-of-process loopback bridge |
| Requirements addressed | A new optional integration requirement (basic parts and positioned assemblies in Solid Edge) and the honesty, authority, and provenance obligations that come with it. It satisfies no existing MVP acceptance criterion on its own. |
| Related ADRs | ADR-0001 (local-first, no egress), ADR-0004 (immutable revisions, journaled artifacts), ADR-0005 (capability-based tool contracts, isolated processes), ADR-0006 (package trust, sandboxing, provenance), ADR-0007 (optional FreeCAD adapter, separate distribution) |
| Implementation status | **First milestone implemented and locally checked; no native capability.** The contract distribution `kala-solidedge-bridge` exists; the bridge runtime (`serve`, `probe`, `token`, the STA worker, the job ledger, the HTTP surface) and the Windows-local client `kala-agent-solidedge` (compiler, oracle, client, CLI) are implemented under `adapters/solidedge-bridge` and `adapters/solidedge` and pass their local checks. **Not implemented: a production native geometry executor, any Windows calibration, and any live feature verification.** No Windows host was available. No Solid Edge edition was probed. |
| Verification | Contract+bridge package: 186 passed, 1 gated live Windows test skipped; `ruff check`, `ruff format --check`, and strict `mypy` clean. Client package: 138 passed; `ruff`/format/strict `mypy` clean. Root boundary and portable-import tests: 16 passed, and the two Linux-core identity tests are declared Linux-only because the core executor is unported. That is a statement about a pure-Python foundation, not about Solid Edge. The hosted Windows workflow is defined but was not executed remotely. **There is no native verification to report.** See "Verification" below for the exact limits. |

## Context

Kala and Solid Edge both run on Windows, on the same machine. Solid
Edge is a licensed desktop application whose automation surface is COM.
Kala is the agent and workspace. A user who owns a Solid Edge seat has a
real workflow: describe a part in Kala, get a real Solid Edge part and
STEP file, keep working in Solid Edge.

The integration is worth doing, and it is not the same problem as the
FreeCAD adapter (ADR-0007). The differences that drive this decision:

1. **COM is apartment-bound and desktop-bound.** Automation needs a
   single-threaded apartment with a pumping message loop, and it needs
   the interactive window station. A Windows service or a scheduled
   task has no desktop to attach to. The process must be started by the
   logged-in user, in their session.
2. **Solid Edge is single-instance and user-owned.** Attaching to
   `SolidEdge.Application` may reach the application the user already
   has open. The bridge can own a *document*, but it cannot own the
   application, and it must never close, save over, or quit anything the
   user owns.
3. **Kala's sandbox does not fit.** ADR-0005's isolation model is a
   short-lived Linux subprocess with a probed jail. There is no way to
   put a COM client for a logged-in desktop application inside that
   jail, and this milestone does not port the strict sandbox to Windows.
   Pretending otherwise would be the worst outcome: a control that is
   documented but not enforced.
4. **A commercial CAD seat is a user asset.** A failed build must be
   recoverable without the user losing work, and the integration must
   never leave orphaned documents behind.
5. **The agent must not become a remote-code surface for the desktop.**
   Even on one machine, an unauthenticated local port is reachable by
   every process and every web page that can reach loopback.

The question this ADR answers is therefore not "how do we call Solid
Edge" but **"what authority does the integration hold, where does that
authority live, and what compensates for the sandbox it cannot use?"**

## Decision

**D1 — The integration ships as two separate distributions, and Kala core
imports neither.**

- `kala-solidedge-bridge` (package `kala_solidedge_bridge`) owns the
  wire contract and the bridge runtime. It does not depend on
  `kala-agent`. Its dependencies are `pydantic` and a Windows-marked
  `pywin32`.
- `kala-agent-solidedge` (package `kala_agent_solidedge`, command
  `kala-solidedge`) owns the Windows-local client: the
  `ConstructionScript` compiler, the closed-form oracle, the HTTP client,
  and the CLI.
- Kala core's dependency list, tool catalog, HTTP routes, and CLI are
  unchanged. `KalaApplication` does not discover, register, or dispatch
  this integration.

The bridge is the only place `pywin32` is imported, and it is imported
lazily inside the session module. Importing the contract must never load
`pythoncom`, `win32com`, or `pywintypes`; a boundary test enforces that
direction, as ADR-0007's AST test enforces the FreeCAD direction.

**D2 — Everything is local. The bridge binds loopback only and requires
authentication on every route.**

`kala-se-bridge serve` binds `127.0.0.1`. Every route, including
`GET /v1/health`, requires a bearer token. The server validates the Host
authority and port, rejects `Origin` headers (this is not a browser
API), rejects forwarded headers as authority, and rejects chunked or
malformed bodies before reading them. There is no unauthenticated
endpoint, no static file server, and no upload route.

There is no remote transport in the supported design. The integration
does not require SSH, a second machine, or an exposed port, and this
document does not describe one as a supported path. A loopback bind is
a security property, not an implementation detail: it means the bridge
is not reachable from the network even if the machine is.

The process is started manually from the desktop, not by a service
manager and not by a scheduled task. Neither has an interactive desktop,
so neither can attach to Solid Edge.

**D3 — COM runs in exactly one STA worker process, and HTTP threads
never call COM.**

The bridge spawns one child process that calls
`CoInitializeEx(NULL, COINIT_APARTMENTTHREADED)` with the explicit value
`2`, matches it with `CoUninitialize`, and pumps messages while idle.
The value is written explicitly rather than inherited from a default,
because a default of `0` is `COINIT_MULTITHREADED`, and an MTA is the
wrong apartment for a desktop application's automation objects.

The HTTP server is a stdlib threaded server, but it never touches COM.
It validates, admits, persists, and reads ledger state. Build work is
handed to the worker. This is what keeps a hung COM call from hanging
health, and what makes the single-worker busy state meaningful.

**D4 — Lifecycle is honest, and "cancelled" is not "stopped".**

- A build has a supervisor deadline. When it expires, the job is marked
  `indeterminate`: the bridge cannot prove whether the COM call
  completed. Further admission is blocked until the state is
  reconciled. The client must not treat the timeout as a cancellation
  and must not resubmit the same semantic request under a new ID.
- A submission whose acknowledgement is lost, or whose worker crashes
  while the build is `running`, is `indeterminate` for the same reason:
  the child may already have started a native operation. Only a backend
  that positively proves it never started the build may report a
  definite failure.
- Cancellation is cooperative between operations. It is **not** a claim
  that a COM call already in progress has stopped, and it is **not** a
  claim that Solid Edge stopped working. Nothing in this design kills
  Solid Edge to enforce a cancel.
- On restart, a job that was still `queued` and had never been handed to
  the worker becomes `interrupted` and does not block new work. A job
  that was `running` may have an external COM operation in flight, so it
  becomes `indeterminate` and blocks admission until it is reconciled.
  Nothing is ever replayed automatically, because replaying a mutation
  that may have partially happened is how a user gets a duplicated or
  corrupted part.
- The bridge never calls `Application.Quit`. It closes only documents it
  created and tagged, using its own ledger. It never adopts
  `ActiveDocument` and never saves over a user file.

**D5 — Authority is compensated, not silently widened.**

The bridge runs outside Kala's sandbox. ADR-0005's strict profile is
unavailable for a COM client of a desktop application, and this ADR
records that as an explicit exception rather than reusing a profile that
would not be enforced. The strict sandbox is **not** being ported to
Windows for this milestone. The compensating controls are:

- **Authentication.** A random bearer token on every route, compared in
  constant time, stored in a private file and Windows current-user
  storage, and never logged. The one command that prints it is the
  explicit `token` command.
- **Schema.** Requests contain only typed operations and bounded
  dimensions. There is no path, URL, script, executable, document
  identifier, or code field. Unknown fields are rejected.
- **Refusal before transport.** The client compiler rejects unsupported
  features before opening a socket, so an unsupported request never
  reaches the bridge.
- **Document ownership.** The bridge creates and tags its own documents,
  keeps a ledger, and cleans up only those. It never touches user
  documents.
- **Journaling.** The job ledger is durable and records script digest,
  request ID, request digest, job ID, and terminal status. It records no
  credentials. Artifacts are published atomically and resolved only
  through completed manifest entries with size and digest checks.
- **Boundedness.** Request and response sizes, procedure length, part
  count, artifact count and size, trace length, and timeouts are capped
  before work is admitted.
- **Calibration.** Native builds are admitted only when a probe record
  matches the host's Solid Edge version, template, binding, and probe
  version. A user-edited JSON file cannot make a host ready.

**D6 — The shared construction semantics are measured, not invented.**

The client compiles the existing `ConstructionScript`, not a new
language. The plane mappings, offsets, cut direction, revolve behaviour,
and local-versus-world bounds are pinned to measured FreeCAD 1.1.4
values in [construction-semantics.md](../construction-semantics.md).
Where earlier plan prose contradicted its own numeric data, the numbers
win: **XZ is `-Y`**.

The v1 operation set is deliberately narrow: basic parts (a primary
solid from a rectangle or circle profile, finite or through-all cuts,
additive features allowed by the wire contract but outside the measured
corpus), rectangle revolve on `XY`/`YZ` about the component-local `Y`
axis, and positioned assemblies with one occurrence per component.
Helix, threads, fillets, fuse, compound, FEA, and an in-Solid-Edge
add-in are out of scope and refused.

**D7 — Readiness is evidence from the actual host, never a constant or
a plan.**

`Health` reports `supported`, `available`, `busy`, version,
calibration, capabilities, and a bounded detail. Capability names are
not a readiness claim. A new build is refused unless the request's
`calibration_id` is non-empty and exactly matches the calibration the
host currently reports; a missing id is a typed `calibration_required`
refusal and a stale id is a typed `conflict` refusal, both before any
job file or queue entry exists. An identical replayed request still
returns its existing job even when the host has since re-probed, because
the bridge must not turn a retry into a second build or re-execute the
first. Until a production executor exists and is live-verified, native
build capability stays unavailable and the bridge fails closed rather
than emitting plausible geometry.

**D8 — Development may happen anywhere; the target runtime is Windows.**

The contract and the client are testable on a development host, and the
suite runs there in CI. That is a convenience for development and does
not make a non-Windows runtime supported. Any command that would perform
a native operation must fail usefully off Windows rather than pretend to
succeed. The public `kala-se-bridge serve` command refuses a non-Windows
host with exit code `2` **before** it would resolve a default data
directory or create a token file; the test seam
(`create_server` with an injected backend) remains usable in tests on
any host and is not a supported runtime path.

**D9 — Studio and shared-loop integration are deferred.**

A Kala Studio launcher and a shared CAD extraction are a plausible next
step, but they are gated on deterministic real-native evidence. They are
not part of this milestone, and nothing here should be read as
authorizing a UI that implies a working native integration before that
evidence exists. When Studio integration arrives, it should use an
injected neutral backend and an explicit authenticated route group, not
a core adapter import.

## Consequences

### Positive

- Kala gains a path to a real commercial CAD seat without gaining a
  dependency on the core, a core import, or a core tool registration.
- The contract is testable without a Windows host, which means most of
  the boundary logic is verified before native work begins. That is the
  part that can be wrong in a way tests catch.
- The sandbox exception is explicit. An operator can read exactly what
  authority the bridge holds and what compensates for it, instead of
  inheriting a control that would not be enforced.
- The single-worker, single-apartment model makes the concurrency story
  trivial: one build at a time, one application instance, one ledger.
- The measured semantics document makes a wrong plane, a mirrored
  extrusion, or a misplaced assembly a detectable mismatch rather than
  an argument.
- A loopback-only, authenticated, local design has a small trust
  boundary. There is no remote transport to secure and no exposed port
  to firewall.

### Negative

- **The sandbox is not there.** The bridge is a trusted local service
  with access to the user's CAD application. The compensating controls
  are real, but they are not the same guarantee as an enforced jail, and
  this ADR does not pretend they are.
- **Cancellation is weak by construction.** A hung COM call can leave a
  job `indeterminate` until a human reconciles it. That is honest and it
  is also a worse experience than a hard kill, which the design
  deliberately refuses because the hard kill would target Solid Edge.
- **No Windows evidence.** The most important claims — that an edition
  exposes the automation surface, that a part builds, that a STEP file
  is correct — are unverified. Until they are verified, the feature is a
  foundation, not a capability.
- **Two packages and a wire protocol to keep in step.** A protocol
  version mismatch is a new failure mode, mitigated by versioning but
  not removed.
- **A Windows-only target narrows the audience.** A Linux user cannot
  run this integration natively, and this ADR does not offer a
  workaround as a supported path.

### Neutral

- The contract is versioned at `1`. A future change that is not
  backward compatible requires a new version, not a reinterpretation of
  the old one.
- `pywin32` is a Windows-marked dependency. It is never installed on a
  non-Windows development host, and importing the contract never loads
  it.
- The data directory is `%LOCALAPPDATA%\Kala\SolidEdgeBridge` on
  Windows. Probe and diagnostic JSON are private operator files and
  contain no bearer tokens.
- The oracle is closed-form and intentionally limited. An unchecked case
  is reported as unchecked, never as a pass.
- A narrow core import fix lets the client and Studio import the shared
  construction contract without loading the Linux executor. That is an
  import-boundary change, not a sandbox port.

## Alternatives Considered

**Keep the earlier split-host design: Linux agent, Windows CAD seat,
SSH forward.**

- Superseded. The user's current requirement is a Windows-only, local
  integration. A remote transport adds a second machine, an SSH
  configuration, a forwarded port, and a trust boundary that this
  product does not need. It is not documented as a supported path, and
  the bridge does not require it.

**Run the whole thing on Linux under Wine or a Windows VM.**

- Rejected as a product path. A commercial CAD seat under Wine is not a
  supported configuration, licensing and GPU behaviour are uncertain,
  and COM-in-Wine is not the same automation surface. A VM is a
  legitimate operator choice, and it is the operator's own environment,
  not something this integration should assume or document as
  supported.

**Run the bridge as a Windows service or scheduled task.**

- Rejected. Neither has an interactive desktop, so Solid Edge cannot
  attach. A service that claims to work would fail at the first
  automation call, and a scheduled task that "works" would be running in
  a session the user cannot see.

**Reuse the kernel's `SubprocessToolHandler` and strict sandbox for the
bridge.**

- Rejected as a category error. The kernel's handler starts a
  short-lived Linux subprocess from a registered tool. There is no such
  subprocess here: the CAD application is a desktop process with a COM
  apartment, and the sandbox is not being ported to Windows. Reusing the
  type would either be a lie about isolation or a reimplementation of
  the executor, both worse than an explicit exception.

**Port the strict sandbox to Windows.**

- Rejected for this milestone. It is a large piece of work with a
  different control model, and doing it badly would be worse than
  recording the exception. The compensating controls above are the
  interim position, and a Windows sandbox is a Revisit Trigger.

**Expose the bridge as a Kala tool in the core registry.**

- Rejected for this milestone. It would put a Windows-only integration
  on the product's tool path and change the approval surface for every
  user. ADR-0007's D9 reasoning applies unchanged.

**Let the bridge accept a script, a path, or a document name.**

- Rejected outright. Any of those turns the bridge into a local code or
  file access surface for a machine holding the user's CAD seat. The
  contract carries operations and bounded dimensions only.

**Kill Solid Edge to enforce a cancel or a timeout.**

- Rejected. The user's Solid Edge session is not the bridge's to kill. A
  kill can lose unsaved user work and can leave the automation state
  worse than an `indeterminate` job. The bridge reports what it knows and
  stops admitting work until a human reconciles.

**Invent native signatures from an older SDK and ship them.**

- Rejected. An older SDK is architectural evidence, not compatibility
  evidence for the installed edition. Guessed signatures produce wrong
  geometry or a crashed session, and a probe that guesses measurements
  is worse than a probe that reports `calibration_required`.

## Trade-offs

- We accept **a Windows-only target** in exchange for a local, small
  trust boundary and no remote transport to secure.
- We accept **an explicit sandbox exception** in exchange for not
  documenting a jail that cannot be enforced. The compensation is
  authentication, schema, refusal before transport, ownership, and
  journaling, and it is weaker than the jail.
- We accept **one build at a time** in exchange for a COM model that is
  actually correct. Parallel builds against one desktop application
  would need a second application instance, which is a different
  decision.
- We accept **cooperative cancellation and `indeterminate` outcomes** in
  exchange for never killing the user's CAD session.
- We accept **a narrow v1** in exchange for a subset that can be measured
  and refused precisely. Threads and fillets are the obvious missing
  features, and they are refused rather than approximated.
- We accept **no native evidence at this milestone** in exchange for not
  fabricating it. The feature is unavailable until a Windows probe and a
  live feature test say otherwise.

## Revisit Triggers

- A real Windows host with a known Solid Edge edition becomes available.
  That is the trigger to record calibration and attempt the private cube
  probe, then a minimal part build.
- The installed edition turns out not to expose the automation surface
  (for example, a Student Edition limitation). That would change the
  supported-edition statement and possibly the whole approach.
- A Windows sandbox with an enforceable control model becomes
  available. That would replace the compensating-controls exception.
- A second CAD integration appears that wants the same bridge shape.
  That is the point to extract a neutral local-CAD contract rather than
  duplicate the protocol.
- The kernel gains a first-class embedding surface. The bridge would then
  be registered through that surface rather than remaining entirely
  outside the product.
- A supported cancellation mechanism exists that stops a COM call
  without killing the application. That would replace the cooperative
  model and the `indeterminate` window.
- Studio integration is proposed. It must bring deterministic
  real-native evidence, not a launcher around an unverified bridge.
- Solid Edge exposes a stable, documented, versioned automation
  interface that is materially easier to target than the current SDK
  surface. That would be a reason to revisit the probe and signature
  strategy.

## Verification

Status as of 2026-10-19. This section is deliberately short, because
almost nothing about Solid Edge itself is verified.

**What was actually checked:**

- The bridge distribution `adapters/solidedge-bridge` runs its suite
  without `pywin32` installed: **186 passed, 1 skipped** (the skipped
  test is the explicitly gated live Windows attach test). The contract's
  original tests are included and still pass. The suite covers the
  procedure validator, digest determinism, bounds, the STA retry
  classification, lazy imports, the authenticated HTTP surface and its
  hostile-header/body-limit paths, the job ledger (calibration equality
  at admission, idempotent replay, ambiguous submission and crash
  `indeterminate`, restart distinction, cancellation, restart), the
  probe with an injected fake COM seam, the child-process worker, and
  the CLI. `ruff check`, `ruff format --check`, and strict `mypy` are
  clean.
- The client distribution `adapters/solidedge` passes **138 tests** with
  `ruff`/format/strict `mypy` clean: compiler refusals, oracle errors,
  loopback URL guards, redirect/oversize/hash/ambiguity transport
  paths, the health preflight, journal identity, oracle exit codes, and
  artifact-directory safety. The CLI `--help`, `--dry-run`, `doctor`,
  and the non-Windows refusals were also run directly through the module
  entry point on Linux.
- Root boundary tests (`tests/unit/test_solidedge_boundaries.py`) and the
  portable-import regression tests pass on the Linux development host.
  Importing the bridge protocol loads no `pywin32` module and no
  `kala_agent` module; importing the client compiler loads no `pywin32`
  module and does not import the Linux application/tool plane.
- Probe behavior is verified against an in-process fake COM seam only:
  the fake reports measurements, and the probe's tolerance, hash, and
  cleanup logic is asserted. A fake is not evidence about Solid Edge.

**What was not checked, and must not be claimed:**

- No Windows host, no Solid Edge installation, no COM attach, no
  type-library read, no `Application.Version`, and no interactive-session
  probe. The edition and its automation surface are unknown.
- No native part, assembly, `.par`, `.asm`, or STEP file was produced by
  Solid Edge.
- No calibration was recorded, and no health payload from a real host
  was observed.
- The hosted Ubuntu/Windows workflow (`solidedge.yml`) is defined but was
  not executed remotely; no Windows CI result is claimed.
- No timeout, cancel, restart, or `indeterminate` path was exercised
  against real COM. Those behaviours are designed and unit-tested at the
  seam, not observed natively.

## What this ADR does **not** claim

- It does not claim Solid Edge builds parts. There is no production
  native executor and no live feature verification.
- It does not claim the sandbox exception is safe in general. It claims
  the exception is recorded, bounded, and compensated, and that the
  compensation is weaker than an enforced jail.
- It does not claim a supported Solid Edge edition. The probe decides
  that on the actual host.
- It does not claim cross-bitness compatibility in either direction.
  COM here is out of process, and the probe decides what works.
- It does not claim the contract is complete for all CAD work. It is
  complete for the narrow v1 subset, and it refuses the rest.
- It does not claim a remote or split-host configuration is supported.
  The target is one Windows machine.
- It does not accept the decision. Status is **Proposed**; user
  authorization to implement is not stakeholder acceptance of policy.

## Implementation notes (2026-10-19)

The module names below are the layout owned by the bridge and client
cells. All of them are present in this checkout and were exercised by
their local suites on a Linux development host; no row is evidence of
Windows or Solid Edge behavior. Inspect the tree and run `--help` rather
than assuming.

| Decision | Implementation | Reference |
|---|---|---|
| D1 separate distributions | `adapters/solidedge-bridge/pyproject.toml` declares `kala-solidedge-bridge` with `pydantic>=2.10,<3` and `pywin32>=306; sys_platform == 'win32'`, no `kala-agent` dependency. `adapters/solidedge/pyproject.toml` declares `kala-agent-solidedge` with console script `kala-solidedge`. | both `pyproject.toml` files |
| D1 host-safe contract | `kala_solidedge_bridge/__init__.py` imports only `.protocol`; `units.py` and `errors.py` are pure Python and import no Windows module. | `src/kala_solidedge_bridge/` |
| D2 loopback + bearer | `server.py` (bridge worker): stdlib `ThreadingHTTPServer`, bind `127.0.0.1`, bearer on every route, `Origin` rejected, Host/port validated, body caps before read. | `src/kala_solidedge_bridge/server.py` |
| D3 explicit STA | `session.py` (bridge worker): lazy `pywin32` import, `CoInitializeEx(..., COINIT_APARTMENTTHREADED)` with the explicit value `2`, matched `CoUninitialize`, idle message pump. | `src/kala_solidedge_bridge/session.py` |
| D4 lifecycle | `jobs.py` (bridge worker): durable ledger, request-ID/payload-digest idempotency, one active job, cooperative cancel, `indeterminate` on deadline / lost acknowledgement / crash, `interrupted` on restart only for a never-submitted queued job (a running job becomes blocked `indeterminate`), no replay, no `Application.Quit`. | `src/kala_solidedge_bridge/jobs.py` |
| D5 compensation | `auth.py` / `workspace.py` / `calibration.py` (bridge worker): private token file, constant-time compare, bounded artifact resolution, calibration keyed to host version/template/binding/probe version. | `src/kala_solidedge_bridge/` |
| D6 measured semantics | [construction-semantics.md](../construction-semantics.md) pins planes, offsets, cut direction, revolve lowering, and local/world bounds to measured FreeCAD 1.1.4 values. | `docs/construction-semantics.md` |
| D7 evidence-gated readiness | `Health` carries `supported`, `available`, `busy`, `calibration_id`, and bounded `capabilities`; `calibration_required` and `unavailable` are distinct typed errors. | `src/kala_solidedge_bridge/protocol.py` |
| D8 Windows target | Native operations require Windows; a non-Windows run fails usefully. Development and CI may run elsewhere. | `__main__.py`, client CLI |
| D9 deferred Studio work | No launcher, no shared-CAD extraction, no Studio import in either package. | — |

The measured geometry reference, quoted from the FreeCAD 1.1.4 run:

```text
XY  (u, v, 0)   normal +Z
XZ  (u, 0, v)   normal -Y
YZ  (0, u, v)   normal +X

offset rectangle: u=2, v=3, width=4, height=5, pad=6
XY  bounds (2, 3, 0) - (6, 8, 6)
XZ  bounds (2, -6, 3) - (6, 0, 8)
YZ  bounds (0, 2, 3) - (6, 6, 8)
volume 120 mm3 on every plane

pocket: 20x20 pad 10 on XY, diameter 2 hole at (5, 5)
finite depth 4   volume 3987.433629 mm3
through-all      volume 3968.584073 mm3
```

## References

- `adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py` —
  the versioned wire contract, procedure invariants, and digest.
- `adapters/solidedge-bridge/src/kala_solidedge_bridge/units.py`,
  `errors.py` — conversions and typed wire failures.
- `adapters/solidedge-bridge/pyproject.toml` — the bridge distribution.
- `adapters/solidedge/` — the local client, when present.
- `docs/solidedge.md` — operator-facing architecture, setup, and
  commands.
- `docs/construction-semantics.md` — planes, offsets, cuts, revolves,
  bounds, and the shared parity corpus.
- ADR-0001, ADR-0004, ADR-0005, ADR-0006, ADR-0007 — the local-first,
  journaling, isolation, trust, and adapter precedents this decision
  builds on.
- `src/kala_agent/contracts/construction.py` — the shared construction
  script the client compiles.
- Siemens Solid Edge automation entry point (older SDK; architectural
  evidence only, not compatibility evidence for a current edition):
  <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>.
