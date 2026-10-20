# cad-r3 REPORT: HLR drawings and exact-area sections in kala-cad

Cell: `cutover-20261020/cad-r3` (Rust worker, DeepSeek V4.1 Flash)
Scope: `backend/crates/kala-cad/**` only, plus this cell report. No commit, push,
sudo, delegation or credential reads. Every shell that ran a binary or Python
entry point exported a temporary `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; the user data tree was never read
or written.

Status: **complete**. The accepted cad-r2 shim is extended, not forked. HLR
views and sections are at parity with the Python `drawing.py`: the L-bracket XZ
section at y = 4 measures `2400 - 25*pi` at relative 1e-9, the loop and
outer/hole classifications match, the HLR polyline structures and point sets
match (observed movement 0 mm; the test bound is the chord tolerance), and all
15 refusal cases have the identical Python class and message.

## Starting point

cad-r2 delivered the standalone `kala-cad` workspace with the compiler,
measurement, tessellation, STEP I/O and the narrow C++ adapter over OCCT 7.9.3.
This cell kept that structure and added:

- `src/drawing.rs`: kernel-independent drawing/section policy, the four view
  bases, the three section bases, the result types, validation messages and a
  Python-compatible float/string `repr` for refusals;
- `src/native/drawing.rs`: the native HLR/section implementation, duplicate
  removal, point caps, loop sorting/nesting, area-consistency policy and RAII
  handles;
- `src/ffi/adapter.cpp`: HLR (`HLRBRep_Algo` + `HLRBRep_HLRToShape`), edge
  sampling (`GCPnts_QuasiUniformDeflection`), section cuts
  (`BRepAlgoAPI_Section`), wire connection (`ShapeAnalysis_FreeBounds`), exact
  face areas (`BRepGProp`), and the opaque drawing/section result objects;
- the same public surface in `src/unavailable.rs` (explicit unavailable mode);
- `tests/native_drawing.rs` (38 tests ported from the Python test module),
  `tests/drawing_differential.rs` + `tests/oracle/drawing_oracle.py` (live
  differential against the real Python module), and drawing refusals in
  `tests/unavailable.rs`;
- `README.md` drawing/section contract and limits.

No dependency, feature or public-constant change was made; only the package
description gained the drawing words.

## Public API

Constants (always available, both feature modes):

```rust
DEFAULT_CHORD_TOLERANCE_MM: f64 = 0.05;
DEFAULT_SECTION_CHORD_TOLERANCE_MM: f64 = 0.01;
MAX_POINTS_PER_VIEW: usize = 20_000;
MAX_SECTION_POINTS: usize = 20_000;
DEFAULT_VIEW_NAMES: [&str; 4] = ["front", "top", "right", "iso"];
```

Native entry points (`feature = "native"`, the default):

```rust
pub fn hlr_views(shape: &Shape) -> Result<Drawing, DrawingError>;

pub fn hlr_views_with_options(
    shape: &Shape,
    views: &[&str],              // "front" | "top" | "right" | "iso"
    chord_tolerance_mm: f64,
    max_points: usize,
) -> Result<Drawing, DrawingError>;

pub fn section(shape: &Shape) -> Result<Section, DrawingError>;

pub fn section_with_options(
    shape: &Shape,
    plane: &str,                 // "XY" | "XZ" | "YZ", case insensitive
    offset_mm: Option<f64>,      // None = mid-plane of the Y bounds
    chord_tolerance_mm: f64,
) -> Result<Section, DrawingError>;

pub fn default_section_offset(shape: &Shape) -> Result<f64, DrawingError>;
```

Plain result data (all `Send + Sync`, no kernel handles):

```rust
pub struct Drawing {
    pub views: Vec<ViewDrawing>,
    pub chord_tolerance_mm: f64,
    pub max_points: usize,
}
impl Drawing {
    pub fn view(&self, name: &str) -> Option<&ViewDrawing>;
    pub fn view_named(&self, name: ViewName) -> Option<&ViewDrawing>;
    pub fn view_names(&self) -> Vec<ViewName>;
    pub fn total_point_count(&self) -> usize;
}

pub struct ViewDrawing {
    pub name: ViewName,               // as_str() -> &'static str
    pub visible: Vec<Polyline>,
    pub hidden: Vec<Polyline>,
    pub truncated: bool,
}
impl ViewDrawing {
    pub fn point_count(&self) -> usize;
    pub fn polyline_count(&self) -> usize;
}

