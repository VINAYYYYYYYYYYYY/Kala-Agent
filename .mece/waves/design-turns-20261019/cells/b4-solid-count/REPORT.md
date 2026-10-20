# B4 solid-count — assemblies verify with their own occurrence count

Cell: `b4-solid-count`. Read: `../b3-freecad-adapter/REPORT.md`,
`../b2-providers/REPORT.md`, `docs/construction-semantics.md` ("Assembly
occurrences"), ADR-0009, and the current `service.py`/`measure.py`/tests.

Status: **done** for the owned surface. No commit, push, deployment,
delegation, or credential reads. Only the files below were edited; `studio/**`
and the Rust crates were not touched.

## The defect this closes

B3 made a place-only multi-component script export a real assembly of N placed
solids. B2's `build_provider_checks` always emitted `single_solid` with
expected 1 and therefore failed an honest two- or three-part assembly, making
`verified` false. An assembly is not a broken single solid: the construction
script itself declares how many solids its result has.

## What changed

| File | Change |
|---|---|
| `src/kala_agent/studio/design/service.py` | new `expected_solid_count(script)`; `build_provider_checks` takes `expected_solids` (default 1) and emits `single_solid` for one-solid results or `solid_count` for assemblies; both callers (fresh build and cold journal reload) derive the count from the script |
| `src/kala_agent/studio/design/measure.py` | one named constant `SOLID_COUNT_TOLERANCE = 0.0` (exported); the OCP `build_checks` path is unchanged |
| `tests/unit/studio_design/test_solid_count.py` | new: derivation cases and the four check-row states |
| `tests/unit/studio_design/test_cad_providers.py` | fake-provider integration: wrong assembly count fails, missing assembly count is `not_reported` |
| `tests/unit/studio_design/test_cad_providers_freecad.py` | real FreeCAD service tests: L-bracket keeps `single_solid`, two- and three-part place-only assemblies verify with `solid_count` 2 and 3, plus a cold-reload check |
| `tests/unit/studio_design/test_design_api.py` | API shape test: a place-only assembly returns the `solid_count` row and `verified: true` |

No request/response schema changed: `DesignCheck` was already generic, and
`measured.solid_count` already existed from B2.

## Derivation: `expected_solid_count(script)`

The count is read only from the `ConstructionScript`; it is never inferred from
the provider result. Per `docs/construction-semantics.md`, "Assembly
occurrences", a place-only assembly is a set of occurrences, one solid each.

| Script shape | Expected solids | Reason |
|---|---|---|
| no assembly steps (e.g. the L-bracket) | **1** | the script builds one part |
| one `place` (including `place 0,0,0`) | **1** | one occurrence; still a single solid, so `single_solid` stays |
| two/three `place` steps | **2 / 3** | one occurrence per placed component |
| `fuse` of members | **1** | a fuse declares one joined solid |
| `compound` of members | **number of members** | a compound keeps its members as separate solids |
| component named by a join and also placed | counted **once** | a join takes over the component's occurrence |
| assembly steps that place nothing | **1** | the exported result is the single tip feature |

The derivation does not invent a number: an unsupported multi-join script is
outside the v1 assembly semantics (see honest note 3).

## Check building

`build_provider_checks(..., expected_solids=1, ...)`:

- `expected_solids == 1` → `single_solid`, label **Single solid**, expected
  `1.0`, unit `solid`, tolerance `0.0`, exactly as B2.
- `expected_solids > 1` → `solid_count`, label **Solid count**, expected
  `float(expected_solids)`, unit `solid`, tolerance `0.0`.
- `measured` is always the provider's `ProviderBuild.solid_count` as a float.
  A missing count is `null` with `status: "not_reported"` and `passed: false`;
  no value is substituted.
- `verified` remains `all(check.passed for check in checks)`, so an assembly
  verifies when its provider count matches the script and fails when it does
  not. A wrong count on a one-solid script still fails `single_solid`.
- The new keyword defaults to `1`, so a caller that does not pass it keeps the
  previous single-solid behavior.

Both call sites pass the derivation: `_design_locked` uses
`resolved.script`, `_load_locked` uses the script parsed back from the stored
script artifact. A cached design therefore does not regress to `single_solid`.

## JSON shape notes for C2 and the Rust mirror

`DesignResult` shape is unchanged. The only observable change is which count
check id can appear. Capture from a real FreeCAD 1.1.4 run of
`assembly_two_parts_placed_once`:

```json
{
  "provider": "freecad",
  "provider_version": "1.1.4",
  "measured_by": "FreeCAD",
  "measured": {
    "volume_mm3": 1000.0,
    "surface_area_mm2": 800.0,
    "solid_count": 2,
    "valid": true
  },
  "checks": [
    {"id": "brep_valid", "label": "B-Rep valid", "expected": true,
     "measured": true, "tolerance": 0.0, "unit": "bool",
     "passed": true, "status": "passed"},
    {"id": "solid_count", "label": "Solid count", "expected": 2.0,
     "measured": 2.0, "tolerance": 0.0, "unit": "solid",
     "passed": true, "status": "passed"}
  ],
  "verified": true,
  "component_count": 2,
  "fused": false
}
```

A single-solid design keeps the existing row (real L-bracket run):

```json
{"id": "single_solid", "label": "Single solid", "expected": 1.0,
 "measured": 1.0, "tolerance": 0.0, "unit": "solid",
 "passed": true, "status": "passed"}
```

The failure and missing states for an assembly:

```jsonc
// provider reported 1 solid, script declares 2
{"id": "solid_count", "label": "Solid count", "expected": 2.0,
 "measured": 1.0, "tolerance": 0.0, "unit": "solid",
 "passed": false, "status": "failed"}

// provider reported no solid count (e.g. FCStd reopen failed)
{"id": "solid_count", "label": "Solid count", "expected": 2.0,
 "measured": null, "tolerance": 0.0, "unit": "solid",
 "passed": false, "status": "not_reported"}
```

For C2:

1. Match count rows by `id`, not by label: `single_solid` (one-solid result) or
   `solid_count` (assembly). Exactly one of the two is present; `brep_valid` is
   always present.
2. `solid_count` with `status: "passed"` is the normal, honest state for an
   assembly. Do not render it as a defect or as "not a solid part". Show
   `verified: true` when all rows pass.
3. Render `expected`/`measured` as an em dash when `measured` is `null`;
   `not_reported` is neither a pass nor a failure, and `verified` is false.
4. Do not read a count from the view (`view.solids`) or recompute one; the check
   row is the provider's measured fact against the script's declared count.
5. `measured.solid_count` remains the same integer field as in B2 and agrees
   with the row's `measured` value. `component_count` and `fused` are unchanged
   diagnostics; `fused` is false for a place-only assembly. `component_count`
   counts component definitions, not occurrences, so the check row is the
   authoritative expected count.

For the Rust mirror:

1. No schema change is required. `checks[]` already carries
   `{id,label,expected,measured,tolerance,unit,passed,status}`.
2. Accept the id `solid_count` alongside `single_solid`. Do not normalize,
   rename, or drop either row.
3. `expected`/`measured` for a count row are JSON numbers serialized from
   floats (`2.0`, not `"2"`). `measured.solid_count` stays an integer. Compare
   exactly (tolerance 0.0); do not round.
4. Keep `verified` as the conjunction of `checks[].passed`; a `not_reported`
   row must make it false and must not be counted as a pass.
5. A cached/reloaded design recomputes the expected count from the stored
   script, so persisted results do not need a stored "expected solids" field.

## Acceptance evidence

| Criterion | Test | Result |
|---|---|---|
| L-bracket unchanged: `single_solid` passes, `verified` true | `test_cad_providers_freecad.py::test_the_l_bracket_keeps_its_single_solid_check`; existing `test_service.py::test_design_measures_the_adr_lbracket` | `single_solid` expected/measured 1.0, `solid_count` absent, `verified: true`; FreeCAD 1.1.4 |
| two-part place-only assembly via real FreeCAD | `...::test_a_place_only_assembly_verifies_with_its_own_solid_count[assembly_two_parts_placed_once-2]` | `solid_count` expected/measured 2.0 passed, `verified: true`; volume `1000.0` mm³, 2 solids |
| three-part place-only assembly via real FreeCAD | `...::test_a_place_only_assembly_verifies_with_its_own_solid_count[assembly_three_parts_placed_once-3]` | `solid_count` expected/measured 3.0 passed, `verified: true` |
| cold reload keeps the assembly check | same test, fresh `DesignService.load(outcome.design_id)` | `solid_count` 2/2 and 3/3 passed, `verified: true` |
| wrong provider count fails | `test_cad_providers.py::test_a_provider_solid_count_must_match_a_multi_part_script` | fake build is 1 solid, script declares 2: `solid_count` 2 vs 1 `failed`, `verified: false` |
| missing count is `not_reported` | `test_cad_providers.py::test_a_missing_assembly_solid_count_is_not_reported` | `measured: null`, `status: "not_reported"`, `verified: false` |
| API JSON carries the assembly row | `test_design_api.py::test_criterion_2_place_only_assembly_verifies_with_a_solid_count` | `opencascade` build of a two-part script: `solid_count` 2/2 passed, `view.solids` 2, `verified: true` |
| derivation cases | `test_solid_count.py` | single part 1, one place 1, two/three places 2/3, fuse 1, compound 2, joined-and-placed counted once |
| one-solid mismatch still fails | `test_solid_count.py::test_a_multi_solid_result_still_fails_a_single_part_script` | `single_solid` 1 vs 2 `failed` |

Exact real FreeCAD assembly evidence (captured through `DesignService`):

```
design_id     c60029cd22ce0784459c1c8cc638dba7daaa83cc24be8680b3bf7a06b8834e1e
provider      freecad 1.1.4
measured      volume 1000.0 mm^3, area 800.0 mm^2, solid_count 2, valid True
checks        brep_valid passed; solid_count expected 2.0 measured 2.0 passed
verified      true
components    2    fused false
```

## Gates (this host, `.venv` CPython 3.14.8, FreeCAD 1.1.4)

| Gate | Command | Result |
|---|---|---|
| New + provider unit tests | `.venv/bin/pytest tests/unit/studio_design/test_solid_count.py tests/unit/studio_design/test_cad_providers.py -q` | **32 passed**, 1 pre-existing `StarletteDeprecationWarning` |
| Real FreeCAD module | `.venv/bin/pytest tests/unit/studio_design/test_cad_providers_freecad.py -q -m freecad` | **8 passed** |
| Design suite | `.venv/bin/pytest tests/unit/studio_design -q` | **227 passed**, same warning |
| Full unit suite | `.venv/bin/pytest tests/unit -q` | **1533 passed**, same warning |
| Security suite | `.venv/bin/pytest tests/security -q` | **189 passed** |
| Boundary/portable imports | `.venv/bin/pytest tests/unit/test_adapter_boundaries.py tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q` | **18 passed** |
| Ruff check | 6 touched files | **All checks passed** |
| Ruff format | 6 touched files | **6 files already formatted** |
| Mypy source | `service.py`, `measure.py` | **Success: no issues found in 2 source files** |
| Mypy tests | 4 touched/created test files, `MYPYPATH=src --python-version 3.14` | **Success: no issues found in 4 source files** |

The `--python-version 3.14` for test files is the pre-existing environment
mismatch recorded by A1/A2/A3/B1/B2/B3: the venv is 3.14 while `pyproject.toml`
pins `python_version = "3.11"`, and numpy's stubs use a 3.12+ `type` statement.
It reproduces on untouched test files; no project setting was loosened.

## Honest notes

1. **A fuse is expected to produce one solid.** FreeCAD's `Part.fuse` of
   disjoint bodies returns a compound holding N solids; the `solid_count` check
   then honestly fails (expected 1, measured N). The fix does not mask a fuse
   that did not join anything, and it does not fuse geometry on the provider
   side.
2. **A compound is expected to keep N solids.** Overlapping occurrences are
   still N compound solids; `construction-semantics.md` says an overlap is not
   a boolean union, and the check counts occurrences rather than guessing a
   union volume.
3. **Mixed join-and-place scripts are outside the supported semantics.** The
   derivation describes the script's declared result: each fuse group is one
   solid, each compound member is one solid, and an additional placed component
   is one occurrence. The FreeCAD adapter exports the last joined tip, so a
   script that mixes a join with extra placements (or uses more than one join
   step) would not match. Those shapes are not in the parity corpus and are not
   claimed as supported here.
4. **A multi-component script with no assembly derives 1**, matching the
   adapter's exported tip feature. Such a script is not a supported assembly
   (the docs require a placement for every part in a supported assembly), so
   this is a conservative fallback rather than a claimed occurrence count.
5. **The count is never taken from the provider.** The provider result is the
   measured side only; the expected side comes from the script. A provider
   cannot make an assembly verify by reporting a different number.
6. **`measure.py` gained only a constant.** `build_checks`/`measure_and_check`
   for the OCP-only path are untouched, so A1's behavior and tests are
   unchanged. `SOLID_COUNT_TOLERANCE` is `0.0`, like the single-solid count.
7. **`build_provider_checks` is backward compatible.** The new parameter
   defaults to 1; B2's direct callers and tests that omit it still get
   `single_solid`.
8. **No schema or journal event type changed.** `design.checked` still carries
   the generic check rows; `provider.build.completed`/`design.measured` already
   carry the provider `solid_count`. Consumers need only accept the new id.
9. **The cold reload path is covered.** Both service call sites derive the
   expected count; the FreeCAD assembly test reopens the data directory and
   loads the stored design, so a cache/journal reload cannot silently revert to
   `single_solid`.
