# docs-cutover-2 REPORT: the last three stale documents brought in line with the Rust-only cutover

Cell: `cutover-20261020/docs-cutover-2` (docs worker).
Scope: `docs/studio.md`, `docs/studio-limitations.md`,
`docs/studio-providers.md` — **and nothing else**.
No code change, no commit, no push, no delegation, no credential read, no
sudo.

**The user's real data was never touched.** Every shell in this cell
exported the CONVENTIONS USER DATA rule
(`RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`, and a fresh
`mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and
`XDG_DATA_HOME`) before it ran anything. The only commands that touched
the filesystem were `ls`, `grep`, `stat` and `sha256sum` over the
**repository** and over the **build artefact** in `studio/release/`. The
user data tree was never read, written, listed or named in a command.

---

## 1. Outcome

| Item | Result |
|---|---|
| `docs/studio.md` | **rewritten in the affected sections** — the two-layer architecture, the native geometry, the 16/0 route surface, the worker as a development-only opt-in, the packaging truth, and the "what this wave does not ship" list |
| `docs/studio-limitations.md` | **rewritten in the affected sections** — the front-door/worker section, the "release packaging is blocked" section, the packaging trust and provenance, the testing procedure, the test-suite table, the corpus section and the "did not test" list |
| `docs/studio-providers.md` | **updated** — the native service is the product, the four provider ids and their behaviour are unchanged, the `codex` refusal is quoted, the credential and consent rules are stated against the native build, and the trust recap no longer implies a shipped geometry worker |
| Code | **none.** No `.rs`, `.cjs`, `.ts`, `.py`, manifest, workflow or fixture was touched. |
| Commit / push / delegation / credential read / sudo | **none.** |

The three documents now say the same thing as the source of truth
(`docs/rust-studio.md`, `docs/studio-install.md`,
`docs/rust-migration-status.md`, `docs/cad-providers.md`, and the
`docs-cutover` cell report).

---

## 2. Changed paths

Exactly three files, all documentation:

| Path | Change |
|---|---|
| `docs/studio.md` | header status block; "How it is put together"; the design-turn diagram note; the AuxView label note; the journal single-writer paragraph; the whole "The API contract" section; the whole "The native Studio service and the worker fallback" section (including the "front door" → "service" wording in that section, so it reads consistently with the rest of the document); the health highlight; the security paragraph; "What the studio does not claim"; "How the desktop shell runs"; "Primary sources"; "What this wave does not ship" |
| `docs/studio-limitations.md` | header status block; the Solid Edge bullet; the FreeCAD bullet; the CAD-provider bullet; the STEP-export and B-rep bullets in "Geometry limits" / "Nominal geometry estimates"; the whole "The native Studio service and the Python geometry worker" section; the cold-reload bug section; one "What is still open" bullet; the whole "Packaging trust" section; the whole "Testing procedure" section; the test-suite table; the whole "The frozen corpus is not all contract" section; the whole "What this wave did not test" list |
| `docs/studio-providers.md` | header status block; the provider-id paragraph; the local-helper section; the `codex` section; the OpenAI and OpenRouter sections; the telemetry bullet; the trust-model recap |

**Not touched** (and therefore still stale — see §7): `.github/workflows/studio.yml`,
`packaging/**`, `studio/scripts/**`, `studio/electron/**`, every Rust crate
and every Python file. `docs/studio-install.md`, `docs/rust-studio.md`,
`docs/cad-providers.md` and `docs/rust-migration-status.md` were already
brought in line by the `docs-cutover` cell and were used here as the source
of truth, not edited.

---

## 3. What was removed, and what replaced it

### 3.1 The "6 native / 10 proxied" split — **removed**

It appeared in three places and is gone from all three.

| Where | Before | After |
|---|---|---|
| `docs/studio.md` §"The API contract" | "**Rust-native (6):** … **Proxied to the geometry worker (10):** …" | a per-route owner table: **all 16 route-methods are answered natively, in-process**, with the note that the readiness report still *labels* ten of them "proxied" because they travel through the geometry-backend seam, and that the default build answers all sixteen |
| `docs/studio-limitations.md` §"Release packaging is blocked" | "the readiness scanner is green … 6 Rust-native and 10 proxied route-methods" | the section was replaced entirely (§3.2) |
| `docs/studio-limitations.md` §"The test suites behind these claims" | "`npm run check:native-readiness` … READY, 6 Rust-native and 10 proxied route-methods, 0 missing" | "**READY**, 15 Python route-methods covered, 16 in the native service, 0 missing; the built stage is classified `release` / `releaseEligible: true`" |

The wording is deliberately *not* "the scanner now says 16/0": the
**scanner's own output still says 6/10**
(`studio/release/native-readiness.json`: `rustNativeRouteMethodCount: 6`,
`proxiedRouteMethodCount: 10`), with a warning that the default build
answers them from the native backend. The documents say exactly that.

### 3.2 The "release packaging is blocked" section — **removed**

The heading `### Release packaging is blocked, pending a decision` and the
whole of its body are gone. It is replaced by
`### The release packaging state` in `docs/studio-limitations.md`, which
states what is true:

* the **Linux AppImage is built and smoke-tested** — path, 163 642 516
  bytes, sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`,
  and the measured contents of the `env -i` smoke (the OpenCascade
  L-bracket `35571.416047113315 mm³`, the exact XZ section
  `2321.4601836602546 mm²`, the STEP round trip, the FreeCAD build);
* the old `metadata.nativeStep === true` release rule is **replaced** by
  the native-design readiness report, a stage without the bundled FreeCAD
  adapter is refused, and publication is forced to `never`;
* the **Windows installer was never produced** — the script is written and
  reviewed but has **never been executed**;
* the **Electron window was never rendered** (`xvfb-run` absent), so the
  smoke verifies the packaged backend and the served UI root by extraction;
* the AppImage is **unsigned and unpublished**, and the release workflow
  was written but not pushed or run.

**The old anchor is gone.** `studio-limitations.md#release-packaging-is-blocked-pending-a-decision`
no longer exists. The only in-repository reference to it was the
`docs-cutover` cell's own `REPORT.md` (a historical cell report, not mine
to edit) and `docs/studio.md`, which now points at
`studio-limitations.md#packaging-trust`. **The conductor should know that
the anchor moved** in case anything external links to it.

### 3.3 The Python worker as a product layer — **removed**

| Where | Before | After |
|---|---|---|
| `docs/studio.md` §"How it is put together" | "Three layers": a Rust front door that "computes **no geometry**", a **supervised Python geometry worker** that owns the CAD kernel work, and the UI | "Two layers": the **native Studio service** that owns the HTTP boundary **and the geometry** (through `kala-cad` over the real OCCT 7.9.3), and the UI. The Python worker is a **development-only opt-in**; the packaged app starts no Python process at all |
| `docs/studio.md` §"What is Rust-native and what is proxied" | a table whose "Geometry, measurement, drawing, section, the STEP import, the strict intent parser, the journal" row said **Python geometry worker** | §"What the native service owns": every one of those concerns is now attributed to its **Rust** crate (`kala-cad`, `kala-design-intent`, `kala-studio-journal`, `kala-cad-providers`, `kala-geometry`/`kala-step`), and the worker row says only "a development-only opt-in" |
| `docs/studio.md` §"Running it in development" | the dev run **required** `KALA_STUDIO_GEOMETRY_WORKER`, and "the development default for the worker command is `<repo>/.venv/bin/python …`" | the dev run needs **no worker, no Python, no virtualenv**; the worker became §"The worker fallback", with "**The front door starts no worker at all unless this variable names one**, and there is **no development default** any more" |
| `docs/studio.md` §"The journal …" | "The **worker** is the single writer of the journal file" | "The journal is the single writer of its own file … The journal is **native** now (`kala-studio-journal`): the same hash-chained SQLite file, the same rows and the same chain digests as the Python journal" |
| `docs/studio-limitations.md` §"The Rust front door and the Python geometry worker" | "two back ends … deliberate (ADR-0010)"; "**Two runtimes ship** in development and, if the packaging decision lands …, in the release bundle too"; "no cutover has been performed"; the primitive crates "are **parked** … behind a non-default Cargo feature" | §"The native Studio service and the Python geometry worker": "**one** back end on the product path"; "**One runtime ships**"; the worker is "a **development-only opt-in** … started **only** when `KALA_STUDIO_GEOMETRY_WORKER` names one, and there is **no development default** any more"; the primitive crates are "**on the default path**"; the two declared differences are named |
| `docs/studio-limitations.md` §"The CAD providers" | "**Both front doors use the same worker** and the same provider rule" | "**The service and the Python reference share the same provider rule**" |
| `docs/studio-limitations.md` §"What this wave did not test" | "The wave did **not** cut over anything … no Rust crate is wired into a running service" | "The wave cut over the **Linux desktop** (ADR-0010 phase 2) and did **not** cut over the rest of the product" (the agent runtime is still not composed into the Studio service) |

### 3.4 The other pre-cutover claims that were removed or corrected

| Claim | Where it was | What replaced it |
|---|---|---|
| STEP export needs the Python `[geometry]` extra (`cadquery-ocp>=7.7`) | `studio-limitations.md` ×2 | the default build links OpenCascade 7.9.3 through `kala-cad`; a portable `--no-default-features` build reports `design_available: false` and answers `503` |
| "Exact B-rep confirmation needs OCP" | `studio-limitations.md` | "Exact B-rep confirmation needs the kernel" — `BRepGProp` through the native adapter (system OCCT 7.9.3 vs the oracle's `cadquery-ocp` 8.0.1.1) |
| the CI matrix is `ubuntu-latest`/`windows-latest` with PyInstaller + electron-builder jobs | `studio-limitations.md` §Provenance | the Linux artefact was built **on the development host**; the release workflow is **written but not pushed or run**; no hosted run was inspected; the old `studio.yml` still describes the pre-cutover matrix and the retired `nativeStep` marker; the OCCT closure is host-built |
| the AppImage is "not proof that a desktop session works" / "this step is currently blocked" | `studio-limitations.md` §Testing procedure | the AppImage **is** built and smoke-tested (with the measured numbers), and the *unproven* part is named precisely: the Electron window was never rendered and the AppImage was not tried on a clean Linux desktop |
| the corpus replay is **red** (56 cases, 5 mismatches) and `corpus_fixture.rs` fails (`left: 56, right: 48`) | `studio-limitations.md` | the replay is **green** — 56 cases, **0 mismatches** (`exact: 48`, `fields: 4`, `step structural: 2`, `skipped: 2`, digest unchanged) — and the honest caveat about the **host-dependent** records is kept and strengthened |
| `cargo test --workspace` = 423, `npm test` = 619 tests | `studio-limitations.md` §test suites | **449** and **628**; the `kala-cad` (118), `kala-studio-journal` (60) and `kala-cad-providers` (40) rows were added, and the "when" column now distinguishes the **cutover wave** (re-run) from the **earlier pass** (carried over) |
| the desktop bundle is not shippable, the packaging decision is pending | `studio.md` §"What the studio does not ship", `studio.md` §"What the studio does not claim" | the AppImage is built, **unsigned**; the Windows installer is a script that has never been executed; the MinGW cross-check is pending |
| the CI pipeline builds and uploads the back door and the desktop bundle | `studio.md` §"What this wave does not ship" | the release workflow is written but was not pushed or run; nothing is published |
| the "local helper" restates "the OpenCascade (OCP) estimate" | `studio-providers.md` | "the OpenCascade estimate" (the reply text is the ported, byte-identical local provider) |
| the strict intent parser is Python-only | `studio-providers.md` | the native build runs `kala-design-intent`; the Python reference runs the same contract |
| the FreeCAD provider must not inherit "the chat API key or the geometry-worker token" | `studio-providers.md` §Trust model recap | the chat API key always; the geometry-worker token only in the development fallback (the token no longer exists in the product path) |
| "the CI matrix does not upload any environment variable …" | `studio-providers.md` §Trust model recap | "the release workflow does not …" |
| `provider=codex` refusal described only in prose | `studio-providers.md` | the exact refusal sentence of the native build is quoted, and the `codex` row is confirmed `configured: false` |

---

## 4. What was **kept** (the honest limitations)

Every limitation that is still true is still in the documents, and the
cutover wave did not remove any of them:

* **No FEA, no CFD, no solver of any kind**; no displacement, no stress,
  no von Mises value, no factor of safety, no fatigue life, no pass/fail
  verdict, and no placeholder stub.
* **Nominal estimates are not analysis**; a measured volume is a geometric
  fact, not a load result.
* **Tolerances are declared, not analysed** — no fit class, no limit, no
  deviation, no stack-up, no GD&T; the UI says *"declared — not analysed"*.
* **"B-Rep Verified" means exactly three things** and nothing more.
* **STEP geometry is not ready-to-solve analysis.**
* **No material database; no density is inferred** from a material label.
* **No unit system beyond length.**
* **No certification, no regulatory claim.**
* **The session journal is a session log, not a revision system.**
* **The clay viewport is a narrow-field-of-view perspective camera** (38°),
  not an orthographic one; the 2D pencil mode uses a 24° perspective; the
  sheet `SCALE` cell reads `FIT`; the section plane is not operator-movable.
* **WebGL renders triangle meshes, not exact B-rep**; tessellation is not
  exact (the sphere is `1.066 %` below the analytic volume).
* **The construction subset is bounded**; a compound's volume is the sum
  of its solids; the STEP import retries once and never truncates.
* **`not_reported` is a first-class state**; a provider that reports
  nothing is reported as nothing; no centroid, no mass.
* **No default CAD provider**; **no default language-model provider**;
  **no fallback provider**; the `codex` route fails closed; no arbitrary
  provider escape hatch; no prompt caching; no telemetry; no arbitrary LLM
  code execution; no app/OpenCode orchestration.
* **Solid Edge has no live proof** on any host and needs a **Windows
  host**; the bridge milestone has no production native executor.
* **The `codex` route is not implemented**; a `codex` binary on `PATH`
  does not make it usable; `KALA_STUDIO_ENABLE_CODEX` is inert.
* **No solver is launched, no solver result file is read, no plausible
  stand-in number is produced.**
* **The test-hygiene and incident notes are untouched**: the USER DATA
  rule block, the `cli-r1` incident of 2026-10-20 05:35:30 +0530 and its
  bounded-exposure assessment with its two caveats, the isolation work
  table, the `stat`/`ls` proof (357 polls, identical size/mtime/inode, 0
  `-shm`/`-wal`), and the three still-open items.
* **The core API status bug** and **the cold-reload surface-area bug**
  sections are kept, with the correction that the cold-reload reader fix
  is now in the **native** service as well.

---

## 5. The state of the world these documents now describe

| Item | How the documents now state it |
|---|---|
| The product | native `kala-studio` with the **real OpenCascade 7.9.3** through `kala-cad` (one narrow C++ adapter, `src/ffi/adapter.cpp`); `kala-cad-providers` for the connections; `kala-design-intent` for the parser; `kala-studio-journal` for the journal |
| The Python worker | a **development-only opt-in**; started only when `KALA_STUDIO_GEOMETRY_WORKER` names one; **no development default**; not in the release bundle |
| The route surface | **16 native, 0 proxied**; the readiness report is READY and classifies the built stage `release` / `releaseEligible: true` |
| The AppImage | **built and smoke-tested**; 163 642 516 bytes; sha256 `b069b7d4…`; **unsigned**; **unpublished** |
| Windows | the build script is **written and reviewed but has never been executed**; no Windows runtime is claimed |
| The MinGW check | **pending** on the missing `x86_64-w64-mingw32-gcc`; the `windows-check` cell has not filed a report |
| Solid Edge | needs a **Windows host**; **no Solid Edge build has ever been executed** |
| FreeCAD | **detected, never bundled**; the adapter and its dependencies are vendored per ABI for **cp311–cp314**; an unsupported Python is reported as provider-**unavailable** with its exact version, never as a crash |
| The differentials | **292 / 268 / 20 / 56 cases, 0 mismatches** |
| The two declared differences | the OpenCascade binding identity (system 7.9.3 vs `cadquery-ocp` 8.0.1.1) and the display-mesh vertex order; the geometry, the measurements, the checks, the drawing and the section are identical |

---

## 6. Commands run in this cell

Every command ran with the USER DATA rule in force. **No binary and no
Python entry point was executed** — this is a documentation cell, so the
only commands were reads, greps, `ls`, `stat` and `sha256sum`.

| Command | Result |
|---|---|
| `sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage` | `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e` |
| `stat -c '%s bytes' studio/release/kala-studio-0.1.0-x86_64.AppImage` | `163642516` — the two numbers quoted in the documents |
| `cat studio/release/native-readiness.json` | `classification: release`, `releaseEligible: true`, `ready: true`, 15 Python route-methods, 16 native route-methods, `rustNativeRouteMethodCount: 6`, `proxiedRouteMethodCount: 10` + the warning that the default build answers the seam natively |
| `grep` over `backend/crates/kala-studio/src/{schemas,providers/mod.rs,cli.rs,boundary.rs}` and `src/native_geometry/mod.rs` | the four `ProviderId` values, the `codex` refusal string and `configured: false`, the "start a worker only when `KALA_STUDIO_GEOMETRY_WORKER` names one" branch, the `default_studio_data_dir` resolution order, and the native `kala_design_intent` parser — i.e. every claim the three documents make about the code |
| `grep` over `docs/` for the removed claims | `release-packaging-is-blocked`, "Release packaging is blocked", "6 Rust-native", "10 proxied" → **no matches** anywhere under `docs/` |
| heading dump of the three documents | every internal anchor used by the three documents and by the other documents was checked against a real heading |

**Not run** (and not claimed): `cargo test`, `cargo clippy`, `cargo fmt`,
`npm test`, `npm run build`, `npm run check:native-readiness`, the
`env -i` AppImage smoke, `packaging/windows/build-windows.ps1`, any
Windows check, any Solid Edge build, any benchmark. Every number in the
three documents is **attributed** to the cell that measured it
(`docs-cutover`, `rust-studio`, `studio-install`, `rust-migration-status`),
except the two artefact numbers re-checked above with `sha256sum` and
`stat`.

---

## 7. Remaining gaps and honest notes

1. **The `studio-limitations.md#release-packaging-is-blocked-pending-a-decision`
   anchor is gone.** The only in-repo reference to it was the
   `docs-cutover` cell report, which is a historical artefact I did not
   edit. If anything outside the repository links to it, it will 404; the
   replacement is `studio-limitations.md#packaging-trust`.
2. **The readiness report still says "6 native / 10 proxied".** The
   documents now explain *why* the scanner's labels differ from the
   product's shape (the seam versus the default build) rather than
   pretending the scanner changed. Changing the scanner is a code change
   and was out of scope.
3. **The `SOURCE: OCP HLR` stamp in the UI still says "OCP"** even though
   the drawing now comes from the native OCCT adapter. That is the literal
   on-screen label in `studio/src/components/drafting/AuxView.tsx`; the
   documents quote it and say so explicitly. Changing the label is a UI
   change and was out of scope.
4. **`.github/workflows/studio.yml` is still stale** — it describes the
   pre-cutover CI matrix and the retired `nativeStep` marker, while
   `.github/workflows/studio-release.yml` is the replacement. Both files
   are not mine; the documents state the state of the workflows rather
   than hiding it.
5. **The `chat_design.rs` module comment still says the front door "asks
   the worker"** for the intent parse. The code routes through the
   geometry-backend seam, which the default build answers natively
   (`kala-design-intent`); the *comment* is stale, the behaviour is not.
   Code was out of scope.
6. **The corpus host-evidence caveat is kept, not closed.** The
   `studio.cad_providers` case records the CAD-provider probe of the
   machine that generated the corpus; the harness now compares it against
   the current host's probe instead of the raw bytes, but the record is
   still host evidence and the document says so.
7. **No new number was measured in this cell** beyond the artefact's
   size and digest. The documents do not claim one.
8. **No commit, no push, no delegation, no credential read, no sudo** was
   performed, and no code was changed. The three documents are the only
   artefacts.

---

## 8. Reproduce

```sh
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
cd /home/v9i/Projects/Kala-Agent

# the two artefact numbers quoted in the documents
sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage
stat -c '%s bytes' studio/release/kala-studio-0.1.0-x86_64.AppImage

# the readiness report the documents describe
cat studio/release/native-readiness.json

# the removed claims are gone from every document
grep -rn "release-packaging-is-blocked\|6 Rust-native\|10 proxied" docs/ || echo "no matches"

# the source of truth these documents follow
sed -n '1,120p' docs/rust-studio.md
sed -n '1,80p'  docs/studio-install.md
sed -n '360,420p' docs/rust-migration-status.md
```
