# B1 api — design turns, journal routes, and chat integration

Cell: `b1-api`. Spec: ADR-0009 §4, §6, the B1 TASK, and the A1/A2/A3 reports.
Status: **complete** for the owned surface; gaps are listed at the end.
No commit, no push, no delegation, and no credential reads were performed.

## Files

Created (owned by this cell):

| File | Lines | Purpose |
|---|---:|---|
| `src/kala_agent/studio/design/templates.py` | 433 | `l_bracket`, `stepped_shaft`, `housing_envelope`; declared envelope, declared tolerances, label |
| `src/kala_agent/studio/design/intent.py` | 231 | pydantic `DesignIntent`, JSON Schema export, fenced-JSON parser |
| `src/kala_agent/studio/design/mesh.py` | 211 | bounded `BRepMesh_IncrementalMesh` tessellation and binary STL |
| `src/kala_agent/studio/design/service.py` | 849 | `DesignService`: cache, geometry lock, journal, exports |
| `tests/unit/studio_design/test_templates.py` | 177 | 20 tests |
| `tests/unit/studio_design/test_intent.py` | 256 | 17 tests |
| `tests/unit/studio_design/test_service.py` | 340 | 18 tests |
| `tests/unit/studio_design/test_design_api.py` | 652 | 29 tests |

Edited (owned by this cell):

| File | Change |
|---|---|
| `src/kala_agent/studio/server.py` | `POST /design`, `GET /design/{id}/drawing`, `GET /design/{id}/section`, `GET /journal`, `GET /journal/verify`, `/export` `design_id`, `/chat` `design`, `/health` `design_available`; app-owned `DesignService`; `KALA_STUDIO_DATA_DIR` honoured |
| `src/kala_agent/studio/schemas.py` | design/drawing/section/journal response models, optional `ChatResponse.design`, `HealthResponse.design_available`, `ExportRequest.design_id` |
| `src/kala_agent/studio/providers/local.py` | strict local design parser (`parse_design_prompt`) and design-aware replies |
| `src/kala_agent/studio/providers/dispatch.py` | `StudioChatTurn` design-intent hook for local and remote providers |

Not touched: `src/kala_agent/__init__.py`, `src/kala_agent/api/**`, A1/A2/A3
modules, `adapters/**`, `backend/**`, or any `studio/**` frontend file.

## Public interfaces

### `kala_agent.studio.design.templates`

```python
class TemplateError(ValueError): ...

class TemplateResult(NamedTuple):
    script: ConstructionScript
    envelope: Bounds            # kala_agent.studio.design.measure.Bounds
    tolerances: tuple[str, ...]
    label: str

def l_bracket(
    width: float, depth: float, height: float, wall: float,
    bore_d: float | None = None, hole_d: float | None = None,
    hole_count: int = 0, edge: float | None = None,
) -> TemplateResult: ...

def stepped_shaft(
    segments: tuple[tuple[float, float], ...] | list[tuple[float, float]],
) -> TemplateResult: ...       # (diameter, length), mm

def housing_envelope(
    width: float, depth: float, height: float, wall: float,
) -> TemplateResult: ...
```

Validation includes `height > wall`, `depth > wall`, `2*edge < width`,
`edge + hole_d/2 < width/2`, `hole_d < wall`, `bore_d < height - wall`,
four-hole depth room, two-hole depth room, non-empty shaft segments, and
housing interior room. The L-bracket upright is the A1 same-plane
construction: an `XZ` pad extruded to `y in [-T, 0]`, bored through, then
placed at `y = +T` and fused to the `XY` base. The housing is built from a
floor plus four prism walls, because the v1 compiler's pocket always cuts
along the plane normal; the union has the same floor-and-wall envelope with
the opening on `+Z`.

Defaults used by the local parser (not silently by the template):
`DEFAULT_BRACKET_WIDTH_MM = 60` and `DEFAULT_BRACKET_DEPTH_MM = 45`
(defined in `intent.py`, re-exported by `templates.py`).

### `kala_agent.studio.design.intent`

```python
class DesignIntent(BaseModel):
    template: LbracketCall | SteppedShaftCall | HousingCall | None
    script: ConstructionScript | None
    material: str | None                    # declared, <= 64 chars
    tolerances: tuple[str, ...] = ()        # declared, <= 16 x 64 chars
    label: str | None                       # <= 120 chars
    # exactly one of template/script; extra fields refused

def design_intent_json_schema() -> dict[str, object]: ...
def design_intent_schema_text() -> str: ...
def parse_design_intent_text(text: str) -> DesignIntent: ...
def strip_design_json_block(text: str) -> str: ...
```

