# B2 providers — CAD provider routing (ADR-0011), Python reference backend

Cell: `b2-providers`. Specs: ADR-0011 (binding), ADR-0009, ADR-0007,
ADR-0008, `../b1-api/REPORT.md`, r1's worker report, and the Opus addendum in
`TASK.md`.

Status: **done** for the owned Python surface. No commit, no push, no
delegation, and no credential reads were performed. Another worker (D1 QA)
owns `studio/**`; the Rust workers own `backend/crates/**`. Neither was
touched.

## What changed

| File | Change |
|---|---|
| `src/kala_agent/studio/design/providers/base.py` | new — `CadProvider` protocol, `Availability`, `MeasuredBounds`, `MeasuredProperties`, `ProviderBuild`, `ProviderStatus`, provider exceptions, closed `CadProviderId` set |
| `src/kala_agent/studio/design/providers/__init__.py` | new — explicit registry, `build_providers`, `probe_providers` |
| `src/kala_agent/studio/design/providers/opencascade.py` | new — the opt-in OCP builder wrapping the ADR-0009 compile/measure/STEP code |
| `src/kala_agent/studio/design/providers/freecad.py` | new — real `FreeCADCmd` subprocess, bounded wall time, no shell, adapter driver |
| `src/kala_agent/studio/design/providers/solidedge.py` | new — ADR-0008 token/port discovery and honest refusal on Linux |
| `src/kala_agent/studio/design/service.py` | rewritten pipeline: provider-required routing, provider build, provider STEP storage, view-only import, provider checks, provider journal |
| `src/kala_agent/studio/design/importer.py` | own STEP limit (64 MiB default, env-configurable), bounded caller contract, one coarser retry, imported shape returned, view bounds from the imported B-Rep |
| `src/kala_agent/studio/design/measure.py` | `Check.measured` may be `None`; `Check.status` is `passed`/`failed`/`not_reported` |
| `src/kala_agent/studio/schemas.py` | `DesignRequest` (required provider), nullable `DesignMeasured`, `DesignView`, extended `DesignArtifacts`, `DesignCheck.status`, `CadProviderRecord(s)`, `ChatRequest.cad_provider` |
| `src/kala_agent/studio/server.py` | `GET /providers/cad`; `POST /design` takes the required provider; chat provider selection/refusal; new result shape and 503 mapping |
| `src/kala_agent/studio/internal_api.py` | `/design` takes `DesignRequest`; `/import` gets its own route-aware body limit; worker app accepts `step_import_max_bytes` |
| `src/kala_agent/studio/providers/dispatch.py` | `StudioChatTurn.cad_provider` pass-through only |
| `tests/unit/studio_design/test_cad_providers.py` | new — routing, honesty, API, chat, view-fidelity tests |
| `tests/unit/studio_design/test_cad_providers_freecad.py` | new — real FreeCADCmd corpus parity, marked `freecad` |
| `tests/unit/studio_design/test_cad_providers_import.py` | new — multi-MB STEP, over-limit 413, bounded chunks, one coarser retry |
| `tests/unit/studio_design/test_design_api.py` | updated for the new result shape and provider-required design API |
| `tests/unit/studio_design/test_service.py` | mechanical update: every direct `service.design` call names `provider="opencascade"`; script digest is no longer the design id |
| `tests/unit/studio_worker/test_worker_http.py`, `test_worker_import.py`, `test_worker_process.py` | mechanical update for the required provider and the import's own limit |
| `pyproject.toml` | registered the `freecad` marker (the task requires the mark and `--strict-markers` is on) |

Core never imports `adapters/**`. The FreeCAD adapter is reached only by a
generated driver inside a `FreeCADCmd` subprocess; the Solid Edge bridge is
reached only by its documented loopback health probe. The AST boundary tests
and the portable-import tests are green.

## Provider contract

```python
class CadProvider(Protocol):
    name: str               # closed set: opencascade | freecad | solidedge
    display_name: str
    measured_by: str        # "OpenCascade (opt-in)" | "FreeCAD" | "Solid Edge"
    mode: str               # "opt-in" | "external" | "bridge"

    def probe(self) -> Availability:
        """available, version?, reason?"""

    def build(self, script: ConstructionScript, intent: DesignIntent) -> ProviderBuild:
        """step_bytes, measured, provider_version, warnings, native_bytes?,
        native_kind?, valid?, solid_count?"""
```

