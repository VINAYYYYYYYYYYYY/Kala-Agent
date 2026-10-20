# rust-e2e-qa — the real Studio UI through the Rust front door (READ-ONLY QA)

Cell: `rust-e2e-qa`, wave `rust-backend-20261019`. Date: 2026-10-20.
Reviewed before starting: `../r2-studio-proxy/REPORT.md`,
`../r2-followup/REPORT.md`, ADR-0010, ADR-0011 and
`../../design-turns-20261019/cells/d1-qa/REPORT.md` (the headless-Chromium
harness approach, without the D1 provider/mesh shims).

**Status: FAIL on the required FreeCAD chat flow. PASS on the OpenCascade
flow, the two-part place-only assembly, the worker crash/restart and the
Python public-server differential.**

The failure is narrow and reproducible: the Rust front door's CAD target
selector advertises FreeCAD 1.1.4 as available, its own `/providers/cad`
route says `available: true`, and its no-provider reply lists FreeCAD as
available on this host — but a chat turn with `cad_provider: "freecad"`
answers with "The freecad CAD provider is not available in this studio
build. No design was built." The same worker builds the identical L-bracket
through `POST /api/studio/design` with `provider: "freecad"` (200, volume
35571.416047 mm³, 1 valid solid, verified), and the Python public server's
chat builds it normally. The defect is the chat branch in
`backend/crates/kala-studio/src/chat_design.rs`, not the worker, the CAD
provider model or the proxy.

No source file was edited by this cell. Only `/tmp/opencode/rust-e2e-qa/**`,
this cell's `shots/` folder, this `REPORT.md` and the standard `cargo` build
output under `backend/target/release/` were written. No commit, push, deploy,
delegation or credential read was performed. The backend used a throwaway,
non-secret `KALA_STUDIO_TOKEN` supplied in the launch environment.

---

## 1. Artifacts under test and exact commands

### 1.1 Studio UI: one build, into a temp copy

`studio/` was snapshotted at **2026-10-20 01:50 local** (2026-10-19T20:20:37Z),
git HEAD `966c185`, including the concurrent C3 working-tree edits that were
present at that moment. The snapshot was copied to
`/tmp/opencode/rust-e2e-qa/studio-src/` (node_modules symlinked), and the
dist was built **once**:

```text
cd /tmp/opencode/rust-e2e-qa/studio-src
npx tsc -p tsconfig.json --noEmit          # exit 0
npx vite build --outDir /tmp/opencode/rust-e2e-qa/dist --emptyOutDir
```

Result: green, `index-V9TXNjJF.js` / `index-DaO8aep4.css`, only the
pre-existing `three.js` >500 kB chunk warning. **No rebuild** was performed
after the snapshot, per the task; this dist served both front doors. If C3
changed `studio/src` after 01:50, those changes are not in this test.

### 1.2 Rust release front door

```text
cd /home/v9i/Projects/Kala-Agent/backend
cargo build --release -p kala-studio
```

`cargo` reported the release build up to date; the binary is
`backend/target/release/kala-studio`, SHA-256
`62cef0f51d8315f1b0b5935c70c2a0ca61cce9a1d192b6ae8ccd9b0223d2199d`,
7,282,416 bytes. The primary Rust run below used a process whose
`/proc/<pid>/exe` hash was verified equal to that file. Toolchain:
rustc/cargo 1.99.1.

Primary run (port 8877, fresh data dir; exact command line):

```text
KALA_STUDIO_TOKEN=<throwaway> \
KALA_STUDIO_GEOMETRY_WORKER=/home/v9i/Projects/Kala-Agent/.venv/bin/python \
backend/target/release/kala-studio \
  --port 8877 \
  --data-dir /tmp/opencode/rust-e2e-qa/data-rust-current \
  --static-dist /tmp/opencode/rust-e2e-qa/dist
```

The front door spawned and supervised the geometry worker itself; the worker
process command line was exactly:

```text
/home/v9i/Projects/Kala-Agent/.venv/bin/python -m kala_agent.studio --worker
```

`GET /api/studio/health` reported
`geometry_worker: {state: "ready", version: "0.1.0", design_available: true}`.
The worker receives a fresh 64-hex token only through `KALA_WORKER_TOKEN`,
the stdin lifeline and the resolved `KALA_STEP_IMPORT_MAX_BYTES`; none of
those values is printed here. Other Rust processes (8871/8873/8875/8878)
were used only for the clean-crash, card-close-up and direct-proxy evidence;
they used the same dist and a release build of the same source tree (the
primary 8877 and the card close-up 8878 ran the exact binary hash above).

