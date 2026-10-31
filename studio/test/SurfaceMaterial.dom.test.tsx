// @vitest-environment happy-dom
// The surface material control.
//
// `viewport.material` is a real store field the viewport already honours.
// Its one home is Details › Properties, inside the Inspector (D3): the old
// duplicate in the bottom ruler bar is gone with the bar's controls.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { BottomBar } from '../src/components/bottombar/BottomBar';
import { Inspector } from '../src/components/inspector/Inspector';
import { getState, resetStore } from '../src/state/store';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

function materialGroup(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('[aria-label="Surface material"]')!;
}

describe('surface material control', () => {
  it('exposes the three real material kinds in Details › Properties', () => {
    const { container } = render(<Inspector showHeading={false} />);
    const group = materialGroup(container);
    const buttons = Array.from(group.querySelectorAll<HTMLButtonElement>('button'));
    expect(buttons).toHaveLength(3);
    expect(buttons.map((button) => button.getAttribute('data-material'))).toEqual([
      'default',
      'matte',
      'wireframe',
    ]);
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Default',
      'Matte',
      'Wireframe',
    ]);
  });

  it('writes the real store field from the Properties inspector', () => {
    const { container } = render(<Inspector showHeading={false} />);
    const wire = container.querySelector<HTMLButtonElement>('button[data-material="wireframe"]')!;
    fireEvent.click(wire);
    expect(getState().viewport.material).toBe('wireframe');
    expect(wire.getAttribute('aria-checked')).toBe('true');

    const matte = container.querySelector<HTMLButtonElement>('button[data-material="matte"]')!;
    fireEvent.click(matte);
    expect(getState().viewport.material).toBe('matte');
    expect(matte.getAttribute('aria-checked')).toBe('true');
    expect(wire.getAttribute('aria-checked')).toBe('false');
  });

  it('has no second copy in the status bar', () => {
    const { container } = render(<BottomBar />);
    expect(container.querySelector('[aria-label="Surface material"]')).toBeNull();
  });

  it('documents that the canvas background follows the theme', () => {
    const { container } = render(<Inspector showHeading={false} />);
    const note = container.querySelector('.inspector-material-note')?.textContent ?? '';
    expect(note).toContain('background follows the workspace theme');
  });
});
