# kala-cad

Native exact CAD foundation for Kala Agent: the Rust port of Studio's v1
construction compiler, measurement/check layer, tessellation/STL writer,
STEP export/import, hidden-line drawings and section cuts. The geometry kernel
is the real OpenCascade 7.9 C++ SDK, reached through one narrow, owned C++
adapter (`src/ffi/adapter.cpp`). Rust owns every product rule and every
user-visible message; C++ only calls OCCT and owns RAII for kernel handles.

This crate is a standalone workspace with its own `Cargo.lock`. It never looks
for Python, conda, FreeCAD or OCP at build time or runtime, and it never
substitutes an approximate solid for a missing kernel. Build with
`--no-default-features` for the explicit unavailable mode: the public API stays
present, `design_available()` is false, and every geometry operation returns a
typed unavailable error.

## Public surface

| Area | Entry points |
| --- | --- |
| Compile | `compile_script`, `compile_script_with_options`, `compile_json_str`, `compile_json_bytes`, `CompilationResult`, `CompiledComponent`, `Shape` |
| Measure | `measure_shape`, `measure_and_check`, `topology`, `Measurement`, `Bounds`, `build_checks`, `Check`, `CheckReport` |
| Mesh | `tessellate`, `MeshOptions`, `MeshData`, `stl_bytes` |
| Drawing | `hlr_views`, `hlr_views_with_options`, `Drawing`, `ViewDrawing`, `Polyline`, `ViewName` |
| Section | `section`, `section_with_options`, `default_section_offset`, `Section`, `SectionLoop`, `SectionPlane`, `Bounds2D`, `Point2` |
| STEP write | `step_bytes`, `step_bytes_with_options`, `StepWriteOptions`, `normalise_step_bytes` |
| STEP read / view | `step_shape`, `import_step_view`, `StepImportOptions`, `StepImportOutcome`, `validate_step_import_max_bytes`, `resolve_step_import_max_bytes`, `resolve_step_import_max_bytes_from_env` |
| Capability | `probe_capability`, `design_available` |

`Shape` is deliberately **not** `Send` or `Sync`: OCCT mutation is thread-affine
for one handle. Independent shapes may be compiled/measured/tessellated on
different threads; move only plain measured data across threads. STEP
export/import is serialised behind one private mutex because OCCT keeps
unit/schema settings in a process-global registry.

Every C++ entry point catches `Standard_Failure`, `std::exception` and `...`,
and returns a status plus a bounded exception class name. No C++ exception and
no panic crosses the C ABI. Shapes, meshes and STEP buffers have exactly one
Rust owner and one `*_free`; adapter allocation paths use RAII so an exception
after `new` cannot leak. `tests/native_ffi.rs` exercises repeated failures,
1,000 compile/measure/mesh/drop cycles, independent shape lifetimes, concurrent
independent compiles and serialised STEP exports.

## Building against OpenCascade

Discovery is deterministic and download-free. Each directory is selected
independently, first usable candidate wins:

1. `OCCT_INCLUDE_DIR` / `OCCT_LIB_DIR` (also `KALA_OCCT_INCLUDE_DIR` /
   `KALA_OCCT_LIB_DIR`);
2. `OCCT_ROOT` / `KALA_OCCT_ROOT` (`include/opencascade`, `inc`, `lib`,
   `lib64`, `win64/vc14/lib`, `win64/vc14/bin`, `bin`);
3. `OpenCASCADE_DIR` and other CMake package locations, reading
   `OpenCASCADE_INCLUDE_DIR` / `OpenCASCADE_LIBRARY_DIR` from
   `OpenCASCADEConfig.cmake`;
4. `pkg-config --cflags/--libs opencascade` on non-Windows hosts;
5. `VCPKG_ROOT` / `VCPKG_INSTALLATION_ROOT` with
   `VCPKG_TARGET_TRIPLET` (default `x64-windows` on MSVC,
   `x64-linux` elsewhere);
6. platform defaults (`/usr/include/opencascade`, `/usr/lib`,
   `C:\OpenCASCADE-7.9.0\...`).

The include directory must contain `Standard_Version.hxx`; the library
directory must contain `libTKernel.so` / `libTKernel.dylib` / `TKernel.lib`.
`KALA_OCCT_LIBS` overrides the module list (semicolon-, comma- or
space-separated). The default list is `TKernel TKMath TKG2d TKG3d TKGeomBase
TKGeomAlgo TKTopAlgo TKBRep TKPrim TKBO TKBool TKMesh TKShHealing TKHLR TKXSBase
TKDE TKDESTEP TKXCAF TKLCAF`.

### Linux

The development host has OCCT 7.9 in `/usr/include/opencascade` and `/usr/lib`;
CMake config discovery finds it without extra variables. A non-system SDK also
needs its `lib` directory at runtime: the build adds an rpath for local test
runs. Packaging must stage the OCCT `.so` files (or set the loader path) for the
installed app.

### Windows MSVC + vcpkg

```bat
set VCPKG_ROOT=C:\src\vcpkg
set VCPKG_TARGET_TRIPLET=x64-windows
cargo build --release
```

`x64-windows` uses the dynamic CRT and provides headers under
`%VCPKG_ROOT%\installed\x64-windows\include\opencascade`, import libraries in
`...\lib`, and DLLs in `...\bin`. Put that `bin` directory on `PATH` (or copy
the OCCT DLLs next to the test executable) before running `cargo test`. For a
static-CRT triplet set `KALA_CAD_STATIC_CRT=1` so the adapter is compiled with
`/MT` to match. The build emits a warning naming the discovered OCCT tree and
library directories.

