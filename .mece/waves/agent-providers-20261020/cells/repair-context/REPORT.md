# Cell repair-context — the VALIDATION repair call gets the same context as the first call

Date: 2026-10-20
Cell: `repair-context` (agent-providers wave)

## USER DATA

Every shell exported `RUSTUP_HOME=/home/v9i/.rustup` and `CARGO_HOME=/home/v9i/.cargo`
and then set a fresh temporary `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`
and `XDG_DATA_HOME` before any binary or Python entry point ran
(`/tmp/opencode/kala_env.sh`, sourced in every command). No credential file and no
settings file was read; no credential was printed. Nothing under
`/home/v9i/.local/share/kala-agent/**` was opened, named in a command or written.
The live Studio on **port 8831** was never stopped, never restarted and never used
by a test: it was still listening and healthy (`GET /api/studio/health` → 200,
pid 2979623) after this cell finished. Every server this cell started used an
ephemeral port and a temporary data directory. No commit, push or sudo.

## The change

The validation repair prompt used to carry only the validation error, a schema
excerpt for the failing location and the user request. Every model call is
stateless, so the repair call saw neither `DESIGN_INTENT_INSTRUCTION` (the
supported subset, the plane rule, the worked examples), nor the full schema, nor
**its own previous JSON**. It re-authored the design blind — which is exactly the
Opus repro: the first reply failed validation, the repaired reply came back in a
different shape (`operation` instead of `op`, no `summary`, an invalid component
name) and the turn ended "No part was built".

### The final prompt template (Rust and Python, byte-identical)

```
{DESIGN_INTENT_INSTRUCTION}

DesignIntent JSON Schema:
{full DesignIntent JSON Schema}

The JSON block you sent:
{the model's own previous JSON block, extracted, bounded to 6 000 characters}

Your previous reply could not be validated as a DesignIntent: {validation error, bounded to 512 characters}

Schema for {the failing field}:
{schema excerpt for that field, bounded to 700 characters}

Return exactly one fenced ```json block containing a corrected DesignIntent for the same request. Keep every value the user stated and do not invent a value the user did not state. If a required value is missing, reply in prose with the missing values instead.

User request:
{user request, unchanged and unbounded, exactly like the first design prompt}
```

Rust — `backend/crates/kala-studio/src/chat_design.rs`:

```rust
pub fn repair_prompt(prompt: &str, error: &str, schema_text: &str, previous: &str) -> String {
    let error = bounded_error(error);
    let location = error_location(&error);
    let schema_section = design_schema_excerpt(schema_text, &error)
        .map(|excerpt| { /* "Schema for {location}:\n{excerpt}" */ })
        .unwrap_or_default();
    let previous_section = previous_design_json_section(previous);
    format!(
        "{DESIGN_INTENT_INSTRUCTION}\n\nDesignIntent JSON Schema:\n{schema_text}{previous_section}\n\nYour previous reply could not be validated as a DesignIntent: {error}{schema_section}\n\nReturn exactly one fenced ```json block containing a corrected DesignIntent for the same request. Keep every value the user stated and do not invent a value the user did not state. If a required value is missing, reply in prose with the missing values instead.\n\nUser request:\n{prompt}"
    )
}
```

Python — `src/kala_agent/studio/providers/dispatch.py::_repair_prompt` builds the
same bytes.

The order is the brief's order: instruction → full schema → the model's own
previous JSON → the validation error → the targeted schema excerpt → the
instruction to return a corrected DesignIntent that keeps every stated value →
the user request. The existing "If a required value is missing, reply in prose
with the missing values instead." sentence is kept verbatim.

### Sizes (the Opus flange repro, the real schema)

| Part | Characters |
|---|---|
| `DESIGN_INTENT_INSTRUCTION` | 3 549 |
| full DesignIntent JSON Schema | 11 291 |
| "The JSON block you sent:" section | 810 |
| the whole repair prompt | **15 680** |

Bounds: previous JSON ≤ `MAX_REPAIR_PREVIOUS_CHARS` = 6 000 (the same bound the
build-repair prompt uses for the script), validation error ≤
`MAX_REPAIR_ERROR_CHARS` = 512 (the bound the old prompt already used),
schema excerpt ≤ 700 (unchanged), the user's own request unbounded and appended
unchanged — exactly like the first design prompt.

### The model's own previous JSON

New, identical in both languages: `extract_design_json_block()` /
`_extract_design_json_block()`.

* The fenced blocks of the reply are scanned (the language tag is skipped, an
  unlabelled fence and a ```json fence behave the same, an unclosed fence runs
  to the end of the reply).
