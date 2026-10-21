# Kala Studio CAD providers

> Scope: the CAD **provider** model of ADR-0011 — who builds a Kala design,
> who measures it, how each provider is discovered on the host, what it
> delivers, and what is *not* proven yet.
>
> This document is about **CAD** providers (the engines that build the
> model). The **language-model** providers (`local`, `openai`,
> `openrouter`, `codex`, `claude_code`, `codex_cli`, `opencode_cli`) are
> a different thing and are documented in
> [studio-providers.md](studio-providers.md).

## The one-paragraph version

Kala is the **orchestrator, not the modeller**. A chat turn is turned into a
`ConstructionScript` plus a *declared* intent (label, material, declared
tolerances), and a **CAD provider builds the model**. The provider reports
**its own** measured properties; Kala never substitutes a number the provider
did not report. The provider's exported `STEP` is then imported by
OpenCascade **for display only** — the model is never modified, re-measured
or re-exported by the viewer.

**There is no default provider.** The composer has a CAD-target selector and
it starts empty. A design turn with no chosen provider asks which one to use
and builds nothing.

**Since the cutover (ADR-0010 phase 2, 2026-10-20) the whole model is
native.** The `opencascade` provider is the Rust `kala-cad` adapter over the
**real OpenCascade 7.9.3** (one narrow C++ shim that calls OCCT and never
rewrites it), and the `freecad` provider is the Rust `kala-cad-providers`
connection that drives the user's own `FreeCADCmd`. The Python provider
implementations are still in the tree as the differential oracle, but they are
**not** on the product path any more.

## The three providers

| Provider | Availability | Builds and measures | Delivers |
|---|---|---|---|
| `solidedge` | Windows with Solid Edge, via the ADR-0008 bridge. **Live proof pending.** | Solid Edge | Native part, STEP export, Solid Edge mass properties |
| `freecad` | Any OS with FreeCAD (1.1.4 on the dev host), via the ADR-0007 adapter. **Detected on the system; FreeCAD is never bundled.** | FreeCAD | FCStd, STEP export, FreeCAD-measured properties |
| `opencascade` | Built-in. **Only when the user explicitly selects it** | Kala's native `kala-cad` adapter over the real **OpenCascade 7.9.3** | STEP, OCCT-measured properties |

**Read the "Delivers" column as the model's contract, not as a record.**
`freecad` and `opencascade` have been exercised on this host and their
deliverables are real. The `solidedge` row is the ADR-0008/ADR-0011
**intent**: the bridge, the protocol client and the lowering are
fixture-tested, but no Solid Edge build has ever been executed, so no
Solid Edge part, mass property or export is claimed today.

The set is **closed**. `POST /api/studio/design` without a `provider` is a
`422 invalid_request` whose detail names all three values; an unknown value
is the same `422`.

### Who measures what

This is the rule the whole model turns on:

* **The provider that built the model is the measuring authority.** The agent
  card and the bottom bar say `measured by <provider> <version>`. A
  FreeCAD part is never labelled "measured by OpenCascade".
* **OpenCascade's default role is view-only.** It imports the provider's
  STEP, tessellates it for display, and renders the section cut and the
  hidden-line pencil view of that imported shape. It reports **no** volume,
  area or centroid for an imported model.
* **If a provider reports no property, the UI shows `—`.** A missing value is
  `null` with the check status `not_reported`. It is never filled in from
  another engine, and `not_reported` is neither a pass nor a failure.
