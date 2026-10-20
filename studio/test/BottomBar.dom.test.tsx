// @vitest-environment happy-dom
// The bottom ruler bar readout.
//
// The sheet carries no fixed model-to-paper ratio (the viewport frames the
// part to fit), so the bar states FIT instead of inventing a 1:1 claim.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { BottomBar } from '../src/components/bottombar/BottomBar';
import { getState, resetStore } from '../src/state/store';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

function readout(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.bottom-bar-readout')!;
}

describe('bottom bar readout', () => {
  it('states the honest scale and never claims 1:1', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
    const text = readout(container).textContent ?? '';
    expect(text).toContain('SCALE');
    expect(text).toContain('FIT');
    expect(text).not.toContain('1:1');
  });

  it('keeps the nominal readout and its source badge', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
    const text = readout(container).textContent ?? '';
    expect(text).toContain('ENV');
    expect(text).toContain('VOL');
    expect(text).toContain('AREA');
    // Without a mesh the readout says so instead of inventing numbers.
    expect(text).toContain('—');
    expect(container.querySelector('.bottom-bar-source')?.getAttribute('data-source')).toBe('—');
  });

  it('re-frames the camera on Reset View', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
    const reset = container.querySelector<HTMLButtonElement>('button[aria-label="Reset View"]')!;
    fireEvent.click(reset);
    expect(getState().viewResetToken).toBe(1);
  });

  it('keeps the fixed readout order ENV · VOL · AREA · SCALE · source', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
    const text = readout(container).textContent ?? '';
    const order = ['ENV', 'VOL', 'AREA', 'SCALE'];
    const positions = order.map((label) => text.indexOf(label));
    expect(positions.every((position) => position >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(readout(container).lastElementChild?.classList.contains('bottom-bar-source')).toBe(true);
  });

  it('renders the view toggles as labelled icon buttons', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
    for (const label of ['Grid', 'Axes', 'Dims', 'Section A-A']) {
      const button = container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
      expect(button).toBeTruthy();
      expect(button.getAttribute('title')).toBeTruthy();
      expect(button.querySelector('svg')).toBeTruthy();
      expect(button.textContent).toBe('');
    }
  });
});
