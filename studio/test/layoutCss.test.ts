// Regression coverage for the root layout and the document-level CSP.
//
// The conductor saw, in a real 1280x800 browser, the viewport canvas grow
// to ~959px and the footer land below the window. Cause: `.app-shell-body`
// left its single row as an implicit `auto` track, so the sidebar's
// intrinsic content (min-height:auto) stretched the row past the shell and
// pushed everything below it. These tests pin the structural invariants
// that fix it (bounded rows + internally scrolling sidebar) and the
// security invariant that let us drop the ignored meta `frame-ancestors`
// directive without weakening any policy.

import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

const studioRoot = path.resolve(__dirname, '..');
const css = fs.readFileSync(path.join(studioRoot, 'src', 'styles', 'studio.css'), 'utf8');
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

describe('root layout invariants', () => {
  it('the document does not scroll: the app shell owns scrolling', () => {
    expect(cssBlock(css, 'html, body, #root')).toContain('overflow: hidden');
  });

  it('bounds the app-shell body row so the sidebar cannot stretch it', () => {
    const body = compactCss(cssBlock(css, '.app-shell-body'));
    expect(body).toContain('grid-template-rows: minmax(0, 1fr)');
    expect(body).toContain('overflow: hidden');
    expect(body).toContain('min-height: 0');
  });

  it('makes the sidebar shrinkable and internally scrollable', () => {
    const sidebar = compactCss(cssBlock(css, '.app-sidebar'));
    expect(sidebar).toContain('min-height: 0');
    expect(sidebar).toContain('overflow-y: auto');
  });

  it('keeps the workspace grid itself shrinkable', () => {
    const main = compactCss(cssBlock(css, '.app-shell-main'));
    expect(main).toContain('min-height: 0');
    expect(main).toContain('grid-template-rows: auto minmax(0, 1fr)');
  });

  it('bounds every row of the compact (<=1100px) stacked layout', () => {
    const compact = compactCss(cssBlock(css, '@media (max-width: 1100px)'));
    expect(compact).toContain(
      'grid-template-rows: auto minmax(0, 1.4fr) minmax(0, 1fr)',
    );
    expect(compact).toContain('"viewport"');
    expect(compact).toContain('"rail"');
    // The old block left an auto-sized row and a phantom `footer-area`
    // grid area (the footer lives outside the main grid), which is what
    // let a stacked rail grow the column past the window.
    expect(compact).not.toContain('footer-area');
  });

  it('gives the <=720px layout a bounded sidebar band and workspace row', () => {
    const min = compactCss(cssBlock(css, '@media (max-width: 720px)'));
    expect(min).toContain('grid-template-rows: auto minmax(0, 1fr)');
    expect(min).toContain('max-height: 42vh');
  });

  it('lets a tall boot-gate card scroll now that the document cannot', () => {
    expect(compactCss(cssBlock(css, '.boot-gate'))).toContain('overflow: auto');
  });
});

describe('chat-first workspace invariants', () => {
  it('gives the agent the full height of the viewport band', () => {
    // The agent owns `.app-shell-viewport`; without an explicit height it
    // would size to its content and either overflow the band or collapse the
    // scrolling thread.
    expect(compactCss(cssBlock(css, '.app-shell-viewport > .chat-panel'))).toContain('height: 100%');
  });

  it('bounds the chat thread so a long conversation cannot push the composer off-screen', () => {
    const panel = compactCss(cssBlock(css, '.chat-panel'));
    // header · intro (geometry context + starters) · thread · composer
    expect(panel).toContain('grid-template-rows: auto auto minmax(0, 1fr) auto');
  });

  it('caps the starter-prompt band so it cannot take over the viewport', () => {
    expect(compactCss(cssBlock(css, '.chat-panel-intro'))).toContain('max-height: 34vh');
  });

  it('anchors the docked rail preview and gives it a bounded height', () => {
    // `.viewer-canvas` is absolutely positioned against its nearest
    // positioned ancestor, so the rail preview must establish one — and it
    // must not be free to grow the rail past the shell.
    const preview = compactCss(cssBlock(css, '.right-rail-preview'));
    expect(preview).toContain('position: relative');
    expect(preview).toContain('overflow: hidden');
    expect(preview).toMatch(/height: \d+px/);
  });

  it('adds a middle rail row only while the preview is docked', () => {
    // tabs · preview · scrolling panel. With no docked preview the rail falls
    // back to the two-row declaration above.
    expect(compactCss(cssBlock(css, '.right-rail.has-preview'))).toContain(
      'grid-template-rows: auto auto minmax(0, 1fr)',
    );
    expect(compactCss(cssBlock(css, '.right-rail'))).toContain(
      'grid-template-rows: auto minmax(0, 1fr)',
    );
  });

  it('keeps the estimate panel readable without leaning on colour', () => {
    const estimates = compactCss(cssBlock(css, '.inspector-estimates'));
    expect(estimates).toContain('border: 1px solid var(--border)');
    expect(compactCss(cssBlock(css, '.inspector-estimates dt'))).toContain('font-size: 11px');
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
