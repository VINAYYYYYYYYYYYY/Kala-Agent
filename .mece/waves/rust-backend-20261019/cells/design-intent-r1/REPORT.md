# design-intent-r1 — `kala-design-intent`

Native Rust port of:

- `src/kala_agent/studio/design/templates.py`
- `src/kala_agent/studio/design/intent.py`
- the design-prompt half of `src/kala_agent/studio/providers/local.py`

New standalone crate: `backend/crates/kala-design-intent` (own `[workspace]`,
own `Cargo.lock`). It consumes accepted `kala-contracts` (typed
`ConstructionScript`) and `kala-json` read-only through path dependencies. It
owns no geometry, never imports OCP/CadQuery, and production code never invokes
Python. `#![forbid(unsafe_code)]`.

## Public API

### Templates
- `l_bracket(width, depth, height, wall, bore_d, hole_d, hole_count: i64, edge)`
- `stepped_shaft(&[TemplateShaftSegment])`
- `housing_envelope(width, depth, height, wall)`
- `TemplateResult` (`script`, `envelope`, `tolerances`, `label`,
  `canonical_script_json`)
- `TemplateError`, `Bounds`/`BoundsError`, `TemplateShaftSegment`
- `DEFAULT_HOLE_EDGE_MM`

### Intent models and parsing
- `DesignIntent`, `TemplateCall`, `LbracketCall`, `SteppedShaftCall`,
  `HousingCall`, `ShaftSegment`, `HoleCount`
- `DesignIntentValidationError`, `DesignIssue`, `DesignIssueLocation`,
  `DesignIntentInputError`, `DesignIntentParseError`
- `parse_design_intent_text`, `strip_design_json_block`
- `design_intent_schema_text` (byte-exact), `design_intent_json_schema`
- `DESIGN_INTENT_INSTRUCTION`
- `DesignIntent::from_json_str` / `from_json_bytes` / `from_canonical_value`,
  `to_wire_value`, `to_wire_value_excluding_none`, `to_wire_json`,
  `to_wire_json_ascii`, `to_canonical_json`
- `DEFAULT_BRACKET_WIDTH_MM`, `DEFAULT_BRACKET_DEPTH_MM`, `MAX_TOLERANCES`,
  `MAX_MATERIAL_LENGTH`, `MAX_LABEL_LENGTH`, `MAX_TOLERANCE_LENGTH`,
  `MAX_SEGMENTS`

### Strict local prompt parser
- `detect_design_family`, `parse_design_prompt`, `DesignPrompt`,
  `DesignPromptError`, `DesignFamily`
- `design_prompt_reply`, `iso_273_medium_clearance`, `M_SIZE_CLEARANCE`
- `MAX_RESPONSE_LENGTH`

### Shared
- `format_g` / `format_g_with` (Python `:g`)
- `MAX_DIMENSION_MM` (studio 2000), `MIN_DIMENSION_MM` (0.1),
  `MAX_COMPONENTS`

## Changed paths

Created only under `backend/crates/kala-design-intent/`:

```
Cargo.toml
Cargo.lock
README.md
assets/design_intent_schema.json      # byte-exact Python schema export
src/lib.rs
src/format.rs                         # Python :g and float() grammar
src/limit.rs                          # studio dimension limits/messages
src/pytext.rs                         # Python strip/lower/whitespace/splitlines
src/json.rs                           # ordered JSON + CPython json.loads reader
src/intent.rs                         # DesignIntent models/schema/reply parsing
src/script.rs                         # Pydantic-compatible script field validation
src/templates.rs                      # three templates + declared metadata
src/prompt.rs                         # family detection, strict parser, replies
tests/design_contract.rs              # 40 unit/edge/security tests
tests/python_differential.rs          # 5 live differential tests + corpus-size check
tests/oracle/design_oracle.py         # test-only Python oracle
```

No parent manifest, lockfile, Python source/test, shared fixture, adapter or
`studio/**` file was edited. No commit, push, deployment, delegation, or
credential read was performed.

## Differential evidence (live Python)

Oracle: `.venv/bin/python -I tests/oracle/design_oracle.py`, repository `src`
inserted by the oracle script; the same cases are evaluated by Rust and Python.

