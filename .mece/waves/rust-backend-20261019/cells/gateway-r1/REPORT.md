# gateway-r1 — `kala-runtime` (the model gateway)

Cell: `gateway-r1` of the `rust-backend-20261019` wave. Owner: the Rust worker
`gateway-r1`. Status: **complete** — the crate is ported, the live differential
against the repository's Python gateway is green with zero unexplained
mismatches, and every Rust gate passes except the Windows cross-check, which
cannot run in this environment (evidence below).

No commit, no push, no delegation, no credential read, and no edit outside
`backend/crates/kala-runtime/` and this file.

---

## 1. What was built

`backend/crates/kala-runtime` is a **new standalone crate** (its own
`[workspace]`, its own `Cargo.lock`) that ports
`src/kala_agent/runtime/model_gateway.py` — the complete module, 1,679 lines —
into Rust, plus the `runtime/__init__.py` exports it contributes.

The crate is the provider-neutral LLM connection layer of ADR-0001 (D2: one
egress choke point, deny by default) and ADR-0003 (policy checkpoints, no
silent provider substitution).

### Changed paths (the only ones I touched)

| Path | What |
|---|---|
| `backend/crates/kala-runtime/Cargo.toml` | the crate manifest (standalone workspace) |
| `backend/crates/kala-runtime/Cargo.lock` | the crate's own lock file (186 packages) |
| `backend/crates/kala-runtime/README.md` | the crate documentation |
| `backend/crates/kala-runtime/src/lib.rs` | the crate root and the re-exports |
| `backend/crates/kala-runtime/src/gateway/mod.rs` | the `gateway` module and its public surface |
| `backend/crates/kala-runtime/src/gateway/error.rs` | the error hierarchy (`ModelError`, `GatewayError`) |
| `backend/crates/kala-runtime/src/gateway/model.rs` | the model types (`ModelRequest`, `ModelResponse`, `ModelArtifact`, `ModelTranscript`, `ModelEgressManifest`, …) |
| `backend/crates/kala-runtime/src/gateway/client.rs` | `LlmClient`, `LocalModelClient`, `OpenAICompatibleClient`, the transport seam |
| `backend/crates/kala-runtime/src/gateway/gateway.rs` | `ModelGateway` (the egress, budget, transcript and manifest flow) |
| `backend/crates/kala-runtime/src/gateway/endpoint.rs` | the endpoint parser and the egress normalization |
| `backend/crates/kala-runtime/src/gateway/redact.rs` | the eight-pattern credential redaction engine |
| `backend/crates/kala-runtime/src/gateway/value.rs` | the bounded validators, the JSON freeze and the decimal input |
| `backend/crates/kala-runtime/src/gateway/decimal.rs` | the local CPython-compatible decimal multiply/divide |
| `backend/crates/kala-runtime/src/gateway/text.rs` | the CPython string helpers |
| `backend/crates/kala-runtime/tests/oracle.py` | the test-only Python oracle |
| `backend/crates/kala-runtime/tests/mock_provider.py` | the test-only mock OpenAI-compatible provider |
| `backend/crates/kala-runtime/tests/support/mod.rs` | the shared test helpers |
| `backend/crates/kala-runtime/tests/gateway_semantics.rs` | the semantics tests |
| `backend/crates/kala-runtime/tests/gateway_security.rs` | the residency/security tests |
| `backend/crates/kala-runtime/tests/python_differential.rs` | the mock-provider differential |
| `backend/crates/kala-runtime/tests/corpus_differential.rs` | the randomized deterministic corpus differential |

Nothing else in the repository was modified: the root `backend/Cargo.toml`,
the root `backend/Cargo.lock`, every accepted crate, the Python sources, the
Python tests, the shared fixtures, `adapters/` and `studio/` are untouched.

---

## 2. Public API

The Python module's public surface is reproduced name for name. The Rust
module is `kala_runtime::gateway`; the crate root re-exports the model types.

### Types

