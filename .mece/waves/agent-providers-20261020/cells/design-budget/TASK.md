# Cell design-budget: no small fixed step/component limits; one generous whole-design budget

Read first: `../../WAVE.md` (incl. "Closing status"), `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials; never run the real claude/codex/opencode CLIs), the REPORTs of `../polish`, `../cut-guard`, `../build-repair` and `../repair-context`, and `docs/construction-semantics.md`.
**/tmp is a small quota tmpfs.** `rm -rf` your temp HOME and any extraction dirs when you finish, and check `df -h /tmp` before large builds.

## User decision (2026-10-21)
The user approved raising the limit, then asked for "as many parts as the design requires". Literally unbounded is not acceptable, because LLM input must stay bounded. So: no small per-component limit, and one generous whole-design budget sized from the real downstream limits.

## Evidence
Live with free MiniMax: a 4-hole flange (12 steps) builds exactly in 9–14 s. A 6-hole flange (16 steps) forces a split into halves or bands, takes 80–180+ s, and often comes out wrong. Today `MAX_COMPONENTS = 8` and `MAX_STEPS = 12` (`backend/crates/kala-contracts/src/construction.rs:22,24`; Python `src/kala_agent/contracts/construction.py:28` and its components bound).

## Changes
1. **Contract budget** (Rust `kala-contracts` + Python `kala_agent.contracts`, identical messages):
   - `MAX_COMPONENTS` 8 → **64**.
   - Per-component `MAX_STEPS` 12 → **500**, so it is no longer a practical constraint.
   - NEW `MAX_TOTAL_STEPS = 500`: the sum of the steps of all components plus the assembly steps. Reject 501 with a clear message naming the total and the cap.
   - Grep the whole repo for every other place that encodes 8/12, or derives from `MAX_STEPS`/`MAX_COMPONENTS`, and update or justify each: the JSON Schema `maxItems`, the FreeCAD tool description `adapters/freecad/src/kala_agent_freecad/cad/models.py:556`, the Solid Edge bridge doc `adapters/solidedge-bridge/src/kala_solidedge_bridge/protocol.py:470`, `adapters/solidedge/tests/test_compile.py:336`, and docs.
2. **Solid Edge stays honest:** the bridge caps a build at `MAX_OPERATIONS = 600` wire ops (`protocol.py:59`, `kala-solidedge-client/src/compile.rs:127`), and `MAX_PARTS` exists in `kala-solidedge-client`. Do NOT change the bridge wire caps. Make sure a script within Kala's budget that compiles to more than 600 ops or more than `MAX_PARTS` parts is refused by the Solid Edge compile with a clear message (it already checks ops), and add a test. Report the lowering ratio (bridge ops per script step).
3. **Hidden reply cap (critical):** `kala-studio/src/providers/remote.rs:508` truncates the model reply to `MAX_RESPONSE_TEXT_LENGTH = 4096` chars BEFORE `remote_design_turn` extracts the design JSON, so any larger script is cut mid-JSON and fails validation. The design JSON must come from the FULL reply (still bounded by `MAX_HTTP_RESPONSE_BYTES` = 1 MB, and `MAX_CLI_STDOUT_BYTES` = 1 MB for the CLIs). Only the DISPLAYED `ChatResponse.text` stays ≤ 4096 (keep that parity), and it is the prose with the JSON block stripped, as today. Check the CLI providers for the same truncation. Mirror in Python (`openai_compat.py` / `dispatch.py`). Keep the plain-chat (non-design) behaviour and every existing differential case byte-identical.
4. **Downstream bounds sized for the budget:**
   - FreeCAD warnings cap (`adapters/freecad/.../design.py:54 MAX_WARNINGS = 16`, Python `studio/design/service.py:99 _MAX_WARNINGS = 16`, and the Rust equivalent): the `cuts_effective` count must be EXACT for up to 500 pockets, independent of how many warning lines are displayed. Have the builder report the ineffective-cut count (or the per-pocket list) separately, and display at most a bounded number of lines plus `… and N more`.
   - Repair prompts: `MAX_REPAIR_PREVIOUS_CHARS` / `MAX_BUILD_REPAIR_SCRIPT_CHARS` = 6000 (`chat_design.rs:272,473`, plus the Python mirrors) must hold a max-budget script. Measure the canonical JSON size of a 500-step script and set the bounds to fit it with margin (byte-identical Rust/Python).
   - FreeCAD build deadline `DESIGN_TIMEOUT_SECONDS = 180` (`adapters/freecad/src/kala_agent_freecad/contracts.py:57`): MEASURE a 100-hole plate (~202 steps) and a ~500-step case in real FreeCAD 1.1.4 (with the per-pocket volume check), then set the deadline to cover the 500-step case with margin, or scale it with the step count. Report the timings.
   - Check the mesh limits (200k vertices / 400k triangles), the drawing/HLR and the section on the 100-hole plate. They must succeed or refuse honestly. Report.
5. **Instruction** (byte-identical in `kala-design-intent/src/intent.rs`, `kala-studio/src/chat_design.rs` and Python `studio/design/intent.py`): replace "A component has at most 12 steps; when a part needs more, split it into components that each carry their own features, place them, and fuse them." with a sentence saying a design may use up to 500 steps in total across up to 64 components, that each solid body is one component, and that a body must NOT be split just to save steps. Pin the numbers to the constants with tests (replace the existing step-limit test).
6. Regenerate the corpus and fixtures with the documented commands (`backend/compat/README.md`) and report the old and new digests.

## Tests (the spec)
- Validation boundaries in Rust and Python: 500 total steps OK, 501 rejected; 64 components OK, 65 rejected; one 300-step component OK.
- A 200 × 200 × 5 mm plate with a 10 × 10 grid of Ø3 holes (202 steps): validates, and builds in FreeCAD (live) AND OpenCascade with the exact volume `200·200·5 − 100·π·1.5²·5`, verified, `cuts_effective` absent. The same plate with 3 holes moved off the plate gives `cuts_effective` expected 100 / measured 97.
- Reply cap: a mock provider reply with a valid 6 000-char design JSON plus prose builds; the displayed text is ≤ 4096; Rust and Python agree.
- Solid Edge compile: an over-600-op script is refused with the clear message.

## Gates
`cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings` with and without `--all-features`; `cargo test --workspace`, or at least `kala-contracts`, `kala-design-intent`, `kala-cad`, `kala-cad-providers`, `kala-solidedge-client` and `kala-studio` (counts); `pytest tests/unit -q`; `adapters/freecad/tests`; `adapters/solidedge*/tests` (counts); the differentials (`python_differential.py` 268, `worker_differential.py` 292, `python_provider_dto_oracle.py` 20), baseline **0/0/0**, which must stay 0/0/0 (report the id sets); corpus `--check` and `self_check.py`; `cargo build --release -p kala-studio`.

## Ownership
Everything named above. Nothing else runs in parallel. Re-read files before patching. Port 8831 is free now, but do not use it. Do not commit.

## REPORT
`REPORT.md` here: per-file summary, the final constants and messages, the instruction text, the lowering ratio, the FreeCAD timings and the chosen deadline, the mesh/drawing results, digests, all counts, and gaps.
