# model-catalog: list the models a configured provider can use
Read `../../WAVE.md` and `kala-studio/src/providers/**` + `settings.rs` (how stored keys are read server-side; keys never leave the backend).
Add `GET /api/studio/providers/{id}/models` (bearer/Host/Origin gates as the other routes) returning `{provider, source, fetched_at, models: [{id, name, context_length?, pricing?: {prompt, completion}, free: bool, supports_tools?: bool}], error?}`:
- openrouter: `GET https://openrouter.ai/api/v1/models` with the stored key (Authorization header); map the fields; `free` = both prices are 0 or the id ends with `:free`. Requires `allow_remote` consent semantics as for chat: the UI passes a consent flag (`?allow_remote=true`); without it, return 403 with a clear consent message.
- openai (and any OpenAI-compatible base URL configured): `GET {base}/v1/models`.
- claude_code / codex_cli / opencode_cli: use the CLI's own model listing if it has one (e.g. `opencode models`); otherwise a documented static list of the CLI's documented model aliases, marked `source: "static"`. Coordinate the provider ids with cli-providers (read its TASK.md).
- local: a single entry.
Cache per provider+key-fingerprint for 10 minutes (memory), bounded response size, timeouts, and errors mapped to the envelope. Never log or return the key.
Tests: a mock OpenRouter and an OpenAI server; mapping, free detection, cache, errors, the consent refusal, auth gates. Full Rust gates. REPORT.md with the exact JSON shape for model-ui.