* The viewer label is a separate statement from the measurer label. The
  viewport, the HLR sheet and the AuxView footer say that the picture is
  the **OpenCascade import** of the provider's STEP — `view: OpenCascade
  import (view-only) of the FreeCAD STEP`, `VIEW: OCC IMPORT OF FREECAD
  STEP` — **for every provider**, the opt-in OpenCascade build included.
  The label never claims that the OpenCascade kernel built the geometry on
  screen.

## `opencascade` — the opt-in builder

The built-in OpenCascade path is the Rust `kala-cad` crate: a bounded
`ConstructionScript` subset (`sketch`, `extrude`, `pocket`, `revolve`,
`place`, `fuse`) compiled to a `TopoDS_Shape` by the **real OCCT 7.9.3 SDK**,
measured with `BRepGProp`, drawn with `HLRBRep_Algo` and sectioned with
`BRepAlgoAPI_Section`. The Rust side calls OCCT through **one narrow C++
adapter** (`src/ffi/adapter.cpp`); it does not re-implement the kernel.

* It is **never selected automatically.** The provider id is `opencascade`,
  its display name is `OpenCascade (opt-in)` and its `mode` is `opt-in`.
* Its availability is a live capability probe of the linked kernel; the
  probe reports the OpenCascade SDK version (**7.9.3** on the build host) or
  an honest unavailable reason. A portable build
  (`--no-default-features`) links no kernel and reports
  `design_available: false`.
* It returns **no native artifact** (there is no FreeCAD-style native file
  for it), only the STEP.
* The measured L-bracket (60 × 45 × 40, 8 mm wall, Ø10 bore, 4 × Ø5.5 holes)
  is `35571.416047113315 mm³` with bounds `[-30, 0, 0] … [30, 45, 40]`, one
  valid solid.
* The **section** at `y = 4` on the XZ plane is `2400 − 25π` =
  `2321.4601836602546 mm²`, measured on the exact planar faces
  (`area_exact: true`), with one outer and one inner loop. The **drawing**
  is `front(7) top(11) right(8)`.
* **The declared difference.** The native build links the *system* OCCT
  7.9.3, while the Python oracle imports the `cadquery-ocp` binding
  8.0.1.1. The two are the same kernel family but not the same identity, so
  the reported provider version names a different binding, and the two
  versions assemble the same STEP file with its faces in a different order
  (the display mesh is compared by an order-independent fingerprint). The
  geometry, the measurements, the checks, the drawing and the section are
  identical.

## `freecad` — a real `FreeCADCmd` subprocess

### How it is found

* `probe()` looks for `FreeCADCmd` then `freecadcmd` on `PATH` and runs
  `<exe> --version` with a **20 s** timeout. The reported version is the
  real one; a probe that fails or reports nothing is `available: false`
  with the reason attached.
* On the dev host the probe reports **`1.1.4`**. That is the version this
  repository has been tested against.

### The installed app: FreeCAD is detected, not bundled

Since the cutover the connection is the Rust `kala-cad-providers` crate, and
the **installed app carries the adapter with it**:

* The package stages `adapters/freecad/src/kala_agent_freecad/` (the adapter
  modules), `adapters/freecad/src/kala_agent/` (the shared pure-Python
  dependency the adapter imports) and `adapters/freecad/mod/Kala` (the
  workbench addon). **FreeCAD itself is not bundled**; the provider finds
  the adapter by walking the executable's ancestors, so no repository
  checkout and **no virtualenv** are needed on the target.
* The adapter's Python dependencies (`pydantic`, `pydantic_core`,
  `annotated_types`, `typing_extensions`, `typing_inspection`) are
  **vendored per CPython ABI** into `site-packages-cp311`, `-cp312`,
  `-cp313` and `-cp314`. At runtime the provider runs a cheap, cached
  **Python-identity probe** inside `FreeCADCmd` (`sys.version` and
  `sys.implementation.cache_tag`), derives the ABI and puts **only the
  matching directory** on `sys.path`, after `src`. No other ABI's compiled
  extension is ever loaded.
* An **unsupported** Python is reported honestly, never as a crash:
  `FreeCADCmd runs Python 3.10.14 (cp310); the bundled adapter dependencies
  support cp311, cp312, cp313, cp314 only. Install a FreeCAD build with a
  supported Python.` (The exact wording names the detected version and the
  supported set.)
* The `mod/Kala` addon is **staged, not installed**: the app never writes
  into FreeCAD's directories. There is exactly **one supported install
  method** — copy the staged add-on and its payload into FreeCAD's user
  `Mod` directory:

  ```bash
  ./kala-studio-0.1.0-x86_64.AppImage --appimage-extract
  ADAPTER=./squashfs-root/resources/backend/adapters/freecad
  ABI=$(FreeCADCmd -c 'import sys; print("cp%d%d" % sys.version_info[:2])')
  MOD="${XDG_DATA_HOME:-$HOME/.local/share}/FreeCAD"
  [ -d "$MOD/v1-1" ] && MOD="$MOD/v1-1"
  mkdir -p "$MOD/Mod/Kala"
  cp -r "$ADAPTER/mod/Kala/." "$MOD/Mod/Kala/"
  cp -r "$ADAPTER/src" "$MOD/Mod/Kala/src"
  cp -r "$ADAPTER/site-packages-$ABI" "$MOD/Mod/Kala/site-packages-$ABI"
  ```

  Only the wheel set that matches the user's FreeCAD is copied (about
  9 MiB); the other three ABIs stay inside the AppImage. The add-on finds
  its payload next to itself, one level above itself (the developer
  symlink) or through `KALA_FREECAD_SYS_PATH`; it never reads a repository
  checkout and never uses a virtualenv. When the payload is missing, the
  design box says so in its status line instead of failing with a
  traceback. The same procedure is in
  [studio-install.md](studio-install.md#installing-the-freecad-add-on-the-one-supported-method)
  and in [packaging/linux/README.md](../packaging/linux/README.md).

This is proven from the **extracted AppImage** under a literal `env -i`: the
provider is available, the L-bracket is `35571.416047 mm³` with
`measured_by: FreeCAD`, the drawing and the section match the OpenCascade
ones exactly, and the STEP round trip re-imports one solid with equal bounds
— with no repository, no virtualenv, no `PYTHONPATH` and no
`KALA_FREECAD_SYS_PATH` in the child environment.

### How it builds

`build()` writes a request JSON and a fixed driver script into a temporary
directory and runs:

```
<FreeCADCmd> <driver.py>          # no shell, cwd = temp dir, scrubbed env,
                                  # timeout = 1800 s, stdout = DEVNULL,
                                  # stderr = a file
