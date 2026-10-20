# kala-geometry

Native Rust analytic preview geometry for Kala Studio. This crate replaces the
`box`, `cylinder`, and `sphere` mesh builders, their validators and defaults,
and the deterministic binary STL writer from
`src/kala_agent/studio/geometry/{limits,primitives,meshes,stl}.py`.

It does **not** implement B-rep, STEP export, Solid Edge COM, or a CAD kernel.
The Python STEP/OCP path remains an explicit separate capability.

## Public API

- `PrimitiveSpec` / `build_spec`: typed dimensions and tessellation counts.
- `build_box`, `build_cylinder`, `build_sphere`: direct typed builders.
- `build_primitive(kind, &PrimitiveParameters)`: Python-compatible dispatch.
- `build_primitive_json(kind, Option<&serde_json::Value>)`: JSON dispatch that
  keeps integer, floating, and boolean JSON kinds distinct.
- `PrimitiveParameters` / `ParameterValue`: serde/JSON parameter adapter.
- `Mesh`, `Bounds`, `compute_bounds`, `signed_volume`,
  `welded_edges_by_position_pairs`.
- `write_binary_stl`: 80-byte ASCII header, little-endian triangle count,
  50-byte records, triangular geometric normals, zero attribute counts.
- Constants: all Python defaults, minima, maxima, and mesh resource caps.

Every validation error is a `GeometryError` whose `Display` text matches the
sanitized Python `ValueError` message. Booleans are refused for numeric slots,
JSON floats are refused for integer count slots even when integral, unknown
parameter names are refused, and dimension/count range messages are identical.

## Parity evidence

`cargo test` consumes the Python-generated corpus read-only:

- all six default/maximum meshes: vertex/triangle counts, positions, normals,
  indices, compact-JSON digests, bounds, watertightness, and positive volume;
- all six STL lengths and SHA-256 digests, plus byte equality for the three
  checked-in default STLs;
- defaults, maxima, and STL layout constants.

`tests/python_differential.rs` additionally invokes the repository virtualenv
as a **test-only** oracle. It:

- re-hashes Rust-emitted float arrays with CPython `json.dumps` and compares
  those digests to the fixture digests;
- compares CPython `repr(float)` against the test-only formatter for 5,000
  random finite bit patterns;
- compares exact float bits, indices, bounds, signed volume, and STL digests
  for boundary/random valid meshes;
- compares Python `ValueError` messages for invalid type/range cases.

The library never invokes Python or another process. Set
`KALA_GEOMETRY_PYTHON` to choose an interpreter, or
`KALA_SKIP_PYTHON_DIFFERENTIAL=1` for an explicit opt-out; the fixture gate is
still authoritative.

## Bit-exactness limits

Geometry is `f64`; only STL packing narrows to `f32`. Construction uses the
same operation order as the Python source, including `math.sin`/`math.cos`,
`math.sqrt`, and the STL writer's `magnitude_sq**0.5`. Rust calls the platform
libm through the same C ABI as CPython on this host, so fixture bytes match on
this Linux x86-64/glibc host. A different libm or architecture may differ in
the last bit for trigonometric tessellations; the checked-in fixture gate is
the stronger host-specific contract. Box geometry uses only exact arithmetic
and does not depend on libm.

No performance or speedup claim is made here. `examples/geometry_cli.rs`
provides a fixture dump and a repeated-build driver for later controlled
measurement.

## Workspace membership and validation

`kala-geometry` is a member of the root `backend/` workspace and uses its
single `Cargo.lock`. From `backend/`:

```bash
cargo fmt --all --check
cargo test -p kala-geometry --locked
cargo test -p kala-geometry --locked --release
cargo clippy -p kala-geometry --all-targets --locked -- -D warnings
cargo build -p kala-geometry --release --locked
```

It is also consumed read-only as a path dependency by the `kala-step` member
and the standalone `kala-studio` workspace. The 32 accepted tests run in both
debug and release from this workspace.
