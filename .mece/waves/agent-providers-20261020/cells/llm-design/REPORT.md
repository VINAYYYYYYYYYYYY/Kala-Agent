# llm-design — model-authored ConstructionScripts, robust extraction, honest no-build

Date: 2026-10-20
Cell: `llm-design` (agent-providers wave)

## USER DATA

Every shell session exported `RUSTUP_HOME`/`CARGO_HOME` and set `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` to fresh
temporary directories before running any binary or Python entry point.
Explicit `--data-dir` values were passed to every launched Studio. The new
integration test builds in a `tempfile` directory with a synthetic
`OPENROUTER_API_KEY`; no credential file and no settings file was read. The
user's real data directory was never opened, written, or named in a shell.

## What changed

The old instruction allowed a raw `ConstructionScript` only when the user had
already supplied a complete script, so a model could never build the 80 × 40 ×
8 mm plate in the wave finding. The new contract makes the model a planner:

1. **Template first, script otherwise.** If one of `l_bracket`,
   `stepped_shaft` or `housing_envelope` matches and every value is stated, the
   model uses the template. Otherwise it MUST author a `ConstructionScript`
   from the dimensions the user stated, using the measured subset: rectangle or
   circle sketch on `XY`/`XZ`/`YZ` with optional `x`/`y`; `extrude`;
   same-plane `pocket` (omit `length` for through-all, give it for a finite
   depth); rectangle `revolve` on `XY`/`YZ`; `place`; `fuse`. It may compute
   coordinates from stated positions, but never invent a dimension, material,
   tolerance or label.
2. **Three worked examples** in the instruction: a plate with four through
   holes, a stepped round post made by two rectangle revolves, and a two-part
   placed assembly. Each example validates against the schema and compiles on
   the real OCP kernel; the plate is also built by real FreeCAD.
3. **No measurements from the model.** The instruction forbids stating volume,
   surface area or envelope; only the kernel measures.
4. **Missing values stay a question.** If a value is missing, the model asks in
   prose and emits no JSON. A prose design answer is not repaired and not
   turned into a fake design.
