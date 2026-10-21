# build-repair — one repair round fed by the BUILD result, not only by validation

Date: 2026-10-20
Cell: `build-repair` (agent-providers wave)

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and
`CARGO_HOME=/home/v9i/.cargo` and then set fresh temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` before any
binary or Python entry point ran (`/tmp/opencode/kala_env.sh`, sourced in
every command). No credential file and no settings file was read. The live
Studio on port 8831 was never stopped, never restarted and never used by a
test: it was still listening (pid 2402398, `GET /api/studio/health` → 200)
after this cell finished. Every test used a mock transport, a mock provider
or an ephemeral port. The user's real data directory was never opened or
written.

## Change 1: the build-feedback repair

### What triggers it

In `remote_design_turn` (Rust) the turn now spends **at most one repair call
in total**, and the build may spend it only when the validation repair was
not needed:

| Situation | Repair? |
|---|---|
| the design built with `verified == false` (any failed check row) | **yes** |
| the builder refused the script for a script reason (`design_rejected`, `invalid_request`, …) | **yes** |
| the CAD provider is not available on this host (`provider_unavailable`) | no |
| the worker has no kernel (`design_unavailable`) | no |
| no `cad_provider` was selected | no |
| a backend error | no |
| the validation repair was already spent | no |
| the design built and verified | no |

`DesignFailure::is_script_refusal()` (Rust) is the single decision point:
it is true for every refusal except `provider_unavailable`, `design_unavailable`
and `design_not_found`. The brief only named `is_provider_unavailable()` as an
exclusion; `design_unavailable` (the native kernel is missing) and
`design_not_found` are also excluded here because neither is about the
model's script — repairing them would burn the turn's one repair call on a
host problem. This is a deliberate, documented widening of the exclusion list.

### The budget

`repair_spent` is set the moment the validation repair is used. The build
repair is only reached when `repair_spent == false`, so the worst case is
still **two model calls per turn** (first call + one repair). The repair call
carries the same 180 s budget as every other design call.

### The exact repair prompt template

Rust — `backend/crates/kala-studio/src/chat_design.rs`:

```rust
pub fn build_repair_prompt(prompt: &str, feedback: &str, schema_text: &str) -> String {
    let feedback: String = feedback
        .chars()
        .take(MAX_BUILD_REPAIR_FEEDBACK_CHARS)   // 2_000
        .collect();
    let schema: String = build_repair_schema_excerpt(schema_text);  // 700
    format!(
        "Your previous reply was built, but the build was rejected: {feedback}\n\n\
         Return exactly one fenced ```json block containing a corrected DesignIntent for the same \
         request. Fix the script so that every cut removes material and the part matches the \
         request, and keep every dimension the user stated: do not invent a value the user did \
         not state and do not change a value the user did state. If a required value is missing, \
         reply in prose with the missing values instead.\n\n\
         Schema for script:\n{schema}\n\nUser request:\n{prompt}"
    )
}
```

Python — `src/kala_agent/studio/providers/dispatch.py::_build_repair_prompt`
produces the byte-identical text (the prompt is not differentially compared,
but the wording is).

The `feedback` body is `BuildFeedback::text()` / `BuildFeedback.text()`:

```
the script you sent:
{compact JSON of the script, <= 6_000 chars}

the builder refused the script:
{design_rejected: the design could not be compiled or measured (unsupported by the Studio OCP compiler v1: pocket)}

