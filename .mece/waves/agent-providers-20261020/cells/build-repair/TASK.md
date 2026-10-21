# Cell build-repair: one repair round fed by the BUILD result, not only by validation

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials), `../design-timeout/REPORT.md` (just landed), and `../cut-guard/TASK.md` (running in parallel: it adds a failed `cuts_effective` check row plus a `removed no material` warning, so a design whose cut removed nothing gets `verified: false`).

## Opus findings (live, 2026-10-20, flange prompt: 100 OD × 12, Ø30 bore, 6×Ø8 on an 80 PCD, FreeCAD)
1. MiniMax M3.1 Flash via OpenCode and Codex both authored scripts that VALIDATE and BUILD but cut none of the bolt holes (volume 85765.48 vs the correct 82146.36). Today `remote_design_turn` (`backend/crates/kala-studio/src/chat_design.rs` ~l.893) repairs only when validation fails. A build that comes back unverified, or a builder refusal caused by the script, is returned as is and the model never learns why.
2. OpenRouter `minimax/minimax-m3.1-flash:free` took longer than 120 s on the flange's first call (the plate takes 23 s). The same model via the OpenCode CLI finished in 80 s. The CLI providers already get 180 s.

## Change 1: build-feedback repair
In `remote_design_turn`, after `finish_design` builds the design:
- **Triggers** (the model's script caused the problem): the design was built with `verified == false` (any failed check row), OR the provider REFUSED the script for a script reason (a compile or validation refusal from the builder, e.g. `unsupported: …`). NOT a provider-unavailable refusal (`is_provider_unavailable()`), a backend error, or a missing `cad_provider`.
- **Budget:** at most ONE repair call per turn IN TOTAL. If the validation repair was already spent, do not do a build repair. Worst case stays 2 model calls.
- **Repair prompt:** a new `build_repair_prompt(prompt, feedback, schema_text)`, modelled on `repair_prompt`, holding the user's original request, the script the model sent (compact JSON, bounded, e.g. ≤ 6 KB), and the concrete build feedback: the failed check rows (id, expected, measured), the warnings (e.g. `component 'flange_right': pocket on XY removed no material`), or the refusal reason. Tell the model to fix the script so every cut removes material and the part matches the request, and to keep the stated dimensions. Bound the whole prompt.
- **Second build:** validate and build the repaired reply. Return the SECOND result as the turn's design if it validated and built. The reply text must say plainly that the first attempt was rejected and why (one short sentence with the reason), then give the second attempt's normal summary. If the repaired reply is invalid, does not build, or is unverified again, return the best honest state: the latest built design, if any, with `verified: false` visible, plus the existing "No part was built: …"-style honesty text when nothing built. Never claim verified unless the returned design says so.
- Both builds are journaled naturally by the existing design path. Keep the single `chat.turn` per turn, pointing at the returned design_id.
- Mirror in the Python reference (`src/kala_agent/studio/server.py` design-turn path / wherever the Python remote design turn lives) so the two implementations agree.

## Change 2: design budget 180 s
Set `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS` to 180 in Rust (`providers/remote.rs`) and Python (`openai_compat.py`), matching the CLI providers' budget. Update the doc comments and the tests that assert 120.

## Tests (the spec)
- Rust (pattern: `tests/chat_mock_provider_design.rs`, mock provider + backend): (a) first reply builds unverified (failed `cuts_effective` row), the second reply builds verified → the turn returns the second design, the text mentions the first rejection reason, and exactly 2 model calls were made, the second prompt containing the failed check and warning text; (b) validation repair already spent → no build repair (2 calls total); (c) provider unavailable → no repair; (d) repaired reply still unverified → the returned design has `verified: false` and the text never says verified.
- Python: the equivalent (a) and (b).

## Gates
- `cargo fmt --check`, `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings`, `cargo test -p kala-studio` (counts).
- `pytest tests/unit -q` (counts).
- `backend/compat/python_differential.py` and `python_provider_dto_oracle.py`: before/after counts. The current baseline is 47 pre-existing mismatches (documented in `../design-timeout/REPORT.md`) and 0. No new mismatch.
- `cargo build --release -p kala-studio`.

## Ownership
Yours: `kala-studio/src/chat_design.rs`, `kala-studio/src/providers/remote.rs` (the constant and docs only), the matching Rust tests, the Python remote design-turn path and `openai_compat.py` constant, and the matching Python tests. NOT yours: `kala-studio/src/native_geometry/**`, `kala-design-intent/**`, `adapters/**`, `kala-cad-providers/**`, `src/kala_agent/studio/design/**` (the cut-guard worker owns those right now). Cargo builds share the target dir, so expect lock waits. A live Studio backend runs on port 8831: do not stop it, and do not use that port. Re-read shared files right before patching. Do not commit.

## REPORT
Write `REPORT.md` here with per-file diff summaries, the exact repair prompt template, test and gate counts, differential counts, and gaps.

## Round 2 (Opus review of round 1, 2026-10-20)
Round 1 was accepted: the budget logic, the triggers, the keep-first honesty, and the gates. There are two defects to fix:

**R1: a failed repair CALL discards the first build.** In `repair_build`, `call(build_repair_prompt(…)).await.map_err(DesignTurnError::Provider)?` turns a provider failure on the REPAIR call (timeout, network, protocol) into an error envelope for the whole turn. The first design, which was already built and journaled, is then thrown away. A free OpenRouter model that spends 150 s on the first call will hit exactly this. Fix: on a repair-call provider failure, keep the first build through the existing `keep_first_build` path, with the reason `the repair attempt failed: <envelope code, e.g. provider_timeout>.`. The turn returns 200 with the first (unverified, or refused) state, never an error envelope, and the single `chat.turn` points at the first design. Mirror in Python. Tests: Rust and Python, with the first build unverified and the repair call timing out → 200, the first design returned with `verified: false`, the text containing the first rejection reason and `the repair attempt failed: provider_timeout`.

**R2: the build-repair prompt lacks the instruction.** Every model call is stateless. The build-repair prompt carries the old script, the feedback and a script-schema excerpt, but not `DESIGN_INTENT_INSTRUCTION`, so the model no longer sees the supported subset, the plane rule or the worked examples. Prepend `DESIGN_INTENT_INSTRUCTION` (the same constant `design_prompt` uses, unchanged) to `build_repair_prompt`, keep the rest, and keep it bounded. Python mirror byte-identical. Update the prompt tests.

Same ownership and gates as round 1. Append a "Round 2" section to REPORT.md.
