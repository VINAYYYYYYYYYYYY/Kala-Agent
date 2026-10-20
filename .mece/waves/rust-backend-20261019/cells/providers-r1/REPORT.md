# providers-r1 cell report — `kala-cad-providers`

Date: 2026-10-20
Wave: `rust-backend-20261019`
Cell: `providers-r1`
Crate: `backend/crates/kala-cad-providers`
Implementer: OpenCode Go **DeepSeek V4.1 Flash** (worker `providers-r1`)
Status: implemented and validated in isolation; ready for independent review.
No commit, push, publish, deploy, delegation, or credential read. No other
crate, root manifest/lock, Python source/test, shared fixture, adapter,
`studio/**` or other cell report was edited.

## Scope and ownership

Created only:

```text
backend/crates/kala-cad-providers/.gitignore
backend/crates/kala-cad-providers/Cargo.toml
backend/crates/kala-cad-providers/Cargo.lock
backend/crates/kala-cad-providers/README.md
backend/crates/kala-cad-providers/src/lib.rs
backend/crates/kala-cad-providers/src/base.rs
backend/crates/kala-cad-providers/src/freecad.rs
backend/crates/kala-cad-providers/src/driver.py
backend/crates/kala-cad-providers/src/solidedge.rs
backend/crates/kala-cad-providers/tests/freecad_differential.rs
backend/crates/kala-cad-providers/tests/freecad_unit.rs
backend/crates/kala-cad-providers/tests/solidedge_differential.rs
backend/crates/kala-cad-providers/tests/solidedge_mock_bridge.rs
backend/crates/kala-cad-providers/tests/support/mod.rs
backend/crates/kala-cad-providers/tests/fixtures/fake_freecad.py
backend/crates/kala-cad-providers/tests/oracle/freecad_oracle.py
backend/crates/kala-cad-providers/tests/oracle/solidedge_oracle.py
.mece/waves/rust-backend-20261019/cells/providers-r1/REPORT.md
```

The crate is a standalone workspace with its own lock. It reads
`kala-contracts`, `kala-json`, `kala-process` and `kala-solidedge-client`
through path dependencies only.

## What was built

### Contract (`src/base.rs`)

`CadProviderId` (`opencascade`, `freecad`, `solidedge`), `ALLOWED_PROVIDER_IDS`,
`Availability`, `MeasuredBounds`, `MeasuredProperties`, `ProviderBuild`,
`ProviderStatus`, `CadValueError`, `CadProviderConfigError`,
`CadProviderUnavailableError`, `CadProviderBuildError`, `CadProviderError`, the
`CadProvider` trait, and the derived provider status row. Validation, error
precedence, truncation (reason 512, version 64, warning 256, 16 warnings) and
messages are the Python `base.py` rules. There is no default provider.
`opencascade` is named in the closed set but deliberately not implemented.

The Python protocol takes a `DesignIntent`; both reference providers delete it
without reading it (`del intent`), so the Rust trait takes the resolved
`ConstructionScript` only. `probe`/`build` take `&mut self` because the probe
cache is instance state, as in Python.

### FreeCAD connection (`src/freecad.rs`, `src/driver.py`)

A real `FreeCADCmd` subprocess:

- embedded driver source byte-for-byte equal to the live Python
  `_DRIVER_SOURCE` (SHA-256 `bf95defd…bcf259`);
- canonical request JSON equal to Python's
  `json.dumps({"script": script.model_dump(mode="json", exclude_none=True)},
  sort_keys=True, separators=(",", ":"), ensure_ascii=True)`;
- explicit absolute executable, explicit argv, no shell, no PATH lookup in the
  supervisor;
- credential-scrubbed environment (`TOKEN`, `API_KEY`, `SECRET`, `PASSWORD`,
  `CREDENTIAL`) with `PYTHONPATH` removed and the `KALA_FREECAD_*` protocol
  variables added;
- bounded wall time 5–600 s (120 s default), 20 s version probe, bounded
  stdout/stderr capture, native process-tree cleanup;
- STEP and FCStd output, workspace-contained paths, SHA-256 and size
  verification, 64 MiB native / 1–512 MiB STEP ceilings;
- the same measured values (`volume_mm3`, `area_mm2`, `bounds`), `valid`,
  `solid_count`, `warnings`, `freecad_version` and `native_kind = "fcstd"`;
- the same generic-refusal ` (op: fuse|compound)` suffix logic; a place-only
  assembly is **not** pre-refused — the adapter owns the export target, as in
  the current reference.

### Solid Edge connection (`src/solidedge.rs`)

