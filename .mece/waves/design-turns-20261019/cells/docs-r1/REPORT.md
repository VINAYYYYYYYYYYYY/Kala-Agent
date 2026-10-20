# docs-r1 — user-facing docs brought in line with today's work

Cell: `docs-r1`, wave `design-turns-20261019`.
Specs read: `TASK.md`, ADR-0009, ADR-0010, ADR-0011, every `REPORT.md` in
`cells/*` of `design-turns-20261019` and `rust-backend-20261019`, the
`rust-backend-20261019/TAKEOVER.md`, and the `vellum-ui-20261019` cell
reports.

Status: **done** for the seven owned files. No commit, push, deploy,
delegation or credential read was performed. **No code, ADR or fixture was
edited** — only documentation.

---

## 1. Files changed

| File | Change |
|---|---|
| **`docs/cad-providers.md`** | **NEW.** The ADR-0011 provider model as a user-facing document: the three providers, "who measures what", the probe of each, the FreeCAD 28/28 corpus, the assembly export, the Solid Edge honesty, the OpenCascade opt-in builder and view-only importer, the API, the journal events, the CAD-target selector, and nine stated limitations. |
| `docs/studio.md` | **Rewritten** around what actually exists now: the three-layer shape (Rust front door → supervised Python geometry worker → React UI), the Vellum drafting board (zones, title block, Options drawer's four cards, the two themes, the measure tool, the CAD-target selector), design turns, the journal ledger, the exact conditions for `B-Rep Verified` and `HASH-CHAIN OK`, the AuxView, the API contract (15 Python / 16 front-door route-methods), the Rust front door, `KALA_STUDIO_GEOMETRY_WORKER`, `KALA_STEP_IMPORT_MAX_BYTES`, the token/lifeline model, and an honest "does not claim" list. |
| `docs/studio-providers.md` | Kept as the **language-model** provider document. Added the `cad_provider` field and the `design` field on the response, the local helper's strict design-intent parsing, the FreeCAD subprocess credential-scrub in the trust recap, and the "never auto-selects a CAD provider" rule. Cross-links to `cad-providers.md`. |
| `docs/studio-limitations.md` | New sections: **the CAD providers** (no default, the provider is the only measurer, no live Solid Edge proof, the front door cannot build with FreeCAD/Solid Edge yet), **the geometry viewport** (the clay mode's narrow-FOV **perspective** camera, `SCALE FIT`, the section plane), **the Rust front door and the Python geometry worker**, **the fixed core API bug** (`errors.py`, the 4xx statuses), and **release packaging is blocked pending the decision**. The engineering-safety list gained explicit *no FEA/CFD/solver*, *tolerances are declared, not analysed*, and *a provider's measured volume is not an analysis result*. The "no persistent state" claim was corrected: the studio now keeps its **own** session journal, which is not the kernel's revision store. The FreeCAD and Solid Edge rows were corrected. |
| `docs/studio-install.md` | Now describes **three** ways to run: the (currently blocked) desktop bundle, the **Rust front door with the geometry worker**, and the **Python reference server**; plus the browser fallback, the `studio/` and `backend/` dev commands, the STEP import ceiling, the launch-token length rule, and five new troubleshooting rows. |
| `studio/README.md` | **Rewritten** as the renderer document: the four zones, the component map, the nine honesty invariants, the render modes, the measure tool, the themes, the build outputs, and the honest limitations. |
| `backend/README.md` | **Rewritten** for the current workspace: the seven members with their measured test counts, the fourteen standalone crates with their counts, the front door (not "a nine-route service"), the launch contract, the live differential, **what is still Python**, **what is pending** (tools executor, agent runtime, application/API/CLI, packaging decision, Windows/Solid Edge proof, MinGW), and the toolchain/compatibility/security sections preserved. |

Files I **did not** touch (not owned by this cell): every ADR, every other
doc (`solidedge.md`, `rust-studio.md`, `construction-semantics.md`,
`ui-screens-wireframes.md`, `how-to-read.md`, `student-guide.md`,
`threat-model.md`, `verification-report.md`, `architecture-report.md`,
`review1.md`), the root `README.md`, and all code, fixtures and tests.

---

## 2. Every documented command: run or explicitly marked

Every command I wrote into a document was executed on this host
(Linux, `.venv` CPython 3.14.8, `cargo`/`rustc` 1.99.1, node 26.8.2,
npm 11.19.1, FreeCAD 1.1.4, OCP 8.0.1.1).

### Run and green

| Command | Where documented | Result |
|---|---|---|
| `cd studio && npm run typecheck` | studio.md, studio/README.md, studio-install.md, studio-limitations.md | clean, exit 0 |
| `cd studio && npm test` | studio.md, studio/README.md, studio-install.md, studio-limitations.md | **43 files, 614 tests passed** on the final re-run (the first run of this cell, before a concurrent worker added a suite, reported 42 files / 592 tests) |
| `cd studio && npm run build` | studio/README.md, studio-install.md | green (`✓ built in 430ms`); only the pre-existing `three` >500 kB warning |
| `cd studio && npm run check:native-readiness` | studio.md, studio-install.md, studio-limitations.md, backend/README.md | READY; "covers all 15 current Python route-methods and DTO capabilities (6 Rust-native, 10 proxied)"; 4 worker-only routes; 0 missing |
| `.venv/bin/python -m pytest tests/unit -q` | studio-limitations.md | **1552 passed**, 0 failed |
| `.venv/bin/python -m pytest tests/security -q` | studio-limitations.md | **189 passed**, 0 failed |
| `.venv/bin/python -m pytest tests/unit/studio_design/test_cad_providers_freecad.py -q` | cad-providers.md, studio-limitations.md | **8 passed in 19.10 s** (the real FreeCAD corpus) |
| `cd backend && cargo build --release -p kala-studio --locked` | studio.md, studio-install.md, studio-limitations.md, backend/README.md | pass |
| `cd backend && cargo test --workspace --locked --release` | backend/README.md, studio-limitations.md | **414 passed, 0 failed, 0 ignored** (identical in debug) |
| `cd backend && cargo test -p kala-studio --locked --release --all-features` | backend/README.md | **160 passed** (the default build is 144) |
| `cd backend && cargo test --workspace --locked --release` for each standalone crate | backend/README.md | kala-api 58, kala-contracts 50, kala-process 46, kala-kernel 152, kala-memory 85, kala-tools 192, kala-sandbox 60, kala-design-intent 54, kala-domains 29, kala-repositories 33, kala-solidedge-client 79, kala-cad-providers 37, kala-cad 64 — all 0 failed |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | backend/README.md, studio-limitations.md | **differential cases: 289 / mismatches: 0 / known gaps: 0** |
| `.../python_differential.py` | backend/README.md | **268 cases, 0 mismatches** |
| `.../replay_binary_corpus.py` | backend/README.md | **48 cases, 0 mismatches** |
| `.../python_provider_dto_oracle.py` | backend/README.md | **20 cases, 0 failures** |
| `.venv/bin/python packaging/studio_smoke.py --backend dist/kala-studio` | studio-install.md, studio-limitations.md | **PASS** (health ok, UI root ok, preview ok, STEP 15 518 bytes with the ISO-10303-21 magic) |
| The Rust front door launch command in studio.md / studio-install.md / studio-limitations.md | all three | **run**: `KALA_STUDIO_TOKEN=devtoken-0123456789abcdef` + `KALA_STUDIO_GEOMETRY_WORKER="[\"$PWD/.venv/bin/python\",\"-m\",\"kala_agent.studio\",\"--worker\"]"` + `backend/target/release/kala-studio --host 127.0.0.1 --port 8794 --data-dir … --static-dist studio/dist` → `geometry_worker {state: "ready", version: "0.1.0", design_available: true}` and the three-provider probe answered |
| The Python reference server launch command | studio.md, studio-install.md | **run**: `PYTHONPATH=src .venv/bin/python -m kala_agent.studio --host 127.0.0.1 --port 8795 …` → `status ok`, `design_available true`, `step_available true`, and the same three-provider probe |
| `python -m kala_agent.studio --worker` (the readiness line) | studio.md | **run**: `KALA_WORKER_READY {"design_available":true,"port":44005,"version":"0.1.0"}` |
| The `curl` checks in studio.md / studio-install.md / studio-limitations.md | all three | **run** against both the front door and the reference server |

### Not run (and marked as such where they appear)

| Command | Why |
|---|---|
| `pip install -e ".[studio]"` / `".[geometry]"` | A network install; the venv already has both. The docs present it as the **from a clean checkout** step and it is not claimed as verified here. |
| `npm ci` | The dependencies are already installed in `studio/node_modules`; `npm ci` would re-resolve the network. The `npm run typecheck` / `npm test` / `npm run build` gates were run against the installed tree. |
| `npm run desktop`, `npm run desktop:pack` | Need a desktop session and the electron-builder toolchain; the **release packaging is blocked** anyway (see §5). |
| `cargo check -p kala-studio --target x86_64-pc-windows-gnu` | **Blocked on this host**: `ring`'s build script needs `x86_64-w64-mingw32-gcc`, which is not installed. The docs state the blocker and make **no** Windows compile or runtime claim. |
| The Windows/MSVC cross-check block | Same reason (`lib.exe` missing), as recorded before this cell. |
| The Solid Edge build | **Not possible here** and never claimed: it needs a Windows host with Solid Edge. The docs state the honest refusal. |
| The desktop-bundle install steps | The release bundle is blocked pending the packaging decision; the docs say so at the top of the section. |
| `rust-e2e-qa` | That cell has **no REPORT.md** (only a TASK.md), so there was nothing to cite; its flows were not re-run. |

---

## 3. Facts I verified live (not taken from a report)

| Claim in the docs | How I checked it |
|---|---|
| FreeCAD `1.1.4`, OpenCascade `8.0.1.1`, Solid Edge unavailable with the exact reason | `GET /api/studio/providers/cad` on both the front door and the reference server |
| FreeCAD measures the L-bracket as `35571.416047 mm³` / `11889.101731 mm²`, `solid_count 1`, an `fcstd` native artefact, and a view of 586 vertices / 568 triangles | a live `POST /api/studio/chat` with `cad_provider: "freecad"` against the reference server |
| OpenCascade measures `35571.416047113315 mm³` with bounds `[-30,0,0]…[30,45,40]`, `verified: true` | a live chat turn with `cad_provider: "opencascade"` **through the Rust front door** |
| The front door **refuses** `freecad` rather than falling back | the same live turn answered *"The freecad CAD provider is not available in this studio build. No design was built."* |
| A chat turn with **no** `cad_provider` asks and builds nothing | the live turn answered *"Choose a CAD provider before I build this design: … No design was built."* with `design: null` |
| The journal verifies | `GET /api/studio/journal/verify` → `{"ok": true, "event_count": 19, "head_digest": "…", "first_bad_sequence": null}` |
| The XZ section at offset 4 measures `2321.4601836602546 mm²` with 1 outer and 1 inner loop | a live `GET /api/studio/design/{id}/section?plane=XZ&offset=4` |
| The HLR drawing returns front/top/right/iso with visible **and** hidden polylines | a live `GET /api/studio/design/{id}/drawing?views=front,top,right,iso` |
| The launch token must be 16–128 characters | a deliberate short token produced *"the studio token must be between 16 and 128 characters"*; the docs now use a 24-character token |
| The route split (6 native / 10 proxied / 15 Python) | `npm run check:native-readiness` |
| The worker readiness line format | `python -m kala_agent.studio --worker` |
| 14 worker routes under `/internal/v1` | counted from `src/kala_agent/studio/internal_api.py` |

---

## 4. Numbers that came from the cell reports (attributed, not re-run)

These are stated in the docs as the **cell's** recorded evidence, not as
something I re-measured:

* the FreeCAD **28/28** corpus result and the per-case assembly volumes
  (B3's report; the test file itself was re-run and is green);
* the **591-case** `kala-api` live differential and the **486/486**
  `kala-contracts` differential (the cell reports);
* the **144 / 160** `kala-studio` counts (both re-measured by me);
* the `cad-r1` interruption and the `kala-cad` parking (ADR-0010 and the
  TAKEOVER).

---

## 5. Honest notes about this docs pass

1. **`kala-runtime` is not accepted and its suite was not green when I
   measured it.** 1 of its 2 tests failed
   (`the_scenario_covers_every_gateway_path`); the live differential test
   passed. The crate's files had modification times of 02:02–02:07 while I
   was measuring, so it was being edited concurrently. I recorded this in
   `backend/README.md` rather than hiding it, and I did **not** edit the
   crate.
2. **`docs/rust-studio.md` is now out of date** and I did not touch it (it
   is not in this cell's ownership). It still describes the old nine-route
   native surface. `backend/README.md` and `docs/studio.md` now carry the
   current description; the next owner of `rust-studio.md` should align it.
3. **The studio README no longer carries the old "Basic AI-generated UI for
   test" note** as its headline. The UI is a real four-zone drafting
   workspace now; keeping that banner would have been the more misleading
   choice. The honest limitations are all still listed.
4. **`docs/studio-limitations.md` says the studio "does not save revisions,
   mark approvals"** — that is still true of the *core*; the studio's own
   session journal is new and is described separately, with the explicit
   statement that it is a session log and not a revision system.
5. **The perspective camera note** is in `docs/studio-limitations.md` and
   `studio/README.md`, not in `docs/studio.md`'s render-mode section, where
   it would have been buried. `docs/studio.md` links to the limitations file.
6. **No ADR was edited.** ADR-0009/0010/0011 keep `Status: **Proposed**`.
7. I did **not** take a screenshot or drive the UI. The UI claims in the
   docs come from the Vellum wave's QA reports and the D1/C2 reports, and
   from the component-level source I read. The **numbers** in the docs are
   the ones I measured live.
8. **Nothing was committed.** The working tree carries the six modified
   files and one new file, as listed in §1.

---

## 6. No overclaims checklist

Each of these was checked explicitly against the source or a live run:

* [x] No FEA / no solver / no fabricated stress, displacement, factor of
      safety or fatigue life — stated in `studio-limitations.md`.
* [x] Tolerances are **declared, not analysed** — stated in
      `studio.md`, `studio-limitations.md` and `cad-providers.md`.
* [x] The clay viewport is a **perspective** camera, not orthographic —
      stated in `studio-limitations.md` and `studio/README.md`.
* [x] Release packaging is **blocked pending the packaging decision** —
      stated in `studio.md`, `studio-limitations.md`, `studio-install.md`
      and `backend/README.md`.
* [x] The core API bug (`errors.py`, the 4xx statuses) is described as
      **fixed**, with the before/after table and the residual policy
      difference — stated in `studio-limitations.md`.
* [x] **There is no default CAD provider** — stated in `studio.md`,
      `cad-providers.md`, `studio-limitations.md`, `studio/README.md`.
* [x] **Solid Edge has no live proof** — stated everywhere it appears.
* [x] The Rust front door **cannot build** with FreeCAD or Solid Edge —
      stated in `cad-providers.md` and `studio-limitations.md`.
* [x] The Rust front door is **not** in production; the Python reference
      is still the oracle — stated in `studio.md`, `studio-limitations.md`
      and `backend/README.md`.
* [x] `kala-geometry` / `kala-step` / `kala-cad` are **parked**, not
      deleted — stated in `studio.md` and `backend/README.md`.
* [x] Python is **not deleted** and no cutover was performed.
* [x] No Windows runtime, COM, installer or speed claim is made.
* [x] The 15 Python / 16 front-door route-method counts are counted from
      the real sources, not asserted.
