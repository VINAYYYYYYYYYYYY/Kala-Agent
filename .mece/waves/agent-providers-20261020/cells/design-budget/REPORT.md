# Cell design-budget — one generous whole-design budget

Date: 2026-10-21
Cell: `design-budget` (agent-providers wave)

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and `CARGO_HOME=/home/v9i/.cargo`
first, then set a fresh temporary `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` (`/tmp/opencode/kala_env.sh`,
sourced in every command). No credential file and no settings file was read;
no credential was printed. Nothing under `/home/v9i/.local/share/kala-agent/**`
was opened, named in a command or written. Every server and every FreeCADCmd
ran with a temporary HOME and workspace. No real `claude`, `codex` or
`opencode` CLI was run. No commit, push or sudo. All temporary HOME and
extraction directories were removed at the end; `df -h /tmp` was checked.

## What changed

### 1. Contract budget (Rust `kala-contracts` + Python `kala_agent.contracts`)

| Constant | Before | After |
|---|---|---|
| `MAX_COMPONENTS` | 8 | **64** |
| `MAX_STEPS` (per component) | 12 | **500** |
| `MAX_TOTAL_STEPS` (new) | — | **500** |
| `MAX_ASSEMBLY_STEPS` | 16 | 16 (unchanged) |

`MAX_TOTAL_STEPS` counts **the steps of every component plus the assembly
steps**. 500 is accepted; 501 is refused. The message is byte-identical in
Rust and Python:

```
a script may use at most 500 steps in total; this one uses 501
```

It is attached at the whole-script root (`script: Value error, …` when nested
in a `DesignIntent`), after the component-name/assembly-reference checks and
before the pocket-plane rule, in both languages. The per-field messages keep
their old vocabulary: `Tuple should have at most 64 items after validation, not
65` and `Tuple should have at most 500 items after validation, not 501`.

The design-intent validator (`kala-design-intent/src/script.rs` and the Python
`ConstructionScript` validator) reproduces the same rule and location, so the
`DesignIntent` error text is exact. The JSON Schema asset was regenerated from
the live Pydantic model (`ComponentScript.steps.maxItems` 12 → 500,
`ConstructionScript.components.maxItems` 8 → 64; 11 291 → 11 293 bytes).

Every other place that encoded 8/12 or derived from the constants:

| Place | Change |
|---|---|
| `adapters/freecad/.../cad/models.py` tool prompt | "components has 1 to 64 objects … steps has 1 to 500 objects, and a whole script may use at most 500 steps in total …" |
| `adapters/solidedge-bridge/.../protocol.py:470` doc | now says 500 per component / 500 total and `MAX_PARTS` |
| `adapters/solidedge/tests/test_compile.py` | the "maximum core IR fits the wire" test became "the largest wire-legal script fits" plus a parts refusal; the 8×12 case is kept as the wire-legal widest case |
| `src/kala_agent/studio/design/templates.py` `MAX_SEGMENTS = MAX_COMPONENTS` | derives 64; a 64-segment stepped shaft is 130 steps, inside the total budget. The intent template call still caps at 8 segments (`intent.MAX_SEGMENTS`), deliberately unchanged |
| `backend/crates/kala-design-intent/src/templates.rs` `MAX_SEGMENTS = MAX_COMPONENTS` | same |
| `docs/adr/0009-…` | budget line names the new constants |

`studio/release/**`, `studio/backend-native/**` and `Rust+buid123/**` still
contain old copies; they are generated packaging stages / a stale extraction,
gitignored and not source (regenerated at packaging time), not edited.

### 2. Solid Edge stays honest

The bridge wire caps are unchanged: `MAX_OPERATIONS = 600`,
`MAX_PARTS = 8`. Both the Rust client (`kala-solidedge-client/src/compile.rs`)
and the Python reference (`adapters/solidedge/.../compile.py`) now refuse a
script that compiles to more than `MAX_PARTS` parts with a clear message, and
keep the existing operation-cap refusal:

```
the script has 9 components; the wire cap is 8 parts
the compiled procedure has 601 operations; the wire cap is 600
```

Tests: `a_script_over_the_wire_part_cap_is_refused` (Rust) and
`test_more_parts_than_the_wire_cap_is_refused` (Python), plus the positive
`a_whole_budget_script_lowers_under_the_wire_operation_cap` in both.