```

The driver imports the adapter distribution's documented entry point
(`kala_agent_freecad.cad.design.run`), lowers the shared
`ConstructionScript`, builds the model in a FreeCAD document, exports **STEP
and FCStd**, reopens the FCStd to measure FreeCAD's own bounding box and
solid count, and writes a JSON result file. The provider re-reads that file,
verifies every artifact path is inside the workspace, re-hashes every
artifact, and only then returns a `ProviderBuild`.

The subprocess environment is **scrubbed** of credential-shaped keys
(`TOKEN`, `API_KEY`, `SECRET`, `PASSWORD`, `CREDENTIAL`) before the driver
starts, so FreeCAD never inherits the worker token or a chat API key.

A refusal or a crash is a typed `CadProviderBuildError` naming the adapter
error and the failing op. **A build is never faked.**

### A cancelled build is killed, not asked to stop

The FreeCAD build is **out of process**, so it can be stopped. When the
client closes the request, the studio cancels the design turn's token; the
provider passes that token into the supervisor, which polls it and
terminates the **provider's own process group**. The whole `FreeCADCmd`
tree is gone, and the provider reports a *cancellation*
(`the FreeCAD build was cancelled; the FreeCADCmd process group was
stopped`) rather than a build refusal.

This is the difference between a cancel that stops the work and a cancel
that only stops the answer: the out-of-process build dies, and the
in-process OpenCascade build is the one that can only be stopped at its
next phase boundary (see
[studio.md](studio.md#cancelling-a-turn)).

### The corpus: 28 / 28

The parity corpus (`tests/fixtures/construction_parity.json`, frozen from the
expectation oracle) is built by the **real** FreeCAD on this host:

```
built                       28 / 28
volume-checked              26 / 26      max relative delta 0.000e+00
bounds-checked              28 / 28      max absolute delta 3.553e-15 mm
rectangle-revolve cases     2            built, no closed-form volume
```

The two rectangle-revolve cases have no closed-form volume oracle; they are
built and their bounds are checked. The measured volumes and bounds agree
with the frozen oracle within the ADR-0009 limits (1e-6 relative, 0.001 mm).

Reproduce it:

```bash
.venv/bin/python -m pytest tests/unit/studio_design/test_cad_providers_freecad.py -q
```

Observed on this host: **8 passed in 19.96 s**. The file is marked `freecad`
and skips with a clear reason when the probe reports FreeCAD absent (or when
the OCP view importer is missing, because the bounds check needs it).

### Assembly export

A place-only assembly is exported as an **assembly of occurrences**, not as
the last component and not as a fused solid:

| Corpus case | Result |
|---|---|
| `assembly_single_translated_part` | 1 solid, volume `500.000000`, bounds `[5,6,7]…[15,16,12]` |
| `assembly_two_parts_placed_once` | 2 solids, volume `1000.000000`, bounds `[0,0,0]…[30,10,5]` |
| `assembly_three_parts_placed_once` | 3 solids, volume `600.000000`, bounds `[0,0,0]…[25,30,2]` |

The adapter builds one `Part.makeCompound` of the placed bodies and exports
that; the same target is used for the STEP, the FCStd and the measurement, so
the three cannot disagree. A compound's volume is FreeCAD's **summed** part
volume — correct for disjoint parts, and it would double-count an overlap.
The corpus has no overlapping assembly.

The expected solid count is derived from the **script**, never from the
provider's answer, so an assembly verifies on its own terms: a two-part
place-only script expects `solid_count = 2` and the check row is
`solid_count` (not `single_solid`).

### What FreeCAD does not report

`centroid_mm` and `mass_g` are `null`. The adapter's measurement block has
volume, area and bounding box; the provider reopens the FCStd for the exact
min/max and the solid count. **Kala never fills those two in.**

## Cuts: the plane rule, the check and the per-pocket warnings

A cut is the operation most likely to go wrong, so the studio says
exactly what happened instead of reporting a part that is not the part
that was asked for.

### The same-plane pocket rule

A pocket cuts into material that **its own component already has at that
step**, and the pocket's sketch plane must be the plane of the sketch that
produced the current solid. The rule is enforced where the intent is
validated, so a script that breaks it is refused **before** any build is
attempted:

```
component 'half_a': pocket at step index 3 on XZ over a XY solid
```

Over HTTP that is a `422 invalid_request` whose detail carries the same
text.

### The cut that removes nothing

When a pocket finds no material, the provider **measures** it and reports
it — it never claims a removal that did not happen. The check compares the
volume of the feature before and after the cut (FreeCAD) or of the solid
before and after the cut (OpenCascade), and the warning names the
component, the step index and the plane:

```
component 'plate': pocket at step index 3 on XY removed no material
```

Every ineffective pocket produces **its own** warning: the step index is
part of the message, so two dead cuts in the same component are two
warnings and not one.

### The check

A build whose warnings report a cut that removed no material gets **one**
extra check row:

| field | value |
|---|---|
| `id` | `cuts_effective` |
| `label` | `Cuts removed material` |
| `expected` | the number of `pocket` steps in the script |
| `measured` | `expected − (the ineffective cuts)`, never below 0 |
| `tolerance` | `0.0` |
| `unit` | `cut` |
| `passed` | `false` |

The row is appended **only** for that failure, so a passing design keeps
its exact check list. A design with a dead cut is `verified: false` and
the repair round of the turn is spent on it.

The warnings shown on screen are bounded to 16 entries (the first 14
verbatim, then at most two summary lines such as
`… and 486 more pockets removed no material`), but the **exact** number
of ineffective cuts is preserved, so `cuts_effective` stays exact even for
a 500-pocket design.

### A live example

The same 80 × 40 × 8 plate, built by both providers:

| script | volume mm³ | `verified` | warnings | `cuts_effective` |
|---|---|---|---|---|
| one Ø6 hole sketched entirely outside the plate | 25600.0 | **false** | one `removed no material` warning | expected 1, measured 0, failed |
| the normal plate with 4 × Ø6 through holes | 24695.221316 | **true** | none | **absent** |

The warning is a failure of the *intent*, and the studio reports it as
one. The design itself is never discarded silently: the turn spends its
one repair round on it and says what was wrong.

## The limits

| limit | value | consequence |
|---|---|---|
| components per script | **64** | a script with 65 components is refused |
| steps per component | **500** | a component with 501 steps is refused |
| **steps in the whole script** | **500** (every component's steps plus the assembly steps) | a script with 501 steps is refused: `a script may use at most 500 steps in total; this one uses 501` |
| **Solid Edge parts** | **8** | a script that compiles to more than 8 parts is refused before it reaches the bridge: `the script has 9 components; the wire cap is 8 parts` |
| Solid Edge bridge operations | 600 | a second line of defence; a contract-valid script cannot reach it (the maximum is 543 operations) |
| FreeCAD build deadline | **1 800 s** (up to 3 600 s) | a bigger build is killed and reported as a timeout |
| OpenCascade compile budget | 60 s | the 500-step script compiles in 16.9 s |
| adapter call list | 8 000 calls | 500 steps lower to at most 4 501 FreeCAD calls |

**The cost of a big FreeCAD build is superlinear.** Measured on FreeCAD
1.1.4 with the real `FreeCADCmd`:

| design | steps | pockets | wall time |
|---|---:|---:|---|
| a 200 × 200 × 5 plate with a 10 × 10 grid of Ø3 holes | 202 | 100 | **58.8 s** in the provider, **59.8 s** through the Studio |
| the same plate with 3 holes off the plate | 202 | 100 | ≈ 57 s |
| a 2 000 × 2 000 × 5 plate with 249 Ø6 holes | 500 | 249 | **1 009.5 s (16.8 min)** idle, **1 250 s (20.8 min)** under load |

So a **500-step FreeCAD build takes roughly 17–21 minutes**, and the
1 800 s deadline covers it with margin only when the host is idle. The
OpenCascade path is much quicker: the same 100-hole plate builds through the
Studio in **3.2 s** (2.5 s of compile time). The cost is the per-pocket
recompute with the volume check; a
future change could report the per-pocket volume without forcing a full
recompute.

## `solidedge` on Windows: the bridge, live proof pending

* `probe()` refuses a non-Windows host immediately with a reason. On Linux
  the route is a `503`, never a fabricated build.
* On Windows it resolves the bridge URL from `KALA_SOLIDEDGE_BRIDGE_URL`
  (default `http://127.0.0.1:8797`) and the token file from
  `KALA_SOLIDEDGE_TOKEN_FILE` (default
  `%LOCALAPPDATA%\Kala\SolidEdgeBridge\bridge.token`). It requires a
  loopback HTTP URL, reads the token **without logging it**, disables proxy
  handling, refuses redirects, and asks the authenticated `/v1/health`
  route. `available` is true only when the bridge itself says `available`
  and `supported`.
