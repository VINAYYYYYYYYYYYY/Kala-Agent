// Regression coverage for the app frame, theme tokens, and the document-level
// CSP.
//
// App-feel wave (2026-10-31), chat-first direction: the chat is the centred,
// primary surface — top bar 44px, chat filling the content area, a drawing
// split panel only while it is open, Details 360 only while it is open, and
// a 28px status bar. The document must never scroll: `.studio-shell` owns the
// window and every zone scrolls internally, so a long chat thread or journal
// cannot push the status bar below the window. These tests pin the tracks and
// the bounded rows that make that true.

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
  'chat.css',
  'shell.css',
  'settings.css',
]
  .map((file) => fs.readFileSync(path.join(stylesDir, file), 'utf8'))
  .join('\n');
const shellCss = fs.readFileSync(path.join(stylesDir, 'shell.css'), 'utf8');
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

describe('app frame invariants', () => {
  it('the document does not scroll: the studio shell owns scrolling', () => {
    expect(compactCss(cssBlock(css, 'html, body, #root'))).toContain('overflow: hidden');
  });

  it('stacks the frame as top bar, content and status bar', () => {
    const shell = compactCss(cssBlock(css, '.studio-shell'));
    expect(shell).toContain('grid-template-rows: auto minmax(0, 1fr) auto');
    expect(shell).toContain('overflow: hidden');
    expect(shell).toContain('min-height: 0');
    expect(shell).toContain('min-width: 0');
    expect(shell).toContain('--chat-min: 400px');
    expect(shell).toContain('--details-width: 360px');
  });

  it('pins the top bar to 44px and the status bar to 28px', () => {
    const top = compactCss(cssBlock(css, '.top-bar'));
    expect(top).toContain('height: 44px');
    expect(top).toContain('min-height: 44px');
    expect(top).toContain('grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr)');
    expect(top).toContain('overflow: hidden');

    const bar = compactCss(cssBlock(css, '.bottom-bar'));
    expect(bar).toContain('height: 28px');
    expect(bar).toContain('min-height: 28px');
    expect(bar).toContain('overflow: hidden');
    expect(bar).not.toContain('height: var(--bottom-bar-height)');
  });

  it('lays out the content row with the chat alone while the drawing is closed', () => {
    const main = compactCss(cssBlock(css, '.studio-main'));
    expect(main).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(main).toContain('position: relative');
    expect(main).toContain('min-width: 0');
    expect(main).toContain('min-height: 0');
    expect(main).toContain('overflow: hidden');

    // The chat column is the primary surface: bounded, paper-toned and
    // internally scrolling, never a fixed dashboard track.
    const chat = compactCss(cssBlock(css, '.studio-chat-column'));
    expect(chat).toContain('grid-template-rows: minmax(0, 1fr)');
    expect(chat).toContain('min-width: 0');
    expect(chat).toContain('min-height: 0');
    expect(chat).toContain('background: var(--sheet)');
  });

  it('splits chat and drawing only while the drawing is open', () => {
    const withDrawing = compactCss(cssBlock(css, ".studio-main[data-drawing='open']"));
    expect(withDrawing).toContain(
      'grid-template-columns: minmax(var(--chat-min), 1fr) minmax(0, 1.3fr)',
    );

    const drawing = compactCss(cssBlock(css, '.studio-drawing-column'));
    expect(drawing).toContain('grid-template-rows: auto minmax(0, 1fr)');
    expect(drawing).toContain('border-left: 1px solid var(--hairline)');
    expect(drawing).toContain('min-width: 0');
    expect(drawing).toContain('min-height: 0');
    expect(drawing).toContain('overflow: hidden');

    // The base grid has no drawing track: the panel is mounted only when
    // the store says it is open.
    const base = compactCss(cssBlock(css, '.studio-main'));
    expect(base).not.toContain('drawing');
  });

  it('renders the Details column at 360px only when it is open', () => {
    const withDetails = compactCss(cssBlock(css, ".studio-main[data-details='open']"));
    expect(withDetails).toContain(
      'grid-template-columns: minmax(0, 1fr) var(--details-width)',
    );

    const both = compactCss(
      cssBlock(css, ".studio-main[data-drawing='open'][data-details='open']"),
    );
    expect(both).toContain(
      'grid-template-columns: minmax(var(--chat-min), 1fr) minmax(0, 1.3fr) var(--details-width)',
    );

    // The base grid has no Details track: the column exists only when open.
    const base = compactCss(cssBlock(css, '.studio-main'));
    expect(base).not.toContain('details-width');

    // Read the base rule from shell.css: the layout media query also has a
    // compound `.details-panel` selector for the overlay case.
    const panel = compactCss(cssBlock(shellCss, '.details-panel'));
    expect(panel).toContain('min-width: 0');
    expect(panel).toContain('min-height: 0');
    expect(panel).toContain('overflow: hidden');
  });

  it('centres the chat content at 760px and docks the composer', () => {
    const frame = compactCss(cssBlock(css, '.chat-panel .chat-panel-frame'));
    expect(frame).toContain('width: 100%');
    expect(frame).toContain('max-width: 760px');
    expect(frame).toContain('margin-inline: auto');

    const panel = compactCss(cssBlock(css, '.chat-panel'));
    expect(panel).toContain('grid-template-rows: auto minmax(0, 1fr) auto');
    expect(panel).toContain('min-height: 0');

    // The empty thread is one centred stage: label, composer, starters.
    const empty = compactCss(cssBlock(css, ".chat-panel[data-empty='yes']"));
    expect(empty).toContain('grid-template-rows: auto minmax(0, 1fr)');
    const center = compactCss(cssBlock(css, '.chat-panel .chat-panel-center'));
    expect(center).toContain('align-content: safe center');
    expect(center).toContain('overflow-y: auto');

    // Once the thread has messages the thread scrolls and the composer
    // stays at the bottom of the column.
    const thread = compactCss(cssBlock(css, '.chat-panel .chat-panel-thread'));
    expect(thread).toContain('overflow-y: auto');
    expect(thread).toContain('overflow-x: hidden');
    const form = compactCss(cssBlock(css, '.chat-panel .chat-panel-form'));
    expect(form).toContain('max-width: 760px');
    expect(form).toContain('margin-inline: auto');
  });

  it('gives the drawing panel one slim header with the only render mode', () => {
    const header = compactCss(cssBlock(css, '.drawing-panel-header'));
    expect(header).toContain('min-height: 34px');
    expect(header).toContain('background: var(--panel)');
    expect(header).toContain('border-bottom: 1px solid var(--hairline)');
    // The segmented control lives here now, not in the top bar.
    expect(compactCss(cssBlock(css, '.drawing-panel-mode'))).toContain(
      'border: 1px solid var(--ink)',
    );
    expect(css).not.toContain('.top-bar-mode');
  });

  it('bounds the AuxView and journal so their lists scroll inside the column', () => {
    expect(compactCss(cssBlock(css, '.aux-view'))).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.journal-panel'))).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.journal-list'))).toContain('overflow-y: auto');
  });

  it('anchors the drafting sheet and keeps the one SceneCanvas bounded', () => {
    const sheet = compactCss(cssBlock(css, '.drafting-sheet'));
    expect(sheet).toContain('overflow: hidden');
    expect(sheet).toContain('min-height: 0');
    expect(compactCss(cssBlock(css, '.drafting-sheet-stage'))).toContain('position: absolute');
  });

  it('lets a tall boot-gate card scroll now that the document cannot', () => {
    expect(compactCss(cssBlock(css, '.boot-gate'))).toContain('overflow: auto');
  });

  it('has compact media queries that keep the zones bounded', () => {
    const compact = compactCss(cssBlock(css, '@media (max-width: 1400px)'));
    expect(compact).toContain('--details-width: 320px');

    // Below 1280 the split view keeps the chat at 360px.
    const split = compactCss(cssBlock(css, '@media (max-width: 1279px)'));
    expect(split).toContain('--chat-min: 360px');

    // Below 1100 Details overlays the drawing column instead of adding a
    // fourth track.
    const narrower = compactCss(cssBlock(css, '@media (max-width: 1100px)'));
    expect(narrower).toContain('--details-width: 300px');
    expect(narrower).toContain(
      "grid-template-columns: minmax(var(--chat-min), 1fr) minmax(0, 1.3fr)",
    );
    expect(narrower).toContain('position: absolute');
    expect(narrower).toContain('width: var(--details-width)');

    const narrow = compactCss(cssBlock(css, '@media (max-width: 900px)'));
    expect(narrow).toContain('--details-width: 260px');
  });

  it('retires the old header/sidebar/rail layout and its fixed tracks', () => {
    expect(css).not.toContain('.app-shell-viewport');
    expect(css).not.toContain('.right-rail');
    expect(css).not.toContain('.app-header');
    const layout = fs.readFileSync(path.join(stylesDir, 'layout.css'), 'utf8');
    expect(layout).not.toContain('--zone-left-width');
    expect(layout).not.toContain('--zone-right-width');
    expect(layout).not.toContain('--bottom-bar-height');
  });

  it('keeps the export panel at its natural height inside the Export dialog', () => {
    expect(compactCss(cssBlock(css, '.export-dialog-body > .export-panel')))
      .toContain('min-height: max-content');
  });

  it('keeps a ledger row a table row despite the legacy journal-entry grid', () => {
    const row = compactCss(
      cssBlock(journalCss, '.studio-zone-right .journal-panel .journal-ledger tr.journal-entry'),
    );
    expect(row).toContain('display: table-row');
  });

  it('keeps the AuxView tabs on their own row so the cut note cannot overlap', () => {
    // The header wraps at every width: the three tabs occupy the first row
    // and the redline cut note is right-aligned underneath them.
    const header = compactCss(cssBlock(draftingCss, '.aux-view .aux-view-header'));
    expect(header).toContain('flex-wrap: wrap');
    const tabs = compactCss(cssBlock(draftingCss, '.aux-view .aux-view-tabs'));
    expect(tabs).toContain('flex: 1 0 100%');
    expect(compactCss(cssBlock(draftingCss, '.aux-view .aux-view-cut-note')))
      .toContain('margin-left: auto');

    const compact = compactCss(cssBlock(draftingCss, '@media (max-width: 1400px)'));
    expect(compact).toContain('.aux-view .aux-view-tabs');
    expect(compact).toContain('flex: 1 0 100%');
    expect(compact).toContain('.aux-view .aux-view-cut-note');
  });

  it('keeps the status-bar values unellipsized at 1440 and ellipsized only at 1280', () => {
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
