# Native STEP cell — report (round 1)

Cell: `.mece/waves/rust-backend-20261019/cells/step`
Owner path: `backend/crates/kala-step/**` (new crate, own `[workspace]`, own
`Cargo.lock` and `target/`). No other path was modified: the root
`backend/Cargo.toml`/`Cargo.lock`/`README.md`, `kala-geometry`, `kala-budget`,
`kala-studio`, `kala-json`, `kala-persistence`, Python sources/tests/corpus,
frontend, adapters and packaging are untouched (`backend/Cargo.toml` and
`backend/Cargo.lock` mtimes predate this cell; only files under
`backend/crates/kala-step/` were created).

## 1. What was delivered

A standalone native Rust crate that emits **real ISO 10303-21 (AP214) exact
analytic B-rep** files for the three studio primitives, with millimetre units
and an origin-centred body, replacing the Python OCP path
(`kala_agent/studio/geometry/step.py`) without any Python, OCP or other sidecar
in the product path.

| primitive | emitted solid |
| --- | --- |
| box | 6 planar faces (outward plane frames), 12 shared line edges, 8 vertices, `+/- size/2` |
| cylinder | 1 cylindrical lateral face + 2 planar caps, 2 full-circle edges, 1 seam line used twice, axis `+Z`, `z` in `[-height/2, +height/2]` |
| sphere | 1 closed spherical face with the degenerate `VERTEX_LOOP` closure OpenCascade itself uses, centred at the origin |

Entity set is analytic only: `CARTESIAN_POINT`, `DIRECTION`, `VECTOR`, `LINE`,
`CIRCLE`, `PLANE`, `CYLINDRICAL_SURFACE`, `SPHERICAL_SURFACE`,
`AXIS2_PLACEMENT_3D`, `VERTEX_POINT`, `EDGE_CURVE`, `ORIENTED_EDGE`,
`EDGE_LOOP`, `FACE_BOUND`, `VERTEX_LOOP`, `ADVANCED_FACE`, `CLOSED_SHELL`,
`MANIFOLD_SOLID_BREP`, `ADVANCED_BREP_SHAPE_REPRESENTATION`, product/context
and unit entities. There is no `POLY_LOOP`, `FACETED_BREP`,
`TRIANGULATED_FACE_SET`, B-spline/Bezier or any other approximation entity; a
native test asserts this explicitly.

Files (1,709 lines of Rust source, 1,496 lines of Rust tests, 288 lines of
test-only Python oracles, 127 lines of README):

- `src/lib.rs` — public API: `export_step_primitive`, `export_step`,
  `export_parameters`, `export_kind_str`, `StepExporter` +
  `NativeStepExporter`, `probe_capability`, `self_check`,
  `StepError::{Invalid, Export}`, `STEP_OUTPUT_MODE`, `STEP_EXPORTER_NAME`,
  `MAX_STEP_BYTES`, ISO constants. `#![forbid(unsafe_code)]`.
- `src/primitives.rs` — box/cylinder/sphere boundary models (const edge/face
  tables, axis placements carrying the outward normal).
- `src/model.rs` — the boundary model and its audit before serialisation
  (closed loops, every edge used once per direction, every vertex used,
  `V - E + F = 2`), plus serialisation into the document writer.
- `src/serialize.rs` — entity interning by exact body text, STEP REAL
  formatting (shortest round-trip, decimal point enforced, `-0.` normalised),
  `FILE_NAME`/schema/unit/context/product assembly.
- `src/error.rs` — typed, sanitized errors; `Invalid` carries the reviewed
  `kala-geometry` error whose `Display` text is the Python `ValueError` text.
- `src/bin/kala-step.rs` — CLI: `capability`, `export --kind ... [dims]
  [--out|--force|--repeat]`, refuses to overwrite without `--force`.
- `tests/ocp_oracle.py` + `tests/ocp_differential.rs` — required independent
  OpenCascade verification.
- `tests/freecad_oracle.py` + `tests/freecad_differential.rs` — second
  application-level reader.
- `tests/native_export.rs`, `tests/cli.rs`, unit tests in `src/**` — native
  structure/behaviour/CLI coverage.

## 2. Native dependency, linking and portability (measured, not assumed)

- **External CAD/solver bindings: none.** The library and CLI link only the
  Rust standard library, `kala-geometry` (path dependency, reviewed) and
  `serde_json` 1.0.154 (with `arbitrary_precision`). No OCCT, FreeCAD, OCP,
  CC, C++ or other native library is linked, loaded or exec'd.
- `readelf -d target/release/kala-step` → `NEEDED: libgcc_s.so.1`,
  `libc.so.6`, `ld-linux-x86-64.so.2`; no `RPATH`/`RUNPATH`; `ldd` resolves
  exactly those three; `nm -D` finds zero `STEPControl*`/`BRepPrimAPI*`
  symbols. Release binary 466,184 bytes.
