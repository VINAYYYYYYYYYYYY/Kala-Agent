# Cell ui-fixes — REPORT

The UI-only findings of `../audit-ui/AUDIT.md` that the Opus adjudication kept, fixed in
`studio/src/**` and `studio/test/**` only.

**No backend file, no `studio/electron/**`, no `studio/scripts/**`, no `electron-builder.json`
was touched.** No commit was made. No credential, auth or settings file was read; no real
`claude` / `codex` / `opencode` CLI was run; nothing under
`/home/v9i/.local/share/kala-agent/**` was read or written. Every shell ran with
`RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo` and a temp `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` under
`/tmp/opencode/kala-ui-fixes` (removed again at the end).

---

## 1. Per finding: what changed and what proves it

### U-01 (MUST) — a live elapsed timer while a turn is in flight

| | |
|---|---|
| Files | `studio/src/components/chat/ChatPanel.tsx`, `studio/src/styles/chat.css` |
| Contract | New local state `turnStartedAt: number \| null` (the instant the request left the browser) and `now: number` (the 1 s tick). The pending row now carries `<span class="chat-panel-elapsed-live" data-elapsed-live="yes">`. The timer is a `window.setInterval(tick, 1000)` created in a `useEffect` keyed on `turnStartedAt`; its cleanup runs when the turn ends **and** when the component unmounts, so no timer outlives the request. `formatElapsed()` is now shared by the live clock and the post-reply chip, so the two can never disagree: `< 100 ms` → `43 ms`, `< 60 s` → `1.2 s`, otherwise `m:ss`. |

`prefers-reduced-motion`: the clock is a **text** update, not an animation, and the pending
dot was already a static square (no `animation` in `chat.css`). Nothing pulses, so the
global reduced-motion rule in `base.css` already covers it — no special case was needed.

Accessibility note: the clock span is `aria-hidden="true"`. The surrounding
`role="status"` region therefore announces *"Awaiting response…"* **once** and not once a
second.

**Test** — `studio/test/ChatPanel.dom.test.tsx`, `describe('the live elapsed clock')`:
* *counts the turn up while it is in flight and stops when the reply lands* — fake timers;
  the clock reads `0 ms` at the start, `3.0 s` after 3 s, and is gone (`null`) once the
  reply lands, with `vi.getTimerCount() === 0` (no timer left behind).
* *cleans its timer up when the panel is unmounted mid-turn* — `view.unmount()` then
  `vi.getTimerCount() === 0`.
* *reads the same way as the post-reply elapsed chip* — the chip matches
  `/^\(\d+(\.\d+)? (ms|s)\)$/`, i.e. the same formatter.

---

### U-02 (MUST) — a client deadline for `postChat`

| | |
|---|---|
| Files | `studio/src/api/client.ts`, `studio/src/components/chat/ChatPanel.tsx` |
| Contract | `export const CHAT_TURN_TIMEOUT_MS = 30 * 60_000;` — `sendChat` now goes through the existing `boundedSignal(external, budget)` helper and `translateFailure()`, exactly like every other bounded route, so an expired budget raises `StudioTimeoutError('/api/studio/chat')` and an operator abort still raises `StudioAbortError`. |

The constant carries the required comment: it is **not** a limit on how long a turn may
take, it covers the backend's own worst case — *two model calls of up to 180 s each* (the
design turn plus its repair call) *plus two CAD builds whose deadline the design-budget
cell is re-sizing* — and it exists only to unstick a wedged backend.

The panel renders it honestly and **distinct from a user cancel**: `describeError()` now
maps `StudioTimeoutError` to the kind `timeout` and to the message
`timeout: /api/studio/chat did not answer within the client deadline — the backend may
still be working on this turn. Press Cancel to stop waiting.` The cancel path keeps its
own line (*"Request cancelled"*, `role="status"`); the timeout is an `role="alert"` error.

**Tests** — `studio/test/apiClient.test.ts`:
* *gives the chat turn a client deadline of thirty minutes* — `CHAT_TURN_TIMEOUT_MS === 30 * 60_000`.
* *abandons a wedged chat turn when its own deadline expires* — fake timers, a fetch that
  never answers: the rejection is a `StudioTimeoutError` whose `path` is `/api/studio/chat`.
* *keeps an operator cancellation a cancellation and not a timeout* — `StudioAbortError`,
  and explicitly **not** a `StudioTimeoutError`.

