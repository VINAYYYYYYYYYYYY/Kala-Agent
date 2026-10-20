# Native Rust analytic geometry and binary STL — cell report

**Status: implementation complete for the assigned geometry slice.**
Native Rust box/cylinder/sphere meshes, validation, resource caps, and
deterministic binary STL are implemented in `backend/crates/kala-geometry`.
The Python-generated corpus is consumed read-only and passes byte-for-byte on
this host, including all six STL digests and the three checked-in default STL
files.

No Python runtime fallback, STEP/B-rep implementation, service cutover,
packaging change, commit, push, or deployment was performed. No speedup claim
is made.

## 1. Scope and ownership

Owned and created:

- `backend/crates/kala-geometry/**` — crate, lockfile, tests, example, README.
- `.mece/waves/rust-backend-20261019/cells/geometry/REPORT.md` and
  `SESSION.md`.

Read-only and untouched:

- `backend/Cargo.toml` and `backend/Cargo.lock` (foundation owns them). The
  foundation exclusion has landed:
  `exclude = ["crates/kala-geometry", "crates/kala-budget"]`.
- `backend/fixtures/**` and `backend/compat/**`.
- All Python product and test source.
- `Rust+buid123/`, `rust-implementation.md`, docs, adapters, workflows.

The crate is standalone: its own `Cargo.lock` and
`backend/crates/kala-geometry/target`. The parent workspace was not edited.

## 2. Deliverables and public API

```
backend/crates/kala-geometry/
  Cargo.toml / Cargo.lock
  README.md
  src/lib.rs          re-exports and crate documentation
  src/error.rs        GeometryError (sanitized Display text)
  src/limits.rs       Python defaults, minima/maxima, resource caps, validators
  src/primitives.rs   PrimitiveKind, DimensionValues, PrimitiveSpec, defaults
  src/params.rs       serde/JSON parameter adapter preserving int/float/bool kind
  src/meshes.rs       Mesh, Bounds, builders, winding, normals, welds, volume
  src/stl.rs          deterministic binary STL writer
  examples/geometry_cli.rs  fixture dump and repeated-build driver
  tests/fixture_parity.rs
  tests/mesh_invariants.rs
  tests/validation.rs
  tests/python_differential.rs
  tests/oracle/geometry_oracle.py   test-only Python oracle
  tests/common/mod.rs               fixture/hash/CPython-float-repr support
```

Public entry points:

- `PrimitiveSpec` + `build_spec`, or `build_box` / `build_cylinder` /
  `build_sphere` for typed Rust callers.
- `build_primitive(kind, &PrimitiveParameters)` for Python-compatible dispatch.
- `build_primitive_json(kind, Option<&serde_json::Value>)` and
  `PrimitiveParameters` (serde `Deserialize`) for JSON boundaries.
- `Mesh` (private invariant-preserving fields with accessors),
  `Mesh::from_parts`, `Mesh::empty`, `Bounds`, `compute_bounds`,
  `signed_volume`, `welded_edges`, `welded_edges_by_position`,
  `welded_edges_by_position_pairs`.
- `write_binary_stl` plus `STL_HEADER_SIZE`, `STL_HEADER_FILL`,
  `STL_TRIANGLE_SIZE`, `STL_BINARY_FORMAT`, `TRIANGLES_PER_FACE`.
- All Python defaults/minima/maxima/resource constants.

Behavior preserved:

- Dimensions: finite `f64` in `[0.1, 2000.0]`, inclusive; booleans refused;
  ints and floats accepted for dimensions.
- Counts: JSON integers only (`2.0` refused), booleans refused, exact
  inclusive ranges (box 1–64, cylinder radial 6–256, height 1–64, sphere rings
  3–96, sectors 4–128).
- Unknown kinds and unknown parameter names refused with the Python messages.
- Python construction order for positions, `_enforce_outward` winding,
  per-vertex normal accumulation, bounds, position-keyed welds (ties-to-even
  millimetre keys), and signed volume.
- STL: 80-byte space-padded header, LE `u32` triangle count, per-triangle
  geometric normal (`magnitude_sq**0.5`), 12 little-endian `f32` values, zero
  `u16` attribute count, mesh index order.
- `f64` until STL packing; no fast math or alternate tessellation.

## 3. Verification actually run

Environment: Linux x86-64, glibc, Rust/Cargo 1.99.1, CPython 3.14.8 in the
repository `.venv`. Dependencies locked in the crate lockfile: serde 1.0.229,
serde_json 1.0.154 (arbitrary precision), sha2 0.10.9 and ryu 1.0.23
(dev/test only). Package `kala-geometry` 0.1.0, edition 2024,
`rust-version = 1.85`.

| Command (from crate manifest) | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo test` | pass: 32 tests (16 unit, 4 fixture, 3 invariants, 1 Python differential, 8 validation) |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo build --release` | pass |
| `cargo test --release` | pass: same 32 tests, including byte gate |
| `cargo run --release --example geometry_cli -- fixtures` | six summaries match fixture counts, bounds, signed volumes, STL lengths and SHA-256 |

The release CLI fixture run reproduced exactly:

