# Wave: completeness, the little necessary things in UI and backend (2026-10-21)

## User request
"also check all the little missing thing in the ui and back end that are necessary things and also add them too"

## Shape
1. **Audit (read-only, parallel):** `audit-ui` (Studio UI, `studio/src/**`) and `audit-backend` (Rust backend, the Python reference only where parity requires it, packaging, the FreeCAD add-on, docs). Each writes `AUDIT.md`.
2. **Opus adjudication:** keep what is NECESSARY (a user hits it in normal use, or it is broken, misleading, unsafe, or blocks install or use), drop the nice-to-haves, and flag anything that needs a user decision.
3. **Fix cells** with disjoint ownership, after the running `agent-providers-20261020/cells/design-budget` cell lands where they overlap.

## Rules for every cell
- USER DATA: in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then a temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`. Never touch `/home/v9i/.local/share/kala-agent/**`. Never read credential, auth or settings files (Kala's or any CLI's). Never run the real `claude`, `codex` or `opencode` CLIs.
- `/tmp` is a small quota tmpfs: `rm -rf` your temp dirs when done.
- UI invariants from `.mece/waves/vellum-ui-20261019/WAVE.md` § Invariants all apply. Above all **No fake data**: the UI shows only real state, or an explicit empty or unknown state.
- **In flight, do NOT report:** the design-budget cell is changing the step/component limits, the 4096-char reply truncation, the FreeCAD warnings cap and the cut count, the repair-prompt bounds, the FreeCAD build deadline and the instruction step sentence (see `../agent-providers-20261020/cells/design-budget/TASK.md`).
- Known facts: Codex and OpenCode `--json` output does NOT name the model when none is chosen (Opus checked live 2026-10-21), so "show the real model" is impossible there. The UI must instead say honestly that the CLI's own default model is in use, and point to the model picker to pin one.
- No commits.

## Status (Opus, 2026-10-21)
Cells landed and reviewed: audit-ui, audit-backend, ui-fixes, packaging, runtime-fixes, downloads-cancel, docs (round 1), plus agent-providers/design-budget. Pending: docs round 2 and corpus-hermetic.
- Opus independent gates (temp dirs): cargo workspace **616 passed / 0 failed**; pytest **1667**; FreeCAD adapter **399**; studio vitest **769**; fmt and clippy (± all-features) clean; differentials **268/0, 295/0 (0 gaps), 20/0**; self_check OK. The corpus `--check` FAILED: the core.health raw body_length depends on the generator's TMPDIR path length (415 committed vs 408 at the default /tmp). Sent to corpus-hermetic.
- Final AppImage: 163 889 259 bytes, sha256 `bf4734e18843fceb1d9bc98a305deddb5de771684f27772bd8c50dfa1d23149e`. Smoke PASS (OCCT + FreeCAD L-bracket exact, section exact). The packaged binary is string-checked for every landed change. `.desktop`: `StartupWMClass=kala-studio`, `Icon=kala-studio`.
- Live (stored OpenRouter key + subscriptions, FreeCAD 1.1.4, real data dir): the **6-hole flange** with free MiniMax builds as ONE 16-step component on the first attempt, exact 82146.364706, verified: OpenRouter **14 s** (was > 180 s fail), OpenCode **31 s** (was 159 s / timeout). Claude plate 9 s exact. FCStd downloaded via `/design/{id}/artifact/fcstd` for all three: 200, a real zip, sha256 = `artifacts.native_sha256`. `/health`: `offline: false` (CLIs available), `design_turn_budget_seconds: 3960`.
- Near-miss assessed: the vitest lifecycle test could reach the real data dir (B2). The real journal copy verified: 153 events, chain OK, none from tests. Fixed by runtime-fixes.
- Closing: docs round 2 and corpus-hermetic landed and were reviewed. Opus verified `generate_corpus.py --check` passes at the default TMPDIR, `self_check.py` OK, `corpus_fixture` 2/2 (corpus `39e0c0a9…`). Opus corrected two "still carries the earlier split" sentences (the final build regenerated native-readiness.json) and three current-digest statements (c0003bae → 39e0c0a9), and wrote the final AppImage row in `packaging/linux/README.md`. Live backend stopped. Nothing committed.
- Open / user decisions: a "Test API key" button (adds outbound key-check endpoints); pattern operations (polar/linear) for large hole counts; Windows build + run on the user's machine (MinGW install, `build-windows.ps1`); a possible FreeCAD single-recompute speed-up for very large parts.