`LbracketCall` fields: `width`, `depth`, `height`, `wall`, `bore_d?`,
`hole_d?`, `hole_count` (`0|2|4`), `edge?`. The ADR-style aliases `W`, `D`,
`H`, `T`, `B`, `h`, `holes`/`holeCount`, and `e` are accepted on input;
responses always use the descriptive field names.
`SteppedShaftCall` fields: `segments[{diameter, length}]` (1–8).
`HousingCall` fields: `width`, `depth`, `height`, `wall` (ADR aliases `W`,
`D`, `H`, `T` also accepted).

### `kala_agent.studio.design.mesh`

```python
DEFAULT_MESH_DEFLECTION_MM = 0.1
DEFAULT_ANGULAR_DEFLECTION_RAD = 0.5

class MeshError(ValueError): ...

@dataclass(frozen=True, slots=True)
class MeshData:
    positions: tuple[float, ...]     # xyz triples, mm
    normals: tuple[float, ...]       # unit xyz triples
    indices: tuple[int, ...]         # zero-based triangle corners
    deflection_mm: float
    angular_deflection_rad: float
    # vertex_count, triangle_count properties

def tessellate(shape, *, deflection_mm=0.1, angular_deflection_rad=0.5,
               max_vertices=MAX_MESH_VERTICES,
               max_triangles=MAX_MESH_TRIANGLES) -> MeshData: ...
def stl_bytes(mesh: MeshData) -> bytes: ...
```

It refuses (never truncates) above `MAX_MESH_VERTICES = 200_000` or
`MAX_MESH_TRIANGLES = 400_000`.

### `kala_agent.studio.design.service`

```python
def design_available() -> bool: ...
def canonical_script_json(script: ConstructionScript) -> bytes: ...
def design_id_for(script: ConstructionScript) -> str: ...

class DesignRejectedError(DesignError):
    reason: str
    design_id: str | None

class DesignNotFoundError(DesignError): ...
class DesignUnavailableError(DesignError): ...

@dataclass(frozen=True, slots=True)
class DesignOutcome:
    design_id: str
    label: str
    script: dict[str, object]
    shape: TopoDS_Shape
    measurement: Measurement
    checks: tuple[Check, ...]
    verified: bool
    mesh: MeshData
    script_sha256: str
    step_sha256: str | None
    journal_head: str | None
    tolerances: tuple[str, ...]
    material: str | None
    declared_envelope: Bounds | None
    cached: bool
    warnings: tuple[str, ...]
    component_count: int
    fused: bool

class DesignService:
    def __init__(self, data_dir: Path, *, journal=None,
                 cache_size=32) -> None: ...
    def design(self, intent: DesignIntent) -> DesignOutcome: ...
    def load(self, design_id: str) -> DesignOutcome: ...
    def drawing(self, design_id: str, views: Sequence[str]) -> Drawing: ...
    def section(self, design_id: str, *, plane="XZ",
                offset_mm=None) -> Section: ...
    def export(self, design_id: str, fmt: str) -> ExportOutcome: ...
    def step_digest(self, design_id: str) -> str | None: ...
    def record_chat_turn(...) -> SessionEvent: ...
    def journal_page(self, after=0, limit=200) -> tuple[SessionEvent, ...]: ...
    def verify(self) -> JournalVerification: ...
    def close(self) -> None: ...      # context manager supported
```

`design_id` is the SHA-256 of the canonical script JSON, and the script
artifact is stored under the same digest. The service holds one
process-wide `RLock` around every OCP compile, measure, mesh, drawing,
section, and export. The LRU holds 32 `DesignOutcome` objects; a cache miss
recompiles from the stored script artifact and recovers the declared
envelope/label/tolerances/material from `design.compiled`.

Journal events written by the service:

| Event | Payload keys |
|---|---|
| `design.intent` | `design_id`, `template`, `label`, `material`, `tolerances`, `envelope`, `cache_hit`, `script_sha256` |
| `artifact.stored` | `design_id`, `kind` (`script`/`step`/`stl`), `sha256`, `size_bytes` |
| `design.compiled` | `design_id`, `components`, `fused`, `warnings`, `envelope`, `tolerances`, `material`, `label` |
| `design.measured` | `design_id`, `volume_mm3`, `surface_area_mm2`, `centroid_mm`, `bounds`, `valid`, `solid_count` |
| `design.checked` | `design_id`, `verified`, `checks` (same row shape as the API) |
| `export.completed` | `design_id`, `format`, `sha256`, `size_bytes`, `filename` |
| `chat.turn` | `provider`, `model`, `offline`, `prompt_sha256`, `prompt_chars`, `design_id`, `design_requested`, `missing` |
| `design.rejected` | `design_id`, `reason` |

