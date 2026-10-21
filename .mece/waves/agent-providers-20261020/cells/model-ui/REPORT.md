# model-ui — the Studio provider picker and the model picker

Cell: `agent-providers-20261020/model-ui` (worker model-ui, MiniMax M3.1 Flash).

Scope: `studio/src/**` and `studio/test/**` only, plus this report. No commit, no
push, no sudo, no delegation. No credential file, no user settings file, and no
provider credential was read: every key in this cell is a synthetic string in a
unit test, and no live provider was contacted.

Status: **complete for the requested surface.** The composer now has a
listbox-based **provider picker** (the local helper, the two API-key routes,
and the three subscription CLIs, with availability badges and the backend's own
reasons) and a searchable **model picker** with a "Free only" filter, a context
length, a price, a consent-gated fetch, and an explicit state for every outcome.
All three Studio gates pass.

---

## 1. What the operator gets

### 1.1 The provider picker (`studio/src/components/chat/ProviderPicker.tsx`)

Replaces the composer's native `<select id="chat-provider-select">`. The rows
are the backend's own provider registry (`GET /api/studio/providers` and the
same list inside `GET /api/studio/settings`); the UI renders what the backend
reported and adds nothing of its own:

| row | what it shows |
| --- | --- |
| name | the backend's `name` |
| id | the backend's `id` (so `claude_code`, `codex_cli`, `opencode_cli` are visible) |
| badge | `Configured` / `Not configured` / `Unavailable` |
| subscription badge | `Subscription (CLI)` for `claude_code`, `codex_cli`, `opencode_cli` and the `codex` alias |
| credential line | where the credential comes from: a key stored in the Studio settings, a key from the environment, or the operator's own CLI session |
| reason | the backend's `reason` (and `notes`) for a provider it probed and found unavailable |

Rules:

* **An unavailable provider cannot be picked.** `available === false` is the
  backend's own probe verdict (the CLI is not installed, not signed in, …); the
  row is `aria-disabled="true"` and carries the reason.
* **A provider without a credential CAN be picked.** The operator has to be
  able to see *why* a route is unusable and where the key goes; the composer
  still refuses to send until the backend reports the provider configured.
* The popup is a real listbox: `role="listbox"` / `role="option"`,
  `aria-selected`, `aria-disabled`, `aria-activedescendant`, DOM focus moves
  into the listbox, `ArrowDown`/`ArrowUp`/`Home`/`End` move the active row,
  `Enter`/`Space` pick it, `Escape` closes it and returns the focus to the
  trigger, `Tab` closes it. A pointer click outside closes it.

### 1.2 The model picker (`studio/src/components/chat/ModelPicker.tsx`)

A combobox (`role="combobox"` + `role="listbox"`) that replaces the free-text
model input:

* **search** over the model id and the name; every whitespace-separated term
  must match, so `claude sonnet` narrows the list instead of widening it;
