# cad-r2 REPORT: kala-cad OpenCascade adapter core

Cell: `cutover-20261020/cad-r2` (Rust worker, DeepSeek V4.1 Flash)
Scope: `backend/crates/kala-cad/**` only (plus this cell report). No commit, push, sudo,
delegation or credential reads. Every shell used a temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; the user data
tree was never read or written.

Status: **complete for the requested core**. Compile, measure, tessellate, STEP write
(with corpus normalisation) and view-only size-limited STEP read are implemented over
real OCCT 7.9.3 through a narrow C++ adapter. All requested gates pass; the live
differential has zero mismatches.

## Starting point

The interrupted cad-r1 left a standalone crate with 64 passing tests: a narrow adapter
(1132 lines of C++), the compiler/measure/mesh core, STEP round-trip, and an OCP oracle.
I inspected it and kept the sound parts. This cell added the missing contract surface and
hardened the boundary:

- corpus STEP normalisation (timestamp + translator counter);
- view-only STEP import with the Python size/mesh/coordinate policy and exact messages;
- `kala_optimal_box_ex` (the importer must use the exact B-Rep envelope, not the
  triangulation-aware box);
- adapter RAII for mesh/buffer allocation paths and bounded error-text copying;
- portable build discovery (CMake, pkg-config, explicit env, vcpkg/MSVC);
- STEP view and normalisation differentials, a Python-written STEP import test, a
  deterministic randomized script corpus, expanded refusal matrix, 1,000-cycle leak
  measurement and negative `Send`/`Sync` doctests;
- README with the Windows MSVC + vcpkg build/run notes.

## Public API

Kernel-independent policy is available in both feature modes:

- `Bounds`, `Measurement`, `Check`, `CheckReport`, `CheckValue`, `build_checks`
- `MeshData`, `MeshOptions`, `stl_bytes`, mesh defaults/limits
- `StepImportOptions`, `StepWriteOptions`, `StepImportError`, `StepImportErrorKind`
- `validate_step_import_max_bytes`, `resolve_step_import_max_bytes`,
  `resolve_step_import_max_bytes_from_env`
- `normalise_step_bytes`, `FIXED_STEP_TIMESTAMP`, `STEP_HEADER`,
  `DEFAULT_MAX_STEP_BYTES`, `MIN_STEP_IMPORT_BYTES`, `MAX_STEP_IMPORT_BYTES`,
  `MAX_VIEW_COORDINATE_MM`, `MAX_WARNING_LENGTH`, `STEP_IMPORT_MAX_BYTES_ENV`
- capability types (`Capability`, `probe_capability`, `design_available`) and frozen
  compiler constants

Native (`feature = "native"`, the default):

- `compile_script`, `compile_script_with_options`, `compile_json_str`,
  `compile_json_bytes`, `CompilationResult`, `CompiledComponent`, `CompileOptions`
- `Shape`, `CompileError`/`CompileErrorKind`, `MeasurementError`, `MeshError`,
  `StepError`, `KernelError`
- `measure_shape`, `measure_and_check`, `topology`
- `tessellate`
- `step_bytes`, `step_bytes_with_options`, `step_shape`, `import_step_view`,
  `StepImportOutcome`
- `probe_capability`, `design_available`

`Shape` owns exactly one C++ `TopoDS_Shape`; it is deliberately not `Send`/`Sync`.
Plain measured data (`Measurement`, `Bounds`, `MeshData`, `CheckReport`) is `Send + Sync`.
STEP export/import is serialised by a private mutex because OCCT stores STEP unit/schema
settings in a process-global registry.

## Changed paths

All under `backend/crates/kala-cad/` unless noted:

| Path | Change |
| --- | --- |
| `src/lib.rs` | public re-exports, shared STEP policy |
| `src/step_common.rs` | **new** limits, options, corpus normaliser, env resolution |
| `src/error.rs` | `StepImportError` / `StepImportErrorKind` |
| `src/native/mod.rs` | extended native re-exports |
| `src/native/ffi.rs` | `kala_shape_is_valid`, `kala_optimal_box_ex` declarations |
| `src/native/shape.rs` | `Shape::is_valid`, `!Send`/`!Sync` compile-fail docs |
| `src/native/measure.rs` | triangulation-aware vs exact optimal box |
| `src/native/step.rs` | normalised write option, view import, exact messages |
| `src/ffi/adapter.cpp` | RAII mesh/buffer paths, bounded error copy, is-valid/box-ex, transfer status |
| `src/unavailable.rs` | same STEP-facing API in the explicit unavailable mode |
| `build.rs` | CMake/pkg-config/env/vcpkg discovery, MSVC/vcpkg flags, rpath |
| `README.md` | **new** API, build, Windows, STEP and test notes |
| `tests/common/mod.rs` | temp dir and oracle interpreter helpers |
| `tests/oracle/ocp_oracle.py` | import-view, import-error and normalisation differentials |
| `tests/oracle/step_writer.py` | **new** test-only Python STEP writer |
| `tests/ocp_differential.rs` | 24 deterministic random scripts; STEP view/normalisation parity |
| `tests/native_step.rs` | Python-written STEP, normalisation, size/malformed/coarse-retry tests |
| `tests/native_ffi.rs` | 1,000-cycle leak measurement, Send/Sync plain-data check |
| `tests/unavailable.rs` | new STEP API in unavailable mode |

No product Python source under `src/` or `adapters/`, no shared fixture, no existing
adapter, no studio, root manifest/lock or other crate was edited. The Python files in
`tests/oracle/` are test-only helpers owned by this crate. The shared
`tests/fixtures/construction_parity.json` was read only and its digest is checked in the
differential output (`72cdb6f5…f972`).

## Gates (exact results)