the build reported:
- the check 'cuts_effective' (expected 1, measured 0)
- warning: component 'plate': pocket on XY removed no material
```

* The script is the compact JSON of the script the model sent (the built
  design's own canonical script when there is one, otherwise the validated
  intent's script), bounded to 6 000 characters.
* The failed check rows are rendered as `the check '<id>' (expected <e>,
  measured <m>)`; whole numbers lose their `.0` tail.
* The provider warnings are the worker's own warning strings, one per line,
  prefixed `- warning: `.
* The refusal is the worker's own error envelope summary.
* `Schema for script:` is the `script` property of the root DesignIntent
  schema, bounded to 700 characters (the whole schema is ~11 KB). The root
  schema summary is used when that property cannot be read.

The one-short-sentence reason that the reply text carries is
`BuildFeedback::reason()` / `BuildFeedback.reason()`:

* refusal → `the builder refused the script: <summary>`
* failed checks → `the build failed the check '<id>' (expected <e>, measured <m>)`
  plus `; the provider warned: <first warning>` when the build warned.

### What the turn returns

| Case | design | text |
|---|---|---|
| second attempt builds and verifies | the **second** design | `The first attempt was rejected: <reason>. <second prose> <second summary (B-Rep verified)>` |
| second attempt builds but is unverified again | the **second** design (`verified: false`) | `The first attempt was rejected: <reason>. <second prose> <second summary (B-Rep NOT verified)>` |
| second reply is invalid / has no design / is refused | the **latest built** design (the first one), `verified: false` visible | `The first attempt was rejected: <reason>. <first prose> <first summary> <why the repair failed>` |
| nothing ever built | none | `The first attempt was rejected: <reason>. No part was built: <why the repair failed>` |

The turn never claims a verification the returned design does not carry: the
summary is the existing house wording `B-Rep verified` / `B-Rep NOT verified`,
driven by the design's own `verified` flag.

Both builds are journaled naturally by the existing design path (the worker
journals every `design.*` event). The front door still appends exactly **one**
`chat.turn` per turn, pointing at the returned `design_id`.

### Files changed (Rust product)

- `backend/crates/kala-studio/src/chat_design.rs`
  * new public items: `BuildFeedback` (+ `text()`, `reason()`), `BuildOutcome`,
    `build_feedback()`, `build_repair_prompt()`,
    `MAX_BUILD_REPAIR_SCRIPT_CHARS` (6 000),
    `MAX_BUILD_REPAIR_FEEDBACK_CHARS` (2 000),
    `MAX_BUILD_REPAIR_SCHEMA_CHARS` (700),
    `DesignFailure::is_script_refusal()`.
  * new private helpers: `build_repair_schema_excerpt()`,
    `render_check_value()`, `compact_script()`, `repair_build()`,
    `keep_first_build()`, the `BuildRepair` context struct.
  * `remote_design_turn()` now tracks `repair_spent`, calls `finish_design()`
    for its `BuildOutcome` and may spend the one repair call on the build.
  * `finish_design()` now returns `Result<BuildOutcome, DesignTurnError>`
    instead of `Result<(), _>` (private function; `local_design_turn` ignores
    the value).
  * module doc: a new "The build-feedback repair" paragraph.
- `backend/crates/kala-studio/src/providers/remote.rs`
  * `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS`: **120 → 180**; the module doc,
    the constant's doc and the `build_client` doc now say 180 s.
- `backend/crates/kala-studio/tests/chat_mock_provider_design.rs`
  * 4 new integration tests + 1 renamed (see below).
- `backend/crates/kala-studio/src/providers/mod.rs`, `server.rs`,
  `schemas.rs`, `settings.rs`: **not touched by this cell** (shared files
  owned by the other cells of the wave; the 180 s budget flows through
  `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS` without an edit).

### Files changed (Python dev reference)

- `src/kala_agent/studio/providers/dispatch.py`
  * new `BuildFeedback` dataclass (`text()`, `reason()`), the
    `MAX_BUILD_REPAIR_*` bounds, `_build_repair_schema_excerpt()`,
    `_build_repair_prompt()`, `build_repair_turn()`, `build_feedback()`,
    `_compact_script()`, `_compact_script_text()`, `_render_check_value()`.
  * `StudioChatTurn` gained `design_repair_spent`; the three turns that spend
    the validation repair set it to `True`.
  * `__all__` extended.
- `src/kala_agent/studio/server.py`
  * the chat route's design branch now calls the new `_build_and_maybe_repair()`
    and the new `_keep_first_build()`; `build_feedback` and `build_repair_turn`
    are imported from the dispatch layer.
- `src/kala_agent/studio/providers/openai_compat.py`
  * `STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS`: **120.0 → 180.0**, plus the
    docstring and the constant's own doc.

### Files changed (tests)

- `backend/crates/kala-studio/tests/chat_mock_provider_design.rs`
  * `an_unverified_build_is_repaired_with_the_build_feedback` — the first reply
    builds unverified (a failed `cuts_effective` row from a pocket sketched
    entirely outside the plate), the second reply builds verified: the turn
    returns the second design, the text names the first rejection reason
    (`cuts_effective`, `removed no material`) and exactly **2** model calls
    were made, and the second prompt carries the failed check and the warning.
  * `a_spent_validation_repair_buys_no_build_repair` — the validation repair
    was spent, the repaired design is unverified: **2** calls in total and the
    text never says the first attempt was rejected.
  * `an_unavailable_cad_provider_buys_no_build_repair` — `cad_provider:
    "solidedge"` on Linux: **1** call, no repair.
  * `a_repair_that_is_still_unverified_never_claims_verified` — the returned
    design has `verified: false` and the text never contains `B-Rep verified`.
  * `design_turns_and_their_repair_carry_the_120_second_budget` →
    `..._180_second_budget`, asserting 180 s on both design calls and 30 s on
    a plain chat.
- `backend/crates/kala-studio/src/chat_design.rs` (unit tests)
  * `only_a_refusal_about_the_script_earns_a_build_repair`,
    `the_build_repair_prompt_carries_the_script_and_the_feedback`,
    `the_build_feedback_reports_the_failed_rows_and_the_warnings`,
    `a_verified_design_and_a_foreign_refusal_earn_no_build_repair`,
    `a_refused_script_carries_the_refusal_and_the_script`.
- `backend/crates/kala-studio/src/providers/remote.rs` (unit test)
  * `design_turn_requests_carry_120_seconds_plain_chat_carries_30` →
    `design_turn_requests_carry_180_seconds_plain_chat_carries_30`.
- `tests/unit/studio_design/test_design_api.py`
  * `test_an_unverified_build_is_repaired_with_the_build_feedback` (the
    equivalent of the Rust (a)),
  * `test_a_spent_validation_repair_buys_no_build_repair` (the equivalent of
    the Rust (b)),
  * `test_design_turn_uses_the_longer_remote_timeout` now asserts 180 s.

## Change 2: the design budget is 180 s

`STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS` is **180** in Rust
(`providers/remote.rs`) and **180.0** in Python (`openai_compat.py`), which
matches the 180 s budget the CLI providers already had
(`providers/cli_runtime.rs::DEFAULT_CLI_TIMEOUT_SECONDS`). The budget is still
chosen by the server, never by the request body: `server.rs` passes
`Duration::from_secs(STUDIO_DESIGN_REQUEST_TIMEOUT_SECONDS)` into the design
closure only, and `dispatch._remote_turn` / `dispatch.build_repair_turn` pass
the same constant for the first call and the one repair call. A plain chat
keeps 30 s.

The doc comments that said 120 s were updated (the `remote.rs` module doc,
the constant's doc, the `build_client` doc, the `openai_compat.py` module
docstring and the `openai_compat_chat` docstring).

## Tests and gates

| Gate | Result |
|---|---|
| `cargo fmt -p kala-studio --check` | **clean** |
| `cargo fmt --all --check` | **not clean** — the only diff is `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs`, the cut-guard worker's untracked file. Not this cell's file; it was left untouched on purpose. |
| `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings` | **clean** (0 warnings) |
| `cargo test -p kala-studio` | **240 passed, 0 failed, 0 ignored** (133 lib + 107 integration across 19 binaries) |
| `pytest tests/unit -q` | **1578 passed, 18 warnings** (this cell adds 2 of its own; the other +6 come from the concurrent cut-guard cell) |
| `ruff check` / `ruff format --check` on the 4 changed Python files | **clean** |
| `cargo build --release -p kala-studio` | **rebuilt** (11 570 848 bytes, 2026-10-20 17:56). The live process on 8831 kept running. |
| `backend/crates/kala-studio/tests/python_differential.py` | **before** 268 cases / **47 mismatches** → **after** 268 cases / **47 mismatches** |
| `backend/crates/kala-studio/tests/python_provider_dto_oracle.py` | **before** 20 cases / **0 failures** → **after** 20 cases / **0 failures** |

The "before" counts were measured with the release binary that existed before
this cell rebuilt it; the "after" counts with the rebuilt binary. The
mismatch **set is identical**: the 47 case ids extracted from the run are
33 `settings.*`, 2 `providers.*`, 3 `read.*`, 5 `origin.*`, 2 `host.*`,
1 `health.*` and 1 `chat.codex_unavailable` — exactly the categories the
`design-timeout` cell documented. All of them are the `cli-providers` cell's
Python-parity gap (the Python reference still publishes the four-row provider
list and the old Codex wording), not a regression here: no case in either
differential exercises a design turn, a build repair or the design budget.

New tests in this cell: 4 integration + 5 unit (Rust `chat_design.rs`) + 1
renamed unit (`remote.rs`) + 2 Python.

## Gaps / notes for the conductor

- **`cargo fmt --all --check` is not green**, but only because of
  `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs`, the
  cut-guard worker's new file. It is not in this cell's ownership and was
  deliberately not formatted. `cargo fmt -p kala-studio --check` is clean.
  The cut-guard cell should run `cargo fmt` before it finishes.
- **The refusal trigger is slightly wider than the brief's list.** The brief
  named only `is_provider_unavailable()` as a non-trigger. `design_unavailable`
  (no native kernel) and `design_not_found` are excluded as well, because
  neither is about the model's script. A hidden test that expects *every*
  non-provider-unavailable refusal to trigger a repair would disagree on
  those two codes.
- **"the text never says verified"** is implemented as "the text never says
  `B-Rep verified`". The honest summary of an unverified design is the
  existing house wording `B-Rep NOT verified`, which contains the substring
  `verified`; the Rust and Python tests assert `B-Rep verified` is absent and
  that `B-Rep NOT verified` is present.
- **The Python reference has no CLI providers**, so the CLI design-turn path
  (including its 180 s budget and its build repair) exists only in the Rust
  product. This is a pre-existing dispatch difference, not a divergence
  introduced here.
- **No live model call was made by this worker.** The transport and server
  tests use a scripted transport only; the conductor should rerun the
  OpenRouter `minimax/minimax-m3.1-flash:free` flange prompt against the
  rebuilt binary to confirm the 180 s budget and the build repair in the
  field. The flange case is the one this cell exists for: the model
  validates, the script builds, the cuts remove no material, the
  `cuts_effective` row fails, and the turn now spends its one repair call on
  the build feedback instead of returning the wrong part unchallenged.
- **Linux only**: the Windows target was not rebuilt or re-tested in this
  cell.
- **No commit, push or sudo** was performed. Nothing under
  `/home/v9i/.local/share/kala-agent/**` was read or written.

---

# Round 2 (Opus review of round 1, 2026-10-20)

Round 1 was accepted; the two defects the review named are fixed. Same
ownership, same gates, no commit.

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and
`CARGO_HOME=/home/v9i/.cargo` and then set fresh temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` before any
binary or Python entry point ran (`/tmp/opencode/kala_env.sh`, sourced in
every command). No credential file and no settings file was read. Nothing
under `/home/v9i/.local/share/kala-agent/**` was opened or written: the
differential probe and every test use their own temporary data directories
and ephemeral ports. The live Studio on port 8831 was never stopped and
never used by a test; it was still listening and healthy
(`GET /api/studio/health` → 200) when this round finished. (It was restarted
by another process at 18:14 with the rebuilt binary — this worker neither
started nor stopped it.)

## R1: a failed repair CALL no longer discards the first build

### What changed

The repair call's provider failure used to become a `DesignTurnError::Provider`
for the whole turn, so the first design — already compiled and journalled —
was thrown away and the turn answered an error envelope. A free OpenRouter
model that spends 150 s on the first call hits exactly this.

Rust — `backend/crates/kala-studio/src/chat_design.rs`:

* `repair_build()` no longer maps the repair call's failure into
  `DesignTurnError::Provider`. The failure now goes through the existing
  `keep_first_build()` path with the reason
  `the repair attempt failed: <envelope code>.`, and the turn returns 200
  with the first (unverified, or refused) state.
* new private `provider_envelope_code(&ProviderFailure) -> &'static str`.
  `server::provider_failure_code` already maps the same failures, but
  `server.rs` is a shared file owned by another cell of the wave, so the
  mapping is duplicated locally instead of edited there. The two agree:
  `provider_unavailable`, `provider_not_configured` / `missing_api_key`,
  `provider_timeout`, `provider_network_error`, `provider_protocol_error`,
  `provider_subprocess_error`, `internal_error`.
* the module doc gained the two facts of this round: a failed repair call
  keeps the first build, and the repair prompt carries the instruction.

Python — the mirror:

* `src/kala_agent/studio/providers/dispatch.py`: `build_repair_turn()` catches
  the provider errors of the repair call (`ProviderTimeoutError`,
  `ProviderNetworkError`, `ProviderProtocolError`, `ProviderSubprocessError`,
  `ProviderConfigurationError` — which also covers
  `ProviderUnavailableError` — and `TooLargeResponseError`) and returns a
  `StudioChatTurn` with no `design_intent` and the new field
  `design_repair_failure` set to `the repair attempt failed: <envelope code>.`.
  The exception no longer escapes the repair and aborts the turn.
* new `_provider_error_code(error)` in `dispatch.py`: the same mapping as the
  route's own error responses (`provider_timeout`, `provider_network_error`,
  `provider_protocol_error`, `provider_subprocess_error`,
  `provider_unavailable`, `provider_not_configured` / `missing_api_key`,
  `internal_error`).
* `StudioChatTurn` gained the `design_repair_failure` field (keyword-only,
  default `None`).
* `src/kala_agent/studio/server.py`: `_build_and_maybe_repair()` uses
  `repaired.design_repair_failure` as the reason when the repair produced no
  design, and keeps the first design and its `design_id`.

### What the turn returns now

| Case | design | text |
|---|---|---|
| the repair call times out, the first design built unverified | the **first** design (`verified: false`) | `The first attempt was rejected: <reason>. <first prose> <first summary (B-Rep NOT verified)> the repair attempt failed: provider_timeout.` |
| the repair call times out, the first script was refused | none | `The first attempt was rejected: the builder refused the script: … No part was built: the repair attempt failed: provider_timeout.` |

The turn never answers an error envelope for a failed repair call, the
journal still holds exactly **one** `chat.turn` per turn, and it points at
the first design. The turn never claims a verification the returned design
does not carry.

## R2: the build-repair prompt carries the instruction again

Every model call is stateless. The build-repair prompt carried the old
script, the feedback and a script-schema excerpt, but not
`DESIGN_INTENT_INSTRUCTION`, so the repair call no longer saw the supported
subset, the plane rule or the worked examples.

Rust — `build_repair_prompt()` now opens with `{DESIGN_INTENT_INSTRUCTION}\n\n`,
the same constant `design_prompt()` uses, unchanged. The rest of the prompt
is unchanged. The bound moved from the ad-hoc `4_000` of the round-1 test to
the new public constant `MAX_BUILD_REPAIR_PROMPT_CHARS = 9_000`, which bounds
the *fixed* part of the prompt; the user's own request is appended unchanged
and is not part of that bound, exactly like in the first design prompt.
The per-part bounds are unchanged: feedback ≤ 2 000, schema excerpt ≤ 700,
script ≤ 6 000 characters.

Python — `_build_repair_prompt()` now opens with
`f"{DESIGN_INTENT_INSTRUCTION}\n\n"`, and `MAX_BUILD_REPAIR_PROMPT_CHARS =
9_000` was added to `dispatch.py`.

### The exact prompt template (Rust and Python, byte-identical)

```
{DESIGN_INTENT_INSTRUCTION}

Your previous reply was built, but the build was rejected: {feedback}

Return exactly one fenced ```json block containing a corrected DesignIntent for the same request. Fix the script so that every cut removes material and the part matches the request, and keep every dimension the user stated: do not invent a value the user did not state and do not change a value the user did state. If a required value is missing, reply in prose with the missing values instead.

Schema for script:
{schema}

User request:
{prompt}
```

`{feedback}` and `{schema}` are the round-1 bodies, unchanged. The two
implementations were compared byte for byte on the same inputs (the real
DesignIntent schema, the same feedback, the same request): **3 884
characters, identical**. The instruction itself is 3 069 characters in both
and is the same constant the first call carries.

## Files changed in round 2

- `backend/crates/kala-studio/src/chat_design.rs`
  * `build_repair_prompt()`: prepends `DESIGN_INTENT_INSTRUCTION`; new doc.
  * `repair_build()`: a failed repair call keeps the first build.
  * new `provider_envelope_code()`.
  * new `MAX_BUILD_REPAIR_PROMPT_CHARS`.
  * module doc: one more paragraph.
- `backend/crates/kala-studio/tests/chat_mock_provider_design.rs`
  * 2 new integration tests, 1 extended.
- `src/kala_agent/studio/providers/dispatch.py`
  * `_build_repair_prompt()`: prepends `DESIGN_INTENT_INSTRUCTION`.
  * `build_repair_turn()`: catches the provider failure of the repair call.
  * new `_provider_error_code()`; new `MAX_BUILD_REPAIR_PROMPT_CHARS`;
    `StudioChatTurn` gained `design_repair_failure`; `__all__` extended.
- `src/kala_agent/studio/server.py`
  * `_build_and_maybe_repair()`: the reason of a failed repair call.
- `tests/unit/studio_design/test_design_api.py`
  * 2 new tests, 1 extended.
- **not touched:** `backend/crates/kala-studio/src/providers/remote.rs` (the
  180 s budget of round 1 stands, unchanged), `server.rs`, `providers/mod.rs`,
  `native_geometry/**`, `kala-design-intent/**`, `adapters/**`,
  `kala-cad-providers/**`, `src/kala_agent/studio/design/**`.

## Tests

Rust:

* `src/chat_design.rs` (unit)
  * `the_build_repair_prompt_still_carries_the_design_instruction` — the
    repair prompt opens with the same instruction the first call carried, and
    the rest of the prompt is unchanged.
  * `a_failed_repair_call_names_the_envelope_code` — every `ProviderFailure`
    maps to the code the route would have answered.
  * `the_build_repair_prompt_carries_the_script_and_the_feedback` — extended:
    the bound is now `MAX_BUILD_REPAIR_PROMPT_CHARS`.
* `tests/chat_mock_provider_design.rs` (integration)
  * `a_failed_repair_call_keeps_the_first_build` — the first reply builds
    unverified, the repair call times out: **200**, the first design is
    returned with `verified: false`, the text carries the first rejection
    reason (`cuts_effective`, `removed no material`) and
    `the repair attempt failed: provider_timeout`, the text never says
    `B-Rep verified`, the journal holds exactly one `chat.turn` pointing at
    the first design, and exactly **2** model calls were made.
  * `a_failed_repair_call_after_a_refusal_says_no_part_was_built` — the
    first script is refused, the repair call times out: **200**, no design,
    `No part was built: the repair attempt failed: provider_timeout.`
  * `an_unverified_build_is_repaired_with_the_build_feedback` — extended: the
    repair prompt must still carry the instruction.

Python — `tests/unit/studio_design/test_design_api.py`:

* `test_a_failed_repair_call_keeps_the_first_build` — the equivalent of the
  Rust case above (200, first design with `verified: false`, the first
  rejection reason, `the repair attempt failed: provider_timeout`, one
  `chat.turn` pointing at the first design).
* `test_a_failed_repair_call_after_a_refusal_says_no_part_was_built` — the
  equivalent of the Rust refusal case.
* `test_an_unverified_build_is_repaired_with_the_build_feedback` — extended:
  the repair prompt must still carry the instruction.

New tests in round 2: 2 Rust unit + 2 Rust integration + 2 Python.

## Gates

| Gate | Result |
|---|---|
| `cargo fmt -p kala-studio --check` | **clean** |
| `cargo fmt --all --check` | **not clean** — the only diffs are in `backend/crates/kala-cad-providers/tests/freecad_cut_guard.rs`, `backend/crates/kala-design-intent/src/script.rs` and `backend/crates/kala-design-intent/tests/design_contract.rs`: the cut-guard worker's files, not in this cell's ownership, deliberately not formatted. (Round 1 saw only the first of the three; the cut-guard cell has since touched two more.) |
| `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings` | **clean** (0 warnings) |
| `cargo test -p kala-studio` | **244 passed, 0 failed, 0 ignored** (135 lib + 109 integration across 20 binaries). Round 1 was 240 (133 + 107): +2 lib unit, +2 integration. |
| `pytest tests/unit -q` | **1580 passed, 18 warnings** (round 1 was 1578: +2) |
| `ruff check` / `ruff format --check` on the 3 changed Python files | **clean** |
| `cargo build --release -p kala-studio` | **rebuilt** (11 571 200 bytes, 2026-10-20 18:11). The live process on 8831 kept running and answered 200 afterwards. |
| `backend/crates/kala-studio/tests/python_differential.py` | **before** 268 cases / **47 mismatches** → **after** 268 cases / **47 mismatches**; the extracted mismatch id set is **identical** (`diff` of the two id lists is empty) |
| `backend/crates/kala-studio/tests/python_provider_dto_oracle.py` | **before** 20 cases / **0 failures** → **after** 20 cases / **0 failures** |

The "before" counts were measured with the release binary that round 1 had
built; the "after" counts with the binary this round rebuilt. The 47
mismatches are the same pre-existing ones (33 `settings.*`, 2 `providers.*`,
3 `read.*`, 5 `origin.*`, 2 `host.*`, 1 `health.*`, 1
`chat.codex_unavailable`) — the `cli-providers` cell's Python-parity gap, not
a regression here. No case in either differential exercises a design turn, a
build repair or the design budget.

## Gaps / notes for the conductor

- **The envelope-code mapping is duplicated.** `chat_design.rs` now carries a
  private `provider_envelope_code()` because `server::provider_failure_code`
  lives in `server.rs`, a shared file this cell does not own. The two agree
  today; a future change to the route's mapping must change both. When the
  wave's shared files are free again, the mapping belongs in
  `providers/mod.rs` as a method on `ProviderFailure`.
- **The build-repair prompt grew by 3 071 characters** (the instruction plus
  its separator). With the bounded feedback (2 000) and schema excerpt (700)
  the fixed part of the prompt stays under
  `MAX_BUILD_REPAIR_PROMPT_CHARS` (9 000); the measured prompt with the real
  schema and a full feedback body is 3 884 characters. The user's own request
  is appended unchanged and is not part of that bound, exactly like in the
  first design prompt.
- **`cargo fmt --all --check` is still not green**, now for three cut-guard
  files instead of one. None of them is in this cell's ownership.
- **The refused-first-build variant of R1 is covered too** (Rust and
  Python): a refused script plus a timing-out repair call answers 200 with
  `No part was built: the repair attempt failed: provider_timeout.` and no
  design.
- **No live model call was made by this worker.** The transport and server
  tests use a scripted transport only; the conductor should still rerun the
  OpenRouter `minimax/minimax-m3.1-flash:free` flange prompt against the
  rebuilt binary. The field case this round exists for is the one where the
  first call spends ~150 s: the part is built and journalled, the repair call
  times out, and the turn must still answer 200 with the first design and
  say that the repair attempt failed.
- **Linux only**: the Windows target was not rebuilt or re-tested in this
  round.
- **No commit, push or sudo** was performed. Nothing under
  `/home/v9i/.local/share/kala-agent/**` was read or written.