`MeasuredProperties` carries `volume_mm3?`, `area_mm2?`, `bounds?`, `mass_g?`,
and an optional `centroid_mm?`. A property the provider does not report stays
`None`.

There is **no default provider**. `DesignService.design(intent, *, provider)`
has no default keyword, the API `DesignRequest.provider` is required, and an
absent/unknown choice is refused before any geometry work.

## The three providers

### `opencascade` — opt-in builder

Wraps the existing ADR-0009 compiler, `measure_shape`, and STEP writer. It is
never selected automatically. `probe()` returns the OCP version
(`OCP.__version__`, `8.0.1.1` on this host) or an honest unavailable reason.
It returns no native artifact.

### `freecad` — real subprocess on this host

`probe()` finds `FreeCADCmd`/`freecadcmd` on `PATH` and runs
`<exe> --version` with a 20 s timeout. On this host it reports `1.1.4`.

`build()` writes a request JSON and a fixed driver script into a temporary
directory, then runs:

```
<FreeCADCmd> <driver.py>          # no shell, cwd=temp dir, env=constructed,
                                  # timeout=120 s, stdout=DEVNULL, stderr=file
```

The driver imports the adapter's documented `kala_agent_freecad.cad.design.run`
entry point, lowers the shared `ConstructionScript` with
`compile_construction`, builds the model in a FreeCAD document, exports STEP
and FCStd, reopens the FCStd to measure FreeCAD's own `BoundBox` and `Solids`,
and writes a JSON result file. The core reads that file, verifies every
artifact path is inside the workspace, re-hashes every artifact, and only then
returns a `ProviderBuild`. A refusal or crash is a typed
`CadProviderBuildError` naming the adapter error (and the op when the adapter
message is generic). The subprocess environment is scrubbed of
credential-shaped keys (`TOKEN`, `API_KEY`, `SECRET`, `PASSWORD`,
`CREDENTIAL`) before the driver starts, so FreeCAD never inherits the worker
token or a chat API key.

Actual result on this host for the ADR-0009 L-bracket:

```
provider            freecad 1.1.4
volume              35571.416047 mm^3      (FreeCAD-measured, rounding 1e-6)
surface area        11889.101731 mm^2
bounds              [-30, 0, 0] … [30, 45, 40]
valid               1 solid
native artifact     FCStd (digest verified)
```

`tests/unit/studio_design/test_cad_providers_freecad.py` is marked `freecad`
and skips with a clear reason when the probe reports FreeCAD absent (or when
the OCP view importer is not installed, because the bounds check needs it).
Both are present on this host, so it runs.

### `solidedge` — honest unavailable state on Linux

`probe()` refuses non-Windows immediately with a reason. On Windows it
resolves `KALA_SOLIDEDGE_BRIDGE_URL` (default `http://127.0.0.1:8797`) and
`KALA_SOLIDEDGE_TOKEN_FILE` (default `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`),
requires a loopback HTTP URL, reads the token without logging it, disables
proxy handling, refuses redirects, and asks the authenticated `/v1/health`
route. `available` is true only when the bridge says `available` and
`supported`.

A build is never faked. On Linux the service refuses before the build; the
503 detail contains the exact required text:

```
Solid Edge build requires the Windows bridge (not reachable); this host is linux,
so the ADR-0008 bridge cannot run here
```

The bridge milestone that landed has no production native executor, so even a
reachable Windows bridge reports no build capability.

## `design_id` and artifact identity (Rust mirror)

```
canonical_script_bytes = canonical JSON of ConstructionScript.model_dump(
    mode="json", exclude_none=True) with sorted keys, compact separators,
    ASCII escapes, and no NaN/Infinity
script_sha256 = sha256_hex(canonical_script_bytes)
design_id      = sha256_hex(canonical_script_bytes || 0x00 || ascii(provider))
```

The provider is part of the identity: the same script built by `freecad` and
`opencascade` is two designs with different measured numbers. The stored
script artifact is keyed by `script_sha256`, **not** `design_id`. Example for
the L-bracket:

```
script_sha256 = 88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae
opencascade   = 594f408b3223505c51bfdef61cfb935fc28f8ec461aa62fefc62c9b731932c92
freecad       = a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7
```