### 1.3 Python public server (reference)

```text
KALA_STUDIO_TOKEN=<throwaway> .venv/bin/python -m kala_agent.studio \
  --port 8872 \
  --data-dir /tmp/opencode/rust-e2e-qa/data-py \
  --static-dist /tmp/opencode/rust-e2e-qa/dist
```

The 8872 session is the primary Python reference (a separate data dir);
several designs were already in that server's cache from the initial API
probes, which is why its OCP chat reports `cached: true`. The visible values
are the same; the card, HLR, section, ledger and export were driven through
the browser UI.

### 1.4 Browser

Playwright 1.64.1 driving Chrome for Testing **153.0.8010.12** headless at
**1440×900**, `--use-gl=angle --use-angle=swiftshader`. The plain-Chromium
run takes the shipping **browser fallback** boot gate (origin + bearer token
form), not the Electron bridge; this is the normal browser path in the same
UI build.

Host: Linux. Python 3.14.8, OCP 8.0.1.1, FreeCAD 1.1.4 (revision 45040).
Node 26.8.2.

---

## 2. Pass/fail checklist

| # | Required check | Rust front door | Python reference | Verdict |
|---|---|---|---|---|
| 1 | release `kala-studio` + supervised worker + temp dist copy | health 200, worker `ready`, worker `/proc` child of the front door | n/a (public server) | **PASS** |
| 2 | headless Chromium 1440×900, connect | browser fallback form → `.studio-shell`, providers loaded | same | **PASS** |
| 3 | select FreeCAD + send the Stitch L-bracket sentence | selection succeeded (`aria-checked=true`), turn sent | same | **PASS (interaction)** |
| 4 | FreeCAD card says `measured by FreeCAD 1.1.4` | **no card**; 428-byte refusal reply, `design: null` | card present, `measured by FreeCAD 1.1.4` | **FAIL (Rust)** |
| 5 | FreeCAD volume `35571.42 mm³` | absent (no design) | `35571.42 mm³` | **FAIL (Rust)** |
| 6 | FreeCAD `B-Rep Verified` | absent | stamp present | **FAIL (Rust)** |
| 7 | FreeCAD HLR drawing with hidden lines | absent | 4 views, hidden dashed polylines | **FAIL (Rust)** |
| 8 | FreeCAD Section `2,321.46 mm²` | absent | `AREA 2,321.46 mm²`, XZ at 4 mm | **FAIL (Rust)** |
| 9 | ledger with server rows + `HASH-CHAIN OK` | `HASH-CHAIN OK`; 12 server rows (seq 1–12) + local rows | `HASH-CHAIN OK`; 39 server rows | **PASS** |
| 10 | STEP export with sha256 | OpenCascade export 55,554 bytes; receipt digest = the saved file's sha256 | FreeCAD 21,648 / OpenCascade 55,554; receipt matches file | **PASS** (OpenCascade); FreeCAD export reached through the API, not chat |
| 11 | repeat with OpenCascade (opt-in) | full pass (see §4) | full pass | **PASS** |
| 12 | Solid Edge disabled with its reason | disabled, reason in title | same | **PASS** |
| 13 | two-part place-only assembly, `solid_count 2`, verified | 200, 2 components, `fused: false`, `solid_count: 2`, `verified: true` | identical semantics | **PASS** |
| 14 | kill the worker: 503-style UI state, chat/settings alive, restart, design works | 503 body + red UI error; settings 200; non-design chat 200; restart 1.285 s; export and a new design succeed | n/a | **PASS** |
| 15 | run the same flows against the Python public server | n/a | FreeCAD + OpenCascade + assembly | **PASS** |
| 16 | diff visible results and network payloads | done; one functional gap + generated-digest/timestamp differences | done | **PASS (documented)** |

The FreeCAD criteria are the only functional failures. They are not
cosmetic: the task's first required screen (FreeCAD card) cannot be produced
through the Rust front door's chat.

---

## 3. The FreeCAD failure in detail (blocker)

### 3.1 What the UI does

1. Connect through the boot gate; the CAD target selector renders three
   rows from `GET /api/studio/providers/cad`:
   - `OpenCascade (opt-in) 8.0.1.1` — available;
   - `FreeCAD 1.1.4` — available;
   - `Solid Edge` — **disabled**, title
     `Solid Edge is unavailable: Solid Edge build requires the Windows bridge
     (not reachable); this host is linux, so the ADR-0008 bridge cannot run
     here`.
