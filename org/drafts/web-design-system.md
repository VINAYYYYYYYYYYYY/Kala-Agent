# Kala web design system — DRAFT
**Owner:** Kala Web · **Date:** 2026-09-07 · **Status:** DRAFT — no publish  
**Stack (agree with Orchestrator):** Vercel host · Astro (static landing + docs) · new `web/` under Kala-Agent (or sibling repo if Founder prefers monorepo split)  
**Copy lock:** CMO soft house — no metric headlines, no pass-rates, no fake dashboards. Mid-Sep = target only. **Pricing HOLD** until reliability bar: no plans, $, paid tiers, waitlist-for-Pro; CTA = pilot only.  
**Related copy:** `org/drafts/` / Web artifact landing-docs-draft v4

---

## 0) Hosting + build (recommendation)

| Choice | Why |
|--------|-----|
| **Vercel** | Zero-ops static + preview deploys per PR; custom domain later; fits Founder shipping code fast |
| **Astro** | Content-first, MDX docs, almost no JS by default → fast + a11y-friendly; islands only if pilot form needs them |
| **`web/` in Kala-Agent** | One clone for Founder; `org/` drafts stay next door; alternative: `kala-web` sibling if site churn must not touch agent CI |

Disagree only if Founder already standardized on Next — then Next App Router static export is fine; Astro still preferred for docs-heavy marketing.

**Do not:** ship fake analytics widgets, any public pricing (plans/$/paid tiers), or eval scoreboards.

---

## 0b) Pricing HOLD (Founder)
Public site must **not** mention plans, $ bands, waitlist-for-Pro, early-access paid tiers, or imply payment. CTA is **Request a pilot** / brief in, STEP out only. Soften drafts the same way. Unlock only when Founder says reliability bar is hit.

---

## 1) Visual direction

### Mood
Dark engineering console meets product marketing: calm, precise, STEP-first. Generous whitespace. Monospace for paths, labels, and “system” accents — not for body paragraphs. One clear H1. No neon cyberpunk, no SaaS purple gradient blobs, no fake 3D dashboards.

### Color tokens (CSS variables)

```css
:root {
  /* Surfaces */
  --bg:            #0B0D10;   /* page */
  --bg-elevated:   #12151A;   /* cards, nav blur base */
  --bg-muted:      #1A1F27;   /* code/path chips */
  --border:        #2A313C;   /* hairlines */
  --border-strong: #3D4654;

  /* Text */
  --text:          #E8ECF1;   /* body — contrast ≥ 12:1 on --bg */
  --text-muted:    #9AA3B2;   /* secondary — aim ≥ 4.5:1 on --bg */
  --text-faint:    #6B7380;   /* captions only if AA still holds */

  /* Brand / accent (steel cyan — engineering, not toy) */
  --accent:        #5B9FD4;   /* links, focus, primary outline */
  --accent-hover:  #7EB3DE;
  --accent-muted:  rgba(91, 159, 212, 0.14);

  /* CTA */
  --cta-bg:        #E8ECF1;   /* high-contrast light button on dark */
  --cta-text:      #0B0D10;
  --cta-hover:     #FFFFFF;

  /* Semantic (sparingly) */
  --ok:            #6FBF8A;   /* “valid STEP” check — not a score */
  --warn:          #D4A85B;   /* draft / target date microcopy */
  --danger:        #D47B7B;   /* form errors only */
}
```

**Contrast rules:** Body/text on `--bg` ≥ 4.5:1; large H1 ≥ 3:1; CTA text on CTA bg ≥ 4.5:1. Never put `--text-muted` on `--bg-muted` without checking. Focus ring: `2px solid var(--accent)` + `2px` offset.

### Type

| Role | Stack | Size / weight | Notes |
|------|--------|---------------|-------|
| Display / H1 | `"IBM Plex Sans", "Inter", system-ui, sans-serif` | clamp(2rem, 4vw, 2.75rem) / 600 | One H1 per page |
| H2 | same | 1.5rem / 600 | Section titles |
| H3 | same | 1.125rem / 600 | Pillar titles |
| Body | same | 1.0625rem / 400 · line-height 1.65 | Max measure ~65ch |
| Eyebrow / label | `"IBM Plex Mono", "JetBrains Mono", ui-monospace, monospace` | 0.75–0.8125rem / 500 · tracking 0.04em · uppercase optional | Theme line, step numbers |
| Code / path | Plex Mono | 0.875rem | `model.step`, brief→tools→STEP |