| Python | Rust | Notes |
|---|---|---|
| `ModelRole` | `ModelRole` | `system` / `user` / `assistant` |
| `ModelCapability` | `ModelCapability` | `structured_output` / `seed` / `tool_calling` |
| `ModelFinishReason` | `ModelFinishReason` | `stop` / `length` / `refusal` / `error` |
| `ModelMessage` | `ModelMessage` | `new`, `from_text`, `to_payload` |
| `ModelArtifact` | `ModelArtifact` | `new`, `from_text`, `with_*`, `to_payload`, `redacted`, `is_design_binary` |
| `ModelRequest` | `ModelRequest` | `new`, `from_parts`, `with_*`, `request_body`, `request_digest` |
| `ModelResponse` | `ModelResponse` | `new`, `with_*`, `total_tokens`, `response_digest`, `canonical_content` |
| `ModelRedaction` | `ModelRedaction` | `to_payload` |
| `ModelTranscript` | `ModelTranscript` | `to_payload` |
| `ModelEgressManifest` | `ModelEgressManifest` | `to_payload` |
| `LLMClient` (Protocol) | `LlmClient` (trait) | `provider_id`, `model_id`, `requires_egress`, `endpoint`, `supports`, `complete` |
| `LocalModelClient` | `LocalModelClient` | `new`, `calls` |
| `OpenAICompatibleClient` | `OpenAICompatibleClient` | `new`, `endpoint`, `allowed_endpoints`, `headers`, `payload`, `authorize_destination`, `parse_response`, `cost` |
| `ModelGateway` | `ModelGateway` | `new`, `with_ledger` / `with_transcript` / `with_policy` / `with_egress_manifest` / `with_clock`, `complete`, `egress_policy_request`, `egress_state_digest`, `egress_action_digest`, `transcripts`, `manifests` |
| `TranscriptCallback`, `EgressManifestCallback` | the same | `Arc<dyn Fn(&ModelTranscript) -> Result<(), CallbackError> + Send + Sync>` |
| `ModelError` and its nine subclasses | `ModelError` (one enum, nine variants) | `python_class_name()` returns the Python class name |
| `EgressDeniedError`, `ModelCapabilityError`, … | documented type aliases of `ModelError` | the Python names stay available |
| `BudgetReservationUnknownError` (from the ledger) | `BudgetError::ReservationUnknown` | the accepted `kala-budget` error |

### Functions

`redact_secrets`, `truncate_encoded`, `is_secret_name`, `estimate_tokens`,
`normalize_endpoint`, `is_loopback_host`, and the Python-private helpers the
repository's own tests import: `_bounded_identifier` → `bounded_identifier`,
`_bounded_text` → `bounded_text`, `_nonnegative_int` → `nonnegative_int`,
`_bounded_float` → `bounded_float`, `_nonnegative_decimal` →
`nonnegative_decimal`, `_freeze_json` → `freeze_json`, `_thaw_json` →
`thaw_json`, `_redact_metadata` → `redact_metadata`, `_parse_destination` →
`parse_destination`, `_normalized_destination` → `normalized_destination`,
`_provider_int` → `provider_int`, `_synthesize_value` → `synthesize_value`,
`_schema_type_names` → `schema_type_names`, `_schema_properties` →
`schema_properties`.

### Constants

`EGRESS_CAPABILITIES`, `MAX_MESSAGES`, `MAX_MESSAGE_LENGTH`,
`MAX_REQUEST_BYTES`, `MAX_TRANSCRIPT_BODY_BYTES`, `MAX_RESPONSE_BYTES`,
`MAX_OUTPUT_TOKENS`, `MAX_SEED`, `MIN_TIMEOUT_SECONDS`,
`MAX_TIMEOUT_SECONDS`, `CHARS_PER_TOKEN`, `LOCAL_PROVIDER_ID`,
`OPENAI_COMPATIBLE_PROVIDER_ID`, `LOOPBACK_HOSTS`, `MAX_ENDPOINT_LENGTH`.

### Accepted crates consumed read-only

`kala-json` (canonical bytes, digests), `kala-contracts` (the artifact,
approval and side-effect enums), `kala-kernel` (the policy engine, the aware
timestamp), `kala-budget` (the ledger, the decimal), `kala-tools` (the schema
validator, the protocol canonical JSON). The HTTP client is the same stack the
accepted `kala-studio` uses: `reqwest` over `rustls` with the platform trust
store, no redirects, no environment proxy.

---

## 3. The live differential

### 3.1 The mock-provider differential (`tests/python_differential.rs`)

The Python gateway and the Rust gateway are driven against **the same
in-process mock provider** (`tests/mock_provider.py`, a deterministic
`ThreadingHTTPServer`) with the same policy states and the same budget states.
The mock server records every request it receives, so the wire traffic of the
two implementations is compared as well.

The scenario has **26 cases** and covers:

| Group | Cases |
|---|---|
| local | deterministic, schema synthesis, seed, secret-shaped prompt, design-binary + text artifacts, unsupported capability, budget exhaustion, invalid request, multiple messages, invalid schema, duplicate artifacts, oversized message, large request (transcript truncation) |
| remote | success, malformed body, oversize body, slow (timeout), 503, redirect, no choices, empty content, artifact refusal, no consent, policy denied, schema violation, seed, oversized string |

