# GPT-6.1-Sol final original-turn review — additional confirmed regressions

Original T3 task now terminal/completed, no live child turns. Consumed final result. Independently passed43locked offline release tests, fmt, all-target/all-feature clippy-Dwarnings and450livePython model oracle cases. Independent /tmp scratch consumer confirms REVIEW-1 ordinary duplicate-key and both profile alias-order examples are now repaired. Preserve those fixes. Full contracts acceptance is still withheld for three newly confirmed cases below, not merely undocumented hypothetical edges.

## P2 — nonadjacent duplicates bypass uniqueness validation

`DesignSpec` JSON `{"id":"spec","name":"probe","summary":"probe","constraints":["a","b","a"]}`: Python rejects `collection values must be unique`, Rust accepts and returns the repeated constraints. `models.rs::canonicalize_texts(...,sort=false)` only checks `windows(2)`, so it detects adjacent duplicates but misses any separated duplicate. Affects DesignSpec.constraints and EvaluationResult.findings. Check analogous helpers for the same issue and preserve original sequence order while validating full uniqueness. Test normalized values and nonadjacent as well as adjacent duplicates across both models.

## P2 — serde arbitrary-precision marker changes valid JSON data

`DesignSpec` JSON `{"id":"spec","name":"probe","summary":"probe","attributes_json":{"$serde_json::private::Number":"42"}}`: Python accepts and preserves attributes_json as `{"$serde_json::private::Number":"42"}`. Rust's ordinary-input serde_json::Value parser turns that ordinary JSON object into a Number and then rejects attributes_json with `value must be a JSON object`. This is serde's private Number-tag collision, not a forbidden Python field. Preserve arbitrary dictionary keys/shapes/integers/codepoint text exactly; do not blacklist the field or hide the mismatch by forcing callers to stringify objects. Test markers at several nested positions with siblings, escaped key spelling and nested canonical objects. Private parser representation must not collide with user JSON.

## P2 — duplicate plus surrogate input still rejects valid model data

`DesignSpec` JSON `{"id":"spec","id":"last","name":"probe","summary":"probe","attributes_json":{"text":"\ud800"}}`: live Python `DesignSpec.model_validate(json.loads(raw))` (FastAPI-equivalent decoded model path) accepts id=last and preserves canonical surrogate text. Rust rejects: serde cannot represent escaped surrogate, strict fallback rejects duplicate keys. Common last-wins and surrogate paths individually pass but their combination is still part of accepted source data. Also deep documents that need fallback cannot lose profile-order semantics or duplicate handling.

Repair native model parsing to preserve insertion-order/last-wins/codepoints/arbitrary integers together, without weakening accepted kala-json hash JSON or imposing tool limits on persisted fields. Use iterative/lifecycle-safe handling for deep accepted input; don't replace one 128/1024-level ceiling with another. Keep a separate orderless from_value API documented; JSON/loader consumers need the faithful path. CanonicalJsonObject supplied as a raw JSON string must still reject duplicate keys exactly as source does; ordinary parsed dict duplicates already collapsed by model input are distinct.

All probes used actual current Rust crate through a /tmp scratch executable plus repo .venv Python; no product/source/fixture edits and no credential data. URI probes retained parity; no extra IDNA defect asserted. Existing ordinary error strings remain a later core422 compatibility gate. Original TASK.md/currentREPORT/REVIEW-1 provide full original brief/prior findings/responses; this review adds narrow unresolved objections.