Load via `fontsource` or self-host — no render-blocking Google Fonts if avoidable. `font-display: swap`.

### Spacing scale (4px base)

`4 · 8 · 12 · 16 · 24 · 32 · 48 · 64 · 96 · 128`

- Section vertical padding: `96` desktop / `64` mobile  
- Card padding: `24`–`32`  
- Nav height: `64`  
- Content max-width: `1120px` (landing) · `720px` (docs prose) · `1280px` (docs with sidebar)

### Radius / elevation
- Radius: `8px` cards · `999px` pills/chips · `6px` buttons  
- Shadow: almost none — prefer `1px` border. Optional soft `0 0 0 1px var(--border)`  
- Dividers: hairline `--border`, not heavy rules

### Motion
Prefer `prefers-reduced-motion: reduce` → no parallax, no autoplay orbits. Subtle fade-in optional (150–200ms). No scroll-jacking.

---

## 2) Section-by-section layout wireframe

ASCII = structure only (not production CSS). Matches CMO hero lock.

### Global
```
[skip to content]
┌─ Nav (sticky, blur optional) ─────────────────────────────┐
│  Kala          Product   Docs          [ Request a pilot ] │
└───────────────────────────────────────────────────────────┘
```

### Hero (`/`)
```
┌─ Hero (min-height ~70vh, max content 1120) ───────────────┐
│  mono eyebrow: Reliability you can open in CAD — STEP first│
│                                                            │
│  H1 (brand-first, wrap ok):                                │
│  Kala is a CAD agent that ships valid STEP —               │
│  not a chat that dumps scripts.                            │
│                                                            │
│  Sub (max ~2 lines): Brief in, STEP out. …                 │
│                                                            │
│  [ Request a pilot ]   See how it works →                  │
│  warn micro: Mid-Sep 2026 target · product still shipping  │
│                                                            │
│  ┌─ Visual slot (optional later) ───────────────────────┐  │
│  │  STEP still OR geometric silhouette placeholder      │  │
│  │  (NO dashboard / NO scores)  alt = part name         │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
```
Mobile: stack CTAs full-width; visual below copy.

### Proof
```
┌─ H2: Reliability you can open in CAD ─────────────────────┐
│  ┌─ Pillar 1 ──┐  ┌─ Pillar 2 ──┐  ┌─ Pillar 3 ──┐       │
│  │ mono label  │  │ mono label  │  │ mono label  │       │
│  │ Brief →     │  │ Mounts,     │  │ No fake     │       │
│  │ FreeCAD →   │  │ brackets,   │  │ benchmarks  │       │
│  │ STEP        │  │ flanges…    │  │             │       │
│  │ body…       │  │ body…       │  │ body…       │       │
│  └─────────────┘  └─────────────┘  └─────────────┘       │
└────────────────────────────────────────────────────────────┘
```
Desktop: 3 equal columns · Mobile: stack. Icon optional (simple line glyphs — path / part / shield) — not charts.

### How it works
```
┌─ H2: How Kala works ──────────────────────────────────────┐
│  01 Brief ─── 02 Build ─── 03 Export ─── 04 Gate          │
│  (horizontal stepper desktop · vertical mobile)            │
│  Each: mono number + title + 1–2 sentence body             │
└────────────────────────────────────────────────────────────┘
```
No animated fake agent UI. Optional later: static tool-trace still.

### Who it’s for
```
┌─ H2 + short body + 4 soft chips ──────────────────────────┐
│  Makers · Robotics · Brackets/machines · Education         │
│  Non-claims line (muted): not SolidWorks replacement / FEA │
└────────────────────────────────────────────────────────────┘
```

### CTA band
```
┌─ elevated band ───────────────────────────────────────────┐
│  H2: Request a pilot                                       │
│  Brief in, STEP out. …                                     │
│  [ Request a pilot ]     (or email field island later)     │
└────────────────────────────────────────────────────────────┘
```

### Footer
```
│  Kala · Docs · Request a pilot · ©                         │
│  a11y one-liner                                            │
```

