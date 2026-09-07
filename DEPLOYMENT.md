# Kala Web Site Deployment Guide

## Quick Start

The marketing and docs site is located in the `web/` directory of this repository.

### Vercel Deployment (Recommended)

1. Import the repository at [vercel.com](https://vercel.com)
2. **CRITICAL**: Set Root Directory to `web` in Project Settings
   - Go to: Project Settings → General → Build & Development Settings
   - Set **Root Directory** = `web`
3. Deploy

Vercel will automatically:
- Detect the Astro framework
- Use the configuration from `web/vercel.json`
- Build the static site
- Deploy to production

### Local Development

```bash
cd web
npm install
npm run dev
```

Visit [http://localhost:4321](http://localhost:4321)

### Production Build

```bash
cd web
npm run build
```

Output will be in `web/dist/`

## Site Structure

- **Landing page** (`/`) - Hero, proof pillars, how it works, audience, CTA
- **Docs hub** (`/docs`) - Documentation overview
- **Docs pages** (`/docs/*`) - Get started, definition of done, public messaging

## Tech Stack

- Astro 7.3 (static output)
- TypeScript (strictest)
- Self-hosted IBM Plex fonts
- CSS with design tokens
- @astrojs/vercel adapter

## Content Guidelines

### ✅ Approved
- Qualitative messaging about tool-calling approach
- "Brief in, STEP out" positioning
- Use cases: mounts, brackets, flanges, pins, housings
- Honest: "Product still shipping"

### 🚫 Banned
- Metrics, benchmarks, pass rates
- Pricing tables
- Fake dashboards or leaderboards
- Competitor comparisons

## Next Steps

1. Update placeholder email: `pilot@kala.example`
2. Consider adding custom domain in Vercel
3. Monitor Web Analytics (automatically enabled)

For full documentation, see `web/README.md`
