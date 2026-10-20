# B3 freecad-adapter — the three FreeCAD adapter defects found by B2

Cell: `b3-freecad-adapter`. Specs read: ADR-0007, ADR-0011,
`docs/construction-semantics.md` ("Assembly occurrences"), `../b2-providers/REPORT.md`
(gaps 1–3 and corpus parity), `adapters/freecad/**` and its tests.

Status: **done**. No commit, no push, no delegation, no credential reads.
Only the files this cell owns were edited; `studio/**` and `backend/crates/**`
were not touched. Core still imports no adapter: the FreeCAD adapter is reached
only through the generated driver inside a `FreeCADCmd` subprocess.

## What changed

| File | Change |
|---|---|
| `adapters/freecad/src/kala_agent_freecad/contracts.py` | one shared `MAX_TRACE = 160`; `DESIGN_OUTPUT_SCHEMA`'s `tool_trace`/`redesign_trace` `maxItems` now uses it instead of `64` |
| `adapters/freecad/src/kala_agent_freecad/cad/design.py` | imports the shared cap; `_Session.export_target()` exports the placed occurrences of a place-only assembly (compound of the placed bodies) or the joined tip; `_exported_shape`/`_measured_shape` measure the exported shape; `_export_all` takes that target |
| `adapters/freecad/tests/test_design_assembly.py` | new: place-only one/two component export target, and fuse still exports the joined result (fake FreeCAD, no install needed) |
| `adapters/freecad/tests/test_worker_envelopes.py` | new: schema/builder trace cap are one number; `invoke_design` accepts an 85-entry trace |
| `adapters/freecad/tests/test_cad_live.py` | new live test: the ADR-0009 L-bracket via `invoke_design` (85 calls → `35571.416047`) |
| `src/kala_agent/studio/design/providers/freecad.py` | driver calls the official `worker.invoke_design`; removed `_refuse_unexportable_assembly` and the driver self-fuse workaround; docs updated |
| `tests/unit/studio_design/test_cad_providers_freecad.py` | corpus test now builds all 28 cases; explicit single/multi placed-component tests; no refusal path |

## 1. `invoke_design` trace cap (defect 1)

The builder truncated `tool_trace`/`redesign_trace` at `160`
(`cad/design.py`) while `DESIGN_OUTPUT_SCHEMA` allowed `64`, so an 85-call
L-bracket succeeded geometrically and then failed envelope validation as
`invalid_result`.

- `MAX_TRACE: Final[int] = 160` now lives in `kala_agent_freecad/contracts.py`
  next to `DESIGN_TIMEOUT_SECONDS`. `_TRACE["maxItems"]` and
  `design.run`'s two trace slices both use it; there is no second number.
- `DESIGN_OUTPUT_DIGEST` necessarily changed because the shipped schema
  changed (`maxItems 64 → 160`). `DESIGN_TOOL_VERSION` was left at `1.0.0`
  (the task did not ask for a version bump; consumers bind the digest).
- The provider driver now runs `worker.invoke_design` with a real
  `ToolRequestEnvelope` (`DEFAULT` workspace, `DESIGN_TOOL_ID/VERSION`,
  `TOOL_CAPABILITIES`, `DESIGN_TIMEOUT_SECONDS`) and reads the response
  envelope. A refusal is reported with its error code and message, never as a
  success.

Evidence (real FreeCAD 1.1.4, run through `invoke_design` directly):

```
status     succeeded
trace      85 of 85 calls
volume     35571.416047 mm^3
area       11889.101731 mm^2
bbox       [60.0, 45.0, 40.0]
solid      True     valid True
step       21648 bytes; fcstd 40294 bytes
```

## 2 & 3. Export target for placed components (defects 2 and 3)

Root cause was the adapter's export target, not the script: `design.run`
exported `session.feature` (the tip feature). For a single placed component the
tip is the local pad (placement lost); for a place-only multi-component script
the tip is only the last component.

Fix in `design.py`:

- `place` records the body it moved.
- `export_target()`: when the script has placements and **no** `fuse`/`compound`
  step, it asserts every placed body has a shape and builds one
  `Part.makeCompound([...])` (`Part::Feature` `Assembly`) as the export target.
  A body's `Shape` already includes its `Body.Placement`, verified on FreeCAD
  1.1.4. One placed part is a one-occurrence assembly (compound of one), so the
  same rule covers defect 2 without a special case.
- A `fuse` or `compound` step sets `_joined`; its own joined tip remains the
  target. A part with no placement still exports its tip feature, so all
  pre-existing single-part behavior is unchanged.
- `_exported_shape` accepts a `Solid` or a `Compound` with at least one solid;
  `_measured_shape` measures the inner solid of a one-solid compound so a fused
  part still reports `measurement.solid == True`. Multi-solid assemblies stay
  compounds: **no fused geometry is invented**, and a compound's `Volume` is
  FreeCAD's summed part volume.
- The same target is used for STEP, FCStd, and the measurement, so the three
  cannot disagree.

Provider-side workarounds removed:

- deleted `_refuse_unexportable_assembly` (no more provider refusal for
  place-only assemblies and no `op: fuse`); `_op_refusal_message` stays only as
  a general diagnostic that names a failing assembly op.
- deleted the driver's self-fuse block and its comment; the driver no longer
  edits the call list at all.
- FCStd reopen still measures bounds and `Solids` from the saved file. The
  export target is created last, so the reopen finds the occurrence compound
  (or the joined tip) rather than a local pad.

Corpus results (real FreeCAD 1.1.4, `tests/fixtures/construction_parity.json`):

```
built                       28 / 28
volume-checked              26 / 26      max relative delta 0.000e+00
bounds-checked              28 / 28      max absolute delta 3.553e-15 mm
rectangle-revolve cases     2            built, no closed-form volume (documented status kept)

assembly_two_parts_placed_once     2 solids  volume 1000.000000  bounds [0,0,0]..[30,10,5]
assembly_three_parts_placed_once   3 solids  volume  600.000000  bounds [0,0,0]..[25,30,2]
assembly_single_translated_part    1 solid   volume  500.000000  bounds [5,6,7]..[15,16,12]
```

## Acceptance evidence

| Criterion | Evidence | Result |
|---|---|---|
| 28/28 corpus via the real provider | `test_cad_providers_freecad.py::test_every_supported_parity_case_builds_and_matches_the_fixture` | **28 supported, 26 volume-checked, 0 refused**; volumes rel ≤ 1e-6 (measured 0), bounds ≤ 0.001 mm (measured 3.6e-15) |
| single placed part | `...::test_freecad_exports_a_single_placed_component_at_its_placement`, adapter `test_design_assembly.py` | `[5,6,7]..[15,16,12]`, volume 500, STEP bounds and provider FCStd bounds agree |
| multi-component place-only | `...::test_freecad_exports_a_multi_component_place_only_assembly`, adapter `test_design_assembly.py` | 2 solids in STEP, volume 1000, bounds `[0,0,0]..[30,10,5]`, `solid_count == 2` |
| L-bracket through `invoke_design` | adapter `test_cad_live.py::test_design_entry_point_builds_the_long_l_bracket_call_list`; provider `test_l_bracket_builds_in_freecad_and_reports_its_own_volume` | 85/85 trace, `35571.416047` mm³, 1 solid, STEP + FCStd |
| shared trace cap | adapter `test_worker_envelopes.py::test_the_design_schema_accepts_the_builder_trace_cap`, `...::test_invoke_design_accepts_a_trace_longer_than_sixty_four` | schema `maxItems == design.MAX_TRACE == 160`; 85-entry result succeeds |
| provider uses the official entry point | provider driver source + `test_cad_providers_freecad.py` | every corpus build runs through `worker.invoke_design` |
| no adapter import in core | `tests/unit/test_adapter_boundaries.py`, `test_solidedge_boundaries.py`, `test_portable_imports.py` | 18 passed |
| gates | see below | all green |

