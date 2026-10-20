# GPT-6.1-Sol review — Studio repairs required

Independent release tests:65passed. Real binary corpus replay:40exact,6explicitSTEPdependency differences,2body-capconfig cases covered by library harness; no unexpected mismatch. Code review found additional actual compatibility issues below. Studio is not accepted for cutover yet. First implementation is useful progress, not a fully equivalent service.

## P2 — Windows-style JSON encodings rejected

Out-of-tree real release-binary versus live Python FastAPI probe, POST /api/studio/preview with valid bearer and Content-Type application/json, body {"kind":"box"}:
- UTF8BOM: Python200/925bytes, native422/135bytes.
- UTF16 (BOM): Python200/925bytes, native400/114bytes.
- UTF32 (BOM): Python200/925bytes, native400/114bytes.
Python json.loads(bytes) performs encoding detection; native std::str::from_utf8 +serde parser loses this accepted domain. Important for Windows-generated JSON. Implement native equivalent decoding, preserve actual source malformed-input status/offsets, no Python runtime dependency. Include UTF16LE/BE,UTF32LE/BE,BOM/no-BOM detection where Python accepts and malformed encodings/surrogate cases. NoContentType was also probed: both return422identical, so do not change that current behavior based on older FastAPI assumptions.

normalise_nonstandard_constants currently replaces NaN/Infinity prefix bytes without token-boundary validation and changes subsequent error offsets. Confirm against oracle and avoid turning invalid NaN123/Infinity1 into valid exponent literals, or shifting later malformed JSON detail positions. Preserve duplicate-key last-wins on HTTP bodies; the strict hash parser is not directly a compatible substitute.

## P2 — valid existing settings silently ignored

Probe: same valid studio-settings.json containing api_keys.openai synthetic value +just over1MiB of trailing legal JSON whitespace. Python GETsettings reports configured=true/source=stored; native reports false/none. MAX_SETTINGS_FILE_BYTES is a new accepted-data restriction. Preserve valid stored keys rather than dropping them solely due total file length. Source files are unchanged by this review; do not reset credentials or log values. Keep atomic writes/permissions and malformed-file behavior. Add real library/binary regression around1MiB boundary and existing-key save/clear.

## P2 — missing ChatResponse output-model validation

Static source evidence: Python chat_route constructs ChatResponse, whose model field has max_length256; a provider reply naming a longer model raises ValidationError and becomes sanitized500. Native chat_success serializes outcome.model directly without this check. Reproduce with isolated transport mocks in Rust and monkeypatched Python provider, then preserve response constraints/error behavior. Check other provider payload/response boundary variants (malformed/non-UTF8/UTF16 bytes, nonfinite unrelated fields) against source rather than only the happy-path mock. No live model/network calls or real credentials.

## TLS trust/configuration compatibility

Source urllib HTTPS uses Python/system default trust (including normal SSL_CERT_FILE/SSL_CERT_DIR paths), while native reqwest rustls-tls uses webpki roots. This can reject user-configured OS/private CAs, especially relevant for Windows desktop/proxy setups. Preserve intended system/default trust and documented configuration without disabling certificate or hostname verification; validate using an isolated local TLS server +synthetic CA rather than a live provider. Source _build_ssl_context helper is not passed explicitly, so confirm actual urllib opener behavior rather than assuming the unused helper alone proves it. Review same source environment proxy behavior and report concrete remaining gaps. Credential normalization should use existing pytext::trim semantics consistently, not Rusttrim where Pythonstr.strip applies.

## Tracked gaps / boundary scope

Native STEP6cases remain legitimate pending dependency, not permission to fake/normalize them away. Lone-surrogate/deepinvalid-request diagnostic differences, MIME subset and exact low-level network error-class names remain documented for eventual cutover; improve if part of the touched parser/transport but do not silently claim parity. Intentional static symlink escape refusal remains good; no need to recreate the Python escape. Existing strictroot/sharedcrate ownership remains unchanged. No Windows/COM/installer or performance proof yet.
