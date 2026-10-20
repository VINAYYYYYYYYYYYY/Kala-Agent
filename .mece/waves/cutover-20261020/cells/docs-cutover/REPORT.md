# docs-cutover REPORT: the Rust-only app, the AppImage and the Windows script, documented

Cell: `cutover-20261020/docs-cutover` (docs worker).
Scope: the six owned documents plus **one** dated note appended to ADR-0010.
No code change, no commit, no push, no delegation, no credential read, no sudo.

**The user's real data was never touched.** Every shell that ran a binary or
a Python entry point exported the CONVENTIONS USER DATA rule
(`RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`, and a fresh
`mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and
`XDG_DATA_HOME`), and every backend run used an explicit temp data
directory. The user data tree was never read, written, listed or named in a
command.

---

## 1. Outcome

| Item | Result |
|---|---|
| `docs/rust-migration-status.md` | **updated** — the module map, the cutover, the declared native differences, what remains, the evidence tables and the short version |
| `docs/studio-install.md` | **rewritten** — the AppImage, the native developer mode, the Windows build script (prerequisites, steps, checklist) and the troubleshooting list |
| `docs/rust-studio.md` | **rewritten** — the front door is now the native Studio service; the 16-route surface, the worker as an opt-in fallback, the staging, the AppImage and the `env -i` smoke |
| `docs/cad-providers.md` | **updated** — the native `opencascade` provider (OCCT 7.9.3), the native `freecad` connection with the bundled per-ABI adapter, the honest unsupported-ABI reason, the journal, the honest limits |
| `backend/README.md` | **updated** — the workspace map (`kala-cad` 118, `kala-studio-journal` 60, `kala-cad-providers` 40, `kala-studio` 175), the owner table, the status, the "still Python" and "pending" tables, the counts |
| `studio/README.md` | **updated** — the Rust-only back door, the staging layout, the unsigned AppImage, the honest limits |
| `docs/adr/0010-…md` | **one dated note appended** ("Phase 2 implemented (2026-10-20)") + a pointer in the Status row. The decision was **not** rewritten. |
| Code | **none.** No `.rs`, `.cjs`, `.ts`, `.py` or manifest was touched. |

---

## 2. Changed paths

Exactly seven files, all documentation:

| Path | Change |
|---|---|
| `docs/rust-migration-status.md` | header (no longer "no runtime cutover"), a new status vocabulary row, §2.1 (the `kala-studio` row), §2.2 (`kala-contracts`, `kala-design-intent`, `kala-cad-providers` updated; `kala-cad` and `kala-studio-journal` added), §2.3 (parked → none), §3 (split into 3.1 the cutover / 3.2 the Python surfaces), §4.8 (new: the native geometry differences; the old 4.8 became 4.9), §5.1 (Windows/release), §5.3, §5.4, §5.6, §5.7, the new §5.9 (the packaging residuals), §6.1/§6.2/§6.3, §7 |
| `docs/studio-install.md` | full rewrite (see §3.1 below) |
| `docs/rust-studio.md` | full rewrite (see §3.2 below) |
| `docs/cad-providers.md` | the one-paragraph version, the provider table, the `opencascade` section, a new "the installed app: FreeCAD is detected, not bundled" section, the chat/design section, the `providers/cad` example, the error table, the STEP ceiling, the journal, the honest limits, the primary sources |
| `backend/README.md` | the member tables, the "Studio service" section, the launch contract, the live differential, the status table, the workspace layout, the accepted capabilities, "what is still Python", "what is pending", the build/test counts, the security boundaries, the excluded members, the scope limits |
| `studio/README.md` | the header, the quick reference, a new "the back door is Rust-only" section, the build outputs, the honest limitations |
| `docs/adr/0010-rust-control-plane-cad-kernel-geometry-worker.md` | the Status row points at the note; **one** appended section `## Phase 2 implemented (2026-10-20)` |

**Not touched** (and therefore **still stale**, see §7): `docs/studio.md`,
`docs/studio-limitations.md`, `docs/studio-providers.md`,
`.github/workflows/studio.yml`, `packaging/**`, `studio/scripts/**`,
`studio/electron/**`, every Rust crate and every Python file.

---

## 3. What the documents now say

### 3.1 `docs/studio-install.md` (rewritten)

* A "deliverables at a glance" table: the Linux AppImage (**built and
  smoke-tested**, path, 163 642 516 bytes, sha256, **unsigned**), the Windows
  NSIS installer (**not built**), FreeCAD (**not bundled**, detected on the
  system, cp311–cp314 vendored), Solid Edge (Windows only, unproven).
* **Section 1 — the Linux AppImage**: how to run it, what is inside
  (`kala-studio` linking OCCT 7.9.3, the 23 staged `libTK*.so*` with
  `RUNPATH=$ORIGIN/lib`, `studio_dist`, the FreeCAD adapter with the four
  per-ABI wheel sets, `native-build.json`) and — explicitly — what is
  **not** inside (any interpreter, `libpython`, `site-packages` or
  PyInstaller payload). The "the Electron window was never rendered" caveat
  is stated where the user will read it.
* **Section 2 — developer mode**: no venv, no `pip install`, no worker. The
  worker is a **development-only opt-in** with the exact
  `KALA_STUDIO_GEOMETRY_WORKER` semantics, including that **there is no
  development default any more**.
* **Sections 3 and 4**: the Python reference server and the browser fallback,
  unchanged in substance.
* **"Building the Linux AppImage yourself"**: the script, the manual chain,
  the release gate, and the headless smoke with the `env -i` invocation.
* **"Building and installing on Windows"** — the requested procedure:
  the prerequisites table (Windows 10/11 x64, VS Build Tools 2022 with
  "Desktop development with C++", rustup `stable-msvc`, Node ≥ 22.12, git,
  vcpkg `opencascade:x64-windows`, FreeCAD 1.1.x optional), the one-time
  vcpkg setup, the two invocation forms, the seven switches, the **nine
  numbered steps** the script performs, the installed layout, the install
  steps, and the **ten-item post-install smoke checklist**. The section opens
  with a bold "**The script has not been run.**".
* A troubleshooting table extended with the new failure modes: the
  `design_available` / `geometry_worker` health blocks, the honest
  unsupported-FreeCAD-Python reason, and "the Windows installer was never
  produced".

### 3.2 `docs/rust-studio.md` (rewritten)

* The front door is the **Studio service**, not a front door in front of a
  worker: **two layers** (Rust service + React UI) with the Python reference
  server as the oracle.
* **"The route surface: 16 native, 0 proxied"** with a per-route owner
  table.
* "The native geometry backend" and what it is built on (`kala-cad`,
  `kala-cad-providers`, `kala-design-intent`, `kala-studio-journal`,
  `kala-contracts`, `kala-json`/`kala-persistence`, `kala-geometry`/
  `kala-step`).
* "The geometry worker is now a development fallback" — the whole launch
  contract, with the opt-in stated first.
* The two **declared native differences** (the kernel identity and the
  display-mesh vertex order) with the numbers.
* The live differential: 292 / 268 / 56 / 20 at **0 mismatches**, and the
  explanation that the corpus replay used to be red and is now green.
* "Native staging and packaging": the staged tree, the OCCT closure and the
  `patchelf` contract, the native-only Electron launch, the release gate.
* "The Linux AppImage (built and smoke-tested)" with the path, the size, the
  sha256, the classification and the **unsigned** status; then the full
  two-phase `env -i` smoke tables.
* "The Windows build (written, **not executed**)" — a short section that
  points at the install guide and repeats that it has never been executed.
* The verification tables: what was run in this pass and what was carried
  over, and what was **not** run.
* "Honest limits" — the agent runtime is not cut over, Solid Edge is
  unproven, Windows is unexecuted, the AppImage is unsigned, the Electron
  window was never rendered, no benchmark was run.

### 3.3 `docs/cad-providers.md` (updated)

* The `opencascade` provider is now the **native `kala-cad` adapter over the
  real OpenCascade 7.9.3**, not the Python OCP compiler. Its section states
  the measured L-bracket, the exact section area, the drawing counts, the
  declared binding difference, and that a portable build reports
  `design_available: false`.
* A new section documents the **installed app**: the adapter and the shared
  `kala_agent` tree are bundled, the four per-ABI wheel sets are vendored, the
  runtime probes `FreeCADCmd`'s Python and puts **only** the matching
  directory on `sys.path`, an unsupported Python is an honest unavailable
  reason naming the version, and the `mod/Kala` addon is staged, not
  installed. The extracted-AppImage proof is quoted.
* The `providers/cad` example now reports the **7.9.3** SDK version, with the
  explanation that it names a different binding than the Python oracle's
  8.0.1.1 and that the harness normalises that one field.
* The journal section notes that the journal itself is now native and
  byte-compatible.
* The honest-limit list: Solid Edge (unchanged, plus "no verified native
  build executor"), the **native providers and the declared identity
  difference** (replacing the old "the Rust front door is the front door
  only" item), FreeCAD not bundled, and the unsupported-ABI behaviour.

### 3.4 `docs/rust-migration-status.md` (updated)

* The header no longer says "no runtime cutover"; it says the Linux cutover
  has happened and lists what is still not claimed (Windows runtime, Solid
  Edge, signing, publication, speed/readiness).
* A new status **"accepted, on the product path"**.
* The module map: `kala-studio` 175 and on the product path; `kala-cad`
  (118) and `kala-studio-journal` (60) added as rows; `kala-cad-providers`
  40; `kala-contracts`, `kala-design-intent`, `kala-geometry` and `kala-step`
  marked on the default path.
* §2.3 "Parked crates: none" — the three crates ADR-0010 parked, and what
  released them.
* §3.1 "The cutover that has happened" as a before/after table, and §3.2 the
  Python surfaces that stay.
* **§4.8, new**: the nine declared differences of the native geometry
  backend.
* §5.1 "The Windows build, the Windows run and the release" with the AppImage
  table and the four "Not done" items; the new §5.9 "The packaging residuals".
* §5.6/§5.7: the red corpus replay and the `corpus_fixture.rs` failure are
  recorded as **closed**, with the new green harness output.
* §6.1 the commands re-run, with the counts; §6.2 what was not run.

### 3.5 `backend/README.md` and `studio/README.md`

* The member tables: `kala-cad` 118, `kala-studio-journal` 60,
  `kala-cad-providers` 40, `kala-studio` 175, and the root workspace
  **449** (was 423).
* "The Studio service (ADR-0010 phase 2)" replaces "The Studio front door,
  not a Studio service", with the owner table and the AppImage summary.
* `kala-cad` and `kala-studio-journal` get their own "accepted capabilities"
  entries.
* "What is still Python" and "What is pending" are rewritten for the new
  shape: the Studio is cut over, the core product is not; the Windows build,
  the signing, the publication, the Electron window path and the FreeCAD
  3.11–3.13 runtime are listed as open.
* `studio/README.md` gains "The back door is Rust-only" (the packaged
  resolver, the sanitised child environment, the dev default, the bundled
  adapter, the staged OCCT closure) and the staging layout.

### 3.6 ADR-0010

One appended section, `## Phase 2 implemented (2026-10-20)`, with a
**Status** block and a **Consequences** block. The Status row points at it.
The decision, the geometry-worker section, the packaging section and the
original consequences were **not** rewritten.

---

## 4. Commands run in this pass (first-hand evidence)

Every command ran with the USER DATA rule in force (temp `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`,
`RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`).

| Command | Result |
|---|---|
| `cd backend && cargo test --workspace --locked --release` | **449 passed**, 0 failed, 0 ignored (66 test-result lines) |
| `cd backend/crates/kala-cad && cargo test --locked --release` | **118 passed**, 0 failed |
| `cd backend/crates/kala-studio-journal && cargo test --locked --release` | **60 passed**, 0 failed |
| `cd backend/crates/kala-cad-providers && cargo test --locked --release` | **40 passed**, 0 failed |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** (exit 0) |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** (exit 0) |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** (exit 0) |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 0 mismatches**; `exact: 48`, `fields: 4`, `step structural: 2`, `skipped: 2`; corpus digest `53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28` (exit 0) |
| `cd studio && npm run typecheck` | **exit 0** |
| `cd studio && npm test` | **43 files, 628 tests passed** |
| `cd studio && npm run build` | **exit 0** (the pre-existing `three` >500 kB warning) |
| `env -i … node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad` | **PASS** (exit 0) |
| `sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage` | `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e` |
| `ls -l studio/release/kala-studio-0.1.0-x86_64.AppImage` | **163 642 516 bytes** |

### 4.1 The AppImage smoke, verbatim (the release evidence)

```
[appimage-smoke] AppImage: .../studio/release/kala-studio-0.1.0-x86_64.AppImage
[appimage-smoke] size: 163642516 bytes (156.1 MiB)
[appimage-smoke] sha256: b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e
[appimage-smoke] RPATH closure ok: 23 libTK resolutions from .../resources/backend/lib (empty environment, no LD_LIBRARY_PATH)
[appimage-smoke] FreeCAD Python deps ok: ABIs cp311, cp312, cp313, cp314 (20 wheels, 663 files; FreeCAD 1.1.4 Revision: 45040 (Git) / Python 3.14.8 (cp314))
[appimage-smoke] payload ok: metadata schema 3, 23 OCCT libs, FreeCAD adapter + 4 ABI sets, no Python runtime
[appimage-smoke] python-free backend env: PATH="" ... (no repository/virtualenv path)
[appimage-smoke] health ok: {"status":"ok",...,"step_available":true,"design_available":true,...,"geometry_worker":{"state":"ready","version":"7.9.3","design_available":true}}
[appimage-smoke] packaged UI root ok
[appimage-smoke] unauthenticated refusal ok
[appimage-smoke] OpenCascade L-bracket ok: volume 35571.416047113315 mm3, measured_by OpenCascade (opt-in), design_id 1f7de2be6721…
[appimage-smoke] drawing ok: front(7) top(11) right(8)
[appimage-smoke] section ok: XZ@4 area 2321.4601836602546 mm2 (exact), outer 1, inner 1
[appimage-smoke] STEP import ok: 55554 bytes, 1 solid(s), bounds {"min":[-30,0,0],"max":[30,45,40]}
[appimage-smoke] freecad backend env: PATH="/usr/bin:/bin:/usr/local/bin" ... (no repository/virtualenv path)
[appimage-smoke] FreeCAD phase health ok
[appimage-smoke] FreeCAD provider available: version 1.1.4
[appimage-smoke] FreeCAD L-bracket ok: volume 35571.416047 mm3, measured_by FreeCAD, design_id ddea67781a58…
[appimage-smoke] drawing ok: front(7) top(11) right(8)
[appimage-smoke] section ok: XZ@4 area 2321.4601836602546 mm2 (exact), outer 1, inner 1
[appimage-smoke] STEP import ok: 21648 bytes, 1 solid(s), bounds {"min":[-30,-8.881784197001e-16,0],"max":[30,45,40]}
[appimage-smoke] PASS (...AppImage) with FreeCAD provider
```

### 4.2 The per-crate release split of the root workspace

`kala-json` 33, `kala-persistence` 65, `kala-geometry` 32, `kala-budget` 65,
`kala-step` 35 (incl. its doctest), `kala-artifacts` 44, `kala-studio` 175
(of which the new `native_geometry` suite is 10, `corpus_fixture` 2) —
**449**. The old `corpus_fixture.rs` failure (it pinned the pre-regeneration
48-case corpus) is gone: the test now follows the feature that links the
STEP exporter and asserts the regenerated 56-case corpus, and it passes.

---

## 5. Commands **not** run in this pass

| Command | Why |
|---|---|
| `cargo fmt --check` / `cargo clippy --all-targets --all-features -- -D warnings` per crate | Not run. The cell reports stand as filed; the release test counts were re-run. |
| `cargo check --target x86_64-pc-windows-gnu` | Not run. The MinGW C toolchain is absent on this host; the `windows-check` cell has not filed a report. |
| `packaging/windows/build-windows.ps1` | Not run. No Windows, no `pwsh`. **The script has never been executed on any host.** |
| The AppImage **build** (`node scripts/desktop-pack.cjs --linux AppImage`) | Not run. The artifact was built by the packaging cell; this pass verified the existing artifact (hash, size, `env -i` smoke). |
| `npm run check:native-readiness` | Not run. The built stage's `studio/release/native-readiness.json` is quoted instead. |
| The Electron **window** rendering | Not exercised. `xvfb-run` is absent on this host. |
| `backend/compat/self_check.py` | Not run in this pass; the previous pass's result is labelled as such in `backend/README.md`. |
| A real FreeCAD on Python 3.11–3.13 | Not run. The official FreeCAD 1.1.4 AppImage ships `py311`; the `cp311`–`cp313` adapter imports were proven with matching CPython builds, not with a real FreeCAD. |
| Any benchmark | Not run. No speed, memory or readiness number is claimed in any document. |

---

## 6. Compatibility and honesty evidence

* **Every number in the documents was either re-measured in this pass or
  attributed to the cell report it came from.** The two tables that carry
  counts the *other* standalone crates (in `docs/rust-migration-status.md`
  §6.1) are explicitly labelled "not re-run in this pass".
* **No Windows claim.** The Windows build script is described as *the
  procedure*, in the imperative, with a bold "The script has not been run"
  at the top of the section, and with the post-install checklist presented as
  the proof the **user's** machine produces.
* **No Solid Edge claim.** Every mention keeps "no Solid Edge build has ever
  been executed" and adds that the bridge has no verified native build
  executor.
* **No signing or publication claim.** The AppImage is called **unsigned**
  in every document that mentions it, and the release workflow is described
  as written but not pushed or run.
* **No performance claim.** No document states a speed, a memory figure or a
  readiness number. The "no benchmark was run" line is present in
  `docs/rust-studio.md` and `backend/README.md`.
* **The two declared native differences are written down**, not hidden: the
  OpenCascade binding identity (system 7.9.3 vs the `cadquery-ocp` 8.0.1.1
  oracle) and the display-mesh vertex order (compared by an
  order-independent fingerprint). The geometry, the measurements, the checks,
  the drawing and the section are identical, and the documents say so.
* **The packaging residuals are listed** in `docs/rust-migration-status.md`
  §5.9 and in the "Honest limits" of `docs/rust-studio.md`: the unsigned and
  unpublished AppImage, the unrendered Electron window, the unproven real
  FreeCAD 3.11–3.13 runtime, the host-built OCCT closure, the staged (not
  installed) `mod/Kala` addon, and the Windows stage as a definition.
* **The USER DATA rule was obeyed in every shell**, and no document writes
  the user data path. The Studio journal backup is referred to only as "the
  Studio journal was backed up by the conductor on 2026-10-19"; the path
  itself is deliberately **not** reproduced in any file I wrote.

---

## 7. Remaining gaps and honest notes

1. **Two documents are now stale and were not mine to edit.**
   `docs/studio.md` and `docs/studio-limitations.md` still describe the
   pre-cutover shape: the "supervised Python geometry worker" as a product
   layer, `kala-geometry`/`kala-step` as "parked", the "6 Rust-native and 10
   proxied route-methods" split, and the "Release packaging is blocked
   pending the packaging decision" section (which is the anchor
   `studio-limitations.md#release-packaging-is-blocked-pending-a-decision`).
   `docs/studio-providers.md` was not reviewed either. **The conductor
   should assign a follow-up** to bring those three in line; this cell was
   scoped to the six documents it owns plus ADR-0010, and the task says
   "edit only those docs".
2. **`.github/workflows/studio.yml`** still describes the old CI matrix and
   the `nativeStep` marker; the packaging cell added
   `.github/workflows/studio-release.yml` (written, not pushed) as the
   replacement. Not my file to edit.
3. **The Windows build is a definition.** Every Windows statement in the
   documents is a procedure or a limitation, never a result. The user's
   Windows machine is the only place the claim can become a fact.
4. **The MinGW cross-check is still pending** the user's `mingw-w64-gcc`
   install, and the `windows-check` cell has not filed a report.
5. **The Electron window has never been rendered** on this host
   (`xvfb-run` absent). The documents say the packaged **backend** and the
   served UI root are verified by extraction.
6. **The AppImage build itself was not re-run in this pass** — the existing
   artifact was verified (hash, size, the full `env -i` smoke). The
   documents do not claim a fresh build.
7. **A real FreeCAD on Python 3.11–3.13 was not run.** The documents state
   the gap and state that an unsupported Python is reported honestly rather
   than crashing.
8. **The app D1 finding is still open** and is still described as open in
   `docs/rust-migration-status.md` §4.1 and §5.6. This cell did not touch
   it.
9. **No commit, no push, no delegation, no credential read, no sudo** was
   performed, and no code was changed. The documents are the only artefacts.

---

## 8. Reproduce

```sh
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
cd /home/v9i/Projects/Kala-Agent

cd backend && cargo test --workspace --locked --release
cd crates/kala-cad && cargo test --locked --release
cd crates/kala-studio-journal && cargo test --locked --release
cd crates/kala-cad-providers && cargo test --locked --release
cd ../.. && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py
../.venv/bin/python crates/kala-studio/tests/python_differential.py
../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py
../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py

cd ../studio && npm run typecheck && npm test && npm run build
cd .. && sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage

env -i PATH=/usr/bin:/bin HOME=$(mktemp -d) TMPDIR=/tmp \
  XDG_DATA_HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
  KALA_STUDIO_DATA_DIR=$(mktemp -d) \
  /path/to/node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad
```
