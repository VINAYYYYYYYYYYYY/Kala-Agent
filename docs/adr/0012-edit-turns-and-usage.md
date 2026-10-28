# ADR-0012: Edit Turns and Reported Token Usage

| Field | Value |
|---|---|
| Status | **Accepted** |
| Date | 2026-10-21 |
| Amends | ADR-0009 (Studio design turns): a turn may revise a stored design instead of drafting from nothing. ADR-0010 (Rust control plane): the front door loads the base design from the store. |
| Scope | `POST /api/studio/chat` in the Rust `kala-studio` service; the Python reference mirrors only the response field. |

## Context

The drafting-sheet redesign lets the engineer click a projected edge or a hole
and type "make this hole 10 mm". By default the turn edits the design that is
active. Until now every design turn drafted from nothing: the model saw only
the user's text, and the 4 096-character prompt limit made it impossible for
the UI to paste the current script into the prompt. Copying the script into
the prompt would also have spent most of that budget on bytes the backend
already holds, and it would have let a client claim any script it liked.

The UI also shows the session's token use, so a chat response has to report
what the providers actually charged, or say plainly that nothing was reported.

Two different requirements therefore meet in one route:

1. **Carry the base design to the model** without widening the prompt or
   trusting client-supplied geometry.
2. **Report usage honestly.** The studio never estimates a count, and a
   malformed usage block must never fail a turn.

## Decision

### 1. Request

`POST /api/studio/chat` accepts two new optional fields:

| Field | Rule |
|---|---|
| `base_design_id` | A stored design id. It is checked with the same `valid_design_id` the `/design/{design_id}/...` routes use (64 lowercase hex), and the store must hold it. |
| `focus` | ≤ 1024 characters after trimming, and free of control characters other than newline. It names the feature the request is about. |

The 4 096-character `prompt` limit is unchanged and covers the user's text
only: the base design does not count against it.

The request is refused **before any model call**, in this order:

1. `focus` without `base_design_id` → `400 focus_requires_base_design`.
2. `base_design_id` without `cad_provider` → `400 edit_requires_design_turn`
   (an edit is a design turn; it needs a CAD target to build the revision).
3. `base_design_id` with the `local` provider → `400 edit_requires_model`
   (the local helper has no model to revise a design with).
4. A `base_design_id` that is not a valid id, or that the store does not
   hold → `404 design_not_found`.

A refused request never reaches the store, the worker or a provider.

### 2. The base design

The front door reads the stored **canonical script JSON** through the existing
`GET /design/{design_id}/artifact/script` route of the geometry backend: the
same bytes the `script` artifact download serves, never a re-encoding. The
artifact must be a non-empty JSON object; it is bounded to 200 000 characters
(the measured maximum canonical script is 90 497) and never truncated. A
stored design that cannot serve as a base is refused with
`502 base_design_unavailable` rather than sent to a model in part.

The edit prompt is the unchanged `DESIGN_INTENT_INSTRUCTION`, then the full
DesignIntent JSON Schema, then one new section, then the request:

```text
CURRENT DESIGN:
<the stored canonical ConstructionScript JSON>

Return the COMPLETE revised DesignIntent and keep everything the request does not change.

SELECTED FEATURE:
<the focus, when the request named one>
```

The section text is one contract string
(`chat_design::EDIT_TURN_SECTION`, plus `SELECTED_FEATURE_LABEL` for the
optional line). The first call, the validation repair and the build repair all
carry it byte for byte: every model call is stateless, so a repair without the
base design would revise a part it can no longer see.

The turn's result is a **new** design with a new id. The base design is never
mutated: the store keeps its script, STEP and native artifacts untouched.

### 3. Usage

`ChatResponse.usage` is always present:

```json
{"input_tokens": 321, "output_tokens": 45}
```

or `null` when no model call of the turn reported a count. It is the sum over
every model call of the turn — the design call plus the one repair call when
the turn spent it. A field the provider did not send stays `null`; the studio
never estimates, rounds or substitutes a zero.

| Provider | Input | Output |
|---|---|---|
| `openai`, `openrouter` | `usage.prompt_tokens` | `usage.completion_tokens` |
| `claude_code` | `usage.input_tokens` + `usage.cache_read_input_tokens` + `usage.cache_creation_input_tokens` | `usage.output_tokens` |
| `codex`, `codex_cli` | the `turn.completed` event's `usage.input_tokens`; otherwise the last `token_count` event's totals | the same event's `output_tokens` |
| `opencode_cli` | the sum of every `step_finish` event's `part.tokens.input` | the sum of their `part.tokens.output` |
| `local` | — | — (always `null`) |

Parsing is defensive: a missing, malformed, non-integer or non-finite block
means "not reported". A usage block never fails a turn and is never logged
with prompt text.

### 4. Journal

The `chat.turn` event records `base_design_id` and `focus_chars` (the length of
the focus, never its text) for an edit turn, and `usage` when a call reported
one. Ordinary turns gain no key, so their payloads stay byte-identical to the
Python reference's and the hash chain is unaffected. The prompt is journalled
as a digest and a length, exactly as before.

### 5. Python reference

The Python reference server mirrors `ChatResponse.usage` (always `null` there:
it reports no provider counts) so the live body differential stays
byte-identical. It does not implement edit turns, and therefore does not accept
the two request fields: silently accepting and ignoring them would draft a new
part and claim it was an edit.

## Consequences

- The UI can send an edit turn with the active design id and the selected
  feature, and receives the new design id in the same response shape as before
  plus `usage`.
- A prompt stays small: the base script rides in the prompt the backend builds,
  not in the client's 4 096 characters.
- The store is the single source of the base script. A client can name a
  design, but cannot supply or alter one.
- A provider whose real usage shape is unknown (a CLI version that changes its
  event names) reports `null` rather than a guess; the UI shows "— TOK".
- The differential oracles cover the response field, and the corpus is
  regenerated: every chat success body now carries `"usage": null` when nothing
  was reported.
