# Cell edit-turns — REPORT

Status: **complete**. Rust backend edit turns plus reported token usage
(ADR-0012). No commits. `studio/**` untouched.

## Files changed

**Product**
- `backend/crates/kala-studio/src/schemas.rs` — `ChatRequest` gains
  `base_design_id` / `focus`; `MAX_FOCUS_LENGTH`; `optional_plain_string`,
  `optional_focus`; `valid_design_id` moved here (one format check shared with
  the `/design/{id}/...` routes).
- `backend/crates/kala-studio/src/providers/usage.rs` *(new)* — `ChatUsage`
  (`{input_tokens, output_tokens}` of `Option<u64>`), `merge`, defensive
  `count`/`sum`.
- `backend/crates/kala-studio/src/providers/mod.rs` — `ChatOutcome.usage`,
  `ChatOutcome::with_usage`, re-export `ChatUsage`.
- `backend/crates/kala-studio/src/providers/remote.rs` — OpenAI/OpenRouter
  `usage.prompt_tokens` / `completion_tokens`.
- `backend/crates/kala-studio/src/providers/claude_cli.rs` — Claude result
  `usage.input_tokens + cache_read_input_tokens + cache_creation_input_tokens`
  / `output_tokens`.
- `backend/crates/kala-studio/src/providers/codex_cli.rs` — `turn.completed`
  `usage`, else the last `token_count` event (`info.total_token_usage`, in both
  the `token_count` and `event_msg`/`payload` spellings).
- `backend/crates/kala-studio/src/providers/opencode_cli.rs` — sum of every
  `step_finish` event's `part.tokens.input` / `.output`.
- `backend/crates/kala-studio/src/chat_design.rs` — `BaseDesign`,
  `EDIT_TURN_SECTION`, `SELECTED_FEATURE_LABEL`, `edit_turn_section`,
  `design_prompt_with_base`, `repair_prompt_with_base`,
  `build_repair_prompt_with_base`, `load_base_design` + `BaseDesignError` +
  `MAX_BASE_DESIGN_CHARS`, `DesignTurn.usage`, usage summed over calls,
  `ChatTurnRecord` journal payload.
- `backend/crates/kala-studio/src/server.rs` — edit-turn request checks, base
  design load, base passed to the turn, `usage` in `ChatResponse`, journal
  record, `design_not_found`.
- `src/kala_agent/studio/schemas.py` — `ChatUsage` + `ChatResponse.usage`
  (mirror only; the Python reference does not implement edit turns).
- `backend/fixtures/corpus/v1/{manifest,studio_http}.json` — regenerated (every
  chat success body now carries `"usage": null` when unreported).
- `docs/adr/0012-edit-turns-and-usage.md` *(new)*, one subsection in
  `docs/studio.md`, one section in `docs/studio-providers.md`.

**Tests**
- `backend/crates/kala-studio/tests/edit_turns.rs` *(new, 8 tests)* — route
  behaviour against a deterministic stub store + scripted transport.
- `backend/crates/kala-studio/tests/cli_providers.rs` — usage per CLI.
- `backend/crates/kala-studio/src/chat_design.rs` (unit) — section text,
  prompt order, repair parity, `load_base_design`.
- `backend/crates/kala-studio/src/schemas.rs` (unit) — focus/base validation.
- `backend/crates/kala-studio/tests/chat_and_remote.rs`,
  `tests/corpus_fixture.rs`, `tests/python_provider_dto_oracle.py` — body and
  corpus-digest updates only.

## Request shape

`POST /api/studio/chat`

```json
{
  "provider": "openrouter",
  "model": "minimax/minimax-m3.1-flash:free",
  "prompt": "make this hole 10 mm",
  "allow_remote": true,
  "cad_provider": "opencascade",
  "base_design_id": "9ec29435…f4bf8",
  "focus": "the 6 mm hole at x=8, y=8"
}
```

| Field | Rule |
|---|---|
| `base_design_id` | optional; trimmed; when present it must be a valid stored design id (64 lowercase hex, the same `schemas::valid_design_id` the `/design/{id}/...` routes use). |
| `focus` | optional; trimmed; ≤ 1024 chars; no control character except `\n`; whitespace-only is absent. |

The 4096-char `prompt` bound is unchanged and covers the user's text only.

## Response shape