2. Click FreeCAD; its button becomes `aria-checked="true"`.
3. Send: *"Design a machined aluminum L-bracket for a 40 mm mounting plate,
   8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting
   holes."*
4. The assistant reply (HTTP **200**, 428 bytes) ends:

   > The freecad CAD provider is not available in this studio build. No
   > design was built.

   `design` is `null`; no agent card, no HLR, no section, no export for that
   turn. The viewport keeps the nominal BOX estimate.

Screenshot: `shots/02-rust-freecad-selected-refused.png`.

### 3.2 The front door contradicts its own provider facts

- `GET /api/studio/providers/cad` → 200, 542 bytes, identical to Python:
  `{"id":"freecad","name":"FreeCAD","available":true,"version":"1.1.4",...}`.
- A turn with no `cad_provider` answers:
  `… Choose a CAD provider before I build this design: opencascade (opt-in),
  freecad (external), solidedge (bridge). **Available on this host:
  opencascade, freecad**. The studio never selects a provider automatically.
  No design was built.`
  (HTTP 200, 573 bytes; saved in
  `/tmp/opencode/rust-e2e-qa/logs/chat-no-provider.json`).
- The UI toolbar/footer also prints `measured by OpenCascade (opt-in) 8.0.1.1`
  after the OpenCascade turn, and the CAD selector prints `FreeCAD 1.1.4`.

So the operator is told FreeCAD is available, selects it, and is then told
it is not available. This is a self-contradiction in the front door.

### 3.3 The cause (code, not a worker failure)

`backend/crates/kala-studio/src/chat_design.rs`, `finish_design` (lines
720–737):

```rust
let Some(cad_provider) = cad_provider else { /* ask the operator */ return Ok(()); };
if cad_provider != CAD_PROVIDER_OPENCASCADE {
    turn.text = join_text(&[&turn.text, &provider_unavailable_message(cad_provider)]);
    return Ok(());
}
let outcome = compile_design(backend, intent, cad_provider).await ...
```

