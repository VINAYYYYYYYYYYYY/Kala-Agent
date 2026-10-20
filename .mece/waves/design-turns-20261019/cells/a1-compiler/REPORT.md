# A1 compiler — report

Date: 2026-10-19
Spec: ADR-0009 §1–2, criteria 1, 3, and the measurement half of 2.
Status: complete for the owned surface; gaps listed at the end.

## Files

Owned and created:

- `src/kala_agent/studio/design/__init__.py` — package docstring only. It does not re-export the OCP modules, so importing the package does not require the kernel.
- `src/kala_agent/studio/design/compile_ocp.py` — ConstructionScript ⇒ OCP B-Rep compiler.
- `src/kala_agent/studio/design/measure.py` — OCP measurement and declared-vs-measured checks.
- `tests/fixtures/construction_parity.json` — frozen 28-case parity fixture.
- `tests/fixtures/generate_construction_parity.py` — fixture generator; the only file that imports the Solid Edge oracle.
- `tests/unit/studio_design/test_compile_ocp.py` — parity, L-bracket, semantics, refusals, budget (19 tests).
- `tests/unit/studio_design/test_measure.py` — measurement and check rows (10 tests).

Not touched: `src/kala_agent/__init__.py`, `src/kala_agent/api/**`, `adapters/**`, `backend/**`, `Rust+buid123/**`, `rust-implementation.md`, `drawing.py`, `session_journal.py`, or any frontend file.

## Public interfaces for A2 / B1

### `kala_agent.studio.design.compile_ocp`

```python
DEFAULT_COMPILE_TIMEOUT_SECONDS: Final[float] = 5.0
SUPPORTED_COMPONENT_OPS: frozenset[str]  # sketch, extrude, revolve, pocket
SUPPORTED_ASSEMBLY_OPS: frozenset[str]   # place, fuse
UNSUPPORTED_PREFIX: str  # "unsupported by the Studio OCP compiler v1: "

@dataclass(frozen=True, slots=True)
class CompiledComponent:
    name: str
    local_shape: TopoDS_Shape          # body-local, before placement
    placed_shape: TopoDS_Shape         # local_shape with placement applied
    placement_mm: tuple[float, float, float]

@dataclass(frozen=True, slots=True)
class CompilationResult:
    components: tuple[CompiledComponent, ...]
    assembly_shape: TopoDS_Shape       # placed or fused assembly
    fused: bool                        # True when any fuse step ran
    warnings: tuple[str, ...]          # e.g. a pocket that removed no material

    @property
    def shape(self) -> TopoDS_Shape: ...          # alias for assembly_shape
    def component(self, name: str) -> CompiledComponent: ...

def compile_script(
    script: ConstructionScript,
    *,
    analysis: AnalysisScript | None = None,
    edits: Sequence[EditStep] = (),
    fillets: Mapping[str, float] | None = None,
    timeout_seconds: float = 5.0,
) -> CompilationResult: ...
```

`analysis`, `edits`, and `fillets` are refusal channels: any non-empty value raises
`ConstructionError("unsupported by the Studio OCP compiler v1: <op>")`. The compiler writes no
artifact and has no journal dependency; a refusal cannot leave a partial file or event behind.

Implemented semantics:

- `sketch` rectangle/circle on `XY`/`XZ`/`YZ`, with in-plane `x`/`y` offsets; the plane normals are
  `XY +Z`, `XZ -Y`, `YZ +X`.
- `extrude` follows the canonical normal; `reversed` flips it without changing volume.
- `pocket` cuts along the canonical normal, finite or through-all, and must share the plane of the
  most recent additive feature. Through-all cuts past the current solid, never an unbounded space.
- A circle `revolve` is lowered to an extrusion of `length` or the diameter, matching the measured
  FreeCAD quirk. A rectangle revolves about component-local `Y` on `XY`/`YZ`; `XZ` is refused.
- `place` translates one component; every component is placed at most once.
- `fuse` unions the named placed components; components not named in a fuse remain a compound.
- Every returned component and the assembly pass a null/empty, positive-volume, and
  `BRepCheck_Analyzer` validity gate before the result is returned.

### `kala_agent.studio.design.measure`