pub struct Polyline { pub points: Vec<Point2>, pub closed: bool }
impl Polyline {
    pub fn point_count(&self) -> usize;
    pub fn bounds(&self) -> Option<Bounds2D>;
    pub fn length_mm(&self) -> f64;
}

pub struct Point2 { pub u: f64, pub v: f64 }   // new, distance, to_array

pub struct Bounds2D { pub min_u: f64, pub min_v: f64, pub max_u: f64, pub max_v: f64 }
impl Bounds2D { width, height, center_u, center_v }

pub struct Section {
    pub plane: SectionPlane,          // as_str() -> "XY" | "XZ" | "YZ"
    pub offset_mm: f64,
    pub loops: Vec<SectionLoop>,
    pub area_mm2: f64,
    pub chord_tolerance_mm: f64,
    pub truncated: bool,
    pub area_exact: bool,
}
impl Section {
    pub fn outer_loops(&self) -> impl Iterator<Item = &SectionLoop> + '_;
    pub fn inner_loops(&self) -> impl Iterator<Item = &SectionLoop> + '_;
    pub fn outer_loop_count(&self) -> usize;
    pub fn inner_loop_count(&self) -> usize;
    pub fn point_count(&self) -> usize;
}

pub struct SectionLoop {
    pub points: Vec<Point2>,
    pub area_mm2: f64,       // always positive
    pub outer: bool,
    pub area_exact: bool,
}
impl SectionLoop { bounds() -> Option<Bounds2D>; centroid() -> Option<Point2>; point_count() }

pub enum ViewName { Front, Top, Right, Iso }       // parse(), as_str()
pub enum SectionPlane { Xy, Xz, Yz }               // parse(), as_str()

pub struct DrawingError { /* private */ }
impl DrawingError { pub fn kind(&self) -> DrawingErrorKind; pub fn message(&self) -> &str; }
pub enum DrawingErrorKind { Argument, Shape, Kernel, Unavailable }
```

Semantics worth carrying into the service:

- Views are in view-plane millimetres: `front` looks from `-Y` (`+X` right,
  `+Z` up), `top` from `+Z` (`+X` right, `+Y` up), `right` from `+X` (`+Y`
  right, `+Z` up), `iso` from `(+X, -Y, +Z)` with `+Z` up.
- `XZ` is the plane normal to `Y`; `offset_mm` is measured along the normal
  from the global origin. `offset_mm = None` always uses the mid-plane of the
  shape's Y bounds (ADR-0009), regardless of the plane name.
- `Polyline::points` includes the closing point when `closed`; the point caps
  are the exact curve endpoints. A collapsed curve is a one-point polyline.
- `SectionLoop::area_mm2` is measured on the exact planar face when
  `area_exact`; otherwise it is the drawn-polygon area and explicitly an
  approximation. `Section::area_mm2` is the exact net face area when
  `area_exact`; it is false after a point-cap truncation.
- `Section::loops` is ordered by measured area (largest first), exactly like
  the Python module; `outer` is true for material boundaries (including an
  island inside a hole).

## Native-design integration (consumer notes)

`Shape` is still deliberately `!Send + !Sync`. **Call the drawing API on the
same thread that owns the shape**, then send the plain `Drawing`/`Section` to
the HTTP/JSON layer. The result types are `Send + Sync` and own all their data.

Map to the existing Python response schemas (field names unchanged):

| Rust | JSON field |
| --- | --- |
| `view.name.as_str()` | `DrawingView.name` |
| `polyline.points` | `DrawingPolyline.points` as `[[u, v], ...]` |
| `polyline.closed` | `DrawingPolyline.closed` |
| `polyline.point_count()` | `DrawingPolyline.point_count` |
| `view.point_count()` | `DrawingView.point_count` |
| `view.polyline_count()` | `DrawingView.polyline_count` |
| `view.truncated` | `DrawingView.truncated` |
| `drawing.chord_tolerance_mm` | `DrawingResponse.chord_tolerance_mm` |
| `drawing.max_points` | `DrawingResponse.max_points` |
| `drawing.total_point_count()` | `DrawingResponse.point_count` |
| `section.plane.as_str()` | `SectionResponse.plane` |
| `section.offset_mm` | `SectionResponse.offset_mm` |
| `loop.points` | `SectionLoopResponse.points` |
| `loop.area_mm2`, `loop.area_exact`, `loop.outer` | same-named loop fields |
| `loop.bounds()` (min/max/width/height/center) | `SectionBounds2D` |
| `loop.centroid()` as `[u, v]` | `SectionLoopResponse.centroid` |
| `loop.point_count()` | `SectionLoopResponse.point_count` |
| `section.area_mm2`, `.area_exact`, `.truncated` | same-named section fields |
| `section.outer_loop_count()`, `.inner_loop_count()`, `.point_count()` | same-named counts |

`design_id` is added by the design service, not by this crate. Suggested error
mapping: `DrawingErrorKind::Argument` is Python's `ValueError` (HTTP 400),
`Shape` is Python's `DrawingError` (HTTP 422 or the existing 400 mapping),
`Kernel` is a 500-class kernel failure, `Unavailable` is the explicit
no-kernel build. The message text is already the Python-visible text.

Example (sketch for the service):

```rust
use kala_cad::{CompileOptions, compile_script, hlr_views, section_with_options};