**Lowering ratio (measured).** One wire operation per consumed sketch/solid
pair (one per script step), four per part (`part.new`, `part.measure`,
`part.save`, `part.export_step`), and one assembly operation per part plus
three (`assembly.new`, `assembly.save`, `assembly.export_step`).
- one 500-step body: **504 ops / 500 steps = 1.008**;
- the widest wire-legal case with the whole budget, 8 parts × 62 steps
  (496 steps): **539 ops / 496 steps = 1.087**;
- the 8-part × 12-step wire-legal script: 235 ops / 96 steps = 2.45.

A 500-step FreeCAD script lowers differently: 2,259 calls for a 249-pocket
plate and 4,501 calls for a rectangle-pair worst case (**4.5–9.0 FreeCAD calls
per step**).

**The 600-op cap cannot bind from a contract-valid script.** The maximum
number of operations for a script inside the 500-step budget and at most 8
parts is 500 + 4·8 + (8 + 3) = **543**. More than 8 parts always trips the parts
cap first. The operation check is kept as defence in depth (and remains
reachable only through a hand-built request); the parts cap is the real Solid
Edge limit. This is a measured correction to the brief's premise.

### 3. Hidden reply cap removed

`MAX_RESPONSE_TEXT_LENGTH = 4 096` was applied in `providers/remote.rs:508`
**before** `remote_design_turn` extracted the design, so any larger script was
cut mid-JSON.

- Rust `ChatOutcome` now carries `text` (display, ≤ 4 096 characters, the
  parity bound) **and** `reply` (the full model reply, still bounded by the
  1 MB HTTP body / 1 MB CLI stdout). `ChatOutcome::new` sets both, so every
  provider keeps the display bound.
- `remote_design_turn` and `repair_build` now extract, validate and repair from
  `reply`; the displayed prose is `strip_design_json_block(reply)` bounded to
  4 096.
- Python `ChatOutcome` gained `reply` (defaults to `text`); `openai_compat`
  returns the full content as `reply` and `text[:4096]` as `text`; `dispatch`
  reads/repairs from `.reply` and displays `_display_text(reply)`
  (`strip_design_json_block(reply)[:4096]`).
- The CLI providers never truncated, and their `reply == text`; only the
  generic display bound now applies to `text`.
- Plain-chat behaviour is unchanged: `dispatch` returns `outcome.text` for a
  non-design chat, and the differential cases stayed byte-identical.

Evidence:
- Rust `a_design_longer_than_the_display_bound_still_builds` (mock OpenRouter
  transport, real OpenCascade): a 100-hole design JSON > 6 000 characters
  builds to `verified: true`, and the displayed text is ≤ 4 096 chars, contains
  the prose, and contains no ```json``` block.
- Python `test_a_reply_longer_than_the_display_bound_is_not_truncated_before_parsing`
  (a > 6 000-char reply split into `text`/`reply` like the HTTP provider) parses
  202 steps and displays ≤ 4 096 chars; `test_the_provider_reader_keeps_the_full_reply`.
- Rust/Python agreement: `the_long_reply_display_and_design_are_byte_identical_in_both_languages`
  asks the live Python reference for `_display_text` and the parsed step count
  and compares them byte-for-byte with Rust (`strip_design_json_block`,
  4 096-char bound, 142 steps).

### 4. Downstream bounds sized for the budget

**FreeCAD warning display / exact cut count.**
- The FreeCAD builder counts every ineffective cut (`_Session.ineffective_cuts`,
  exact) and its result also carries `ineffective_cuts` as a separate integer
  field (`DESIGN_OUTPUT_SCHEMA` updated).
- The warning display is bounded to `MAX_WARNINGS = 16`: the first **14**
  warnings verbatim, then at most two summary lines:
  `… and 486 more pockets removed no material` and `… and 5 more warnings`
  (singular nouns for 1). The exact ineffective count is the sum of the
  individual cut lines and the number the summary states, so
  `cuts_effective` is exact for 500 pockets independent of the display.
- The same rule lives in `kala-cad-providers/src/base.rs`
  (`bound_warnings`, `ineffective_cut_count`, idempotent re-bounding) and
  Python `providers/base.py`; `ProviderBuild` and the native service's
  `bounded_warnings` use it. Rust and Python name the fields and the summary
  text identically.
- Tests: 500 cut warnings → `… and 486 more pockets removed no material`, count
  500 (Rust and Python); 20 cuts + 5 others → two summaries, count 20; singular
  summary; the FreeCAD adapter display test; `build_provider_checks` returns
  `expected 500, measured 0`.

**Repair prompts.** Measured canonical JSON of a 500-step script:

| Case | Characters |
|---|---|
| realistic 249-hole plate (500 steps), canonical | 81 366 |
| worst-case values, 500 steps, canonical | 90 497 |
| same, pretty-printed (indent 2) | 171 546 |

`MAX_REPAIR_PREVIOUS_CHARS` and `MAX_BUILD_REPAIR_SCRIPT_CHARS` are now
**200 000** in Rust and Python (byte-identical), which holds all three whole
with margin. (The `MAX_BUILD_REPAIR_PROMPT_CHARS = 9 000` bound now describes
the prompt without the script and the user request; the script is bounded
separately.)

**FreeCAD build deadline — measured on FreeCAD 1.1.4 (real FreeCADCmd, with
the per-pocket volume check):**

| Case | Steps | Pockets | Wall time |
|---|---|---|---|
| 100-hole plate (200 × 200 × 5, 10×10 grid of Ø3) | 202 | 100 | **58.8 s** provider, **59.8 s** through Studio |
| same with 3 holes off the plate (97 effective) | 202 | 100 | ≈57 s |
| whole-budget plate (2 000 × 2 000 × 5, 249 Ø6 holes) | 500 | 249 | **1 009.5 s (16.8 min)** |

The 3-off-plate plate gives exactly three `removed no material` warnings and
volume 196 571.737017 mm³; the clean 100-hole plate gives 196 465.708265 mm³
and **zero** warnings; the 500-step plate gives 19 964 798.454316 mm³.
`DESIGN_TIMEOUT_SECONDS` is therefore **1 800 s**, which covers the measured
500-step case with 78 % margin when idle and 44 % under load. The FreeCAD
provider's default wall time is also 1 800 s (max 3 600 s) in Rust and Python, because the provider's 120 s
process timeout used to kill a legal large build before the adapter deadline
mattered. The OpenCascade compile budget went from 5 s to **60 s** (the
100-hole plate compiles in 2.5 s, the 500-step plate in 16.9 s).

**Mesh, drawing, section on the 100-hole plate (OpenCascade, measured):**
- view mesh **10 624 vertices / 10 812 triangles** — inside the 200 000 /
  400 000 limits, no refusal;
- HLR drawing **8 452 points**, `truncated: false` (20 000-point cap);
- XZ section at offset 100 (through a hole row): **11 outer loops, 0 inner
  loops, area 850.0 mm² exactly** (200×5 minus ten 3×5 slots).
The FreeCAD build of the same plate produces the same 10 624/10 812 view.

### 5. Instruction (byte-identical in three files, 3 602 characters)

Replaced the old sentence with (inside the existing paragraph):

```
A design may use up to 500 steps in total across up to 64 components, and one component may use all of them. Each solid body is one component: do not split a body into components just to save steps.
```

`kala-design-intent/src/intent.rs`, `kala-studio/src/chat_design.rs` and Python
`studio/design/intent.py` compare equal byte-for-byte (3 602 characters; the
existing cross-language byte tests pass). Tests now pin the numbers to the
constants: `the_instruction_states_the_whole_design_budget` (Rust) and
`test_the_instruction_states_the_whole_design_budget` (Python) require the
sentence `A design may use up to {MAX_TOTAL_STEPS} steps in total across up to
{MAX_COMPONENTS} components`, require the body sentence, and forbid the old
"A component has at most" text.

### 6. Corpus and fixtures

Regenerated with the documented commands
(`.venv/bin/python backend/compat/generate_corpus.py`, `--check`,
`self_check.py`).

| | Before | After |
|---|---|---|
| corpus SHA-256 | `61328f9143238ca85eeabc99d8ccd9c983ead0f1481ee491573e047ab52367ee` | `e86c7d93157fb844af0e4cf8711824df4e75a782ac9f88b94318d57f21df0f6b` |
| source pin `python_sources.tree_digest` | `dcd136adab653ebc2ce52aac6883e8387e9b90844a4b80c6940f80a69396133a` | `f777fb0f03723420e2fb37968563ab0ad307361ba1683f7a2ddd87b44391bb79` |
| pinned product Python files | 98 | 98 |
| design intent schema | 11 291 bytes | 11 293 bytes |

`canonical_json.json`'s `MAX_STRING_LENGTH` cases now move with the constant
(524 288 / 524 289); the generator imports the product constants instead of
hardcoding 32 768. `corpus_fixture.rs`'s `CORPUS_SHA256` and
`backend/compat/README.md` were updated.

## Other hidden bounds found and sized (all measured, not guessed)