* The first fenced block that carries a balanced `{...}` object wins; the object
  is found with a string-aware brace scan, so a `{` inside a string does not
  open an object.
* A reply with no fenced object falls back to the first balanced object in the
  text outside the fences (a bare object in prose).
* A reply whose JSON is not a balanced object shows the model **its own text**
  under the header `Your previous reply:` instead of `The JSON block you sent:`,
  so the model always sees exactly what it wrote.

### The honest text names BOTH errors

When the turn ends with no part after the repair, the reason now names the
first error and the second error, each bounded to 512 characters:

```
the first reply was not a valid design (script: Field required), and the reply after one repair attempt was not a valid design either (template: Input should be a valid dictionary).
```

Rust: `validation_repair_failed_reason()` in `chat_design.rs`.
Python: `_validation_repair_failed_reason()` in `dispatch.py`.
The text is emitted through the unchanged `no_build_text()` / `_no_build_text()`
helper, so the turn still reads `… No part was built: the first reply was not a
valid design (…), and the reply after one repair attempt was not a valid design
either (…).`

## Byte-compare evidence

The cross-language comparison is a **test**, not a manual diff:
`backend/crates/kala-studio/tests/repair_prompt_parity.rs` asks the live Python
reference (through the test-only oracle `backend/crates/kala-studio/tests/repair_prompt_oracle.py`)
for its prompt on the same inputs and compares the bytes.