What is compared, byte for byte, over canonical JSON:

* **decisions** — the outcome (`ok` / `error`), the Python exception class
  name and the exact message of every failure,
* **results** — the answer content, cost, finish reason, token counters,
  provider request id, the structured answer and the total token count,
* **budget ledger bytes** — the whole exported ledger state (budget, usage,
  reserved, reservations, next reservation),
* **transcripts and manifests** — every field, including the redaction
  records, the artifact digests, the recorded instant and the request and
  response bodies,
* **egress digests** — the state digest and the action digest,
* **the requests on the wire** — the path, the method, the body and the
  headers (`content-type`, `content-length` and the *presence* of
  `authorization`, whose value is replaced with `<redacted>` before it is
  recorded).

Result: **zero mismatches**. The mock provider recorded 11 requests in each
phase and the two request logs are identical.

### 3.2 The randomized deterministic corpus (`tests/corpus_differential.rs`)

A seeded linear congruential generator produces the same corpus on every run
and on both sides:

* 2,000 redaction inputs built from 49 credential-shaped and benign
  fragments,
* 500 secret-name candidates,
* 500 endpoint candidates,
* 324 decimal operand pairs (18 × 18).

Compared: the redacted text, the finding count, the byte-safe truncation, the
secret-name verdict, the normalized endpoint (or the exact refusal message)
and the loopback verdict, and the exact decimal product and quotient
(strings, not just values).

Result: **zero mismatches** over all 3,324 corpus entries.

Two real defects were found and fixed by the corpus during the port:

1. the endpoint parser reported the query/fragment refusal before the port
   refusal, while CPython reads the port while it splits the URL;
2. the decimal division did not reproduce `libmpdec`'s "scale the coefficient
   towards the ideal exponent" rule, so `Decimal(1000000)/Decimal(1)` was
   `1E+6` instead of `1000000`; and `0/0` raised `DivisionByZero` instead of
   `InvalidOperation`.

### 3.3 The Python oracle

`tests/oracle.py` is the live oracle: it imports the repository's own
`kala_agent.runtime.model_gateway` with the repository's
`.venv/bin/python`, exactly like the accepted cells do. The oracle runs in
CPython's **isolated mode** (`python -I`) and puts the repository sources on
the path itself through the `KALA_SRC` environment variable, so no
`PYTHONPATH`, no user site package and no `PYTHONSTARTUP` can influence the
result. Python is a **test oracle only**; the crate never invokes it at
runtime.

---

## 4. Test counts and the gates

```
cargo fmt --check                                     → clean
cargo clippy --locked --all-targets --all-features -- -D warnings → clean
cargo test  --locked            → 55 passed, 0 failed
cargo test  --locked --release  → 55 passed, 0 failed
cargo build --locked --release  → ok
cargo check --target x86_64-pc-windows-gnu            → FAILS (environment, see §6)
```

The 55 tests are:

| Binary | Tests | What |
|---|---|---|
| `tests/gateway_semantics.rs` | 37 | every public function: the bounded validators, the decimal input, the JSON freeze, the redaction engine, the truncation, the secret-name detection, the metadata redaction, the token estimate, the schema synthesis, the message/artifact/request/response/transcript validation, the endpoint parsing, the local client, the provider payload and answer parsing, the gateway flow (transcript, budget, substitution, degradation, capability refusal, egress refusal, digest binding) |
| `tests/gateway_security.rs` | 14 | egress without a policy, per-request approval, stale approval, the action-digest facets, the manifest-before-dispatch ordering, the missing/failing manifest, the byte-safe truncation, the artifact redaction, the AWS-style secrets, the hosted allow-list and consent, the policy failure without degradation, the oversized content, the secret-name conservatism, the credential-free `Debug` |
| `tests/python_differential.rs` | 2 | the mock-provider differential (results, ledger bytes, digests, transcripts, manifests and the wire traffic) and the scenario-coverage assertion |
| `tests/corpus_differential.rs` | 1 | the 3,324-entry randomized deterministic corpus |
| the crate doc test | 1 | the README/lib example compiles and runs |

The Python tests that the port mirrors were read in full:
`tests/unit/test_runtime.py` (the gateway part), `tests/unit/test_model_gateway_branches.py`
(all 1,311 lines), `tests/security/test_model_residency.py` (all 627 lines) and
`tests/integration/test_mechanical_loop.py` (the gateway part).

---

## 5. Compatibility and security evidence

