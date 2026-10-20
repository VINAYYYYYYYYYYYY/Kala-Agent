# kala-design-intent

Pure-logic port of the Studio design surface:

- `kala_agent.studio.design.templates` — deterministic L-bracket, stepped-shaft
  and housing-envelope templates with every feasibility check and message;
- `kala_agent.studio.design.intent` — the `DesignIntent` models, aliases, the
  byte-identical JSON Schema export, fenced-JSON extraction and the
  product-visible validation text;
- the design half of `kala_agent.studio.providers.local` —
  `detect_design_family`, `parse_design_prompt`, the ISO 273 medium clearance
  table, canonical template defaults and the missing-value replies.

It owns no geometry and never imports OpenCascade or any CAD kernel. A template
returns a `kala_contracts::ConstructionScript`, a declared `Bounds` envelope,
declared tolerance strings and a label. Volume and surface area stay kernel
measurements.

## Public API

| Area | Items |
|---|---|
| Templates | `l_bracket`, `stepped_shaft`, `housing_envelope`, `TemplateResult`, `TemplateError`, `Bounds`, `TemplateShaftSegment`, `DEFAULT_HOLE_EDGE_MM` |
| Intent | `DesignIntent`, `TemplateCall`, `LbracketCall`, `SteppedShaftCall`, `HousingCall`, `ShaftSegment`, `HoleCount`, `DesignIntentValidationError`, `DesignIssue`, `parse_design_intent_text`, `strip_design_json_block`, `design_intent_schema_text`, `design_intent_json_schema`, `DESIGN_INTENT_INSTRUCTION` |
| Prompt parser | `detect_design_family`, `parse_design_prompt`, `DesignPrompt`, `DesignPromptError`, `DesignFamily`, `design_prompt_reply`, `iso_273_medium_clearance`, `M_SIZE_CLEARANCE` |
| Formatting | `format_g` (Python `:g`) |
| Constants | `DEFAULT_BRACKET_WIDTH_MM`, `DEFAULT_BRACKET_DEPTH_MM`, `MAX_DIMENSION_MM`, `MIN_DIMENSION_MM`, `MAX_TOLERANCES`, `MAX_MATERIAL_LENGTH`, `MAX_LABEL_LENGTH`, `MAX_TOLERANCE_LENGTH`, `MAX_SEGMENTS`, `MAX_RESPONSE_LENGTH` |

`DesignIntent::from_json_str` uses CPython `json.loads` semantics for the
payload (duplicate keys last-wins, `NaN`/`Infinity` accepted and then refused by
the finite-number rule). `to_wire_json`/`to_wire_json_ascii` reproduce
`model_dump(mode="json")`; `to_canonical_json(exclude_none)` produces sorted,
ASCII-escaped bytes through `kala-json`.

The accepted `kala-contracts` crate validates and owns the typed
`ConstructionScript`; a local Pydantic-compatible validator reproduces the
`DesignIntent.script` error locations and ordering before the typed value is
constructed.

## Compatibility notes

- The schema text is a checked-in byte artifact (`assets/design_intent_schema.json`)
  generated from the live Pydantic model. The differential test regenerates it
  from Python on every run.
- Rust's typed template arguments replace Python's dynamic `bool`/`int`/`float`
  coercion; `hole_count` remains `i64` so the Python range message is kept.
- `parse_design_intent_text` bounds JSON nesting at 512 levels; CPython raises
  `RecursionError` near its interpreter recursion limit.
- CPython's 4,300-digit integer conversion limit is not applied; integers stay
  arbitrary precision (`num-bigint`), and a huge numeric field value fails the
  finite-number rule instead.
- Pydantic string fields reject lone surrogates with
  `Input should be a valid string, unable to parse raw data as a unicode string`.
- The local prompt parser implements Python's `str.isalnum()` word boundaries
  itself because the `regex` crate's Unicode `\w` also counts combining marks.

## Tests

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --release --locked
cargo check --locked --target x86_64-pc-windows-gnu
```

The live differential (`tests/python_differential.rs` with
`tests/oracle/design_oracle.py`) uses the repository virtualenv only at test
time. Production code never spawns Python.
