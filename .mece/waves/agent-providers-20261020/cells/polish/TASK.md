# Cell polish: clearer design instruction, per-pocket warnings, loose ends

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials), and the REPORTs of `../llm-design`, `../cut-guard` and `../build-repair`.

## Why (Opus, live evidence 2026-10-20)
On the flange prompt (100 OD × 12, Ø30 bore, 6×Ø8 on an 80 PCD), MiniMax and Codex both hit the 12-step component limit, split the flange into two halves, and then drilled holes into material that did not exist (MiniMax pocketed away each half's OWN side and drilled there). The instruction never states where a rectangle's x/y sits, the step limit, or that a pocket must cut existing material of its own component. The ineffective-cut warning is also one line per component, not per pocket, so the repair feedback cannot say WHICH holes missed.

## 1. Instruction wording (byte-identical everywhere it lives)
`DESIGN_INTENT_INSTRUCTION` lives in `backend/crates/kala-design-intent/src/intent.rs` (~l.39), `backend/crates/kala-studio/src/chat_design.rs` (~l.36; a test pins it to the contract crate) and the Python `src/kala_agent/studio/design/intent.py` (and wherever else `grep` finds it). Directly after the sentence ending "…and the component must end with a solid.", add, adjusting only if the code proves a fact wrong:

> Sketch coordinates are in the sketch plane: a rectangle's x/y is its lower-left corner, so it spans x to x+width and y to y+height; a circle's x/y is its centre. Every pocket must cut into material that its own component already has at that step; a cut where the component has no material removes nothing, and the build is rejected. A component has at most 12 steps; when a part needs more, split it into components that each carry their own features, place them, and fuse them.

First VERIFY each fact against `docs/construction-semantics.md` and the compilers (`kala-cad/src/native/compile.rs`, `src/kala_agent/studio/design/compile_ocp.py`, and the FreeCAD lowering), above all the rectangle corner and the circle centre on XY, XZ and YZ. If a fact is wrong, fix the sentence and report it. Add a Rust test asserting that the instruction's step count equals `kala_contracts::MAX_STEPS`, so the two cannot drift. Update every pinned length or digest test the change touches. The corpus and fixture digests that embed the instruction (see `backend/compat/README.md` and `backend/fixtures/README.md`) must be regenerated with the documented commands and REPORTED (old and new digest), never hand-edited.

## 2. Per-pocket ineffective-cut warnings
Make the warning name the pocket: `component '<name>': pocket at step index <i> on <PLANE> removed no material`, matching the cut-guard plane message's `pocket at step index <i>` wording, ONE warning per ineffective pocket (find and remove whatever collapses duplicates today; the MiniMax repro `../cut-guard/repro-minimax-flange-intent.json` gives 6 ineffective pockets but only 2 warnings). Do this in the OCCT compiler (Rust `kala-cad/src/native/compile.rs` + Python `compile_ocp.py`) and the FreeCAD adapter (`adapters/freecad/src/kala_agent_freecad/cad/design.py`), byte-identical. The step index is the index within the component's `steps`, the same as the plane message. Keep `cuts_effective` counting correct (live: expected 10, measured 4 for that repro). STOP clause: if a corpus or fixture digest changes, regenerate it with the documented command and report old and new. If a parity test against the live Python reference changes meaning, report it rather than forcing it.

## 3. Loose ends
- Move the provider-failure → envelope-code mapping into ONE place (`ProviderFailure::envelope_code()` in `kala-studio/src/providers/mod.rs`) and use it from `server.rs` and `chat_design.rs`. build-repair duplicated it into `chat_design.rs` (`provider_envelope_code`) because `server.rs` was busy.
- `cargo fmt --all` (cut-guard left `kala-cad-providers/tests/freecad_cut_guard.rs`, `kala-design-intent/src/script.rs` and `tests/design_contract.rs` unformatted).
- The pre-existing `kala-studio/tests/geometry_exports.rs:18` unused import that breaks `cargo clippy -p kala-studio --all-targets -- -D warnings` without `--all-features`: cfg-gate it correctly.

## Tests and gates
- Live FreeCAD (1.1.4 on this host; drive it as `kala-cad-providers/tests/freecad_cut_guard.rs` does): the MiniMax repro gives exactly 6 per-pocket warnings with the right step indices, `cuts_effective` 10/4 failed, `verified: false`. The normal 4-hole plate stays 24695.221316, verified, with no warnings. OCCT gives the same warnings for the same repro.
- `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings` AND with `--all-features`; `cargo test` for `kala-design-intent`, `kala-cad`, `kala-cad-providers` and `kala-studio`; `pytest tests/unit -q`; `adapters/freecad/tests` (counts for all).
- Differentials (`python_differential.py`, `worker_differential.py`, `python_provider_dto_oracle.py`): before and after. The known baseline is 47 / 57 / 0, all provider-list drift (a parallel cell is fixing that drift in the Python reference, so the counts may DROP during your run; report the mismatch id sets, and nothing in `design.*`, `convert.*`, `import.*` or `journal.*` may newly mismatch).
- `cargo build --release -p kala-studio`.

## Ownership
Yours: the files named above (both instruction copies, `chat_design.rs`, `providers/mod.rs`, `server.rs` for the mapping only, `kala-cad/src/native/compile.rs`, `compile_ocp.py`, the FreeCAD `design.py`, `geometry_exports.rs`, the formatted files) and their tests. NOT yours: `src/kala_agent/studio/providers/**`, `src/kala_agent/studio/{settings,schemas}.py` and the Python `server.py` (the python-parity cell owns those now). Re-read shared files right before patching. A live Studio runs on port 8831: do not stop it, and do not use that port. Do not commit.

## REPORT
Write `REPORT.md` here: the final instruction text and length in both languages, the verified facts with code citations, the old and new digests, per-file summaries, all counts, the live evidence, and gaps.
