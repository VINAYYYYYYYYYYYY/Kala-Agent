# Documentation cell — report

Status: **complete for the documented milestone, with the Windows native
proof still absent.** Three docs plus this cell's files were written. No
other worker's files were changed.

## Scope correction applied

Mid-task the user corrected the integration scope: **Windows only**. Kala
and Solid Edge run on the same Windows machine; the bridge is a
loopback-local service and the client is Windows-local. The earlier
Linux-agent -> SSH forward -> Windows-CAD split-host plan is **not** the
supported path and is not documented as one.

The docs were rewritten for that correction. Remaining mentions of Linux
are limited to:

- development/CI hosts for the pure-Python contract, explicitly marked as
  not a target runtime;
- the core executor that a narrow import fix must avoid loading on
  Windows;
- the superseded alternative in ADR-0008's "Alternatives Considered".

No SSH command, forwarded-port instruction, remote token-transfer
procedure, or Linux runtime install flow remains.

## Changed paths

- `docs/solidedge.md` — operator-facing architecture, install, commands,
  security posture, data/artifact handling, doctor and failure modes,
  honest current status.
- `docs/construction-semantics.md` — measured planes, offsets, cuts,
  revolves, local-vs-world bounds, assembly occurrences, parity corpus,
  units.
- `docs/adr/0008-solid-edge-windows-bridge.md` — the architecture
  decision, authority exception, lifecycle, alternatives, trade-offs,
  verification, non-claims.
- `.mece/waves/solidedge-20261019/cells/docs/REPORT.md` (this file).
- `.mece/waves/solidedge-20261019/cells/docs/SESSION.md`.

Nothing else was touched. No delegation, commit, push, publish, deploy, or
package installation was performed.

## What the docs now say

### `docs/solidedge.md`

- One Windows machine: Kala/Studio, the client, the bridge, and Solid Edge
  all local. Bridge binds `127.0.0.1:8797` only; no remote transport.
- Three pieces: `kala-solidedge-bridge` (contract + runtime),
  `kala-agent-solidedge` / `kala-solidedge` (local client), and the
  standalone STA COM worker the bridge spawns.
- The bridge is started manually in the interactive desktop session, not
  as a Windows service or scheduled task.
- Public commands: `kala-se-bridge token|probe|serve`, and
  `kala-solidedge doctor|build --script|status|cancel`. The document says
  to verify `--help` and that a command may be absent from a checkout.
- Windows defaults: `%LOCALAPPDATA%\Kala\SolidEdgeBridge`, token file
  under it, `KALA_SOLIDEDGE_BRIDGE_URL` and
  `KALA_SOLIDEDGE_TOKEN_FILE` overrides.
- Security posture: bearer auth on every route, loopback-only bind, no
  browser origin, no arbitrary paths or code, bounded requests, token
  hygiene, calibration gate, document ownership. The sandbox exception is
  stated explicitly rather than implied.
- Status section states plainly: contract tested on a development host;
  bridge runtime and client not present in this checkout; no production
  native executor; no Windows calibration; no feature verification.

### `docs/construction-semantics.md`

Preserved the measured geometry facts and removed the Linux-side framing:

- `XY (u,v,0)` normal `+Z`; `XZ (u,0,v)` normal `-Y`; `YZ (0,u,v)` normal
  `+X`. Numeric data wins over the earlier contradictory prose.
- Offset rectangle `u=2, v=3, w=4, h=5, pad=6`: bounds `(2,3,0)-(6,8,6)`,
  `(2,-6,3)-(6,0,8)`, `(0,2,3)-(6,6,8)`, volume `120`.
- Cut direction is the canonical plane normal, independent of a preceding
  pad's `reversed`. The measured FreeCAD case used `Pocket.Reversed=True`
  to remove `+Z` from an `XY +Z` pad; the contract has no `reversed` field
  on a cut, so a native executor must reproduce the canonical direction
  and report uncalibrated if it cannot.
- Volumes: finite `4` mm hole **`3987.433629` mm³**, through-all
  **`3968.584073` mm³**.