`design.rejected` is not in the A3 enum, but A3's journal accepts any safe
identifier; the task explicitly requires this event. A compile/measure/mesh
refusal writes it and stores no artifact and no `design.compiled`.

### Providers

`StudioChatTurn(ChatOutcome)` adds `design_intent`, `design_missing`,
`design_notes`, `design_family`, `design_error` for the server only. The
local parser recognises L-bracket, stepped-shaft, and rectangular housing
phrasings with explicit mm values, maps `M5` to `Ø5.5` per ISO 273 medium,
and asks for exactly the missing values otherwise. Remote providers receive
the `DesignIntent` JSON Schema in the request, return one fenced `json`
block, get exactly one repair round-trip on a validation failure, then fail
honestly with no design. `allow_remote` semantics are unchanged.

## Routes

All routes are under `/api/studio` and the existing Host/Origin/body-limit
middleware. All **new** routes also require
`Authorization: Bearer <KALA_STUDIO_TOKEN>`; the existing GET routes
(`/health`, `/integrations`, `/providers`, `/settings`) keep their old
behaviour. POST routes are already guarded by the bearer middleware.

| Method | Path | Request | Response |
|---|---|---|---|
| POST | `/design` | `DesignIntent` | `DesignResult` |
| GET | `/design/{id}/drawing?views=front,top,right,iso` | — | `DrawingResponse` |
| GET | `/design/{id}/section?plane=XZ&offset=4` | — | `SectionResponse` |
| GET | `/journal?after=0&limit=200` | — | `JournalPageResponse` |
| GET | `/journal/verify` | — | `JournalVerifyResponse` |
| POST | `/chat` | unchanged `ChatRequest` | `ChatResponse` + optional `design` |
| POST | `/export` | `{design_id, format:"step"\|"stl"}` | binary file + digest headers |
| GET | `/health` | — | adds `design_available: bool` |

`views` allow-list: `front`, `top`, `right`, `iso` (1–4, no repeats; absent
means all four). `plane` allow-list: `XY`, `XZ`, `YZ` (case-insensitive).
`offset` must be finite and `|offset| <= 10000` mm. `limit` is 1–500.

Error envelope (unchanged): `{"code": str, "message": str, "details": [str]}`.
New statuses/codes: `401 unauthorized`, `404 design_not_found`,
`422 design_rejected`, `503 design_unavailable`, `500 journal_unavailable`.
Existing `invalid_host` (421), `cross_origin_denied` (403),
`payload_too_large` (413), and `invalid_request` (422) apply unchanged.

### `DesignResult` — exact shape

```json
{
  "design_id": "64 lowercase hex chars",
  "label": "string",
  "script": {
    "summary": "string",
    "components": [
      {"name": "string", "steps": [
        {"op": "sketch", "plane": "XY|XZ|YZ", "profile": "rectangle|circle",
         "width": 0.0, "height": 0.0, "diameter": 0.0, "length": 0.0,
         "pitch": 0.0, "angle": 0.0, "x": 0.0, "y": 0.0, "reversed": false}
      ]}
    ],
    "assembly": [
      {"op": "place|fuse|compound", "component": "string|null",
       "components": ["string"], "x": 0.0, "y": 0.0, "z": 0.0}
    ]
  },
  "measured": {
    "volume_mm3": 0.0,
    "surface_area_mm2": 0.0,
    "centroid_mm": [0.0, 0.0, 0.0],
    "bounds": {"min": [0.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0]},
    "size_mm": [0.0, 0.0, 0.0],
    "valid": true,
    "solid_count": 1
  },
  "checks": [
    {"id": "brep_valid", "label": "B-Rep valid",
     "expected": true, "measured": true,
     "tolerance": 0.0, "unit": "bool", "passed": true},
    {"id": "single_solid", "label": "Single solid",
     "expected": 1.0, "measured": 1.0,
     "tolerance": 0.0, "unit": "solid", "passed": true},
    {"id": "envelope_x", "label": "Envelope X",
     "expected": [-30.0, 30.0], "measured": [-30.0, 30.0],
     "tolerance": 0.01, "unit": "mm", "passed": true}
  ],
  "verified": true,
  "mesh": {
    "positions": [0.0, "... flat xyz triples ..."],
    "normals": [0.0, "... flat xyz unit triples ..."],
    "indices": [0, 1, 2, "... zero-based triangle corners ..."],
    "vertex_count": 586,
    "triangle_count": 568,
    "deflection_mm": 0.1,
    "engine": "ocp"
  },
  "artifacts": {"script_sha256": "64 hex", "step_sha256": null},
  "journal_head": "64 hex or null",
  "material": "string or null",
  "tolerances": ["bore diameter 10 mm (declared)", "ISO 2768-m (declared)"],
  "declared_envelope": {"min": [-30.0, 0.0, 0.0],
                        "max": [30.0, 45.0, 40.0]},
  "cached": false,
  "warnings": [],
  "component_count": 2,
  "fused": true
}
```

