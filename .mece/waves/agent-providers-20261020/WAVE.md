# Wave: agent providers, model catalog, LLM-authored designs (2026-10-20)

## User request
"I should be able to use the already existing Claude, OpenAI, OpenCode subscription in the agent; when I give the API key of OpenRouter it should show me the models that are available, and with any API integration I need that also; when I give a prompt it should be able to build using the model … use MiniMax M3.1 Flash (free) and try generating some model in FreeCAD in Linux."

## Opus finding (live, 2026-10-20)
With the stored OpenRouter key, `minimax/minimax-m3.1-flash:free`, cad_provider=freecad, and the prompt "aluminium mounting plate 80 × 40 × 8 mm with four Ø6 through holes 8 mm from the edges": the reply came back in 27 s as PROSE ONLY, with NO design. Cause: `DESIGN_INTENT_INSTRUCTION` (kala-studio `chat_design.rs:33` and the Python reference) allows a raw ConstructionScript "only when the user supplied a complete script", so anything outside the three templates can never be built by a model.

## Rules
The Rust product is `backend/crates/kala-studio` (native). The Python Studio is a dev-only reference: keep it in parity where a shared contract changes (the instruction text, the DTOs), so the existing live differentials stay at 0. USER DATA rule (`.mece/waves/rust-backend-20261019/cells/CONVENTIONS.md`) in every shell. Workers NEVER read credential files or the user's settings. Tests use mock provider servers and fake CLIs. Opus runs the live tests with the user's stored key and subscriptions. No commit, push, or sudo. All remote calls require `allow_remote` consent (ADR-0001), and the CLI providers are remote egress too.

## Cells (disjoint ownership)
- llm-design (DeepSeek): `kala-studio/src/chat_design.rs`, the Python `studio/design/intent.py` + `providers/dispatch.py` instruction parity, and the relevant tests.
- cli-providers (MiniMax): NEW `kala-studio/src/providers/{claude_cli,codex_cli,opencode_cli}.rs` + the registration in `providers/mod.rs` + the settings provider list.
- model-catalog (DeepSeek): NEW `kala-studio/src/providers/catalog.rs` + the route `GET /api/studio/providers/{id}/models` (minimal server.rs wiring).
- model-ui (MiniMax): `studio/src/**` (the model picker, the subscription providers in the provider picker) against the contracts in the other cells' briefs.
Shared files (`providers/mod.rs`, `server.rs`, `schemas.rs`): minimal additive edits; re-read immediately before each patch; list them in REPORT.

## Closing status (Opus, 2026-10-20 ~20:00)
Follow-up cells after the first four: design-timeout, cut-guard, build-repair (2 rounds), polish, python-parity, repair-context. All reviewed and accepted.
- Independent final gates (temp data dirs): cargo test kala-studio + kala-design-intent + kala-cad-providers **358 passed / 0 failed**; pytest tests/unit **1623 passed**; python_differential **268 / 0**; worker_differential **292 / 0 / 0 gaps**; provider DTO oracle **20 / 0**; corpus `--check` and self_check OK (corpus digest `61328f91…`).
- Live (stored OpenRouter key + subscriptions, FreeCAD 1.1.4, real data dir): plate 80×40×8 4×Ø6 = 24695.221316, verified, via claude_code (7 s), codex_cli (24 s), opencode_cli (11 s) and OpenRouter minimax-m3.1-flash:free (9 s). 4-hole flange 83352.736285 via MiniMax free (9–14 s). 6-hole flange: Claude and Codex exact (82146.364706); MiniMax via OpenCode exact but 159 s; MiniMax via OpenRouter free still exceeds the 180 s first-call budget.
- A wrong part can no longer read "verified": the Codex no-holes flange is now rejected at validation (plane rule), and the MiniMax no-holes flange gives a failed `cuts_effective` row (10/4) with six per-pocket warnings → the build-repair round.
- AppImage rebuilt: `studio/release/kala-studio-0.1.0-x86_64.AppImage`, **163 859 910 bytes**, sha256 `b9be013066ec978d8797892419ed7d0dd537c59f9687decf296920841c5aa4c2`, unsigned; smoke PASS with `--require-geometry --require-freecad`. The packaged backend (patchelf'd copy, sha `4b638317…`) contains every final change (string-checked).
- OPEN, user decision: raise `MAX_STEPS` 12 → 32 (Rust `kala-contracts/src/construction.rs:24`, Python `contracts/construction.py:28`, FreeCAD tool text `models.py:556`, Solid Edge bridge doc `protocol.py:470`, corpus regen). Evidence: 4-hole flange (12 steps) 9–14 s exact; 6-hole (16 steps) 80–180+ s and error-prone.
- Incident: `/tmp` (7.7 GB tmpfs, per-user quota) filled with worker temp HOMEs and old extractions → the first smoke run failed with "Unexpected end of JSON input" (0-byte files on extraction). Opus removed non-empty `tmp.*` older than 60 min plus `kala-sol-*`, `kala-cad-asan` and `kala-studio-diff-*` (3.6 GB, all ours, inspected first); the re-run passed.