| Corpus | Cases | Result |
|---|---:|---|
| Templates (valid + infeasible randomized grid) | 908 | 0 mismatches |
| Local design prompts | 563 | 0 mismatches |
| Model replies (`parse_design_intent_text` + `strip_design_json_block`) | 58 | 0 mismatches |
| Raw intent payloads (valid/invalid JSON blocks) | 192 | 0 mismatches |
| JSON Schema text/value | 1 | byte-identical |

What is compared, exactly:

- Templates: canonical script JSON from
  `model_dump(mode="json", exclude_none=True)` with sorted keys, compact
  separators and ASCII escaping; label; tolerance list; envelope min/max; or
  the exact `TemplateError` message.
- Prompts: `None` / family / intent canonical JSON / missing list / notes /
  `_design_prompt_reply` text / exact `_validation_text` on validation failure.
- Replies: parse success canonical JSON, exact `DesignIntentParseError` text,
  and `strip_design_json_block` prose.
- Intents: Python `json.loads` error `msg`, `_validation_text`, or canonical
  `model_dump(mode="json", exclude_none=True)`.
- Schema test: schema text bytes, canonical schema value, the exact
  `DESIGN_INTENT_INSTRUCTION` bytes, and every `MAX_*`/`DEFAULT_*` constant
  (`MAX_TOLERANCES`, material/label/tolerance lengths, segment counts, bracket
  defaults, hole edge, response length) against live Python.

Corpus composition:

- 908 template cases: the ADR/Python-test examples, 420 randomized L-brackets,
  120 housings, 140 randomized shafts, 120 continuous-random L-brackets and 80
  continuous-random shafts, including boundary (`0.1`, `2000`), out-of-range,
  negative-zero, empty/oversized segment lists and total-length failures.
- 563 prompts: the four Studio starters, Stitch-style sentences, `x`/`×`
  separators, unit variants (`mm`/`cm`, ignored `m` forms), missing values,
  defaults, ISO 273 M sizes, housing/cylindrical families, Unicode decimal
  digits, combining-mark word-boundary edges, control characters, long and
  adversarial text.
- 58 replies: bare/fenced/uppercase/control-character fences, prose, malformed
  JSON, CPython error vocabulary, duplicate keys, `NaN`/`Infinity`, deep
  nesting, script shapes, invalid intents, lone-surrogate strings.
- 192 intents: valid template/script payloads, aliases, extra fields, both/no
  source, all field type/range/length bounds, literal hole counts, script
  model-validator ordering, extras at every level, surrogates, huge integers.
- Schema artifact: 11,291 bytes, SHA-256
  `4c39bbb8f60fdca452180d142c613563e53dbfbe1c56943c0bba5b8a3b1c8f65`,
  identical to the live `design_intent_schema_text()`.

The ADR L-bracket example (`60 x 45 x 40`, wall 8, bore 10, 4 x 5.5 holes)
produces 1,457 canonical bytes, SHA-256
`88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae`,
matching the live Python `canonical_script_json`.

## Rust gates

Run from `backend/crates/kala-design-intent` on Rust 1.99.1 / cargo 1.99.1:

| Command | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass, no warnings |
| `cargo test --locked` | 54 passed (8 lib + 40 `design_contract` + 6 `python_differential`), 0 failed; 0 doc tests |
| `cargo test --locked --release` | 54 passed, 0 failed |
| `cargo build --release --locked` | pass |
| `cargo check --locked --target x86_64-pc-windows-gnu` | pass (target installed; compile check only, no Windows runtime claim) |

## Security and edge evidence

- `tests/design_contract.rs::source_is_pure_logic_without_geometry_or_process_io`
  scans every `src/*.rs` and refuses `std::process`, `std::net`, `std::fs`,
  `OCP`, `cadquery`, and `Command::new`.
- `deeply_nested_replies_are_refused_without_overflow`: 600-level nesting is
  refused with a bounded message; 200-level nesting validates as an extra
  field, with no overflow.
- `huge_numeric_literals_do_not_panic`: 5,000-digit integers stay
  arbitrary-precision and fail the finite-number field rule instead of
  overflowing.
- `long_prompts_and_control_characters_stay_bounded`: C0 separators, 100k input
  and repeated instruction text stay bounded; replies are truncated at 1,024
  code points.