Probe and build through `kala-solidedge-client`:

- exact Python URL resolution (loopback `http` only, explicit port, no
  userinfo/path/query/fragment, `localhost`/`127.0.0.1`/`::1`);
- token file resolution (`token_file` → `KALA_SOLIDEDGE_TOKEN_FILE` →
  `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`), ASCII, trimmed,
  16–256 characters;
- authenticated `GET /v1/health` through the accepted client, no proxy, no
  redirect;
- `available` only when the bridge says `available` and `supported`;
- the exact off-Windows reason and the exact
  `Solid Edge build requires the Windows bridge (not reachable)` refusal;
- a reachable bridge still refuses a build with `op: bridge.build`.

## Differential evidence

### FreeCAD — real FreeCAD 1.1.4 on this host

`tests/freecad_differential.rs` runs the repository Python
`FreeCadProvider` in an isolated interpreter against the live sources and
compares:

- all 28 cases of `tests/fixtures/construction_parity.json`;
- 15 deterministic randomized scripts (pads on all planes, circles/rectangles,
  extrudes, revolves, pockets, a placed part, two place-only assemblies, a fuse
  assembly).

Result on the final run:

```text
FreeCAD differential: 43 builds and 0 refusals matched; 28 corpus cases, 15 randomized cases
```

For every case the test compared:

- the exact canonical script hand-off bytes (43/43 equal);
- the embedded driver source (equal, SHA-256 `bf95defd…bcf259`);
- `volume_mm3` and `area_mm2` (exact equality of the parsed float);
- `bounds` min/max, `warnings`, `valid`, `solid_count`, `provider_version`,
  native artifact presence (exact);
- the STEP magic (`ISO-10303-21`) and the FCStd zip magic (`PK`);
- every Rust STEP imported by the live Python `import_step_bytes`: bounds
  within 1e-6 mm and solid counts exactly equal to the Python provider's STEP
  import.

Representative matched values from the live provider:

| case | volume mm³ | bounds mm | solids |
|---|---|---|---|
| `rectangle_pad_xy_origin_forward` | 120.0 | (0,0,0)–(4,5,6) | 1 |
| `assembly_two_parts_placed_once` | 1000.0 | (0,0,0)–(30,10,5) | 2 |
| `assembly_three_parts_placed_once` | 600.0 | (0,0,0)–(25,30,2) | 3 |
| `assembly_single_translated_part` | 500.0 | (5,6,7)–(15,16,12) | 1 |
| `pocket_circle_xy_through_all` | 3968.584073 | (0,0,0)–(20,20,10) | 1 |

The current adapter accepts every corpus script, so the live differential has
no refusal. Refusal parsing, message truncation and the ` (op: …)` suffix are
covered by `freecad_unit.rs` against a fake `FreeCADCmd`.

### Solid Edge — live Python provider on Linux

`tests/solidedge_differential.rs`:

- **probe**: 27 cases — Linux/default/macOS platform reasons, nine invalid
  URL shapes, missing/short/long/non-ASCII/whitespace tokens, the
  `KALA_SOLIDEDGE_TOKEN_FILE` and `LOCALAPPDATA` sources, available/
  unavailable/no-capability/blank-detail health documents, 401, invalid JSON,
  redirect, `localhost` and a trailing slash. Availability (available,
  version, reason) matched exactly in every case.
- **build**: 4 cases — Linux, reachable bridge with no executor, bridge with
  no capability, unreachable bridge. The exception kind, message, reason,
  provider and op matched exactly.
- **URL corpus**: 39 fixed URLs plus 180 deterministic generated URLs,
  compared with no network (a missing token stops accepted URLs before a
  socket). 219/219 matched, with Python's `urlparse().port` `ValueError`
  cases asserted as the documented native difference.

`tests/solidedge_mock_bridge.rs` adds 13 mock-bridge tests: authenticated GET
`/v1/health`, no `Origin` header, no token in any failure message, no redirect
followed, token-source precedence, cache behaviour, non-protocol documents,
the documented strictness differences, loopback-only URL validation, the
reachable-but-unbuildable refusal, and the Linux refusal that opens no socket.

## Test counts

| Target | Debug | Release |
|---|---|---|
| `src/lib.rs` unit tests | 0 | 0 |
| `freecad_differential` | 1 | 1 |
| `freecad_unit` | 20 | 20 |
| `solidedge_differential` | 3 | 3 |
| `solidedge_mock_bridge` | 13 | 13 |
| doc tests | 0 | 0 |
| **total** | **37 passed, 0 failed** | **37 passed, 0 failed** |