- Windows packaging implication (build-time reasoning, **not** an execution
  result): there is no vendor kernel to ship, install or license, and no
  `LD_LIBRARY_PATH`/DLL search problem, because the exporter is pure Rust.
  No Windows host was available in this cell; **no Windows build or run is
  claimed**. `kala-geometry`'s libm caveat does not apply: STEP output uses no
  transcendental functions (only halving, subtraction, multiplication, square
  root of exact axis deltas for unit directions), so the emitted coordinates
  are exact rationals of the input dimensions and platform-independent.
- Determinism: identical inputs produce byte-identical output (no clock, no
  filesystem, no kernel state; `FILE_NAME` timestamp is a fixed placeholder).
  Debug and release binaries produce identical bytes for all three defaults
  (verified with `cmp`).
- Robustness: no FFI, no `unsafe`, no temporary files or paths (the document is
  assembled in memory, `workdir` is accepted for contract symmetry and never
  touched — a test asserts the directory is not even created), bounded output
  (`MAX_STEP_BYTES` = 64 KiB; real documents are 1.9–5.3 KiB), sanitized errors
  that never echo submitted values or paths.

## 3. Verification executed (exact commands)

Environment: Linux x86-64, rustc/cargo 1.99.1, 16 CPUs (builds limited to
`-j2` per instruction), 15.7 GiB host with other workers active (load average
1.22 at measurement time).

```
cargo fmt --check                                              # clean
cargo clippy --offline --all-targets --locked -j2 -- -D warnings   # clean
cargo test --offline --locked -j2                              # 35 passed, 0 failed
cargo test --offline --locked --release -j2                    # 35 passed, 0 failed
cargo build --offline --release --locked -j2                   # ok
```

Test inventory (all executed here, none skipped):

- 9 unit tests (REAL formatting incl. bit-exact round-trip and `-0.`,
  interning, audit rejection cases, Euler helper).
- 16 `native_export` tests: ISO header/trailer/schema/mm unit; `FILE_NAME`
  parameter conformance; sequential entity numbering with every reference
  resolved; exact per-kind topology counts; exhaustive entity-type allow-list
  (no tessellation/approximation); default centring literals; non-default and
  extreme half-extents; determinism and boundedness; invalid/bool/string/
  null/array/object/overflowing-JSON dimension refusals with Python messages;
  unknown kind and unknown-parameter behaviour; capability/self-check honesty;
  work-directory untouched; 8-thread × 25-export concurrency; error messages
  never echo input; an unwritable workdir is never touched.
- 7 `cli` tests: real process runs of `capability`, file/stdout export,
  overwrite guard + `--force`, seven invalid-input cases with exit codes and
  no file written, `--repeat` byte-identity, `--help`.
- 1 OCP differential (`tests/ocp_differential.rs` → `tests/ocp_oracle.py`)
  over **28 cases**: defaults (3), minimum `0.1` (3), maximum `2000` (3),
  non-cube, four extreme-ratio cases, non-default values (3) and 12 seeded
  log-uniform random cases (seed `0x5EED20261019`, splitmix64).
  For every case the oracle (repository `.venv`, Python 3.14.8, **OCP
  8.0.1.1.0 / OCCT 8.0**, `IFSelect_RetDone` read + `TransferRoots`) reports:
  `BRepCheck_Analyzer` valid, exactly 1 solid, 1 shell,
  `BRepCheck_Shell::Closed`/`Orientation` = `BRepCheck_NoError`,
  `ShapeAnalysis_Shell` free edges 0, expected analytic surface histogram
  (`plane×6`; `cylinder×1 + plane×2`; `sphere×1`), and volume/area/bounds in
  millimetres. The same oracle re-exports each case with the *current Python
  exporter* (`src/kala_agent/studio/geometry/step.py`) and the test compares
  native vs Python metrics: volume/area within `1e-9` relative (measured
  agreement ~1e-15), bounds within `1e-6` mm (OpenCascade `Bnd_Box` pads by
  the 1e-7 shape tolerance), identical surface histograms and solid counts.
  The oracle fails the test when the interpreter/OCP is missing
  (`KALA_STEP_SKIP_OCP_ORACLE=1` is an explicit, loud opt-out).
