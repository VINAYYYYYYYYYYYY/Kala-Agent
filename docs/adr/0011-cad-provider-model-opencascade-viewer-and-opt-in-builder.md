# ADR-0011: CAD Provider Model: External CAD Builds, OpenCascade Views (Builds Only on Request)

| Field | Value |
|---|---|
| Status | **Proposed**. Direction set by the user on 2026-10-19: "the cad kernel here is just for rendering when the final model is done and the model is imported from the external provider … opencascade is used here to just view, no modify … except when the user wants it to be rendered in the opencascade". |
| Date | 2026-10-19 |
| Amends | ADR-0009 (Studio design turns): the built-in OCP compiler becomes ONE opt-in provider, not the default. ADR-0010 (Rust control plane): the geometry backend serves the viewer import plus the opt-in OpenCascade provider. |

## Decision

1. **Kala is the orchestrator, not the modeller.** The agent turns a request into a `ConstructionScript` plus declared intent (material, tolerances). A **CAD provider** builds the model.
2. **Providers** (same `ConstructionScript` contract, same result shape):

   | Provider | Availability | Builds and measures | Delivers |
   |---|---|---|---|
   | `solidedge` | Windows with Solid Edge, via the ADR-0008 bridge | Solid Edge | Native part, STEP export, Solid Edge mass properties |
   | `freecad` | Any OS with FreeCAD (1.1.4 on the dev host), via the ADR-0007 adapter | FreeCAD | FCStd, STEP export, FreeCAD-measured properties |
   | `opencascade` | Built-in. **Only when the user explicitly selects it** | Kala's ADR-0009 OCP compiler and measure | STEP, OCP-measured properties |

   There is no silent fallback. If the selected provider is unavailable, the turn says so and suggests the available ones. `opencascade` is never picked automatically.
3. **OpenCascade's default role is view-only.** The provider's exported STEP is stored in the artifact store (sha256). OpenCascade **imports** it and **tessellates** it for display; the display aids (section-cut rendering, hidden-line pencil view) are rendered views of the imported model. The model is never modified, re-measured as if authoritative, or re-exported by Kala unless the provider is `opencascade`.
4. **Numbers come from the provider that built the model.** The agent card and bottom bar label the source, e.g. "measured by Solid Edge" / "measured by FreeCAD" / "measured by OpenCascade (opt-in)". "Verified" compares the provider-reported properties with the declared intent. If a provider reports no properties, the UI shows "—", never an OpenCascade substitute.
5. **Viewer import is backend-side** (same OpenCascade backend as the opt-in provider). No in-browser WASM kernel, so no CSP change.

## Flow

```
chat → DesignIntent/ConstructionScript ─┬─ provider=solidedge  → bridge ops → Solid Edge → STEP + mass props ─┐
                                         ├─ provider=freecad    → adapter    → FreeCAD    → STEP + props      ─┤
                                         └─ provider=opencascade (opt-in) → OCP compiler → STEP + OCP props   ─┤
                                                                                                              ▼
                                    artifact store (sha256) → OpenCascade import (view-only) → mesh/views → UI
                                    session journal: design.intent, provider.build.{requested,completed,failed},
                                    artifact.stored, view.imported (each with provider name and digests)
```

## API changes (Studio, mirrored later by the Rust front door)
- `POST /design` gains `provider: "solidedge" | "freecad" | "opencascade"` (required; there is no default). The response includes `provider`, `provider_version`, `measured_by`, `artifacts` (native + STEP digests), and `view` (mesh + bounds from the OpenCascade import).
- `GET /providers/cad`: availability per provider (probe results, never assumed).
- `/design/{id}/drawing` and `/section` render views of the imported model; they never modify it.
- Chat: the composer has a CAD-target selector (persisted per session, non-secret). Chat turns pass it through.

## Consequences
- The ADR-0009 work stays useful: it is the `opencascade` provider and the import/view engine.
- On Linux the real end-to-end path is `freecad`. Solid Edge needs a Windows host for live proof.
- The Rust migration ports the orchestration (provider routing, journal, connections). The CAD engines stay external, except the OpenCascade view and opt-in builder (ADR-0010 phase 1 Python worker; phase 2 Rust→OCCT adapter pending the user's OK).
