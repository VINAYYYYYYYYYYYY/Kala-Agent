# r1-worker-mode — Python Studio geometry worker (ADR-0010 + ADR-0011)

Cell: `r1-worker-mode`. Specs: ADR-0010 "Geometry worker (v1)" and
ADR-0011 "External CAD builds, OpenCascade views (builds only on request)".
Status: **done** for the Python worker contract. No commit, no push, no
delegation, no credential reads.

Round 2 change: OpenCascade now has two explicit roles. The existing
`design` / `drawing` / `section` routes are the **opt-in builder**
(`provider: "opencascade"`) and keep their previous behaviour. The new
`POST /internal/v1/import` is **view-only**: it reads a STEP file exported by
an external provider (Solid Edge or FreeCAD), tessellates it for display, and
stores the original bytes. It never modifies or re-exports the shape.

## Files

| File | Note |
|---|---|
| `src/kala_agent/studio/worker.py` | new (round 1) — process lifecycle, readiness line, signals, stdin lifeline |
| `src/kala_agent/studio/internal_api.py` | new (round 1), extended — gates, internal router, intent/journal routes, view-only import route, provider labels |
| `src/kala_agent/studio/design/importer.py` | new (round 2) — `STEPControl_Reader` → `BRepMesh` → artifact store |
| `src/kala_agent/studio/cli.py` | minimal `--worker` wiring only (round 1; unchanged in round 2) |
| `tests/unit/studio_worker/test_worker_process.py` | new subprocess contract tests (+ real export→import round trip) |
| `tests/unit/studio_worker/test_worker_http.py` | in-process gate/route/design/journal/intent tests (+ provider labels) |
| `tests/unit/studio_worker/test_worker_import.py` | new view-only STEP import tests |

Nothing outside the cell's ownership was edited. In particular
`design/service.py` and `server.py` were **not** touched; the worker mounts
their existing handlers unchanged. The importer reuses the service's
process-wide `_GEOMETRY_LOCK` so an import can never interleave OCP work with
a compile, measure, drawing, or export.

## Process contract

Command: `python -m kala_agent.studio --worker`

- Binds `127.0.0.1:0` and listens before announcing readiness.
- Reads the bearer token **only** from `KALA_WORKER_TOKEN`. Missing, shorter
  than 32 characters, or outside `[A-Za-z0-9._~-]` (max 128) → exit 2 with
  no stdout.
- Prints exactly one stdout line, then redirects process stdout to stderr:

  ```
  KALA_WORKER_READY {"design_available":true,"port":54321,"version":"0.1.0"}
  ```

  Keys are sorted and the JSON is compact. `version` is `STUDIO_VERSION`.
  `design_available` is `true` only when `OCP` is importable. The token is
  never printed.
- Logs go only to stderr. Uvicorn access log is off; proxy headers are off.
- `KALA_WORKER_STDIN_LIFELINE=1` → EOF on stdin stops the worker (no orphan
  when the Rust supervisor dies). Without it, stdin is ignored.
- SIGTERM/SIGINT → graceful shutdown, exit 0.
- No `/api/studio/*`, settings, credential, or provider route is mounted, so
  the worker makes no provider network call.

## Gate contract (every route, every method)

Applied in this order: security headers → Host → body size → Origin → bearer.

| Condition | Status | Body `code` |
|---|---|---|
| `Host` is not exactly `127.0.0.1:<bound port>` | 421 | `invalid_host` |
| Body larger than `262_144` bytes (Studio `MAX_REQUEST_BODY_BYTES`) | 413 | `payload_too_large` |
| Any `Origin` header present (including same-origin) | 403 | `cross_origin_denied` |
| Missing/wrong `Authorization: Bearer <KALA_WORKER_TOKEN>` | 401 | `unauthorized` (+ `www-authenticate: Bearer realm="kala-studio-worker"`) |

Every error body is the flat Studio envelope:

```json
{"code":"...","message":"...","details":["..."]}
```

Bearer is required on GET routes too. Unknown paths are 404 `not_found`;
unknown methods are 405.

## Internal route contract (`/internal/v1`)

All success bodies are JSON unless marked binary. 13 routes exactly.