`expected`/`measured` in a check row is `boolean | number | [number, number]`.
`checks` always contains `brep_valid`; `single_solid` is always present for
Studio designs; `envelope_x|y|z` are present only when the intent carried a
declared envelope (templates do; a raw script does not). `verified` is
`all(passed)`. `script` is the canonical model dump (`exclude_none=True`).

### Sample `DesignResult` (real run, mesh/script abbreviated)

```json
{
  "design_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
  "label": "L-bracket 60 x 45 x 40",
  "script": {
    "summary": "L-bracket 60 x 45 x 40 mm",
    "components": [
      {"name": "base", "steps": [
        {"op": "sketch", "plane": "XY", "profile": "rectangle", "width": 60.0,
         "height": 45.0, "x": -30.0, "y": 0.0, "reversed": false},
        {"op": "extrude", "plane": "XY", "length": 8.0, "x": 0.0, "y": 0.0,
         "reversed": false},
        "... six more sketch/pocket steps ..."
      ]},
      {"name": "upright", "steps": [
        {"op": "sketch", "plane": "XZ", "profile": "rectangle", "width": 60.0,
         "height": 40.0, "x": -30.0, "y": 0.0, "reversed": false},
        {"op": "extrude", "plane": "XY", "length": 8.0, "x": 0.0, "y": 0.0,
         "reversed": false},
        {"op": "sketch", "plane": "XZ", "profile": "circle", "diameter": 10.0,
         "x": 0.0, "y": 24.0, "reversed": false},
        {"op": "pocket", "plane": "XY", "x": 0.0, "y": 0.0, "reversed": false}
      ]}
    ],
    "assembly": [
      {"op": "place", "component": "upright", "components": [], "x": 0.0,
       "y": 8.0, "z": 0.0},
      {"op": "fuse", "components": ["base", "upright"], "x": 0.0, "y": 0.0,
       "z": 0.0}
    ]
  },
  "measured": {
    "volume_mm3": 35571.416047113315,
    "surface_area_mm2": 11889.101731097317,
    "centroid_mm": [1.0866456588108263e-15, 14.752847946637859, 12.282876031570037],
    "bounds": {"min": [-30.0, 0.0, 0.0], "max": [30.0, 45.0, 40.0]},
    "size_mm": [60.0, 45.0, 40.0],
    "valid": true,
    "solid_count": 1
  },
  "checks": [
    {"id": "brep_valid", "label": "B-Rep valid", "expected": true,
     "measured": true, "tolerance": 0.0, "unit": "bool", "passed": true},
    {"id": "single_solid", "label": "Single solid", "expected": 1.0,
     "measured": 1.0, "tolerance": 0.0, "unit": "solid", "passed": true},
    {"id": "envelope_x", "label": "Envelope X", "expected": [-30.0, 30.0],
     "measured": [-30.0, 30.0], "tolerance": 0.01, "unit": "mm", "passed": true},
    {"id": "envelope_y", "label": "Envelope Y", "expected": [0.0, 45.0],
     "measured": [0.0, 45.0], "tolerance": 0.01, "unit": "mm", "passed": true},
    {"id": "envelope_z", "label": "Envelope Z", "expected": [0.0, 40.0],
     "measured": [0.0, 40.0], "tolerance": 0.01, "unit": "mm", "passed": true}
  ],
  "verified": true,
  "mesh": {
    "positions": [-30.0, 0.0, 0.0, -30.0, 0.0, 8.0, "..."],
    "normals": [-0.0, -1.0, -0.0, "..."],
    "indices": [3, 1, 0, 3, 2, 1, "..."],
    "vertex_count": 586,
    "triangle_count": 568,
    "deflection_mm": 0.1,
    "engine": "ocp"
  },
  "artifacts": {
    "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
    "step_sha256": null
  },
  "journal_head": "6924d115383725e16ff6f30ceea7c9496c1d444b9fd0e9d855aa07d65139214a",
  "material": "aluminium",
  "tolerances": [
    "bore diameter 10 mm (declared)",
    "4 x hole diameter 5.5 mm (declared)",
    "ISO 2768-m (declared)"
  ],
  "declared_envelope": {"min": [-30.0, 0.0, 0.0],
                        "max": [30.0, 45.0, 40.0]},
  "cached": false,
  "warnings": [],
  "component_count": 2,
  "fused": true
}
```