**Fidelity.** The validation messages, the error precedence, the canonical
request bytes, the digests, the redaction results, the transcript bytes and
the ledger bytes are identical to the Python implementation — that is what
the two differentials compare, and they are green.

The error precedence is preserved: sanitize → authorize egress → check
capabilities → reserve budget → dispatch → verify identity → validate the
structured answer → record. A request that fails the schema check never
reaches the policy engine, a denied egress never reserves budget, and a
provider failure releases the reservation.

**Residency (ADR-0001).** The evidence is in `tests/gateway_security.rs`:

* a hosted provider without a policy engine is denied before the call
  (`provider … requires egress and no policy engine is attached`),
* every call needs a **per-request** approval: an approval for one request
  does not authorize another, and a mutated request is refused as
  `stale_approval`,
* the action digest binds the destination, the provider, the model, the
  capability list, the redactions **and** the request digest, and it is
  recomputed at call time,
* the egress manifest is recorded **before** the dispatch (the ordering is
  asserted), and a missing or failing manifest callback denies the call and
  records no transcript,
* the transcript and the manifest bodies are bounded at 64 KiB and the
  truncation is byte-safe (a character that straddles the cut is dropped
  whole),
* an artifact preview text and artifact metadata are redacted, and a design
  binary is referenced by digest only,
* a hosted endpoint needs an explicit consent **and** an allow-list entry;
  an empty allow-list is a denial,
* a remote provider never receives an artifact payload.

**Credentials.** No credential is ever logged, echoed or embedded in an
error message:

* the provider client's `Display` prints `api_key='<redacted>'` (or
  `api_key=None`), never the key,
* the outbound `ProviderRequest` has a custom `Debug` that replaces the
  `authorization` header with `<redacted>`,
* the transport failures carry the provider id and the transport class name
  only,
* the mock provider redacts the `authorization` value **before** it records
  the request, so the differential never stores a credential,
* the test-only transport asserts that no `Bearer` value is ever echoed.

The Python's own `ProviderDegradedError` reason is passed through
`_safe_reason`, which replaces a credential-shaped error text with
`<redacted provider error>` — reproduced exactly.

---

## 6. Intentional differences, and the remaining gaps

Every difference below is deliberate and was checked against the Python
behaviour.

1. **The provider call is asynchronous.** The Python `complete()` is
   synchronous; the accepted HTTP stack (reqwest over rustls, the same stack
   `kala-studio` uses) is asynchronous. `LlmClient::complete` and
   `ModelGateway::complete` are `async`. Every message, precedence and byte
   is unchanged; only the calling convention differs.
2. **One error enum instead of an exception hierarchy.** The nine Python
   exception classes are the nine variants of `ModelError`;
   `ModelError::python_class_name()` returns the Python class name, and the
   `GatewayError` enum keeps the budget and the policy families separate so
   that a budget exhaustion still reads
   `TokenBudgetExhaustedError` and a policy failure still reads
   `PolicyError`. The Python class names are also available as documented
   type aliases.
3. **The type checks are compile-time.** The Python validators accept any
   object and reject a non-string with `"{field} must be a string"`; the Rust
   validators take the typed value, so that message is unreachable. The
   *value* checks (`must be a safe identifier`, `exceeds the bounded length`,
   `must be nonnegative`, `must be finite and within its bounds`, `must be a
   JSON object`, `must be a nonnegative decimal`, `must be a finite
   nonnegative decimal`, …) are reproduced exactly, including the four
   different decimal messages.
4. **The transport exception names are an approximation.** The Python reports
   the `httpx` class name in `provider … is unreachable: <class>`. The Rust
   maps the reqwest failures to the closest `httpx` class: `ConnectTimeout`,
   `ReadTimeout`, `ConnectError`, `TransportError`. The differential shows
   the same name for the timeout case (`ReadTimeout`) and for the ordinary
   transport failures. A `httpx` class that Rust cannot produce exactly
   (for example `WriteTimeout`) has no exact counterpart; the mapping table
   is in `TransportFailure::httpx_class_name`.
5. **The `Decimal` multiplication and division are implemented locally.**
   The accepted `kala-budget` crate exposes only `Decimal::add` and
   `Decimal::sub`:
   ```
   $ grep -n 'pub fn ' backend/crates/kala-budget/src/decimal/mod.rs
   pub fn add ... pub fn sub ...      (no multiplication, no division)
   ```
   The gateway needs both (the provider cost is
   `(input_tokens * input_cost + output_tokens * output_cost) / 1_000_000`), so
   `src/gateway/decimal.rs` implements the two missing operations against the
   CPython default context (precision 28, `ROUND_HALF_EVEN`, `Emax = 999999`,
   `Emin = -999999`) using the accepted `Decimal` type. The accepted crate was
   **not** patched. The 324-pair decimal corpus proves the results are
   byte-identical to CPython, including the "scale the coefficient towards the
   ideal exponent" rule of `libmpdec` and the `0/0 → InvalidOperation` case.