5. **Robust extraction.** A reply is accepted when it carries a fenced block
   (```json, unlabelled, or any language tag), a bare JSON object, or a bare
   object wrapped in prose. Balanced `{...}` scanning respects strings and
   escapes. Identical blocks agree; two conflicting design objects are refused
   instead of guessed. The prose shown to the user has fences and valid bare
   design objects removed.
6. **One informative repair round-trip.** On a validation failure the repair
   prompt carries the exact error (bounded to 512 characters) and a bounded
   schema excerpt for the failing field (700 characters), including the
   discriminator path (`template.l_bracket.width`,
   `script.components.0.steps.0.op`). No second repair is allowed.
7. **Honest no-build text.** A reply with no design object ends with
   `No part was built: the reply did not include a design.`; a reply that still
   fails after repair ends with
   `No part was built: the reply after one repair attempt was not a valid design (<error>).`
   A repair answer that asks for values is presented as a question, not as an
   error. No measured number is substituted in either case.
8. **Non-template design turns reach the model.** `/api/studio/chat` treats a
   remote provider with an explicit `cad_provider` as a design turn even when
   the strict template parser finds no family. The local helper remains
   template-only; no provider is selected automatically and there is no
   fallback.

## Contract and parity

`DESIGN_INTENT_INSTRUCTION` is now 3069 bytes and byte-identical in:

- `backend/crates/kala-studio/src/chat_design.rs`
- `backend/crates/kala-design-intent/src/intent.rs`
- `src/kala_agent/studio/design/intent.py`

The live `live_python_schema_is_byte_identical` differential asserts this
byte equality on every run, and `live_python_reply_parse_differential`
compares the Rust and Python extraction/validation results and messages over
the reply corpus (including the new bare-with-prose and conflicting-block
cases).

`kala-design-intent` is an accepted standalone crate, but it owns the
`DesignIntent` parser and the instruction, so leaving it untouched would have
made the Rust product's `intent/validate` route keep the old behaviour and
would have broken the live instruction differential. The edit there is the
shared contract, not a second implementation.

## Changed paths

Product (this cell):

- `backend/crates/kala-design-intent/src/intent.rs` — instruction, fenced/bare
  extraction, conflict refusal, stripping.
- `backend/crates/kala-studio/src/chat_design.rs` — instruction, repair prompt
  with schema excerpt, no-candidate/no-build flow, repair-question flow.
- `backend/crates/kala-studio/src/server.rs` — **minimal shared-file hunk**:
  remote + explicit `cad_provider` counts as a design turn. The same file also
  carries the model-catalog cell's route wiring; that is not this cell's edit.
- `src/kala_agent/studio/design/intent.py` — instruction, extraction, schema
  excerpt.
- `src/kala_agent/studio/providers/dispatch.py` — design gate, repair prompt,
  honest no-build, question handling.
- `src/kala_agent/studio/server.py` — one line: forward `cad_provider` into
  `dispatch.chat` (Python dev reference parity).

Tests:

- `backend/crates/kala-studio/tests/chat_mock_provider_design.rs` (new) —
  scripted OpenRouter-compatible transport, real native backend.
- `backend/crates/kala-design-intent/tests/design_contract.rs`
- `backend/crates/kala-design-intent/tests/python_differential.rs`
- `tests/unit/studio_design/test_intent.py`
- `tests/unit/studio_design/test_design_api.py`

No credential file, settings file, `studio/**`, adapter, or other cell's source
was edited.

## Tests and evidence

- `kala-design-intent`
  - `cargo fmt --check`: clean.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: clean.
  - `cargo test --locked`: **56 passed** (8 lib + 42 contract + 6 live
    Python differential), debug.
  - `cargo test --locked --release`: **56 passed**.
  - `cargo check --target x86_64-pc-windows-gnu`: passes.
- `kala-studio`
  - `rustfmt --check src/chat_design.rs` and the new test file: clean.
    Whole-crate `cargo fmt --check` currently flags only the concurrent
    `cli-providers` import ordering in `codex_cli.rs`/`opencode_cli.rs`.
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`:
    clean on the current tree.
  - `cargo test --locked --lib`: **123 passed**.
  - `cargo test --locked --test chat_mock_provider_design`: **6 passed** in
    debug and **6 passed** in release.
  - `cargo test --locked --test chat_with_design`: **5 passed**.
  - Whole-suite run after the concurrent `cli-providers` edits landed:
    **203 collected, 201 pass, 2 fail outside this cell**.
    `chat_and_remote::codex_stays_fail_closed...` expects the old
    `provider=codex` wording but now gets `provider=codex_cli`, and
    `corpus_fixture::replay_all_56_studio_cases` expects the old four-row
    provider list. Both are consequences of the `cli-providers` cell's
    in-flight rename/additions, not of this cell's files. With the
    `chat_and_remote` case skipped, every other integration suite passes.
- Worker differential (`tests/worker_differential.py`, release binary):
  **292 cases, 0 mismatches, 0 known gaps** before the `cli-providers` cell
  added its subscription rows. After those rows landed the same run reports
  **58 mismatches, all in `settings.*`, `providers.*`, `health.*`,
  `read.*`, `origin.*`, `host.*` and the codex wording**; every `design.*`
  case still matches, so the design route differential remains at 0.
- Python
  - `python -m pytest -q tests/unit`: **1567 passed**.
  - `ruff check` and `ruff format --check` on every changed Python file: clean.
  - `mypy` could not run in this environment: the installed numpy stub uses a
    Python 3.12 `type` statement while the project config pins
    `python_version = 3.11`, so mypy exits before checking the project. This is
    a pre-existing toolchain mismatch, not a source error.
- Cross-check: `kala-studio` for `x86_64-pc-windows-gnu` cannot link here
  because the Windows OCCT headers are absent and `x86_64-w64-mingw32-gcc` is
  not installed. `kala-design-intent` cross-checks cleanly.

### Real builds

`chat_mock_provider_design.rs` sends the wave's plate prompt through the real
`/api/studio/chat` route with a scripted provider reply and the real native
backend:

- `opencascade`: `verified: true`, volume `24695.221316` mm³, area
  `8696.991118` mm², `solid_count: 1`.
- `freecad` (real `FreeCADCmd` 1.1.4 on this host): same volume and area,
  `measured_by: "FreeCAD"`, `verified: true`.

The scripted reply is bare JSON (no fence), so the same test also proves the
bare-object extraction path end to end. The test checks that the outbound
provider prompt contains `When no template matches, author a ConstructionScript`, the schema,
and the user's dimensions.

## Compatibility and security

- No provider is selected automatically and there is no fallback; the ADR-0011
  pass-through is unchanged.
- Conflicting design blocks are refused; the parser never picks one silently.
- Extraction is linear in reply size, uses no recursion that can overflow, and
  keeps the existing bounded repair (one round) and bounded error/schema text.
- A no-build turn never carries a kernel measurement.
- The Python reference forwards `cad_provider` and uses the same instruction
  bytes, parser messages and repair wording.
- Mock tests use only a synthetic key and a fake transport; no network call was
  made by this cell.

## Remaining gaps / notes for the conductor

- At report time two whole-suite tests fail because the concurrent
  `cli-providers` cell has renamed/added provider rows (the codex refusal
  wording and the four-row provider-list fixture), and that cell's import
  ordering is not yet rustfmt-clean. Re-run the full suite after it lands. Our
  six mock-provider tests and every design test are green.
- The Python HTTP route now forwards `cad_provider`, but the HTTP-level Python
  test for a non-template remote design is covered at the dispatch layer, not
  through FastAPI.
- No live model call was made by this worker. The conductor should run the
  user's stored OpenRouter/MiniMax key against the new prompt; the mock
  provider proof covers the request/response contract only.
- Windows runtime is not proven; only the pure-logic crate cross-checks.
