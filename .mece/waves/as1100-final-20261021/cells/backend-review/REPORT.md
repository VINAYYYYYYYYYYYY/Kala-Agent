# Backend final review — REPORT

Status: **complete** (read-only review; no product edits, no delegation, no commits).

Subject: the dirty backend edit-turn / usage changes of
`.mece/waves/as1100-sheet-20261021/cells/edit-turns/REPORT.md` (ADR-0012) and
the actual `git diff` of `backend/**`, `src/kala_agent/studio/schemas.py` and
the fixture files.

## Verdict

The implementation is **sound**: the request validation, the base-design
context, the cumulative usage accounting, the journal payload and the
client/server contract all match ADR-0012 and the wave contract, and every
gate I could run is green. No functional defect that breaks the edit-turn
contract was found.

Four defects were found, all in the **report/document layer rather than in
the product code**:

| # | Severity | Area | One line |
|---|---|---|---|
| F1 | Medium | report accuracy | The REPORT's stated reason for the 404 on a malformed `base_design_id` is factually wrong: the design routes answer **422**, not 404. |
| F2 | Medium | documentation | The corpus digest moved to `5914b484…` in this wave; **five** files still declare `39e0c0a9…` as "the value `corpus_fixture.rs` pins now". |
| F3 | Low | report accuracy | The REPORT's error table is not exhaustive: the base-design read can also answer **501** and **504**. |
| F4 | Low | usage accounting | A failed *validation-repair* call drops the first call's already-reported tokens; the *build-repair* failure path keeps them. The two repair paths disagree. |

---

## F1 — The REPORT's justification for the 404 is wrong (the design routes answer 422)

`.mece/waves/as1100-sheet-20261021/cells/edit-turns/REPORT.md:112-114`:

> A malformed `base_design_id` is `design_not_found` on purpose: the design
> routes' own format check makes such a path a 404, and a UI that receives it
> detaches the edit chip exactly as it does for a deleted design.

The *rule* (one 404 for both "malformed" and "not held") is deliberate and is
stated twice — ADR-0012 §1 rule 4 and the report's own "Gaps" bullet at
`cells/edit-turns/REPORT.md:237-238`. What is wrong is the stated **reason**:
the design routes do **not** answer 404 for a malformed id. They answer
**422 `invalid_request`** with the detail "the design id is not a sha256
digest":

- `backend/crates/kala-studio/src/server.rs:1381-1388` — the drawing and
  section routes: `if !valid_design_id(design_id) { return sorted(FlatError::new(422, "invalid_request", …)) }`
- `backend/crates/kala-studio/src/server.rs:1400-1408` — the artifact route
  (the very route the base design is read through): the same 422.

So the *decision* (one 404 for both "malformed" and "not held", ADR-0012 §1
rule 4) is documented and implemented consistently at `server.rs:1544-1546`
and `server.rs:1773-1781`; only the stated **reason** is false, and the same
malformed identifier yields **422 on `/design/{id}/…` and 404 on `/chat`**.

Impact: none for the shipped UI — `studio/src/api/client.ts:513` validates
the id with the identical `/^[0-9a-f]{64}$/` pattern before it leaves the
browser, so a malformed id cannot reach the route from the app. It is a
documentation/protocol-consistency defect, and the "a UI that receives it
detaches the edit chip" argument in the REPORT does not hold for a hand-made
client that would get 422 on the sibling route.