- Template bounds (`0.1..2000`), segment counts (8), total shaft length,
  tolerances (16), material/label/tolerance lengths and script tuple limits
  (8 components, 12 steps, 16 assembly steps) all have explicit tests.
- Regexes in the prompt parser are linear-time `regex` patterns; Python's
  `str.isalnum()` word boundaries are implemented manually because the regex
  crate's Unicode `\w` also counts combining marks.
- JSON reply parsing uses a bounded (512-level) recursive reader; it cannot
  overflow on attacker-sized input.

## Intentional differences

1. **Schema generation.** The schema is a checked-in byte artifact generated
   from live Pydantic, not a Rust schema generator. Every differential run
   regenerates it from Python and compares bytes. `design_intent_json_schema`
   returns a key-sorted `kala_json::CanonicalValue`; the byte text function is
   the product artifact.
2. **Typed template arguments.** Rust `f64`/`i64` arguments replace Python's
   dynamic numeric coercion. A boolean dimension or a non-integer/boolean
   `hole_count` is rejected by the type system rather than at runtime. Integer
   `hole_count` values outside `0,2,4` still produce the Python message.
3. **`Bounds` arity.** Rust `[f64; 3]` makes Python's "bounds require three
   minimum and three maximum values" case unrepresentable; the finite and
   ordered-corner checks and messages are preserved.
4. **JSON nesting.** Reply parsing is bounded at 512 levels; CPython raises
   `RecursionError` near its interpreter recursion limit. The pathological
   error text therefore differs.
5. **CPython integer digit limit.** The 4,300-digit `int` conversion limit is
   not applied; integers remain arbitrary precision. A huge numeric field fails
   the finite-number rule with a bounded message instead.
6. **Lone surrogates.** Pydantic string fields reject them with the exact
   `Input should be a valid string, unable to parse raw data as a unicode
   string` message. For a few script fields (`op`, `profile`) CPython can raise
   a raw `UnicodeEncodeError` while formatting the message; Rust returns a
   bounded validation message with the replacement character instead.
   `to_wire_json` fails on lone surrogates like Python's UTF-8 dump;
   `to_wire_json_ascii` escapes them.
7. **`from_json_bytes` invalid UTF-8.** Python's `json.loads(bytes)` raises
   `UnicodeDecodeError`; Rust returns `DesignIntentInputError::Json("payload is
   not valid UTF-8")`.
8. **Parser errors are typed.** `parse_design_prompt` returns
   `Result<Option<DesignPrompt>, DesignPromptError>` rather than letting
   Pydantic's `ValidationError` escape. `Display`/`_validation_text` text is
   identical to Python's.
9. **Scope.** `local_chat`, `ChatOutcome`, `DEFAULT_LOCAL_MODEL`,
   `MAX_PROMPT_LENGTH`, provider dispatch and the analytic estimate path are not
   ported; the task scoped this cell to the design-prompt parts of `local.py`.
10. **Error metadata.** Only Pydantic's `loc` and `msg` are modeled, because
    `_validation_text` exposes only those. The `type`/`input` fields are not
    public.
11. **Unchecked construction.** Pydantic's `model_construct` escape hatch is
    not exposed; every Rust `DesignIntent` comes from validation.

## Remaining gaps / coordination

- No runtime cutover, HTTP route, worker wiring, Windows runtime test, speed
  claim or readiness claim is made. This is a pure-logic crate and a test-time
  differential.
- Concurrent worker B2 had **not** changed `intent.py`/`templates.py` while this
  cell ran. Recorded hashes:
  - `src/kala_agent/studio/design/intent.py`
    `1e85d6b6df759750d01ebd5c6cca8bff`
  - `src/kala_agent/studio/design/templates.py`
    `63b99eb9690a070b5abd520bfe6de3a7`
  - `src/kala_agent/studio/providers/local.py`
    `afcf8540f0f015af0b365700d6407ed6`
  If B2 lands the `provider` field or provider routing in `intent.py`, the
  schema artifact and intent model must be re-ported; the differential schema
  test will fail until then.
- The differential covers the randomized corpus above, not every possible
  Unicode/Pydantic corner. Exotic Python-only inputs (Pydantic model instances,
  `Decimal`, custom mapping subclasses) are outside the JSON surface.
- Geometry, volume, surface area, envelopes-as-measurements and kernel validity
  are deliberately absent; templates only declare.