```python
ENVELOPE_TOLERANCE_MM: Final[float] = 0.01
SINGLE_SOLID_TOLERANCE: Final[float] = 0.0
BREP_VALID_TOLERANCE: Final[float] = 0.0

CheckValue: TypeAlias = bool | float | tuple[float, float]

@dataclass(frozen=True, slots=True)
class Bounds:
    min_mm: tuple[float, float, float]
    max_mm: tuple[float, float, float]
    @property
    def size_mm(self) -> tuple[float, float, float]: ...

@dataclass(frozen=True, slots=True)
class Measurement:
    volume_mm3: float
    surface_area_mm2: float
    centroid_mm: tuple[float, float, float]
    bounds: Bounds
    valid: bool
    solid_count: int

@dataclass(frozen=True, slots=True)
class Check:
    id: str
    label: str
    expected: CheckValue
    measured: CheckValue
    tolerance: float
    unit: str
    passed: bool

@dataclass(frozen=True, slots=True)
class CheckReport:
    checks: tuple[Check, ...]
    verified: bool

class MeasurementError(ValueError): ...

def measure_shape(shape: TopoDS_Shape) -> Measurement: ...
def build_checks(
    measurement: Measurement,
    *,
    expected_envelope: Bounds | None = None,
    expect_single_solid: bool = True,
) -> CheckReport: ...
def measure_and_check(
    shape: TopoDS_Shape,
    *,
    expected_envelope: Bounds | None = None,
    expect_single_solid: bool = True,
) -> tuple[Measurement, CheckReport]: ...
```

`build_checks` always emits `brep_valid`; it emits `single_solid` when
`expect_single_solid=True`; it emits `envelope_x`, `envelope_y`, `envelope_z` only when the caller
declares an envelope. Envelope rows hold `(min, max)` pairs in mm and use a 0.01 mm tolerance.
`verified = all(check.passed)`.

Suggested B1 flow:

```python
result = compile_script(script)
measurement = measure_shape(result.assembly_shape)
report = build_checks(measurement, expected_envelope=template_envelope)
# result.assembly_shape -> STEP export; result.warnings -> surface or reject
```

Suggested A2 flow: use `result.assembly_shape` for drawings/sections, or
`result.components[i].local_shape` when a per-component view is needed.

## Fixture

- Path: `tests/fixtures/construction_parity.json`.
- Generator command (recorded in the JSON):
  `PYTHONPATH=adapters/solidedge/src:src .venv/bin/python tests/fixtures/generate_construction_parity.py`
- The generator inserts `adapters/solidedge-bridge/src` into `sys.path` because the bridge package
  is not installed in this environment; the oracle client itself is imported from
  `adapters/solidedge/src`.
- Records the oracle source path and SHA-256. The core test reads only the JSON (never the adapter),
  fails if the fixture is missing, and checks that the provenance fields are present and
  well-formed; the generator is the only code that touches the oracle.
- 28 cases: 12 rectangle pads (three planes × offset/origin × forward/reversed), 6 circle pads,
  2 circle-revolve quirks, 2 rectangle revolves, 3 pocket/pad reference cases, 3 assemblies.
- `documented` entries encode every measured number in `docs/construction-semantics.md`
  (120 mm³ and the three plane boxes; 4000 mm³; 3987.433629 mm³; 3968.584073 mm³). The generator
  refuses to write the fixture if the oracle disagrees with any of them.
- The parity test reads only the JSON and fails (never skips) if it is missing. It does not import
  the adapter.

## Gate results

Run from the repository root:

- `.venv/bin/python -m pytest tests/unit/studio_design/test_compile_ocp.py tests/unit/studio_design/test_measure.py -q`
  → **29 passed**.
- `.venv/bin/ruff check <owned files>` → **All checks passed**.
- `.venv/bin/ruff format --check <owned files>` → **6 files already formatted**.
- `.venv/bin/mypy src/kala_agent/studio/design/__init__.py src/kala_agent/studio/design/compile_ocp.py src/kala_agent/studio/design/measure.py tests/fixtures/generate_construction_parity.py`
  → **Success: no issues found in 4 source files** (literal strict gate).
- `.venv/bin/mypy --python-version 3.14 <the four files above plus the two test files>`
  → **Success: no issues found in 6 source files**. The `3.14` override is only to work around the
  pre-existing test-stub issue described below.
- `.venv/bin/python -m pytest tests/unit/test_solidedge_boundaries.py tests/unit/test_portable_imports.py -q`
  → **16 passed**.