let compiled = compile_script(&script)?;
let drawing = hlr_views(compiled.shape())?;
let cut = section_with_options(compiled.shape(), "XZ", Some(4.0), 0.01)?;
// Serialise drawing.views / cut.loops here; no kernel work after this point.
```

## Changed paths

All under `backend/crates/kala-cad/`:

| Path | Change |
| --- | --- |
| `src/drawing.rs` | **new** policy, bases, result types, validation, Python repr, unit tests |
| `src/native/drawing.rs` | **new** HLR/section implementation and RAII kernel handles |
| `src/native/ffi.rs` | `FfiSectionLoopInfo` and drawing/section FFI declarations |
| `src/ffi/adapter.cpp` | HLR, sampling, section, exact areas, opaque result objects and `extern "C"` wrappers |
| `src/error.rs` | `DrawingError`, `DrawingErrorKind` |
| `src/native/mod.rs` | native re-exports |
| `src/lib.rs` | module, public re-exports, both feature modes |
| `src/unavailable.rs` | same drawing/section surface, typed unavailable errors |
| `tests/native_drawing.rs` | **new** 38 tests ported from `test_drawing.py` plus handle reuse |
| `tests/drawing_differential.rs` | **new** live differential against the Python module |
| `tests/oracle/drawing_oracle.py` | **new** test-only oracle (Python drawing + Python-test shape builders) |
| `tests/unavailable.rs` | drawing refusals in the unavailable build |
| `Cargo.toml` | package description mentions drawings/sections (no dependency or feature change) |
| `README.md` | drawing/section policy, API, consumer notes, limits |

No product Python source, shared fixture, adapter, Studio file, root
manifest/lock or other crate was edited by this cell.

## Gates (exact results)

Environment: Linux x86_64, rustc 1.99.1 (edition 2024), OCCT 7.9.3, OCP
8.0.1.1, Python 3.14.8, `.venv/bin/python`.

| Gate | Result |
| --- | --- |
| `cargo fmt -- --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test --locked` (debug) | **118 tests, 0 failures** |
| `cargo test --locked --release` | **118 tests, 0 failures** |
| `cargo build --locked --release` | pass |
| `cargo test --locked --no-default-features` | **10 tests, 0 failures** (7 lib + 3 unavailable) |
| `cargo check --target x86_64-pc-windows-gnu --no-default-features` | pass |
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | pass |
| `cargo check --target x86_64-pc-windows-gnu` (native) | fails at the build script: no Windows OCCT SDK; message names `OCCT_INCLUDE_DIR`/`OCCT_ROOT`/`VCPKG_ROOT` |
| `cargo check --target x86_64-pc-windows-msvc` (native) | same expected missing-SDK failure |

Per-target debug counts: lib 8, `drawing_differential` 1, `freecad_differential` 1
(optional second reader, ran), `native_compile` 23, `native_drawing` 38,
`native_ffi` 11, `native_measure` 11, `native_mesh` 10, `native_step` 12,
`ocp_differential` 1, `unavailable` 0 (native), doctests 2. Release counts are
identical. The unavailable build has 7 lib tests and 3 explicit-unavailable
tests; the drawing API is among the unavailable entry points.

C++ is compiled with `-Wall -Wextra`; the new adapter code produced no
warnings.

## Live differential (mandatory oracle)

`cargo test --locked --test drawing_differential -- --nocapture` runs the real
`src/kala_agent/studio/design/drawing.py` under the repository `.venv`.

```
checked:
  shapes: 13          (10 native STEP documents + 3 tolerance/cap variants)
  views: 55
  sections: 55
  oracle_shapes: 3    (Python-built test shapes, STEP round-tripped)
  refusals: 15