### `POST /preview`

Request (`PreviewRequest`): `{"kind":"box"|"cylinder"|"sphere", "parameters"?: {string: number|string}}` (extras refused).
Response 200 (`PreviewResponse`): `{positions:number[], normals:number[], indices:int[], bounds:{min:[x,y,z],max:[x,y,z]}, engine:string, warnings:string[]}`.
Errors: 422 `invalid_request`.

### `POST /export`

Request (`ExportRequest`): exactly one of
`{"kind":"box"|"cylinder"|"sphere","parameters"?}` or `{"design_id":"<64 hex>"}`,
plus `{"format":"stl"|"step"}`.
Response 200 is **binary** (`model/stl` or `application/step`) with
`content-disposition: attachment; filename="..."`; a stored-design export
also carries `x-studio-design-id`, `x-studio-digest`, `x-studio-artifact-kind`.
Errors: 404 `design_not_found`; 422 `invalid_request` / `design_rejected`;
503 `design_unavailable` / `step_unavailable`.

### `POST /handoff`

Request (`HandoffRequest`): `{"integration_id":"...", "kind":"box"|"cylinder"|"sphere", "parameters"?: {...}, "format":"stl"|"step"}`.
Response 200 binary with `content-disposition` and `x-studio-handoff`.
Errors: 422 `unknown_integration` / `handoff_format_unsupported` /
`invalid_request`; 503 `step_unavailable`.

### `POST /design` — provider `opencascade` (opt-in builder)

Request (`DesignIntent`): exactly one of `template` (an `l_bracket`,
`stepped_shaft`, or `housing` call) or `script` (a `ConstructionScript`),
plus optional declared `material`, `tolerances`, `label`.
Response 200 (`DesignResult`):

```json
{
  "design_id":"<sha256 of canonical script>",
  "label":"...",
  "script":{...},
  "measured":{"volume_mm3":35571.416047,"surface_area_mm2":...,"centroid_mm":[x,y,z],
              "bounds":{"min":[x,y,z],"max":[x,y,z]},"size_mm":[x,y,z],
              "valid":true,"solid_count":1},
  "checks":[{"id":"...","label":"...","expected":...,"measured":...,
             "tolerance":...,"unit":"...","passed":true}],
  "verified":true,
  "mesh":{"positions":[...],"normals":[...],"indices":[...],"vertex_count":N,
          "triangle_count":N,"deflection_mm":0.1,"engine":"ocp"},
  "artifacts":{"script_sha256":"<design_id>","step_sha256":null|"<64 hex>"},
  "journal_head":"<64 hex>"|null,
  "material":"aluminium"|null,
  "tolerances":[...],
  "declared_envelope":{"min":[...],"max":[...]}|null,
  "cached":false,
  "warnings":[...],
  "component_count":1,
  "fused":true
}
```

Errors: 422 `invalid_request` (framework validation) / `design_rejected`;
503 `design_unavailable`. The ADR-0009 L-bracket measures
`volume_mm3 = 35571.416047` and bounds `[-30,0,0] … [30,45,40]`.
Route metadata: `x-kala-provider: "opencascade"`, `x-kala-provider-mode:
"opt-in"`; description says it is never selected automatically.

### `GET /design/{design_id}/drawing` — view of an `opencascade` design

Query: `views` optional, comma-separated from `front,top,right,iso`
(the service default is all four).
Response 200 (`DrawingResponse`): `{design_id, views:[{name, visible:[{points:[[u,v],...], closed, point_count}], hidden:[...], truncated, point_count, polyline_count}], chord_tolerance_mm, max_points, point_count}`.
Errors: 404 `design_not_found`; 422 `invalid_request`; 503 `design_unavailable`.
Route metadata as `/design`.

### `GET /design/{design_id}/section` — view of an `opencascade` design

Query: `plane=XY|XZ|YZ` (default `XZ`), `offset` finite mm, `|offset| <= 10000`.
Response 200 (`SectionResponse`): `{design_id, plane, offset_mm, loops:[{points:[[u,v],...], area_mm2, area_exact, outer, bounds:{min_u,min_v,max_u,max_v,width,height,center_u,center_v}, centroid:[u,v], point_count}], area_mm2, area_exact, chord_tolerance_mm, truncated, outer_loop_count, inner_loop_count, point_count}`.
Errors: 404 `design_not_found`; 422 `invalid_request`; 503 `design_unavailable`.
Route metadata as `/design`.