| Bound | Before | After | Why |
|---|---|---|---|
| `tools.protocol.MAX_STRING_LENGTH` + `kala-json` default | 32 768 | **524 288** | the design tool passes its compiled call list as one JSON string: 130 000 chars for a 500-step pocket plate, 303 813 for a rectangle-pair worst case |
| `tools.protocol.MAX_ARGUMENT_BYTES` | 262 144 | **524 288** | the same call list plus its argument object; still below the 1 MB envelope |
| FreeCAD `_CALLS_JSON.maxLength` | 24 000 | **524 288** | the input schema rejected a 202-step plate before the adapter ran |
| FreeCAD `MAX_CALLS` | 2 000 | **8 000** | 500 steps = up to 4 501 calls |
| FreeCAD provider default wall time | 120 s | **1 800 s** | the process timeout killed large legal builds |
| OpenCascade compile budget | 5 s | **60 s** | 500-step compile 16.9 s |
| FreeCAD repair bounds | 6 000 | **200 000** | max canonical script 90 497 chars |

## Per-file changes (this cell's hunks only)

Shared files (`kala-studio/src/{server.rs,providers/mod.rs}`, Python
`providers/dispatch.py`, the corpus) also carry concurrent cells' changes; each
of my patches was re-read immediately before editing. No other cell's hunks
were reverted.

| File | Change |
|---|---|
| `backend/crates/kala-contracts/src/construction.rs` | constants 64/500, new `MAX_TOTAL_STEPS`, root total-step check |
| `backend/crates/kala-contracts/src/lib.rs` | export `MAX_TOTAL_STEPS` |
| `backend/crates/kala-contracts/tests/construction_contracts.rs` | 500 OK / 501 refused / 300-step component / 64 OK / 65 refused |
| `src/kala_agent/contracts/construction.py` | same constants and rule |
| `src/kala_agent/contracts/__init__.py` | export |
| `backend/crates/kala-design-intent/src/script.rs` | same total rule at the Pydantic location |
| `backend/crates/kala-design-intent/src/intent.rs` | new instruction sentence |
| `backend/crates/kala-design-intent/assets/design_intent_schema.json` | regenerated (64/500) |
| `backend/crates/kala-design-intent/tests/design_contract.rs` | budget test, 65-segment template refusal, schema length |
| `backend/crates/kala-design-intent/tests/python_differential.rs` | 501-step and 65-component live cases; template segments vs `MAX_COMPONENTS` |
| `backend/crates/kala-solidedge-client/src/compile.rs` | parts-cap refusal + two tests |
| `adapters/solidedge/src/kala_agent_solidedge/compile.py` | parts-cap refusal |
| `adapters/solidedge/tests/test_compile.py` | wire-legal widest case, parts refusal |
| `adapters/solidedge-bridge/.../protocol.py` | doc only (wire caps unchanged) |
| `adapters/freecad/.../cad/models.py` | tool prompt text |
| `adapters/freecad/.../cad/design.py` | exact `ineffective_cuts`, bounded display with summaries, separate result field |
| `adapters/freecad/.../cad/procedure.py` | `MAX_CALLS` 8 000 |
| `adapters/freecad/.../contracts.py` | `DESIGN_TIMEOUT_SECONDS` 1 800, `calls_json` maxLength, `ineffective_cuts` output field |
| `backend/crates/kala-cad-providers/src/base.rs` | threshold-free bound, exact count, summaries, tests |
| `backend/crates/kala-cad-providers/src/freecad.rs` | default/max timeout, warning pass-through without a 16-item pre-cap |
| `backend/crates/kala-cad-providers/src/driver.py` | forwards `ineffective_cuts` |
| `backend/crates/kala-cad-providers/src/lib.rs` | export the warning helpers |
| `backend/crates/kala-cad-providers/tests/freecad_unit.rs` | new bound shape, new timeout bounds |
| **new** `backend/crates/kala-cad-providers/tests/freecad_budget.rs` | live 100-hole/97-hole/500-step measurement |
| `backend/crates/kala-cad/src/lib.rs` | compile budget 60 s |
| `backend/crates/kala-json/src/limits.rs` | protocol string limit |
| `backend/crates/kala-tools/src/protocol/mod.rs` | string/argument bounds |
| `backend/crates/kala-studio/src/providers/mod.rs` | `ChatOutcome.text` + `reply`, `ChatOutcome::new` |
| `backend/crates/kala-studio/src/providers/{local,remote,claude_cli,codex_cli,opencode_cli}.rs` | construct with the full reply |
| `backend/crates/kala-studio/src/chat_design.rs` | read/repair from `reply`, display from full prose; repair bounds 200 000; new instruction |
| `backend/crates/kala-studio/src/server.rs` | `ChatOutcome::new` (2 sites) |
| `backend/crates/kala-studio/src/native_geometry/service.rs` | exact cut count, summary-aware display |
| `backend/crates/kala-studio/tests/{chat_mock_provider_design,native_geometry,repair_prompt_parity,corpus_fixture}.rs` | long-reply build, 100-hole/97-hole live tests, long-reply parity, digest |
| `backend/crates/kala-studio/tests/repair_prompt_oracle.py` | `display_reply` comparison |
| `src/kala_agent/studio/design/intent.py` | new instruction |
| `src/kala_agent/studio/design/service.py` | exact cut count, summary-aware display |
| `src/kala_agent/studio/design/compile_ocp.py` | compile budget 60 s |
| `src/kala_agent/studio/design/providers/{base,freecad}.py` | `ChatOutcome.reply`, warning helpers, timeout, driver copy |
| `src/kala_agent/studio/providers/{base,dispatch,openai_compat}.py` | `reply`, `_display_text`, 200 000 repair bounds |
| `src/kala_agent/tools/protocol.py` | string/argument bounds |
| `backend/compat/corpuslib/canonical_json_cases.py` | limit cases follow the product constants |
| `backend/compat/README.md` | new digest row |
| `docs/adr/0009-…` | budget line |
| Python tests | `test_construction.py`, `test_protocol_branches.py`, `test_executor_branches.py`, `studio_design/{test_intent,test_design_api,test_solid_count,test_templates,test_compile_ocp}.py`, `adapters/freecad/tests/test_design.py` |