```json
{
  "text": "…",
  "provider": "openrouter",
  "model": "mock-served-model",
  "offline": false,
  "design": { "design_id": "…", "…": "…" },
  "usage": { "input_tokens": 321, "output_tokens": 45 }
}
```

`usage` is always present. `null` means **no call of the turn reported a
count**; it never means zero tokens. A partial block keeps `null` for the field
the provider did not send, e.g. `{"input_tokens": 17, "output_tokens": null}`.
Field order is `…"design", "usage"` in both Rust and Python, so the live body
differential stays byte-identical.

## Error codes

Checked in this order, all **before** any model call and before the store read
(except the store lookup itself):

| Status | Code | When |
|---|---|---|
| 400 | `focus_requires_base_design` | `focus` sent without `base_design_id` |
| 400 | `edit_requires_design_turn` | `base_design_id` sent without `cad_provider` |
| 400 | `edit_requires_model` | `base_design_id` with `provider: "local"` |
| 404 | `design_not_found` | id is malformed (`not-a-design`, wrong length/case) **or** the store does not hold it |
| 422 | `invalid_request` | `focus` > 1024 (`string_too_long`), control character (`value_error`), or `base_design_id` not a string (`string_type`) |
| 502 | `base_design_unavailable` | the stored `script` artifact is empty, not UTF-8, not a JSON object, > 200 000 chars, or the store answered a non-404 error |
| 503 | `geometry_unavailable` | the geometry backend is not reachable while the base design is read |

A malformed `base_design_id` is `design_not_found` on purpose: the design
routes' own format check makes such a path a 404, and a UI that receives it
detaches the edit chip exactly as it does for a deleted design.

## The base-section contract string

`chat_design::EDIT_TURN_SECTION` (documented as a contract string) plus
`chat_design::SELECTED_FEATURE_LABEL`:

```text
CURRENT DESIGN:
<the stored canonical ConstructionScript JSON, the exact bytes the `script`
 artifact serves>

Return the COMPLETE revised DesignIntent and keep everything the request does not change.

SELECTED FEATURE:
<the focus, when the request named one>
```

Exact bytes:

```text
"CURRENT DESIGN:\n{design}\n\nReturn the COMPLETE revised DesignIntent and keep everything the request does not change."
```

`{design}` is replaced verbatim; when a focus is given `"\n\nSELECTED
FEATURE:\n" + focus` is appended. The first design call
(`design_prompt_with_base`), the validation repair
(`repair_prompt_with_base`) and the build repair
(`build_repair_prompt_with_base`) all carry the same block:

- first call: instruction → full schema → base block → `User request:`
- validation repair: instruction → full schema → base block → previous JSON →
  exact error → `User request:`
- build repair: instruction → feedback → script schema → base block →
  `User request:`

Without a base design all three produce byte-identical output to before
(pinned by `a_draft_prompt_is_byte_identical_to_the_unchanged_prompt` and by
`tests/repair_prompt_parity.rs`).

The base design is read with `GET /internal/v1/design/{id}/artifact/script`,
checked to be a non-empty JSON object, bounded to `MAX_BASE_DESIGN_CHARS =
200 000` (measured worst case: 90 497), and never truncated. The turn's result
is a new design id; the base design's artifacts are never written.

## Usage mapping and the fixture each test uses

| Provider | Input | Output | Test / fixture |
|---|---|---|---|
| `openai`, `openrouter` | `usage.prompt_tokens` | `usage.completion_tokens` | `tests/edit_turns.rs::an_edit_turn_carries_the_stored_script_and_returns_a_new_design` — scripted reply `{"usage":{"prompt_tokens":321,"completion_tokens":45}}`; `a_malformed_usage_block_never_fails_a_turn_and_is_never_a_number` covers string/negative/float/array/null blocks and a partial `{"prompt_tokens":17}` |
| `claude_code` | `usage.input_tokens` + `usage.cache_read_input_tokens` + `usage.cache_creation_input_tokens` | `usage.output_tokens` | `tests/cli_providers.rs::each_cli_reports_the_usage_it_actually_prints` — fake CLI printing `"usage":{"input_tokens":3,"cache_creation_input_tokens":1200,"cache_read_input_tokens":5000,"output_tokens":100}` → 6 203 / 100; `CLAUDE_RESULT` (no usage block) → `null` |
| `codex`, `codex_cli` | `turn.completed` → `usage.input_tokens`; else the last `token_count` total | the same event's `output_tokens` | `codex_cli_turn_uses_the_documented_argv_and_reads_the_jsonl` — `CODEX_EVENTS` `turn.completed` 1 / 2; `each_cli_reports_the_usage_it_actually_prints` — `event_msg` / `payload.info.total_token_usage` 100 / 20 (`cached_input_tokens` not added) |
| `opencode_cli` | Σ `step_finish` `part.tokens.input` | Σ `part.tokens.output` | `each_cli_reports_the_usage_it_actually_prints` — two `step_finish` events 11+5 / 7+3 → 16 / 10; `OPENCODE_EVENTS` (no step-finish) → `null` |
| `local` | — | — | `the_local_helper_reports_null_usage_and_adds_no_usage_to_the_journal` — `usage: null`, no `usage` key in the journal |
| design call + repair | sum of both calls | sum of both calls | `the_validation_repair_carries_the_same_base_section_and_its_usage` (11+13 / 4+6 = 24 / 10); `the_build_repair_carries_the_base_section_and_its_usage` (7+9 / 3+5 = 16 / 8) |

