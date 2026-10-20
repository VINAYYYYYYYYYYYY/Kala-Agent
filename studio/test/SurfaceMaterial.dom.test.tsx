// @vitest-environment happy-dom
// The surface material control.
//
// `viewport.material` is a real store field the viewport already honours.
// The control that writes it exists twice — compact in the bottom ruler
// bar, spelled out on Options card 01 — and both write the one field.

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
  it('exposes the three real material kinds in the bottom ruler bar', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
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

  it('writes the real store field from the ruler bar', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);
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

  it('writes the same field from Options card 01', () => {
    const { container } = render(<Inspector />);
    const group = materialGroup(container);
    const matte = group.querySelector<HTMLButtonElement>('button[data-material="matte"]')!;
    expect(matte.textContent).toBe('Matte');
    fireEvent.click(matte);
    expect(getState().viewport.material).toBe('matte');
  });

  it('documents that the canvas background follows the theme', () => {
    const { container } = render(<Inspector />);
    const note = container.querySelector('.inspector-material-note')?.textContent ?? '';
    expect(note).toContain('background follows the workspace theme');
  });
});
