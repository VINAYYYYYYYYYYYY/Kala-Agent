# Cell edit-turns (Phase A, backend): edit an existing design in a chat turn; report token usage

Read `../../WAVE.md` § Rules and § Contract ("Backend chat additions") first. The UI cells rely on exactly that contract.

## Why
The redesigned UI lets the engineer click a projected edge or hole and type "make this hole 10 mm". By default it also edits the active design (D16). Today every design turn drafts from nothing, and the 4096-char prompt cap means the UI cannot paste the current script into the prompt. The backend must carry the base design to the model itself. The UI also shows session token use, so the chat response must report usage where providers report it.

## Do
1. **`POST /api/studio/chat` request.**
   - **`base_design_id`** (optional): an existing design id in the backend's design store, with the same format check as the `/design/{id}/...` routes.
     - Unknown → `design_not_found` (404).
     - Set without `cad_provider` → `edit_requires_design_turn` (400).
     - Set with the `local` provider → `edit_requires_model` (400).
     - Refuse before any model call.
   - **`focus`** (optional): ≤ 1024 chars, trimmed, control characters other than newline refused. Without `base_design_id` it is refused as `focus_requires_base_design` (400).
   - The existing 4096-char `prompt` limit is unchanged. It covers the user's text only.
2. **Edit prompt (`chat_design.rs`).**
   - With a base design, the design prompt is the unchanged `DESIGN_INTENT_INSTRUCTION`, then the schema, then a new section:
     - `CURRENT DESIGN:` followed by the stored canonical script JSON (the same bytes the `script` artifact serves);
     - one sentence saying to return the COMPLETE revised DesignIntent and keep everything the request does not change;
     - `SELECTED FEATURE:` plus the focus, when given;
     - then the request.
   - Keep the section text in ONE constant, documented as a contract string.
   - The validation repair and the build repair carry the same base section.
   - The turn's result is a NEW design (new id). The base design is never mutated.
3. **Usage.** `ChatResponse.usage` is always present: `{input_tokens, output_tokens}` or `null`. It is the sum over every model call of the turn (the design call plus any repair).
   - **OpenAI/OpenRouter:** `usage.prompt_tokens` and `usage.completion_tokens`.
   - **Claude Code CLI JSON result:** `usage.input_tokens` + `cache_read_input_tokens` + `cache_creation_input_tokens` as input, and `output_tokens` as output.
   - **Codex `exec --json`:** the `turn.completed` (or `token_count`) event's input/output counts.
   - **OpenCode `run --format json`:** step-finish `tokens.input` / `tokens.output`.
   - A field the provider did not send stays null; never estimate. The `local` helper → `null`.
   - Parse defensively. A malformed usage must never fail a turn, and must never be logged with prompt text.
   - Test with FAKE CLIs and a mock HTTP server only.
4. **Journal.** The `chat.turn` event records `base_design_id` and `focus_chars`, never the focus text. It also records `usage` when reported. Keep the hash chain intact.
5. **Parity and corpus.**
   - If the Python reference mirrors the chat DTOs or responses that a differential checks, mirror the new optional fields so `python_differential.py`, `worker_differential.py` and `python_provider_dto_oracle.py` stay green with 0 gaps.
   - Regenerate the corpus with the hermetic generator if bodies change, and confirm `generate_corpus.py --check` and `self_check.py` pass.
6. **Docs.** Write a short `docs/adr/0012-edit-turns-and-usage.md` (status, context, decision, consequences; date 2026-10-21), and add one paragraph each to `docs/studio.md` and `docs/studio-providers.md`.

## Own
- `backend/crates/kala-studio/src/**` and the `kala-contracts` files the chat DTOs live in, with their tests.
- The Python reference files that mirror those DTOs (only where parity requires).
- `backend/compat/**` and `backend/fixtures/corpus/**` (regenerated only).
- The new ADR and the two docs paragraphs.
- Do NOT touch `studio/**`: sheet-shell adds the TypeScript side.

## Gates
- **Rust:** `cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` (with and without `--all-features`, as before).
- **Python:** `pytest` on the touched Python, plus the three differentials and the corpus `--check` / `self_check.py`.
- USER DATA rule in every shell. No commits. Clean up /tmp.

Write `REPORT.md`: the exact request and response shapes, error codes, the base-section constant text, usage mapping per provider (with the fixture each test uses), gate counts, and gaps (e.g. a CLI whose real usage shape you could not confirm).