Suggested fix (documentation only): correct the rationale in
`cells/edit-turns/REPORT.md:112-114` (ADR-0012 §1 states the *rule*
correctly; only the report's prose is wrong) and state explicitly that
`/chat` answers 404 where `/design/{id}/…` answers 422 for the same
malformed identifier.

## F2 — The corpus digest documentation is stale in five files

This wave regenerated the corpus: the pinned digest moved from
`39e0c0a9…` to `5914b484af8dd705ee3fecea91262ad699ea70050ed7dd509e3da32f1812a999`
(`backend/crates/kala-studio/tests/corpus_fixture.rs:46`, and
`backend/fixtures/corpus/v1/manifest.json` → `corpus_sha256`).

Five files (seven statements) still state the old digest as the current one:

| File:line | Statement that is now false |
|---|---|
| `backend/fixtures/README.md:38` | "The current corpus digest is `39e0c0a9…`" |
| `backend/fixtures/README.md:197` | the `self_check.py` observation names `corpus_sha256 39e0c0a9…` |
| `backend/compat/README.md:51` | the digest table has no row for this regeneration (it is a per-regeneration table) |
| `backend/compat/README.md:55` | the narrative ends at `39e0c0a9…` |
| `docs/rust-studio.md:216` | "`39e0c0a9…`, the value `backend/crates/kala-studio/tests/corpus_fixture.rs` pins **now**" |
| `docs/rust-migration-status.md:521` | "`39e0c0a9…`, which `corpus_fixture.rs` pins **now**" |
| `docs/studio-limitations.md:917` | "`39e0c0a9…`, the value `backend/crates/kala-studio/tests/corpus_fixture.rs` pins **now**" |

A second, related statement in the same three docs is now false for the
first time: "`studio_http.json` is byte-identical across that regeneration,
so the replay result stands". This regeneration **did** change
`studio_http.json` (the one chat success body gained `"usage": null`, see
`backend/fixtures/corpus/v1/studio_http.json:2662-2700`).

I re-ran the replay, so the *result* stands and only the reasoning is stale:
`replay_binary_corpus.py` → **59 cases, 0 mismatches** (exact 51, fields 4,
step structural 2, skipped 2). The three docs also still say "56 cases"
where the corpus has 59 — that part was already stale before this wave.

## F3 — The REPORT's error table is not exhaustive

`cells/edit-turns/REPORT.md:100-110` presents the error table as the complete
set for an edit turn. The base-design read can also fail with:

- **501 `geometry_route_unavailable`** — `BaseDesignError::Backend` wrapping
  `BackendError::Unsupported` (`backend/crates/kala-studio/src/chat_design.rs:1407-1418`
  and `:1432-1435`) mapped by `backend/crates/kala-studio/src/server.rs:1490-1495`.
  This is the reachable case for any backend build that does not serve the
  `artifact` route (the trait's default arm,
  `backend/crates/kala-studio/src/geometry_backend.rs:194-200`).
- **504 `geometry_worker_timeout`** — `BackendError::Timeout`,
  `server.rs:1477-1483`.

Only 503 `geometry_unavailable` is listed. A client that switches on the
documented codes will not recognise 501/504. The UI does not need them
(`studio/src/state/chatTurn.ts:171-181` only treats the exact
`design_not_found`/404 and `base_design_unavailable`/502 pairs as a stale
edit base, which is correct), but the table should say it is the *shape*
refusals plus the base-load outcomes, not the whole vocabulary.

## F4 — A failed validation-repair call drops the first call's reported tokens

The cumulative usage is correct on every path that ends in a 200, but not on
the one path that ends in a provider failure:

- `backend/crates/kala-studio/src/chat_design.rs:1696-1705` — the validation
  repair call is `call(repair_prompt_with_base(…)).await.map_err(DesignTurnError::Provider)?`.
  `DesignTurnError::Provider(ProviderFailure)` carries no usage, so the
  first call's already-reported `usage` (held in the local `usage` variable,
  set at `chat_design.rs:1671`) is dropped with the error.
- `backend/crates/kala-studio/src/server.rs:1668-1683` — the failure branch
  journals `usage: crate::providers::ChatUsage::NONE` and returns the
  provider error envelope (which has no body field for usage).

The *build*-repair failure path behaves differently and **keeps** the first
call's counts: `chat_design.rs:1817-1823` leaves `turn.usage` untouched, the
turn still answers 200, and the merge at `chat_design.rs:1827` adds the
repair's counts when the repair succeeds.