## Gates (this host, `.venv` CPython 3.14.8, FreeCAD 1.1.4)

| Gate | Command | Result |
|---|---|---|
| Adapter suite (incl. live FreeCAD) | `cd adapters/freecad && pytest tests -q` | **388 passed** |
| Design suite | `pytest tests/unit/studio_design -q` | **211 passed**, 1 pre-existing `StarletteDeprecationWarning` |
| Full unit suite | `pytest tests/unit -q` | **1517 passed**, same warning |
| Security suite | `pytest tests/security -q` | **189 passed** |
| Boundary/portable imports | `pytest tests/unit/test_adapter_boundaries.py tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q` | **18 passed** |
| Ruff check | 7 touched files | **All checks passed** |
| Ruff format | 7 touched files | **7 files already formatted** |
| Mypy source | `contracts.py`, `cad/design.py`, `providers/freecad.py`, repo config | **Success: no issues found in 3 source files** |
| Mypy adapter tests | 3 test files, adapter config, `--python-version 3.14` | **Success: no issues found in 3 source files** |
| Mypy provider test | `MYPYPATH=src mypy --python-version 3.14 tests/unit/studio_design/test_cad_providers_freecad.py` | **Success: no issues found in 1 source file** |

The `--python-version 3.14` on tests is the same pre-existing environment
mismatch recorded by A1/A2/A3/B1/B2: the venv is 3.14 while `pyproject.toml`
pins `python_version = "3.11"`, and numpy's stubs use a 3.12+ `type` statement.
It reproduces on untouched files; no project setting was loosened.

## Honest notes

1. **The contract digest changed.** `DESIGN_OUTPUT_SCHEMA` changed, so
   `DESIGN_OUTPUT_DIGEST` is new. The tool version is unchanged. Anything that
   pins the old digest must refresh it; nothing in `tests/unit` pinned it.
2. **An assembly is not a single solid, by construction.** A place-only
   assembly now returns `solid_count = 2` or `3` (FreeCAD's `Solids` count) and
   `measurement.solid == False` on the multi-solid compound. B2's generic
   `build_provider_checks` (`service.py`, not owned by this cell) therefore
   reports the `single_solid` row as failed and `DesignResult.verified` as
   false for an assembly. That is factually honest (it is an assembly), but a
   later cell that owns `service.py`/UI may want to express "N placed
   occurrences" instead of treating it as a bad B-Rep. `component_count` and
   `fused` already distinguish the cases.
3. **Compound volume sums the solids.** FreeCAD reports a compound's volume as
   the sum of its solids, which is exactly what the corpus expects for its
   disjoint parts. If future scripts place overlapping occurrences, that sum
   would count the overlap twice; `construction-semantics.md` warns that an
   overlap is not a boolean union. The corpus has no overlapping assembly.
4. **The adapter's live L-bracket test imports the Studio template**
   (`kala_agent.studio.design.templates.l_bracket`) in a test only. The adapter
   runtime still imports no Studio code, and core still imports no adapter. The
   test exists because the acceptance names that exact part; the provider's
   own test covers the same path end to end.
5. **Bounds still come from reopening the FCStd.** The task asks for
   FreeCAD-measured bounds; the driver keeps its reopen-and-read-`BoundBox`
   step, now over the occurrence compound. If the reopen fails, bounds and
   `solid_count` stay unreported with a warning — never guessed.
6. **`place` at zero translation is an occurrence.** A one-component script
   with an explicit `place (0,0,0)` exports a one-solid compound rather than
   the bare tip. Geometry, volume, and bounds are identical, and it matches
   the "assembly with one placement" semantics in `construction-semantics.md`.
7. **The trace cap is adapter-side.** The provider still bounds the whole
   response (4 MiB) and the subprocess wall time (120 s); 160 is only the
   schema/trace length, not a raise of the runtime budget.
8. **Files outside the owned list:** none edited. `pyproject.toml` already had
   the `freecad` marker from B2; the new tests reuse it for the live module,
   and the fake-only tests carry no marker.