* The bridge milestone that has landed has **no production native executor**,
  so even a reachable Windows bridge reports no build capability.
* A native Rust client for the bridge protocol (protocol v1 plus
  `ConstructionScript` lowering) exists and is tested against the bridge
  contract fixtures only. **No live Solid Edge build has been performed**, so
  no Solid Edge part, mass property or export is claimed.

On the dev host the probe answers:

```json
{"id":"solidedge","name":"Solid Edge","available":false,"version":null,
 "reason":"Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here",
 "measured_by":"Solid Edge","mode":"bridge"}
```

## Chat and design through the front door

The Rust front door and the **Python reference server** (the parity oracle)
share the same provider rule. A chat turn carries an optional
`cad_provider`; `POST /design` carries a required `provider`. The selection
is passed through **unchanged**. The front door performs no availability
check of its own, adds no provider, and never tries a second provider after
a refusal.

| Turn through the Rust front door | What happens |
|---|---|
| `cad_provider: "freecad"` | the backend runs `FreeCADCmd` and builds the model |
| `cad_provider: "opencascade"` | the native OCCT adapter builds the model |
| `cad_provider: "solidedge"` on Linux | the backend's `provider_unavailable` refusal is relayed; `design` stays `null` |
| no `cad_provider` | the provider-choice prompt; nothing is built |

