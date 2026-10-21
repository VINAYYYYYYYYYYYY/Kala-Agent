# Kala Studio providers

> Scope: which language model providers the chat-first studio agent
> exposes, how each is configured, the consent and isolation rules,
> and the primary sources the studio's behaviour is anchored to.
>
> This document is about **language-model** providers. The **CAD**
> providers — the engines that actually build the model — are a different
> thing and are documented in [cad-providers.md](cad-providers.md).
>
> **Current state (2026-10-21).** The studio is **Rust-only** (ADR-0010
> phase 2): the product runs the native `kala-studio` service, and the
> provider set, the consent gate, the credential handling and the
> refusals are the same there as in the Python reference server, which
> survives as the differential oracle. Nothing in this document depends on
> a Python process being present.

## The seven provider ids

The chat surface accepts **seven** provider identifiers, published by
`GET /api/studio/providers` in this order:

```
local, openai, openrouter, codex, claude_code, codex_cli, opencode_cli
```

The native service defines them in
`backend/crates/kala-studio/src/providers/mod.rs` (`PROVIDER_IDS`); the
Python reference server publishes the same seven ids.

| id | what it is | credential | model id |
|---|---|---|---|
| `local` | deterministic offline helper. No network, no prompt context beyond the request body itself. | none | optional, ignored |
| `openai` | direct HTTPS request to a fixed OpenAI endpoint. | `OPENAI_API_KEY` (Studio settings or the backend environment) | **required** |
| `openrouter` | direct HTTPS request to a fixed OpenRouter endpoint. | `OPENROUTER_API_KEY` (Studio settings or the backend environment) | **required** |
| `claude_code` | the operator's own **Claude Code** CLI, run as a sandboxed subprocess. | the operator's own CLI login — Kala never sees it | optional |
| `codex_cli` | the operator's own **Codex** CLI, run as a sandboxed subprocess. | the operator's own CLI login | optional |
| `opencode_cli` | the operator's own **OpenCode** CLI, run as a sandboxed subprocess. | the operator's own CLI login | optional |
| `codex` | the **historical id** of the Codex CLI route. It still resolves and still runs the same CLI; it is the same route as `codex_cli` under an older name. | the operator's own CLI login | optional |

`codex` and `codex_cli` name the same CLI, so the backend publishes two
rows with the same name and the same notes. The Studio's provider picker
hides the alias row when the canonical row is listed, and a stored
selection that still says `codex` is shown and sent as `codex_cli`.

The `chat` request schema is bounded: `provider`, optional `model`,
required `prompt`, `allow_remote: bool = False`, and the optional
`cad_provider` (the CAD target — see [cad-providers.md](cad-providers.md)).
The studio does not attach geometry, preview context, nominal geometry
estimates, model entitlement messages, or any client-side context to a chat
request. The `allow_remote` field is opt-in and gates every external
request.

The response is `{text, provider, model, offline}` plus an optional
`design` — the `DesignResult` — when the turn produced a real, measured
design. The `design` is `null` whenever nothing was built.

## The three subscription CLIs

The three subscription routes run the operator's **own, already
authenticated** command-line tool as a subprocess and let that tool talk
to its vendor. Kala never sees, stores or forwards the CLI's credential;
the CLI reads its own login from its own configuration directory.

### What they run

The prompt is **never** on the argv. It is written to the child's standard
input and the standard input is closed afterwards, so a prompt cannot
leak through the process table and cannot overflow the argument limit.

| route | argv |
|---|---|
| `claude_code` | `claude --print --output-format json --safe-mode --tools "" --strict-mcp-config --permission-mode dontAsk --no-session-persistence [--model <model>]` |
| `codex_cli` (and the `codex` alias) | `codex exec --sandbox read-only --skip-git-repo-check --ephemeral --ignore-user-config --json [--model <model>]` |
| `opencode_cli` | `opencode run --standalone --format json --agent kala-chat [--model <provider/model>]` |

What each flag is for:

