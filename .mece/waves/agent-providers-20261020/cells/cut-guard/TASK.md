# Cell cut-guard: no "verified" part with a cut that removed nothing

Read first: `../../WAVE.md`, `../../../rust-backend-20261019/cells/CONVENTIONS.md` (the USER DATA rule: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then set a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`; never touch `/home/v9i/.local/share/kala-agent/**`; never read credentials), and `docs/construction-semantics.md` § "Cuts and pockets" (it already REQUIRES this: "a future native executor must compare the measured volume before and after a cut and report a warning or a failure when the volume did not decrease").

## Opus finding (live, 2026-10-20)
Prompt: flange 100 OD × 12, Ø30 bore, 6×Ø8 on an 80 PCD, built by FreeCAD. The Codex CLI authored the intent in `repro-codex-flange-intent.json` (this folder). FreeCAD built it, and the reply said "B-Rep verified" with `verified: true`, but the measured volume is 85765.479443 mm³ against the correct 82146.364706 mm³. The difference is exactly 6 × π·4²·12, so NONE of the bolt holes were cut. Cause: each hole is sketched on `XZ` while the component's solid came from an `XY` sketch (a revolve), and the pocket step says `plane: XY`. The OpenCascade compiler (`src/kala_agent/studio/design/compile_ocp.py` ~l.258–275, and `backend/crates/kala-cad/src/native/compile.rs` ~l.300) REFUSES "pocket on XZ over a XY solid" as unsupported and warns "pocket … removed no material". The FreeCAD path (`adapters/freecad/src/kala_agent_freecad/cad/design.py` `pocket()` ~l.549) does neither, and `build_provider_checks` (`backend/crates/kala-studio/src/native_geometry/service.rs` ~l.1778) has no cut check, so `verified` stays true.

## Change A: validation enforces the same-plane pocket rule (provider-independent)
In intent/script validation (Rust `backend/crates/kala-design-intent/src/script.rs` and the Python mirror the differential compares, `src/kala_agent/studio/design/intent.py` or wherever ConstructionScript validation lives), reject a pocket when (a) its own `plane` differs from the pending sketch's plane, or (b) the pending sketch's plane differs from the plane of the sketch that produced the component's current solid. Use exactly the same rule the OCCT compiler applies, and give a precise message naming the component, the step index and both planes. The chat flow's existing single repair round then feeds the message back to the model. Byte-identical messages in Rust and Python. First check `tests/fixtures/construction_parity.json`, the differential corpora and the template outputs (l_bracket, stepped_shaft, housing_envelope) for any case this would newly reject. If any exists, STOP and report it rather than changing the rule.

## Change B: the FreeCAD builder detects a cut that removed nothing
In `design.py` `pocket()`, measure the body solid's volume before and after the recompute. If it did not decrease (use a tolerance such as `max(1e-9·before, 1e-6)` mm³, and write down your choice), append a session warning. It must contain the substring `removed no material` and name the component (or body) and the plane, matching the OCCT wording `component '<name>': pocket on <PLANE> removed no material` as far as the adapter knows those facts. Do not raise. Find how adapter warnings reach the Studio response (`backend/crates/kala-cad-providers/src/driver.py` → `freecad.rs` → `kala-studio`) and make sure this one does. Keep the adapter's existing tests green (`adapters/freecad/tests`).

## Change C: a cut that removed nothing fails verification (every provider)
When a build's warnings contain a `removed no material` cut warning (FreeCAD, and the opt-in OpenCascade provider, which already emits it), add ONE failed check row: `id: "cuts_effective"`, label `Cuts removed material`, `passed: false`, expected = number of pockets in the script, measured = pockets that did remove material (or the closest honest value; write down your choice). `verified` must therefore be false. Add the row ONLY when a cut was ineffective, so every existing passing design stays byte-identical (no corpus or digest churn). Rust `service.rs` (both the provider path and the opencascade path ~l.561/~l.990) plus the Python mirror, so the differentials stay at 0 mismatches.

## Tests (the spec)
- Validation: the repro intent is rejected with the new same-plane message (Rust and Python); every existing fixture still validates.
- FreeCAD, live (FreeCAD 1.1.4 is installed on this host; see how `backend/crates/kala-cad-providers/tests/freecad_differential.rs` finds and drives it): a same-plane plate with a Ø6 hole sketched entirely OUTSIDE the plate builds, carries the warning, has a failed `cuts_effective` row, and `verified: false`. The normal 80×40×8 plate with 4×Ø6 holes is still volume 24695.221316, `verified: true`, with no new row.
- OpenCascade opt-in: the same off-plate hole gives a failed `cuts_effective` row and `verified: false`.

## Gates
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` (or per touched crate), and `cargo test` for `kala-design-intent`, `kala-cad-providers` and `kala-studio` (report counts).
- `pytest tests/unit -q` and `adapters/freecad/tests` (report counts).
- Live differentials from `backend/compat/README.md` (`python_differential.py`, `worker_differential.py`): mismatch counts before and after, 0 expected.
- `cargo build --release -p kala-studio`.

## Ownership (another worker is editing the timeout/chat path at the same time)
Yours: `kala-design-intent/src/script.rs` (+ its tests), `adapters/freecad/src/kala_agent_freecad/cad/design.py` (+ tests), `kala-cad-providers/src/{driver.py,freecad.rs}` only if the warning plumbing needs it, `kala-studio/src/native_geometry/service.rs`, the Python validation and check mirrors under `src/kala_agent/studio/design/`, and the matching tests.
NOT yours, do not edit: `kala-studio/src/{server.rs,chat_design.rs,providers/**}`, `src/kala_agent/studio/{server.py,providers/**}`. Re-read any shared file immediately before patching it. Do not commit.

## REPORT
Write `REPORT.md` here with per-file diff summaries, the exact new messages, the tolerance choice, test and gate counts, the differential counts, the live FreeCAD evidence (volumes, checks), and any gap.

## Addendum (Opus, while you work): a second repro, all same-plane
`repro-minimax-flange-intent.json`: MiniMax M3.1 Flash via the OpenCode CLI, same flange prompt. FreeCAD built it, `verified: true`, volume 85765.479443 again (no holes). Every step is on XY, so Change A does NOT apply. Each half pockets away its OWN side (`flange_right` removes the rectangle x∈[0,100]) and then drills its holes there, so all 6 hole pockets cut empty space. This is the canonical Change B/C case. Add a live FreeCAD test with exactly this intent: it must yield 6 ineffective cuts, a failed `cuts_effective` row and `verified: false`, and Change A must still accept it (same plane). If the half-removal rectangle pockets themselves DO remove material, they must not be counted as ineffective.