The live differential now includes the **real FreeCAD chat turn** through the
Rust front door and compares it with the live Python server: **295 cases, 0
mismatches, 0 known gaps** (the other harnesses report 268, 56 and 20 cases,
0 mismatches). On this host that turn returns design
`a9d04f8ff14e…` with `provider: "freecad"`, `provider_version: "1.1.4"`,
`measured_by: "FreeCAD"`, volume **35571.416047 mm³**, surface area
**11889.101731 mm²**, envelope 60 × 45 × 40 mm, one valid solid,
`verified: true` (5/5 checks) and a view of 568 triangles / 586 vertices.
The reply text is byte-identical to the Python server's; only the generated
STEP/FCStd digests differ, because both writers embed their build time.

Solid Edge is not buildable here. Its chat turn answers with the worker's own
sentence and builds nothing:

```
The solidedge CAD provider is not available: Solid Edge build requires the
Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge
cannot run here. Available: opencascade, freecad. No design was built.
```

No fallback provider is tried, and the turn's `design` is `null`. On a
Windows host with the bridge this remains **unproven**: no Solid Edge build
has ever been executed.

## The API

### `GET /api/studio/providers/cad`

Availability per provider, from a **live probe** — never a constant `true`.
The route requires the launch bearer (so a same-origin page that never proved
it holds the token cannot enumerate the host's CAD providers).

```json
{"providers":[
  {"id":"opencascade","name":"OpenCascade (opt-in)","available":true,"version":"7.9.3","reason":null,"measured_by":"OpenCascade (opt-in)","mode":"opt-in"},
  {"id":"freecad","name":"FreeCAD","available":true,"version":"1.1.4","reason":null,"measured_by":"FreeCAD","mode":"external"},
  {"id":"solidedge","name":"Solid Edge","available":false,"version":null,"reason":"Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here","measured_by":"Solid Edge","mode":"bridge"}
]}
```

The `opencascade` version names the **linked OpenCascade SDK** (7.9.3 on the
build host). The Python reference server reports the `cadquery-ocp` binding
it imports (8.0.1.1). The two are the same kernel family and are **not
comparable**, so the live differential replaces that one field with a
placeholder **for the `opencascade` provider only**; the FreeCAD and Solid
Edge identities are still compared exactly.

### `POST /api/studio/design`

The request is a `DesignIntent` **plus a required `provider`**:

```json
{"provider":"freecad",
 "template":{"template":"l_bracket","width":60,"depth":45,"height":40,"wall":8,"bore_d":10,"hole_d":5.5,"hole_count":4},
 "material":"aluminium","tolerances":[],"label":null}
```

The response (`DesignResult`) carries the provider identity and the
provider's own numbers:

| Field | Meaning |
|---|---|
| `provider`, `provider_version`, `measured_by` | who built it and who measured it |
| `measured` | the provider's numbers; every field may be `null` |
| `checks[]` | `id`, `label`, `expected`, `measured`, `tolerance`, `unit`, `passed`, `status` (`passed` / `failed` / `not_reported`) |
| `verified` | `all(check.passed)` — **not** "the part is good" |
| `view` | the OpenCascade **import** of the provider's STEP: `positions`, `normals`, `indices`, `vertex_count`, `triangle_count`, `deflection_mm`, `engine`, `bounds`, `solids`, `warnings` |
| `artifacts` | `script_sha256`, `step_sha256`, `native_sha256`, `native_kind` (`fcstd` for FreeCAD) |
| `design_id` | `sha256(canonical script ‖ 0x00 ‖ provider)` — the provider is part of the identity |

`design_id` is **not** the script digest: the same script built by FreeCAD
and by OpenCascade is two designs with different measured numbers. The
stored script artifact is keyed by `script_sha256`.

A failed check is **not** an error. `POST /design` answers `200` with
`verified: false` and a per-check `status`, so a declared-envelope mismatch
or a `not_reported` value is visible. A **provider refusal** is a `422
design_rejected` and stores no artifact.

### `GET /api/studio/design/{design_id}/artifact/{kind}`

The stored files of one design, downloaded:

```
GET /api/studio/design/{design_id}/artifact/{kind}
```

`kind` is one of `script`, `step`, `fcstd`, `stl` — and **only the kinds
that design stored** answer. For a FreeCAD design that is the canonical
construction `script` (JSON), the provider's `step` (the STEP the provider
exported) and the provider's own `fcstd` (the FreeCAD document, exported
and measured by FreeCAD itself). The `stl` kind answers only after the
design was exported as an STL.