Parsing is defensive everywhere: a missing, malformed, non-integer or
non-finite block is "not reported", never a failed turn, and no usage value is
ever logged with prompt text.

## Journal

`chat.turn` keeps its existing keys and order. New keys, only when they say
something:

- `base_design_id` (string) and `focus_chars` (number, `0` when no focus) for
  an edit turn;
- `usage` `{input_tokens, output_tokens}` when a call reported a count.

Ordinary turns therefore gain no key, their payloads stay byte-identical to
the Python reference's, and the hash chain is untouched (the worker appends as
before). The focus text and the prompt text are never journalled
(`an_edit_turn_carries_the_stored_script_and_returns_a_new_design` asserts the
raw payload does not contain either).

## Parity and corpus

- `src/kala_agent/studio/schemas.py` mirrors `ChatResponse.usage`
  (always `null`: the Python reference reports no provider counts). The
  differential bodies match byte for byte.
- The Python reference deliberately does **not** accept `base_design_id` /
  `focus`: it has no edit-turn implementation, and accepting then ignoring
  them would draft a new part and call it an edit.
- Corpus regenerated with `backend/compat/generate_corpus.py` (only
  `manifest.json` and `studio_http.json` changed; the one chat success body
  gained `"usage": null`), `--check` and `self_check.py` pass.

## Gates (all run with the USER DATA isolation; no real CLI, no live model)

| Gate | Result |
|---|---|
| `cargo test --workspace` | **72 test binaries, 583 passed, 0 failed** |
| `cargo fmt --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo check -p kala-studio --no-default-features --lib` | clean |
| `pytest tests/unit` | 1667 passed |
| `pytest tests/integration` | 12 passed |
| `pytest tests/security` | 189 passed |
| `python_differential.py` | 268 cases, **0 mismatches** |
| `worker_differential.py` | 295 cases, **0 mismatches, 0 known gaps** |
| `python_provider_dto_oracle.py` | 20 cases, **0 failures** |
| `generate_corpus.py --check` | determinism check passed |
| `self_check.py` | `self-check OK` |

## Gaps

- **No real CLI or model was run** (hard rule). The Claude Code, Codex and
  OpenCode usage shapes come from their documented JSON event names and are
  covered only by fake CLIs. A CLI version that renames a usage field reports
  `null`; the studio never estimates a count.
- Codex accepts `turn.completed.usage`, `token_count` /
  `event_msg.payload.info.total_token_usage`, top-level `info`
  `total_token_usage` and direct `input_tokens`/`output_tokens` on a
  `token_count` event. `last_token_usage` is not used on purpose: a turn's
  multi-step total is the honest number, and `turn.completed` wins when both
  appear.
- OpenCode adds only `tokens.input` / `tokens.output`; `tokens.cache.*` is not
  added, because the brief names input/output only (it may be a subset of
  input, as Codex's cached count is).
- A failed repair **call** (timeout / network) carries no outcome, so its
  tokens are not counted; the turn reports only what a reply reported.
- A malformed `base_design_id` returns `design_not_found` (404) rather than a
  422; documented in the ADR and above.
- Python mirrors only the response field, not the request fields (deliberate,
  see Parity).

## Requests

None. The contract in WAVE.md ("Backend chat additions") is implemented as
written; the two defensive codes `base_design_unavailable` (502) and the
existing `geometry_unavailable` (503) only cover a broken store and are not
part of the UI's edit-turn vocabulary.