| Mesh | STL bytes | SHA-256 |
|---|---:|---|
| box default | 684 | `e6c04f2a46a0897aaa157a63b3efc89f0632201728753ce82c7ca0e7d2c7dfd5` |
| box max | 2,457,684 | `6ae74307e1e3ac9101e3fcf0a2ca8ebd3af1b04fc88be7f269a82429da2febde` |
| cylinder default | 9,684 | `0c87786bde66b7cffd9a7c71ff7b501922c4d747eb8e768f4ae7469ca8548243` |
| cylinder max | 1,664,084 | `cd67e50d0b99d4bb364e1b7c32c35187c55ec0752cf0072df610e62c770efcec` |
| sphere default | 73,684 | `ea50ff2ea4f09bffcee161e017f85565f0ad87434ea8c4743881d40f202419e3` |
| sphere max | 1,216,084 | `b7b27e4721efe1f832eca21370e10406039305bec941baf4e0e463b8cc32ea61` |

The three checked-in default STL files are compared byte-for-byte with
`include`-free `std::fs::read`; max STLs are compared by length and SHA-256.

Test coverage details:

- All six default/max meshes: vertex/triangle counts, compact-JSON digests for
  positions/normals/indices, first/last samples, bounds, watertightness, and
  positive volume; STL length/hash/header/count for all six.
- Defaults, maxima, STL layout constants, and the corpus manifest digest.
- Deterministic random valid dimensions/counts (24 generated meshes plus
  minimum and maximum tessellations): finite arrays, valid non-degenerate
  indices, outward winding by centroid dot product, unit normals,
  position-keyed two-use watertightness, box exact bounds/volume,
  cylinder/sphere bound and volume envelope, and STL record bytes.
- Every STL record's normal is recomputed independently in the test and
  compared as `f32`, positions are compared per corner to the packed source
  vertices, and attribute counts are checked zero.
- Validation boundaries: 0.1/2000 inclusive, just outside both, NaN/±inf,
  all count minima/maxima, booleans, strings, null, arrays, objects, integral
  JSON floats in count slots, unknown kinds/names, and non-echoing messages.
- Resource caps: 200,001 vertices and 400,001 triangles rejected with exact
  messages; mesh structural mismatches rejected.
- Python oracle (test only): CPython `json.dumps` re-hashes Rust-emitted arrays
  and matches every fixture compact digest; CPython `repr(float)` matches the
  test formatter for 5,000 random finite bit patterns plus fixed threshold
  values; 29 fixed/random valid meshes match exact float bits, indices,
  bounds, signed volume, and STL digest/length; 18 invalid cases match
  `ValueError` type and text.

No timing or throughput number is reported. `geometry_cli` prints elapsed time
only as a driver for the conductor's later interleaved measurement.

## 4. Parity gaps and limitations

1. **Platform libm.** Trigonometry uses the platform libm through the same C
   ABI as CPython. Fixture bytes are exact on this Linux x86-64/glibc host.
   Another libm/architecture can differ in the last bits of cylinder/sphere
   positions and normals. Box geometry uses only exact arithmetic. The checked-in
   host gate is the stronger contract; there is no cross-platform byte claim.
2. **STEP / B-rep / Solid Edge COM are not implemented.** The fixture STEP
   record is informational and untouched. This crate is a preview tessellator,
   not a CAD kernel and not a COM adapter. No claim of a full geometry stack.
3. **Extremely wide JSON integers.** Integers beyond `i128` use their decimal
   text. Counts are still treated as integers and rejected by range. A
   dimension wider than the `f64` range (>~1e308) maps to the sanitized
   "must be a finite number" message, whereas Python's `float()` raises
   `OverflowError` (which the user-facing Python server does not catch as a
   422). The Rust behavior is deliberately sanitized; no fixture covers this.
4. **Strengthened native API edges.** `compute_bounds` refuses malformed
   non-multiple-of-3 arrays with a sanitized error instead of Python's
   `min()` exception; `Mesh::from_parts` additionally checks normal length,
   index triples, and index bounds. Product builders always satisfy these.
   A non-object top-level parameter value gets the adapter message
   "parameters must be a JSON object"; the Python service schema rejects that
   shape before geometry with a Pydantic error.
5. **Python differential availability.** The live differential test requires
   the repository virtualenv/importable product source or
   `KALA_GEOMETRY_PYTHON`. It fails rather than silently skipping when the
   oracle is missing; `KALA_SKIP_PYTHON_DIFFERENTIAL=1` is an explicit opt-out
   for offline runs. It ran successfully here.
6. **No Windows execution evidence.** Nothing was run on Windows. No COM,
   STEP, NSIS, or Windows CI result is claimed.
7. **No performance claim.** The example is a driver, not a benchmark result;
   no Python-vs-Rust speedup is asserted.

## 5. Handoff

The crate is ready for Sol review and later workspace integration. Integration
should preserve the standalone lock/target during review and then either move
the crate under the parent workspace or keep its own lock as the conductor
decides. The crate does not depend on `kala-json`; its test-only float
formatter lives under `tests/` and is not a replacement for canonical JSON.