### Docs hub (`/docs`)
```
┌─ H1 Docs ─────────────────────────────────────────────────┐
│  intro                                                     │
│  ┌ card ┐ ┌ card ┐ ┌ card ┐ ┌ card ┐                      │
│  Get started · Definition of done · CLI · Trust FAQ        │
└────────────────────────────────────────────────────────────┘
```
Docs article: left toc (desktop) · prose measure 65ch · mono for commands.

---

## 3) Component list

| Component | Responsibility | Notes |
|-----------|----------------|-------|
| `SkipLink` | Skip to `#main` | First focusable |
| `SiteNav` | Wordmark, Product, Docs, primary CTA | Sticky; mobile disclosure button labeled |
| `Button` | `primary` (CTA), `ghost` (secondary), `link` | Focus ring; no color-only state |
| `Hero` | Eyebrow, H1, sub, CTAs, microcopy, optional media | **Exactly one H1** |
| `ProofGrid` | 3× `ProofPillar` | Titles from CMO lock; no metric slots |
| `ProofPillar` | Mono label, title, body, optional glyph | |
| `HowItWorks` | 4 steps stepper | |
| `Audience` | Short copy + chips | |
| `CtaBand` | Closing pilot ask | Pilot request only — no price implication |
| `Footer` | Links + a11y line | |
| `DocsCard` | Title, blurb, href | Hub grid |
| `DocsLayout` | Sidebar toc + prose | MDX |
| `Callout` | Honest-limits / draft notes | `warn` tone, not alarmist |
| `PathChip` | Inline mono `model.step` | |
| `PilotForm` *(later island)* | Name/email/brief | Labels tied to inputs; Founder picks endpoint |

**Explicit non-components:** MetricBadge, Leaderboard, PricingTable, PlanCard, PaidTierBadge, FakeDashboard, LogoCloud (until real).

---

## 4) Asset checklist (Founder / Content)

### Required to soft-launch visually
- [ ] **Wordmark** — SVG “Kala” (light-on-dark); optional mark + word  
- [ ] **Favicon** — 32 + 180 apple-touch; simple K mark on `--bg`  
- [ ] **OG image** (1200×630) — wordmark + theme line; **no scores**  
- [ ] **Hero visual** — placeholder geometric silhouette OK for draft; replace with STEP still when unlocked  

### STEP stills (optional later — Founder unlock)
- [ ] 1–3 isometric/orbit stills of non-extreme parts (mounts/brackets/flanges)  
- [ ] Filenames without score suffixes in public paths  
- [ ] Alt text = part description only (“L-bracket solid”, not “score 1.0”)  
- [ ] Prefer real viewer screenshot or clean render; no composite “analytics” chrome  

### Docs / product
- [ ] Screenshot of CLI or bot session **only** when UX labels stable (coordinate UX)  
- [ ] `model.step` path examples as text chips, not fake file-browser UI  

### Engineering handoff in `web/`
- [ ] `public/favicon.ico` + `public/og.png`  
- [ ] `src/styles/tokens.css` (variables above)  
- [ ] MDX content collections: `src/content/docs/`  
- [ ] `astro.config` → static output · Vercel adapter if needed  
- [ ] Preview protect or `DRAFT` banner until Founder publish  

---

## 5) A11y acceptance (basics)
- Landmark: `header` / `main` / `footer`  
- One H1; H2 per section  
- Hit targets ≥ 44×44px on CTAs  
- Keyboard: nav disclosure, buttons, form  
- Visible `:focus-visible`  
- `prefers-reduced-motion` respected  
- Images: meaningful `alt` or empty if decorative  

---

## 6) Founder decisions needed
1. Confirm **Astro + Vercel + `web/`** (or name sibling repo)  
2. Pilot CTA = form vs Calendly vs email `mailto:` for v1  
3. When to unlock STEP stills (and still no metrics)  
4. Custom domain timing vs `*.vercel.app` preview  

---

## Handoffs
- **→ Orchestrator / Founder:** approve direction; scaffold `web/` when ready  
- **→ CMO:** visual locked to soft copy; no metric surfaces in components  
- **→ Content:** stills checklist above  
- **→ UX:** docs CLI card waits on real labels  
- **Do not publish** until Founder asks