### `POST /import` — view-only STEP import (ADR-0011)

Request: the raw STEP file as the request body (recommended
`content-type: application/step`; the header is not required). Maximum
`262_144` bytes — the same Studio body limit. The bytes must start with
`ISO-10303-21`.

Response 200 (`StepImportResponse`):

```json
{
  "view": {
    "positions": [x0,y0,z0, ...],
    "normals":   [nx0,ny0,nz0, ...],
    "indices":   [i0,i1,i2, ...],
    "bounds":    {"min": [x,y,z], "max": [x,y,z]}
  },
  "step_sha256": "<sha256 of the exact uploaded bytes>",
  "solids": 1,
  "warnings": []
}
```

- `view` is the `BRepMesh_IncrementalMesh` tessellation (0.1 mm / 0.5 rad)
  read back face by face, inside `MAX_MESH_VERTICES = 200_000` and
  `MAX_MESH_TRIANGLES = 400_000`. A mesh over either limit is refused, never
  truncated.
- `bounds` is the min/max of the displayed mesh vertices, not a re-measured
  mass property.
- `solids` is a structural `TopExp_Explorer` count. No volume, area, or
  centroid is reported; OpenCascade is not presented as the authority.
- `step_sha256` is the key in the session artifact store
  (`<data_dir>/artifacts`). The original bytes are stored unchanged. Two
  journal events are appended: `artifact.stored` (`kind:"step"`,
  `source:"step_import"`) and `view.imported` (`viewer:"opencascade"`,
  `role:"view-only"`, counts, bounds, warnings).
- Warnings are honest display notes, never a silent substitution: an
  unreadable shape is a 4xx, an invalid-but-readable shape is displayed with
  a warning, and a STEP with no solids says so.

Errors:

| Status | `code` | Cause |
|---|---|---|
| 413 | `payload_too_large` | body over 262 144 bytes (middleware) |
| 422 | `invalid_step` | empty body, no ISO-10303-21 header, reader refusal, no transferable root, null shape |
| 422 | `step_too_complex` | tessellation exceeds the mesh limits |
| 503 | `design_unavailable` | OCP is not importable |
| 500 | `journal_unavailable` | the session journal chain cannot be extended |

The importer never writes a STEP file back, never calls a STEP writer, and
never edits the imported shape; only the display triangulation is added in
memory.

### `GET /journal`

Query: `after` integer ≥ 0 (default 0), `limit` 1..500 (default 200).
Response 200 (`JournalPageResponse`): `{events:[{sequence, event_type, aggregate_id, payload, digest, occurred_at, previous_event_digest, event_digest}], next_after, has_more, limit}`.

### `GET /journal/verify`

Response 200: `{ok:bool, event_count:int, head_digest:string|null, first_bad_sequence:int|null}`.

### `POST /journal/append`

Request: `{"event_type":"chat.turn"|"settings.changed"|"provider.call"|"export.completed", "payload":{...}, "aggregate_id"?: "..."}`.
Only the allow-listed event types are accepted. The payload must be a JSON
object whose canonical JSON is at most 65 536 bytes (and within the HTTP body
limit).
Response 201 (`JournalEventRecord`): the appended event with `sequence`,
`payload`, `digest`, `occurred_at`, `previous_event_digest`, `event_digest`.
Errors: 422 `event_type_not_allowed`; 422 `invalid_request` (payload not an
object, non-canonical, or too large); 500 `journal_unavailable`.

### `POST /intent/parse`

Request: `{"prompt":"..."}` (1..4096 chars).
Response 200: `{family:string|null, intent:DesignIntent|null, missing:string[], notes:string[]}`.
The local strict parser returns `family:null` for a non-design prompt, an
`intent` only when every required value is stated or a documented template
default applies, and otherwise `missing` listing exactly what to ask.
Example: the Stitch sentence → complete L-bracket; `"make me a bracket"` →
`missing:["width (mm)","depth (mm)","height (mm)","wall thickness (mm)"]`.

