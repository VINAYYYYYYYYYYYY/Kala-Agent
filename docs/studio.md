# Kala Studio

> Status: **preview**. This document states what the studio actually
> builds, measures and shows today, and the limits a user must understand
> before trusting anything on the screen.
>
> **Current state (2026-10-21).** The studio is **Rust-only** (ADR-0010
> phase 2). The shipped app runs the native `kala-studio` service and
> **no Python process at all**; the geometry comes from the **real
> OpenCascade 7.9.3** through the `kala-cad` adapter. The Python geometry
> worker survives as a **development-only opt-in** and as the differential
> oracle. The **Linux AppImage is built and smoke-tested but unsigned**;
> the **Windows installer is a script that has not been executed yet**.
>
> Read with: [studio-providers.md](studio-providers.md) (language-model
> providers), [cad-providers.md](cad-providers.md) (CAD providers),
> [studio-limitations.md](studio-limitations.md) (the honesty sheet) and
> [studio-install.md](studio-install.md) (how to run it).

## What Kala Studio is

Kala Studio is a **chat-first Mechanical Engineering Agent** with a
**Vellum drafting board** beside it. The user describes a part in chat; the
studio turns that description into a `ConstructionScript`, hands it to a
**CAD provider** that builds and measures the model, and shows the result on
a real drafting sheet with a section cut, hidden-line views and a
hash-chained session ledger.

The board is not a decoration. Every number on it was measured by the
provider that built the model, and every stamp on it is backed by a real
backend answer.

The studio is a **local-first, single-user desktop workspace**. It runs on
loopback only, talks to no host unless the operator explicitly picks a
remote language-model provider, and stores nothing outside its own data
directory.

## How it is put together

Two layers, and the boundary between them is deliberate:

1. **The native Studio service** (`backend/crates/kala-studio/`) that owns
   the HTTP boundary (Host / Origin / bearer / body-limit gates), the
   settings and token stores, the static UI, the chat providers, the
   CAD-provider routing, the static file server — **and the geometry**.
2. **A React + TypeScript + Vite + Three.js UI** under `studio/` that the
   back door serves same-origin from `studio/dist`.

The service computes its own geometry, through the **real OpenCascade
7.9.3**. `kala-cad` owns one narrow C++ adapter (`src/ffi/adapter.cpp`)
that **calls** the OCCT SDK and never rewrites it; `kala-cad-providers`
owns the CAD provider **connections** (`FreeCADCmd` out of process, the
Solid Edge loopback bridge); `kala-design-intent` owns the intent contract
and the strict prompt parser; and `kala-studio-journal` owns the
hash-chained session journal and the content-addressed artifact store. The
geometry is never re-implemented in Rust.

The **Python geometry worker** (`python -m kala_agent.studio --worker`)
still exists in the tree, but it is **not a product layer any more**: it is
a **development-only fallback** that the service starts **only** when
`KALA_STUDIO_GEOMETRY_WORKER` names one. The packaged app starts no Python
process at all.