The bytes are the **stored** bytes: the service re-hashes them and refuses
the download if they do not match the sha256 the journal recorded
(`500 artifact_digest_mismatch`). A stored object above **64 MiB** is a
`413 artifact_too_large`; a kind the design did not store is a
`404 artifact_not_found`; a bad id or an unknown kind is a
`422 invalid_request`. The route carries its own bearer gate.

The download filename is `<label-slug>-<digest8>.<ext>` — for FreeCAD, the
`.FCStd` extension. Full detail in
[studio.md](studio.md#downloading-what-a-design-stored).

### Errors

| Status | `code` | Cause |
|---|---|---|
| 422 | `invalid_request` | missing provider (the detail names all three values), unknown provider, invalid intent |
| 422 | `design_rejected` | the provider refused or failed, or its STEP could not be imported |
| 422 | `invalid_step` / `step_too_complex` | the uploaded STEP is unreadable / still too complex after one coarser retry |
| 404 | `design_not_found` | no completed provider build under that id |
| 413 | `payload_too_large` | the STEP import is over `KALA_STEP_IMPORT_MAX_BYTES`; every other route is over 262 144 bytes |
| 499 | `design_cancelled` | the client closed the request; the turn stopped and nothing after it was journalled |
| 503 | `provider_unavailable` | the probe says unavailable; the detail carries the reason and the available ids |
| 503 | `design_unavailable` | the OpenCascade kernel cannot execute, so the view importer cannot run (a portable build links no kernel) |

### The STEP import ceiling

The view-only import has its **own** limit, separate from the shared 256 KiB
Studio body limit:

| Value of `KALA_STEP_IMPORT_MAX_BYTES` | Result |
|---|---|
| absent or blank | the 64 MiB default |
| an integer in `1 … 536 870 912` (1 B … 512 MiB) | that value |
| anything else | a startup refusal, with the worker's own message |

The front door resolves it **once**, enforces it on the public import route,
and hands the same number to the worker process when the development
fallback is in use, so both sides accept exactly the same body. The front
door never raises the worker's ceiling.

## The journal

A successful build writes, in order:

```
design.intent → provider.build.requested → provider.build.completed
  → artifact.stored (script) → artifact.stored (step) → artifact.stored (native)
  → view.imported → design.compiled (opencascade only) → design.measured
  → design.checked
```

A refusal writes `provider.build.failed` + `design.rejected` and stores no
artifact. A cache hit writes only `design.intent` with `cache_hit: true` and
reloads the stored STEP; it does not rerun FreeCAD or the OCCT kernel.

A **cancelled** turn writes `design.intent` and `provider.build.requested`
(they happened) and then exactly one `design.cancelled`, and **nothing
after it**: no build completion, no `artifact.stored`, no measurement, no
check. The half-built design is discarded and is never journaled as a
design.

The journal itself is now **native** (`kala-studio-journal`): the same
hash-chained SQLite file, the same rows and the same chain digests as the
Python journal, which can still open and extend it.

A design restored from the journal keeps its measured surface area. The
cold-reload reader reads the stored `surface_area_mm2` key, with the legacy
`area_mm2` key consulted only as a fallback; before this fix a restored
design showed `—` for area on both front doors. See
[studio-limitations.md](studio-limitations.md#the-cold-reload-surface-area-bug-fixed).

`view.imported` and `provider.build.*` are **not** members of the ADR-0009
`JournalEventType` enum; the journal accepts safe identifiers, so consumers
match the strings.

## The UI

The composer's **CAD-target selector** is the only way to choose a provider.

* The rows come from `GET /api/studio/providers/cad`; the choice is written
  to the **in-memory session store only** (it is never written to
  `localStorage`, and it is not a secret).
* **There is no default.** With no choice the chat request carries
  `cad_provider: null`, the backend replies with its provider-choice prompt
  and builds nothing, and the selector says
  *"No CAD target selected — the first design turn will ask you to choose."*
* An unavailable provider is **disabled** and carries the backend's `reason`
  in the row and in the button tooltip. It is never guessed and never
  silently replaced.
* OpenCascade is always labelled opt-in.
* The probe is gated on the backend reporting `design_available === true`:
  without a view importer the selector shows the capability note instead of
  calling a route that may not exist.
* A `503 provider_unavailable` body already carries the reason and the
  available ids; the UI renders those instead of a generic error.

## Honest limitations

1. **Solid Edge has no live proof, and no fallback is tried.** The Windows
   bridge, the protocol v1 client and the lowering are tested against
   fixtures; no Solid Edge build has been executed on any host. On Linux the
   bridge cannot run here, so the backend refuses with its reason and the
   providers that can build; a chat turn leaves `design` `null`. The Rust
   front door relays that refusal in the backend's own wording and does not
   build with another provider. The bridge milestone that has landed also
   has **no verified native build executor**, so even a reachable Windows
   bridge reports no build capability. The bridge also caps a procedure at
   **8 parts**, which is far below the 64-component script budget: a
   Solid Edge design can never use the whole budget.
2. **A 500-step FreeCAD build is slow** — roughly 17–21 minutes, measured.
   The contract allows it and the 1 800 s deadline covers it, but it is not
   a pleasant chat turn, and the margin shrinks on a loaded host.
2. **The providers are native now, and the identity difference is declared.**
   The front door computes the geometry itself (through `kala-cad`) and owns
   the provider connections (through `kala-cad-providers`). The two
   **declared** differences against the Python oracle are the OpenCascade
   binding identity and the display-mesh vertex order; the geometry, the
   measurements, the checks, the drawing and the section are identical.
3. **A provider that reports nothing is reported as nothing.** `not_reported`
   is a first-class state in the result, the checks and the UI.
4. **No mass, no centroid, no material database.** `mass_g` is `null` in
   this round for every provider; a declared `material` string is not a
   density and no density is inferred from it.
5. **The STEP import retries once.** A tessellation failure is retried once
   at a coarser deflection (`0.5 mm` / `0.8 rad`) with a warning and the
   coarser `deflection_mm` reported. A second failure is `step_too_complex`.
   It never truncates.
6. **A compound's volume is the sum of its solids.** Correct for the
   disjoint corpus; an overlapping assembly would be double-counted.
7. **The construction subset is bounded.** `helix`, `fillet`, `move`,
   `resize` and `compound` are refused by name; Kala never approximates a
   thread as a cylinder or skips a fillet.
8. **A design with no `provider.build.completed` event is not loadable.**
   Pre-provider designs in an old session journal return `404`; the journal
   itself stays verifiable.
9. **FreeCAD must be installed on the target.** It is detected, never
   bundled, and the provider reports the reason honestly when it is absent.
   The vendored adapter dependencies cover **cp311–cp314**; another Python
   is reported as unavailable with its exact version.
10. **Only the out-of-process build can be killed mid-flight.** A cancelled
    FreeCAD build has its whole process group terminated, promptly. The
    in-process OpenCascade build cannot be interrupted inside a single
    compile, so a cancel there is honoured at the next phase boundary —
    the worst case is one **60 s** compile. And the **Python reference**
    cannot cancel a running synchronous build at all (its route runs in a
    FastAPI thread pool, and a thread cannot be dropped by the event
    loop); the cancellation described here is the **native** path, which is
    the shipped one.

## Primary sources

- ADR-0007 (optional FreeCAD CAD/CAE/FEM adapter),
  ADR-0008 (Solid Edge Windows bridge), ADR-0009 (Studio design turns),
  ADR-0010 (Rust control plane, CAD kernel geometry) and ADR-0011 (the CAD
  provider model) in [adr/](adr/).
- `docs/construction-semantics.md` pins the construction semantics that the
  provider corpus is measured against.
- The **native** provider implementation:
  `backend/crates/kala-cad/` (the OpenCascade adapter),
  `backend/crates/kala-cad-providers/` (the FreeCAD and Solid Edge
  connections) and `backend/crates/kala-studio/src/native_geometry/`
  (the design service).
- The **Python** provider implementation, kept as the differential oracle:
  `src/kala_agent/studio/design/providers/` (`base.py`, `opencascade.py`,
  `freecad.py`, `solidedge.py`) and `src/kala_agent/studio/design/service.py`.
- The packaging contract for the bundled adapter:
  `packaging/linux/README.md` and `packaging/windows/README.md`.
