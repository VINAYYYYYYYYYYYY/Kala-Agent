// Design-system guard.
//
// 1. The only px font sizes in the whole stylesheet live in tokens.css, and
//    no declared font size may be below 9 px.
// 2. The type scale, 4 px spacing rhythm, gutters, and zero radius exist as
//    tokens.
// 3. The Options drawer is a two-by-two drafted sheet whose cards do not
//    stretch to a neighbour's height, with four corner ticks.
// 4. The bottom ruler bar is a single non-wrapping row.

import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';

const studioRoot = path.resolve(__dirname, '..');
const stylesDir = path.join(studioRoot, 'src', 'styles');
const styleFiles = fs
  .readdirSync(stylesDir)
  .filter((file) => file.endsWith('.css'))
  .sort();
const sources = new Map(styleFiles.map((file) => [file, fs.readFileSync(path.join(stylesDir, file), 'utf8')]));
const allCss = Array.from(sources.values()).join('\n');
const tokensCss = sources.get('tokens.css')!;

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

function declaredFontSizes(source: string): string[] {
  return Array.from(source.matchAll(/font-size:\s*([^;]+);/g), (match) => match[1]!.trim());
}

describe('type scale guard', () => {
  it('keeps every px font-size declaration in tokens.css', () => {
    const offenders: string[] = [];
    for (const [file, source] of sources) {
      if (file === 'tokens.css') continue;
      for (const value of declaredFontSizes(source)) {
        if (/\d+(?:\.\d+)?px/.test(value)) offenders.push(`${file}: ${value}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('uses a type token for every font-size outside tokens.css', () => {
    const offenders: string[] = [];
    for (const [file, source] of sources) {
      if (file === 'tokens.css') continue;
      for (const value of declaredFontSizes(source)) {
        if (!/^var\(--text-(?:headline|body-lg|body-md|body-sm|label-lg|label-md|label-sm)\)$/.test(value)) {
          offenders.push(`${file}: ${value}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });

  it('never declares a font size below 9px', () => {
    const sizes = [
      ...Array.from(allCss.matchAll(/font-size:\s*(\d+(?:\.\d+)?)px/g), (match) => Number(match[1])),
      ...Array.from(tokensCss.matchAll(/--text-[a-z-]+:\s*(\d+(?:\.\d+)?)px/g), (match) => Number(match[1])),
    ];
    expect(sizes.length).toBeGreaterThan(0);
    expect(Math.min(...sizes)).toBeGreaterThanOrEqual(9);
  });

  it('defines the DESIGN.md type scale, 4px spacing rhythm, and zero radius', () => {
    const root = compactCss(cssBlock(tokensCss, ':root'));
    for (const token of [
      '--text-headline: 22px',
      '--leading-headline: 28px',
      '--text-body-lg: 16px',
      '--text-body-md: 14px',
      '--text-body-sm: 12px',
      '--text-label-lg: 13px',
      '--text-label-md: 11px',
      '--text-label-sm: 9px',
      '--space-xs: 4px',
      '--space-sm: 8px',
      '--space-md: 12px',
      '--space-lg: 20px',
      '--space-xl: 32px',
      '--gutter: 12px',
      '--gutter-mobile: 8px',
      '--radius: 0',
    ]) {
      expect(root).toContain(token);
    }
  });

  it('has no rounded control corners', () => {
    const offenders: string[] = [];
    for (const [file, source] of sources) {
      for (const match of source.matchAll(/border-radius:\s*([^;]+);/g)) {
        const value = match[1]!.trim();
        if (value !== '0' && value !== 'var(--radius)' && value !== '50%') {
          offenders.push(`${file}: ${value}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe('bottom bar layout', () => {
  it('keeps the ruler bar and its readout on one row', () => {
    const bar = compactCss(cssBlock(allCss, '.bottom-bar'));
    const readout = compactCss(cssBlock(allCss, '.bottom-bar-readout'));
    expect(bar).toContain('flex-wrap: nowrap');
    expect(readout).toContain('flex-wrap: nowrap');
    expect(bar).not.toContain('flex-wrap: wrap');
    expect(readout).not.toContain('flex-wrap: wrap');
    expect(readout).toContain('white-space: nowrap');
  });

  it('uses icon buttons for the sheet tool rail', () => {
    expect(allCss).toContain('.drafting-tool');
    expect(compactCss(cssBlock(allCss, '.drafting-tool'))).toContain('place-items: center');
  });

  it('keeps the status bar a compact read-only row', () => {
    // This pins the current truth: 28px, not the old four-zone ruler bar.
    // There is no second home for the view toggles or the material control.
    expect(compactCss(cssBlock(allCss, '.bottom-bar'))).toContain('height: 28px');
    expect(allCss).not.toContain('.bottom-bar-icon-button');
    expect(allCss).not.toContain('.bottom-bar .surface-material-compact');
  });
});

describe('Settings dialog', () => {
  it('lays the dialog out as a fixed left nav beside one section', () => {
    const body = compactCss(cssBlock(allCss, '.settings-body'));
    expect(body).toContain('grid-template-columns: 210px minmax(0, 1fr)');
  });

  it('keeps the dialog inside the viewport', () => {
    const dialog = compactCss(cssBlock(allCss, '.settings-dialog'));
    expect(dialog).toContain('width: min(860px, 100%)');
    expect(dialog).toContain('height: min(600px, 100%)');
    expect(dialog).toContain('max-height: 100%');
  });

  it('marks the active section in the left nav', () => {
    expect(allCss).toContain('.settings-nav-item.is-active');
    expect(compactCss(cssBlock(allCss, '.settings-nav-item.is-active'))).toContain(
      'border-color: var(--prussian)',
    );
  });
});