## Gates

| Gate | Result |
|---|---|
| `cargo fmt --all --check` (backend workspace) | **clean** |
| `cargo fmt --check` in each touched standalone crate | **clean** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **clean** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **clean** |
| `cargo clippy --all-targets --all-features -- -D warnings` in kala-contracts, kala-design-intent, kala-json, kala-cad, kala-cad-providers, kala-solidedge-client, kala-tools | **clean** |
| `cargo test --workspace` | **554 passed, 0 failed** |
| `cargo test -p kala-studio` (standalone) | **280 passed, 0 failed** |
| `cargo test` kala-contracts | **52 passed** |
| `cargo test` kala-design-intent (incl. live Python differential) | **61 passed** |
| `cargo test` kala-cad | **119 passed** |
| `cargo test` kala-solidedge-client | **81 passed** |
| `cargo test` kala-cad-providers (lib + unit + differential + cut guard + Solid Edge) | **49 passed** (5 lib + 23 freecad_unit + 1 freecad_differential + 4 freecad_cut_guard + 3 solidedge_differential + 13 solidedge_mock_bridge) |
| `cargo test --test freecad_budget` (live, 4 tests) | **4 passed, 0 failed** (1 380.8 s) |
| `pytest tests/unit -q` | **1654 passed, 18 warnings** |
| `pytest adapters/freecad/tests -q` | **399 passed** |
| `pytest adapters/solidedge/tests -q` | **140 passed** |
| `pytest adapters/solidedge-bridge/tests -q` | **186 passed, 1 skipped** (live Solid Edge needs Windows) |
| `python_differential.py` | **268 cases, 0 mismatches** (mismatch id set: empty) |
| `worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** (id set: empty) |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| corpus `generate_corpus.py --check` | **determinism check passed** |
| `self_check.py` | **OK** — 56 Studio cases, 15 routes, 49 core cases, 17 routes |
| `cargo build --release -p kala-studio` | **pass** (`backend/target/release/kala-studio`, 11 681 088 bytes, 2026-10-21 02:28) |

### Live FreeCAD evidence (FreeCADCmd 1.1.4, this host)

`backend/crates/kala-cad-providers/tests/freecad_budget.rs --nocapture`:

```
100-hole plate (202 steps): 196465.708265 mm3 (expected 196465.7082647115), 0 warnings, 58.764001971s (73.38344453s in the final loaded run)
100-hole plate with 3 off-plate holes: 196571.737017 mm3, warnings [
  "component 'plate': pocket at step index 3 on XY removed no material",
  "component 'plate': pocket at step index 25 on XY removed no material",
  "component 'plate': pocket at step index 201 on XY removed no material"]