mismatches: 0
maximum sampled point movement: 0 mm
```

Both implementations draw the same re-imported STEP document in every case, so
a mismatch would be an implementation mismatch. The corpus:

- native compiler shapes: ADR L-bracket, box with through hole, box with bore
  and island, cylinder, multi-additive part, translated part, four frozen
  fixture cases;
- extra L-bracket views at 0.5 mm and 0.001 mm chord tolerance, and a
  `max_points = 5` truncation case;
- Python-built copies of the three shapes in `test_drawing.py`
  (`build_l_bracket`, `build_box_with_through_hole`,
  `build_box_with_an_island_in_a_bore`), each written to STEP and re-imported
  before either side draws it;
- plane-on-face (`XZ` at 0), mid-plane, explicit offsets on `XZ`/`XY`/`YZ`, and
  a lowercase `"xz"` name.

Compared fields:

- views: names and order, visible/hidden polyline counts, every per-line point
  count and closed flag, view point count, truncated flag, visible/hidden point
  sets with a symmetric Hausdorff bound (the drawing chord tolerance) and a
  per-point bound (the same tolerance). Observed movement was 0 mm.
- sections: plane, offset, chord tolerance, truncated flag, loop count/order,
  per-loop outer and area_exact flags, point counts, point sets (chord
  tolerance), loop bounds (1e-6 mm), per-loop areas and net area at relative
  1e-9, and `area_exact`.
- `default_section_offset` within 1e-9 mm.
- refusals: exact Python exception class and message (15 cases).

Oracle source digests from the run:

```
drawing.py     e29663328d56211030aa4f290cf687248587226da8cd3a6a4346a2cd4e0ac5d3
test_drawing.py 3daeb5be12fd5a624a3e84a35cd22da47595762f947d56dcb2f1cd51e94ef70b
```

Versions: OCP 8.0.1.1, Python 3.14.8, OCCT 7.9.3 (Rust), rustc 1.99.1.

## Parity evidence

- **L-bracket XZ at y = 4** (the ADR criterion): one outer 60 x 40 loop and one
  inner diameter-10 loop centered at `(0, 24)`; outer area 2400 and inner area
  `25*pi` measured on the exact faces; net `2400 - 25*pi =
  2321.4601836602552` with a relative error around 1e-16. `area_exact` is true
  for both loops and the section.
- **Exact-area repair**: a round loop's drawn polygon is compared with the
  measured area; the measured area is the circle's, not the polygon's. A
  tangent cut (`z = 5` through the box bore) reports the drawn 1200 mm² with
  `area_exact = false` for both the loop and the section. An island inside a
  bore reports 2 outer + 1 inner loops and net
  `800 - 100*pi + 16*pi` at relative 1e-9.
- **Loop topology**: all 55 sections have identical loop counts, order, outer
  flags and `area_exact` flags to Python. The oracle's exact Python-test
  island shape gives 2 outer + 1 inner and net `536.106217098` mm².
- **HLR structure**: every one of the 55 views has identical visible/hidden
  polyline counts, per-line point counts, closed flags and truncation; the
  point sets are identical to 0 mm on this host. The `max_points = 5` case
  reproduces Python's visible-first cap order.
- **Refusals**: `unknown view 'side'; expected one of ['front', 'iso',
  'right', 'top']`, `unknown section plane 'AB'; expected one of ['XY', 'XZ',
  'YZ']`, `the chord tolerance must be a positive finite number: 0.0` (also
  `-1.0`, `nan`, `inf`), `the point limit must be positive: 0`, `the section
  offset must be a finite number: nan` (also `inf`, `-inf`) and
  `the drawing needs a non-null shape` all match Python's class and text.

## FFI and memory safety

- Every new adapter entry catches `Standard_Failure`, `std::exception` and
  `...`, and writes a bounded status/exception class; no exception crosses the
  C ABI.
- `KalaDrawing` and `KalaSection` are heap-owned, copied into Rust plain data
  and freed exactly once by their `Drop` guards. Point and line accessors
  validate the handle, index and destination capacity before writing;
  `memcpy` uses the adapter-owned size.
- Section wires are kept only inside the opaque section object so the net face
  can be built after Rust classification; no `TopoDS` handle escapes.
- `wire_is_simple` rejects null and non-finite vertices before sorting, so a
  malformed vertex cannot violate the sort's strict weak ordering.
- `tests/native_drawing.rs::repeated_drawing_and_section_handles_are_released`
  runs 50 full draw+section cycles and then 50 refused draws followed by a
  successful draw, checking handle reuse and deterministic output.
- All drawing/section result types are statically asserted `Send + Sync`;
  `Shape` remains `!Send + !Sync` (cad-r2 compile-fail doctests still pass).

## Intentional differences and remaining gaps

1. **Error classes.** Python raises `ValueError` for bad arguments and
   `DrawingError` for a null/undrawable shape. Rust has one `DrawingError` with
   `kind() == Argument` / `Shape`; `message()` is the exact Python text. A
   caller mapping to HTTP can reproduce the Python statuses.
2. **`max_points` and numeric repr types.** The Rust API takes `usize`, so
   `int(max_points)` and its fractional/negative coercions have no Rust
   equivalent. `0` is refused with the same message; a fractional or negative
   limit is a type error. Chord tolerances and offsets are `f64`, so a Python
   caller passing an integer would see the original `repr` (e.g. `0`) while
   Rust formats the `f64` (`0.0`); the Studio request schema is a float, so the
   service path always passes floats and the messages match.
3. **Name validation order.** Python validates a view name only when it reaches
   that view; Rust validates all requested names before drawing. The refusal
   class and message are identical; only the amount of work already done
   differs when a later name is unknown.
4. **Kernel exception wording.** Python would propagate a raw
   `Standard_Failure`; Rust returns `DrawingErrorKind::Kernel` with
   `"the geometry kernel could not draw the shape: <class>"` (or `cut`). No
   product refusal depends on this path; the refusal corpus does not hit it.
5. **Pathological faces.** If `BRepBuilderAPI_MakeFace` throws while measuring
   a simple loop, Python propagates the exception; the adapter reports a
   kernel exception and Rust returns a `Kernel` drawing error. If the face is
   merely not done or inconsistent, the module uses the documented polygon
   fallback with `area_exact = false` in both implementations.
6. **Intermediate HLR memory.** Like Python, the point cap is applied after the
   kernel has produced the view's polylines; the adapter does not silently
   drop geometry. A very large B-Rep can therefore allocate proportionally to
   its edge count. The design service should keep its existing shape/STEP
   budgets; drawing itself has no wall-clock budget in either implementation.
7. **Section point-cap path.** `MAX_SECTION_POINTS` (20,000) truncation is
   implemented with Python's skip-and-mark-inexact semantics, but no test
   shape in this cell exceeds it (the Python test module does not exceed it
   either). View truncation is exercised and matches Python.
8. **Island unit-test geometry.** The native unit test uses a 40 x 30 x 24 box
   with a diameter-20 bore, not the Python test's 20 mm-tall box. In the
   compiler's 20 mm case the bore is exactly tangent to both faces, the block
   splits into two solids, and `ConnectEdgesToWires` joins their tangency into
   one self-touching wire; the three-loop nesting contract is identical and is
   verified on an unambiguous non-tangent box. The exact Python 20 mm shape is
   covered by the oracle differential (2 outer + 1 inner,
   `800 - 100*pi + 16*pi`, all exact).
9. **Windows.** The OCCT-linked Windows build was not run here (no Windows
   OCCT SDK); the portable Rust surface cross-checks for both Windows targets.
   The new adapter code is portable C++17 and adds no new library modules
   (`TKHLR` and `TKTopAlgo` were already linked).
10. **FreeCAD.** The optional FreeCAD second reader is unchanged and still
    passes/skips as before; drawings are not exported to FreeCAD.

## Reproduce

```sh
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
cd backend/crates/kala-cad
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
cargo test --locked --no-default-features
cargo test --locked --test drawing_differential -- --nocapture
cargo test --locked --test ocp_differential -- --nocapture
```

The drawing differential is mandatory: it fails (never skips) when the `.venv`
interpreter or OCP is missing. `KALA_CAD_ORACLE_PYTHON` overrides the
interpreter.
