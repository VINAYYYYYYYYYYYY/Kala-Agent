# Cell report — AGENT-DOCS (chat-first Mechanical Engineering Agent)

Cell: `delivery` (docs). Wave: `studio-20261018`.
Brief: `cells/delivery/TASK-AGENT-DOCS.md`.
Ownership: docs-only. No `src/**`, `studio/**`, `packaging/**`, or
`.github/**` file was modified.

## What was requested

Reframe Kala Studio's documentation as a **Chat-First Mechanical
Engineering Agent** with an integrated, default-active **OpenCascade
(OCP) Geometry, Estimation & Neutral-File Handoff Workbench**, keep
the honesty/security documentation intact, remove any hobbyist/pro
discrimination, and document the nominal analytical geometry
estimates.

## Files changed

| Path | Change |
|---|---|
| `README.md` | Rewritten: chat-first framing, "Chat-first agent with an integrated geometry workbench" section (two workflows, non-discrimination statement), "Nominal geometry estimates" table, honesty disclaimer updated. |
| `docs/studio.md` | Rewritten: chat-first "What Kala Studio is", new "The agent chat workspace" and "The OpenCascade (OCP) geometry, estimation & neutral-file handoff workbench" sections (including the estimate table and formulas), honest capabilities reframed, route contract, shell, sources and "does not ship" list preserved. |
| `docs/studio-limitations.md` | Targeted edits: scope block reframed, new "Nominal geometry estimates" subsection, new "Nominal estimates are not analysis" engineering-safety bullet. All existing security, provider, packaging, and testing content preserved. |
| `docs/studio-providers.md` | Targeted edits: scope line reframed; chat-request clause adds that no estimate is attached; local helper documents dimension extraction and nominal estimates; "What the studio never does" adds that no geometry/estimate is sent to a provider. |
| `.mece/waves/studio-20261018/cells/delivery/REPORT-AGENT-DOCS.md` | This report (new). |

## Requirements coverage

1. **Chat-first framing.** `README.md` and `docs/studio.md` now open
   with Kala Studio as a conversational Mechanical Engineering Agent
   whose chat workspace is the primary focal point; the workbench is
   described as default-active alongside it.
2. **Workbench role.**
   - External CAD/CAE (`SOLIDWORKS`, `FreeCAD`, `Fusion`, `ANSYS
     Mechanical`, `Abaqus`, `COMSOL`, etc.): parametric geometry,
     bounding-envelope, and volume/area estimator, plus neutral
     `STEP` (`ISO-10303-21`) / `STL` handoff generator, before or
     alongside full feature modeling.
   - Standalone: OpenCascade (`STEP` / `STL`) plus the analytic 3D
     preview is the **default complete geometry builder** out of the
     box.
3. **Non-discrimination rule.** No hobbyist/pro or amateur/professional
   language exists in any of the four docs; both are explicitly a
   single unified workflow. Verified with a case-insensitive scan for
   `hobby|amateur|professional|\bpros?\b|beginner|expert` across the
   four files — no matches.
4. **Honesty/security preserved.**
   - `docs/studio-limitations.md`: loopback/token posture, unsigned
     packaging trust, provider honesty, and testing procedure are
     untouched; the analysis-solver and native-connector statements
     are unchanged.
   - `docs/studio-providers.md`: the fail-closed `codex` route, fixed
     `openai`/`openrouter` endpoints, key-never-in-renderer, and
     `allow_remote` consent gate are unchanged.
   - No doc claims live COM/IPC automation of proprietary CAD/CAE or
     fabricated FEA/CFD results.
5. **Nominal geometry estimates documented** with units — volume
   (`mm³`, `cm³`), surface area (`mm²`, `cm²`), bounding envelope
   (`mm`), and envelope volume (`mm³`) — including the closed-form
   formulas, the nominal unit conversions (`1 cm³ = 1000 mm³`,
   `1 cm² = 100 mm²`), and the explicit statement that they are
   ideal-primitive sizing aids, not solver output.

## Grounding in real code

The estimate claim is not aspirational; the current working tree
implements it, so the docs describe shipped behaviour:

- `studio/src/viewer/localPrimitives.ts:452` exports
  `GeometryEstimate` / `computeGeometryEstimate` (box `xyz` /
  `2(xy+yz+zx)`; cylinder `πr²h` / `2πr(r+h)`; sphere `(4/3)πr³` /
  `4πr²`) with mm³/cm³ and mm²/cm² conversions and `envelopeVolumeMm3`.
- `studio/src/components/inspector/Inspector.tsx:139` renders the
  "OpenCascade estimate (nominal)" readout (Volume, Surface area,
  Bounding envelope, Envelope volume).
- `studio/src/components/chat/ChatPanel.tsx` shows the active geometry
  context beside the chat input; the request body still carries only
  `{provider, model, prompt, allow_remote}` — enforced by
  `src/kala_agent/studio/schemas.py` (`extra="forbid"`).
- `src/kala_agent/studio/providers/local.py` extracts dimensions from
  the prompt, computes the same nominal estimates, and restates the
  unified OpenCascade workflow ("One shared workflow for every user,
  without tiers").
- `src/kala_agent/studio/geometry/step.py` writes exact B-rep STEP via
  native OCP `BRepPrimAPI_*` + `STEPControl_Writer`, and
  `docs/studio-limitations.md` records the `BRepGProp` `1e-7` relative
  round-trip confirmation from `../OCP-VERIFIED.md`.
- `src/kala_agent/studio/schemas.py` preview responses return the
  server mesh plus `bounds`; the envelope is re-derived from that
  mesh, which the docs state as "server-verified".

I did **not** edit any of the above code; it is cited only to show the
docs match the implementation.

## Checks performed

| Check | Result |
|---|---|
| Profile-label scan on the four docs | No `hobby/amateur/professional/pros/beginner/expert` matches. |
| Cross-reference scan | README links (`studio.md`, `studio-install.md`, `studio-providers.md`, `studio-limitations.md`) all resolve to existing files. |
| Claim audit against source | Estimate table, local-helper behaviour, request schema, and STEP/BRepGProp statements each traced to the file/line listed above. |
| Markdown structure | Headings, tables, and code fences reviewed after edit; UTF-8 (`³`, `²`, `×`, `π`, `—`) preserved. |
| Scope | Only the four docs and this report were edited by this cell; no code, packaging, workflow, or `studio/**` write was issued. |

## Notes and residual risk

- The docs describe the workbench estimate readout as **nominal**
  ideal-primitive values, not a simulation. This is deliberate and
  matches the code comments and the limitations file.
- It is now the case that both the UI and the backend local provider
  compute the estimates; the docs say the request body carries no
  estimate, which the frozen request schema enforces. If a future wave
  attaches geometry to a provider call, that sentence must change.
- No behavioural change was requested or made; this was a
  documentation-only pass. No test run was required to validate prose,
  and none was run.

## Did not do

- No code, `packaging/**`, `.github/**`, `src/**`, or `studio/**`
  edits.
- No commit, push, deployment, or credential access.