- 1 FreeCAD differential: FreeCADCmd 1.1.4 (system OCCT 7.9.3 — a different
  kernel version than the OCP wheel's 8.0) re-imports 6 representative files
  (defaults, non-cube, extreme ratio, maximum) as `isValid`/`isClosed` single
  solids, 1 shell, expected faces/surfaces, volume/area within `1e-9` relative
  and bounds exact within `1e-9` mm (FreeCAD's `BoundBox` is not padded).
- 1 doctest for the documented API.

Failure/repair log (each failure fixed within two focused repairs):

1. `FILE_NAME` parameters 5–6 were emitted as lists; OpenCascade reported
   `Unresolved Reference : Fails Count : 2` on load. Fixed to plain quoted
   strings (per ISO 10303-21), regression-tested structurally in
   `file_name_parameters_are_well_formed`.
2. During design, the cylinder's lateral face had the top circle traversed
   forward instead of reversed, which made the re-imported volume 5/7 of
   `π r² h`. The wire order now mirrors the parametric rectangle and the
   re-imported volume matches the Python reference.
3. The FreeCAD test initially produced no report: `FreeCADCmd` executes a
   script with `__name__` set to the script stem, so an `if __name__ ==
   "__main__"` guard never fires there. Documented and fixed in the test-only
   script.
4. Native structure expectations were corrected against measured documents
   (9 points for a box: 8 corners + representation origin; 6 `ORIENTED_EDGE`
   for a cylinder: seam and circles appear in two faces each; 24 for a box).

Measured failure-mode checks: with `KALA_STEP_ORACLE_PYTHON=/nonexistent/python`
the OCP differential fails loudly naming the missing oracle and leaves no
scratch directory behind; `KALA_STEP_SKIP_OCP_ORACLE=1` prints an explicit
"this is not STEP evidence" notice; the CLI never writes a file on an invalid
argument. No unresolved failure and no blocker remains in this cell.

## 4. Local measurement (no headline, single process)

Release binary, `--repeat 1000`, one run per kind, host load average 1.22 with
other workers active: box 48.3 µs/export, cylinder 17.2 µs/export, sphere
6.2 µs/export. Documents (deterministic, debug and release bytes identical):
box 5,225 B `ea4096565e409565b5ff810dcde37983d8f89f95d8ed126d231750a91047b18e`,
cylinder 2,910 B `27439a3d94a481949b11b3c1ffd63feb67ca7354b288cd17dbe5e16e1ff7fd42`,
sphere 1,925 B `db949f4a2b008f63fd54376b61fb75aad54ea9fa2ece2e3972595606031034e8`
(default dimensions; digests document determinism at this revision only and are
not a STEP contract). These are measurements of this implementation only.
No Python comparison, RSS claim or speedup headline is published here: the
conductor runs the controlled interleaved benchmark after workers quiesce, and
any Python comparison must disclose that the Python path loads the whole OCP
kernel in-process (`cadquery-ocp` 8.0.1.1.0) while this path loads no CAD
kernel.

## 5. Honest gaps and limits

1. **No Windows execution evidence.** Portability is argued from the absence of
   any native dependency, not from a Windows build.
2. **Versioned kernel semantics.** The file structure was designed against
   OpenCascade's own AP214 output for the same builders (read-only inspection)
   and verified by OCP 8.0 and FreeCAD 1.1.4. A reader that rejects a
   degenerate `VERTEX_LOOP` sphere boundary or a seam-doubled cylinder wire
   would need an alternative (e.g. two-hemisphere) encoding; neither reader on
   this host required it. Solid Edge was not available here.
3. **Fixed `FILE_NAME` timestamp** (`1970-01-01T00:00:00`) for determinism.
   Timestamps are metadata; bytes/entity ordering are explicitly not a STEP
   contract.
4. **Not a full parity artifact on its own.** This cell produces STEP bytes and
   a capability seam; wiring `kala-studio`'s `StepExporter` to it, route
   responses, fixtures and the `/health` flag are the Studio adapter's separate
   owned task. The Studio route's `step_unavailable` 503 branch therefore stays
   until that integration lands.
5. **Jumbo integer dimensions** behave as `kala-geometry` defines: values that
   cannot become a finite `f64` are refused with the sanitized "must be a
   finite number" message rather than Python's route-level `OverflowError`
   branch (documented in `kala-geometry`'s report; the Studio adapter keeps
   that legacy status).
6. **No cross-language benchmark** in this report (see §4).

## 6. Handoff notes for the Studio adapter cell

- Map `kala_step::StepError::Invalid(GeometryError)` to the existing 422
  `invalid_request` path (message text is Python-identical) and
  `StepError::Export` to the 500-class path; `NativeStepExporter.capability()`
  can back `/health`'s STEP flag without changing route code shape.
- `export_primitive(kind, parameters, workdir)` matches the Studio
  `StepExporter` signature; `workdir` is ignored on purpose and no temporary
  file is created, so the adapter can pass the studio data directory
  unchanged.
- Keep the workspace ownership split: `kala-step` is an independent workspace
  with its own lock; integration into a parent workspace is the conductor's
  decision.