Environment: Linux x86_64, rustc 1.99.1 (edition 2024), OCCT 7.9.3, OCP 8.0.1.1,
Python 3.14.8, `.venv/bin/python`.

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test --locked` (debug) | **73 tests + 2 doctests, 0 failures** |
| `cargo test --locked --release` | **73 tests + 2 doctests, 0 failures** |
| `cargo build --release` | pass |
| `cargo test --locked --no-default-features` | **6 tests, 0 failures** |
| `cargo check --target x86_64-pc-windows-msvc --no-default-features` | pass |
| `cargo check --target x86_64-pc-windows-gnu --no-default-features` | pass |
| `cargo check --target x86_64-pc-windows-msvc` (native) | fails only because no Windows OCCT SDK exists here; message names `OCCT_INCLUDE_DIR`/`OCCT_ROOT`/`VCPKG_ROOT` and the tried paths |

Per-target debug counts: lib 4, FreeCAD optional 1, native_compile 23, native_ffi 11,
native_measure 11, native_mesh 10, native_step 12, OCP differential 1, unavailable 0,
doctests 2. Release counts are identical. The unavailable build has 3 policy tests in
`lib` plus 3 explicit-unavailable tests.

## Live differential (mandatory oracle)

`cargo test --locked --test ocp_differential -- --nocapture` runs the real Python
`compile_ocp.py`, `measure.py`, `mesh.py` and `importer.py` against the native results.

```
cases: 60  (28 frozen fixture + 8 boundary + 24 deterministic random)
parts: 89
refusals: 16
meshes: 5
steps: 5
stls: 4
imports (view): 6
import failures: 5
normalisations: 5
mismatches: 0
```

Compared fields: warnings, `fused`, component order/names, per-part and assembly volume,
surface area, centroid, optimal bounds, B-Rep validity, solid count, unique topology
counts, mesh vertex/triangle counts and bounds, STEP re-import volume/bounds/topology,
STL reader face count/bounds, view-import bounds/solids/warnings/mesh counts, import
refusal code+message, and normalised bytes.

The deterministic corpus is an internal LCG (seed `0x4b414c4143414421`): no clock, no
external RNG, no network. It covers XY/XZ/YZ, forward/reversed extrusions, rectangle and
circle profiles, finite and through-all pockets, multi-additive components, placements,
fused subsets and unfused compounds.

Oracle source digests (from the run):

```
compile_ocp.py 3401af76ccb5c8c2b9f9c4701bbdf78fdb0b6724742351b3b496d31444967652
measure.py     6b18a40852cb22de65fe39d156441107cbf8741807d2f3fcc9a02744acedd6a2
mesh.py        f83c1d8a034010b2ae1f128bc0e4af12dfb069864f083d91b29bf450d69d5949
importer.py    535ed687f63f678bb2214f9d6e5fad3251e2da8059e53a41d82593a2e32b56fa
construction.py b742475ebe0fc5a9632366e05d511f2a3f3de87d7517e526fa4b832be7936eea
corpus common  bb0641303165949cb84b50873a81cb8296c8d8b5774cd188ed1f85ad79611370
fixture        72cdb6f5de4de93d6d87432b639f6ab686000411859518999dde10f897c5f972
```

## Parity evidence

- **L-bracket**: Python reference `35571.416047113315 mm³`; native test passes at
  relative `1e-6`, and against the closed form at `1e-9`. Bounds
  `[-30, 0, 0] .. [30, 45, 40]` at `0.001 mm`; one valid solid.
- **28-case corpus**: every fixture part/assembly is compiled and measured in
  `native_compile.rs` (volume `1e-6` relative, bounds `0.001 mm`) and cross-checked live
  against Python in the differential.
- **Refusals**: exact Python strings for `helix`, assembly `compound`, every analysis op
  (`material`, `fix`, `force`, `pressure`, `mesh`, `solve`), every edit op (`move`,
  `resize`, `fillet`), the fillet map, rectangle revolve on XZ, cross-plane pocket,
  duplicate placement, cut-all, and the compiler invariants. 16 refusal cases in the
  differential, zero message mismatches.
- **STEP**: Rust-written L-bracket/hole plate/compound/translated/multi-additive STEP is
  re-read by OCCT/Python and by the Rust view importer with volume `1e-6` and bounds
  `0.001 mm`. A Python-written STEP (real `compile_ocp.py` + `STEPControl_Writer`) is
  imported by Rust and matches Python's own measurement at `0.001 mm`; the Rust-written
  STEP for the same script matches it as well.
- **Normalisation**: all five native exports are normalised identically by Rust and the
  corpus helper `normalise_step_bytes` (byte equality, length preserved). Raw STEP bytes
  are not claimed to be stable.

## FFI and memory safety

- Every exported C function catches `Standard_Failure`, `std::exception` and `...`, and
  returns a status plus a bounded (≤1024-byte) exception class name. No exception crosses
  the C ABI; Rust never unwinds into C++.
- `set_error` uses a bounded scan instead of `strlen`, so the fixed transport buffer is
  never over-read.
- Shapes, meshes and buffers have one Rust owner and one matching free. Mesh creation and
  STEP-buffer creation now use `std::unique_ptr` and release only on success, so a
  `std::bad_alloc` after `new` cannot leak; earlier `delete` on selected paths was
  removed.
- `kala_mesh_copy`/`kala_buffer_copy` validate destination capacities before writing.
- Repeated failure reuse: 50 cut-all failures followed by successful compile/mesh/STEP,
  and 50 malformed STEP imports followed by a successful round trip.
- Concurrency: 8 threads × 20 independent compiles/measure/mesh (160 results), and
  6 threads × 5 STEP export/import round trips, serialised only for the global STEP
  settings. `Shape` is not shared across threads.
- Leak check: 1,000 full compile/measure/tessellate/STL/drop cycles after warm-up grew
  resident memory by **6,610,944 bytes** (about 6.6 KiB per cycle), below the test's
  conservative 128 MiB bound; no monotonic unbounded growth is claimed either way. OCCT
  keeps internal pools, so this is reported as measured rather than “zero”.
- `Shape`'s `!Send`/`!Sync` property is enforced by two `compile_fail` doctests; the
  plain-data types are statically asserted `Send + Sync`.
- AddressSanitizer smoke check: `KALA_CAD_SANITIZE=address` built the instrumented
  adapter and the 1,000-cycle compile/measure/mesh/drop test passed with no ASan
  memory-error report (`ASAN_OPTIONS=detect_leaks=0`, because OCCT keeps process
  pools). Resident growth under ASan was 16,908,288 bytes, consistent with
  instrumentation overhead rather than a new per-cycle handle leak.

## Build and Windows notes

Discovery is deterministic and download-free: explicit `OCCT_INCLUDE_DIR` /
`OCCT_LIB_DIR` (also `KALA_OCCT_INCLUDE_DIR`/`KALA_OCCT_LIB_DIR`), `OCCT_ROOT` /
`KALA_OCCT_ROOT`, `OpenCASCADE_DIR`/CMake `OpenCASCADEConfig.cmake`, pkg-config on Unix,
`VCPKG_ROOT`/`VCPKG_INSTALLATION_ROOT` with `VCPKG_TARGET_TRIPLET`, then platform
defaults. The include dir must contain `Standard_Version.hxx`; the lib dir must contain
`TKernel.lib`/`libTKernel.so*`/`libTKernel.dylib`.

Windows MSVC + vcpkg (documented in `README.md`):

```bat
set VCPKG_ROOT=C:\src\vcpkg
set VCPKG_TARGET_TRIPLET=x64-windows
cargo build --release
```

`x64-windows` gives headers in `installed\x64-windows\include\opencascade`, import
libraries in `...\lib`, and DLLs in `...\bin`. Add `...\bin` to `PATH` (or copy the DLLs
next to the executable) for tests. A static-CRT triplet needs `KALA_CAD_STATIC_CRT=1` so
the adapter is compiled `/MT`. A MinGW link against an MSVC OCCT distribution is not
supported; use an OCCT build for the same toolchain.

On this Linux host I verified the vcpkg path discovery with a synthetic
`VCPKG_ROOT/installed/x64-windows` tree: the build found the include and `TKernel.lib`
and proceeded to compile, then failed only because there is no MSVC cross compiler/OCCT
header set here. The portable Rust surface (`--no-default-features`) cross-checks cleanly
for both `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu`.

**No Windows runtime or OCCT-linked Windows build is claimed.**

## Intentional differences and remaining gaps

1. **Raw/normalised STEP bytes are not a cross-export contract.** Two exports of the
   same complex L-bracket differed in entity numbering/order (and therefore length)
   before normalisation. The corpus normalises only the `FILE_NAME` timestamp and the
   per-process translator counter; geometry, units, solid count, volume and bounds are
   the contract. The differential compares those and the normalised bytes of the *same*
   export.
2. **Coarser retry is faithful but usually ineffective.** OCCT keeps the triangulation
   it attached on the first `BRepMesh_IncrementalMesh`; a second call with 0.5 mm reuses
   it. `importer.py` has the same behaviour, so the retry path is implemented and
   message-compatible, but a coarsened success is only reachable when the first attempt
   failed before a triangulation was attached. The test documents this rather than
   pretending the retry always coarsens.
3. **No SHA-256 in the crate.** Python's importer returns `step_sha256`; the native core
   returns the bytes-derived facts and leaves the digest to the artifact store/design
   service, which already owns content addressing. This avoids a hashing dependency in
   the geometry crate.
4. **STEP unit setting.** The adapter sets `write.step.unit=MM` explicitly (Python relies
   on the OCCT default). Geometry matches; raw bytes are not compared.
5. **Zero-placement handle copy.** `CompiledComponent.placed_shape` is an independent
   kernel handle even when the translation is zero; Python reuses the same object. The
   validation/measurement behaviour is equivalent.
6. **`topology()` is additive.** It reports unique sub-shape counts (indexed map);
   `Measurement.solid_count` remains the source-compatible explorer occurrence count.
7. **Out of scope, as scheduled:** HLR drawing and section views (`cad-r3`), Studio
   journal/artifacts, HTTP/auth, design service, desktop packaging. The exact `Shape`
   and STEP view outcome are the hand-off surface.
8. **Windows execution** is not tested here; only the portable surface cross-checks and
   the build-discovery path are verified. The final OCCT-linked Windows build belongs to
   the wave's Windows script/host.

## Reproduce

```sh
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
cd backend/crates/kala-cad
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --release
cargo test --locked --no-default-features
cargo test --locked --test ocp_differential -- --nocapture
```

The differential is mandatory: it fails (never skips) when the `.venv` interpreter or
OCP is missing. `KALA_CAD_ORACLE_PYTHON` overrides the interpreter. The FreeCAD check is
an optional second reader and skips only when no FreeCAD executable exists.