## Design flow and journal

For a successful build, in order:

1. `design.intent`
2. `provider.build.requested`
3. `provider.build.completed`
4. `artifact.stored` (script, keyed by `script_sha256`)
5. `artifact.stored` (STEP, `source: "provider:<id>"`)
6. `artifact.stored` (native, e.g. `fcstd`; absent for opencascade)
7. `view.imported`
8. `design.compiled` (opencascade only)
9. `design.measured`
10. `design.checked` → `journal_head`

A refusal writes `provider.build.failed` + `design.rejected` and stores no
artifact. A cache hit writes only `design.intent` with `cache_hit: true` and
reloads the stored STEP; it does not rerun FreeCAD or OCP.

Payloads (all bounded by the journal's 256 KiB payload limit):

```jsonc
// design.intent
{"design_id","provider","script_sha256","template","label","material",
 "tolerances","envelope","cache_hit"}

// provider.build.requested
{"design_id","provider","provider_version","script_sha256"}

// provider.build.completed
{"design_id","provider","provider_version","measured_by","script_sha256",
 "step_sha256","native_sha256","native_kind",
 "measured":{"volume_mm3","surface_area_mm2","centroid_mm","bounds","mass_g"},
 "valid","solid_count","warnings"}

// provider.build.failed
{"design_id","provider","code","reason"}

// artifact.stored
{"design_id","provider","kind":"script"|"step"|"fcstd"|"stl","sha256",
 "size_bytes","source"?}

// view.imported (service; the raw /import route omits design_id/provider)
{"design_id","provider","step_sha256","viewer":"opencascade","role":"view-only",
 "solids","vertices","triangles","bounds","warnings"}

// design.measured
{"design_id","provider","measured_by","volume_mm3","surface_area_mm2",
 "centroid_mm","bounds","mass_g","valid","solid_count"}

// design.checked
{"design_id","provider","verified",
 "checks":[{"id","label","expected","measured","tolerance","unit","passed","status"}]}

// design.compiled (opencascade only)
{"design_id","provider","components","fused","warnings","envelope",
 "tolerances","material","label"}
```

`view.imported` and `provider.build.*` are not members of the ADR-0009
`JournalEventType` enum; the journal accepts safe identifiers, so consumers
match the strings.

## Updated API JSON shapes

### `POST /api/studio/design` (and worker `POST /internal/v1/design`)

Request = `DesignIntent` **plus a required `provider`**:

```json
{
  "provider": "opencascade | freecad | solidedge",
  "template": {"template": "l_bracket", "width": 60, "depth": 45,
               "height": 40, "wall": 8, "bore_d": 10, "hole_d": 5.5,
               "hole_count": 4},
  "material": "aluminium",
  "tolerances": [],
  "label": null
}
```

Exactly one of `template`/`script` as before. A missing provider is a 422
`invalid_request` whose `details[0]` names all three values:

```json
{"code":"invalid_request","message":"the request body or parameters are invalid",
 "details":["body: value_error: Value error, provider is required (choose opencascade, freecad, solidedge)"]}
```

An unknown provider is a pydantic enum 422 naming the three values. A
provider that is unavailable is 503:

```json
{"code":"provider_unavailable",
 "message":"the solidedge CAD provider is not available",
 "details":["the solidedge CAD provider is not available",
            "reason: Solid Edge build requires the Windows bridge (not reachable); this host is linux, ...",
            "available: opencascade, freecad"]}
```

### `DesignResult` — exact shape

```json
{
  "design_id": "64 hex = sha256(canonical script || 0x00 || provider)",
  "label": "L-bracket 60 x 45 x 40",
  "provider": "opencascade | freecad | solidedge",
  "provider_version": "1.1.4 | 8.0.1.1 | null",
  "measured_by": "OpenCascade (opt-in) | FreeCAD | Solid Edge",
  "script": { "...": "canonical script model dump, as before" },
  "measured": {
    "volume_mm3": 35571.416047,
    "surface_area_mm2": 11889.101731,
    "centroid_mm": [0.0, 0.0, 0.0] | null,
    "bounds": {"min": [x,y,z], "max": [x,y,z]} | null,
    "size_mm": [x,y,z] | null,
    "valid": true | null,
    "solid_count": 1 | null,
    "mass_g": null
  },
  "checks": [
    {"id":"brep_valid","label":"B-Rep valid","expected":true,
     "measured":true,"tolerance":0.0,"unit":"bool","passed":true,
     "status":"passed"},
    {"id":"single_solid","label":"Single solid","expected":1.0,
     "measured":1.0,"tolerance":0.0,"unit":"solid","passed":true,
     "status":"passed"},
    {"id":"envelope_x","label":"Envelope X","expected":[-30.0,30.0],
     "measured":[-30.0,30.0],"tolerance":0.01,"unit":"mm",
     "passed":true,"status":"passed"}
  ],
  "verified": true,
  "view": {
    "positions": [0.0, "... flat xyz triples ..."],
    "normals": [0.0, "... flat xyz triples ..."],
    "indices": [0, 1, 2, "... zero-based triangle corners ..."],
    "vertex_count": 586,
    "triangle_count": 568,
    "deflection_mm": 0.1,
    "engine": "ocp",
    "bounds": {"min": [-30.0, 0.0, 0.0], "max": [30.0, 45.0, 40.0]},
    "solids": 1,
    "warnings": []
  },
  "artifacts": {
    "script_sha256": "88a7ec13...",
    "step_sha256": "80e11bd9...",
    "native_sha256": "c0f43df5..." | null,
    "native_kind": "fcstd" | null
  },
  "journal_head": "64 hex or null",
  "material": "aluminium" | null,
  "tolerances": ["bore diameter 10 mm (declared)", "..."],
  "declared_envelope": {"min": [-30.0,0.0,0.0], "max": [30.0,45.0,40.0]} | null,
  "cached": false,
  "warnings": [],
  "component_count": 2,
  "fused": true
}
```

Changes from the B1 shape, for C2 and the Rust mirror:

| B1 | B2 |
|---|---|
| `mesh` | **`view`**, same mesh fields plus `bounds`, `solids`, `warnings`; the mesh is the import of the provider's STEP for every provider |
| `design_id == artifacts.script_sha256` | **not equal**; `design_id` includes the provider, `script_sha256` is the script digest |
| `measured` all required | **nullable** (`volume_mm3`, `surface_area_mm2`, `centroid_mm`, `bounds`, `size_mm`, `valid`, `solid_count`, new `mass_g`) |
| `checks[].measured` non-null | may be **null** with `status: "not_reported"` |
| — | **`provider`**, **`provider_version`**, **`measured_by`** |
| `artifacts.step_sha256` process-local | stored at build time, provider export; new **`native_sha256`**, **`native_kind`** |

FreeCAD sample (real run; mesh abbreviated):

```json
{
  "design_id": "a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7",
  "label": "L-bracket 60 x 45 x 40",
  "provider": "freecad",
  "provider_version": "1.1.4",
  "measured_by": "FreeCAD",
  "measured": {
    "volume_mm3": 35571.416047,
    "surface_area_mm2": 11889.101731,
    "centroid_mm": null,
    "bounds": {"min": [-30.0, -5.329070518200751e-15, 0.0],
               "max": [30.0, 45.0, 40.0]},
    "size_mm": [60.0, 45.00000000000001, 40.0],
    "valid": true,
    "solid_count": 1,
    "mass_g": null
  },
  "verified": true,
  "view": {
    "bounds": {"min": [-30.0, -8.881784197001e-16, 0.0],
               "max": [30.0, 45.0, 40.0]},
    "solids": 1,
    "vertex_count": 586,
    "triangle_count": 568,
    "deflection_mm": 0.1,
    "engine": "ocp"
  },
  "artifacts": {
    "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
    "step_sha256": "24a5dca7bf55b1bcc2d4f33e796a739c210b2f85a89c964a7d59393f2bd58403",
    "native_sha256": "e0f9fb920cebad9eb39e79fb2124772f0ec99f93503aab93e158c9a4e44c710e",
    "native_kind": "fcstd"
  }
}
```

Not-reported sample (a provider that returns a STEP but no numbers). The
design is still returned, with `verified: false` and every row
`not_reported`; no number is substituted:

```json
"measured": {"volume_mm3": null, "surface_area_mm2": null,
             "centroid_mm": null, "bounds": null, "size_mm": null,
             "valid": null, "solid_count": null, "mass_g": null},
"checks": [
  {"id":"brep_valid","expected":true,"measured":null,
   "tolerance":0.0,"unit":"bool","passed":false,"status":"not_reported"},
  {"id":"single_solid","expected":1.0,"measured":null,
   "tolerance":0.0,"unit":"solid","passed":false,"status":"not_reported"},
  {"id":"envelope_x","expected":[-30.0,30.0],"measured":null,
   "tolerance":0.01,"unit":"mm","passed":false,"status":"not_reported"}
],
"verified": false
```

### `GET /api/studio/providers/cad`

Requires the launch bearer (new GET-route convention). The full path is
`/api/studio/providers/cad`; the Rust front door should expose
`GET /providers/cad`.

```json
{
  "providers": [
    {"id":"opencascade","name":"OpenCascade (opt-in)","available":true,
     "version":"8.0.1.1","reason":null,
     "measured_by":"OpenCascade (opt-in)","mode":"opt-in"},
    {"id":"freecad","name":"FreeCAD","available":true,
     "version":"1.1.4","reason":null,
     "measured_by":"FreeCAD","mode":"external"},
    {"id":"solidedge","name":"Solid Edge","available":false,
     "version":null,
     "reason":"Solid Edge build requires the Windows bridge (not reachable); this host is linux, ...",
     "measured_by":"Solid Edge","mode":"bridge"}
  ]
}
```

Probe results are live evidence; a provider that is absent reports
`available: false` with a reason, never a constant `true`.

### Chat

`POST /api/studio/chat` request gains an optional target:

```json
{"provider":"local","model":null,"prompt":"...","allow_remote":false,
 "cad_provider":"opencascade | freecad | solidedge | null"}
```

- A turn with a complete design intent and `cad_provider: null` returns
  `design: null` and a reply that starts
  `Choose a CAD provider before I build this design: ... Available on this
  host: opencascade, freecad. No design was built.`
- A selected but unavailable provider returns `design: null` with
  `The <id> CAD provider is not available: <reason>. Available: .... No
  design was built.`
- A selected available provider builds and returns `design` (the same
  `DesignResult`), and the text says `measured by <measured_by>`.

The `chat.turn` journal payload gains `cad_provider`.

### Errors

| Status | `code` | Cause |
|---|---|---|
| 422 | `invalid_request` | missing provider (detail names values), unknown provider, invalid intent |
| 422 | `design_rejected` | the selected provider refused/failed, or its STEP could not be imported |
| 503 | `provider_unavailable` | provider probe says unavailable; details include reason + available ids |
| 503 | `design_unavailable` | OCP is not importable, so the view importer cannot run |
| 404 | `design_not_found` | no completed provider build under that id |
| 413 | `payload_too_large` | `/import` over its own STEP limit; every other route over 262144 bytes |
| 422 | `invalid_step` / `step_too_complex` | the upload is not readable STEP / still too complex after one coarser retry |

`POST /design` is 200 even when the checks fail or report no values; the
result carries `verified: false` and per-check `status`. A provider refusal
is still 422 and stores no artifact.

## STEP import limit and retry (Opus addendum)

- New default: **64 MiB** for the STEP import route and for provider-returned
  STEP inside the service. `KALA_STEP_IMPORT_MAX_BYTES` overrides it
  (1 … 512 MiB). Invalid values refuse startup.
- The import route has its own route-aware body middleware. It checks a
  declared `content-length` first, then reads chunks with the existing
  bounded `_buffer_body`, which stops before the buffer can exceed the limit.
  Every other worker route still uses `MAX_REQUEST_BODY_BYTES = 262144`.
- The importer keeps the 200k-vertex / 400k-triangle budget. On a `MeshError`
  it retries **once** at `0.5 mm` / `0.8 rad`; if that also fails it raises
  `step_too_complex`. A successful coarser retry appends a warning and the
  view reports the coarser `deflection_mm`; it never truncates.
- The multi-MB test generates 200 disjoint OCP boxes (about 3.4 MB) and posts
  them to a worker limited to 8 MiB: 200, 200 solids. The same bytes with a
  1 MiB limit: 413 before the handler. The old 262145-byte body is now
  `invalid_step` (422), proving the generic cap is no longer imposed on the
  import.
- Import view bounds are the imported B-Rep's optimal box
  (`BRepBndLib.AddOptimal`, `useTriangulation=False`, gap 0), not the mesh
  vertices. Mesh vertices under-report a tessellated circle's extreme by up
  to the 0.1 mm deflection; the imported shape's envelope is exact for the
  0.001 mm parity acceptance. No volume, area, or centroid is added; the
  provider remains the measuring authority.

## Acceptance evidence

| Criterion | Evidence | Result |
|---|---|---|
| opencascade L-bracket volume | `test_cad_providers.py::test_opencascade_provider_measures_the_lbracket`, `test_service.py::test_design_measures_the_adr_lbracket` | `35571.416047113315` mm³ (rel ~2.5e-15 vs 35571.416047) |
| opencascade imported view bounds | `test_cad_providers.py::test_service_stores_the_provider_step_and_imports_the_view`, `test_design_api.py::test_criterion_2_lbracket_end_to_end` | `[-30, 30] × [0, 45] × [0, 40]` (±0.001; exact in the view) |
| FreeCAD real probe | `test_cad_providers_freecad.py::test_probe_reports_the_real_freecad_1_1_installation` | `available: true`, `1.1.4` |
| FreeCAD L-bracket | `test_cad_providers_freecad.py::test_l_bracket_builds_in_freecad_and_reports_its_own_volume` | FreeCAD volume `35571.416047`, 1 valid solid, FCStd + STEP; STEP view bounds `[-30,30]×[0,45]×[0,40]` |
| FreeCAD corpus parity | `test_cad_providers_freecad.py::test_every_supported_parity_case_builds_and_matches_the_fixture` | 28 corpus cases: 26 build (24 checked volumes + 2 revolve builds); every checked volume within 1e-6; every available bound within 0.001 mm; 2 multi-component place-only assemblies refused naming `fuse` |
| solidedge on Linux | `test_cad_providers.py::test_solidedge_is_unavailable_on_linux_and_refuses_a_build`, `test_post_design_with_solidedge_is_a_503` | probe `available: false` with reason; `POST /design` 503 `provider_unavailable`; no success |
| no default provider | `test_cad_providers.py::test_service_refuses_a_missing_or_unknown_provider`, `test_missing_provider_is_422_naming_allowed_values` | service keyword required; API 422 detail names `opencascade, freecad, solidedge` |
| view is the provider export | `test_cad_providers.py::test_view_is_the_provider_export_not_a_recompile` | L-bracket script + a fake FreeCAD STEP of a 20×30×40 box → view bounds are the box, not a recompiled bracket |
| provider numbers only | `test_cad_providers.py::test_missing_provider_values_are_not_reported`, `test_checks_compare_provider_bounds_with_the_declared_envelope` | nulls + `not_reported`; no substitution |
| import size separation | `test_cad_providers_import.py` | 3.4 MB accepted at 8 MiB; 413 at 1 MiB; other routes still 262144 |
| retry once | `test_cad_providers_import.py` | first `MeshError` → one 0.5 mm retry + warning; second failure → `step_too_complex` |
| boundary / no adapter import | `tests/unit/test_adapter_boundaries.py`, `test_solidedge_boundaries.py`, `test_portable_imports.py` | 18 passed |

FreeCAD corpus summary (from the test):

```
supported: 26
  - 24 with a checked volume and bounds (all within 1e-6 / 0.001 mm)
  - 2 rectangle-revolve builds with no closed-form volume oracle
refused: 2
  - assembly_two_parts_placed_once
  - assembly_three_parts_placed_once
    (multi-component place-only; the adapter would export only its last
     component, so the provider refuses and names the missing `fuse`)
special: assembly_single_translated_part builds correctly via the driver's
  self-fuse no-op (see gap 2)
```

## Gates (this host, `.venv` CPython 3.14.8)

| Gate | Command | Result |
|---|---|---|
| Design suite | `pytest tests/unit/studio_design -q` | **210 passed** |
| Worker suite | `pytest tests/unit/studio_worker -q` | **35 passed** |
| Full unit suite | `pytest tests/unit -q` | **1511 passed**, 1 warning (`StarletteDeprecationWarning`, pre-existing) |
| Security suite | `pytest tests/security -q` | **189 passed** |
| Boundary/portable imports | `pytest tests/unit/test_adapter_boundaries.py tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q` | **18 passed** |
| Ruff check | 20 touched/created files | **All checks passed** |
| Ruff format | same 20 files | **20 files already formatted** |
| Mypy source | 12 touched source files, repo config | **Success: no issues found in 12 source files** |
| Mypy tests | 8 updated/new test files, `--python-version 3.14` | **Success: no issues found in 8 source files** |

The literal repo-config mypy run over the test files hits the pre-existing
environment issue recorded by A1/A2/A3/B1/r1:

```
.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error: Type
statement is only supported in Python 3.12 and greater  [syntax]
```

The venv is 3.14 while `pyproject.toml` pins `python_version = "3.11"`; the
same failure reproduces on untouched test files. With the matching target
(`--python-version 3.14`) the tests are clean. No project setting was
loosened to hide it.

## Gaps and honest notes

1. **`worker.invoke_design` is not used.** The adapter's exported
   `cad.design.run` is used instead. The worker entry point wraps the same
   builder in `DESIGN_OUTPUT_SCHEMA`, whose `tool_trace` cap is 64 while the
   adapter's own result truncates at 160; an L-bracket has >64 calls, so the
   wrapper rejects a successful build as `invalid_result`. `design.run` is an
   adapter `__all__` entry point and the same builder; the driver still runs
   only adapter code. This is an adapter defect note, not a reimplementation.
   Fixing it belongs to the adapter distribution.
2. **A single placed component is corrected in the driver.** `design.run`
   exports/measures its tip feature. For one component with a non-zero
   `place` and no union, the tip is the pad and the body placement would be
   lost. The driver appends a self-fuse of that one body (a geometric no-op)
   so the placed body is the tip. The script and geometry are unchanged; only
   the export target is corrected. This makes the corpus case
   `assembly_single_translated_part` build at `[5,6,7]…[15,16,12]`.
3. **Multi-component assemblies that are not fully fused are refused.** The
   adapter would export only the last component (place-only) or only the fused
   subset and present it as the design. The provider refuses first and names
   `fuse`; those two corpus cases are not claimed as supported. There is no
   silent partial model.
4. **FreeCAD reports no centroid or mass.** The adapter's measurement block
   has volume, area, and bounding-box size; the provider reopens the FCStd to
   get exact min/max and solid count. `centroid_mm` and `mass_g` are null.
   OpenCascade never fills them in.
5. **FreeCAD's interpreter compatibility.** The driver runs inside
   `FreeCADCmd`'s embedded Python and imports the adapter from the repo source
   and the Studio venv's `purelib`. On this host FreeCAD 1.1.4 embeds Python
   3.14 and can load the venv's pydantic 2.13.5. On a host whose FreeCAD
   embeds a different Python minor, the import fails and the build refuses
   with the import error; it does not fabricate a model. The adapter path is
   the repo layout (`adapters/freecad/src`); a packaged build must ship or
   install the adapter on the driver's path.
6. **Import view bounds changed from r1.** r1 reported mesh-vertex bounds.
   Those under-report a curved extreme by up to the 0.1 mm deflection (a
   Ø4 pad was 0.0146 mm inside), which fails the 0.001 mm provider acceptance.
   The view bounds are now the imported B-Rep's optimal box with
   `useTriangulation=False`; no volume/area/centroid is added and the provider
   remains the measuring authority.
7. **Check failures do not reject the design.** `POST /design` returns 200
   with `verified: false` and per-check `status` so a declared-envelope
   mismatch or a `not_reported` value is visible in the UI. B1 made a failed
   check a 422; that would make the ADR-0011 "—" state unrepresentable. The
   geometry-refusal path is still 422.
8. **The geometry lock covers the FreeCAD subprocess.** Concurrent design
   turns serialise, and a drawing/section waits for an in-flight build. A
   future provider worker pool could relax this; correctness and one shared
   OCP lock were preferred for this cell.
9. **No native Solid Edge build.** The probe/refusal path is complete; a live
   Windows bridge with a production executor is still required for a real
   Solid Edge part. On Linux the route is a 503.
10. **The public route path is `/api/studio/providers/cad`.** The worker does
    not mount it because r1's documented route set is exact and worker-only.
    The Rust front door should expose `GET /providers/cad` and may use the
    worker `/design` 503 `available` details for its own selection message.
11. **Pre-B2 designs are not loadable.** A design with no
    `provider.build.completed` event has no provider identity; `load` returns
    404. The journal remains verifiable. No migration was written.
12. **`GET /providers/cad` requires the bearer.** It follows the new-GET-route
    convention; C2 must send the studio token.
13. **Test/config edits outside the production list.** `test_service.py`,
    the three worker test files, and `pyproject.toml` were updated
    mechanically because the provider is now required, the design id changed,
    the import limit moved, and the `freecad` marker must be registered under
    `--strict-markers`. `measure.py` gained `Check.status`/nullable measured
    for the `not_reported` contract. No other worker's files were touched.
14. **`mass_g` is always null in this round.** There is no density table in
    Studio; a declared `material` string is not a density. A provider that
    reports mass in a future adapter result can populate it.

## For C2 (provider UI) — checklist

1. Fetch `GET /api/studio/providers/cad` with the studio bearer. Render the
   three rows with `available`, `version`, and `reason`; disable unavailable
   providers instead of guessing. Keep `opencascade` labelled "opt-in".
2. Add a CAD-target selector to the composer, persisted per session
   (non-secret), default **none** (force a choice). Send it as
   `cad_provider` on `POST /api/studio/chat`. When it is absent, the server
   replies with the provider-choice prompt and returns `design: null`.
3. Use `DesignResult.provider` and `measured_by` in the agent card and bottom
   bar, e.g. "measured by FreeCAD". Do not call OpenCascade the measurer of a
   FreeCAD/Solid Edge part.
4. Render every nullable `measured` field as an em dash; never show a
   substituted number. The same applies to `checks[].measured`.
5. Render `checks[].status`: `passed`/`failed`/`not_reported`. Show
   "B-Rep Verified" only when `verified` is true; a `not_reported` row is not
   a pass and not a failure.
6. Replace the old `design.mesh` read with `design.view` (same
   `positions`/`normals`/`indices`/counts/`deflection_mm`/`engine`, plus
   `bounds`, `solids`, `warnings`). The view is always the imported provider
   STEP, including for the opt-in OpenCascade provider.
7. Show `artifacts.step_sha256` and, when present, `native_sha256` +
   `native_kind` (e.g. an "FCStd" chip). `artifacts.script_sha256` is the
   script digest; `design_id` is not the script digest.
8. A 503 `provider_unavailable` body already carries the reason and the
   available ids; render those rather than a generic error.
9. `GET /providers/cad` is under `/api/studio`; the drawing/section/journal
   GET routes still require the bearer.

## For the Rust r2 mirror — checklist

1. Model the closed enum `opencascade | freecad | solidedge`; make the design
   request field required and return a 422 listing the values when absent.
2. Compute `script_sha256` and `design_id` exactly as in the formula above;
   store the script under `script_sha256`, not `design_id`.
3. Mirror `DesignResult` with `provider`, `provider_version`, `measured_by`,
   nullable `measured`, `checks[].status`, `view` (not `mesh`), and
   `artifacts.native_sha256`/`native_kind`.
4. Expose `GET /providers/cad` at the front door. Probe; do not assume. The
   Python worker's exact route set is unchanged and does not include it.
5. Pass `cad_provider` through chat. A complete intent with no provider must
   produce the choice prompt and no design; an unavailable provider must
   produce its reason and no design.
6. Map the error codes: `provider_unavailable` (503),
   `design_rejected` (422), `design_unavailable` (503 when OCP is missing),
   `invalid_request` (422), `design_not_found` (404), `payload_too_large`
   (413).
7. Set `KALA_STEP_IMPORT_MAX_BYTES` on the Python worker if a non-default
   STEP import ceiling is wanted. The worker's `/internal/v1/import` reads
   that value; every other worker route stays at 262144 bytes. If Rust proxies
   the upload, enforce the same ceiling at the front door.
8. Journal the provider events with the payloads above. `view.imported` and
   `provider.build.*` are string event types, not members of the ADR-0009
   enum. Include the provider name and digests; do not claim a provider
   measured a number it did not report.
9. `POST /internal/v1/design` now takes `DesignRequest` (provider required).
   Drawing/section/export work on the stored provider STEP through the
   view-only importer; `POST /internal/v1/export` STEP returns the provider's
   exact stored bytes.
10. The worker's `/internal/v1/import` view bounds are the imported B-Rep's
    optimal box, not mesh vertices; keep the 200k/400k mesh limits.