| flag | why |
|---|---|
| `claude --print` | the CLI's non-interactive mode for scripted callers |
| `claude --output-format json` | one JSON object on stdout; the assistant's final text is its `result` field |
| `claude --safe-mode` | switches off `CLAUDE.md`, skills, plugins, hooks, MCP servers, custom commands and agents, while leaving the login, the model selection and the permissions alone |
| `claude --tools ""` | removes **every** built-in tool: no file read, no command, no URL fetch |
| `claude --strict-mcp-config` | no MCP server is connected, even when the operator configured one |
| `claude --permission-mode dontAsk` | anything that would still ask for a permission is denied, so a turn can never block |
| `claude --no-session-persistence` | the turn leaves no transcript in the CLI's session store |
| `codex exec` | the CLI's non-interactive mode; with no prompt argument it reads the instructions from standard input |
| `codex --sandbox read-only` | every command the model may run is confined to a read-only sandbox |
| `codex --skip-git-repo-check` | the turn runs in a private temporary directory, which is not a repository |
| `codex --ephemeral` | the turn leaves no session file in the CLI's store |
| `codex --ignore-user-config` | `$CODEX_HOME/config.toml` is not loaded: the operator's MCP servers, hooks and custom instructions stay out of the turn; the login still comes from `$CODEX_HOME` |
| `codex --json` | the turn's events as JSON lines |
| `opencode run` | OpenCode's non-interactive mode; with no message argument it reads the message from standard input |
| `opencode --standalone` | a private server for the turn instead of the operator's background service |
| `opencode --format json` | the turn's events as JSON lines |
| `opencode --agent kala-chat` | the agent the turn runs as; that agent **denies every action on every resource** and every tool is switched off in the configuration the child receives |

**Honest limit (Codex).** The Codex CLI has no flag that removes its shell
tool. `--sandbox read-only` plus `--ignore-user-config` is the strongest
read-only, no-agentic-extras posture the CLI offers; a read-only command
the model chooses to run can still read a file it can see.

### The sandboxing

| property | how it is enforced |
|---|---|
| no shell | the child is started from an argv array; nothing is handed to a shell |
| the prompt never on argv | the prompt goes to standard input, which is then closed |
| no tool use | `claude --tools ""`; OpenCode's deny-all agent; the Codex CLI's read-only sandbox (see the honest limit above) |
| a private working directory | every run gets a fresh temporary directory that is deleted when the turn ends, so the CLI cannot read or write the project the Studio happens to run in |
| a sanitized environment | the child starts from an **empty** environment and receives only `PATH`, `HOME`, the `XDG_*` names, `TMPDIR`/`TMP`/`TEMP`, `LANG`/`LC_ALL`/`LC_CTYPE` and `TERM` — plus `CLAUDE_CONFIG_DIR` (Claude) or `CODEX_HOME` (Codex), the two names a CLI needs to find its own login |
| no Kala secret reaches the CLI | no `OPENAI_API_KEY`, no `OPENROUTER_API_KEY`, no studio token, no data directory |
| bounded output | standard output ≤ 1 000 000 bytes, standard error ≤ 65 536 bytes; a larger output kills the child and fails the turn |
| no operator credential is read by Kala | the availability probe reads only the **exit status** of the CLI's own status command; the CLI's own text never leaves the probe |

**Honest limit (environment-var logins).** Because the child receives only
the allowlist, a CLI that the operator authenticated through an
environment variable (for example `ANTHROPIC_API_KEY`) reports itself as
signed out. The subscription logins (OAuth in `~/.claude`, the keychain,
`$CODEX_HOME/auth.json`, the OpenCode credential store) are found normally,
because `HOME` and the `XDG_*` names are forwarded.

### Consent

Every subscription route requires `allow_remote: true` in the request,
exactly like `openai` and `openrouter`. Without it the route answers
`400 provider_not_configured` and **the CLI is never started**:

```
provider=<id> requires allow_remote=true at request time
```

### The optional model, and the honest "CLI default model"

`model` is **optional** for the three subscription routes. When the request
names no model, the CLI's own default model answers and the studio says
exactly that — it never invents a model name, because the CLIs do not
report which model they picked (checked live on 2026-10-21).

* The literal id `default` means "the CLI's own default". It is **not**
  passed on the argv as `--model`; only a real model id is.