### `DrawingResponse` — exact shape and sample

```json
{
  "design_id": "64 hex",
  "views": [
    {
      "name": "front|top|right|iso",
      "visible": [
        {"points": [[0.0, 0.0], "..."], "closed": false, "point_count": 2}
      ],
      "hidden": [ "... same polyline shape ..." ],
      "truncated": false,
      "point_count": 210,
      "polyline_count": 31
    }
  ],
  "chord_tolerance_mm": 0.05,
  "max_points": 20000,
  "point_count": 210
}
```

Sample front view (one visible polyline shown; the rest are `...`):

```json
{
  "design_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
  "views": [{
    "name": "front",
    "visible": [
      {"points": [[-30.0, 0.0], [-30.0, 8.0]], "closed": false, "point_count": 2},
      "... 7 more visible polylines ..."
    ],
    "hidden": ["... 23 hidden polylines ..."],
    "truncated": false,
    "point_count": 210,
    "polyline_count": 31
  }],
  "chord_tolerance_mm": 0.05,
  "max_points": 20000,
  "point_count": 210
}
```

Points are `[u, v]` in view-plane millimetres; the UI places/scales the
views. A closed polyline repeats its first point and sets `closed: true`.
`truncated` is true only when A2's 20k point budget dropped lines.

### `SectionResponse` — exact shape and sample

```json
{
  "design_id": "64 hex",
  "plane": "XY|XZ|YZ",
  "offset_mm": 4.0,
  "loops": [
    {
      "points": [[0.0, 0.0], "..."],
      "area_mm2": 2400.0,
      "area_exact": true,
      "outer": true,
      "bounds": {"min_u": 0.0, "min_v": 0.0, "max_u": 0.0, "max_v": 0.0,
                 "width": 0.0, "height": 0.0,
                 "center_u": 0.0, "center_v": 0.0},
      "centroid": [0.0, 0.0],
      "point_count": 6
    }
  ],
  "area_mm2": 2321.4601836602546,
  "area_exact": true,
  "chord_tolerance_mm": 0.01,
  "truncated": false,
  "outer_loop_count": 1,
  "inner_loop_count": 1,
  "point_count": 56
}
```

Criterion-4 sample (`GET /design/{id}/section?plane=XZ&offset=4`):

```json
{
  "design_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
  "plane": "XZ",
  "offset_mm": 4.0,
  "loops": [
    {
      "points": [[-30.0, 0.0], [-30.0, 8.0], [-30.0, 40.0],
                 [30.0, 40.0], [30.0, 8.0], [30.0, 0.0]],
      "area_mm2": 2399.9999999999995,
      "area_exact": true,
      "outer": true,
      "bounds": {"min_u": -30.0, "min_v": 0.0, "max_u": 30.0,
                 "max_v": 40.0, "width": 60.0, "height": 40.0,
                 "center_u": 0.0, "center_v": 20.0},
      "centroid": [-0.0, 20.0],
      "point_count": 6
    },
    {
      "points": [[0.0, 19.0], [-0.6266661678215213, 19.039426493427612],
                 "... 48 more points ...", [0.6266661678214969, 19.03942649342761]],
      "area_mm2": 78.53981633974483,
      "area_exact": true,
      "outer": false,
      "bounds": {"min_u": -4.990133642141358, "min_v": 19.0,
                 "max_u": 4.990133642141359, "max_v": 29.0,
                 "width": 9.980267284282718, "height": 10.0,
                 "center_u": 4.440892098500626e-16, "center_v": 24.0},
      "centroid": [2.0976145333101254e-16, 23.999999999999986],
      "point_count": 50
    }
  ],
  "area_mm2": 2321.4601836602546,
  "area_exact": true,
  "chord_tolerance_mm": 0.01,
  "truncated": false,
  "outer_loop_count": 1,
  "inner_loop_count": 1,
  "point_count": 56
}
```