500-step plate: 19964798.454316 mm3 (expected 19964798.454316527) in 1009.536671591s (1250.01431127s in the final loaded run)
```

Studio routes (`native_geometry`, live):

```
OpenCascade 100-hole plate (202 steps) built in 3.2s
FreeCAD 100-hole plate (202 steps) built in 59.8s
100-hole plate view: 10624 vertices, 10812 triangles
100-hole plate drawing: 8452 points, truncated false
100-hole plate XZ section: 11 outer loops, 0 inner loops, area 850.0
three_off_plate_holes_are_counted_exactly_on_opencascade ... ok
three_off_plate_holes_are_counted_exactly_on_freecad ... ok
```

Both 97-hole tests assert `cuts_effective` expected 100 / measured 97 and
`verified: false`; both 100-hole tests assert the exact volume, `verified: true`
and that no `cuts_effective` row is present.

The 4-test `freecad_budget` binary was re-run with the final code: **4 passed,
0 failed** in 1 380.8 s, with the same volumes and the same three warnings. The
repeated run confirms the final constants (1 800 s deadline, 8 000 calls,
512 KiB call-list bound) do not change the result.

## Differentials

All three baselines stayed **0 / 0 / 0** and the mismatch id sets are empty:

| Differential | Cases | Mismatches | Ids |
|---|---|---|---|
| `python_differential.py` | 268 | 0 | ∅ |
| `worker_differential.py` | 292 | 0 (0 known gaps) | ∅ |
| `python_provider_dto_oracle.py` | 20 | 0 | ∅ |

The design cases (`design.chat_with_freecad`, `design.chat_with_opencascade`,
`design.with_provider`, geometry, journal, import, export) are unchanged; the
plain-chat cases are byte-identical because the non-design path still returns
`outcome.text`.

## Gaps / honest notes

1. **A 500-step FreeCAD build takes 16.8 minutes.** The contract now allows it,
   the deadline (1 800 s) covers it with 78 % idle / 44 % loaded margin, but it is not a pleasant
   chat turn. The cost is the per-pocket recompute with the volume check; it is
   superlinear (100 pockets 59 s, 249 pockets 1 009 s). A future cell could
   report the per-pocket volume without forcing a full recompute, or cap the
   number of pockets separately with a lower design budget.
2. **The 600-operation Solid Edge cap cannot be reached from a contract-valid
   script.** With ≤ 8 parts and ≤ 500 steps the maximum is 543 ops; a script
   with more parts is refused by the parts cap first. The ops check remains as
   defence in depth but has no reachable refusal path through
   `compile_operations`; the new parts test is the reachable one.
3. **Raising `MAX_STRING_LENGTH` and `MAX_ARGUMENT_BYTES` to 512 KiB loosens a
   global tool-envelope bound** (was 32 KiB / 256 KiB). The whole envelope is
   still bounded at 1 MB, and a single string can use at most the argument
   budget. This was required because the FreeCAD design tool passes the whole
   compiled call list as one string; the alternative (an array argument) would
   change the accepted FreeCAD tool contract and its schema digest.
4. **The `ineffective_cuts` count reaches Studio through the warning display,
   not a typed DTO field.** The adapter also reports it as a separate integer in
   its own result, but `ProviderBuild` has no such field, so
   `build_provider_checks` parses the summary line. The count is exact and
   tested for 500 cuts; a future cell can promote it to a typed field without
   changing the display.
5. **`templates.MAX_SEGMENTS` moved 8 → 64** because it derives from
   `MAX_COMPONENTS`. The intent template call still caps a `stepped_shaft` at 8
   segments, so a 9+-segment template result cannot be expressed as the
   corresponding template intent. This is deliberate (the template path is a
   direct construction), but it is a parity asymmetry worth a decision.
6. **No real model was called.** The long-reply and repair evidence uses the
   mock OpenRouter transport and the live Python reference; the conductor
   should still run one live MiniMax/Claude design larger than 4 096 characters
   to see the fix end to end.
7. **CLI provider display text is now bounded to 4 096** by `ChatOutcome::new`
   (previously only the HTTP provider was), but their `reply` is complete, so
   design JSON is not cut. No CLI test asserts a > 4 096-character display, so
   no existing expectation changed.
8. **The workspace test count includes concurrent cells' changes** (the
   CLI-provider and model-catalog work is in the same uncommitted tree); the
   kala-studio count is 280, kala-design-intent 61, kala-contracts 52,
   kala-cad 119, kala-solidedge-client 81 and kala-cad-providers 49 (plus the
   live budget binary).
