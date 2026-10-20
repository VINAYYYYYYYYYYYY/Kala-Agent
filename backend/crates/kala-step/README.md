# kala-step

Native exact analytic B-rep STEP export for Kala Studio primitives (`box`,
`cylinder`, `sphere`). This crate replaces the Python
`kala_agent.studio.geometry.step` path — `BRepPrimAPI_MakeBox` /
`MakeCylinder` / `MakeSphere` plus `STEPControl_Writer` — with a
self-contained Rust writer that emits ISO 10303-21 (AP214) exchange files.

## What it emits

An exact analytic B-rep solid in millimetres, centred on the origin:

| kind     | faces                                                    | extents                                |
| -------- | -------------------------------------------------------- | -------------------------------------- |
| box      | 6 planes, 12 line edges, 8 shared vertices                | `+/- size_x / 2`, `+/- size_y / 2`, `+/- size_z / 2` |
| cylinder | 1 cylindrical face + 2 planar caps, 2 circles, 1 seam line | axis `+Z`, `z` from `-height / 2` to `+height / 2` |
| sphere   | 1 closed spherical face (degenerate vertex loop)          | centre at the origin, radius `radius`  |

There is **no** tessellation, mesh substitute, Python/OCP subprocess or
sidecar, and no C/C++ dependency in this build. The only dependencies are the
Rust standard library, the reviewed `kala-geometry` crate (kinds, dimension
defaults/bounds, Python-identical validation messages, JSON parameter adapter)
and `serde_json`.

## Native dependency and linking footprint

* No OCCT, FreeCAD, OCP, C or C++ library is linked or loaded at runtime by the
  library or the CLI. `readelf -d` on the release binary lists only the Rust
  binary's standard loader entries (`libgcc_s`, `libc`, `ld-linux`); there is no
  `libTKSTEP`, no sidecar process and no `LD_LIBRARY_PATH` requirement.
* Consequence for Windows packaging: the exporter is a pure Rust cdylib/exe
  with no vendor kernel to ship, install or license. This is a build-time
  property (no native dependency at all), not a Windows execution result; no
  Windows host was available and none is claimed.
* The test-only oracle scripts under `tests/` do use the repository `.venv`
  (`cadquery-ocp`) and `FreeCADCmd`. They are never part of the product build
  and never run from library code.

## Public API

- `export_step_primitive(kind, Option<&Value>)` / `export_step`: JSON route
  parameters to STEP bytes.
- `export_parameters(kind, &PrimitiveParameters)`: typed Rust path (CLI).
- `export_kind_str(kind, Option<&Value>)`: lowercase kind string dispatch.
- `StepExporter` trait + `NativeStepExporter`: the typed service seam, with
  `capability()` and `export_primitive(kind, parameters, workdir)`.
- `probe_capability()` / `self_check()`: whether the built-in exporter is
  usable in this build.
- `StepError::{Invalid, Export}`: validation failures carry the sanitized
  Python `ValueError` text; writer failures carry a bounded message.
- Constants: `STEP_OUTPUT_MODE`, `STEP_EXPORTER_NAME`, `MAX_STEP_BYTES`,
  `ISO_PREFIX`, `ISO_TRAILER`, `ISO_TERMINATOR`.

`workdir` is accepted for contract symmetry with the Python entry point and is
never touched: the document is assembled in memory, so there is no temporary
file to create or clean up, and no path can be overwritten. Repeated exports of
the same input are byte-identical (no timestamps, stable entity numbering).

## Definition and verification of the B-rep

The writer does not drive a CAD kernel, so correctness rests on two
independent checks:

1. **Native boundary audit** (`src/model.rs`) before serialisation: every face
   boundary closes, every edge is used exactly twice (once per direction),
   every vertex is used, and `V - E + F = 2`. A failing model returns a typed
   error instead of a document.
2. **Independent kernel re-import** (`tests/ocp_oracle.py`, run by
   `tests/ocp_differential.rs`) with the OCP 8.0.1.1.0 bindings: read status,
   `TransferRoots`, `BRepCheck_Analyzer` validity, `BRepCheck_Shell`
   closedness/orientation, `ShapeAnalysis_Shell` free edges, solid/shell counts,
   per-face analytic surface types, volume, area, and bounds in millimetres —
   for defaults, minimum, maximum, extreme ratios and seeded random
   dimensions, and compared against the current Python exporter's own
   output. A second, application-level reader (`tests/freecad_oracle.py`,
   FreeCAD 1.1.4 over the system OCCT 7.9.3, a different kernel version than
   the OCP wheel's 8.0) re-imports representative files as well, which shows the
   files do not depend on one reader version.

Bounds are asserted against analytic values in millimetres, so a missing or
wrong unit declaration cannot pass. The OpenCascade `Bnd_Box` measurement is
allowed 1e-6 mm because `BRepBndLib` grows the box by the shape tolerance
(1e-7); FreeCAD's `BoundBox` is exact and is asserted to 1e-9.

Tolerances for volume and area are `1e-9` relative against both the analytic
formulas and the Python exporter; measured agreement is ~1e-15 relative.

## Running the checks

`kala-step` is a member of the root `backend/` workspace and uses its single
`Cargo.lock`. From `backend/`:

```sh
cargo fmt --all --check
cargo test -p kala-step --locked -j2            # 34 tests + 1 doctest: native + OCP + FreeCAD
cargo test -p kala-step --locked --release -j2
cargo clippy -p kala-step --all-targets --locked -j2 -- -D warnings
cargo build -p kala-step --release --locked -j2

# CLI
cargo run -p kala-step --release -- export --kind box --size-x 20 --out box.step
cargo run -p kala-step --release -- capability
```

Environment variables for the differentials:

- `KALA_STEP_ORACLE_PYTHON`: interpreter with `OCP` (default: repository
  `.venv/bin/python`). Missing oracle fails the test.
- `KALA_STEP_SKIP_OCP_ORACLE=1`: explicit opt-out, prints a loud notice; the
  native writer is then unverified by a kernel.
- `KALA_STEP_FREECAD` / `KALA_STEP_SKIP_FREECAD=1`: same for the FreeCAD
  second-reader check.

Both differential tests write their scratch files into a self-cleaning
temporary directory, so a failing run leaves nothing behind.

## Limits and honest gaps

- Dimensions are validated by `kala-geometry` to `[0.1, 2000.0]` mm with the
  Python messages; a JSON integer dimension wider than the `f64` range maps to
  the sanitized "must be a finite number" message, whereas the Python route
  raises `OverflowError` (handled at the service layer, not here).
- The `FILE_NAME` timestamp is a fixed placeholder (`1970-01-01T00:00:00`) so
  output is deterministic. STEP timestamps are metadata; the value is not a
  geometry contract.
- No performance claim: the CLI's `--repeat` timing is a local harness. The
  conductor runs controlled interleaved benchmarks after worker quiescence.
- No Windows execution evidence; see the linking note above.
- The sphere uses the same single-face `VERTEX_LOOP` closure OpenCascade writes
  for a complete sphere; readers that reject degenerate vertex loops would need
  a two-hemisphere variant, which neither OCP nor FreeCAD required here.
