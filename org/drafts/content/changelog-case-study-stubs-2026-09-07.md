# Kala Content stubs — DRAFT ONLY (Founder approve before any publish)

**Status:** DRAFT — do not publish · Hold until Founder  
**Release target:** 2026-09-17 (aspirational)  
**Do not hand to Social/Web** until Founder picks portfolio + unlocks.  
**Revise:** Product not complete — don’t lead on eval context. Dual-track below so metrics can’t leak into shippable copy.  
**Founder HOLD (pricing):** No public pricing posture — no price bands, Pro waitlists, or founding paid experiments. Pilot CTA = interest only (not paid).

---

# TRACK 1 — Public-facing stubs (shippable shape)

> **DRAFT — do not publish**  
> Allowed in body: **path · job type · honest limits** only.  
> Forbidden in this track: pass-rate, 150/150, avg score, per-run scores, score callouts, “public-safe line” with metrics.

## Theme

Kala is a CAD agent that ships valid STEP — not a chat that dumps scripts.  
Story arc: **brief → FreeCAD tools → STEP.**

## Changelog stub (pre-release)

### Unreleased · toward 2026-09-17

#### Demo portfolio (path · job type)

| Job type | STEP path |
|----------|-----------|
| Bearing support bracket | `outputs/eval/batch_cad1000_150/runs/simple-ca-ad4623cb/model.step` |
| Clevis bracket | `outputs/eval/batch_cad1000_150/runs/simple-so-160138d7/model.step` |
| L-shaped mechanical bracket | `outputs/eval/batch_cad1000_150/runs/simple-nx-91d2a335/model.step` |

#### Honest limits
- Product not complete; don’t treat eval folders as ship-ready marketing proof.
- Extreme-tier demos: no strong claims until Founder visual review.
- Dimensions approximate when the brief omits numbers; proportions follow the description.
- Single-part runs fuse to one solid; we do not fake assemblies as one blob.
- Not claiming AGMA-true gears, FEA, or drop-in SolidWorks replacement.

#### CTA (when Founder unlocks channel)
- Request a pilot (interest only — not paid / not a Pro waitlist / not a founding paid experiment) — brief in, STEP out.

---

## Case-study stubs (3)

### 1. Bearing support bracket

- **Job type:** Bearing support bracket  
- **Path:** `outputs/eval/batch_cad1000_150/runs/simple-ca-ad4623cb/model.step`  
- **Brief (qualitative):** Two raised cylindrical end supports joined by an elongated base and connecting web; fuse to one solid.  
- **Story arc:** Brief → FreeCAD tool loop (box/cylinder/fuse/cut/fillet/hole) → STEP.  
- **Honest limits:** Exact dims need a numbered brief; not an extreme-tier claim.

### 2. Clevis bracket

- **Job type:** Clevis bracket  
- **Path:** `outputs/eval/batch_cad1000_150/runs/simple-so-160138d7/model.step`  
- **Brief (qualitative):** Arched upright, large central through-hole, paired fork arms, mounting + pin holes; fuse to one solid.  
- **Story arc:** Brief → FreeCAD tools → STEP.  
- **Honest limits:** Not kinematics, pin-fit tolerances, or a drawing package.

### 3. L-shaped mechanical bracket

- **Job type:** L-shaped mechanical bracket  
- **Path:** `outputs/eval/batch_cad1000_150/runs/simple-nx-91d2a335/model.step`  
- **Brief (qualitative):** Base flange, perpendicular upright, rounded transitions, circular mounting features.  
- **Story arc:** Brief → FreeCAD tools → STEP.  
- **Honest limits:** Familiar shape for UX checklist demos only after Founder visual confirm; no loud claims yet.

---

## README snippet (org / product) — public track

> **DRAFT**

```markdown
## What Kala is
A CAD agent that calls FreeCAD tools and exports STEP — not a chat that dumps scripts.

## Sample STEPs (paths · job types)
- Bearing support: `outputs/eval/batch_cad1000_150/runs/simple-ca-ad4623cb/model.step`
- Clevis bracket: `outputs/eval/batch_cad1000_150/runs/simple-so-160138d7/model.step`
- L-bracket: `outputs/eval/batch_cad1000_150/runs/simple-nx-91d2a335/model.step`

## Pilot (interest only — not paid)
Brief in → STEP out. Extreme-tier demos wait on Founder review.
No price bands, Pro waitlists, or founding paid experiments in public copy.
```

---

# TRACK 2 — INTERNAL appendix (Founder unlock required)

> **INTERNAL — not for Social/Web/ship copy**  
> Do not paste this section into public drafts, landing, or posts.  
> Founder must unlock before any number below appears outside this appendix.

## Batch (from `outputs/eval/batch_cad1000_150/summary.json`)

| Field | Value |
|-------|--------|
| batch | `batch_cad1000_150` |
| n | 150 |
| passed | 150 |
| pass_rate | 1.0 |
| avg_score | 0.912 |
| done | true |
| updated | 2026-09-07T17:48:47Z |

## Portfolio runs (ids + scores — internal)

| Run id | Job type | Score | Notes |
|--------|----------|-------|-------|
| `simple-ca-ad4623cb` | Bearing support bracket | 1.0 | `ok=true`; elapsed 23.36 s; 20 tools / 1 fail recovered; 1 solid |
| `simple-so-160138d7` | Clevis bracket | 1.0 | `ok=true`; elapsed 66.91 s; 63 tools / 9 fails recovered; 1 solid |
| `simple-nx-91d2a335` | L-shaped bracket | 0.95 | `ok=true`; reason `step_read=step_probe_exit=-11`; elapsed 28.72 s; 21 tools / 0 fails |

## Extreme-tier

Blocked for strong public claims until Founder visual review (eval note: some extreme exports may risk stub geometry even when a scorer passes).

---

## Hard limits (Content)

- Dual-track only: Track 1 = path · job type · honest limits; Track 2 = metrics/ids/scores behind INTERNAL label.
- Hold publish; no handoff to Social/Web until Founder picks portfolio + unlocks.
- No public pricing posture: no price bands, Pro waitlists, founding paid experiments; pilot CTA = interest not paid.
- No scraping copyrighted manuals.
- Watermark every draft until Founder approve.

## Handoffs

- ← CMO (Founder revise + dual-track ask)
- → Social / Web: **only after** Founder portfolio pick + unlock — send Track 1 only unless Founder unlocks Track 2
- → Founder / Orchestrator: unlock numbers when product is ready

**Package:** `/workspace/kala-content/changelog-case-study-stubs-2026-09-07.md`  
**Mirror:** `org/drafts/content/changelog-case-study-stubs-2026-09-07.md`
