# Contract foundation — implementation report (review repair, round 2)

Review resolved 4 items: cut direction wording, multiple additive features per
part, the artifact cap, and output ops during a pending profile. This round
touched only the contract-owned files below plus this report.

## Changed paths

- `adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py`
- `adapters/solidedge-bridge/tests/test_protocol.py`
- `.mece/waves/solidedge-20261019/cells/contract/REPORT.md` (this file)

Unchanged from round 1 (still contract-owned, verified present):
`adapters/solidedge-bridge/pyproject.toml`,
`src/kala_solidedge_bridge/{__init__,units,errors}.py`,
`tests/test_units.py`.

No core files, other adapter files, or other workers' paths were changed. No
delegation, commit, push, publish, deploy, or package installation was
performed.

## Package and CLI

- Distribution: `kala-solidedge-bridge`; import package: `kala_solidedge_bridge`.
- `requires-python = ">=3.11"`; dependencies `pydantic>=2.10,<3` and
  `pywin32>=306; sys_platform == 'win32'`.
- No dependency on `kala-agent`; `__init__.py` imports only `.protocol`.
- Pytest `pythonpath = ["src"]` only. Ruff and strict mypy are configured
  (`mypy_path = ["src"]`, pydantic plugin, `strict = true`).
- Entry point `kala-se-bridge = "kala_solidedge_bridge.__main__:main"` is
  declared, but `__main__.py` is intentionally not created here: the bridge
  worker owns the worker entry point. Until that file exists the console script
  is not runnable.

## Public contract for dependent workers

`kala_solidedge_bridge.protocol` implements frozen, `extra="forbid"`,
`validate_default=True` Pydantic v2 models. All real-valued fields reject
booleans, numeric strings, `NaN`, and infinities; plain JSON integers are
accepted for real numbers. `protocol_version` is the integer `1` only
(`true` and `1.0` are rejected).

Constants: `PROTOCOL_VERSION=1`, `MAX_PARTS=8`, `MAX_OPERATIONS=600`,
`MIN_SIZE_MM=0.001`, `MAX_SIZE_MM=5000.0`, `MAX_OFFSET_MM=5000.0`,
`MAX_ANGLE_DEG=360.0`, `MIN_TIMEOUT_SECONDS=1.0`,
`MAX_TIMEOUT_SECONDS=900.0`, `MAX_DENSITY_KG_M3=22000.0`,
`MAX_TRACE_STEPS=600`, `MAX_ARTIFACTS=2*MAX_PARTS+2=18`, `MAX_WARNINGS=16`.
`MAX_ARTIFACTS` is derived (not a round guess) so that all 8 parts (`.par` and
`.step` each) plus the assembly (`.asm` and `.step`) always fit.

Enums: `Plane` (`XY`, `XZ`, `YZ`), `JobState` (`queued`, `running`,
`succeeded`, `failed`, `cancelled`, `indeterminate`, `interrupted`),
`ArtifactFormat` (`par`, `asm`, `step`), `ErrorCode` (stable closed set
including `invalid_request`, `invalid_sequence`, `unknown_operation`,
`unsupported_operation`, `unavailable`, `calibration_required`, `busy`,
`conflict`, `timeout_indeterminate`, `cancelled`, `internal_error`).

Operation models (discriminated on `op`):

- `PartNew(name)`
- `SketchRectangle(plane, width, height, x=0, y=0)`
- `SketchCircle(plane, diameter, x=0, y=0)`
- `SolidExtrude(length, reversed=False)`
- `SolidCut(length=None)` — `None` is through-all; no `reversed` field.
- `SolidRevolve(angle=360, axis="Y")` — `angle` is `(0, 360]`.
- `PartMeasure()`
- `PartSave()`
- `PartExportStep()`
- `AssemblyNew(name)`
- `AssemblyPlace(component, x=0, y=0, z=0)`
- `AssemblySave()`
- `AssemblyExportStep()`

`BridgeOp` is the annotated discriminated union of those models.

`BuildRequest` fields: `protocol_version=1`, `request_id` safe `<=64`,
optional `calibration_id` safe `<=128`, `summary` `<=500`, `procedure`
1..600 operations, `timeout_seconds=120` in `[1, 900]`, optional
`density_kg_m3` in `(0, 22000]`. It exposes:
- `canonical_request_bytes() -> bytes`: UTF-8, sorted keys, compact
  separators, ASCII escapes, complete payload except `request_id`.
- `request_digest() -> str`: SHA-256 hex digest of those bytes.

`request_id` is excluded because it is transport correlation only; retrying
the same semantic request must produce the same digest. Everything else,
including calibration, summary, ordered procedure, timeout, and density, is
covered. `calibration_id` stays optional so a request can be compiled offline;
a server must still require it before native admission.

Response/result models: `StepTrace`, `RangeBox`, `PartResult`, `Artifact`,
`ErrorResponse`, `BuildResult`, `BuildStatus`, `Health`, `BuildAccepted`.
`Artifact` carries `name`, `format`, `media_type`, `size_bytes`, `sha256` only:
no disk path is accepted or returned. `Health.capabilities` are bounded names;
they are not a readiness claim. Native support must be reported through
`supported`/`available`, calibration state, and `detail`.

