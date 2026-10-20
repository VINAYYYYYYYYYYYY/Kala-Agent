# design-intent-r1: kala-design-intent (templates, DesignIntent, local strict parser)
Follow `../CONVENTIONS.md`. Port into NEW standalone `backend/crates/kala-design-intent`, as pure logic (no geometry, no OCP):
- `src/kala_agent/studio/design/templates.py` (`l_bracket`, `stepped_shaft`, `housing_envelope` → ConstructionScript + declared envelope/tolerances/label, with every feasibility validation and message);
- `src/kala_agent/studio/design/intent.py` (the DesignIntent model, aliases, JSON Schema export byte-identical to Python's, fenced-JSON extraction and parsing, validation errors);
- the design-prompt parts of `src/kala_agent/studio/providers/local.py` (`parse_design_prompt`, `detect_design_family`, the missing-value replies, M-size clearance mapping).
Consume the accepted kala-contracts (ConstructionScript) and kala-json read-only. NOTE: a concurrent worker (B2) is adding a `provider` field and provider routing in Python. Port the CURRENT intent/templates/parser. If B2 lands while you work and changes `intent.py`, re-read it and match it.
Differential vs live Python:
- every template over a randomized parameter grid, including infeasible inputs (identical script canonical JSON or identical error);
- the schema text, byte-identical;
- parser outcomes on a corpus of at least 300 prompts (the four starters, Stitch-style sentences, unit variants, missing values, adversarial text);
- intent validation on valid and invalid JSON blocks.
Full Rust gates. REPORT.md as usual.