Do not attempt a GNU/MinGW link against an MSVC OCCT distribution: the import
libraries and C++ ABI do not match. Use an OCCT build for the same target
toolchain, or build the portable Rust surface with `--no-default-features`.
`cargo check` for a Windows target does not need the native SDK; linking
`kala-cad` with the `native` feature does.

### Optional hardening flags

- `KALA_CAD_CXXFLAGS`: extra whitespace-separated compiler flags.
- `KALA_CAD_SANITIZE=address`: AddressSanitizer build on a native, non-MSVC
  host (the adapter and the test binary are instrumented).
- `KALA_CAD_ORACLE_PYTHON`: override the `.venv/bin/python` used by the live
  differential tests.

## STEP policy

Export transfers the shape `AsIs` in millimetres through an in-memory stream;
no script-controlled path reaches the filesystem. `normalise_step_bytes`
replaces the first `FILE_NAME` timestamp with `2026-01-01T00:00:00` and zeroes
the per-process `Open CASCADE STEP translator <version> <counter>` counter,
preserving byte length. This is exactly the compatibility corpus'
normalisation; it never changes geometry. Byte parity of raw STEP documents is
not promised because timestamps and entity numbering vary.

View-only import (`import_step_view`) enforces a 1-byte..512-MiB ceiling
(default 64 MiB), requires the ISO-10303-21 header, reads through
`STEPControl_Reader`, tessellates with `BRepMesh_IncrementalMesh` inside the
200k-vertex/400k-triangle budget, retries once at 0.5 mm / 0.8 rad, reports the
exact B-Rep envelope (not mesh vertices), counts solids, and returns the honest
Python warning strings. It never modifies or re-exports the imported shape.

## Drawing and section policy

The drawing API is the native port of
`src/kala_agent/studio/design/drawing.py`. It returns plain data only; no
OpenCascade handle escapes the crate, and every result type is `Send + Sync`.

- `hlr_views(shape)` draws the four ADR views (`front`, `top`, `right`, `iso`)
  with a 0.05 mm chord tolerance and a 20,000-point cap per view.
- `hlr_views_with_options(shape, views, chord_tolerance_mm, max_points)` draws
  the requested views in order. Names are the lower-case Python names; an
  unknown name is refused with Python's exact `ValueError` text.
  `Drawing::view("front")` and `Drawing::view_named(ViewName::Front)` look a
  view up; `ViewDrawing::visible`/`hidden` hold the polylines and `truncated`
  says whether the cap dropped a line.
- `section(shape)` cuts the `XZ` plane at the mid-depth of the shape's bounds.
  `section_with_options(shape, plane, offset_mm, chord_tolerance_mm)` cuts
  `XY`, `XZ` or `YZ` (case insensitive) at an explicit offset or the mid-plane.
  `Section::outer_loops()`/`inner_loops()` expose the material boundaries and
  holes; `area_mm2` is the measured net area and `area_exact` says whether it
  is a kernel measurement. `default_section_offset(shape)` is the ADR-0009
  default.
- Every polyline and loop is in view-plane millimetres. Curves are sampled with
  `GCPnts_QuasiUniformDeflection`; the first and last points are the exact
  curve caps, a closed curve repeats its first point at the end, and a curve
  that collapses to a point yields a one-point polyline.
- Section areas are measured on the exact planar faces of the cut with
  `BRepGProp::SurfaceProperties`, never on the sampled polygon. A loop's area
  is only reported as exact when the face describes the loop; a face the kernel
  could not build, or one that disagrees with the drawn loop, falls back to the
  polygon area with `area_exact = false`. The net area falls back to the signed
  loop sum when a region face cannot be built or disagrees, and is never
  reported as exact after the point cap dropped a loop.

The crate-level API accepts the Python names as `&str`; the validation order
and refusal messages match the Python module (`DrawingErrorKind::Argument` is
Python's `ValueError`, `DrawingErrorKind::Shape` is Python's `DrawingError`).
One deliberate difference: Rust's `max_points` is a `usize`, so a fractional
or negative limit is rejected by the type system instead of by
`int(max_points)`.

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --release
```

The native suites require the real OCCT SDK and the repository `.venv` with
OCP for the live differentials. There is no skip switch: a missing oracle fails
the gate loudly. `tests/ocp_differential.rs` covers compile/measure/mesh/STEP;
`tests/drawing_differential.rs` covers HLR drawings and section cuts against
the real Python drawing module. `tests/freecad_differential.rs` is an optional
second-reader check that only skips when no FreeCAD executable exists.

## Limits

- Drawing and section results are exactly the Python module's ADR subset: four
  orthographic/axonometric views and the three principal section planes. There
  is no arbitrary view direction, section hatching or sheet layout in this
  crate; the Studio sheet layer owns presentation.
- Studio journal/artifact storage, HTTP/auth and the design service are not in
  this crate. STEP view import returns the bytes-derived facts; the caller owns
  the content digest and the journal.
- Raw STEP and tessellated mesh bytes are OCCT-version dependent. Geometry,
  units, topology counts, volume, bounds, drawing point sets and measured areas
  are the parity contract.
