# Cell cut-guard — report

**Status:** implemented (Changes A, B, C), all owned gates green except the workspace-wide
clippy run, which is blocked by a pre-existing warning in a file this cell does not own
(see Gaps). No commit, push, or deployment. The live Studio backend on port 8831 was not
stopped or addressed; every shell and every test run used a fresh temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`, and every explicit
`--data-dir` pointed into a temporary directory. Nothing under
`/home/v9i/.local/share/kala-agent/**` was read or written, and no credential or settings
file was read.

## Change A — same-plane pocket validation (intent layer)

The rule is enforced where the chat/API flow validates a `DesignIntent.script`:

* Rust: `backend/crates/kala-design-intent/src/script.rs` — after the existing component,
  sequence, uniqueness and assembly-reference checks, `pocket_plane_error` walks each
  component and returns the first pocket whose **pending sketch plane differs from the
  plane of the sketch that produced the current solid**. The issue is attached to the
  `script` field, exactly where the Python `AfterValidator` below attaches it.
* Python: `src/kala_agent/studio/design/intent.py` — `_check_pocket_planes`, wired through
  `Annotated[ConstructionScript, AfterValidator(...)]` on `DesignIntent.script`. The JSON
  Schema is unchanged (field validators do not alter `model_json_schema`), and
  `load_construction` / the OCP compiler still accept a cross-plane script so the
  compiler's own refusal branch remains covered by its existing test.

This is exactly the OCP compiler rule
(`compile_ocp._build_component` / `kala-cad/src/native/compile.rs`): a cut follows the
pending sketch's plane, and that plane must equal `last_solid_plane`.

**New messages (byte-identical in Rust and Python):**

```
component '<name>': pocket at step index <i> on <SKETCH_PLANE> over a <SOLID_PLANE> solid
```

The live repro now returns:

```
script: component 'half_a': pocket at step index 3 on XZ over a XY solid
```

and over HTTP (`POST /api/studio/design`, provider `freecad`) the response is
`422 invalid_request` with
`body.script: value_error: Value error, component 'half_a': pocket at step index 3 on XZ over a XY solid`.

### Rule (a): STOP finding — literal rule (a) rejects the shipped template

The brief asked for a second rejection when the pocket's own `plane` differs from the
pending sketch's plane. **That rule cannot be applied as literally stated.** The shipped
`l_bracket` template builds its upright as `sketch XZ → extrude → sketch XZ → pocket`, and
constructs the pocket as `ComponentStep(op="pocket")`; the contract default makes its
effective plane `XY`. Its canonical wire form — in both Python and Rust — therefore always
contains `{"op":"pocket","plane":"XY"}` over an `XZ` sketch. Literal rule (a) would reject:

* the template at the moment it is constructed in Python, and
* the template's own canonical script when re-validated as a `DesignIntent` (proven by the
  Rust test `template_scripts_still_validate_as_design_intents` before narrowing).

The brief's stop condition therefore applies. This cell did **not** change the rule to work
around it by editing the template or the construction contract; it implemented only the
OCCT rule (b), which rejects the live repro and is exactly what the compiler applies. The
pocket's own `plane` field is ignored by the compiler, so accepting a false value there is
harmless; a future wave can reject it only after the template emits `plane: XZ` (which
would change template digests). This is the main deviation from the brief and needs the
conductor's decision if rule (a) is still wanted.

## Change B — FreeCAD detects a cut that removed nothing

`adapters/freecad/src/kala_agent_freecad/cad/design.py`:

* `_Session` tracks the last `sketch.new` plane (`_sketch_plane`).
* `pocket()` measures the feature tip's volume before the pocket and the pocket feature's
  volume after the recompute. When `before - after <= max(before · 1e-9, 1e-6)` mm³ it
  appends `component '<name>': pocket on <PLANE> removed no material` through the existing
  bounded `_warn`, and never raises. The component name is `partdesign.body`'s name, or
  the body label/name as a fallback.
* `_feature_volume_mm3` is best-effort: an unmeasurable shape skips the check instead of
  failing the build.

**Tolerance choice:** `max(1e-9 · before, 1e-6)` mm³, compared with `<=`. This is
deliberately more conservative than the OCCT compiler's floor (`max(1e-9, before · 1e-12)`)
to absorb FreeCAD's recompute noise; a cut below `1e-6` mm³ is not representable at the
six-decimal volume Studio reports. Written down here as required.

Warning reachability was verified end to end (adapter → `driver.py` `result["warnings"]` →
`freecad.rs` `ProviderBuild.warnings` → `kala-studio` response): see the live evidence
below. The adapter test suite stays green.

## Change C — an ineffective cut fails verification (every provider)

`build_provider_checks` in both mirrors gains `expected_pockets` and `warnings`. When at
least one warning contains `removed no material`, it appends exactly one row:

| field | value |
|---|---|
| `id` | `cuts_effective` |
| `label` | `Cuts removed material` |
| `expected` | number of `pocket` steps in the script |
| `measured` | `expected − ineffective-cut warnings`, clamped at 0 |
| `tolerance` | `0.0` |
| `unit` | `cut` |
| `passed` | `false` (so `status` is `failed` and `verified` is `false`) |

The row is appended **only** for that failure; every passing design keeps its exact check
list and digest. Both Rust call sites (provider build and journal cold reload) and both
Python call sites were updated. `expected_pocket_count` is new in both mirrors.

## Per-file changes (only this cell's hunks)

No file from the concurrent worker's ownership list
(`kala-studio/src/{server.rs,chat_design.rs,providers/**}`,
`src/kala_agent/studio/{server.py,providers/**}`) was edited. No commit was made.

| file | change |
|---|---|
| `backend/crates/kala-design-intent/src/script.rs` | `StepData` carries the effective plane; `pocket_plane_error` + call in `validate_script`; `ComponentData` carries steps. |
| `backend/crates/kala-design-intent/tests/design_contract.rs` | repro refused test, cross-plane sketch-over-solid test, pocket-plane-field-is-ignored test, template canonical script still validates. |
| `backend/crates/kala-design-intent/tests/python_differential.rs` | 5 new intent payloads: repro, cross-plane, omitted plane, ignored pocket plane, L-bracket canonical pocket. |
| `src/kala_agent/studio/design/intent.py` | `ComponentStep` import, `_check_pocket_planes`, `Script` alias, `script` field type. The rest of this file's diff is the pre-existing `llm-design` cell work; only the hunks at lines 31, 210–254 are this cell's. |
| `adapters/freecad/src/kala_agent_freecad/cad/design.py` | `_sketch_plane`, before/after volume in `pocket()`, `_feature_volume_mm3`. |
| `adapters/freecad/tests/test_design.py` | no-op pocket warns; material-removing pocket does not. |
| `backend/crates/kala-studio/src/native_geometry/service.rs` | `CUTS_EFFECTIVE_TOLERANCE`, `CUT_NO_MATERIAL`, `expected_pocket_count`, the `cuts_effective` row, both call sites, three unit tests. |
| `backend/crates/kala-studio/tests/native_geometry.rs` | live OpenCascade and FreeCAD off-plate-hole tests; four-hole regression (volume, `verified: true`, no new row). |
| `src/kala_agent/studio/design/service.py` | `CUTS_EFFECTIVE_TOLERANCE`, `CUT_NO_MATERIAL`, `expected_pocket_count`, `cuts_effective` row, both call sites. |
| `tests/unit/studio_design/test_intent.py` | three validation tests with byte-exact messages. |
| `tests/unit/studio_design/test_solid_count.py` | passing list unchanged; ineffective cut adds the row; unrelated warning adds nothing. |
| **new** `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs` | live FreeCADCmd tests for the no-op warning, the four-hole volume, and provider availability from an explicit env. |

## Gates

| gate | result |
|---|---|
| `cargo fmt --check` (workspace) | **pass** |
| `cargo clippy -p kala-design-intent -p kala-cad-providers --all-targets -- -D warnings` | **pass** |
| `cargo clippy -p kala-studio --lib --bins -- -D warnings` | **pass** |
| `cargo clippy -p kala-studio --test native_geometry -- -D warnings` | **pass** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **fails on a file this cell does not own**: `crates/kala-studio/tests/geometry_exports.rs:18 unused import kala_studio::integrations::build_registry` (pre-existing; it was already reported by `cargo check --all-targets` before this cell's first edit). `chat_design.rs`'s earlier `too_many_arguments` error was fixed concurrently by its owner. |
| `cargo test -p kala-design-intent` | **60 passed, 0 failed** (lib 8, `design_contract` 46, `python_differential` 6) |
| `cargo test -p kala-cad-providers` | **43 passed, 0 failed** (`freecad_cut_guard` 3, `freecad_differential` 1, `freecad_unit` 23, `solidedge_differential` 3, `solidedge_mock_bridge` 13) |
| `cargo test -p kala-studio` | **243 passed, 0 failed** (lib 135; `native_geometry` 12; `chat_mock_provider_design` 13; the rest as before) |
| `pytest tests/unit -q` | **1579 passed, 0 failed, 18 warnings** |
| `pytest adapters/freecad/tests -q` | **390 passed, 0 failed** (the live `freecad`-marked tests ran, not skipped) |
| `cargo build --release -p kala-studio` | **pass** (rebuilt after the final edit; `backend/target/release/kala-studio`) |
| `ruff check` / `ruff format --check` on changed Python files | **pass** |

## Differential counts

| differential | cases | mismatches now | baseline before this cell |
|---|---|---|---|
| `backend/crates/kala-studio/tests/python_differential.py` | 268 | **47** | 268 / 0 (r2-datadir-fix, corpus-sync reports) |
| `backend/crates/kala-studio/tests/worker_differential.py` | 292 | **57** | 292 / 0 (r2-datadir-fix, r2-chat-providers, corpus-sync reports) |
| `kala-design-intent` live Python differential | 6 tests | **0** | 0 |

**Every one of the 47/57 mismatches is provider-catalog drift from the concurrent
`cli-providers` / `model-catalog` work, not a design change.** Breakdown:

* `python_differential.py`: settings 33, origin 5, read 3, providers 2, host 2,
  `chat.codex_unavailable` 1, health 1 = 47.
* `worker_differential.py`: settings 43, origin 5, read 3, providers 2, host 2,
  `chat.codex_unavailable` 1, health 1 = 57.

The native binary lists `claude_code`, `codex_cli` and `opencode_cli`; the live Python
reference still lists only `local`, `openai`, `openrouter`, `codex`, so settings and health
bodies differ in every case that echoes the provider list. **Zero mismatches are in
`design.*`, `convert.*`, `import.*`, `journal.*`, geometry or export cases; notably
`design.chat_with_freecad`, `design.chat_with_opencascade` and `design.with_provider` all
matched**, which is the design/check parity this cell protects. The design DTO row shape
was also checked directly: Python `check_payload` and the Rust response produce the same
keys/values for `cuts_effective` (`expected 1.0`, `measured 0.0`, `tolerance 0.0`,
`unit "cut"`, `passed false`, `status "failed"`).

## Live FreeCAD evidence (FreeCADCmd 1.1.4, this host)

`backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs --nocapture`:

```
off-plate build: 25600 mm3, warnings ["component 'plate': pocket on XY removed no material"]
four-hole plate build: 24695.221316 mm3, warnings []
```

The off-plate script is an 80 × 40 × 8 plate (`25600 mm³` if nothing is cut) with one Ø6
circle sketched at `(100, 20)` on `XY`, entirely outside the body. The four-hole plate is
the normal 80 × 40 × 8 plate with 4 × Ø6 through holes.

End-to-end through the release Studio binary (`POST /api/studio/design`, temp data dir,
free port; the 8831 backend was left running):

| provider | script | status | volume mm³ | `verified` | warnings | `cuts_effective` |
|---|---|---|---|---|---|---|
| freecad | off-plate | 200 | 25600.0 | **false** | `component 'plate': pocket on XY removed no material` | expected 1.0, measured 0.0, failed |
| freecad | four-hole | 200 | 24695.221316 | **true** | none | **absent** |
| opencascade | off-plate | 200 | 25599.999999999993 | **false** | same text | expected 1.0, measured 0.0, failed |
| opencascade | four-hole | 200 | 24695.22131576613 | **true** | none | **absent** |

The repro `repro-codex-flange-intent.json` posted with `provider: freecad` now returns
`422` and the message above; no FreeCAD build is attempted.

## Gaps / honest notes

1. **Rule (a) is not implemented** — see the STOP finding above. Rule (b) is exact OCCT
   parity and rejects the repro. If rule (a) is required, the template must first emit
   `plane: XZ` on its upright pocket (changing template wire bytes/design ids) or the
   contract must distinguish an omitted plane from an explicit `XY`.
2. **Warning-count bound.** `measured` is `expected_pockets − ineffective warnings`. The
   FreeCAD adapter caps warnings at 16 (`MAX_WARNINGS`); a design with more than 16
   ineffective cuts under-reports the ineffective count, so `measured` would be optimistic
   in that extreme. The row would still be present and `verified` false. No corpus or test
   design approaches that bound.
3. **Contract vs intent.** The rule lives in the intent layer (Rust
   `kala-design-intent`, Python `DesignIntent`), matching the brief's named files and
   keeping `kala-contracts` / `ConstructionScript` and the compiler's own cross-plane
   refusal untouched. A direct typed-script load (`load_construction`) still accepts a
   cross-plane script and relies on the compiler's refusal; that is deliberate and keeps
   the existing compiler test meaningful.
4. **Workspace clippy** is blocked by the unrelated pre-existing unused import in
   `crates/kala-studio/tests/geometry_exports.rs`. This cell's crates and touched files are
   clippy-clean.
5. Differential baseline is taken from the previous wave's reports (268/0 and 292/0);
   the current 47/57 are attributable to the concurrent provider work as itemised above.
   This cell changed no provider list, settings document or health payload.