* **"Free only"** toggle, filtering on the backend's own `free` flag;
* each row shows the name, the id, the **context length**
  (`200k ctx`; the exact token count is in the tooltip), the **price**
  (`in 0.000003 · out 0.000015` — no unit is claimed, because the backend
  passes the provider's own numbers through unconverted), a `Free` badge, and
  a `tools` mark;
* the chosen id is sent as `model` in the chat request;
* the list is bounded at 500 rows, the backend's own `MAX_MODELS`, so the count
  the picker prints is the number of models the provider reported and never a
  silently truncated list.

### 1.3 The states (each one has a test)

| state | what the operator sees |
| --- | --- |
| no provider selected | "Choose a provider to list its models." — nothing is requested |
| provider not configured | "… is not configured server-side, so it has no model list yet." + an **Open API keys** button — nothing is requested |
| loading | "Loading the model list for OpenRouter…" |
| consent required | the backend's own 403 body (`consent_required: the remote model listing requires explicit consent (403) — pass allow_remote=true to list models for provider 'openrouter'`) + a **Grant consent and load models** button that ticks the composer's consent stamp |
| failed listing | the backend's message + **Retry** |
| error inside the envelope | the backend's `error` string (HTTP 200 + `source:"none"` + `error`) |
| no models | "OpenRouter reported no models." |
| no match | "No model matches “zzz”." and, with the filter on, "No free model matches “zzz”. Turn off “Free only” to see the other N models." |
| list | the rows above, with the count and the catalog `source` in the tooltip |

---

## 2. The contract this cell consumes

Taken from `cells/model-catalog/REPORT.md` (landed) and the implementation it
describes (`backend/crates/kala-studio/src/providers/catalog.rs`).

```
GET /api/studio/providers/{id}/models?allow_remote=true|false
Authorization: Bearer <studio token>          ← the route's own bearer gate
```

Success (200):

```json
{
  "provider": "openrouter",
  "source": "openrouter",
  "fetched_at": "2026-10-20T12:34:56Z",
  "models": [
    { "id": "minimax/minimax-m3.1-flash:free", "name": "MiniMax M3.1 Flash (free)",
      "context_length": 262144, "pricing": { "prompt": 0.0, "completion": 0.0 },
      "free": true, "supports_tools": true }
  ]
}
```

Provider failure (also 200, `models: []` + `error`); gate failures use status
codes: 401 `unauthorized`, 403 `consent_required`, 403 `cross_origin_denied`,
404 `not_found`, 405 `method_not_allowed`, 421 `invalid_host`, 500
`internal_error`.

The UI's validator (`studio/src/api/validators.ts`,
`parseProviderModelCatalog`) accepts exactly that shape and **refuses**
anything else rather than rendering half a list:

* `provider`, `source` and `models` are required; `fetched_at` and `error` are
  optional (an absent `fetched_at` is "not reported", never an invented time);
* `context_length`, `pricing` and `supports_tools` are optional — an omitted
  field is "the provider did not report it" and is carried as `null`, never
  filled in by the UI;
* `pricing.prompt` / `pricing.completion` must be numbers and `free` must be
  a boolean: a malformed row raises `StudioValidationError` with the dotted
  path of the offending field;
* `name` falls back to the id when the backend omits it, exactly as the
  backend documents.

### Provider ids

The closed set is `local`, `openai`, `openrouter`, `codex` (the historical
alias), `claude_code`, `codex_cli`, `opencode_cli` — the same set the catalog
route canonicalises (`canonical_provider`). `studio/src/types/studio.ts`
carries it as `ProviderId` / `PROVIDER_IDS`, and the chat-response validator
refuses a provider outside it.

### The provider policy (`studio/src/providerPolicy.ts`)

One place decides what a provider id means for the composer:

| id | leaves the machine | model id |
| --- | --- | --- |
| `local` | no (loopback only) | optional, ignored by the helper |
| `openai`, `openrouter` | yes → consent stamp | **required** |
| `claude_code`, `codex_cli`, `opencode_cli`, `codex` | yes → consent stamp | optional (the CLI's own default model) |

This replaces the composer's old `REMOTE_PROVIDERS` set and its
`requiresModel = !isLocal` rule, which would have blocked a subscription CLI
that needs no model id.

---

## 3. The consent model (ADR-0001)

* The model listing is remote egress, so the request carries the operator's
  consent stamp in `?allow_remote=`. **The UI never sets the flag itself**: it
  passes the value of the composer's own consent checkbox, and the backend
  refuses the request (403 `consent_required`) when the flag is absent. The
  backend checks consent before the stored key, before any CLI spawn and
  before any HTTP request, so a refused listing opens no socket.
* The consent stamp is per route: changing the provider clears it, and the
  "Grant consent and load models" button in the consent-required state ticks
  the same checkbox the operator sees under the composer.
* A provider that needs no egress (`local`, and the static `claude_code` /
  `codex_cli` lists) is listed without any consent at all — the backend
  answers those without a flag, and the UI simply asks.
* The consent state is only reached from a 403 whose code or message names
  consent, egress or `allow_remote`. A `cross_origin_denied` 403 is shown as
  the error state, not as a request for consent.

## 4. The storage rule

The remembered choice is the **model id and nothing else**:

```
localStorage["kala-studio.model.<provider id>"] = "<model id>"
```

No key, no credential, no prompt, and no provider secret is written. A test
walks the whole storage and asserts that every key is a model key and that no
value contains `sk-or`. The model id of each provider is restored when the
operator switches back to that provider.

## 5. Changed paths

New:

| path | what it is |
| --- | --- |
| `studio/src/components/chat/ProviderPicker.tsx` | the provider listbox |
| `studio/src/components/chat/ModelPicker.tsx` | the model combobox and its states |
| `studio/src/providerPolicy.ts` | the provider policy and the provider-status reader |
| `studio/src/modelCatalog.ts` | the catalog helpers: filter, formatting, the remembered choice |
| `studio/src/styles/models.css` | the drafted-sheet styles of both controls |
| `studio/test/ProviderPicker.dom.test.tsx` | the provider picker tests |
| `studio/test/ModelPicker.dom.test.tsx` | the model picker tests |
| `studio/test/modelCatalog.test.ts` | the contract, the filter, the formatting, the storage, the policy |
| `studio/test/providerFixtures.ts` | the test helper that drives the picker |

Modified:

| path | change |
| --- | --- |
| `studio/src/types/studio.ts` | `ProviderId` + `PROVIDER_IDS` (+ the three subscription ids and the `codex` alias), `ProviderRecord` gains the optional `available` / `reason` / `source` fields the backend's `ProviderRecord` now carries, the `ProviderModel` / `ProviderModelCatalog` types |
| `studio/src/api/validators.ts` | `parseProviderModelCatalog`; the chat provider set widened |
| `studio/src/api/client.ts` | `getProviderModels(connection, providerId, allowRemote, signal)` — a 20 s budget, the bearer, and the validated envelope |
| `studio/src/components/chat/ChatPanel.tsx` | the provider and model controls replace the `<select>` and the text input; the per-provider consent and the per-provider remembered model |
| `studio/src/components/panels/ProviderPanel.tsx` | the availability badge, the reason, and the subscription-CLI note (no key form for a CLI) |
| `studio/src/styles/chat.css` | the route row is one column; the model picker gets its own full-width row |
| `studio/test/AgentWorkspace.dom.test.tsx` | the picker drives the provider choice; the consent stamp is addressed by its own selector |
| `studio/test/ChatAgentCard.dom.test.tsx` | the picker drives the provider choice |
| `studio/test/ChatPanel.dom.test.tsx` | the picker drives the provider choice; five new composer-route tests |
| `studio/test/chatJournal.dom.test.tsx` | the picker drives the provider choice |
| `studio/test/nativeBackendPackaging.test.ts` | the Rust route snapshot now carries `GET /api/studio/providers/{}/models` (7 native routes, 17 in total) |

The `nativeBackendPackaging` change is a consequence of the **model-catalog**
cell's route landing in the Rust route table that the readiness checker parses;
it is a snapshot of that table, not a Studio change.

## 6. Tests and gates

New tests: **50** (19 in `ModelPicker.dom.test.tsx`, 9 in
`ProviderPicker.dom.test.tsx`, 22 in `modelCatalog.test.ts`), plus 5 new
composer-route tests in `ChatPanel.dom.test.tsx`.

* **Model picker** — the eight states; the consent flag is the operator's
  (`allow_remote=false` until the stamp is ticked, `true` after); a refusal
  offers the grant button; a failure retries; the envelope error is shown; the
  search and the free-only filter narrow the list; the keyboard picks a model
  (`ArrowDown` + `Enter`); `Escape` and an outside click close the list
  without changing the choice; the choice is stored per provider and only the
  model id is stored; one provider's models are never shown under another
  provider; the list is asked for as soon as the provider becomes configured.
* **Provider picker** — the rows, the badges, the subscription mark, the
  credential line; an unavailable provider cannot be picked and shows its
  reason; a provider without a credential can; the popup closes and returns
  the focus; the arrow keys and `Enter` pick; `Escape` closes without picking;
  the popup opens from the keyboard.
* **Catalog contract and helpers** — the exact envelope is accepted; a
  malformed payload is refused with the dotted field path; the filter, the
  formatting, the storage, and the provider policy.
* **Composer route** — the picked model is the `model` of the turn; each
  provider remembers its own model; a subscription CLI may send without a model
  id but still needs the consent stamp; changing the route clears the stamp;
  the "Open API keys" button of an unconfigured route opens the key section.

Existing tests updated (the provider `<select>` is gone): `AgentWorkspace`,
`ChatAgentCard`, `ChatPanel`, `chatJournal`.

Gates, run in `studio/`:

| gate | result |
| --- | --- |
| `npm run typecheck` | pass (no errors) |
| `npm test` | **46 files, 683 tests, 0 failures** (was 43 files / 628 tests) |
| `npm run build` | pass (`tsc --noEmit` + `vite build`, 607 ms) |

The Studio design-system guard test (the only px font sizes live in
`tokens.css`, no rounded corners, the 4 px rhythm) passes with the new
stylesheet.

## 7. Deliberate decisions and gaps

1. **The provider `<select>` is gone.** It could not carry the badges, the
   reasons, or the credential source, and it disabled the unconfigured
   providers — which hid the reason a route was unusable. The replacement is a
   listbox with the same keyboard contract.
2. **The model list is asked for only when the provider is configured.** The
   backend would answer an unconfigured provider with an error envelope; the UI
   already says "not configured" and offers the key form, and the brief asks
   for the listing "after a provider is configured (or a key is saved)". The
   flip from unconfigured to configured (the `server/settings` dispatch of the
   key form) re-runs the fetch.
3. **The price is printed without a unit.** The backend passes the provider's
   numbers through unconverted and does not say whether they are per token or
   per million tokens, so the picker does not claim one. The exact values are
   in the row's tooltip.
4. **The context length is abbreviated** (`200k ctx`); the exact token count is
   in the tooltip. A provider that does not report one shows nothing — the UI
   never substitutes a number.
5. **The static CLI lists** (`default`, `sonnet`, `opus`, `haiku` for Claude
   Code; `default`, `gpt-5-codex`, `gpt-5`, `o3`, `o4-mini` for Codex) come
   from the backend as `source:"static"` and are shown like any other list.
   The picker does not special-case them.
6. **The `codex` alias** is canonicalised to `codex_cli` by the backend. The
   picker remembers the choice under the id the operator picked, and the
   catalog is matched on the **requested** id, so the alias works.
7. **The model input is still a text field.** A model id the provider did not
   list can be typed by hand; the typed text is what the chat request carries.
8. **`studio/README.md` and `docs/` were not touched**: they are outside this
   cell's ownership (`studio/src/**`, `studio/test/**`). The controls are
   documented here instead.

## 8. What the other cells must provide (checked, not assumed)

* **model-catalog** — landed. The route, the envelope, the status codes, the
  consent semantics and the provider ids above are all implemented against the
  report and the source; the Studio validator accepts that exact shape and
  refuses anything else. One shared-file consequence was carried here: the
  Rust route snapshot in `studio/test/nativeBackendPackaging.test.ts` gained
  `GET /api/studio/providers/{}/models` (7 native routes, 17 in total, and
  `extraNativeRouteMethods` now carries the models route next to the STEP
  import).
* **cli-providers** — its REPORT had not landed when this cell finished, but its
  source in the tree was read and matches what this cell was built against:
  `PROVIDER_IDS = [local, openai, openrouter, codex, claude_code, codex_cli,
  opencode_cli]` (the `codex` alias included), the `ProviderRecord` fields
  `available: Option<bool>`, `reason: Option<String>`,
  `source: Option<&'static str>` with `SUBSCRIPTION_SOURCE = "subscription
  (CLI)"`, and the model rule "the CLI falls back to its own default model"
  (`--model` is passed only when the request names one, and the alias
  `default` means the CLI's own default). The picker reads the three
  optional fields **optionally**: a backend that does not send them still
  renders, and the row falls back to `configured` + `notes`. If the
  cli-providers cell lands different ids or field names, the change is
  confined to `studio/src/providerPolicy.ts` and the two `ProviderRecord`
  optional fields in `studio/src/types/studio.ts`.
* **llm-design** — no Studio surface: the design turn's schema prompt and the
  repair round-trip live in the backend, and the composer already sends the
  chosen `model` with the turn.

## 9. Reproduce

```sh
cd studio
npm run typecheck
npm test
npm run build
```
