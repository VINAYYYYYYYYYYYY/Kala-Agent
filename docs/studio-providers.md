# Kala Studio providers

> Scope: which language model providers the chat-first studio agent
> exposes, how each is configured, the consent and isolation rules,
> and the primary sources the studio's behaviour is anchored to.

The studio chat surface accepts one of four provider identifiers
defined by `kala_agent.studio.schemas.ProviderId`:

- `local` — deterministic offline helper. No network, no prompt
  context beyond the request body itself.
- `codex` — the ChatGPT-subscription route. **Not implemented in this
  build**: the route fails closed with a sanitised `503` and sends no
  request. See below.
- `openai` — direct HTTPS request to a fixed OpenAI endpoint.
  Requires `OPENAI_API_KEY` configured in Studio settings or the backend environment.
- `openrouter` — direct HTTPS request to a fixed OpenRouter
  endpoint. Requires `OPENROUTER_API_KEY` configured in Studio settings or the backend environment.

The `chat` request schema is bounded: `provider`, optional `model`,
required `prompt`, and `allow_remote: bool = False`. The studio
does not attach geometry, preview context, nominal geometry estimates,
model entitlement messages, or any client-side context to a chat
request. The `allow_remote` field is opt-in and gates every external
HTTPS request.

## Local helper (`provider: "local"`)

The local provider is the default. It returns a deterministic,
templated response derived from the prompt string alone: it extracts
any numeric dimensions from the prompt, computes nominal analytic
volume and surface-area estimates for a box, cylinder, or sphere, and
restates the OpenCascade (OCP) estimate / neutral-handoff workflow. It
does not issue network requests and does not look at the active preview
geometry, the active mesh, or the integration registry. It never
invents engineering facts (material strength, factor of safety,
fatigue life, etc.); when the prompt asks for those, it returns an
honest refusal naming the limitation. The local helper is suitable
for offline drafting and unit-test scenarios.

## Codex CLI (`provider: "codex"`)

The `codex` route is the ChatGPT-subscription path, and in this build
it is **disabled by design**. Every request returns a sanitised `503`
(`provider_unavailable`); the studio does not launch the Codex CLI,
does not read its authentication material, and does not send the
prompt anywhere.

The reason is safety, not an oversight: a correct Codex integration
needs a reviewed subprocess wrapper that pins the CLI's documented
flags (`codex exec --sandbox read-only --skip-git-repo-check
--ephemeral --ignore-user-config`), passes the prompt on stdin only, runs
with a minimal sanitised environment, enforces a bounded deadline,
reaps the child, and caps the reader buffer. Shipping a half-built
subprocess path could leak prompt content through argv, environment
variables, or buffered stderr, so the studio refuses the route until
that wrapper exists and has been acceptance-tested against the real
CLI. This wave did **not** sign in to a ChatGPT subscription, did not
run the CLI, and does not claim the route works.

Consequences the studio states honestly:

- `/api/studio/providers` always reports the `codex` row with
  `configured: false`. A `codex` binary merely present on `PATH` does
  **not** make the provider usable, and the studio does not report
  the CLI's location.
- The `KALA_STUDIO_ENABLE_CODEX` flag is inert: it is recorded for
  diagnostics only and never enables the route.
- The route still requires `allow_remote: true` (the same consent
  gate as the API providers) before it returns the `503`, so a client
  can never mistake the refusal for a successful send.
- The subscription and the API routes are separate services billed
  separately by the provider. The studio never converts a `codex`
  request into a paid API call and never reuses an inherited
  `OPENAI_API_KEY` for the subscription path.

## OpenAI (`provider: "openai"`)

The studio backend makes a direct HTTPS request to the OpenAI
chat-completions endpoint. It does not follow redirects, does not
proxy through any third party, and rejects a body that does not
match the schema. It truncates the response to a bounded size and
rejects a non-success HTTP status.

Configuration:

- `OPENAI_API_KEY` can be set directly in the Studio UI (**API Keys** button in the Mechanical Engineering Agent header or the **Providers** workspace in the left sidebar) via `POST /api/studio/settings`, or inherited from the backend's process environment.
- When saved through Studio settings, the key is written atomically to `<studio-data-dir>/studio-settings.json` with `0o600` (owner-only) permissions.
- The route returns a `missing_api_key` error when no key is configured; the studio does not refuse to start, so other providers remain usable.
- The renderer cannot read stored or environment keys: `GET /api/studio/settings` and `POST /api/studio/settings` return only `configured: true | false` and `source: "stored" | "environment" | "none"`.
- The route requires `model`. The studio does not pre-approve any
  model; the user supplies the identifier and the studio forwards
  it to the configured endpoint.

This is a paid-API route. The studio never uses free credits or
quota as a substitute for a paid plan.

## OpenRouter (`provider: "openrouter"`)

Same shape as OpenAI, with `OPENROUTER_API_KEY` configurable via the Studio **API Keys** settings UI (`POST /api/studio/settings`) or the backend environment. OpenRouter is an
HTTP aggregator; the studio backend treats it identically to
OpenAI: fixed endpoint, no redirects, bounded body, bounded
timeout. The studio does not pre-approve any specific model on
OpenRouter; it forwards the user-supplied `model` identifier
verbatim and reports any error from the aggregator.

## Remote-prompt consent

Every call to `POST /api/studio/chat` with `provider` other than
`local` requires `allow_remote: true` in the body. If the field
is missing or `false`, the backend refuses the request. The
renderer surfaces a consent dialog before sending the request.

This is an opt-in double-step:

1. The user picks a remote provider in the UI; the UI does not
   silently upgrade the local helper.
2. The user checks the "Allow this prompt to leave the device"
   box; the backend then proceeds.

A prompt is not sent to a remote endpoint until both conditions
are met.

## What the studio never does

- Sends a credential back to the renderer. The renderer is told only
  the boolean `configured: true | false` and `source: "stored" | "environment" | "none"`.
- Invokes the Codex CLI at all in this build: the `codex` route
  refuses before any child process, argv, or environment is
  constructed, so no CLI flag is ever passed.
- Auto-selects a provider. The renderer's default is `local`. A
  remote provider must be picked explicitly.
- Sends geometry or estimates to a provider. The workbench's nominal
  volume, surface-area, and bounding-envelope estimates are computed
  locally from validated parameters; a chat prompt carries only its
  text.
- Caches prompts or responses on disk. Each call is stateless.
- Sends telemetry to a third party. The studio makes no outbound
  connection other than the chosen provider call.

## Primary sources

The provider behaviour is anchored to the following primary
sources checked on October 18, 2026. The studio does not assert
terms-of-service claims beyond what these sources state; provider
eligibility is the provider's decision, not the studio's.

- Codex CLI authentication and the separation between ChatGPT
  subscription access and OpenAI API credentials:
  <https://learn.chatgpt.com/docs/auth> (redirect target of
  https://developers.openai.com/codex/auth).
- Codex CLI noninteractive mode and the read-only default:
  <https://developers.openai.com/codex/noninteractive>.
- OpenRouter chat-completion request shape:
  <https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request>.
- OpenCascade.js (WASM) project, used as a future browser-side
  B-rep option but not part of this wave's preview:
  <https://github.com/donalffons/opencascade.js>.

## Trust model recap

- The hardened core (`src/kala_agent/`) does not import
  `kala_agent.studio`. The studio backend is the only place a
  provider call exists.
- Only the `openai` and `openrouter` routes make an outbound
  request, and only when `allow_remote=true` and the matching key is
  configured. The `codex` route never leaves the process.
- The desktop shell only injects a bearer token. It has no access
  to provider credentials.
- The renderer has no access to `process.env`, `fs`, or
  `child_process` in Electron production builds (sandbox: true,
  nodeIntegration: false, contextIsolation: true).
- The CI matrix does not upload any environment variable, does
  not authenticate against any provider, and does not enable any
  API/credentials workflow. See
  [studio-limitations.md](studio-limitations.md).
