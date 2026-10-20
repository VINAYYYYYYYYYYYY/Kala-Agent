// Regression coverage for the Vellum 4-zone layout, theme tokens, and the
// document-level CSP.
//
// The document must never scroll: the `.studio-shell` owns the window and
// every zone scrolls internally, so a long chat thread or journal cannot
// push the bottom ruler bar below the window. These tests pin the grid
// tracks and the bounded rows that make that true.

import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

const studioRoot = path.resolve(__dirname, '..');
const stylesDir = path.join(studioRoot, 'src', 'styles');
const css = [
  'tokens.css',
  'base.css',
  'layout.css',
  'studio.css',
]
  .map((file) => fs.readFileSync(path.join(stylesDir, file), 'utf8'))
  .join('\n');
const journalCss = fs.readFileSync(path.join(stylesDir, 'journal.css'), 'utf8');
const draftingCss = fs.readFileSync(path.join(stylesDir, 'drafting.css'), 'utf8');
const bottomBarCss = fs.readFileSync(path.join(stylesDir, 'bottombar.css'), 'utf8');
const indexHtml = fs.readFileSync(path.join(studioRoot, 'index.html'), 'utf8');

// Extract a `{ ... }` block for a selector, matching braces so it also
// works for at-rules that contain nested rules.
function cssBlock(source: string, selector: string): string {
  const index = source.indexOf(`${selector} {`);
  if (index < 0) throw new Error(`missing selector: ${selector}`);
  const open = source.indexOf('{', index);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    if (source[i] === '{') depth += 1;
    else if (source[i] === '}') {
      depth -= 1;
      if (depth === 0) return source.slice(open + 1, i);
    }
  }
  throw new Error(`unbalanced block: ${selector}`);
}

function compactCss(source: string): string {
  return source.replace(/\s+/g, ' ').trim();
}

describe('4-zone shell invariants', () => {
  it('the document does not scroll: the studio shell owns scrolling', () => {
    expect(compactCss(cssBlock(css, 'html, body, #root'))).toContain('overflow: hidden');
  });

  it('lays out the three desktop columns with a flexible center', () => {
    const shell = compactCss(cssBlock(css, '.studio-shell'));
    expect(shell).toContain('grid-template-columns: var(--zone-left-width) minmax(0, 1fr) var(--zone-right-width)');
    expect(shell).toContain('grid-template-rows: minmax(0, 1fr)');
    expect(shell).toContain('overflow: hidden');
    expect(shell).toContain('min-height: 0');
  });

  it('keeps the center column as sheet + exactly 48px bottom bar', () => {
    const center = compactCss(cssBlock(css, '.studio-zone-center'));
    expect(center).toContain('grid-template-rows: minmax(0, 1fr) var(--bottom-bar-height)');
    expect(center).toContain('min-height: 0');
  });

  it('bounds the right column into AuxView + journal', () => {
    const right = compactCss(cssBlock(css, '.studio-zone-right'));
    expect(right).toContain('grid-template-rows: var(--aux-height) minmax(0, 1fr)');
    expect(right).toContain('min-height: 0');
  });

  it('makes the left chat zone shrinkable and the chat internally scrolling', () => {
    const left = compactCss(cssBlock(css, '.studio-zone-left'));
    expect(left).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.chat-panel'))).toContain(
      'grid-template-rows: auto auto minmax(0, 1fr) auto',
    );
  });

  it('anchors the drafting sheet and keeps the one SceneCanvas bounded', () => {
    const sheet = compactCss(cssBlock(css, '.drafting-sheet'));
    expect(sheet).toContain('overflow: hidden');
    expect(sheet).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.drafting-sheet-stage'))).toContain('position: absolute');
  });

  it('pins the bottom ruler bar to 48px', () => {
    const bar = compactCss(cssBlock(css, '.bottom-bar'));
    expect(bar).toContain('height: var(--bottom-bar-height)');
    expect(bar).toContain('min-height: var(--bottom-bar-height)');
    expect(bar).toContain('overflow: hidden');
  });

  it('bounds the AuxView and journal so their lists scroll inside the column', () => {
    expect(compactCss(cssBlock(css, '.aux-view'))).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.journal-panel'))).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.journal-list'))).toContain('overflow-y: auto');
  });

  it('lets a tall boot-gate card scroll now that the document cannot', () => {
    expect(compactCss(cssBlock(css, '.boot-gate'))).toContain('overflow: auto');
  });

  it('has compact media queries that keep the zones bounded', () => {
    const compact = compactCss(cssBlock(css, '@media (max-width: 1400px)'));
    expect(compact).toContain('--zone-left-width: 280px');
    expect(compact).toContain('--zone-right-width: 260px');

    const narrow = compactCss(cssBlock(css, '@media (max-width: 900px)'));
    expect(narrow).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(narrow).toContain('grid-template-rows: auto minmax(0, 1.2fr) minmax(0, 1fr)');
  });

  it('retires the old header/sidebar/rail layout', () => {
    expect(css).not.toContain('.app-shell-viewport');
    expect(css).not.toContain('.right-rail');
    expect(css).not.toContain('.app-header');
  });

  it('keeps the export panel at its natural height inside the options grid', () => {
    expect(compactCss(cssBlock(css, '.options-card-body > .export-panel')))
      .toContain('min-height: max-content');
  });

  it('keeps a ledger row a table row despite the legacy journal-entry grid', () => {
    const row = compactCss(
      cssBlock(journalCss, '.studio-zone-right .journal-panel .journal-ledger tr.journal-entry'),
    );
    expect(row).toContain('display: table-row');
  });

  it('moves the AuxView tabs to their own row at 1280 so the cut note cannot overlap', () => {
    const compact = compactCss(cssBlock(draftingCss, '@media (max-width: 1400px)'));
    expect(compact).toContain('.aux-view .aux-view-tabs');
    expect(compact).toContain('flex: 1 0 100%');
    expect(compact).toContain('.aux-view .aux-view-cut-note');
  });

  it('keeps the bottom-bar values unellipsized at 1440 and ellipsized only at 1280', () => {
    const base = compactCss(cssBlock(css, '.bottom-bar-readout-value'));
    expect(base).not.toContain('text-overflow');
    const compact = compactCss(cssBlock(bottomBarCss, '@media (max-width: 1400px)'));
    expect(compact).toContain('text-overflow: ellipsis');
  });

  it('compacts the ledger below 1400px instead of adding a horizontal scrollbar', () => {
    const compact = compactCss(cssBlock(journalCss, '@media (max-width: 1400px)'));
    // UTC (2nd) and CHAIN (6th) are hidden in the compact ledger; their
    // values live in each row's title.
    expect(compact).toContain('.journal-ledger thead th:nth-child(2)');
    expect(compact).toContain('.journal-ledger tbody td:nth-child(2)');
    expect(compact).toContain('.journal-ledger thead th:nth-child(6)');
    expect(compact).toContain('.journal-ledger tbody td:nth-child(6)');
    expect(compact).toContain('display: none');
    expect(compact).toContain('min-width: 0');
    expect(compact).toContain('overflow-x: hidden');
  });
});