### Direction semantics (item 1)

Planes use the measured normals `XY +Z`, `XZ -Y`, `YZ +X`.

- Extrusion runs along the plane normal unless `reversed` is true.
- A cut removes material **along the canonical plane normal itself**, not the
  opposite of it, and is independent of whether the preceding pad used
  `reversed`. `length=None` means through-all in that one direction. There is
  no `reversed` field: the canonical cut direction is fixed by the contract.
- `SolidRevolve` turns around the **component-local** `Y` axis; assembly
  placement is separate and never part of the operation.

This wording now matches the measured FreeCAD observation (`Pocket.Reversed=
True` on an `XY +Z` pad removes `+Z` material) without over-claiming: realizing
these canonical directions on a real host still requires native calibration,
and the contract invents no measured physics.

### Procedure invariants (items 2 and 4)

Enforced by `BuildRequest`:

- 1..600 operations, independent of the core IR's 12-steps-per-component cap.
- At most 8 parts; part names are unique bounded identifiers.
- The current part is implicit; no operation can name a document or select an
  arbitrary part.
- A sketch sets one pending profile; another sketch/new part is refused until
  it is consumed.
- `solid.extrude`/`solid.revolve` consume the pending profile and require a
  current part. Several additive features per part are allowed: the first
  creates the part's solid, later ones extend the solid that already exists,
  and that existing solid state is preserved rather than reset or rejected.
  How a native executor chains those additive calls is its own concern, not a
  reason to shrink the wire capability.
- A `solid.cut` consumes the pending profile and requires an existing solid.
- `part.measure`/`part.save`/`part.export_step` require a current solid and
  are refused while a profile is pending, so `sketch -> measure -> cut` can no
  longer produce a stale or intermediate export.
- Assembly creation requires at least one finished part. After
  `assembly.new`, only assembly operations are allowed. Placement may reference
  only a finished part, and each component may be placed once. Assembly save
  and STEP export require at least one placement.
- A procedure must end with no pending profile and a solid in the current part.
- Artifact paths are impossible: every op model forbids extra fields.

## Units and errors

`kala_solidedge_bridge.units`: `mm_to_m`, `m_to_mm`, `deg_to_rad`,
`m3_to_mm3`, `m2_to_mm2`. Each rejects booleans, numeric strings, `NaN`, and
infinities with `ValueError`.

`kala_solidedge_bridge.errors`: `BridgeError(code, message)` plus typed
`InputError`, `UnavailableError`, `CalibrationRequiredError`, `BusyError`, and
`ConflictError`. Each maps to an `ErrorCode` and can produce a bounded
`ErrorResponse` via `to_response()`.

## Checks and outcomes

Run from `adapters/solidedge-bridge` with `/home/v9i/Projects/Kala-Agent/.venv`:

- `python -m pytest -q` — **87 passed** (82 before, +5 new: additive
  features, its two output ops, artifact cap, and the parameterized pending-
  profile rejection).
- `coverage run -m pytest -q && coverage report` — **96.20%** total
  (`protocol.py` 95.67%), above the configured `fail_under=80`.
- `ruff check .` — **All checks passed**.
- `ruff format --check .` — **6 files already formatted**.
- `mypy` — **Success: no issues found in 6 source files** (strict, Python 3.11
  target).

New/updated tests for this round:

- `test_additive_features_extend_a_part_that_already_has_a_solid` — one base
  extrude, then a second extrude and a revolve, then an optional cut.
- `test_output_ops_are_rejected_while_a_profile_is_pending` — parameterized
  over measure/save/export-step with the profile later consumed by a cut, so
  rejection proves the interleaving itself fails, not just the end-pending
  check.
- `test_maximum_eight_part_artifacts_fit_the_cap` — 8 parts with 16 part
  artifacts plus assembly `.asm`/`.step`, accepted at 18 and rejected at 19.
- `test_operation_cap_is_independent_of_the_core_ir` still exercises 600 cut
  pairs after an existing solid.

## Limitations and blockers

- No Windows host, Solid Edge installation, COM attach, type-library probe, or
  live feature build was available. Nothing here claims native Windows
  evidence; `pywin32` is declared only as a Windows marker dependency and is
  never imported. Cut direction and additive chaining are canonical contract
  statements pending native calibration, not measured runtime claims.
- `kala_solidedge_bridge.__main__` does not exist yet, so the declared
  `kala-se-bridge` script will fail until the bridge worker adds it.
- The mypy config skips the internals of `pytest`/`numpy` because the installed
  numpy 2.5.4 stubs contain Python 3.12 `type` statements mypy cannot parse
  under the package's 3.11 target. Test bodies are still checked;
  `disallow_untyped_decorators` is disabled only for the two test modules.
  Source is fully strict-checked.
- Checks ran on the repository virtualenv's Python 3.14 interpreter while mypy
  targets 3.11. No wheel build or installation was attempted.
- Worker session: task/child-thread IDs are recorded in `SESSION.md`; the
  OpenCode session ID was not reported by the transport.
