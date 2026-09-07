# Kala Marketing & Docs Site

Static marketing and documentation site for Kala CAD agent, built with Astro and TypeScript.

## Tech Stack

- **Framework:** Astro 7.3+ (static output)
- **Adapter:** @astrojs/vercel (static mode)
- **Styling:** Custom CSS with design tokens
- **Fonts:** IBM Plex Sans & IBM Plex Mono (self-hosted via fontsource)
- **Deployment:** Vercel

## Local Development

### Prerequisites

- Node.js 22.12.0 or higher
- npm 10.9.7 or higher

### Install Dependencies

```bash
cd web
npm install
```

### Development Server

```bash
npm run dev
```

Open [http://localhost:4321](http://localhost:4321) in your browser.

### Build

```bash
npm run build
```

Output will be in `dist/` directory.

### Preview Production Build

```bash
npm run preview
```

## Deployment on Vercel

### Option 1: Via Vercel Dashboard

1. Import the GitHub repository in Vercel
2. **Set Root Directory to `web`** in Project Settings → General → Build & Development Settings
3. Vercel will auto-detect Astro and use the settings from `vercel.json`
4. Deploy

### Option 2: Via Vercel CLI

```bash
cd web
npx vercel
```

Follow the prompts. The CLI will use `vercel.json` configuration automatically.

### Important: Root Directory Configuration

The site is located in the `web/` subdirectory of the repository. When deploying to Vercel:

- **Root Directory MUST be set to `web`**
- This can be configured in:
  - Vercel Dashboard → Project Settings → General → Root Directory
  - Or during initial project setup via CLI

Without this setting, Vercel will try to build from the repository root and fail.

## Project Structure

```
web/
├── public/              # Static assets (favicon, etc.)
├── src/
│   ├── components/      # Astro components
│   │   ├── Button.astro
│   │   ├── CtaBand.astro
│   │   ├── DocsCard.astro
│   │   ├── Footer.astro
│   │   ├── Hero.astro
│   │   ├── HowItWorks.astro
│   │   ├── Audience.astro
│   │   ├── ProofGrid.astro
│   │   ├── SiteNav.astro
│   │   └── SkipLink.astro
│   ├── layouts/         # Page layouts
│   │   └── BaseLayout.astro
│   ├── pages/           # Routes (file-based routing)
│   │   ├── index.astro
│   │   ├── docs.astro
│   │   └── docs/
│   │       ├── get-started.astro
│   │       ├── definition-of-done.astro
│   │       └── public-messaging.astro
│   └── styles/
│       └── global.css   # Global styles + design tokens
├── astro.config.mjs     # Astro configuration
├── vercel.json          # Vercel deployment config
├── package.json
├── tsconfig.json
└── README.md
```

## Design System

### Color Tokens (CSS Variables)

```css
--bg: #0B0D10;              /* Main background */
--bg-elevated: #12151A;      /* Elevated surfaces */
--bg-muted: #1A1F27;         /* Muted backgrounds */
--border: #2A313C;           /* Default borders */
--border-strong: #3D4654;    /* Emphasized borders */

--text: #E8ECF1;             /* Primary text */
--text-muted: #9AA3B2;       /* Secondary text */
--text-faint: #6B7380;       /* Tertiary text */

--accent: #5B9FD4;           /* Brand accent */
--accent-hover: #7EB3DE;     /* Accent hover state */
--accent-muted: rgba(91,159,212,0.14); /* Accent backgrounds */

--cta-bg: #E8ECF1;           /* CTA button background */
--cta-text: #0B0D10;         /* CTA button text */
--cta-hover: #FFFFFF;        /* CTA hover state */

--ok: #6FBF8A;               /* Success states */
--warn: #D4A85B;             /* Warning states */
--danger: #D47B7B;           /* Error states */
```

### Typography

- **Headings:** IBM Plex Sans (600 weight)
- **Body:** IBM Plex Sans (400/500 weight)
- **Code/Mono:** IBM Plex Mono (400/500 weight)

### Spacing Scale (4px base)

```
--space-1 through --space-24
(0.25rem to 6rem)
```

### Layout Constants

- Nav height: 64px
- Content max-width: 1120px (landing)
- Prose max-width: 720px (docs)
- Card border-radius: 8px
- Section padding: 96px desktop / 64px mobile

## Pages

### Landing Page (`/`)

- Hero with locked copy
- Proof pillars (3 qualitative points)
- How it works (4-step process)
- Audience section
- CTA band with pilot request
- Footer

### Docs Hub (`/docs`)

- Overview page with cards linking to:
  - Get started
  - Definition of done
  - What we say publicly

### Documentation Pages

- `/docs/get-started` - Pilot access and workflow
- `/docs/definition-of-done` - Quality gates and validation
- `/docs/public-messaging` - Messaging guidelines

## Content Guidelines

### Approved Messaging

- "Kala is a CAD agent that ships valid STEP — not a chat that dumps scripts"
- "Brief in, STEP out"
- "Reliability you can open in CAD — STEP first"
- Qualitative proof only (tool-calling approach, supported part types, honest positioning)

### BANNED Content

Do **NOT** include:

- Quantitative metrics (pass rates, scores, benchmarks)
- Leaderboards or competitor comparisons
- Pricing tables or cost information
- Fake dashboards or invented data
- "150/150" or similar metric badges

Metrics stay internal until Founder approval.

## Accessibility

- Skip link for keyboard navigation
- Semantic HTML
- Focus indicators (2px accent + 2px offset)
- `prefers-reduced-motion` support
- ARIA labels where appropriate

## Performance

- Static site generation (SSG)
- Minimal JavaScript (no islands by default)
- Self-hosted fonts with `font-display: swap`
- Optimized CSS with CSS variables
- Clean HTML output

## Browser Support

Modern evergreen browsers:

- Chrome/Edge (last 2 versions)
- Firefox (last 2 versions)
- Safari (last 2 versions)

## License

Proprietary - Kala Agent © 2026
