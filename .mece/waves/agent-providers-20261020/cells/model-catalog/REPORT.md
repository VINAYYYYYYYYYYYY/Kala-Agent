# model-catalog — provider model listing (`GET /api/studio/providers/{id}/models`)

**User data:** the USER DATA rule was followed in every shell. Every command ran
with `RUSTUP_HOME`/`CARGO_HOME` pointing at the toolchain and a fresh
`HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`.
No file under the user's real data directory was read or written, and no
stored credential or CLI credential was read. The only provider traffic was one
unauthenticated `curl https://openrouter.ai/api/v1/models` (public data, no key)
used to size the limits, plus `--help`/`models --help` against the CLIs with a
throwaway `HOME` (no authentication, no session).

## Contract for model-ui (exact wire shape)

`GET /api/studio/providers/{id}/models?allow_remote=true`

Request headers required by the route:

```
Authorization: Bearer <studio token>
```

Success (`200`):

```json
{
  "provider": "openrouter",
  "source": "openrouter",
  "fetched_at": "2026-10-20T12:34:56Z",
  "models": [
    {
      "id": "minimax/minimax-m3.1-flash:free",
      "name": "MiniMax M3.1 Flash (free)",
      "context_length": 262144,
      "pricing": { "prompt": 0.0, "completion": 0.0 },
      "free": true,
      "supports_tools": true
    }
  ]
}
```

Provider failure (also `200`; the list is honestly unavailable):

```json
{
  "provider": "openrouter",
  "source": "none",
  "fetched_at": "2026-10-20T12:34:56Z",
  "models": [],
  "error": "the provider rejected the stored credential (HTTP 401)"
}
```

Field rules:

| field | presence | value |
| --- | --- | --- |
| `provider` | always | canonical provider id (`codex` is returned as `codex_cli`) |
| `source` | always | `openrouter`, `openai`, `cli`, `static`, or `none` |
| `fetched_at` | always | RFC 3339 UTC with seconds: `%Y-%m-%dT%H:%M:%SZ`; on a cache hit it is the original fetch time |
| `models` | always | array, 0..500 rows |
| `models[].id` | always | provider model id, `[A-Za-z0-9._:/-]{1,256}` (the chat request schema's pattern) |
| `models[].name` | always | human label, at most 200 code points, control characters removed; falls back to the id |
| `models[].context_length` | omitted when unreported | non-zero integer |
| `models[].pricing` | omitted when unreported | `{"prompt": <number>, "completion": <number>}` in the provider's own unit; both members always present together |
| `models[].free` | always | `true` when both prices are zero or the id ends with `:free` |
| `models[].supports_tools` | omitted when unreported | boolean |
| `error` | only on a provider failure | short sanitized reason; never a key, URL, or provider reason phrase |

`pricing` is passed through unconverted. OpenRouter publishes USD **per token**
(`"0.000003"` is returned as `0.000003`), so the picker must not claim a unit it
does not know (model-ui already follows this).

Status codes:

| status | body | when |
| --- | --- | --- |
| 200 | catalog envelope | success **or** any provider/transport/parse failure (failure is `source:"none"` + `error`) |
| 401 | `{code:"unauthorized",…}` + `WWW-Authenticate: Bearer` | missing/bad route bearer |
| 403 | `{code:"consent_required",…}` | remote provider requested without `allow_remote=true` |
| 403 | `{code:"cross_origin_denied",…}` | foreign `Origin` |
| 404 | `{code:"not_found",…}` | unknown provider id or unknown route |
| 405 | `{code:"method_not_allowed",…}` | non-GET |
| 421 | `{code:"invalid_host",…}` | foreign `Host` |
| 500 | `{code:"internal_error",…}` | settings file cannot be read (same as the other provider routes) |

Provider ids and behaviour:

| id | source | consent needed | list |
| --- | --- | --- | --- |
| `local` | `static` | no | one row: `deterministic-paraphrase-v1` |
| `openai` | `openai` | yes | `GET {base}/v1/models` |
| `openrouter` | `openrouter` | yes | `GET https://openrouter.ai/api/v1/models` |
| `claude_code` | `static` | no (no call) | documented aliases: `default`, `sonnet`, `opus`, `haiku` |
| `codex_cli` (alias `codex`) | `static` | no (no call) | documented ids: `default`, `gpt-5-codex`, `gpt-5`, `o3`, `o4-mini` |
| `opencode_cli` | `cli`, else `static` | yes | `opencode models --standalone`; on absence/error: empty list + `error` |

Any other id (including `cad`, `freecad`) is a 404.

Consent: only the literal `allow_remote=true` consents (`?allow_remote=true`,
`&allow_remote=true`, or percent-encoded `%74rue`). `false`, `TRUE`, `1`, an
empty value or a missing member does not. Consent is checked before the stored
key, before any CLI spawn and before any HTTP request (tested: zero calls on
refusal). The UI's "never auto-consent" rule therefore holds at the backend too.

OpenAI-compatible base URL: `OPENAI_BASE_URL` (default
`https://api.openai.com`). It must be HTTPS, except an HTTP loopback host
(`localhost`, `127.0.0.1`, `::1`) for local model servers; credentials in the
URL, a query and a fragment are refused. A trailing `/v1` is normalized away
before `/v1/models` is appended. There is no stored base-URL setting yet (see
Gaps).

## What was added

Public API in `src/providers/catalog.rs`:

- `pub async fn list_models(&ModelCatalog, provider_id, allow_remote, &environ, &StoredKeys, &dyn HttpTransport) -> Result<CatalogListing, CatalogFailure>`
- `pub fn canonical_provider(&str) -> Option<&'static str>`
- `pub fn query_flag_true(Option<&str>, &str) -> bool`
- `pub fn format_timestamp(u64) -> String`
- `pub struct ModelCatalog` with `new`, `with_clock(ttl, clock)`, `ttl_seconds`, `clear`
- `pub struct CatalogListing` (`provider`, `source`, `fetched_at`, `models`, `error`) and `to_json`
- `pub struct ModelEntry`, `ModelPricing`, `enum CatalogSource`, `enum CatalogFailure`
- constants: `MODEL_CACHE_TTL_SECONDS=600`, `CATALOG_REQUEST_TIMEOUT_SECONDS=15`,
  `MAX_CATALOG_RESPONSE_BYTES=4_000_000`, `MAX_MODELS=500`, `MAX_CACHE_ENTRIES=32`,
  `MAX_MODEL_NAME_LENGTH=200`, `MAX_BASE_URL_LENGTH=2048`,
  `CLI_MODEL_LIST_TIMEOUT_SECONDS=10`, `MAX_CLI_OUTPUT_BYTES=1_000_000`.

Implementation notes:

- **Cache:** one in-memory cache per `StudioService`, keyed by provider + SHA-256
  fingerprint of the key (and base URL for OpenAI). TTL 600 s, 32 entries max;
  expired rows are pruned on every insert and the oldest row is evicted when
  full. Only successful lists are cached; an error is retried on the next
  request. A cache hit opens no socket and preserves `fetched_at`.
- **Transport:** `HttpTransport` gained an additive `get` with a default that
  fails closed (existing mocks that only implement `post` still compile).
  `ReqwestTransport::get` reuses the same no-redirect client, per-request proxy
  resolution, timeout and streamed size cap as `post`.
- **CLI:** `opencode` is resolved on the configured `PATH`, run with an argv
  array (no shell), `--standalone`, stdin null, stderr null, a temp working
  directory, a 10 s wall clock and a 1 MB stdout cap; the environment is
  rebuilt from an allowlist (`PATH`, `HOME`, `XDG_*`, `TMP*`, `LANG`, `LC_*`,
  `TERM`) plus `NO_COLOR=1`. Nothing of Kala's configuration is inherited.
  Claude Code and Codex expose no model-listing command, so their lists are
  documented aliases marked `static`; the CLI passes any id through.
- **Errors:** HTTP status phrases from the provider are never echoed (they are
  server-controlled and could reflect the credential); internal transport
  failures collapse to `"the provider model list could not be fetched"`.

## Changed paths

New:

- `backend/crates/kala-studio/src/providers/catalog.rs`
- `backend/crates/kala-studio/tests/model_catalog.rs`
- `.mece/waves/agent-providers-20261020/cells/model-catalog/REPORT.md`

Shared files (minimal, additive):

- `backend/crates/kala-studio/src/providers/mod.rs`: `pub mod catalog;`
- `backend/crates/kala-studio/src/providers/remote.rs`: new `GetRequest`, the
  default `HttpTransport::get`, `ReqwestTransport::get`, and
  `credential` made `pub(crate)` for reuse (no behaviour change to chat).
- `backend/crates/kala-studio/src/server.rs`: `RouteKind::ProviderModels`, the
  path classifier, GET-only method table, route-level bearer gate, `State.catalog`
  + construction, the route handler, two unit-test updates and doc lines.

No Python file, schema/DTO, settings DTO, fixture, or other crate was modified.
No commit/push/sudo/delegation.

## Tests and gates

New tests: **19** — 13 unit tests in `providers::catalog` and 6 integration
tests in `tests/model_catalog.rs`.

Unit (13): OpenRouter mapping/free detection/headers; OpenAI-compatible base
URL + mapping; base-URL rules; cache TTL + key fingerprint; cache pruning/bound;
error envelopes (401, timeout, bad JSON, missing list, internal); consent refusal;
local/static lists and the `codex` alias; unknown providers; 500-row/200-name
bounds; query parsing; RFC 3339 timestamps; dynamic OpenCode listing with a fake
CLI and the `KALA_STUDIO_TOKEN` environment assertion; OpenCode fallback.

Integration (6): OpenRouter route mapping + cache; the server-held stored key
(used in `Authorization`, never returned); Host/Origin/bearer/method/consent
gates; provider failures inside the envelope; local/static/alias lists; and a
**real loopback HTTP server** exercising the production
`ReqwestTransport::get` path (`GET /v1/models`, bearer asserted).

Full crate gates (from `backend/crates/kala-studio`):

- `cargo fmt --check`: our files pass `rustfmt --check --edition 2024`; the
  crate-wide `cargo fmt --check` currently also flags
  `src/chat_design.rs`, which another cell is actively editing (not this cell's
  file, left untouched).
- `cargo clippy --all-targets --all-features -- -D warnings`: this cell's code
  is lint-clean (`cargo clippy --all-features --lib --test model_catalog --
  -D warnings` passes now; the full run passed before the concurrent llm-design
  cell added its new test file). A re-run at report time is blocked by that
  cell's still-in-progress `tests/chat_mock_provider_design.rs`
  (`clippy::let_and_return`); it is not this cell's file and was left untouched.
- `cargo test --locked` (debug): **201 passed, 0 failed** (123 lib + 78
  integration), including all 19 new tests. Four of those integration tests are
  the llm-design cell's new `chat_mock_provider_design.rs`; this cell's suites
  pass on their own too:
  `cargo test --locked --lib providers::catalog` → 13 passed;
  `cargo test --locked --test model_catalog` → 6 passed.
- `cargo test --locked --release`: **201 passed, 0 failed** (same tree point).
- `cargo build --locked --release`: pass.
- `cargo check --locked --target x86_64-pc-windows-gnu`: **blocked by the
  environment**, not by this code: the target is installed but the cross C
  compiler is absent —
  `error occurred in cc-rs: failed to find tool "x86_64-w64-mingw32-gcc": No such file or directory`.
  No Windows runtime claim is made.

Test note: on the first parallel release run one pre-existing
`step_import_limit` test flaked; the same binary and the full release suite
passed on rerun (201 tests). Nothing in that test exercises this cell.

## Security evidence

- Synthetic keys only (`sk-or-synthetic-*`, `sk-synthetic-*`); the tests assert
  the key never appears in a response body.
- The cache stores a SHA-256 fingerprint, never the key; the key is never logged.
- Provider reason phrases are discarded; error text is bounded and composed by
  this module.
- CLI environment allowlist is tested: a fake `opencode` prints
  `leaked/secret` when `KALA_STUDIO_TOKEN` is present, and the test asserts the
  real list is returned instead.
- Redirects are refused by the shared transport; HTTPS is required except
  loopback HTTP; responses are read with a 4 MB cap and parsed into at most 500
  bounded rows; names are control-character-free.
- Route bearer, Host and Origin gates are covered by integration tests; the
  consent refusal is checked before any credential or network/CLI work.
- No credential file, CLI auth state, or user settings file was read by this
  worker.

## Gaps / intentional differences

- **Base URL is environment-only.** The task allowed "any OpenAI-compatible base
  URL configured", but there is no stored settings field for it, so
  `OPENAI_BASE_URL` is the seam. A future settings DTO should feed the same
  validator.
- **OpenRouter `~` variant ids are filtered.** 19 of 470 live ids (for example
  `~deepseek/deepseek-pro-latest`) contain `~`, which the chat request schema
  rejects; returning an id that chat cannot accept would be a broken choice.
  Admit them after `MODEL_ID_PATTERN` widens.
- **Claude Code / Codex lists are static documentation aliases**, not a live
  probe: neither CLI exposes a machine-readable model list, and the Codex shim
  could not be installed in the isolated environment (mise: disk quota). They
  are marked `source:"static"` as required.
- **OpenCode listing is heuristic**: line-delimited `provider/model` tokens.
  An unfamiliar table shape degrades to the static empty list with an `error`
  rather than inventing rows.
- **Errors are HTTP 200 envelopes**, matching the model-ui contract
  (`catalogError`) and the task's `error?` field; only gate/unknown/method
  failures use status codes.
- **No request coalescing**: simultaneous cold requests for one provider may
  each fetch; the cache is per process and not persisted.
- **OpenRouter live shape checked once** (unauthenticated): 788,305 bytes, 470
  models, 20 free, max name 56 chars, max context 2,000,000 — within the 4 MB /
  500-row bounds, with 19 `~` ids filtered as above.
- No Python differential: this is a new Rust-only endpoint with no Python
  reference implementation; existing Python behavior is untouched.