6. **The integer width is `i128`, not arbitrary precision.** The Python
   counters are unbounded integers. The Rust port uses `i128` for the token
   counters and the artifact size, and the accepted `kala-budget` `Counter`
   (a `BigUint`) for the ledger. A provider that reports more than 2^127
   tokens is clamped instead of counted exactly; no realistic provider does.
7. **The naive/aware distinction is explicit.** CPython's `datetime` can be
   naive; the gateway refuses a naive timestamp with `recorded_at must
   include a timezone` (transcript, manifest) or `the gateway clock must
   return an aware datetime` (clock). The Rust port keeps the distinction in
   the `GatewayTimestamp` enum (`Aware(KernelTimestamp)` / `Naive`) so both
   refusals stay observable, instead of hiding them behind a type that cannot
   express a naive value.
8. **The request builder re-validates on every step.** `dataclasses.replace`
   re-runs `__post_init__`, so setting the `seed` capability before the seed
   fails in Python too. `ModelRequest::from_parts` builds a request in one
   step (the Python dataclass constructor) and the `with_*` methods mirror
   `replace`.
9. **The redaction engine does not use a look-around regex.** The `regex`
   crate has no look-around, and three of the eight Python patterns use a
   look-behind, a look-ahead or both. `src/gateway/redact.rs` therefore
   implements the secret-name matcher explicitly, enumerating the candidates
   at one start position in CPython's backtracking order (the longest prefix
   first, then the key alternatives in their declared order, then each
   alternative's own internal alternatives), and applies the boundary checks
   by hand. The 2,000-entry randomized corpus proves the results are
   identical to `re.subn`.
10. **The metadata redaction is iterative.** The Python `_redact_metadata`
    recurses; the Rust port walks an explicit stack, so a hostile document
    cannot overflow the call stack. The depth of a structured value is
    bounded at 32 by `freeze_json`, exactly like the Python.
11. **`httpx` is not used; `reqwest` is.** The transport seam is
    `ProviderTransport` instead of `httpx.BaseTransport`, and the production
    implementation is `ReqwestTransport` (reqwest + rustls + the platform
    trust store, no redirects, no proxy, a bounded response body).
    `httpx`'s `trust_env=False` maps to reqwest's `.no_proxy()`; the
    `SSL_CERT_FILE` environment override that `trust_env=False` also
    suppresses is not reproduced (rustls reads the platform trust store).
12. **The Windows cross-check cannot run here.** The target
    `x86_64-pc-windows-gnu` **is** installed, but the mingw-w64 C toolchain
    is not, and the `ring` crate (a rustls dependency) needs it:
    ```
    $ cargo check --target x86_64-pc-windows-gnu
    Compiling ring v0.17.14
    error occurred in cc-rs: failed to find tool "x86_64-w64-mingw32-gcc"
    ```
    The **accepted `kala-studio` crate fails identically** in this
    environment, so this is a missing system toolchain and not a defect of
    this crate. No Windows runtime was exercised and no Windows claim is
    made.
13. **No runtime cutover.** The Python runtime is still the product. The Rust
    crate is a library; the next cell (`agent-r1`) builds the agent runtime on
    top of it. No speed, readiness or deployment claim is made.

### Remaining gaps

* The Python `runtime/agent.py` (the planner and the agent coordinator) is
  **not** part of this cell; it is the `agent-r1` cell's work.
* The Python `application.py` composition root and the API layer still build
  the Python gateway; nothing in the product uses the Rust crate yet.
* The `ModelGateway` does not write a journal entry itself: the Python one
  does not either (it hands the transcript and the manifest to callbacks),
  and the callbacks are the seam a later cell uses to journal them.
* The `LocalModelClient` is the deterministic test double the Python one is
  (ADR-0001, D7). It is not a language model and does not pretend to be one.
* The `OpenAICompatibleClient` covers the OpenAI-compatible chat-completions
  shape only, exactly like the Python one; there is no streaming, no tool
  calling and no provider SDK.

---

## 7. How to reproduce

```bash
cd backend/crates/kala-runtime
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --locked --release
```

The differential tests need the repository virtual environment
(`<repo>/.venv/bin/python`); `KALA_PYTHON` overrides it. The mock provider
binds a loopback port; no test opens an outbound connection.