`area_mm2` is the kernel-measured net area (holes removed); `area_exact:
false` means A2 had to fall back to a polyline area. `outer` is the
material boundary; `false` is a hole. Use 45° hatching between loops.

### `JournalPageResponse` — exact shape and sample

```json
{
  "events": [
    {
      "sequence": 1,
      "event_type": "design.intent",
      "aggregate_id": "64 hex",
      "payload": {"...": "event-specific object or null"},
      "digest": "64 hex payload sha256",
      "occurred_at": "2026-10-19T16:46:57.617024Z",
      "previous_event_digest": "64 hex",
      "event_digest": "64 hex chain digest"
    }
  ],
  "next_after": 2,
  "has_more": true,
  "limit": 2
}
```

Sample page (`after=0&limit=2`):

```json
{
  "events": [
    {
      "sequence": 1,
      "event_type": "design.intent",
      "aggregate_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
      "payload": {
        "cache_hit": false,
        "design_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
        "envelope": {"max": [30.0, 45.0, 40.0], "min": [-30.0, 0.0, 0.0]},
        "label": "L-bracket 60 x 45 x 40",
        "material": "aluminium",
        "script_sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
        "template": "l_bracket",
        "tolerances": ["bore diameter 10 mm (declared)",
                       "4 x hole diameter 5.5 mm (declared)",
                       "ISO 2768-m (declared)"]
      },
      "digest": "7e6134981f8033d8b3de85ff8cc8d2c10d12210c99ffb901fa22a741e2ec0dbb",
      "occurred_at": "2026-10-19T16:46:57.617024Z",
      "previous_event_digest": "0000000000000000000000000000000000000000000000000000000000000000",
      "event_digest": "c2594c21375c576dbdf45a4efb32b5a3ff7cbb5c19fef28742075b9c8953d07b"
    },
    {
      "sequence": 2,
      "event_type": "artifact.stored",
      "aggregate_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
      "payload": {
        "design_id": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
        "kind": "script",
        "sha256": "88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae",
        "size_bytes": 1457
      },
      "digest": "09d2fc67cfcbef8cad4a4a11e395bb18fd0aecca10a3981f1ac452060dd53dbd",
      "occurred_at": "2026-10-19T16:46:57.654634Z",
      "previous_event_digest": "c2594c21375c576dbdf45a4efb32b5a3ff7cbb5c19fef28742075b9c8953d07b",
      "event_digest": "01509e146a3763fb7b7704e2acde3d44e391ede304fdb387a6e3f2a50859a20f"
    }
  ],
  "next_after": 2,
  "has_more": true,
  "limit": 2
}
```

The page deliberately omits `payload_json` (the raw canonical text); the
`digest` is the SHA-256 of that text and `event_digest` is the chain digest.
Poll with `after=next_after` while `has_more` is true. `next_after` equals
the last returned sequence, or the request's `after` when the page is empty.

### `JournalVerifyResponse` — exact shape and sample

```json
{
  "ok": true,
  "event_count": 5,
  "head_digest": "6924d115383725e16ff6f30ceea7c9496c1d444b9fd0e9d855aa07d65139214a",
  "first_bad_sequence": null
}
```

After a payload byte is tampered through sqlite:

```json
{
  "ok": false,
  "event_count": 5,
  "head_digest": "ffb0b0fb354423ea20ce6133455579546b5114a21a88824f87ced9928cd76ba0",
  "first_bad_sequence": 4
}
```

Show `HASH-CHAIN OK` only when `ok` is true. A broken chain still serves
`/journal` history; `verify` is the authority.

### `ChatResponse` with an optional design

```json
{
  "text": "string (<= 4096 chars)",
  "provider": "local|openai|openrouter|codex",
  "model": "string or null",
  "offline": true,
  "design": "DesignResult or null"
}
```

Sample local L-bracket reply (text after the latest fix; `design` is the
same object shown above):

```json
{
  "text": "Local helper. Design intent parsed from declared values. 4x M5 mapped to 5.5 mm clearance holes (ISO 273 medium). Plate width 60 mm and depth 45 mm are canonical template defaults; state them to override. Compiling and measuring with OpenCASCADE. L-bracket 60 x 45 x 40: B-Rep verified. Measured volume 35571.416 mm^3, surface area 11889.102 mm^2, envelope 60.000 x 45.000 x 40.000 mm. design_id 88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae.",
  "provider": "local",
  "model": "deterministic-paraphrase-v1",
  "offline": true,
  "design": {"...": "DesignResult, abbreviated here"}
}
```