ADR-0012 §3 says the reported usage "is the sum over every model call of the
turn". A turn whose first call reported 321/45 and whose repair then timed
out reports nothing at all, while the same failure in the build-repair path
reports 321/45. The cells' own REPORT acknowledges the gap
(`cells/edit-turns/REPORT.md:234-236`: "A failed repair **call** … carries no
outcome, so its tokens are not counted"), but the wording does not cover the
fact that the *first* call's tokens are lost with it, and it does not
mention that the build-repair path behaves differently.

Severity is low: no wrong number is ever reported, only a missing one, and
only on a turn that failed anyway. A fix would mean carrying the partial
usage in `DesignTurnError` (or in the journal record) — a product change I
did not make.

---

## What I verified as correct

**Validation** (`backend/crates/kala-studio/src/schemas.rs`)
- `valid_design_id` (`:223`) is byte-for-byte the check the server used
  inline before; it is now shared by the front door and the design routes
  (one format check, as claimed).
- `optional_plain_string` (`:535`) and `optional_focus` (`:563`): trimmed,
  empty ⇒ absent, non-string ⇒ `string_type`, >1024 **characters** ⇒
  `string_too_long`, control characters other than `\n` ⇒ `value_error`.
  `strip` uses `trim_matches(char::is_whitespace)`, so a whitespace-only focus
  is absent, and `\r`/NUL/C1 controls are rejected (`char::is_control`).
- `validate_chat` (`schemas.rs:904-959`) admits the two new fields and keeps
  the existing extra-field rejection; the 422 detail order is deterministic
  (push order, not map order).
- The four shape refusals run in the documented order and **before** the
  settings read, the store read and any model call:
  `server.rs:1510-1547` (`focus_requires_base_design` →
  `edit_requires_design_turn` → `edit_requires_model` → `design_not_found`).
  `tests/edit_turns.rs::an_edit_turn_without_a_cad_target_or_a_model_is_refused_before_any_call`
  asserts that no store call, no provider call and no journal row happened.
- The `base_design_id` is length-unbounded at validation but is bounded by
  the shared body limit, and every echo of it is truncated to 64 characters
  (`crate::errors::truncate_chars`, `server.rs:1529`, `:1540`, `:1780`).

**Base-design context** (`backend/crates/kala-studio/src/chat_design.rs`)
- `EDIT_TURN_SECTION` (`:306`) and `SELECTED_FEATURE_LABEL` (`:312`) are the
  documented contract strings; `edit_turn_section` (`:319`) appends the focus
  block verbatim and never leaves the `{design}` placeholder in the text.
- `design_prompt_with_base` (`:337`), `repair_prompt_with_base` (`:383`) and
  `build_repair_prompt_with_base` (`:693`) put the base block in the same
  relative position (after the schema, before the request) and are
  **byte-identical to the previous prompts when there is no base**
  (`a_draft_prompt_is_byte_identical_to_the_unchanged_prompt`), which the
  cross-language oracle `tests/repair_prompt_parity.rs` also re-checks
  against the Python reference (3/3 green, and it fails rather than skips
  when the oracle cannot run).
- `load_base_design` (`:1427`) reads the **same bytes the `script` artifact
  serves** (the call at `:1432-1435`) — the worker's artifact route serves
  `canonical_script_json(&resolved.script)`
  (`backend/crates/kala-studio/src/native_geometry/service.rs:509-512`,
  `:709`) — checks non-empty, UTF-8, ≤ 200 000 characters
  (`MAX_BASE_DESIGN_CHARS`, `:1403`, matching the Python reference's
  `MAX_REPAIR_PREVIOUS_CHARS`/`MAX_BUILD_REPAIR_SCRIPT_CHARS` of 200 000 and
  the measured 90 497-character worst case in
  `src/kala_agent/studio/providers/dispatch.py:129`) and a JSON **object**,
  and never truncates.
- The base design is never mutated: the turn's result is a new design built
  from the model's own script; `tests/edit_turns.rs` asserts the stored base
  script is byte-identical after the edit.
- No new `unwrap`/`expect`/`panic!` in product code (only in tests).

**Cumulative usage** (`backend/crates/kala-studio/src/providers/usage.rs`)
- `ChatUsage` is `Option<u64>` per field; `merge` sums what both calls
  reported and keeps what one reported, with a saturating add
  (`usage.rs:39-52`); a hostile `u64::MAX` stays a reported number
  (`usage.rs:111-120`).
- `count` accepts only a real non-negative integer. Under
  `serde_json/arbitrary_precision` (enabled in
  `backend/crates/kala-studio/Cargo.toml:68`) a JSON number is stored as its
  raw text, so `12` parses and `12.0`, `-1`, `"12"`, `1e3` do not — the
  defensive behaviour the report claims.
- The provider mappings match ADR-0012: OpenAI/OpenRouter
  `prompt_tokens`/`completion_tokens` (`providers/remote.rs:628-635`),
  Claude Code `input+cache_read+cache_creation` / `output_tokens`
  (`providers/claude_cli.rs:221-233`), Codex `turn.completed` first and the
  last `token_count` otherwise (`providers/codex_cli.rs:222-231`,
  `:250-276`), OpenCode the sum over every `step_finish`
  (`providers/opencode_cli.rs:243-248`, `:278-288`).
- The turn sum is right: design call + validation repair
  (`chat_design.rs:1671`, `:1705`), design call + build repair
  (`chat_design.rs:1827`), and the route's final
  `turn.usage.merge(outcome.usage)` (`server.rs:1736`). The design path
  rebuilds the outcome with `ChatOutcome::new` (no usage), so nothing is
  counted twice, and the plain-chat path takes the provider's counts from
  the outcome. (See F4 for the one failing path.)
- `usage_json` (`server.rs:1826-1835`) always emits the field: `null` when
  nothing was reported, a partial block keeps `null` for the missing side.
  The Rust field order (`text, provider, model, offline, design, usage`)
  matches `ChatResponse` in `src/kala_agent/studio/schemas.py:416-431`.

**Journal semantics** (`backend/crates/kala-studio/src/chat_design.rs`)
- `ChatTurnRecord` (`:1473`) + `append_chat_turn` (`:1506`) keep the existing
  key order and append the new keys last: `base_design_id` + `focus_chars`
  only for an edit turn, `usage` only when something was reported
  (`:1564-1576`). `serde_json` runs with `preserve_order`
  (`backend/crates/kala-studio/Cargo.toml:68`), so the insertion order really
  is the payload order.
- The prompt is journalled as a digest + a length, and the focus only as a
  length; the text never reaches the payload. The journal append is
  free-form (the worker's `journal_append` takes any JSON object —
  `backend/crates/kala-studio/src/native_geometry/mod.rs:518-560`), so the
  new keys do not break the append, and the chain digest is the worker's, so
  the hash chain is unaffected.
- The Python reference's `chat.turn` payload is unchanged, so the ordinary
  payload stays byte-identical — confirmed by the live worker differential
  (295 cases, 0 mismatches).

**Client/server protocol**
- `studio/src/api/client.ts:442` (`chatRequestBody`) sends the two fields
  only when set, validates the id with the same hex-64 pattern as the
  backend and bounds the focus to `CHAT_FOCUS_MAX_CHARS = 1024`
  (`studio/src/types/studio.ts:400`).
- `studio/src/api/validators.ts:688` (`parseChatUsage`) treats an absent,
  null or invalid usage as "not reported" and never as a zero.
- The Python reference deliberately does **not** accept the two request
  fields (`src/kala_agent/studio/schemas.py:270-293`, `extra="forbid"`); it
  mirrors only the response field, and the live body differential stays
  byte-identical (268 cases, 0 mismatches).

---

## Gates run (all with isolated `HOME`/data dirs; no real credential, no remote API, no real CLI)

Environment: `HOME=/tmp/opencode/kala-review/home`,
`KALA_AGENT_DATA_DIR=/tmp/opencode/kala-review/data/kala-agent`,
`KALA_STUDIO_DATA_DIR=/tmp/opencode/kala-review/data/kala-studio`,
`XDG_DATA_HOME=/tmp/opencode/kala-review/xdg`; the real
`CARGO_HOME=/home/v9i/.cargo` and `RUSTUP_HOME=/home/v9i/.rustup` were
preserved so the toolchain resolves. No release build was produced.

| Gate | Result |
|---|---|
| `cargo test --workspace --offline` | **583 passed, 0 failed** (61 binaries) |
| `cargo test -p kala-studio --offline` | **309 passed, 0 failed** (22 binaries): lib unit **169**, `edit_turns` **8**, `chat_and_remote` **7**, `cli_providers` **24**, `chat_with_design` 5, `chat_mock_provider_design` 15, `repair_prompt_parity` 3 (cross-language byte compare, not skipped), `corpus_fixture` 2, … |
| `cargo fmt --all --check` | clean |
| `cargo clippy -p kala-studio --all-targets -- -D warnings` | clean |
| `cargo check -p kala-studio --no-default-features --lib` | clean |
| `pytest tests/unit` | **1667 passed** |
| `pytest tests/integration` | **12 passed** |
| `pytest tests/security` | **189 passed** |
| `generate_corpus.py --check` | `determinism check passed` (corpus `5914b484…`) |
| `self_check.py` | exit 0, `self-check OK`, `corpus_sha256 5914b484…`, 59 Studio cases / 16 routes, 49 core cases / 17 routes |
| `python_differential.py` | **268 cases, 0 mismatches** (run against a locally built **debug** binary via `--binary`, so the parent's release packaging was not touched) |
| `worker_differential.py` | **295 cases, 0 mismatches, 0 known gaps** (same debug binary) |
| `replay_binary_corpus.py` | **59 cases, 0 mismatches** (exact 51, fields 4, step structural 2, skipped 2) — **not listed in the cell's own gate table**; I ran it because the corpus fixture changed |
| `python_provider_dto_oracle.py` | **20 cases, 0 failures** |

The `edit_turns` binary (8/8 green) covers: the three shape refusals with no
store/provider/journal side effect, the malformed and unknown id → 404, the
edit turn carrying the stored script and returning a new design (with the
prompt order asserted byte-wise and the base design proven unmutated), the
validation repair, the build repair, the malformed/partial usage blocks, the
local helper's `null` usage, and the focus bounds.

## Blockers / notes for the parent

- **No blocker.** The backend diff needs no product change from this review.
- **F2 is a documentation change the parent may want to fold in** (five
  files, one digest). It is outside every cell's owned file list, so it needs
  an owner.
- **Corpus sensitivity for the packaging step:** the corpus manifest compares
  `source_state.python_sources` (a per-file digest of every product Python
  source, `backend/compat/corpuslib/common.py:225-243`). Any change under
  `src/kala_agent/**` during packaging makes `generate_corpus.py --check`
  fail and forces a regeneration; a change under `studio/**` does not
  (`source_state.git` is excluded from the comparison,
  `backend/compat/generate_corpus.py:60-67`).
- **The release binary is not required by any gate I ran**; I used a debug
  binary so the parent's release packaging stays uncontended.
- The `edit-recovery` cell's fix (the UI detaches the edit chip on
  `design_not_found`/404 and `base_design_unavailable`/502) is consistent
  with the backend codes; the two codes the backend can also emit for a
  base read (501/504, F3) are correctly *not* treated as a stale base.

## Not done (out of scope or blocked by the brief)

- No `cargo build --release` (the brief reserves it for the parent).
- No real `claude`/`codex`/`opencode` CLI was run, no live model was called,
  no credential file was read: the CLI usage shapes are covered only by the
  fake CLIs in `tests/cli_providers.rs` (as the cell's own "Gaps" section
  states).
- The UI and packaging waves are the parent's; I read
  `studio/src/api/**` only to check the wire contract with the backend.