* The reply's `model` field is the model the CLI reported when it reported
  one, otherwise the literal `default`.
* The Studio shows this honestly. A reply that ran on the CLI's own
  default reads **`Codex CLI default model (chosen by Codex)`** (or
  `Claude Code default model (chosen by Claude Code)`, or `OpenCode CLI
  default model (chosen by OpenCode)`). A pinned model still reads
  `codex_cli · gpt-5-codex`.
* Under the model field the picker says: *"Leave empty to use <Provider>'s
  own default model — the CLI does not report which model it picked. Pick a
  row here to pin one; the reply then names it."*

### The 180 s budget, and killing the whole process tree

One CLI turn is bounded by **180 s** by default
(`KALA_STUDIO_CLI_TIMEOUT_SECONDS`, clamped to 1…3600 s).

Every turn is started in its **own process group** (`process_group(0)` on
Unix, `CREATE_NEW_PROCESS_GROUP` on Windows). A guard kills the **whole
group** — not only the direct child — when the turn times out, when it
fails, and when the client disconnects or the operator cancels:

* Unix: the process group is killed with `SIGKILL`, with a direct kill of
  the child as a backstop for a CLI that creates its own session.
* Windows: `taskkill /T /F /PID <pid>` walks the parent/child tree. A Job
  Object with `KILL_ON_JOB_CLOSE` would be stronger, but it needs a
  Windows host to land safely; the documented OS tool is used instead.

The private working directory is deleted when the turn ends, and the
timeout path leaves no temporary directory behind.

### Windows: the `.cmd` shims

An npm-installed Windows CLI is a **`.cmd` shim**, not an `.exe`. The
Studio resolves the executable the way Windows does:

* the `PATHEXT` extensions are searched in order (default
  `.COM;.EXE;.BAT;.CMD` when `PATHEXT` is unset), and an
  extension-less bare file is never chosen ahead of the shim;
* a resolved `.cmd`/`.bat` is run as `cmd.exe /d /s /c "<quoted line>"`
  with an argv array; the prompt stays on standard input;
* a control character in the argv is refused, because it would end the
  `cmd.exe` line early;
* on a non-Windows host a resolved `.cmd`/`.bat` is refused with a clear
  message instead of `Exec format error`.

The executable may also be pinned explicitly with
`KALA_STUDIO_CLAUDE_CLI_PATH`, `KALA_STUDIO_CODEX_CLI_PATH` or
`KALA_STUDIO_OPENCODE_CLI_PATH`; an override gets the platform's
extension appended.

### The fast availability probe

`GET /api/studio/providers` never waits for a CLI:

* each probe runs the CLI's own `--version` and, when it has one, its own
  authentication-status command (`claude auth status`, `codex login
  status`, `opencode auth list --format json`), and reads **only the exit
  status**;
* a probe is capped at **2 s** per CLI and its verdict is cached for
  **120 s**;
* the routes (`/providers`, `/settings`, `/health`) answer from the cache
  at once and refresh it in the background. A first probe that has not
  finished inside the **600 ms** route budget is reported as
  `available: null` with the honest reason **`checking...`** — never as
  "unavailable" and never as a guessed login;
* a CLI that is installed but does not answer its probe within 2 s is
  reported as *installed but its availability check did not finish within
  2 seconds*.

## The local helper (`provider: "local"`)

The local provider is the default. It returns a deterministic,
templated response derived from the prompt string alone: it extracts
any numeric dimensions from the prompt, computes nominal analytic
volume and surface-area estimates for a box, cylinder, or sphere, and
restates the OpenCascade estimate / neutral-handoff workflow. It
does not issue network requests and does not look at the active preview
geometry, the active mesh, or the integration registry. It never
invents engineering facts (material strength, factor of safety,
fatigue life, etc.); when the prompt asks for those, it returns an
honest refusal naming the limitation. The local helper is suitable
for offline drafting and unit-test scenarios.

For a **design** prompt the local helper does one extra thing: it runs
the **strict intent parser**. In the native build that parser is
`kala-design-intent` (Rust); the Python reference server runs the same
contract from `src/kala_agent/studio/design/intent.py`. Either returns a
complete intent, or it returns the exact list of values it still needs:

