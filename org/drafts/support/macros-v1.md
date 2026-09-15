# Support macros v1 (DRAFT)

**Owner:** Support · **Sources:** `org/drafts/ux-first-l-bracket.md` §2, `outputs/eval/*/rectify_queue.md`, `org/STATUS.md`  
**Rule:** Never paste API keys, Authorization headers, or full `providers.json`. Masked last-4 only if status needed.  
**Refunds/money → Founder + CFO. Production agent changes → CTO only.**

---

## A. First-win / UI macros (user reply)

### Missing or invalid API key
**Title:** Planner needs an API key  
**Steps:** Open `···` → OpenRouter API… and paste a key from openrouter.ai/keys (`sk-or-…`). Save, confirm top bar shows a model id (not `stub`), then Run again.

### URL pasted as key
**Title:** That looks like a URL, not a key  
**Steps:** Paste the key (`sk-or-…`), not the website address. Base URL stays `https://openrouter.ai/api/v1`.

### Stub planner / top bar says `stub`
**Title:** Stub planner is on  
**Steps:** This run used heuristics, not a live model. Add an OpenRouter key, pick a model, Save. Top bar should show the model id before Run.

### FreeCAD missing / open failed
**Title:** FreeCAD isn’t available  
**Steps:** Install FreeCAD (on PATH) or switch Backend → Mock to practice the flow without live CAD. Kala never loads FreeCAD into the UI process.

### FreeCAD sync / heartbeat stale
**Title:** FreeCAD isn’t synced  
**Steps:** Click FreeCAD to open the live doc. Geometry stays in a separate process (avoids crashes).

### Run timeout (15m)
**Title:** Run timed out  
**Steps:** Try a shorter brief, or check FreeCAD isn’t stuck. Golden first-win: L-bracket via `simple_bracket` with parts off.

### Nonzero exit / no JSON
**Title:** Agent run failed  
**Steps:** Run `kala logs --tail 1` (keys redacted). Share the redacted log + brief + backend (freecad/mock) if it still fails.

### JSON parse failure
**Title:** Couldn’t read the run result  
**Steps:** Usually a crashed FreeCAD backend. Check `kala logs`; retry with Mock to confirm UI path, then FreeCAD again.

### First successful L-bracket (golden path)
```text
L-bracket base 60x40x4 and vertical wall 60x4x50 at origin, fuse, two Ø5 base holes at (12,20) and (48,20), export L_bracket.step
```
Prefer UI starter **L-bracket** when shipped; else paste brief. Backend `freecad`, standard parts **off**, procedure `simple_bracket`.

---

## B. Known failure themes → escalate (not user macros)

From latest `rectify_queue` themes (CAD/CTO ownership). Support: capture brief, tool line, `kala logs --tail 1` (redacted), STEP path if any → hand to CAD Lead / CTO.

| Theme | User-facing one-liner | Escalate to |
|-------|----------------------|-------------|
| `boolean_cut` / `boolean_fuse` failed | Cut/fuse failed on this brief — logging for geometry fix | CAD Lead |
| `fillet` failed | Fillet failed — often edge selection / radius | CAD Lead |
| `create_cylinder: Bad args` | Cylinder args rejected — planner/tool-arg issue | CTO |
| `translate` bad args / failed | Translate failed — placement args | CTO / CAD Lead |
| `unknown_part_id` | Unknown part id — catalog/alias gap | Parts Librarian |
| `too_few_tools` / `step_read` (eval) | Internal scorer themes — not user-facing | Eval Ops / Design QA |
| Missing export / max turns | Run ended without STEP | CTO (playbook / turns) |

**Bug report template (to CTO / UX):**
1. What they wanted (brief)  
2. Path: `kala ui` / CLI + backend + parts on/off  
3. Exact error title/body shown (or redacted log line)  
4. Repro steps (≤5)  
5. Expected vs actual  
6. Attachments: redacted log, failed STEP if present  

---

## C. Routing

| Signal | Hand to |
|--------|---------|
| UI/CLI friction, empty states, error copy | UX |
| Geometry / fuse-cut-fillet / STEP wrong | CAD Lead |
| Planner / tool contracts / production agent | CTO |
| Refund / billing / tokens→$ | Founder + CFO |
| Community noise → ticket | Community → Support |

---

## D. STATUS snapshot (do not quote metrics publicly)

See `org/STATUS.md`. Product not complete; no public pass-rate claims until Founder unlocks. Release target ~2026-09-17 aspirational.