For `"make me a bracket"` the response has `"design": null` and text:
`"Local helper. I can draft an L-bracket, but I need these declared values:
width (mm), depth (mm), height (mm), wall thickness (mm). No design was
compiled."`

### Design export

Request: `{"design_id": "<64 hex>", "format": "step"}` (or `"stl"`).
Response: the file bytes with `content-type: application/step` or
`model/stl` and headers:

```text
content-disposition: attachment; filename="kala-studio-88a7ec139868.step"
x-studio-design-id: 88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae
x-studio-digest: <sha256 of the response bytes>
x-studio-artifact-kind: step
```

The same digest is stored as an artifact and appears in the next
`DesignResult.artifacts.step_sha256` for this process. STL starts with the
80-byte header `kala-studio design mesh, binary STL export`.

## Acceptance evidence

- **Criterion 2** — `POST /design` with `l_bracket(width=60, depth=45,
  height=40, wall=8, bore_d=10, hole_d=5.5, hole_count=4)` returns
  `volume_mm3 = 35571.416047113315` (relative error ~2.5e-15 vs the ADR's
  35571.416047), `bounds.min = [-30, 0, 0]`, `bounds.max = [30, 45, 40]`,
  one solid, valid, `verified = true`, and all five checks pass. This is
  `test_design_api.py::test_criterion_2_lbracket_end_to_end` and
  `test_service.py::test_design_measures_the_adr_lbracket`.
- **Criterion 4** — `GET /section?plane=XZ&offset=4` on the same design
  returns exactly 1 outer and 1 inner loop, outer 60.0000 × 40.0000 mm,
  inner Ø10 at (0, 24), and `area_mm2 = 2321.4601836602546` (the exact
  `2400 - 25π`; `area_exact = true`, `truncated = false`). Tested at the
  service and HTTP levels.
- **Criterion 5** — `/journal/verify` after a design is `ok: true`; a
  single payload byte mutated through sqlite (`payload_json || ' '`) gives
  `ok: false` with `first_bad_sequence: 4`.
- **Criterion 6** — `"make me a bracket"` returns `design: null` and names
  width, depth, height, and wall thickness. The Stitch sentence
  (`Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm
  wall thickness, with a 10 mm centered bore and 4x M5 corner mounting
  holes.`) returns the criterion-2 volume, states the ISO 273-medium Ø5.5
  clearance, and labels the 60/45 plate values as canonical defaults.
- **Criterion 7** — every new route rejects a missing/wrong bearer
  (401), a non-loopback Host (421), a cross-origin Origin (403), and an
  oversized body (413); `views`, `plane`, `offset`, `after`, and `limit` are
  allow-listed/bounded (422 on refusal).
- **Idempotency** — the same script twice gives the same `design_id`; the
  journal has one `design.compiled` and the second `design.intent` has
  `cache_hit: true`.
- **Concurrency** — 8 parallel `POST /design` calls all return 200, share
  one `design_id`, write one `design.compiled`, and `/journal/verify` is ok.
- **No partial artifact on refusal** — a `helix` script returns 422
  `design_rejected`, the journal has `design.intent` + `design.rejected`,
  no `design.compiled`, and `journal.has_artifact(design_id)` is false.

## Gates

Run from the repository root with `.venv/bin/python` (CPython 3.14.8).

| Gate | Command | Result |
|---|---|---|
| New tests | `pytest tests/unit/studio_design/test_templates.py test_intent.py test_service.py test_design_api.py -q` | **84 passed** in 2.5 s (20 templates, 17 intent, 18 service, 29 API) |
| Studio design suite (A1+A2+A3+B1) | `pytest tests/unit/studio_design -q` | **179 passed** in 3.0 s |
| Boundary/portable imports | `pytest tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q` | **16 passed** |
| Full unit suite | `pytest tests/unit -q` | **1445 passed, 1 warning** in 8.4 s |
| Security suite | `pytest tests/security -q` | **189 passed, 1 warning** in 15.0 s |
| Ruff check (12 touched files) | `ruff check ...` | **All checks passed** |
| Ruff format check | `ruff format --check ...` | **12 files already formatted** |
| Mypy source (literal strict) | `mypy` on the 8 touched source files | **Success: no issues found in 8 source files** |
| Mypy tests | `MYPYPATH=src mypy --python-version 3.14` on the 4 new test files | **Success: no issues found in 4 source files** |