- `.venv/bin/python -m pytest tests/unit -q` → **1358 passed, 1 warning** in 6.98 s. The warning is
  the pre-existing `StarletteDeprecationWarning` from `fastapi.testclient`. (A transient run caught
  A2's `drawing.py` mid-edit with 13 `test_drawing.py` failures; that had cleared by the final run.)

## Acceptance evidence

- Criterion 1: every oracle-checked corpus case matches volume within `1e-6` relative and bounds
  within `0.001 mm`. The L-bracket-style two-component fuse is also covered by the compiler tests.
- Criterion 2 (measurement half): hand-built L-bracket
  `l_bracket(60, 45, 40, 8, B=10, h=5.5, 4 holes)` measures
  **35 571.416047113315 mm³**, bounds exactly `[-30, 30] × [0, 45] × [0, 40]`, valid, one solid,
  `verified=True`. The closed-form check is also asserted at `1e-9` relative.
- Criterion 3: `helix`, `compound`, `material`/analysis, `move`, `resize`, `fillet`, a rectangle
  revolve on `XZ`, and a cross-plane pocket each raise `ConstructionError` naming the op. Duplicate
  placement and a non-positive compile budget also raise. The compiler never writes an artifact.

## Gaps and notes

1. **Rectangle revolve is oracle-unchecked.** `kala_agent_solidedge.oracle` deliberately returns
   `checked=False` for a rectangle revolve, although `docs/construction-semantics.md` lists it in
   the corpus. The fixture records both cases as unchecked; the test compiles them and proves a
   valid, single, non-empty solid, but there is no numeric oracle parity for those two cases. A
   numeric assertion would require extending the adapter oracle (not owned by A1).
2. **Envelope rows need a declared envelope.** `brep_valid` is always present and `single_solid` is
   present when one part is expected, but `envelope_x|y|z` cannot be emitted for a raw script with
   no declared envelope without inventing a number. Templates must pass `expected_envelope` to get
   the three envelope rows. This is how "templates declare an envelope; they never declare volume"
   is enforced.
3. **Same-plane pockets.** The v1 subset says pockets are same-plane. The compiler requires the
   pocket sketch plane to equal the most recent additive plane. Consequently the L-bracket bore
   cannot be cut in a single XY component: the test/template builds the upright as an `XZ` pad
   (extruded forward to `y ∈ [-T, 0]`, bored through, then placed at `y = +T`) and fuses it with the
   base. This is the ADR-compatible construction and is documented in the test.
4. **No-op cuts warn, they do not fail.** `docs/construction-semantics.md` allows a warning or a
   failure when a cut removes nothing. The compiler returns the measured shape plus a warning like
   `component 'plate': pocket on XY removed no material`; it never claims material was removed.
   B1 should either surface `result.warnings` or reject the design turn.
5. **OCP is a hard import for these modules.** `compile_ocp` and `measure` import OCP at module load;
   the package `__init__` does not. B1 should add a capability probe/503 for a Studio install without
   `cadquery-ocp`, mirroring `geometry/step.py`'s `StepCapability`.
6. **Budget granularity.** The 5 s budget is checked at step boundaries and before returning. A
   single OCP boolean that blocks past the deadline is detected when it returns; there is no thread
   kill. No partial artifact exists because the compiler never writes one.
7. **Contract limits.** `MAX_STEPS`, `MAX_COMPONENTS`, and `MAX_DIMENSION_MM` are enforced by the
   pydantic contract before the compiler sees the script; the compiler does not re-validate them.
8. **Mypy test-stub environment issue (pre-existing).** The literal strict gate on test files fails
   before type-checking our code because `pytest`'s stubs pull `numpy` 2.5, whose PEP 695 `type`
   statements are rejected by mypy configured for Python 3.11. A trivial file containing only
   `import pytest` reproduces it. Source and generator pass the literal gate; the test files pass
   strict checking with `--python-version 3.14`. No project file was changed to mask this.
9. **Journal/artifact behavior belongs to B1/A3.** The compiler structurally cannot store an
   artifact or emit `design.compiled`; proving that at the route level needs B1's route and A3's
   journal.
10. **Unfused multi-part assemblies are compounds.** `CompilationResult` exposes each placed
    component separately, but `measure_shape(assembly_shape)` on a compound reports the kernel's
    summed solid volume. For an overlapping, unfused assembly that sum is not a material claim.
    B1 should report per-component measurements (or require a `fuse`) rather than present the
    compound volume as one part's volume.
