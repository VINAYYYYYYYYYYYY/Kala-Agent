# Cell polish — clearer design instruction, per-pocket warnings, loose ends

Date: 2026-10-20
Cell: `polish` (agent-providers wave)

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and
`CARGO_HOME=/home/v9i/.cargo` and then set fresh temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`
(`/tmp/opencode/kala_env.sh`). Every Studio the tests launched received an
explicit temporary `--data-dir`. No credential file and no settings file was
read. Nothing under `/home/v9i/.local/share/kala-agent/**` was opened, named in
a command, or written. The live Studio on **port 8831** was never stopped,
restarted or addressed by any test; it was still listening and healthy
(`GET /api/studio/health` → 200, pid 2655785) after this cell finished. Every
live server this cell started used an ephemeral port. No commit, push or sudo.

Files shared with the concurrent `python-parity` cell
(`src/kala_agent/studio/providers/**`, `src/kala_agent/studio/{settings,schemas}.py`,
the Python `server.py`) were **not** edited. The Rust shared files
(`kala-studio/src/providers/mod.rs`, `server.rs`) were re-read immediately
before the single mapping hunk.

## 1. The instruction

### Final text (byte-identical in all three source copies)

The paragraph added directly after "…and the component must end with a solid.":

> Sketch coordinates are in the sketch plane: a rectangle's x/y is its lower-left corner, so it spans x to x+width and y to y+height; a circle's x/y is its centre. Every pocket must cut into material that its own component already has at that step; a cut where the component has no material removes nothing, and the build is rejected. A component has at most 12 steps; when a part needs more, split it into components that each carry their own features, place them, and fuse them.

**Length: 3 549 bytes / 3 549 characters in both languages** (the instruction is
pure ASCII; the previous text was 3 069 bytes). The three copies are:

- `backend/crates/kala-design-intent/src/intent.rs:37-59` (the contract crate)
- `backend/crates/kala-studio/src/chat_design.rs:49-71`
- `src/kala_agent/studio/design/intent.py:45-112`

`grep` also finds the old text in two **generated, gitignored packaging
stages** (`studio/release/linux-unpacked/resources/backend/...` and
`studio/backend-native/...`). They are not source; `studio/scripts/native-backend.cjs`
regenerates them on the next packaging run. They were deliberately not
hand-edited (see Gaps).

Byte equality was checked three ways: a direct extraction comparison, the
existing Rust `the_instruction_matches_the_contract_crate` test, and the live
`live_python_schema_is_byte_identical` differential (6/6 tests pass).

### The facts, verified against the code

| Fact | Evidence |
|---|---|
| A rectangle's `x/y` is its lower-left corner in the sketch plane; it spans `x..x+width`, `y..y+height`. | Rust `kala-cad/src/native/compile.rs:492-497` (`u0 = sketch.x()`, `u1 = u0 + width`, `v0 = sketch.y()`, `v1 = v0 + height`, then `to_world`). Python `compile_ocp.py:433-440` (same `u0/v0 → u1/v1`). FreeCAD `design.py:374-386` (points `(x,y)`, `(x+w,y)`, `(x+w,y+h)`, `(x,y+h)`) and `compile.py:191-204` (same polyline). |
| A circle's `x/y` is its centre. | Rust `compile.rs:511` (`centre = to_world(plane, x, y)` → `circle_face(centre, …)`). Python `compile_ocp.py:456` (same). FreeCAD `design.py:399-405` (`part.Circle(center=(x,y), normal, radius)`). |
| The local `(x,y)` map on XZ and YZ is the measured one. | `docs/construction-semantics.md:36-38` (`XY (u,v,0)`, `XZ (u,0,v)`, `YZ (0,u,v)`) and the measured offset-rectangle bounds at lines 57-75; both compilers' `to_world` are identical. |
| A pocket cuts only the component's own current solid; a cut that finds no material removes nothing and is reported. | Rust `compile.rs:373-391` (`cut(shape, …)`, `removed = before - after > …`, warning at 301-308). Python `compile_ocp.py:331-350`. FreeCAD `design.py:551-580` (before/after feature volume, warning, never raises). `docs/construction-semantics.md:103-110` says a no-op cut is a failure of the intent that must be reported, not claimed as removal. |
| One component has at most 12 steps. | `backend/crates/kala-contracts/src/construction.rs:24` `pub const MAX_STEPS: usize = 12;` enforced at line 967; Python `src/kala_agent/contracts/construction.py:28` and `Field(..., max_length=MAX_STEPS)` at line 317. |
| A part that needs more than 12 steps can be split, placed and fused. | `compile.rs` `assemble()` honours `place`/`fuse`; `MAX_COMPONENTS = 8` (`kala-contracts/src/construction.rs:22`); the contract has no rule against the split. The MiniMax repro itself exercises a two-half fuse. |

### One wording nuance, kept

The brief says "the build is rejected". The code **does not** reject the HTTP
build: the design route answers **200** with `verified: false`, appends the
failed `cuts_effective` check, and the chat turn spends its one repair round
saying "The first attempt was rejected". The sentence was kept because that is
exactly what the model sees (a rejected attempt), and changing it to "not
verified" would weaken the model-facing rule. This is a deliberate, reported
nuance, not an unverified claim. Evidence: live response below, plus
`kala-studio/src/native_geometry/service.rs::build_provider_checks`
(`cuts_effective` row) and `chat_design.rs::build_feedback`.

### The drift guard

- Rust: `backend/crates/kala-design-intent/tests/design_contract.rs:734`
  `the_instruction_states_the_contract_step_limit` scans the instruction for
  `A component has at most <n> steps` and asserts the single stated value is
  `kala_contracts::MAX_STEPS`, then asserts the exact split sentence.
- Python: `tests/unit/studio_design/test_intent.py`
  `test_the_instruction_states_the_contract_step_limit` does the same against
  `kala_agent.contracts.construction.MAX_STEPS`.
- `chat_design.rs`'s `the_instruction_matches_the_contract_crate` plus the live
  Python byte differential keep the three copies from drifting.
- New assertions that the model prompt really carries the paragraph:
  `chat_design.rs` unit test, `chat_mock_provider_design.rs` (outbound request
  body), `design_contract.rs`, and the Python `test_intent.py`.

## 2. Per-pocket ineffective-cut warnings

The warning is now:

```
component '<name>': pocket at step index <i> on <PLANE> removed no material
```

`<i>` is the 0-based index inside that component's `steps`, the same index the
cut-guard plane message uses.

| Layer | Change |
|---|---|
| Rust OCCT (`kala-cad/src/native/compile.rs:259,304`) | `for (index, step) in component.steps().iter().enumerate()` and `pocket at step index {index}`. |
| Python OCCT (`src/kala_agent/studio/design/compile_ocp.py:245,274`) | `for index, step in enumerate(component.steps)` and the same message; `{component.name!r}` renders single quotes for every contract-valid name (`^[A-Za-z][A-Za-z0-9_-]{0,39}$`), so Rust and Python are byte-identical. |
| FreeCAD lowering (`adapters/freecad/src/kala_agent_freecad/cad/compile.py:149,212-218`) | The component loop passes the true step index to `_solid`; `partdesign.pocket` carries `step_index=<i>`. |
| FreeCAD session (`adapters/freecad/src/kala_agent_freecad/cad/design.py:575`) | `pocket()` reads `step_index` and formats `pocket at step index <i> on <PLANE>`; a legacy `calls_json` pocket with no step index keeps the older plane-only wording instead of inventing an index. |

### What collapsed the duplicates

Today's loss was not a `dedup()` in the compilers: each compiler already pushed
one warning per pocket. The collapse was the **exact-string de-duplication** in
`bounded_warnings` / `_bounded_warnings`
(`kala-studio/src/native_geometry/service.rs:2049`,
`src/kala_agent/studio/design/service.py:1431`): the MiniMax repro's three
per-half holes produced three identical strings per component, so 6 warnings
became 2. With the step index in the message no two ineffective-pocket
warnings can be equal, so all 6 survive. The de-duplication was kept because it
is the generic guard for genuinely identical warnings from other sources;
removing it outright would change unrelated warning behaviour. The 16-warning
bounds (`MAX_WARNINGS`) are unchanged.

`cuts_effective` needed no change: `build_provider_checks` counts warnings
containing `removed no material` and computes `expected − ineffective`. The
live repro now measures **expected 10, measured 4**, failed, `verified: false`.

Tests updated/added:
- `backend/crates/kala-cad/tests/native_compile.rs`: exact one-warning assert;
  new `the_flange_repro_warns_once_for_every_ineffective_pocket` (6 warnings,
  exact indices, volume 85 765.479443).
- `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs`: exact
  off-plate index; new `freecad_warns_once_for_every_ineffective_flange_pocket`.
- `backend/crates/kala-studio/tests/native_geometry.rs`: exact off-plate index;
  new live OpenCascade and FreeCAD flange-repro tests asserting all 6 warnings,
  `cuts_effective` 10/4, `verified: false`, volume 85 765.479443.
- `backend/crates/kala-studio/src/chat_design.rs`, `tests/chat_mock_provider_design.rs`:
  the repair prompt now carries the step-index warning.
- `backend/crates/kala-studio/src/native_geometry/service.rs` (test input only):
  the two synthetic warnings use the new shape.
- `adapters/freecad/tests/test_design.py`: index case + no-index fallback.
- `adapters/freecad/tests/test_compile.py`: the lowering carries `step_index: 3`.
- `tests/unit/studio_design/test_compile_ocp.py`: exact Python warning tuple.
- `tests/unit/studio_design/test_solid_count.py`: current warning shape.

## 3. Loose ends

### One envelope-code mapping

`ProviderFailure::envelope_code()` now lives in
`backend/crates/kala-studio/src/providers/mod.rs:109-119`:

```rust
Self::Unavailable(_)            => "provider_unavailable",
Self::Configuration { code, .. } => code,
Self::Timeout(_)                => "provider_timeout",
Self::Network(_)                => "provider_network_error",
Self::Protocol(_)               => "provider_protocol_error",
Self::Subprocess(_)             => "provider_subprocess_error",
Self::Internal(_)               => "internal_error",
```

`server.rs:1540` (`let code = failure.envelope_code();`) and
`chat_design.rs:1333` (`the repair attempt failed: {failure.envelope_code()}`)
use it. `provider_failure()` in `server.rs` also derives its envelope code from
the method, so the strings exist in exactly one Rust match. The local
`server::provider_failure_code` and `chat_design::provider_envelope_code`
copies were deleted. New test
`every_provider_failure_names_its_envelope_code` covers all eight arms,
including `Configuration` with a custom code, `missing_credential` and
`Internal`; `a_failed_repair_call_names_the_envelope_code` stays.

### `cargo fmt --all`

Ran in `backend/`. This formatted the three cut-guard files
(`kala-cad-providers/tests/freecad_cut_guard.rs`,
`kala-design-intent/src/script.rs`, `kala-design-intent/tests/design_contract.rs`)
and our edits. `cargo fmt --all --check` is clean.

### `geometry_exports.rs`

`kala-studio/tests/geometry_exports.rs:18` now gates the import:

```rust
#[cfg(feature = "native-step")]
use kala_studio::integrations::build_registry;
```

`build_registry()` is only used by the `#[cfg(feature = "native-step")]` handoff
test, so `cargo clippy -p kala-studio --all-targets -- -D warnings` (no
`--all-features`) is clean, and `--all-features` still resolves it.

## 4. Corpus and fixtures

The only fixture that embeds the changed product source is the compatibility
corpus, because `manifest.json` pins the SHA-256 of every product Python file
(98 now, up from 95 with the parity cell's new provider modules). No fixture
contains the instruction text itself. Regenerated with the documented command:

```
.venv/bin/python backend/compat/generate_corpus.py
```

| | corpus SHA-256 |
|---|---|
| before this cell (`backend/fixtures/corpus/v1/manifest.json`) | `53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28` |
| after this cell | `61328f9143238ca85eeabc99d8ccd9c983ead0f1481ee491573e047ab52367ee` |

The checked-in manifest bytes also moved (source pin `92bb7e4`; 98 Python
files; the provider-list rows the concurrent `python-parity` cell landed). The
digested-file set itself did not change on the final regeneration (the manifest
is not one of the twelve digested payload files), which is why the digest stayed
`61328f91…` after the last source-pin-only refresh. Commands and results:

- `.venv/bin/python backend/compat/generate_corpus.py --check` → **exit 0,
  "determinism check passed"**
- `.venv/bin/python backend/compat/self_check.py` → **exit 0, "self-check OK"**,
  56 Studio cases / 15 routes, 49 core cases / 17 routes
- no hand-edited digest: `corpus_fixture.rs`'s `CORPUS_SHA256` and the two
  README digest mentions were updated to the regenerated value, and the
  fixture files themselves came from the generator.

## 5. Gates

Run in `backend/` unless noted, with the temp USER DATA environment.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | **pass** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **pass** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **pass** |
| `cargo clippy` in `kala-cad`, `kala-cad-providers`, `kala-design-intent` (`--all-targets --all-features`) | **pass** each |
| `cargo test` — `kala-design-intent` | **61 passed, 0 failed** (8 lib + 47 `design_contract` + 6 live Python differential) |
| `cargo test` — `kala-cad` | **119 passed, 0 failed** |
| `cargo test` — `kala-cad-providers` | **44 passed, 0 failed** (4 live `freecad_cut_guard`, all ran on FreeCAD 1.1.4) |
| `cargo test -p kala-studio` | **247 passed, 0 failed, 0 ignored** (136 lib + 111 across 17 integration binaries) |
| `pytest tests/unit -q` | **1619 passed, 18 warnings** |
| `pytest adapters/freecad/tests -q` | **391 passed** |
| `cargo build --release -p kala-studio` | **pass**, `backend/target/release/kala-studio`, 11 572 000 bytes, 2026-10-20T19:06:11 |
| `generate_corpus.py --check` / `self_check.py` | **pass / OK** |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures** |

Count deltas: `kala-studio` 244 → 247 (+2 flange live tests, +1
`envelope_code` unit test); `kala-cad` +1 flange test (24 in
`native_compile`); `kala-cad-providers` 43 → 44 (+1 flange live test);
`kala-design-intent` +1 step-limit test (61); Python unit suite 1580 → 1619
(our +1, the rest from the concurrent cells).

## 6. Differentials

| Differential | Before (release binary pre-rebuild) | After (rebuilt) | Mismatch ids |
|---|---|---|---|
| `python_differential.py` | 268 cases / **0** | 268 cases / **0** | none (empty set) |
| `worker_differential.py` | 292 cases / **0** | 292 cases / **0** | none (empty set) |
| `python_provider_dto_oracle.py` | 20 cases / **0 failures** | 20 cases / **0 failures** | none |

The wave's documented baseline was 47 / 57 / 0, all provider-list drift. During
this run the concurrent `python-parity` cell landed its 7-row provider list and
current Codex wording, so the counts **dropped to 0 before this cell's rebuild**
and stayed at 0 after. The before/after mismatch id sets are both **empty**;
nothing in `design.*`, `convert.*`, `import.*` or `journal.*` mismatched. The
corpus replay (`corpus_fixture.rs`, 56 cases) also passes against the
regenerated corpus, including the provider bodies the parity cell changed.

## 7. Live evidence

### FreeCAD 1.1.4 and OpenCascade on the MiniMax repro

`FreeCADCmd` 1.1.4 on this host, driven through the real provider
(`cargo test -p kala-cad-providers --test freecad_cut_guard -- --nocapture`):

```
MiniMax flange repro: 85765.479443 mm3, warnings [
 "component 'flange_right': pocket at step index 7 on XY removed no material",
 "component 'flange_right': pocket at step index 9 on XY removed no material",
 "component 'flange_right': pocket at step index 11 on XY removed no material",
 "component 'flange_left': pocket at step index 7 on XY removed no material",
 "component 'flange_left': pocket at step index 9 on XY removed no material",
 "component 'flange_left': pocket at step index 11 on XY removed no material"]
```

The same script through the Python OCP compiler prints the **identical six
strings**. Through the rebuilt release binary (`POST /api/studio/design`,
ephemeral port, temp data dir; the 8831 server was untouched):

| provider | status | warnings | `verified` | volume mm³ | `cuts_effective` |
|---|---|---|---|---|---|
| `opencascade` | 200 | 6, identical | **false** | 85 765.479443 | expected 10, measured 4, failed |
| `freecad` | 200 | 6, identical | **false** | 85 765.479443 | expected 10, measured 4, failed |

The normal four-hole plate stays **24 695.221316 mm³**, `verified: true`, with
**no warnings** and **no `cuts_effective` row**, on both providers (live
`native_geometry` regression and the provider test). The off-plate single hole
still reports one warning, now
`component 'plate': pocket at step index 3 on XY removed no material`.

### Instruction and prompt

- The three copies are byte-identical at 3 549 bytes (direct comparison and
  `live_python_schema_is_byte_identical`).
- `chat_mock_provider_design.rs` asserts the outbound OpenRouter prompt contains
  `a rectangle's x/y is its lower-left corner`,
  `Every pocket must cut into material…` and `A component has at most 12 steps`;
  the build-repair prompt starts with the same instruction.

## 8. Per-file summary (this cell's hunks)

| File | Change |
|---|---|
| `backend/crates/kala-design-intent/src/intent.rs` | instruction paragraph. |
| `backend/crates/kala-studio/src/chat_design.rs` | instruction paragraph; `failure.envelope_code()`; local mapping removed; instruction assertions. |
| `src/kala_agent/studio/design/intent.py` | instruction paragraph (byte-identical). |
| `backend/crates/kala-design-intent/tests/design_contract.rs` | step-limit drift test; coordinate/pocket assertions. |
| `tests/unit/studio_design/test_intent.py` | step-limit drift test; coordinate/pocket assertions. |
| `backend/crates/kala-cad/src/native/compile.rs` | enumerate step index; per-pocket warning. |
| `src/kala_agent/studio/design/compile_ocp.py` | enumerate step index; per-pocket warning. |
| `adapters/freecad/src/kala_agent_freecad/cad/compile.py` | carries `step_index` on `partdesign.pocket`. |
| `adapters/freecad/src/kala_agent_freecad/cad/design.py` | warning names the step index; plane-only fallback for legacy calls. |
| `backend/crates/kala-studio/src/providers/mod.rs` | `ProviderFailure::envelope_code()` + unit test. |
| `backend/crates/kala-studio/src/server.rs` | uses `envelope_code()`; duplicate mapping deleted. Only this hunk is ours. |
| `backend/crates/kala-studio/tests/geometry_exports.rs` | `#[cfg(feature = "native-step")]` import gate. |
| `backend/crates/kala-studio/tests/corpus_fixture.rs` | new `CORPUS_SHA256`. |
| `backend/crates/kala-studio/tests/native_geometry.rs` | exact off-plate warning; two live flange tests. |
| `backend/crates/kala-studio/tests/chat_mock_provider_design.rs` | new warning text; instruction assertions in the outbound prompt. |
| `backend/crates/kala-studio/src/native_geometry/service.rs` | test-input warning strings only. |
| `backend/crates/kala-cad/tests/native_compile.rs` | exact warning; live OCP flange test. |
| `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs` | exact warning; live FreeCAD flange test. |
| `adapters/freecad/tests/test_design.py` | index + fallback tests. |
| `adapters/freecad/tests/test_compile.py` | lowering carries the index. |
| `tests/unit/studio_design/test_compile_ocp.py` | exact Python warning. |
| `tests/unit/studio_design/test_solid_count.py` | current warning shape. |
| `backend/fixtures/corpus/v1/{manifest.json,studio_http.json}` | regenerated by the documented command. |
| `backend/fixtures/README.md`, `backend/compat/README.md` | new corpus digest recorded. |
| formatted files | `kala-cad-providers/tests/freecad_cut_guard.rs`, `kala-design-intent/src/script.rs`, `kala-design-intent/tests/design_contract.rs` via `cargo fmt --all`. |

## 9. Gaps / honest notes

1. **"the build is rejected" is the model-facing wording** for a 200 response
   with `verified: false`; the route does not return 4xx. Documented in §1.
2. **Generated packaging copies are stale.** `grep` finds the old instruction
   in `studio/release/linux-unpacked/...` and `studio/backend-native/...`.
   Both are gitignored build stages, not tracked source; they are refreshed by
   `studio/scripts/native-backend.cjs` when the desktop package is staged. They
   were not edited by hand.
3. **The corpus source pin is a moving target while `python-parity` works.**
   The final regeneration was made after the parity cell's last provider edit
   (all 98 pinned files matched at report time) and `generate_corpus.py --check`
   + `self_check.py` both passed. If that cell edits a product Python file
   again, regenerate and update `CORPUS_SHA256`; the digest may not move for a
   source-pin-only change because `manifest.json` is not one of the digested
   payload files.
4. **Warning-count bound (pre-existing):** the FreeCAD adapter caps session
   warnings at 16 (`MAX_WARNINGS`), so a design with more than 16 ineffective
   pockets under-reports the `cuts_effective` `measured` value. No test or
   corpus design approaches it.
5. **`ProviderFailure::envelope_code` is now the only Rust mapping.** The
   Python reference keeps its own `_provider_error_code` (a different language;
   byte parity is covered by the differentials, not by sharing code).
6. **No live model call was made by this worker.** The prompt changes are
   proven by the outbound-request assertions and the byte-identical contract;
   the conductor should rerun the MiniMax flange prompt against the rebuilt
   binary to confirm the model now states the coordinate/step rules.
7. **Windows** was not built or tested in this cell.
8. **No commit** was made; the working tree also contains the concurrent cells'
   changes.