The differential skips (does not fake a build) only when `FreeCADCmd` is
absent. It ran for real on this host in both profiles.

## Gates

All run in `backend/crates/kala-cad-providers`:

```text
cargo fmt --check                                              pass
cargo clippy --locked --all-targets --all-features -- -D warnings   pass
cargo test --locked                                            pass (37)
cargo test --locked --release                                  pass (37)
cargo build --locked --release                                 pass
cargo clippy --locked --target x86_64-pc-windows-gnu \
    --all-targets --all-features -- -D warnings                pass
cargo check --locked --target x86_64-pc-windows-msvc \
    --all-targets                                              pass
```

Both Windows targets are installed on this host; only compilation was
checked. No Windows runtime, no Solid Edge installation and no native Solid
Edge build exists here.

One `cargo test --release` run failed transiently while a concurrent worker
rewrote `adapters/freecad/src/kala_agent_freecad/cad/design.py` (mtime
01:25). Re-runs against the current sources are green in debug and release.
The Python reference provider itself changed concurrently at 01:20 (from
`design.run` to `worker.invoke_design`, removing the place-only pre-refusal);
this cell updated to the final observed reference and verified the driver
hash before the final gates. No Python/adapter file was edited by this cell.

## Security evidence

- **No shell, no PATH lookup at spawn.** `ProcessSpec` takes an absolute
  executable and an explicit argv. An executable path containing spaces is
  tested.
- **Environment is explicit and scrubbed.** Tested with `OPENAI_API_KEY`,
  `KALA_WORKER_TOKEN`, `DATABASE_PASSWORD`, `MY_SECRET`, `AWS_CREDENTIAL` and
  `PYTHONPATH`; none reaches the child. The protocol variables and
  `PYTHONDONTWRITEBYTECODE=1` are present.
- **Bounded process.** 5–600 s wall (120 default), 5 s bootstrap, 5 s
  termination, 64 MiB stdout / 16 MiB stderr caps, native process-group/job
  cleanup from `kala-process`. Resource limits are deliberately disabled to
  match Python (a provider export may exceed the tools executor's 8 MiB
  default).
- **Artifacts are confined and verified.** The resolved path must be inside
  the canonical workspace (symlink escape tested), the declared size must be
  within the limit and equal to the read bytes, and SHA-256 must match the
  driver's digest.
- **Response envelope is bounded** to 4 MiB and parsed strictly; a missing,
  invalid, non-object, or oversized response is a typed "did not report a
  result" with a bounded, whitespace-collapsed stderr tail.
- **Solid Edge stays on loopback.** The URL must be `http`, loopback, with an
  explicit port; proxies are unused and redirects are refused. The token never
  appears in a message (tested with a 401). On Linux no token is read and no
  socket is opened.
- **No real credential was read.** The token-path code is exercised only with
  test fixtures in temporary directories; on this Linux host the platform
  check short-circuits before any token read.

## Intentional differences

The full list is in `README.md`; summary:

FreeCAD: filtered environment keys for `kala-process`; discovered adapter
search path instead of Python `purelib`; strict `kala-json` response parsing
(duplicate/non-finite refusal, overflowing ints dropped); bounded stderr
capture instead of an unbounded log; lossy probe decoding; absolute
resolution of a relative explicit executable; any deadline maps to the same
timeout message.

Solid Edge: protocol-v1 `Health` strictness (wrong version, unknown/missing
fields, over-long fields/body); `localhost` transported as `127.0.0.1`;
out-of-range/non-numeric ports classified as an invalid URL instead of
propagating Python's `ValueError`; missing home returns the token-file reason
instead of `RuntimeError`; 1 MiB client JSON cap vs Python's 64 KiB health cap.

## Remaining gaps

- No runtime cutover, Windows-runtime, speed or readiness claim.
- No Windows execution of `kala-process` or the Solid Edge bridge.
- No `opencascade` provider here, by task design; the geometry worker keeps it.
- `DesignIntent` is not part of the Rust trait because both reference
  providers ignore it; a future service resolves the intent before calling.
- Cancellation is not exposed on the provider API (Python has none either);
  `kala-process` supports a `CancelToken` if a caller needs it later.
- The adapter `sys_path` discovery is a repository-layout heuristic; production
  deployment should pass `FreeCadProviderOptions::sys_path`.
- The live corpus currently yields no FreeCAD refusal, so adapter refusal
  messages are proven with the fake executable, not with real FreeCAD.
- The Python reference is untracked and was edited concurrently during this
  cell; the driver-source differential will fail on the next reference change
  by design, which is the intended signal to re-sync.