The one warning is the pre-existing `StarletteDeprecationWarning` from
`fastapi.testclient` (`httpx` vs `httpx2`). The literal mypy gate on test
files is blocked by the pre-existing environment issue recorded by A1/A2/A3:
`pyproject.toml` pins `python_version = "3.11"` while the venv is 3.14 and
numpy 2.5 uses a 3.12+ `type` statement. It reproduces on untouched test
files. With `--python-version 3.14` (matching the venv) the test files pass.
No project file was changed to mask this.

## Gaps and honest notes

1. **Remote design prompt is not a true `system` role.** B1's ownership
   excludes `providers/openai_compat.py`, which sends one user message. The
   DesignIntent instruction and JSON Schema are therefore prepended to the
   user content in `dispatch._design_prompt`. Semantics are the same; the
   provider sees the schema and the one-repair rule. Moving it to a real
   system message is a small follow-up for whoever owns that file.
2. **The housing template is five prisms, not one box-plus-pocket.** The v1
   compiler's pocket cuts along the sketch normal (+Z for XY), so a
   single-pad box would open downward. A floor plus four overlapping walls
   fused into one solid produces the specified `+Z`-open floor-and-wall
   envelope and the exact shell volume; the construction is documented at
   the top of `templates.py`.
3. **The Stitch sentence needs two canonical defaults.** It states the
   height (40), wall (8), bore (10), and 4× M5 but not the 60 × 45 plate.
   The local parser fills only those two missing plate dimensions from the
   documented canonical defaults and says so in the reply. No other value
   is guessed; every other missing value is listed and no design is built.
4. **`housing` in the UI starter is cylindrical.** The "Housing Envelope"
   starter prompt is a cylindrical housing with no wall thickness; it is
   not the rectangular `housing_envelope` template, so the local parser
   leaves it on the existing analytic/non-design path rather than forcing
   the wrong family.
5. **New GET routes require the bearer.** ADR criterion 7 asks every new
   route to reject a missing/wrong bearer, so `/journal`, `/journal/verify`,
   `/drawing`, and `/section` require `Authorization: Bearer ...`. Existing
   GET routes keep their old behaviour. C1 must send the studio token on
   those polls.
6. **`single_solid` is always expected.** A raw script that leaves an
   unfused multi-part compound is rejected by the `single_solid` check
   rather than presenting a compound volume as one part. Per-component
   measurements are not exposed.
7. **Mesh deflection is 0.1 mm**, not an ADR-pinned number. It is a display
   tessellation; drawing uses 0.05 mm and section 0.01 mm from A2. The mesh
   is refused (not truncated) above 200k vertices / 400k triangles.
8. **STEP export writes a temporary file** under `<data_dir>/exports` and
   removes it after reading; the bytes are then content-addressed. OCP's
   `STEPControl_Writer` prints its transfer statistics to stdout; that is
   kernel noise, not an API field.
9. **`step_sha256` is process-local.** `/design` reports the last STEP
   digest exported in this process. After a restart the artifact and the
   `export.completed` event are still in the journal, but a fresh
   `DesignResult` shows `step_sha256: null` until the next export (the
   digest is still discoverable from the journal).
10. **No live remote-provider call was made.** The remote hook is tested
    with a fake `openai_compat_chat` (schema presence, one repair, honest
    failure, non-design passthrough). Live network calls remain behind
    `allow_remote=true` and the fixed endpoints.
11. **No frozen tolerances/fit analysis.** Every `tolerances[]` string is
    declared by the template or caller and is labelled `(declared)`. The
    service does not compute fits, tolerance stacks, or manufacturability.
12. **The journal is tamper-evident, not tamper-proof.** `head_digest` must
    be anchored outside the database for a real audit.
13. **`design_id` covers the script only.** Material, tolerances, and label
    are declared metadata and are not part of the content digest. The same
    script posted with different declared metadata is a cache hit and
    returns the originally compiled design's metadata; the second
    `design.intent` still records the new intent with `cache_hit: true`.
    The geometry is identical by construction because the script is.
14. **Remote replies are capped at 4096 characters** by the existing
    `openai_compat` reader. A verbose model reply can therefore be cut
    before its JSON block ends; the one repair round-trip is the only
    recovery. Templates produce compact intents, so the normal path fits.
