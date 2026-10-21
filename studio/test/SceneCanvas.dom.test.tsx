// @vitest-environment happy-dom
// The 3D viewport's keyboard access.
//
// The viewport is a three.js canvas: the pointer orbits it, but a keyboard
// user must reach it too. These tests pin the DOM contract of that:
//   * the viewport is focusable and says so,
//   * it names every key it owns,
//   * the keys it owns are consumed (the page must not scroll behind it),
//   * the focus ring is visible.
//
// The camera maths itself is pinned in `viewportKeys.test.ts`; a WebGL
// context does not exist in this DOM, so the live scene cannot be asserted
// here — the command that reaches the scene is the one under test.

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/react';
import fs from 'node:fs';
import path from 'node:path';

import { SceneCanvas } from '../src/viewer/SceneCanvas';

const NOOP = (): void => {};

function renderCanvas(overrides: Partial<Parameters<typeof SceneCanvas>[0]> = {}) {
  const props = {
    mesh: null,
    meshSource: 'local' as const,
    designMesh: false,
    designProvider: null,
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default' as const,
    background: 'paper' as const,
    renderMode: 'pencil2d' as const,
    showSection: true,
    viewResetToken: 0,
    measure: { active: false, points: [] },
    onMeasurePoint: NOOP,
    ...overrides,
  };
  const view = render(<SceneCanvas {...props} />);
  const canvas = view.container.querySelector<HTMLElement>('.viewer-canvas')!;
  return { view, canvas };
}

afterEach(() => {
  cleanup();
});

describe('the viewport keyboard contract', () => {
  it('is focusable and carries the keyboard semantics', () => {
    const { canvas } = renderCanvas();

    expect(canvas.getAttribute('tabindex')).toBe('0');
    // The viewport answers keys itself, so it is an application widget and
    // not a static picture.
    expect(canvas.getAttribute('role')).toBe('application');
  });

  it('names every key it owns in its accessible name', () => {
    const { canvas } = renderCanvas({ renderMode: 'clay3d', designMesh: true });

    const label = canvas.getAttribute('aria-label') ?? '';
    expect(label).toContain('3D clay viewport');
    expect(label).toContain('design');
    expect(label).toContain('arrow keys orbit');
    expect(label).toContain('+ and - zoom');
    expect(label).toContain('0 resets the view');
  });

  it('consumes the arrow keys, the zoom keys and the reset key', () => {
    const { canvas } = renderCanvas();

    for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', '+', '-', '0']) {
      const event = new KeyboardEvent('keydown', { key, cancelable: true, bubbles: true });
      canvas.dispatchEvent(event);
      expect(event.defaultPrevented, `the viewport must consume ${key}`).toBe(true);
    }
  });

  it('leaves a key it does not own to the browser', () => {
    const { canvas } = renderCanvas();

    const event = new KeyboardEvent('keydown', { key: 'Tab', cancelable: true, bubbles: true });
    canvas.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  it('has a visible focus ring', () => {
    const css = fs.readFileSync(
      path.resolve(__dirname, '../src/styles/studio.css'),
      'utf8',
    );
    const start = css.indexOf('.viewer-canvas:focus-visible {');
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, css.indexOf('}', start));
    expect(block).toContain('outline: 2px solid var(--focus-ring)');
  });
});