The **Python reference server** (`python -m kala_agent.studio`) still exists
and is still the parity oracle: it serves the same public API without the
Rust service in front. It is what the differential harnesses compare
against. See [The native Studio service and the worker
fallback](#the-native-studio-service-and-the-worker-fallback).

The **Electron shell** (`studio/electron/`) launches the back door as a child
process, generates a random bearer token in the parent, and exposes it to
the renderer only through one narrow IPC channel. In the packaged app it
resolves `resources/backend/kala-studio` **only** — never a PATH Python, a
repository virtualenv or a worker command.

## The Vellum drafting board

The board is a four-zone drafting workspace. There is no top header bar and
no application footer; the connection information lives in the journal
footer and in the Options drawer.

| Zone | Width (1440×900) | Contents |
|---|---:|---|
| **Left — agent chat** | 340 px | the header stamp, the engineering starter prompts, the thread, the composer with the egress consent box and the **CAD-target selector** |
| **Centre — the drafting sheet** | flexible (780 px) | the sheet itself: the A–F / 1–8 coordinate ticks, the 50 px drafted grid, the model, redline dimensions, the title block, the small tool palette, the sheet tab strip; the sheet touches the **top edge** |
| **Right — top** | 320 px (AuxView exactly 250 px) | the **AuxView**: `Section A–A` / `Ortho` / `Tolerance` tabs |
| **Right — rest** | 320 px | the **Agent & Kernel Journal**: the mono ledger with chain state |
| **Bottom bar** (centre column only) | 48 px | render-mode switch, Grid/Axes/Dims/Section toggles, Reset View, the live readout, Export STEP, the FreeCAD registry pill, the ⚙ Options trigger |

At 1280×800 the columns become 280 / 740 / 260. There is exactly **one**
WebGL canvas in the whole app and it survives mode switches, theme switches
and drawer opens.

### Render modes

* **2D Pencil** — a vector drafting view. With a design active it draws the
  kernel's hidden-line result (visible edges solid, hidden edges dashed) as
  SVG over the sheet rather than relying on browser edge heuristics.
* **3D Clay** — a warm clay solid in the live WebGL viewport, rendering the
  **imported** provider STEP (see
  [CAD providers](cad-providers.md#who-measures-what)).

Switching between them re-frames the camera, because the field of view
changes. Changing the theme, the material, or the grid/axes/dimension
toggles does **not** re-frame and does not rebuild the GPU buffers.

### The title block

Real values only: sheet, part id, title, `SCALE`, units, projection, the
declared tolerance list, revision and author. `SCALE` reads **`FIT`**, not
`1:1` — the viewport frames the part to fit the sheet, so there is no fixed
model-to-paper ratio and claiming one would be false. With a design active
the block gains a **`MEASURED BY`** cell naming the provider.

### The Options drawer

A modal opened from the ⚙ trigger in the bottom bar. `Esc` closes it, focus
is trapped while it is open and returns to the trigger, and it carries
`role="dialog"` and `aria-modal="true"`. It holds four drafted cards:

| Card | Title | Contents |
|---|---|---|
| **01** | Parametric Inspector & Primitive Controls | the primitive selector, the parameter sliders, the surface-material control |
| **02** | Neutral-File Export & CAD/CAE Handoff | the export panel, the design export block with its digests, the integration registry |
| **03** | Model Gateway & API Providers | provider status and API-key configuration |
| **04** | Loopback Boot Gate & Recovery | the live origin, mode, health, version, and a reconnect/retry action |

Cards 02 and 04 read the live store: the port is `NO LOOPBACK` before a
connection exists, and the export note follows the backend's
`step_available` flag (`STEP —` / `ISO 10303 READY` / `STEP UNAVAILABLE`).
No token is displayed or persisted there.

### Themes

Two themes ship: **`vellum`** (day) and **`slate`** (night). Only the theme
**name** is persisted, in `localStorage['kala-studio.theme']`. No origin, no
token, no chat text and no journal content is written there. The canvas
background follows the theme (paper by day, slate at night) so the viewport
surface and the sheet behind it can never disagree about the paper they are
drawn on.

### The measure tool

The fifth tool in the sheet palette. It is **disabled while no mesh
exists**.

* **Picking** is a real raycast against the face mesh. The picked point is
  in model millimetres, so the number is a real length. A pointer that
  moved more than 4 px between press and release was an orbit drag and is
  ignored; a click that misses the mesh changes nothing.
* The **leader** is a redline line with jaws at both ends and a mono label
  (`20.10 mm`), projected from the same camera as the dimension labels, so
  it follows the orbit.
* The **sheet note** says what the tool is waiting for
  (`MEASURE · PICK POINT 1 ON THE MESH`) or the measured distance. It never
  invents a value while the measurement is incomplete.
* **Escape** or a second click on the tool button clears the measurement; a
  third pick starts a new one.
* It measures the drawn solid only. There is no "distance to nothing".

## The CAD-target selector

The composer carries a **CAD-target selector**. It is the only way to pick
a provider, and it starts with **no choice**.

* The rows come from `GET /api/studio/providers/cad` — a live probe of the
  host, never a constant.
* An unavailable provider is **disabled** and shows the backend's reason.
  OpenCascade is always labelled **opt-in**.
* The choice lives in the in-memory session store only; it is never written
  to `localStorage`.
* With no choice, the chat request carries `cad_provider: null` and the
  backend answers with its provider-choice prompt and builds nothing.

Full detail in [cad-providers.md](cad-providers.md).

## Design turns

A chat turn can now produce a real, measured, journaled part.

```
chat prompt ──► DesignIntent (template call, or a raw ConstructionScript
                  plus DECLARED tolerances and material)
                  │  validated; an invalid intent is an explicit error turn
                  ▼
              CAD provider builds the model and reports ITS OWN properties
                  ▼
              checks: declared envelope vs the provider's measurement
                  ▼
              the provider's STEP is stored (sha256) and imported by
              OpenCascade for DISPLAY ONLY (mesh, HLR views, section)
                  ▼
              the session journal records the turn, the build, the
              artefacts and the measurement, with chain digests
                  ▼
              the UI: the agent card with the measured table and the check
              rows, the sheet with the drawing, the section and the ledger
```

The viewer is a separate statement from the measurer. The viewport says
`view: OpenCascade import (view-only) of the FreeCAD STEP` — or of the
Solid Edge STEP, or of the OpenCascade (opt-in) STEP — **for every
provider**. The picture on the screen is always the OpenCascade *import*
of the provider's STEP; the label never claims that the OpenCascade kernel
built the geometry the operator is looking at.

The local helper is a **strict parser**: it reads the prompt into a
declared intent, and when a value is missing it asks for exactly that value
("`Local helper. I can draft a L-bracket, but I need these declared
values: …`"). It never guesses a dimension. With a remote provider the
DesignIntent JSON Schema is added to the prompt, the reply is validated,
and **one** repair round-trip is allowed before the turn fails honestly.

The studio also refuses to build a design from an *unparseable* prompt: the
answer asks for the missing values and no design is compiled.

### Editing a stored design

A design turn can revise a stored design instead of drafting a new part
(ADR-0012). The request carries `base_design_id` — a design id the backend
already holds — and optionally `focus`, the one feature the request is about
(at most 1 024 characters). The backend reads the design's stored canonical
script from the artifact store, puts it into the prompt it builds for the
model, and the model is asked for the **complete** revised DesignIntent with
everything the request does not change left alone. The result is a **new**
design with its own id and its own measurement; the base design is never
mutated. The prompt limit of 4 096 characters still covers the operator's text
only, because the script never travels through the request.

The route refuses an edit turn before any model call: `focus` without a base
design is `400 focus_requires_base_design`, a base design without a CAD target
is `400 edit_requires_design_turn`, the `local` provider is
`400 edit_requires_model`, and an id the store does not hold is
`404 design_not_found`. The same validation repair and build repair the draft
turns use carry the base design too, so a repair never revises a part it can no
longer see.

Every chat response carries `usage`: `{input_tokens, output_tokens}` summed
over the model calls of that turn, or `null` when no provider reported a
count. The studio never estimates a token. The journal's `chat.turn` event
records `base_design_id` and `focus_chars` (the length, never the text) and the
usage when it was reported, under the same hash chain as every other event.

## Design checks and limits

Everything in this section is enforced by the code, and every number is a
measured or a configured value — never an estimate.

### The whole-design budget

| bound | value | where it is enforced |
|---|---|---|
| components per script | **64** | the construction contract |
| steps per component | **500** | the construction contract |
| **steps in the whole script** | **500** | the construction contract: the steps of every component **plus** the assembly steps |
| assembly steps | 16 | the construction contract |
| Solid Edge parts | **8** | the bridge wire cap |

A script with 500 steps is accepted and a script with 501 is refused, with
the message `a script may use at most 500 steps in total; this one uses
501`. The same budget is stated to the model in the design instruction, so
the model knows the number it may spend.

**Solid Edge refuses more than 8 parts.** The bridge protocol caps a
procedure at 8 parts, so a script that compiles to more is refused before
it reaches the bridge: `the script has 9 components; the wire cap is 8
parts`. The protocol's 600-operation cap is kept as a second line of
defence, but a contract-valid script cannot reach it: with at most 8 parts
and at most 500 steps the maximum is 543 operations. **The parts cap is
the real Solid Edge limit**, and it is much lower than the script budget.

### The build deadline, and what a big build costs

| item | value |
|---|---|
| FreeCAD provider wall time | **1 800 s** by default (up to 3 600 s) |
| FreeCAD adapter deadline | **1 800 s** |
| OpenCascade compile budget | **60 s** |

Measured on FreeCAD 1.1.4 with the real `FreeCADCmd`:

| design | steps | pockets | wall time |
|---|---:|---:|---|
| a 200 × 200 × 5 plate with a 10 × 10 grid of Ø3 holes | 202 | 100 | **58.8 s** in the provider, **59.8 s** through the Studio |
| the same plate with 3 holes off the plate | 202 | 100 | ≈ 57 s |
| a 2 000 × 2 000 × 5 plate with 249 Ø6 holes | 500 | 249 | **1 009.5 s (16.8 min)** idle, **1 250 s (20.8 min)** under load |

So a **500-step FreeCAD build takes roughly 17–21 minutes**. The 1 800 s
deadline covers it with margin when the host is idle and with less when it
is loaded, but it is not a pleasant chat turn. The cost is superlinear in
the number of pockets (100 pockets ≈ 59 s, 249 pockets ≈ 1 009 s), because
every pocket recomputes the body to measure what it removed. The
OpenCascade path is far quicker: the same 100-hole plate builds through the
Studio in **3.2 s** (2.5 s of compile time), and the 500-step plate
compiles in **16.9 s**.

### The check rows

A build's `checks[]` are computed from the provider's own facts. Their
ids are:

| id | when it appears | expected | measured |
|---|---|---|---|
| `brep_valid` | always | `true` | the provider's B-Rep validity |
| `single_solid` | when the script expects exactly one solid | `1` | the provider's solid count |
| `solid_count` | when the script expects more than one solid | the script's solid count | the provider's solid count |
| `envelope_x`, `envelope_y`, `envelope_z` | when the intent declares an envelope | the declared bounds on that axis | the measured bounds on that axis |
| `cuts_effective` | **only when a cut removed no material** | the number of pocket steps in the script | the number of pockets that removed material |

`cuts_effective` has tolerance `0.0` and unit `cut`, and it is the row
that makes a design with a dead cut **`verified: false`**. It is
**appended only for that failure**, so a passing design keeps its exact
check list and its exact digest.

### The cut rules

**The same-plane pocket rule.** A pocket cuts into material that its own
component already has at that step, and the pocket's sketch plane must be
the plane of the sketch that produced the current solid. A script that
breaks the rule is refused before any build is attempted, with

```
component '<name>': pocket at step index <i> on <PLANE> over a <SOLID_PLANE> solid
```

as a `422 invalid_request` detail.

**The per-pocket warning.** A cut that removes no material is reported,
never claimed as a removal. FreeCAD measures the feature's volume before
and after the pocket; OpenCascade compares the solid's volume before and
after the cut. The warning names the component, the step index and the
plane:

```
component '<name>': pocket at step index <i> on <PLANE> removed no material
```

The warning list shown on screen is bounded to **16** entries — the first
14 verbatim, then at most two summary lines
(`… and 486 more pockets removed no material`, `… and 5 more warnings`) —
but the **exact** number of ineffective cuts is preserved, so
`cuts_effective` stays exact even for a 500-pocket design.

### One repair round per turn

A design turn spends **at most one repair call in total**, and that one
call is **shared** by the two ways a turn can go wrong:

| what went wrong | is the repair spent? |
|---|---|
| the reply did not validate as a DesignIntent | yes — the **validation** repair |
| the design built but did not verify, or the builder refused the script | yes — the **build** repair |
| the CAD provider is not available on this host | no |
| the host has no native kernel | no |
| no CAD target was chosen | no |
| the design built and verified | no |

The **validation** repair prompt carries: the full design instruction, the
full DesignIntent JSON Schema, **the model's own previous JSON block**,
the validation error (bounded to 512 characters), a schema excerpt for the
failing field (bounded to 700 characters) and the user's request,
unchanged. The model therefore sees exactly what it wrote before, not a
summary of it.

The **build** repair prompt carries: the same instruction, the build
feedback (the failed check rows, the provider's warnings and the refusal),
the script the model sent (bounded to 200 000 characters) and the user's
request.

If the repair call itself fails, the **first build is kept** and the turn
still answers `200` with the honest reason (`the repair attempt failed:
provider_timeout.`). A turn never claims a verification its design does
not carry.

### The reply and the design are read from different places

The **displayed** reply text is bounded to **4 096 characters**. The
**design** is extracted from the **full** reply, so a long script is never
cut in the middle of its JSON: a reply of more than 6 000 characters with
a 100-pocket design still builds, and the text the operator reads has the
design block removed.

## Downloading what a design stored

A design stores its files in the content-addressed store under the studio
data directory. The **download route** serves those exact bytes:

```
GET /api/studio/design/{design_id}/artifact/{kind}
```

`kind` is an exact allow-list of four names — `script`, `step`, `fcstd`,
`stl` — and **only the kinds that design actually stored** can be
downloaded. A kind the design did not store is a `404 artifact_not_found`
with the detail `design_id=<id>; kind=<kind>`.

| what | value |
|---|---|
| `design_id` | a 64-character lowercase hex sha256; anything else is `422 invalid_request` |
| an unknown `kind` | `422 invalid_request`, and the detail names the four allowed kinds |
| a kind the design did not store | `404 artifact_not_found` |
| a stored object above **64 MiB** | `413 artifact_too_large` |
| a stored object that does not hash to its journalled digest | `500 artifact_digest_mismatch` — a refusal, not a download |
| no bearer | `401` (the route carries its own bearer gate, like `/drawing` and `/journal`) |

**The bytes are the stored bytes.** The service reads the object out of the
store, re-hashes it, compares it with the sha256 the journal recorded, and
refuses the download on a mismatch — so a download can never hand over a
file that is not the file the design was measured with. The bytes are never
re-exported from the model.

The response headers are:

| header | value |
|---|---|
| `content-type` | `application/json` (script), `application/step` (step), `model/stl` (stl), `application/octet-stream` (the native document) |
| `content-disposition` | `attachment; filename="<label-slug>-<digest8>.<ext>"` |
| `x-studio-design-id` | the design the bytes belong to |
| `x-studio-digest` | the sha256 the bytes hash to |
| `x-studio-artifact-kind` | the kind that was served |

The **filename** is the design's label, slugged: ASCII alphanumerics are
lowercased, every other run of characters becomes a single `-`, the result
is bounded to **40 characters**, and a label with no usable character
becomes `kala-studio`. The extension follows the kind — `json`, `step`,
`stl`, and `FCStd` for the provider's own native document. A hostile label
cannot escape the filename: the slug keeps it to a bounded, alphanumeric
name.

The **UI** offers the download next to every digest chip: the **design
card** in the chat panel (the script chip, the STEP chip and the provider's
native chip) and the **export panel** (the same three chips). Each button
saves the bytes under the backend's own `content-disposition` filename and
writes a receipt into the local ledger (`Saved <filename>`, plus the byte
count and the first eight characters of the digest). A refusal is shown
verbatim as `code: message`.

Two honest notes. The **`stl`** kind exists in the route, but the studio
only stores an STL when the design was exported as one, and the chat and
export panels offer no STL chip — an STL download is an API call today.
And the **native chip** offers a button only when the provider's format is
one the route can serve: a FreeCAD design offers `fcstd`, a provider whose
native format is `stp` or `step` offers the STEP download, and a provider
with an unknown native format offers no button rather than a broken one.

## Cancelling a turn

The Cancel button in the chat panel aborts the browser's request. The
backend's request future is dropped with it, and the native service turns
that drop into a real stop:

* A **cancellation token** is created for the design turn and checked at
  every **phase boundary**: validation → intent, probe → build request,
  build → import, import → measure/journal, and again before each
  `artifact.stored` / `design.*` event of the post-build record.
* The **FreeCAD build is killed**, not asked to stop: the provider's own
  process group is terminated, and the killed build is reported as a
  *cancellation*, never as a "build refused".
* The **in-process OpenCascade build cannot be interrupted mid-operation**:
  the token is honoured at the next phase boundary, so the worst case is
  one **60 s** compile that finishes before the turn stops. The FreeCAD
  build is the long case (up to 1 800 s), and that one dies promptly.
* The **subscription CLI** turns die with their **whole process group** —
  `SIGKILL` to the group on Unix, `taskkill /T /F` on Windows — because
  the guard that owns the group is dropped with the request future. See
  [studio-providers.md](studio-providers.md#the-180-s-budget-and-killing-the-whole-process-tree).
* The **journal** records **exactly one** `design.cancelled` event, with
  the design id and a bounded reason and nothing else — no prompt, no
  credential, no provider output. The reason is the honest one
  (`the client disconnected, so the design turn stopped`, or the
  provider's `the FreeCAD build was cancelled; the FreeCADCmd process group
  was stopped`). **No later `design.*`, `provider.build.*`,
  `artifact.stored` or `chat.turn` event is written**, so a cancelled turn
  is never journaled as a design.
* The service's own refusal for a cancelled turn is a `499
  design_cancelled` — *"the client closed the request, so the design turn
  was stopped"*. The client that closed the request is, of course, the one
  that will not read it.

The UI says exactly that: the turn line reads `Cancelled — the backend
stopped the turn.` and the local ledger records `Turn cancelled — the
backend stopped it`. A timeout is still a timeout and is reported as one.

**The Python reference cannot cancel a running build.** Its `/design`
route is synchronous inside the FastAPI thread pool, and the event loop
cannot drop a thread, so a disconnected client does not stop that build.
The reference says so in its own `DesignService.design` docstring rather
than pretending a cancelled turn stopped. The **native Rust service is the
shipped path**, and the cancellation described above is native-only.

## What the workspace shows you

These are the behaviours of the shipped Studio UI. Each of them states a
fact the studio can actually back.

* **A live elapsed clock while a turn is in flight.** The pending row
  carries a running counter (`0 ms`, `1.2 s`, `m:ss`) that starts when the
  request leaves the browser and stops when the reply lands. It is a text
  update, not an animation, and it is `aria-hidden`, so the surrounding
  status region announces *"Awaiting response…"* once and not once a
  second. The same formatter produces the post-reply `(1.2 s)` chip, so
  the two can never disagree.
* **A client deadline for a chat turn.** The deadline is **derived from
  the backend**, not pinned in the UI: `/health` publishes the backend's
  own worst-case design-turn budget (`design_turn_budget_seconds`), and
  the client adds **5 minutes** to it. With the shipped budget of
  **3 960 s** (66 min) the browser abandons an unanswered turn after
  **71 minutes**; a backend that reports no usable budget falls back to
  **70 minutes**. It is a *client* safety net for a wedged backend, not a
  limit on how long a turn may take: the backend's own budget is two
  model calls of up to 180 s each plus two CAD builds of up to 1 800 s
  each. A timeout is reported as a timeout; an operator cancellation
  stays a cancellation, with its own message.
* **A download button on every stored file.** The design card and the
  export panel each carry a `download` button next to the script, the
  STEP and the provider's native digest chips. The button saves the
  **stored bytes** under the backend's own filename and writes a receipt
  (`Saved <filename>` and the digest) into the local ledger; a refusal
  is shown verbatim.
* **A cancellation that says what happened.** A cancelled turn reads
  `Cancelled — the backend stopped the turn.` and the ledger records
  `Turn cancelled — the backend stopped it`. The wording is not
  "Request cancelled", because the backend really does stop the work.
* **Keyboard access to the 3D viewport.** The viewport is focusable and
  answers the **arrow keys** (orbit), **`+` / `-`** (zoom) and **`0`**
  (reset the view). Every other key is left to the browser. The viewport
  announces its own keys in its accessible name.
* **Honest view and source labels.** The viewport, the HLR sheet and the
  AuxView footer all say the same thing: the picture is the **OpenCascade
  import** of the provider's STEP (`view: OpenCascade import (view-only)
  of the FreeCAD STEP`, footer `VIEW: OCC IMPORT OF FREECAD STEP`), and
  the drawing is `SOURCE: OCP HLR OF THE IMPORTED MODEL` — a hidden-line
  view *of the imported model*. No label ever claims the kernel built the
  geometry on screen.
* **Honest default-model wording.** A reply that ran on the CLI's own
  default model reads `Codex CLI default model (chosen by Codex)` (or the
  Claude Code / OpenCode equivalent), never a model name the studio
  invented. Under the model field the picker says that an empty field
  means *"the CLI's own default model — the CLI does not report which
  model it picked"*.
* **The failed turn says so.** A `chat.turn` row that failed reads
  `failed: <code>` instead of `design requested`, so a turn that died can
  never read as a turn that was merely asked for.
* **Real errors, where they happen.** A refused server preview is shown
  in the inspector, on the drafting sheet and in the bottom bar; a failed
  journal poll says the ledger below may be stale. The chain verdict is
  untouched: a failed poll never claims `HASH-CHAIN OK`.
* **One Codex row.** The provider picker shows one Codex entry, not two:
  the historical `codex` id is the same route as `codex_cli`, and the
  picker folds it into the canonical row.
* **A key that cannot be cleared is not offered.** "Clear key" appears
  only for a key the backend says it stored. A key that comes from the
  environment carries the note that it is not stored by the Studio and
  cannot be removed there.

## The journal, the ledger, and the two stamps

The studio keeps its **own** SQLite journal and artifact store under the
studio data directory. It shares no state with the core `KalaApplication`;
it reuses only the chain and artifact libraries. The journal is the single
writer of its own file, so there are never two writers on one chain. The
journal is **native** now (`kala-studio-journal`): the same hash-chained
SQLite file, the same rows and the same chain digests as the Python
journal, which can still open and extend it.

The **ledger** on the right is a real table:

```
SEQ | UTC | TYPE | MESSAGE | DIGEST | CHAIN
```

* `SEQ` is the backend sequence; a local UI row has none and shows `—`.
* `TYPE` is the backend event type (`design.intent`, `provider.build.*`,
  `artifact.stored`, `view.imported`, `chat.turn`, …) or the local tag
  (`[PREVIEW]`, `[CHAT]`, `[EXPORT]`, `[TOOL]`, `[BRIDGE]`, `[SYSTEM]`).
* `DIGEST` is cut to eight characters with a copy button that copies the
  **full** digest.
* `CHAIN` is `local` for a UI row, `ok` for an event the chain covers,
  `broken` for one it does not, and `—` while nothing has been verified.

The poller reads the journal in bounded pages: **1 s** while a turn is
running, **5 s** while idle, paused entirely while the document is hidden,
with an exponential backoff (1 s → 30 s) when the backend fails.

### When the two stamps appear

| Stamp | Appears when | Never when |
|---|---|---|
| **`B-Rep Verified`** | the design's `verified` is `true` — every check passed, i.e. the provider's measurement agrees with the declared envelope and the model is a valid solid of the expected count | a check failed, a check is `not_reported`, or there is no design |
| **`HASH-CHAIN OK`** | the backend's `/journal/verify` answered `ok: true` | the chain has not been verified, the chain is broken, or the poll failed |
| `CHAIN BROKEN @ n` | verification failed and the backend named the first bad sequence | — |
| `SESSION LOG` | nothing has been verified yet | — |

Neither stamp is derived from local state. A local row, a page, or a poll
failure never lights `HASH-CHAIN OK`; a failed check never lights
`B-Rep Verified`.

## The AuxView

| Tab | With a design | Without a design |
|---|---|---|
| **Section A–A** | the kernel's section of the imported model: the exact area (`KERNEL AREA`), the offset, the outer/inner loop counts, a 45° ANSI hatch | the analytic profile of the active primitive |
| **Ortho** | the kernel's hidden-line front/top/right views in third-angle order, labelled `SOURCE: OCP HLR OF THE IMPORTED MODEL` | the AABB silhouettes of the live mesh |
| **Tolerance** | the **declared** tolerance list, labelled `declared — not analysed` | an explicit empty state |

The tolerance tab never computes a fit, a limit or a GD&T call. It states
plainly: *"The backend reports declared limits only. Kala Studio computes no
fits, GD&T, or stack-up."*

> The AuxView footer prints two honest labels: the source of the drawing
> (`SOURCE: OCP HLR OF THE IMPORTED MODEL`) and the provenance of the model
> it draws (`VIEW: OCC IMPORT OF FREECAD STEP`, or of the Solid Edge STEP,
> or of the OpenCascade (opt-in) STEP). The drawing behind them is
> produced by the native `kala-cad` adapter over the real OpenCascade
> 7.9.3 — but it is a hidden-line view **of the imported STEP**, so the
> label says exactly that.

## The API contract

All routes live under `/api/studio`. The **Python reference surface is 16
route-methods**; the native service answers **18** (the same 16 plus the
ADR-0011 view-only import and the provider model catalog).

**All 18 are answered natively, in-process.** In the default build no route
is handed to a Python worker and no Python process runs at all:

| Route | Owner |
|---|---|
| `GET /health` | the HTTP boundary and the capability probe |
| `GET /integrations` | the integration registry |
| `GET /providers` | the chat provider rows |
| `GET /providers/{id}/models` | the provider model catalog (the live OpenRouter list, the static CLI alias lists) |
| `GET /settings`, `POST /settings` | the settings and token stores |
| `POST /chat` | the chat providers and the design turn |
| `POST /preview`, `POST /export`, `POST /handoff` | the analytic primitives (`kala-geometry`, `kala-step`) |
| `POST /design` | the native geometry backend |
| `GET /design/{id}/drawing`, `GET /design/{id}/section` | the native geometry backend (the OCCT hidden-line drawing and the exact-area section) |
| `GET /design/{id}/artifact/{kind}` | the native geometry backend (the stored script, STEP, native document or STL) |
| `GET /journal`, `GET /journal/verify` | the native journal |
| `GET /providers/cad` | the CAD provider connections |
| `POST /import` | the native view-only STEP import |

`POST /import` and `GET /providers/{id}/models` are the two **native-only**
routes: the ADR-0011 view-only STEP import and the model catalog exist on
the service, but the Python public server does not mount them. The
readiness scanner reports them as "native-only (allowed, not parity)" and
does not count them as a difference.

`npm run check:native-readiness` in `studio/` parses the real Python and
Rust sources and reports **READY**: **16** Python route-methods covered,
**0** missing, **0** missing DTO capabilities, **7** answered directly and
**11** through the **geometry-backend seam**. That seam is answered by the
**default** native build — the supervised Python worker is a development
opt-in — and the report now names it that way instead of calling those
route-methods "proxied". The built stage
(`studio/release/native-readiness.json`) classifies the artefact `release`
with `releaseEligible: true`. That file is a snapshot of the sources at
build time; the final 2026-10-21 build regenerated it with the current
split (11 route-methods through the geometry-backend seam, answered
natively).

Highlights:

- `GET /health` returns `{"status":"ok"}` plus the service, version,
  provider, integration and capability fields, the ADR-0009
  `design_available` flag, and the ADR-0010 `geometry_worker`
  `{state, version, design_available}` block. In the native build that
  block reports the **native** geometry backend (`state: "ready"`,
  `version: "7.9.3"`); it reports the Python worker only when the
  development fallback is in use.
- `GET /health` also publishes **`design_turn_budget_seconds`**: the
  backend's own worst-case design-turn wall time, computed from the real
  provider budgets as `2 × (180 s model call + 1 800 s build) = 3 960 s`
  (66 min). The Studio client derives its own deadline from it (the budget
  plus a 5-minute margin, with a 70-minute fallback), so the browser
  never pins a number the design budget can outgrow.
- `POST /chat` accepts `{provider, model, prompt, allow_remote}` plus the
  optional `cad_provider`, and returns `{text, provider, model, offline}`
  plus an optional `design` (the `DesignResult`) when the turn produced one.
- `POST /design` takes a `DesignIntent` **plus a required `provider`**.
  There is no default.
- `GET /providers/cad` reports each provider's live availability, version
  and reason.
- `GET /providers/{id}/models` answers the model catalog; see
  [studio-providers.md](studio-providers.md#the-model-catalog).
- `GET /design/{id}/artifact/{kind}` serves a design's stored bytes; see
  [Downloading what a design stored](#downloading-what-a-design-stored).

### Security posture

Every mutating route requires `Authorization: Bearer <token>`. The read-only
design, journal and CAD-provider routes carry their own bearer gate too. The
middleware order is the policy: security headers → loopback `Host` guard
(421) → body-size cap (413) → same-origin guard (403) → bearer (401) →
static → router. The STEP import route has its own, larger body cap (see
`KALA_STEP_IMPORT_MAX_BYTES` below). The back door never raises the
geometry backend's ceiling; a refusal is forwarded unchanged.

## The native Studio service and the worker fallback

### What the native service owns

| Concern | Owner |
|---|---|
| The HTTP boundary, the Host/Origin/bearer/body gates | **Rust** (`kala-studio`) |
| `/health`, `/integrations`, `/providers`, `/settings`, `/chat` | **Rust** |
| CAD provider routing and the "never default to opencascade" rule | **Rust** |
| The settings and token stores, the static UI server | **Rust** |
| The construction compiler, the measurement, the hidden-line drawing, the exact-area section, the view-only STEP import | **Rust** — `kala-cad`, one narrow C++ adapter over the real OCCT 7.9.3 |
| The strict intent parser and the part templates | **Rust** — `kala-design-intent` |
| The session journal and the artifact store | **Rust** — `kala-studio-journal` |
| The CAD provider **connections** (`FreeCADCmd`, the Solid Edge bridge) | **Rust** — `kala-cad-providers` |
| The analytic primitive meshes and the analytic STEP writer | **Rust** — `kala-geometry`, `kala-step` (on the default path) |
| The **Python geometry worker** | **a development-only fallback** — it is not started unless `KALA_STUDIO_GEOMETRY_WORKER` names one |

ADR-0010 keeps the geometry in the CAD kernel on purpose, and phase 2
reached it without rewriting the kernel: the measured numbers stay the ones
the tested OCCT code produces; Rust never fabricates a mesh, a STEP file, a
measurement, a design or a provider availability fact.

The two **declared** differences against the Python oracle are the identity
of the kernel binding (the native build links the **system OCCT 7.9.3**,
the Python oracle imports the `cadquery-ocp` binding **8.0.1.1**) and the
**display-mesh vertex order** (the same geometry, assembled in a different
order; the differential compares it by an order-independent fingerprint).
The geometry, the measurements, the checks, the drawing and the section are
identical. The full list is in
[rust-migration-status.md §4.8](rust-migration-status.md).

### Running it in development

Build the service and serve the built UI:

```bash
cd backend
cargo build --release -p kala-studio --locked

cd ..
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef      # 16–128 characters
backend/target/release/kala-studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

```bash
curl -fsS http://127.0.0.1:8765/api/studio/health
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  http://127.0.0.1:8765/api/studio/providers/cad
```

**No worker, no Python and no virtualenv** are needed: the health document
must carry `design_available: true` and `step_available: true`, and the
provider probe must answer the three CAD providers with a live version or a
live reason.

The Python reference server alone (no Rust in the path) is still a
supported way to run the studio:

```bash
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
PYTHONPATH=src .venv/bin/python -m kala_agent.studio \
  --host 127.0.0.1 --port 8765 --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

### The worker fallback

`KALA_STUDIO_GEOMETRY_WORKER` names the development fallback. It is
**either** a JSON argv array:

```json
["/abs/path/python","-m","kala_agent.studio","--worker"]
```

**or** a single executable path, in which case the fixed arguments
`-m kala_agent.studio --worker` are appended. **Whitespace splitting is not
performed**: a value that names an executable *and* arguments is refused
with a clear message rather than silently re-parsed.

**The service starts no worker at all unless this variable names one**,
and there is **no development default** any more. An unset variable means
the native backend answers everything.

### `KALA_STEP_IMPORT_MAX_BYTES`

The one STEP import ceiling, shared by the service and the worker.

| Value | Result |
|---|---|
| absent or blank | the 64 MiB default |
| an integer in `1 … 536 870 912` (1 B … 512 MiB) | that value |
| anything else | a startup refusal (exit 2) with the worker's own message |

The service resolves it **once, before anything is started**, enforces
it on the public import route, and hands the same number to the worker
process when the fallback is in use. Every other route keeps the shared
256 KiB Studio limit.

### The worker's token and the lifeline

These apply to the **development fallback** only.

* The worker token is **32 fresh random bytes** rendered as 64 hexadecimal
  characters, minted per launch.
* It is passed **only** through the `KALA_WORKER_TOKEN` environment
  variable. It is never in `argv`, never in a URL, and never logged. The
  lifecycle test reads `/proc/<pid>/cmdline` and asserts that the token and
  the variable name are absent from it, and that the token is absent from
  the worker's stdout and stderr.
* `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops when the front
  door closes its stdin: **a dead front door can never orphan a worker.**
* The worker binds `127.0.0.1:0`, listens before it announces itself, and
  prints exactly one readiness line:

  ```
  KALA_WORKER_READY {"design_available":true,"port":54321,"version":"0.1.0"}
  ```

  Keys are sorted and the JSON is compact. `design_available` is true only
  when OCP is importable. The token is never printed.
* The worker enforces the token plus `Host: 127.0.0.1:<port>` and **refuses
  any `Origin` header** — it is never browser-facing.
* A worker that cannot start leaves the native routes working and the
  geometry routes answering `503 geometry_worker_unavailable`. A worker
  that dies is restarted with a capped exponential backoff (250 ms → 5 s).
  **No mesh, no STEP file and no measurement is ever fabricated while the
  worker is down.**

### The worker's internal route set

Internal, under `/internal/v1`, behind the worker's own gate stack
(Host → body size → Origin → bearer). This set belongs to the **fallback**
and to the Python reference server; the native service answers the public
routes itself:

`POST /preview`, `POST /export`, `POST /handoff`, `POST /design`,
`GET /design/{id}/drawing`, `GET /design/{id}/section`,
`GET /design/{id}/artifact/{kind}`, `POST /import`,
`GET /journal`, `GET /journal/verify`, `POST /journal/append`,
`POST /intent/parse`, `POST /intent/validate`, `GET /intent/schema`,
`GET /providers/cad`.

The worker mounts **no** `/api/studio/*` route, no settings route and no
credential route. It makes no language-model provider call; the only network
call it can make is the loopback Solid Edge bridge health probe, and only on
Windows.

## Honest capabilities

### What the studio really does

- **Chat-first engineering assistance** over a bounded text prompt, with a
  strict local parser for design intent and an explicit "I need these
  values" answer when something is missing.
- **Real, provider-measured geometry.** The volume, area, bounds and solid
  count on screen were measured by the CAD provider that built the model.
- **A real drafting view.** The hidden-line drawing, the section cut and the
  imported 3D solid are kernel results, not browser heuristics.
- **A hash-chained session journal** with artefact digests and a verifiable
  chain.
- **Neutral-file export** — STEP from the provider, STL for the primitives.

### What the studio does not claim

- **No FEA, no CFD, no solver.** See
  [studio-limitations.md](studio-limitations.md).
- **Tolerances are declared, not analysed.** A tolerance string is what the
  operator said. Kala computes no fit, no limit, no stack-up and no GD&T.
- **"B-Rep Verified" means exactly three things**: the provider's
  measurement agrees with the declared envelope, the model is a valid solid,
  and it has the expected solid count. It says nothing about strength,
  fatigue, or manufacturability.
- **No live Solid Edge build.** The bridge, the protocol client and the
  lowering are fixture-tested, and the bridge needs a **Windows host** and
  a licensed Solid Edge installation. See
  [cad-providers.md](cad-providers.md#solidedge-on-windows-the-bridge-live-proof-pending).
- **The clay viewport is a perspective camera**, not an orthographic one —
  see [studio-limitations.md](studio-limitations.md).
- **The release artefact is unsigned.** The Linux AppImage is built and
  smoke-tested, but it carries no signature and is not published; the
  Windows installer is a script that has never been executed. The MinGW
  cross-check is still pending on a missing C toolchain. See
  [studio-limitations.md](studio-limitations.md#packaging-trust).
- **No arbitrary LLM code execution.** Prompts are returned as text. The
  studio never evaluates prompt text as Python, CSG, or any other
  instruction language.
- **The subscription CLIs are chat providers, not app orchestrators.** The
  studio runs Claude Code, Codex and OpenCode only to answer a chat turn,
  with every tool denied, a read-only sandbox where the CLI offers one, a
  sanitized environment and a private working directory. It does **not**
  drive other applications, editors or agent runtimes, and it never asks
  a CLI to touch the user's project. See
  [studio-providers.md](studio-providers.md#the-three-subscription-clis).
- **A subscription CLI's default model is not named.** The CLIs do not
  report which model they picked, so the studio says
  `Codex CLI default model (chosen by Codex)` instead of inventing a name.
  Pin a model in the picker to see it named in the reply.

## How the desktop shell runs

The Electron shell (`studio/electron/main.cjs`) launches the back door as a
child process with a random bearer token. It:

- sets `nodeIntegration: false`, `contextIsolation: true`, `sandbox: true`,
  and restricts navigation and window opening;
- exposes only one IPC channel, `getConnection`, returning the loopback
  base URL and the bearer token for the trusted current window;
- loads the UI from the back door over HTTP (there is no separate static
  asset origin);
- in the **packaged** app resolves `resources/backend/kala-studio[.exe]`
  **only** — never `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND`, a PATH
  Python or a repository virtualenv — and strips `PYTHONPATH`,
  `PYTHONHOME`, `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and
  `KALA_STUDIO_GEOMETRY_WORKER` from the child environment;
- shuts the back door down on `before-quit` and on startup failure, and does
  not terminate unrelated processes;
- **writes the backend's output to a bounded, rotating log** in Electron's
  own log directory (`~/.config/kala-studio/logs/backend.log` on Linux,
  `%APPDATA%\kala-studio\logs\backend.log` on Windows; at most five files
  of 1 MiB each), with the bearer token scrubbed out of every line;
- **reports a startup failure in a dialog** — the missing binary, the
  early exit and the health timeout each open a modal that names what
  failed, where the log is and the last lines of the log, before the app
  exits.

The browser-fallback path requires manual token entry. There is no
auto-login and no third-party auth flow.

## Primary sources

The integration and provider notes in this document are anchored to the
primary sources checked on October 18, 2026, plus the ADRs listed below.
The studio does not assert terms-of-service or eligibility claims
beyond what those sources state.

- ADR-0007 (FreeCAD adapter), ADR-0008 (Solid Edge bridge), ADR-0009
  (Studio design turns), ADR-0010 (Rust control plane / geometry worker),
  ADR-0011 (CAD provider model) and ADR-0012 (edit turns and reported token
  usage) — [adr/](adr/).
- `docs/construction-semantics.md` — the construction semantics the
  provider corpus is measured against.
- OpenCascade.js (WASM, not used):
  <https://github.com/donalffors/opencascade.js>.
- Solid Edge automation (older SDK):
  <https://support.industrysoftware.automation.siemens.com/trainings/se/106/api/ConnectingToSolidEdge.html>.
- AutoCAD .NET / ObjectARX:
  <https://help.autodesk.com/cloudhelp/2024/ENU/OARX-DevGuide-Managed/files/GUID-390A47DB-77AF-433A-994C-2AFBBE9996AE.htm>.
- Ansys PyMechanical:
  <https://mechanical.docs.pyansys.com/version/stable/architecture.html>.
- COMSOL API:
  <https://doc.comsol.com/6.3/doc.com.comsol.help.comsol/comsol_api_intro.46.01.html>.
- OpenRouter chat completions:
  <https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request>.
- Codex CLI authentication:
  <https://learn.chatgpt.com/docs/auth> (redirect target of
  <https://developers.openai.com/codex/auth>).

## What this wave does not ship

- A signed installer, or a published artefact.
- A tested end-to-end installer run on a clean host.
- A validated runtime on a clean Windows host — the Windows build script
  has **never been executed**.
- A proof that the AppImage launches a desktop on every supported distro:
  the **Electron window was never rendered** on the build host, so the
  smoke verifies the packaged **backend** and the served UI root by
  extraction.
- A bundled FreeCAD, CalculiX, Gmsh, or any other solver binary. FreeCAD
  is **detected on the system, never bundled**; the app carries the adapter
  and its vendored dependencies for **cp311–cp314**.
- A live native bridge to any commercial CAD or analysis tool.
- A **Solid Edge** build: it needs a Windows host with the ADR-0008 bridge,
  and no Solid Edge build has ever been executed.
- A performance, memory or readiness number. No benchmark was run.
- App orchestration; the studio only produces a neutral handoff.

The **Linux AppImage is built and smoke-tested**:
`studio/release/kala-studio-0.1.0-x86_64.AppImage`, **unsigned**. Its
size and SHA-256 are recorded in
[packaging/linux/README.md](../packaging/linux/README.md) — they change
with every rebuild, so they are not duplicated here. The release workflow
is written but was not pushed or run. See
[studio-install.md](studio-install.md) and
[studio-limitations.md](studio-limitations.md) for the complete honesty
notes.