`studio/test/ChatPanel.dom.test.tsx`, `describe('the client deadline of a chat turn')`:
* *reports an expired deadline as a timeout and not as a cancellation* — the alert carries
  `timeout: /api/studio/chat` and the text *"Request cancelled"* is absent.
* *still reports an operator cancellation as a cancellation* — the cancel line appears and
  no `.chat-panel-error` is rendered.

---

### U-04 (MUST) — the `failure` code of a failed turn in the ledger

| | |
|---|---|
| Files | `studio/src/state/journal.ts` |
| Contract | The `chat.turn` case of `summarisePayload()` now reads `payload['failure']`. When it is present the summary carries `failed: <code>` **instead of** `design requested`, so a turn that died can never read as a turn that was merely requested. A turn without a failure keeps its old wording byte for byte. |

**Test** — `studio/test/journalLedger.dom.test.tsx`, `describe('a failed chat turn')`:
* *shows the failure code instead of reading as a normal design request* — a real
  `chat.turn` page with `failure: 'provider_timeout'`; the summary and the rendered
  MESSAGE cell both contain `failed: provider_timeout` and neither contains
  `design requested`.
* *keeps the plain wording for a turn that did not fail* — exactly
  `local · CAD freecad · offline · design requested`.

---

### U-05 (MUST) — an empty / `default` model is stated honestly

| | |
|---|---|
| Files | `studio/src/providerPolicy.ts` (new pure helpers), `studio/src/components/chat/ChatPanel.tsx`, `studio/src/components/chat/ModelPicker.tsx`, `studio/src/components/chat/ProviderPicker.tsx`, `studio/src/modelCatalog.ts` |
| Contract | `defaultModelLabel(providerId, model)` returns `null` unless the provider is one of the three subscription CLIs **and** the model is the literal id `default` (or empty). Otherwise it returns e.g. `Codex CLI default model (chosen by Codex)`, `Claude Code default model (chosen by Claude Code)`, `OpenCode CLI default model (chosen by OpenCode)`. No model name is ever invented — the CLIs do not report which model they picked (checked live 2026-10-21). `optionalModelNote(providerId, providerName)` returns the hint *"Leave empty to use <Provider>'s own default model — the CLI does not report which model it picked. Pick a row here to pin one; the reply then names it."* for a subscription CLI and `null` for a route that requires a model id. |

Where it shows:
* the **reply badge** (`chatBadgeText`) — `codex_cli · default` now reads
  `Codex CLI default model (chosen by Codex)`; a pinned model still reads
  `codex_cli · gpt-5-codex`;
* the **model picker's `default` row** — the row name is the honest label, the literal id
  `default` stays visible (it is what the request carries), and the tooltip adds
  *"the CLI's own default model — the CLI does not report which model it picked"*;
* the **model picker note** under the field — the hint above;
* the **provider picker's subscription row** — *"Model optional — the CLI runs with its own
  default model unless you pin one in the model picker."*

**Tests** — `studio/test/providerPolicy.test.ts` (new, 11 cases): the label per CLI, the
empty-model case, the alias, the "a reply that named a real model" case, the "not a CLI"
case, `isDefaultModelId`, and the note's presence/absence.
`studio/test/ChatPanel.dom.test.tsx`, `describe('the reply badge and the CLI default model')`:
`says honestly that the CLI ran with its own default model`, `says the same when the
backend reported no model at all`, `names the model the operator pinned`.
`studio/test/ModelPicker.dom.test.tsx`, `describe('the CLI default model')`: the honest
`default` row (and that it contains no `gpt-` name), the note, and no note for an API
route. `studio/test/ProviderPicker.dom.test.tsx`, `describe('the subscription rows')`: the
row note, and its absence for an API route.

---

### U-07 (SHOULD) — a CLI without a model list gives the real reason

