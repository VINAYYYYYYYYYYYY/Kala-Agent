# UX draft — first successful L-bracket (`kala ui` + CLI)

**Owner:** UX (draft) · **Implementer:** Founder  
**Due:** ~2026-09-08 · **Release:** 2026-09-17  
**Status:** DRAFT — not implemented  
**Hard limits:** keep FreeCAD out of the Qt process (subprocess runs only); never log provider API keys; do not change agent planner (CTO owns).

Canonical golden brief (from `designs/batch_sample.jsonl` → `sample-l-bracket`):

> L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step

Procedure: `simple_bracket` (envelope → features → standard_parts → export). Standard parts **off** for first success.

---

## 1) Priority UX fix / flow list

Ordered for “first successful L-bracket in `kala ui`” — not a full redesign.

| P | Fix | Why | Where | Done when |
|---|-----|-----|-------|-----------|
| **P0** | **Provider gate before Run** | Top bar shows cryptic `stub` when no key; user can still Run and get a weak/failed session | `kala/ui/desktop.py` `_refresh_planner` / `_send`; `providers_dialog.py` | If no valid key: Run disabled *or* confirm dialog; label reads `no API key` not `stub` |
| **P0** | **Add L-bracket starter chip** | Current starters are Flanged / Bushing / Base plate — not the golden first-win | `STARTERS` in `desktop.py` | First chip = “L-bracket” with golden brief above |
| **P0** | **Humanize run failures** | `_fail` dumps last stderr line; users can’t tell FreeCAD vs key vs parse | `desktop.py` `_fail` (+ map from known exit strings) | Error bubble uses copy from §2; never paste API key material |
| **P1** | **Empty-state teaches first win** | Today: “Tool calls show up here after you run a brief.” — no path | `_paint_empty` | Copy from §2 empty state; CTA points at L-bracket starter |
| **P1** | **Session rail clarity** | `FREECAD: —` and `STATUS: idle` look broken; procedure name hidden | rail + tip | Show procedure id `simple_bracket`; FREECAD states: `idle` / `opening` / `live` / `unavailable`; tip: “Kala plans tools · FreeCAD holds geometry (separate process)” |
| **P1** | **Post-success CTA** | Export filename in rail, but no “open folder / show in FreeCAD” affordance | `_done` | After `done` + export: secondary actions “Open STEP folder” + “Show in FreeCAD” (still via `ensure_live_shown`, not in-process Part.so) |
| **P1** | **Surface backend + parts in chrome** | Backend/parts only in `···` menu — first-timers miss Mock vs FreeCAD | top bar or rail | Compact chips: `backend: freecad` · `parts: off` (default off for L-bracket) |
| **P2** | **CLI first-win hint** | Help suggests `80x50x8 bracket` mock, not golden L-bracket | `kala/cli.py` help footer | Add `kala run "<golden brief>" --backend freecad` and `kala ui` |
| **P2** | **Timeout / parse copy** | 15m timeout + JSON parse errors are engineer-speak | `RunWorker` messages | Use §2 strings |
| **P2** | **Keep crash-safe architecture visible in UX copy** | Users may try to “embed” FreeCAD into the app | FreeCAD button + docs tip | Copy states companion app / subprocess; never imply loading Part into Qt |

**Do not change (UX):** agent planner prompts, tool contracts, procedure step IDs (CTO).  
**Preserve:** `RunWorker` subprocess isolation (`kala run … --json`); password echo on API key; `to_public_dict` masking.

---

## 2) Empty-state + error copy

### Empty states

**Main feed (no prior run)**  
Title (optional): `Ready for a part brief`  
Body:

> Geometry lives in FreeCAD. Kala plans the tool calls.  
> Start with **L-bracket** below — mm units, then Export STEP.

Secondary (dim): `Enter runs · Shift+Enter newline · playbook: simple_bracket`

**Main feed (hydrated last run — keep behavior, clarify label)**  
Keep last-run summary; prefix with `Last session` so it doesn’t look like a live reply.

**Providers dialog empty / no models**  
`No models yet. Paste an OpenRouter key (sk-or-…) and click Refresh models.`

**Providers dialog no key on save** (tighten existing):  
`Paste your OpenRouter API key (starts with sk-or-) to use the LLM planner. Without a key, Kala stays in stub mode and won’t plan a real L-bracket.`

**Session rail tip (replace current two-liner)**  
`FreeCAD = geometry (separate process). Kala = planner. Keys never appear in logs.`