```
$ cargo test -p kala-studio --test repair_prompt_parity -- --nocapture
the validation-repair prompt is byte-identical in both languages: 6 cases, 99265 bytes compared
the build-repair prompt is byte-identical in both languages: 6 cases, 26267 bytes compared
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

| Comparison | Cases | Bytes compared | Result |
|---|---|---|---|
| validation-repair prompt, Rust vs Python | 6 | 99 265 | **identical** |
| build-repair prompt, Rust vs Python (regression guard) | 6 | 26 267 | **identical** |

The six cases cover: the Opus repro (`operation` instead of `op`, nested
location), a template with a negative width, a bare JSON object in prose, a
reply whose JSON is unbalanced (the "Your previous reply:" fallback), an error
that names no field (the "the failing field" header), and a 20 000-character
previous JSON (the 6 000-character bound). The schema used on both sides is the
live Python `design_intent_schema_text()`; it is byte-identical to the Rust
crate's `design_intent_schema_text()` (the live
`live_python_schema_is_byte_identical` differential).

**Two pre-existing divergences were found by this comparison and fixed** (they
were hidden before because the real schema has a `script` property, so neither
fallback was ever reached):

1. **The schema-section header.** Python derived the location from
   `error.split(": ", 1)[0]`, so an error that names no field produced the
   header `Schema for a design intent must carry exactly one of template or
   script:`; Rust produced `Schema for the failing field:`. Python now uses the
   same rule as Rust (a dotted identifier path only), through the new
   `_error_location()`.
2. **The root schema summary's key order.** Rust rendered the synthesised
   summary in insertion order (`type`, `required`, `properties`), Python's
   `sort_keys=True` renders `properties`, `required`, `type`. Rust's
   `root_schema_summary()` now renders the sorted order and sorts the property
   names too. This also touched the **build-repair** prompt's fallback excerpt,
   which is why the build-repair byte comparison is in the test as well.

## Tests

New in this cell:

| Kind | Test |
|---|---|
| Rust unit (`chat_design.rs`) | `the_repair_prompt_carries_the_instruction_the_schema_and_the_previous_json` |
| Rust unit | `the_repair_prompt_bounds_the_previous_json` |
| Rust unit | `the_previous_json_block_is_extracted_from_the_reply` |
| Rust unit | `the_honest_text_names_both_validation_errors` |
| Rust integration (new file) | `the_validation_repair_prompt_is_byte_identical_in_both_languages` |
| Rust integration (new file) | `the_build_repair_prompt_is_still_byte_identical_in_both_languages` |
| Python (`tests/unit/studio_design/test_design_api.py`) | `test_the_validation_repair_prompt_carries_the_instruction_the_schema_and_the_previous_json` |
| Python | `test_the_validation_repair_prompt_bounds_the_previous_json` |
| Python | `test_the_previous_json_block_is_extracted_from_the_reply` |
| Python | `test_the_validation_repair_text_names_both_errors` |

Extended (pinned expectations updated):

* Rust `chat_mock_provider_design.rs::an_invalid_reply_is_repaired_with_the_failing_field_schema`
  — the repair prompt must now also carry the instruction, the worked examples,
  the full schema and **the model's own previous JSON**.
* Rust `chat_mock_provider_design.rs::invalid_twice_says_no_part_was_built` —
  two *different* invalid replies, so the honest text must name both errors.
* Rust `chat_design.rs::the_repair_prompt_carries_the_error_and_the_failing_field_schema`
  — the new `previous` argument.

## Gates

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | **clean** |
| `cargo clippy -p kala-studio --all-targets -- -D warnings` | **clean** (0 warnings) |
| `cargo clippy -p kala-studio --all-targets --all-features -- -D warnings` | **clean** (0 warnings) |
| `cargo test -p kala-studio` | **253 passed, 0 failed, 0 ignored** (140 lib + 113 across 17 integration binaries). The polish cell's baseline was 247: **+6** (4 lib unit, 2 integration). |
| `pytest tests/unit -q` | **1623 passed, 18 warnings**. The polish cell's baseline was 1619: **+4**. |
| `ruff check` / `ruff format --check` on the 3 changed Python files | **clean** |
| `mypy --strict src/kala_agent/studio/providers/dispatch.py` | **clean** |
| `cargo build --release -p kala-studio` | **rebuilt** (`backend/target/release/kala-studio`, 11 591 552 bytes, 2026-10-20 19:34). The live process on 8831 kept running and answered 200 afterwards. |
| `generate_corpus.py --check` | **exit 0**, "determinism check passed" |
| `self_check.py` | **exit 0**, "self-check OK", 56 Studio cases / 15 routes, 49 core cases / 17 routes |

## Differentials

| Differential | Before | After | Mismatch ids |
|---|---|---|---|
| `python_differential.py` | 268 cases / **0** | 268 cases / **0** | none (empty set) |
| `worker_differential.py` | 292 cases / **0** | 292 cases / **0** | none (empty set) |
| `python_provider_dto_oracle.py` | 20 cases / **0 failures** | 20 cases / **0 failures** | none |

The "before" run used the release binary the polish cell had built (19:06) with
the **new** Python reference; the "after" run used the binary this cell rebuilt
(19:34). Both are 0/0/0, and the polish cell's documented baseline for the old
binary with the old Python is 0/0/0 as well, so the change moves no case. No
differential case exercises a validation repair (the probe has no remote
provider), which is why the change is invisible there — the byte comparison is
the evidence for this cell.

## Corpus and fixtures

The Python edit moved the corpus manifest's source pin, so the corpus was
regenerated with the documented command
(`.venv/bin/python backend/compat/generate_corpus.py`).

| | before | after |
|---|---|---|
| corpus SHA-256 (the payload files) | `61328f9143238ca85eeabc99d8ccd9c983ead0f1481ee491573e047ab52367ee` | `61328f9143238ca85eeabc99d8ccd9c983ead0f1481ee491573e047ab52367ee` (**unchanged**) |
| source pin `python_sources.tree_digest` | `656bb22ab31c4adee71cd182386336fbdcf6272404b62b8c53144ed21a742578` | `dcd136adab653ebc2ce52aac6883e8387e9b90844a4b80c6940f80a69396133a` |
| pinned product Python files | 98 | 98 |

The corpus digest did not move: the corpus replay does not exercise a
validation repair, so the payload files are byte-identical.
`backend/crates/kala-studio/tests/corpus_fixture.rs`'s `CORPUS_SHA256` and the
two README digest mentions therefore need no edit. Only
`backend/fixtures/corpus/v1/manifest.json` changed (the source pin and the
tracked-changes list).

## Files changed

| File | Change |
|---|---|
| `backend/crates/kala-studio/src/chat_design.rs` | the new `repair_prompt` (instruction + full schema + previous JSON + error + excerpt + instruction + request); `extract_design_json_block`, `fence_spans`, `first_object_span`, `outside_fences`, `previous_design_json_section`, `bounded_error`; `validation_repair_failed_reason`; `remote_design_turn` threads the previous reply into the repair and names both errors; `root_schema_summary` renders the sorted order; the module doc. |
| `backend/crates/kala-studio/tests/chat_mock_provider_design.rs` | 2 extended integration tests. |
| `backend/crates/kala-studio/tests/repair_prompt_parity.rs` | **new** — the cross-language byte comparison. |
| `backend/crates/kala-studio/tests/repair_prompt_oracle.py` | **new** — the test-only Python oracle. |
| `src/kala_agent/studio/providers/dispatch.py` | `_repair_prompt` (new shape), `_previous_design_json_section`, `_extract_design_json_block`, `_fence_spans`, `_first_object_span`, `_error_location`, `_validation_repair_failed_reason`, `MAX_REPAIR_PREVIOUS_CHARS`, `MAX_REPAIR_ERROR_CHARS`, `_remote_turn` threads the previous reply and names both errors, `__all__`. |
| `tests/unit/studio_design/test_design_api.py` | 4 new tests. |
| `backend/fixtures/corpus/v1/manifest.json` | regenerated (source pin only). |

**Not touched:** `providers/mod.rs`, `server.rs`, `schemas.rs`, `settings.rs`,
`providers/remote.rs`, `studio/design/intent.py`, `studio/server.py`, every
other crate, `adapters/**`, `studio/src/**`, the shared fixtures.

## Gaps / honest notes

1. **No live model call was made by this worker.** The repair prompt is proven
   by the outbound-request assertions, the unit tests and the byte comparison.
   The conductor should still rerun the OpenRouter
   `minimax/minimax-m3.1-flash:free` flange prompt against the rebuilt binary:
   the case this cell exists for is the one where the first reply fails
   validation and the repair used to come back in a different shape.
2. **The parity test skips loudly when no Python interpreter is available**
   (no `KALA_PYTHON`, no `.venv/bin/python`). A skip is not byte-compare
   evidence; in this environment it ran and compared 99 265 bytes. A Python
   interpreter that starts but cannot import the reference is a hard failure,
   not a skip.
3. **The "reply asks for values instead of including a design" path still names
   only that reason.** There is no second validation error in that path (the
   repair answered in prose), and the existing pinned expectation
   (`!text.contains("not a valid design")`) is kept. The first error is named
   only where a second error exists.
4. **The Python reference now renders the 11 KB schema on every validation
   repair** (`design_intent_schema_text()`), exactly as it already did for the
   first design prompt (`_design_prompt`). The Rust product fetches the schema
   once per turn from `intent/schema`.
5. **The generated packaging copies are stale.** `studio/backend-native/**` and
   `studio/release/linux-unpacked/resources/backend/**` still carry the old
   `dispatch.py` and the old binary. Both are gitignored build stages, not
   source; `studio/scripts/native-backend.cjs` regenerates them at packaging
   time. They were deliberately not hand-edited.
6. **The parity test depends on the live Python reference**, so
   `cargo test -p kala-studio` now needs a Python interpreter with the studio
   extras installed (the CI job installs them and exports `KALA_PYTHON`; the
   same pattern the `kala-design-intent` live differential already uses).
7. **Windows** was not built or tested in this cell.
8. **No commit** was made; the working tree also contains the other cells'
   changes.