### `POST /intent/validate`

Request: `{"text":"..."}` (1..65 536 chars), a model reply that may contain a
fenced JSON `DesignIntent`.
Response 200: `{"intent":{...}}`.
Errors: 422 `invalid_intent` with bounded parser details.

### `GET /intent/schema`

Response 200 `text/plain`: the pretty-printed `DesignIntent` JSON Schema used
in remote-provider prompts.

## Tests and gates

Commands and results on this host (`.venv`, Python 3.14):

| Gate | Result |
|---|---|
| `python -m pytest tests/unit/studio_worker -q` | **34 passed**, 1 warning |
| `python -m pytest tests/unit -q` | **1479 passed**, 1 warning |
| `ruff check <owned paths>` | **pass** |
| `ruff format --check <owned paths>` | **pass** (7 files) |
| `mypy <owned paths>` | **pre-existing environment failure**, see below |
| `mypy --python-version 3.12 <owned paths>` | **Success: no issues found in 7 source files** |

`mypy` with the repo config (`python_version = "3.11"`) fails before checking
any project file:

```
.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error: Type
statement is only supported in Python 3.12 and greater  [syntax]
```

The venv is Python 3.14 and its numpy stubs use PEP 695 syntax, which mypy
refuses under the configured 3.11 target. This is not caused by this cell;
raising only the target for the same paths reports no issues.

Coverage highlights:

- Real subprocess: readiness line parsed, `design` returns
  `volume_mm3 = 35571.416047`, `export` produces ISO STEP, `import` returns
  the same L-bracket bounds `[-30,0,0]…[30,45,40]`, `solids = 1`; clean
  SIGTERM and stdin-EOF shutdown with empty stdout (proves the kernel's STEP
  chatter does not pollute the readiness line).
- Token missing/31 chars/unsupported character → refuse to start; wrong or
  missing bearer → 401; bad Host → 421; any Origin → 403; oversize → 413.
- Journal append allow-list and chain-continuation; intent parse/validate;
  design/drawing/section behaviour unchanged.
- Import: exported L-bracket round trip, original bytes retained under their
  digest, `artifact.stored` + `view.imported` journalled, malformed and empty
  STEP → 422 with no artifact stored, oversize → 413.
- Route contract: exactly the 13 documented paths, no `/api/*`, and the
  builder routes carry the opt-in `opencascade` metadata.

## Gaps and honest notes

1. **Drawing/section apply to OpenCascade-built designs, not arbitrary
   imports.** ADR-0011 describes the display aids as views of the imported
   model; this round implements the import+tessellation route only. Rendering
   HLR/sections from an imported `TopoDS_Shape` needs a stored shape handle
   (or re-reading the artifact) and is left to the follow-up worker.
2. **Import does not identify the provider.** The route receives bytes and
   cannot honestly claim Solid Edge or FreeCAD, so the journal records
   `viewer:"opencascade"` and `role:"view-only"` without a `provider` field.
   Rust knows the provider and can append `provider.build.completed` /
   `artifact.stored` itself.
3. **The upload ceiling is 256 KiB**, inherited from the Studio body limit as
   required. A large assembly STEP exported by a provider may exceed it; that
   must raise the shared Studio limit, not bypass the gate.
4. **Bounds are view bounds.** They come from the display mesh, not from
   `BRepGProp`; no volume/area is reported for imports, so OpenCascade is
   never presented as the measuring authority.
5. **`view.imported` is not in `JournalEventType`.** The journal accepts any
   safe identifier; the enum remains the ADR-0009 list and was not edited
   (library-owned). Consumers should match the string.
6. **The importer depends on `DesignService`'s process-wide `_GEOMETRY_LOCK`**
   for OCP thread safety. If the service module is refactored, keep one
   process-wide lock shared by both paths or move the lock to a shared module.
7. **STEP success and error messages come from OCP.** Reader/writer statistics
   are kernel noise on stdout/stderr; in worker mode stdout is already
   redirected to stderr after the readiness line, so the JSON line stays the
   only stdout output.