| | |
|---|---|
| Files | `studio/src/components/chat/ModelPicker.tsx`, `studio/src/components/chat/ChatPanel.tsx` |
| Contract | `ModelPicker` takes two new props from the backend's own provider record: `providerAvailable?: boolean \| null` and `providerReason?: string \| null`. When a **subscription CLI** is not configured, the note now reads `<Provider> has no model list: <the backend's own reason>.` (or, when the backend gave none, `<Provider> was not found on this machine or is unavailable, so it has no model list.`) and **offers no "Open API keys" button** — a CLI has no key. An API route without a credential keeps the old sentence and the button. |

**Test** — `studio/test/ModelPicker.dom.test.tsx`, `describe('a CLI without a model list')`:
the backend's reason text is shown verbatim and no `.model-picker-action` exists; the
generic fallback when there is no reason; and the API route still offers the key form
(and calls the callback when clicked).

---

### U-08 (SHOULD) — the view-provenance labels say what is true

| | |
|---|---|
| Files | `studio/src/viewer/designSource.ts`, `studio/src/components/drafting/AuxView.tsx` (`studio/src/viewer/HlrDrawing.tsx` and `studio/src/viewer/SceneCanvas.tsx` already used `designViewLabel`) |
| Contract | One honest label for every surface: `view: OpenCascade import (view-only) of the FreeCAD STEP` / `… of the Solid Edge STEP` / `… of the OpenCascade (opt-in) STEP`. The short stamp for the drafted footer: `VIEW: OCC IMPORT OF FREECAD STEP`. The drawing source: `SOURCE: OCP HLR OF THE IMPORTED MODEL`. |

The old `view: OpenCascade (opt-in) build` and `VIEW: OPC BUILD` are gone: for **every**
provider — the opt-in OpenCascade one included — the picture is the OpenCascade *import*
of the provider's STEP (`view.imported … role: "view-only"` in the journal). The `SOURCE:
OCP HLR` label is kept, but it now says that the drawing is an OpenCascade hidden-line
view **of the imported model**.

**Tests** — `studio/test/designSource.test.ts` (new, 6 cases): the label per provider, the
opt-in case, *"never claims that the kernel built anything"*, the no-provider case, the
upper-case stamp and the drawing source stamp.
`studio/test/AuxView.dom.test.tsx` and `studio/test/DraftingSheet.dom.test.tsx`: the two
existing assertions of the **old** labels were updated to the new ones (the behaviour is
still covered; the old strings asserted a claim that was false) and the AuxView test now
additionally asserts that `OPC BUILD` is gone.

---

### U-09 (SHOULD) — "Clear key" must not claim success for an environment-sourced key

| | |
|---|---|
| Files | `studio/src/components/panels/ProviderPanel.tsx`, `studio/src/styles/studio.css` |
| Contract | The button is now gated on the **settings row's source field** (`cred.source === 'stored'`), not on `provider.configured`. An environment-sourced key gets no button and a new note: *"This key comes from the environment (OPENAI_API_KEY) — it is not stored by Studio and cannot be removed here. Clear it in the environment to revoke it."* The success message of a clear is now `Stored API key cleared.`; the generic `API key cleared.` is gone. If the backend reports no source, no button is offered either (the panel does not claim what it cannot prove). |

**No STOP was needed**: the API *does* expose the source —
`ProviderCredentialStatus.source` ∈ `none | environment | stored | subscription`
(`studio/src/types/studio.ts:190`) — so the required field exists and was used. The
`env_var` of the same row names the environment variable.

**Test** — `studio/test/ProviderPanel.dom.test.tsx` (new, 4 cases): a stored key offers
"Clear key", posts `{ provider: 'openai', api_key: null }` and says `Stored API key
cleared.`; an environment key offers **no** button and shows the note; the panel never
prints `API key cleared.`; a key with no named source offers no button.

---

### U-10 (SHOULD) — the stored server-preview error is rendered

| | |
|---|---|
| Files | `studio/src/components/inspector/Inspector.tsx`, `studio/src/components/drafting/DraftingSheet.tsx`, `studio/src/components/bottombar/BottomBar.tsx`, `studio/src/styles/studio.css`, `studio/src/styles/drafting.css`, `studio/src/styles/layout.css` |
| Contract | `preview.lastError` is now read in three places, all in the existing visual language: the **inspector** shows `<p class="inspector-preview-error" role="alert">Server preview refused: <the real message></p>`; the **drafting sheet** shows the same message in a drafted note `<p class="drafting-preview-note is-error" role="alert">`; the **bottom bar** source chip reads `server refused` with `data-preview-state="refused"` and the real message in its tooltip. `preview.inFlight` shows `Verifying the mesh on the server…` in the inspector and on the sheet, and the bottom-bar chip reads `verifying…` with `data-preview-state="verifying"`. |

Self-clearing on recovery: the store already clears `preview.lastError` on
`preview/request-start`, on `preview/request-success` and on a parameter change, so the
note disappears by itself as soon as the preview works again.

**Test** — `studio/test/previewStatus.dom.test.tsx` (new, 6 cases): the real message in
the inspector (with `role="alert"`), on the sheet, in the bottom-bar chip and its tooltip;
the clearing on recovery; the in-flight state in all three; and the empty state.

---

### U-11 (SHOULD) — the journal-poll error is rendered

| | |
|---|---|
| Files | `studio/src/components/journal/JournalPanel.tsx` |
| Contract | `sessionJournal.lastError` is now read and shown under the toolbar: `<p class="journal-panel-note journal-panel-poll-error" role="status">Journal poll failed: <the real message> — the ledger below may be stale.</p>`. It reuses the existing `.journal-panel-note` language and self-clears: the store clears `lastError` on the next successful `journal/server-page`. The chain verdict is untouched — a failed poll still never claims "HASH-CHAIN OK". |

**Test** — `studio/test/journalLedger.dom.test.tsx`, `describe('a failing journal poll')`:
the real message and the staleness warning are shown, the note disappears again after a
successful page, and nothing is shown when no poll has failed.

---

### U-12 (SHOULD) — ONE Codex row

| | |
|---|---|
| Files | `studio/src/components/chat/ProviderPicker.tsx`, `studio/src/providerPolicy.ts` |
| Contract | `PROVIDER_ALIASES = { codex: 'codex_cli' }` and `canonicalProviderId(id)` live in the provider policy. The picker hides an alias row **when its canonical route is also listed**; when the backend lists only `codex`, the row stays (it is the only route to that provider). A stored selection that still names the alias is accepted and **shown as the canonical route** (`codex_cli` is the selected row, and picking it reports `codex_cli`). |

**Test** — `studio/test/ProviderPicker.dom.test.tsx`, `describe('the Codex alias')`: one
Codex row out of the two; a stored `codex` selection shown and marked selected as
`codex_cli`; picking it reports `codex_cli`; the alias row survives when the backend lists
no canonical id.

---

### U-13 (SHOULD) — keyboard access to the 3D viewport

| | |
|---|---|
| Files | `studio/src/viewer/viewportKeys.ts` (new, pure), `studio/src/viewer/SceneCanvas.tsx`, `studio/src/styles/studio.css` |
| Contract | The viewport container is `tabIndex={0}`, `role="application"` (it answers keys itself, so it is an application widget and not a static picture) and its `aria-label` names every key it owns: *"… Keyboard: the arrow keys orbit, + and - zoom, 0 resets the view."* The arrow keys orbit, `+`/`=` zoom in, `-`/`_` zoom out, `0` re-frames; every other key is left to the browser. The camera maths is a pure module (`viewportCommand`, `orbitCamera`, `zoomCamera`) so the contract is testable without a WebGL context; the live scene applies the same numbers to the camera the pointer drives (`SceneState.applyCommand`). A visible focus ring is added: `.viewer-canvas:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }` (drawn inside the canvas, because the sheet frame would clip an outside outline). |

**Tests** — `studio/test/viewportKeys.test.ts` (new, 9 cases): the key map, the keys that
are left alone, the help sentence, the orbit maths, the polar clamp, the zoom clamp.
`studio/test/SceneCanvas.dom.test.tsx` (new, 5 cases): the container is focusable with the
right role, the label names every key, the owned keys are consumed
(`event.defaultPrevented === true`) and an unowned key is not, and the focus ring exists in
the CSS.

---

## 2. Files changed

**New source (2)**
* `studio/src/viewer/viewportKeys.ts` — the viewport keyboard contract (pure).
* `studio/src/viewer/designSource.ts` — rewritten: the honest provenance labels.

**New tests (8)**
* `studio/test/viewportKeys.test.ts`, `studio/test/designSource.test.ts`,
  `studio/test/providerPolicy.test.ts`, `studio/test/SceneCanvas.dom.test.tsx`,
  `studio/test/ProviderPanel.dom.test.tsx`, `studio/test/previewStatus.dom.test.tsx`.

**Modified source (13)**
* `studio/src/api/client.ts` — `CHAT_TURN_TIMEOUT_MS`, the bounded `sendChat`.
* `studio/src/components/chat/ChatPanel.tsx` — the live clock, the timeout message, the
  honest badge, the provider reason handed to the picker.
* `studio/src/components/chat/ModelPicker.tsx` — the honest `default` row, the note, the
  CLI reason.
* `studio/src/components/chat/ProviderPicker.tsx` — the alias row, the model note.
* `studio/src/components/drafting/AuxView.tsx` — the provenance stamp.
* `studio/src/components/drafting/DraftingSheet.tsx` — the preview state note.
* `studio/src/components/inspector/Inspector.tsx` — the preview error / verifying state.
* `studio/src/components/journal/JournalPanel.tsx` — the poll-error note.
* `studio/src/components/bottombar/BottomBar.tsx` — the preview state of the source chip.
* `studio/src/components/panels/ProviderPanel.tsx` — the source-aware "Clear key".
* `studio/src/state/journal.ts` — the `failure` code in the summary.
* `studio/src/providerPolicy.ts`, `studio/src/modelCatalog.ts` — the policy helpers.
* `studio/src/viewer/SceneCanvas.tsx` — the keyboard contract of the viewport.

**Styles (5)** — `chat.css`, `drafting.css`, `layout.css`, `studio.css` (and the
existing `models.css` gained the picker rows for the new notes through `styles/models.css`
of the previous wave — no change needed there).

**Modified tests (6)** — `ChatPanel.dom.test.tsx`, `apiClient.test.ts`,
`journalLedger.dom.test.tsx`, `ModelPicker.dom.test.tsx`, `ProviderPicker.dom.test.tsx`,
`AuxView.dom.test.tsx`, `DraftingSheet.dom.test.tsx`.

---

## 3. Gates

Run in `studio/` (the only package touched):

| gate | result |
|---|---|
| `npm run typecheck` | **exit 0** |
| `npm test` | **exit 0** — **52 files / 750 tests passed** (baseline 46 files / 683 tests) |
| `npm run build` | **exit 0** (`✓ built in 537ms`) |

The baseline was green before the change and is green after it: **+6 test files,
+67 tests, 0 removed**. Two existing assertions had to change — `AuxView.dom.test.tsx`
and `DraftingSheet.dom.test.tsx` asserted the **old** provenance strings
(`SOURCE: OCP HLR · VIEW: IMPORT`, `view: OpenCascade (opt-in) build`), which were the
false claims this cell removes. The behaviour they covered is still covered, with the
honest strings, and the AuxView test now additionally asserts that `OPC BUILD` is gone.
No behavioural coverage was deleted.

`/tmp` was cleaned: the temp `HOME` / data dirs of this cell
(`/tmp/opencode/kala-ui-fixes`) were removed.

---

## 4. Left open / not done

* **U-03** (the FreeCAD `.FCStd` download) and **U-06** (a real backend cancel) are **not**
  in this cell, per the brief. U-06's wording fix ("Cancelled in the UI — the provider call
  may still be running on the backend") therefore **was not** applied; the panel still says
  *"Request cancelled"*.
* **U-14 … U-16** were skipped, as instructed.
* **No browser pass.** The desktop browser bridge was not used in this session, so the
  changes were verified against the DOM contract and the CSS, not against pixels. The two
  visual additions (the live clock in the pending row, the preview note on the sheet) use
  the existing note/elapsed styles and were not eyeballed at 1440×900.
* **The live camera cannot be asserted in a DOM test** — happy-dom has no WebGL context,
  so `createScene` throws and the viewport renders its fallback. The keyboard contract is
  therefore pinned in two halves: the pure key map and camera maths
  (`viewportKeys.test.ts`) and the DOM contract (`SceneCanvas.dom.test.tsx`), which proves
  the keys reach the scene (`defaultPrevented`) but not the resulting camera position.
* The **measure tool is still pointer-only** in the 3D viewport. The audit mentioned it
  under U-13; the adjudicated task listed only orbit / zoom / reset, and a keyboard raycast
  pick is a separate piece of work.
* `role="application"` on the viewport replaces the old `role="img"`. This is deliberate —
  the element now answers keys itself — but it is a semantic change a reviewer should
  look at with the app in front of them.
* The **"Clear key"** wording is `Stored API key cleared.`. If the operator wants the
  shorter `API key cleared.` back, it is a one-line change; the important part is that the
  button is no longer offered for a key the panel cannot remove.