- Rectangle revolve is about the **component-local `Y`** axis on `XY`/`YZ`;
  `XZ` is refused before submission. Circle "revolve" is lowered to an
  extrusion (supplied length, else diameter).
- Local part bounds versus world assembly bounds; duplicate placement
  rejected; empty assembly save refused.
- The wire contract permits several additive features per part, but the
  measured parity corpus covers only a single primary solid plus a
  same-plane hole. Additive features are called out as **unchecked**, not
  as passing.

### `docs/adr/0008-solid-edge-windows-bridge.md`

- Status **Proposed**, consistent with ADR-0001..0007. User authorization
  to implement is not recorded as stakeholder acceptance of policy.
- D1 two distributions, core imports neither; the bridge is the only
  place `pywin32` is imported, lazily.
- D2 local only: loopback bind, bearer on every route, no remote
  transport, manual desktop start.
- D3 one STA worker process with the explicit apartment value `2`; HTTP
  threads never call COM.
- D4 lifecycle: `indeterminate` on deadline, cooperative cancel that does
  not claim a COM call stopped, `interrupted` on restart with no replay,
  no `Application.Quit`, owned-document cleanup only.
- D5 authority is compensated, not silently widened. The strict sandbox
  is not ported to Windows; the ADR says so and lists the compensating
  controls.
- D6 measured construction semantics; v1 scope.
- D7 readiness is probe evidence from the actual host.
- D8 development may happen anywhere; target runtime is Windows.
- D9 Studio/shared-CAD integration deferred behind real-native evidence.
- Alternatives record the superseded split-host design, Wine/VM, service
  or scheduled-task hosting, reusing the kernel sandbox, porting the
  sandbox, core tool registration, accepting scripts/paths, killing Solid
  Edge, and inventing signatures.
- Verification reports only what was actually run, and separates it from
  what is unverified.

## Checks actually run

- `cd adapters/solidedge-bridge && ../../.venv/bin/python -m pytest -q`
  — **87 passed** (the contract worker's repaired suite; docs quote 87,
  not the earlier 82).
- `../../.venv/bin/python -m ruff check .` — **All checks passed**.
- `../../.venv/bin/python -m ruff format --check .` — **6 files already
  formatted**.
- `../../.venv/bin/python -m mypy src` — **Success: no issues found in 4
  source files**.
- Consistency passes over the three docs: relative links resolve; no SSH
  or forwarded-port instructions remain; cut direction, revolve axis,
  additive-feature allowance, and artifact/operation caps were re-read
  against the current `protocol.py` after it changed mid-task.
- Confirmed the tree contains only the contract modules under
  `adapters/solidedge-bridge/src/kala_solidedge_bridge/` — no
  `server.py`, `session.py`, `jobs.py`, `auth.py`, `workspace.py`,
  `calibration.py`, `__main__.py`, and no `adapters/solidedge/`. The docs
  describe those as intended layout, not as verified files.

## Limitations and blockers

- **No Windows host, no Solid Edge installation, no COM attach, no
  type-library read, no probe, no calibration, and no native build.** The
  docs do not claim otherwise. The installed edition and whether it
  exposes the automation surface (including Student Edition limits) are
  unknown until an actual probe runs.
- **No production native geometry executor exists.** The bridge can carry
  a procedure and report honest status; it cannot yet drive real Solid
  Edge features. The docs say native builds stay unavailable until a
  live-calibrated executor exists.
- **The bridge runtime and the client are not in this checkout.** Their
  command surfaces are documented as intended, with an explicit
  instruction to run `--help` rather than assume.
- **Core import compatibility is another worker's narrow fix.** The docs
  describe the requirement (contracts/Studio importable on Windows without
  loading the Linux executor) and do not claim it is done.
- **The geometry expectations are FreeCAD 1.1.4 measurements**, not Solid
  Edge observations. They are stated as the parity target, not as
  reproduced behaviour.
- **No tests were added** — this cell owns documentation only. The test
  count above is the contract package's, run to make sure the docs quote
  current numbers.

## Session

Transport: T3 OpenCode delegation. Model:
`opencode/deepseek-v4.1-flash-free`. The child-thread ID is not an
OpenCode session ID, and none was reported.