`available_cad_providers` (same file, lines 339–398) already fetches the
worker's live rows and knows FreeCAD is available; `finish_design` never
consults that list and hard-codes OpenCascade as the only chat-buildable
provider. The test
`backend/crates/kala-studio/tests/chat_with_design.rs:248–266` pins the
refusal (`"The freecad CAD provider is not available in this studio build"`),
so this is a deliberate limitation of the current Rust cell — and
`r2-followup/REPORT.md` §9.2 lists it as a known gap ("The front door still
cannot drive FreeCAD"). It nevertheless blocks the end-to-end acceptance in
this task.

### 3.4 Proof that the worker, the proxy and the provider model are fine

Direct `POST /api/studio/design` on the **current** Rust binary (8877) with
`provider: "freecad"` and the L-bracket script:

```text
HTTP 200, 55,686 bytes
design_id a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7
provider freecad, provider_version 1.1.4, measured_by FreeCAD
measured.volume_mm3 35571.416047, surface_area_mm2 11889.101731,
bounds 60 × 45 × 40, solid_count 1, valid true, verified true
artifacts: STEP + FCStd (timestamps make the digests run-specific)
```

The Rust proxy then serves that FreeCAD design's views and export:

| Route through Rust | Result |
|---|---|
| `GET /design/a9d04…/drawing` | 200, 40,343 bytes; **byte-identical to Python**; hidden polylines 22/18/32/40 for front/top/right/iso |
| `GET /design/a9d04…/section?plane=XZ&offset=4` | 200, 2,843 bytes; **byte-identical to Python**; area 2321.460184 mm², 1 outer / 1 inner loop |
| `POST /export` `{design_id, format:"step"}` | 200, 21,648 bytes; `x-studio-digest` equals the design's STEP sha; same size as Python (digest differs only by the STEP header timestamp) |

The Python public server's chat, the Rust `/design` route and the worker all
build the identical FreeCAD part. Only `POST /chat` with
`cad_provider:"freecad"` is refused.

### 3.5 What the Python reference shows for the same sentence

`shots/21-python-freecad-card.png` and `shots/21b-python-freecad-card-closeup.png`:

- provider text **`measured by FreeCAD 1.1.4`**;
- `volume 35571.42 mm³`, `area 11889.10 mm²`,
  envelope `60.00 × 45.00 × 40.00 mm`;
- **`B-Rep Verified`**; 5/5 checks passed (`brep_valid`, `single_solid`,
  `envelope_x/y/z`);
- digests `script 88a7ec13`, `step 3a32209c`, `FCStd 8920ebf4` (the 8872 run);
- HLR: 4 views, `2B VISIBLE / 2H HIDDEN`, caption
  `view: OpenCascade import (view-only)`;
- Section A–A: `AREA 2,321.46 mm²`, `OFFSET 4.00 mm`,
  `1 OUTER · 1 INNER LOOP`, `KERNEL AREA`;
- ledger: `HASH-CHAIN OK`, 39 server rows + local rows;
- FreeCAD STEP export in the first Python pass: 21,648 bytes,
  sha256 `3a32209c…`, matching the card's step chip
  (`shots/python-freecad-export.step`).

### 3.6 Suggested fix (not applied — read-only QA)

Make `finish_design` compile any provider the front door can serve, using
the same `available_cad_providers` answer, or simply pass the selected
`cad_provider` to `compile_design` and let the worker's provider service
decide (the worker already refuses unknown/unavailable providers). Keep the
ADR-0011 rule that nothing is auto-selected. Add a chat test that
`cad_provider: "freecad"` yields a design through the Rust front door, not
just through `/design`. No source was changed here.

---

## 4. OpenCascade flow (pass)

After the FreeCAD turn, the same session selected
`OpenCascade (opt-in) 8.0.1.1` and sent the same sentence.

| Screen check | Evidence (Rust current run) |
|---|---|
| Provider | `measured by OpenCascade (opt-in) 8.0.1.1` |
| Volume | `35571.42 mm³` (API 35571.416047113315) |
| Area | `11889.10 mm²` |
| Envelope | `60.00 × 45.00 × 40.00 mm` |
| Stamp | **`B-Rep Verified`** |
| Checks | 5/5 passed: B-Rep valid, Single solid, Envelope X/Y/Z |
| HLR | `.hlr-drawing`, 4 view groups (iso/top/front/right), 1 visible + 1 hidden path per view, 4 hidden paths total; hidden stroke `5px, 3px`; heading `2B VISIBLE / 2H HIDDEN`; no canvas in pencil mode |
| Section A–A | `AREA 2,321.46 mm²`, `OFFSET 4.00 mm`, `1 OUTER · 1 INNER LOOP`, `KERNEL AREA`; SVG aria-label `Section XZ at offset 4.00 millimetres, area 2,321.46 square millimetres` |
| 3D clay | live canvas; bottom bar `measured by OpenCascade (opt-in) 8.0.1.1` |
| Ledger | `HASH-CHAIN OK`; 12 server rows (seq 1–12) with digests + chain, plus local rows; no server rows missing in this snapshot |
| STEP export | `kala-studio-594f408b3223.step`, **55,554 bytes**, receipt sha256 `f22a8fc3…`; downloading the file and hashing it gives the same sha256; the card's `step` chip is the same digest |
| Options drawer | all four cards load; active-design export card shows the design, script/step chips and the export action |

The OCP design id in both front doors is
`594f408b3223505c51bfdef61cfb935fc28f8ec461aa62fefc62c9b731932c92`; the
API values are 35571.416047113315 mm³ and 11889.101731097317 mm².

Screenshots: `03-rust-opencascade-card.png` (card, scrolled),
`03b-rust-opencascade-card-closeup.png` (card header/stamp/measurements on a
second fresh process), `04-rust-hlr-4views-hidden.png` (HLR crop),
`05-rust-section-2321.46.png`, `06-rust-clay3d.png`,
`07-rust-ledger-hash-chain-ok.png`, `08-rust-options.png`,
`09-rust-export-receipt.png`.

Note on `03b`: it was captured on a second fresh front door (same binary and
dist) so the card header is not scrolled out of the chat viewport. Its step
digest is run-specific; the values match the primary 8877 evidence.

---

## 5. Two-part place-only assembly (pass)

Request: the corpus case `assembly_two_parts_placed_once` from
`tests/fixtures/construction_parity.json` — two 10×10×5 pads placed at
(0,0,0) and (20,0,0) — sent as `POST /api/studio/design` with
`provider: "freecad"` and the script (no `fuse` step). Files:
`shots/assembly-request.json`, `shots/assembly-rust.json`,
`shots/assembly-python.json`.

| Field | Rust (current 8877) | Python (8874) |
|---|---|---|
| HTTP | 200 | 200 |
| design_id | `c60029cd22ce…` | `c60029cd22ce…` |
| components / fused | 2 / **false** | 2 / **false** |
| measured.solid_count | **2** | **2** |
| measured.volume_mm3 | 1000.0 | 1000.0 |
| measured.surface_area_mm2 | 800.0 | 800.0 |
| valid / verified | true / **true** | true / **true** |
| checks | `brep_valid`, `solid_count` both passed | same |

Both responses are 3,556 bytes; all semantic fields match. The STEP and
FCStd digests differ per build because the files embed timestamps; the
geometry and the measured values are identical. This is the ADR-0011
place-only two-occurrence assembly with two solids, not a fused single
solid.

---

## 6. Worker crash / restart (pass)

The worker child of the primary 8877 front door was killed with `SIGKILL`
mid-session; the supervisor was not touched.

| Observation | Evidence |
|---|---|
| Killed / new worker PID | 1428605 → 1434855 |
| Geometry route while down | `GET /api/studio/journal` → **503**, body `{code: "geometry_worker_unavailable", message: "the geometry worker is not available; the studio cannot compute geometry without it", details: ["the geometry worker is unreachable: error sending request for url (http://127.0.0.1:<ephemeral>/internal/v1/journal)"]}` |
| Health while down | `geometry_worker.state = "restarting"`, `version = null`, `design_available = false` |
| UI state | Options → Export design → red `geometry_worker_unavailable: … — the geometry worker is not running` (`10-rust-worker-down-export-503.png`) |
| Settings while down | `GET /api/studio/settings` → 200; Options cards 03/04 still render (provider status, connection, token-in-memory note) |
| Chat while down | a non-design prompt (`Give me the nominal envelope of the current part.`) → 200, normal local-helper reply, no design (`11-rust-worker-down-chat-works.png`) |
| Restart | health back to `ready` in **1,285 ms**; new PID; `design_available: true` |
| Export after restart | same design, 55,554 bytes, receipt sha `f22a8fc3…`, matches the saved bytes (`12-rust-after-restart-export.png`) |
| Design after restart | new OpenCascade chat → 200, card, volume `35571.42 mm³`, `B-Rep Verified`, same design id (`13-rust-after-restart-design.png`) |

The one console error in the whole Rust run is the expected
`Failed to load resource: the server responded with a status of 503
(Service Unavailable)` produced by the deliberate outage; there are no page
errors and no failed requests.

**Shared post-restart caveat:** after the restart, the restored design's
`measured.surface_area_mm2` is `null`, so the card's area row shows `—`
(volume, envelope and all checks remain). This is a pre-existing Python
worker bug, not a Rust front-door regression; see §7.4.

---

## 7. Python reference and Rust-vs-Python differential

The Python public server ran the same UI flows (same dist, same browser)
against `http://127.0.0.1:8872` and produced the FreeCAD and OpenCascade
cards, HLR, section, ledger and export. The machine-readable diff is
`shots/diff-summary.json`; the raw captures are
`shots/rust-evidence.json` and `shots/python-evidence.json`.

### 7.1 Visible result diff

| Surface | Rust | Python | Equal? |
|---|---|---|---|
| CAD target rows (order, labels, versions) | OpenCascade 8.0.1.1, FreeCAD 1.1.4, Solid Edge unavailable + reason | identical | **yes** |
| Solid Edge disabled + reason | disabled; Windows-bridge reason | identical | **yes** |
| No CAD target default | hint "No CAD target selected…" | identical | **yes** |
| FreeCAD card | **none; refusal reply** | `measured by FreeCAD 1.1.4`, 35571.42 mm³, B-Rep Verified, 5/5 checks, HLR, section, ledger, STEP | **NO — blocker** |
| OpenCascade card | provider, volume, area, envelope, stamp, 5 checks | identical | **yes** |
| HLR (OCP design 594f408b) | 4 views, 4 hidden paths, dashed | identical | **yes** |
| Section (OCP, XZ 4 mm) | `AREA 2,321.46 mm²`, 1 outer / 1 inner | identical | **yes** |
| Ledger stamp | `HASH-CHAIN OK` | `HASH-CHAIN OK` | **yes** |
| Ledger server rows | 12 (seq 1–12) + local | 39 + local | both present; counts differ by session |
| Export receipt | 55,554 bytes, sha `f22a8fc3…`, file hash matches | 55,554 bytes, sha `8f982c41…`, file hash matches | value semantics equal; digest run-specific (timestamp) |

### 7.2 Network payload diff (status + JSON)

The table is from `shots/diff-summary.json`; "equal" means after
normalising generated digests, the session journal head and the design-cache
flag where stated.

| Route | Rust status / bytes | Python status / bytes | Equal? |
|---|---|---|---|
| `GET /health` | 200 / 411 | 200 / 333 | yes after removing the Rust-only `geometry_worker` supervisor block (ADR-0010, by design) |
| `GET /integrations` | 200 / 4,419 | 200 / 4,419 | **byte-identical** |
| `GET /providers` | 200 / 1,133 | 200 / 1,133 | **byte-identical** |
| `GET /providers/cad` | 200 / 542 | 200 / 542 | **byte-identical** (FreeCAD 1.1.4 available; Solid Edge unavailable + reason) |
| `GET /settings` | 200 / 1,340 | 200 / 1,340 | **byte-identical** |
| `POST /chat` (FreeCAD) | 200 / **428**, no design | 200 / **56,861**, design | **NO** |
| `POST /chat` (OpenCascade) | 200 / 53,174, design | 200 / 53,173, design | **yes** after normalising the generated STEP digest, journal head and cache flag; reply text and every measured/view value are identical (`cached` differs because Python had the design in memory from an earlier call; the Rust run built cold) |
| `POST /design` FreeCAD L-bracket | 200 / 55,686 | 200 / 55,686 | yes: same id, provider, version, volume 35571.416047, area 11889.101731, solid 1, verified; STEP/FCStd digests and journal head are run-specific |
| `POST /design` place-only assembly | 200 / 3,556 | 200 / 3,556 | yes: 2 components, not fused, solid_count 2, verified, 1000 mm³, 800 mm²; artifact digests run-specific |
| `GET /design/594f…/drawing` | 200 / 38,120 | 200 / 38,120 | **byte-identical** |
| `GET /design/594f…/section` | 200 / 2,829 | 200 / 2,829 | **byte-identical** |
| `GET /design/a9d04…/drawing` (FreeCAD, direct proxy) | 200 / 40,343 | 200 / 40,343 | **byte-identical** |
| `GET /design/a9d04…/section` (FreeCAD, direct proxy) | 200 / 2,843 | 200 / 2,843 | **byte-identical** (area 2321.460184, XZ 4 mm, 1 outer/1 inner) |
| `POST /export` (OCP STEP) | 200 / 55,554, sha `f22a8fc3…` | 200 / 55,554, sha `8f982c41…` | same size/model; sha differs only by the embedded STEP timestamp |
| `POST /export` (FreeCAD STEP) | 200 / 21,648, sha `0f412daf…` | 200 / 21,648, sha `cef2a3bd…` | same size/model; sha differs only by the embedded timestamp |
| `GET /journal/verify` | 200 / 135, verdict OK | 200 / 135, verdict OK | both OK; `event_count`/`head_digest` differ (different session journals) |

### 7.3 Differences that are not front-door defects

1. **STEP/FCStd digests are run-specific.** The OCP STEP writer (and the
   FreeCAD writer) embeds the export/build time in `FILE_NAME`; two builds
   of the identical model produce the same size and geometry with different
   sha256. Example: OCP 55,554 bytes, `f22a8fc3…` (Rust current) vs
   `8f982c41…` (Python 8872); FreeCAD 21,648 bytes, `0f412daf…` vs
   `cef2a3bd…`. The UI receipt always equals the bytes the browser saved
   (verified by hashing the downloads); the artifact chip equals the design's
   reported digest. Do not use a cross-process STEP sha as a parity oracle.
2. **`cached`** differs when one side has the design in memory from an
   earlier call and the other builds cold. It is a cache-state field, not a
   computed value.
3. **`journal_head` / `event_count`** differ because each front door has its
   own session journal; both chains verify.
4. **`geometry_worker`** appears only in the Rust `/health` document, by
   ADR-0010 design.

### 7.4 Shared pre-existing bug: surface area lost on journal restore

After the worker restart, the Rust card's area row shows `—`. I reproduced the
same on the Python public server by stopping and restarting it against the
same data dir and re-requesting design `594f408b…`: the response is 51,989
bytes with `measured.surface_area_mm2: null` while the in-memory response
before the restart had `11889.101731097317`. The cause is in
`src/kala_agent/studio/design/service.py`:

- `_measured_payload` (lines 1313–1320) writes `"surface_area_mm2"`;
- `_measured_from_payload` (lines 1323–1332) reads `"area_mm2"`.

A design restored from the journal therefore loses area. This is a Python
worker bug that affects both front doors after a restart; it is not caused by
the Rust migration and was not edited (read-only QA). Volume, envelope,
validity and all checks survive the restore, so "design works again" still
passes; the area row is honestly `—` rather than a substituted number.

---

## 8. Console and network errors

| Run | Console errors | Page errors | Failed requests |
|---|---|---|---|
| Rust (8877, full scenario) | 1: `Failed to load resource: the server responded with a status of 503 (Service Unavailable)` — the deliberate export during the worker outage | 0 | 0 |
| Python (8872) | 0 | 0 | 0 |

All `/api/studio/*` responses were 200 except the intentional 503 above.
There were no 4xx/5xx responses in the normal FreeCAD/OpenCascade/assembly
flows, no CORS/CSP errors, no React errors, and no failed assets. The only
network anomaly is the expected geometry-worker outage, which produced the
clear 503 envelope and the UI's red error text quoted in §6.

---

## 9. Findings and gaps

1. **P1 — Rust chat cannot build FreeCAD (blocker).**
   `chat_design.rs::finish_design` refuses every provider except
   `opencascade`, despite `available_cad_providers` returning FreeCAD as
   available and the worker/proxy building it correctly. This also makes the
   provider selector misleading. The current test pins the refusal; the
   r2-followup report documents it as a known gap. It blocks the FreeCAD
   acceptance and the ADR-0011 Linux end-to-end path through the Rust front
   door.
2. **P2 — journal-restored designs lose surface area (shared backend bug).**
   `surface_area_mm2` vs `area_mm2` mismatch in the Python design service; it
   is visible after a worker restart on both front doors. This cell did not
   edit it.
3. **STEP/FCStd digests are timestamped.** Not a parity failure, but a
   cross-run equality trap for future QA. Compare size, geometry and the
   front door's own receipt/file match instead.
4. **Server rows are present.** The tested dist shows `HASH-CHAIN OK` and
   server rows with sequence/digest/chain in both front doors. The C3
   server-row work is effective in this snapshot; I did not see the earlier
   missing-server-rows symptom.
5. **The Rust front door adds the `geometry_worker` health block.** Expected
   by ADR-0010; not a difference to "fix".
6. **The `cached` flag and journal head differ by session.** Expected; not
   value differences.
7. **No Windows/Solid Edge runtime claim.** Solid Edge is correctly disabled
   on this Linux host with its reason; the ADR-0008 bridge cannot run here.

---

## 10. Evidence files

All paths are relative to this cell.

### Screenshots

| File | Shows |
|---|---|
| `shots/01-rust-connected-providers.png` | Rust UI connected; CAD target selector: OpenCascade 8.0.1.1, FreeCAD 1.1.4, Solid Edge disabled + reason; no default target |
| `shots/02-rust-freecad-selected-refused.png` | Rust, FreeCAD selected (`aria-checked`), the refusal reply, no design card |
| `shots/03-rust-opencascade-card.png` | Rust OpenCascade design card in the session; HLR sheet and section rail |
| `shots/03b-rust-opencascade-card-closeup.png` | Unscrolled card close-up: `B-Rep Verified`, `measured by OpenCascade (opt-in) 8.0.1.1`, volume/area/envelope, checks (second fresh process, same binary/dist) |
| `shots/04-rust-hlr-4views-hidden.png` | HLR crop: iso/top/front/right views, visible + dashed hidden lines, `2B VISIBLE / 2H HIDDEN` |
| `shots/05-rust-section-2321.46.png` | Section A–A: XZ, 4.00 mm, `AREA 2,321.46 mm²`, hatch, A–A markers |
| `shots/06-rust-clay3d.png` | 3D clay view of the OCP design |
| `shots/07-rust-ledger-hash-chain-ok.png` | Ledger: `HASH-CHAIN OK`, server rows with seq/digest/chain, local rows marked local |
| `shots/08-rust-options.png` | Options drawer (inspector/export/providers/connection) |
| `shots/09-rust-export-receipt.png` | STEP export receipt: 55,554 bytes and the full sha256 |
| `shots/10-rust-worker-down-export-503.png` | Worker down: red `geometry_worker_unavailable` error in the export card; settings cards still render |
| `shots/11-rust-worker-down-chat-works.png` | Worker down: normal non-design chat reply still arrives |
| `shots/12-rust-after-restart-export.png` | After restart: export succeeds again |
| `shots/13-rust-after-restart-design.png` | After restart: new OpenCascade design card, volume, stamp |
| `shots/20-python-connected-providers.png` | Python reference: identical provider selector |
| `shots/21-python-freecad-card.png` | Python FreeCAD chat card (8872): `measured by FreeCAD 1.1.4`, volume, B-Rep Verified, HLR view-only |
| `shots/21b-python-freecad-card-closeup.png` | FreeCAD card close-up (8876, chat-only fresh instance): stamp, measurements, 5 checks, FCStd/STEP chips |
| `shots/22-python-opencascade-card.png` | Python OpenCascade card |
| `shots/23-python-hlr.png` | Python HLR crop (4 views, hidden lines) |
| `shots/24-python-section-2321.46.png` | Python Section A–A |
| `shots/25-python-clay3d.png` | Python 3D clay |
| `shots/26-python-ledger-hash-chain-ok.png` | Python ledger, `HASH-CHAIN OK`, server rows |
| `shots/27-python-options.png` | Python Options drawer |
| `shots/28-python-export-receipt.png` | Python OpenCascade STEP receipt, 55,554 bytes, sha256 |

### Machine-readable evidence and artifacts

| File | Contents |
|---|---|
| `shots/rust-evidence.json` | Full Rust capture: providers, FreeCAD/OpenCascade cards, HLR/section values, ledger, export receipt, crash/restart, all `/api/studio` statuses/bodies, console/page errors |
| `shots/python-evidence.json` | The same capture for the Python public server |
| `shots/diff-summary.json` | Normalised Rust-vs-Python visible/network diff, FreeCAD proxy routes, export files |
| `shots/assembly-request.json` | The two-part place-only request |
| `shots/assembly-rust.json`, `shots/assembly-python.json` | Assembly responses through both front doors |
| `shots/freecad-design-rust-api.json`, `shots/freecad-design-python-api.json` | FreeCAD `/design` responses (Rust current binary vs Python) |
| `shots/freecad-drawing-rust.json`, `shots/freecad-section-rust.json` | FreeCAD drawing/section through the Rust proxy (byte-identical to Python) |
| `shots/freecad-export-rust.step`, `shots/freecad-export-python.step` | FreeCAD STEP exports (same size; timestamped digests) |
| `shots/rust-opencascade-export.step`, `shots/python-opencascade-export.step` | OCP STEP downloads (same size; timestamped digests) |
| `shots/python-freecad-export.step` | FreeCAD STEP downloaded during the first Python UI pass (21,648 bytes, sha `3a32209c…`) |

The raw shell logs and all intermediate request/response files are under
`/tmp/opencode/rust-e2e-qa/logs/`; the frozen dist is
`/tmp/opencode/rust-e2e-qa/dist/`. The read-only harness scripts are
`/tmp/opencode/rust-e2e-qa/scenario.mjs` (the full UI scenario),
`card_shot.mjs` (card close-up), `probe.mjs` (initial probe) and
`make_diff.py` (the diff summary). None is product source.

---

## 11. Limits and honesty

- This is Linux-only evidence. No Windows build, Solid Edge runtime, COM
  bridge or installer was exercised; Solid Edge is correctly reported
  unavailable on this host.
- No remote chat provider was called and no API key/credential was read. The
  chat provider is the local deterministic helper; every number shown is
  kernel-measured or declared.
- The UI was built once from the 01:50 snapshot; C3's later edits are not
  in this dist. The test is of that snapshot, as the task requires.
- The Rust binary was the current release build at test time (hash verified
  against `/proc/<pid>/exe`); the workspace's `backend/` tree was already
  untracked/concurrently edited by other cells, and this cell did not edit
  it.
- The FreeCAD failure is a front-door chat limitation, not a worker,
  provider or proxy failure; the direct `/design` and view/export routes
  prove the rest of the chain.
- The area `—` after restart is a shared Python restore bug documented in
  §7.4; it is reported rather than hidden, and no number is substituted.
- No speed, benchmark, Windows-runtime, packaging or cutover claim is made.