### Error copy (user-facing; map from causes)

| Cause | Title | Body | Next step |
|-------|-------|------|-----------|
| Missing / invalid API key | `Planner needs an API key` | `Open ··· → OpenRouter API… and paste a key from openrouter.ai/keys (sk-or-…).` | Open providers dialog |
| URL pasted as key | `That looks like a URL, not a key` | `Paste the key (sk-or-…), not the website address. Base URL stays https://openrouter.ai/api/v1.` | Focus key field |
| FreeCAD missing / open failed | `FreeCAD isn’t available` | `Install FreeCAD or switch Backend → Mock to practice the flow without live CAD.` | FreeCAD button or backend menu |
| FreeCAD sync / heartbeat stale | `FreeCAD isn’t synced` | `Click FreeCAD to open the live doc. Kala won’t load FreeCAD into this window (avoids crashes).` | FreeCAD button |
| Run timeout (15m) | `Run timed out` | `The agent hit the 15-minute limit. Try a shorter brief, or check FreeCAD isn’t stuck.` | Retry / simplify brief |
| Nonzero exit, no JSON | `Agent run failed` | `Something went wrong outside the UI. Run \`kala logs --tail 1\` for details (keys redacted).` | logs |
| JSON parse failure | `Couldn’t read the run result` | `The agent finished but the UI couldn’t parse its output. Check \`kala logs\` — usually a crashed FreeCAD backend.` | logs / Mock |
| Generic tool fail in feed | keep tool line `fail` | First line of tool message only (≤140 chars) — **never** include env or key fragments | — |
| Stub mode if user force-runs | `Stub planner is on` | `This run used heuristics, not a live model. Add an OpenRouter key for a real L-bracket.` | providers |

**Logging rule (Support FAQ too):** Never print `api_key`, Authorization headers, or full `providers.json` in UI, CLI, or `kala logs` summaries. Masked last-4 only if status needed.

---

## 3) Checklist — first successful L-bracket

### A. Preflight (once per machine)

- [ ] FreeCAD installed and launches from PATH (or accept Mock for dry-run only)
- [ ] OpenRouter account + key (`sk-or-…`)
- [ ] From repo: `kala ui` (or AppImage / desktop entry) — **not** importing FreeCAD into Python REPL with Qt
- [ ] Confirm top bar does **not** say `stub` after saving key (shows model id)

### B. UI path (preferred demo)

1. [ ] Launch `kala ui --backend freecad`
2. [ ] `···` → **OpenRouter API…** → paste key → pick a solid model → **Save**
3. [ ] Confirm rail: `BACKEND freecad` · `parts` off · procedure steps visible
4. [ ] Click starter **L-bracket** (or paste golden brief)
5. [ ] Press **Run** (Enter)
6. [ ] Watch feed: tool lines appear; procedure advances envelope → features → export
7. [ ] Rail `STATUS` → `done`; `EXPORT` shows `L_bracket.step` (or dated equivalent)
8. [ ] Optional: **FreeCAD** button → rail `FREECAD` = `live` / `synced` / `in FreeCAD`
9. [ ] Open the STEP externally or in FreeCAD — confirm L shape + 2 base holes

### C. CLI path (parity / CI-friendly)

```bash
kala run "L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step" \
  --backend freecad --standard-parts off --procedure simple_bracket
```

- [ ] `status: done`
- [ ] `export:` path printed
- [ ] Tool log shows fuse + cuts + export ok
- [ ] `kala logs --tail 1` shows same run; no key material

### D. Crash-safety smoke (must stay green)

- [ ] UI process never `import FreeCAD` / `Part` (runs stay in subprocess via `kala run --json`)
- [ ] Killing FreeCAD mid-run surfaces FreeCAD error copy, UI stays up
- [ ] Mock backend can complete the same brief without FreeCAD for offline UX testing

### E. Definition of done (launch bar)

First-time user, cold machine assumptions aside: **≤5 minutes** from app open → valid L-bracket STEP with providers configured, without reading source.

---

## Handoffs

- **Founder:** implement P0–P1 in `kala/ui/` + CLI help footer  
- **CTO:** only if error mapping needs new structured exit codes from agent/FreeCAD  
- **Content:** screenshots of empty → running → done for docs/launch  
- **Support:** macro answers = §2 table  

**Needs from Founder:** none to start coding — approve/tweak copy if desired.