describe('theme tokens', () => {
  it('defines the vellum drafting palette on the root', () => {
    const root = compactCss(cssBlock(css, ':root'));
    expect(root).toContain('--sheet: #f6f3eb');
    expect(root).toContain('--ink: #23211e');
    expect(root).toContain('--redline: #c84b31');
    expect(root).toContain('--prussian: #2b5b84');
    expect(root).toContain('--stamp: #2a6b4c');
  });

  it('defines the night-slate palette under [data-theme="slate"]', () => {
    const slate = compactCss(cssBlock(css, "[data-theme='slate']"));
    expect(slate).toContain('--sheet: #0a0a0b');
    expect(slate).toContain('--panel: #121214');
    expect(slate).toContain('--card: #161619');
    expect(slate).toContain('--ink: #f5f5f7');
    expect(slate).toContain('--hairline: #26262a');
  });

  it('bundles three local font families and never references Google Fonts', () => {
    const root = compactCss(cssBlock(css, ':root'));
    expect(root).toContain("'Space Grotesk Variable'");
    expect(root).toContain("'Newsreader Variable'");
    expect(root).toContain("'JetBrains Mono Variable'");
    expect(css).not.toContain('fonts.googleapis.com');
    expect(css).not.toContain('fonts.gstatic.com');
  });
});

describe('document CSP', () => {
  const meta = /<meta http-equiv="Content-Security-Policy" content="([^"]*)"/.exec(indexHtml);

  it('is present', () => {
    expect(meta).not.toBeNull();
  });

  it('drops the ignored frame-ancestors directive', () => {
    // `frame-ancestors` is not honoured from a <meta> tag, so it only
    // produced a console warning. It is enforced by the response header
    // instead (asserted below).
    expect(meta![1]).not.toContain('frame-ancestors');
  });

  it('keeps every other directive intact', () => {
    const csp = meta![1];
    expect(csp).toContain("default-src 'self'");
    expect(csp).toContain("script-src 'self'");
    expect(csp).toContain("connect-src 'self'");
    expect(csp).toContain("font-src 'self' data:");
    expect(csp).toContain("base-uri 'none'");
    expect(csp).toContain("form-action 'self'");
  });

  it('is still enforced by the Electron session response header', () => {
    const mainCjs = fs.readFileSync(path.join(studioRoot, 'electron', 'main.cjs'), 'utf8');
    expect(mainCjs).toContain("frame-ancestors 'none'");
  });

  it('is still enforced by the backend response header', () => {
    const backend = path.join(
      path.resolve(studioRoot, '..'),
      'src',
      'kala_agent',
      'studio',
      'middleware.py',
    );
    expect(fs.existsSync(backend)).toBe(true);
    expect(fs.readFileSync(backend, 'utf8')).toContain("frame-ancestors 'none'");
  });
});
