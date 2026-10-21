# Cell repair-context: the VALIDATION repair call gets the same context as the first call

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials), and the REPORTs of `../build-repair` (Round 2 = R2) and `../polish`.

## Opus finding (live, 2026-10-20)
OpenRouter `minimax/minimax-m3.1-flash:free`, 6-hole flange prompt: the first reply failed validation, and the repaired reply came back in a DIFFERENT shape (`operation` instead of `op`, no `summary`, an invalid component name). The turn ended "No part was built: the reply after one repair attempt was not a valid design (…)". Cause: `repair_prompt` (`backend/crates/kala-studio/src/chat_design.rs`, ~l.257) carries only the validation error, a schema excerpt for the failing location, and the user request. Every model call is stateless, so the repair call sees neither `DESIGN_INTENT_INSTRUCTION` (the supported subset, the conventions, the worked examples), nor the full schema, nor its OWN previous JSON. It re-authors from scratch, blind. build-repair R2 fixed exactly this for the BUILD repair prompt. The validation repair prompt needs the same.

## Change
- `repair_prompt` becomes: `DESIGN_INTENT_INSTRUCTION` + the full DesignIntent JSON Schema (as `design_prompt` sends it) + the model's previous fenced JSON block (extracted, bounded, e.g. ≤ 6 000 chars, the same bound build-repair uses for the script) + the validation error (bounded as today) + the existing targeted schema excerpt for the failing location + the instruction to return a corrected DesignIntent that keeps every stated value + the user request. Keep the existing "if a required value is missing, reply in prose" sentence. Thread the previous reply text into `repair_prompt` from `remote_design_turn`.
- Python reference mirror (where the Python validation repair prompt is built, likely `src/kala_agent/studio/providers/dispatch.py`), byte-identical. Prove it with a test comparing the two prompts for the same inputs (pattern: the build-repair R2 byte-compare).
- Honesty text: when the turn ends with no part after the repair, name BOTH errors briefly (the first and the second), each bounded, so the user sees what went wrong the first time too.

## Tests and gates
- Rust and Python: the repair prompt contains the instruction, the schema, the previous JSON and the error, and is byte-identical across languages. The honesty text names both errors. The existing repair tests stay green (update the pinned prompt expectations).
- `cargo fmt --all --check`; `cargo clippy -p kala-studio --all-targets -- -D warnings` with and without `--all-features`; `cargo test -p kala-studio`; `pytest tests/unit -q`.
- Differentials (`python_differential.py`, `worker_differential.py`, `python_provider_dto_oracle.py`): before and after id sets. No new mismatch.
- `cargo build --release -p kala-studio`.

## Ownership
`kala-studio/src/chat_design.rs`, the Python validation repair path, and the matching tests. Re-read shared files right before patching. A live Studio runs on port 8831: do not stop it, and do not use that port. Do not commit.

## REPORT
`REPORT.md` here: the final prompt template, the byte-compare evidence, counts, and gaps.

## Addendum (Opus, at launch)
- The python-parity and polish cells have landed: the differentials are now **0 / 0 / 0** (`python_differential` 268, `worker_differential` 292, DTO oracle 20), and that is the baseline. The corpus digest is `61328f91…` (see `../polish/REPORT.md`). If your Python edits move the corpus manifest's source pin, regenerate with the documented `generate_corpus.py` command, run `--check` and `self_check.py`, and report the old and new digests.
- The Python remote design path now takes a provider-call seam (python-parity), so CLI providers also go through it. Keep that working.