```
Local helper. I can draft a L-bracket, but I need these declared values:
width (mm), depth (mm), height (mm), wall thickness (mm)
```

It never guesses a dimension. When the intent *is* complete, the turn
continues into the CAD-provider step; see
[cad-providers.md](cad-providers.md) for what happens next when no CAD
target has been chosen.

## OpenAI (`provider: "openai"`)

The native studio service makes a direct HTTPS request to the OpenAI
chat-completions endpoint (`https://api.openai.com/v1/chat/completions`).
It does not follow redirects, does not
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
- The base URL may be overridden with `OPENAI_BASE_URL` for an
  OpenAI-compatible local model server. It must be HTTPS, except for an
  HTTP loopback host, and a credential in the URL, a query or a fragment
  is refused.

This is a paid-API route. The studio never uses free credits or
quota as a substitute for a paid plan.

## OpenRouter (`provider: "openrouter"`)

Same shape as OpenAI, with `OPENROUTER_API_KEY` configurable via the Studio **API Keys** settings UI (`POST /api/studio/settings`) or the backend environment, and a fixed endpoint
(`https://openrouter.ai/api/v1/chat/completions`). OpenRouter is an
HTTP aggregator; the studio backend treats it identically to
OpenAI: fixed endpoint, no redirects, bounded body, bounded
timeout. The studio does not pre-approve any specific model on
OpenRouter; it forwards the user-supplied `model` identifier
verbatim and reports any error from the aggregator.

## The time budgets

| budget | value | where it applies |
|---|---|---|
| plain chat | **30 s** | every OpenAI-compatible chat request (`openai`, `openrouter`) |
| design turn | **180 s** | the first design call **and** the one repair call of that turn |
| subscription CLI turn | **180 s** | `claude_code`, `codex_cli`, `opencode_cli` (own budget, `KALA_STUDIO_CLI_TIMEOUT_SECONDS`, 1…3600 s) |
| model catalog fetch | 15 s | one `GET …/models` request |
| CLI model listing | 10 s | `opencode models` |
| provider availability probe | 2 s per CLI | the cached background probe |

The budgets are chosen by the server, never by the request body. The
**worst case of a design turn is two model calls** — the first call plus
one repair — so a design turn can take up to 360 s of provider time.

## The model catalog

`GET /api/studio/providers/{id}/models?allow_remote=true`

The route carries its own bearer gate. It answers `200` with the catalog
envelope in both the success and the provider-failure case; only the
gates use a status code.

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

| field | meaning |
|---|---|
| `provider` | the canonical provider id (`codex` is returned as `codex_cli`) |
| `source` | `openrouter`, `openai`, `cli`, `static` or `none` |
| `fetched_at` | RFC 3339 UTC with seconds; on a cache hit it is the original fetch time |
| `models` | 0…500 rows |
| `models[].free` | true when both prices are zero **or** the id ends with `:free` |
| `models[].context_length`, `pricing`, `supports_tools` | omitted when the provider did not report them |
| `error` | only on a provider failure: a short sanitized reason, never a key or a URL |

Where the list comes from:

| id | source | consent |
|---|---|---|
| `local` | `static` — one row, `deterministic-paraphrase-v1` | no |
| `openai` | `openai` — `GET {base}/v1/models` | yes |
| `openrouter` | `openrouter` — `GET https://openrouter.ai/api/v1/models` | yes |
| `claude_code` | `static` — the documented aliases `default`, `sonnet`, `opus`, `haiku` | no (no call) |
| `codex_cli` (alias `codex`) | `static` — the documented ids `default`, `gpt-5-codex`, `gpt-5`, `o3`, `o4-mini` | no (no call) |
| `opencode_cli` | `cli` — `opencode models`; on an absence or an error an empty list plus `error` | yes |

Any other id (including `cad` and `freecad`) is a `404`.

