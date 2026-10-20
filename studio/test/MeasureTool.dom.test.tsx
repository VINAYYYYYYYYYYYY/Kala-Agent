// @vitest-environment happy-dom
// The measure tool on the drafting sheet.
//
// The viewport is mocked so the test can drive the pick the real
// SceneCanvas performs: the mock reports a real intersection point in
// model millimetres, exactly like a raycast against the face mesh. The
// distance maths and the tool state machine are covered by
// `measure.test.ts`; the raycast itself by `raycast.test.ts`.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  pick: vi.fn(),
}));

vi.mock('../src/viewer/SceneCanvas', async () => {
  const React = await import('react');
  return {
    SceneCanvas: (props: {
      meshSource: string;
      onMeasurePoint(point: { x: number; y: number; z: number } | null): void;
    }) =>
      React.createElement(
        'div',
        { className: 'viewer-canvas', 'data-mesh-source': props.meshSource },
        React.createElement(
          'button',
          {
            type: 'button',
            className: 'mock-pick',
            onClick: () => props.onMeasurePoint(mocks.pick()),
          },
          'pick',
        ),
      ),
  };
});

import { DraftingSheet } from '../src/components/drafting/DraftingSheet';
import type { DraftingSheetProps, TitleBlockData } from '../src/components/drafting/DraftingSheet';
import { buildLocal, defaultBox } from '../src/viewer/localPrimitives';
import { getState, resetStore } from '../src/state/store';
import type { StudioLocalPreview } from '../src/types/studio';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
  mocks.pick.mockReset();
});

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

const mesh: StudioLocalPreview = buildLocal('box', defaultBox());

function props(overrides: Partial<DraftingSheetProps> = {}): DraftingSheetProps {
  return {
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

function measureButton(container: HTMLElement): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>('button[aria-label="Measure distance"]')!;
}

function note(container: HTMLElement): string {
  return container.querySelector('.drafting-measure-note')?.textContent ?? '';
}

describe('measure tool', () => {
  it('is disabled while there is no mesh to measure on', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    const button = measureButton(container);
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('aria-pressed')).toBe('false');
  });

  it('arms the tool and waits for the first point', () => {
    const { container } = render(<DraftingSheet {...props({ mesh, meshSource: 'local' })} />);
    const button = measureButton(container);
    expect(button.disabled).toBe(false);

    fireEvent.click(button);
    expect(getState().measure.active).toBe(true);
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(note(container)).toBe('MEASURE · PICK POINT 1 ON THE MESH');
  });

  it('measures the real distance between two picked points', () => {
    const { container } = render(<DraftingSheet {...props({ mesh, meshSource: 'local' })} />);
    fireEvent.click(measureButton(container));

    // Two real raycast hits on the front face of the 40 mm box.
    mocks.pick.mockReturnValueOnce({ x: 0, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    expect(note(container)).toBe('MEASURE · PICK POINT 2 ON THE MESH');
    expect(getState().measure.points).toHaveLength(1);

    mocks.pick.mockReturnValueOnce({ x: 30, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    expect(note(container)).toBe('MEASURE · 30.00 mm');
    expect(getState().measure.points).toHaveLength(2);
  });

  it('ignores a pick that missed the mesh', () => {
    const { container } = render(<DraftingSheet {...props({ mesh, meshSource: 'local' })} />);
    fireEvent.click(measureButton(container));

    mocks.pick.mockReturnValueOnce(null);
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    expect(getState().measure.points).toHaveLength(0);
    expect(note(container)).toBe('MEASURE · PICK POINT 1 ON THE MESH');
  });

  it('clears the measurement on Escape', () => {
    const { container } = render(<DraftingSheet {...props({ mesh, meshSource: 'local' })} />);
    fireEvent.click(measureButton(container));
    mocks.pick.mockReturnValueOnce({ x: 0, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    mocks.pick.mockReturnValueOnce({ x: 40, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    expect(note(container)).toBe('MEASURE · 40.00 mm');

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(getState().measure.active).toBe(false);
    expect(getState().measure.points).toEqual([]);
    expect(container.querySelector('.drafting-measure-note')).toBeNull();
  });

  it('clears the measurement on a second click of the tool', () => {
    const { container } = render(<DraftingSheet {...props({ mesh, meshSource: 'local' })} />);
    fireEvent.click(measureButton(container));
    mocks.pick.mockReturnValueOnce({ x: 0, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    mocks.pick.mockReturnValueOnce({ x: 40, y: 0, z: 15 });
    fireEvent.click(container.querySelector<HTMLButtonElement>('.mock-pick')!);
    expect(getState().measure.points).toHaveLength(2);

    fireEvent.click(measureButton(container));
    expect(getState().measure.active).toBe(false);
    expect(getState().measure.points).toEqual([]);
    expect(container.querySelector('.drafting-measure-note')).toBeNull();
  });
});
