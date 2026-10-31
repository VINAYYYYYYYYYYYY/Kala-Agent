// @vitest-environment happy-dom
// The drawing split panel header (chat-first direction, 2026-10-31).
//
// The panel is the existing DraftingSheet under a slim mono-caps header. The
// header is the single home of the render mode (D3), names the active
// drawing, and closes the panel (which unmounts the sheet and its canvas).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';

vi.mock('../src/viewer/SceneCanvas', async () => {
  const { createElement } = await import('react');
  return {
    SceneCanvas: () => createElement('div', { className: 'viewer-canvas' }),
  };
});

import { DrawingPanel } from '../src/components/shell/DrawingPanel';
import type { DrawingPanelProps } from '../src/components/shell/DrawingPanel';
import type { TitleBlockData } from '../src/components/drafting/DraftingSheet';
import { dispatch, getState, resetStore, selectRenderMode, useUiState } from '../src/state/store';

/** Mirrors AppShell: the sheet's render mode follows the store. */
function Harness(props: DrawingPanelProps) {
  const renderMode = useUiState(selectRenderMode);
  return <DrawingPanel {...props} renderMode={renderMode} />;
}

const titleBlock: TitleBlockData = {
  sheet: 'SHEET 01',
  partId: 'STUDIO-BOX',
  title: 'box · 40.00 × 20.00 × 30.00 mm',
  scale: 'FIT',
  units: 'mm',
  projection: '3RD ANGLE',
  tolerance: '—',
  revision: 'DRAFT',
  author: 'Kala Studio',
};

function panelProps(overrides: Partial<DrawingPanelProps> = {}): DrawingPanelProps {
  return {
    name: 'BOX',
    mesh: null,
    meshSource: 'none',
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default',
    background: 'paper',
    renderMode: 'pencil2d',
    showSection: true,
    titleBlock,
    units: 'mm',
    viewResetToken: 0,
    ...overrides,
  };
}

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
  resetStore();
});

describe('the drawing panel header', () => {
  it('names the drawing and holds the only render-mode control', () => {
    const { container } = render(<DrawingPanel {...panelProps({ name: 'L-bracket 100 x 80 x 60' })} />);

    expect(container.querySelector('.drawing-panel-label')?.textContent).toBe('DRAWING');
    expect(container.querySelector('.drawing-panel-name')?.textContent).toBe(
      'L-bracket 100 x 80 x 60',
    );
    const group = container.querySelector('[aria-label="Render mode"]')!;
    expect(group).toBeTruthy();
    expect(
      Array.from(group.querySelectorAll('button')).map((button) => button.textContent),
    ).toEqual(['2D Pencil', '3D Clay']);
    // The sheet still owns exactly one canvas.
    expect(container.querySelectorAll('.viewer-canvas')).toHaveLength(1);
  });

  it('switches the render mode from the header and updates the sheet', () => {
    const { container } = render(<Harness {...panelProps()} />);
    const pencil = container.querySelector<HTMLButtonElement>(
      '.drawing-panel-mode-button[data-render-mode="pencil2d"]',
    )!;
    const clay = container.querySelector<HTMLButtonElement>(
      '.drawing-panel-mode-button[data-render-mode="clay3d"]',
    )!;
    expect(pencil.getAttribute('aria-pressed')).toBe('true');
    expect(clay.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(clay);
    expect(getState().renderMode).toBe('clay3d');
    expect(clay.getAttribute('aria-pressed')).toBe('true');
    expect(pencil.getAttribute('aria-pressed')).toBe('false');
    expect(container.querySelector('.drafting-sheet')?.getAttribute('data-render-mode')).toBe(
      'clay3d',
    );
  });

  it('closes the panel from its close button', () => {
    dispatch({ type: 'chrome/drawing-open' });
    const { container } = render(<DrawingPanel {...panelProps()} />);
    const close = container.querySelector<HTMLButtonElement>('button[aria-label="Close drawing"]')!;
    expect(close.getAttribute('title')).toContain('Ctrl/Cmd+B');

    fireEvent.click(close);
    expect(getState().drawingOpen).toBe(false);
  });
});