The OpenRouter list is the **live** list, so it carries the free models
too — on 2026-10-21 the unauthenticated list held 468 models, 20 of them
free (ids ending in `:free`), and the maximum reported context length
was 2 000 000 tokens. Nineteen of the live ids contain a `~`, which the
chat request schema does not accept, so they are filtered out rather than
offered as a choice the chat route could not use.

A successful remote listing is cached in memory for **600 s** per provider
and key (the cache is keyed by a SHA-256 fingerprint of the key, never by
the key itself); at most **32** entries are kept. Only successful lists
are cached — an error is retried on the next request.

Consent is checked **before** the stored key, before any CLI spawn and
before any HTTP request, so a refused listing opens no socket. Only the
literal `allow_remote=true` consents.

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

## What `offline` means now

`offline` is an **evidence-backed answer, not an API-key check**.

* In `GET /api/studio/health` and `GET /api/studio/settings` it means
  *"no remote route is known to be available"*: no OpenAI-compatible
  credential is configured **and** no subscription CLI is installed and
  signed in. A signed-in CLI therefore makes the studio **online** even
  with no stored key. While the first CLI probe is still running the
  field stays conservative (`true`) for at most the 600 ms route budget.
* In a **chat reply** it means *"this particular turn did not leave the
  machine"*. Only the local helper answers `offline: true`; every
  remote and every subscription-CLI turn answers `false`, because the CLI
  talks to its vendor.

## What the studio never does

- Sends a credential back to the renderer. The renderer is told only
  the boolean `configured: true | false` and `source: "stored" | "environment" | "none"`.
- Reads a CLI's credential store. The probe reads only the exit status of
  the CLI's own status command, and the CLI's own text never reaches a
  response — so the studio can say *that* a CLI is signed in, never *as
  whom*.
- Auto-selects a **language-model** provider. The renderer's default is
  `local`. A remote provider must be picked explicitly.
- Auto-selects a **CAD** provider. The CAD target starts empty and
  must be chosen in the composer; with no choice the design turn asks
  which provider to use and builds nothing. See
  [cad-providers.md](cad-providers.md).
- Sends geometry or estimates to a provider. The workbench's nominal
  volume, surface-area, and bounding-envelope estimates are computed
  locally from validated parameters; a chat prompt carries only its
  text. A **design** result is the opposite case and is stated
  plainly: the measured numbers come from the CAD provider that built
  the model, and the card says which one.
- Caches prompts or responses on disk. Each call is stateless.
- Sends telemetry to a third party. The studio makes no outbound
  connection other than the chosen provider call and the model catalog
  listing; the CAD-provider probes are local (`FreeCADCmd` runs as a child
  process, the Solid Edge bridge is a loopback address).

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
- OpenRouter model list:
  <https://openrouter.ai/api/v1/models>.
- OpenCascade.js (WASM) project, used as a future browser-side
  B-rep option but not part of this wave's preview:
  <https://github.com/donalffors/opencascade.js>.

## Trust model recap

- The hardened core (`src/kala_agent/`) does not import
  `kala_agent.studio`. The studio service is the only place a
  provider call exists.
- Every route that leaves the machine — the two API-key routes and the
  three subscription CLI routes — requires `allow_remote=true`. The
  subscription routes run the operator's own CLI; the API-key routes call
  the provider's fixed endpoint.
- The **CAD** providers are a separate trust domain. The FreeCAD
  provider runs a `FreeCADCmd` subprocess with a **scrubbed**
  environment: the credential-shaped keys (`TOKEN`, `API_KEY`,
  `SECRET`, `PASSWORD`, `CREDENTIAL`) are removed before the driver
  starts, so the CAD engine never inherits the chat API key (nor, in
  the development fallback, the geometry-worker token). The Solid Edge
  provider reaches only a
  loopback bridge and reads its token without logging it.
- The desktop shell only injects a bearer token. It has no access
  to provider credentials.
- The renderer has no access to `process.env`, `fs`, or
  `child_process` in Electron production builds (sandbox: true,
  nodeIntegration: false, contextIsolation: true).
- The release workflow does not upload any environment variable, does
  not authenticate against any provider, and does not enable any
  API/credentials workflow. See
  [studio-limitations.md](studio-limitations.md).
