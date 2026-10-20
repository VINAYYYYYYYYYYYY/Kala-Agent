// @vitest-environment happy-dom
// Vernier caliper slider: the drafted rail, ticks, triangular thumb,
// steppers, mono value input, ARIA contract, and keyboard stepping.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import {
  VernierSlider,
  computeVernierTicks,
  snapToStep,
} from '../src/components/inspector/VernierSlider';
import { Inspector } from '../src/components/inspector/Inspector';
import { getState, resetStore } from '../src/state/store';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

function renderSlider(value = 10, onChange = vi.fn()) {
  const view = render(
    <VernierSlider
      label="Width"
      value={value}
      min={0}
      max={100}
      step={1}
      unit="mm"
      onChange={onChange}
    />,
  );
  const slider = view.container.querySelector<HTMLElement>('[role="slider"]')!;
  return { view, slider, onChange };
}

describe('vernier tick rail', () => {
  it('derives bounded major/minor ticks from min, max, and step', () => {
    const ticks = computeVernierTicks(0, 100, 1);
    expect(ticks[0]).toMatchObject({ value: 0, ratio: 0, major: true });
    expect(ticks[ticks.length - 1]).toMatchObject({ value: 100, ratio: 1, major: true });
    expect(ticks.length).toBeLessThanOrEqual(34);
    expect(ticks.some((tick) => tick.major)).toBe(true);
    expect(ticks.some((tick) => !tick.major)).toBe(true);
  });

  it('samples a huge range instead of drawing thousands of ticks', () => {
    const ticks = computeVernierTicks(0.1, 2000, 0.5);
    expect(ticks.length).toBeLessThanOrEqual(34);
    expect(ticks[0]!.value).toBeCloseTo(0.1, 6);
    expect(ticks[ticks.length - 1]!.value).toBe(2000);
  });

  it('snaps and clamps to the step grid', () => {
    expect(snapToStep(10.26, 0, 100, 0.5)).toBe(10.5);
    expect(snapToStep(-3, 0, 100, 0.5)).toBe(0);
    expect(snapToStep(103, 0, 100, 0.5)).toBe(100);
    expect(snapToStep(12.6, 1, 64, 1, true)).toBe(13);
  });
});

describe('VernierSlider', () => {
  it('renders a real slider with aria values, a thumb, and visible scale labels', () => {
    const { view, slider } = renderSlider();
    expect(slider.getAttribute('aria-valuemin')).toBe('0');
    expect(slider.getAttribute('aria-valuemax')).toBe('100');
    expect(slider.getAttribute('aria-valuenow')).toBe('10');
    expect(slider.getAttribute('aria-valuetext')).toBe('10 mm');
    expect(view.container.querySelector('.vernier-thumb')).toBeTruthy();
    expect(view.container.querySelectorAll('.vernier-tick.is-major').length).toBeGreaterThan(0);
    expect(view.container.querySelectorAll('.vernier-tick:not(.is-major)').length).toBeGreaterThan(0);
    const scale = view.container.querySelector('.vernier-scale')!.textContent;
    expect(scale).toContain('MIN 0');
    expect(scale).toContain('STEP 1');
    expect(scale).toContain('MAX 100');
  });

  it('steps with the −/+ buttons and clamps at the range ends', () => {
    const { view, onChange } = renderSlider(10);
    fireEvent.click(view.getByLabelText('Increase Width'));
    expect(onChange).toHaveBeenLastCalledWith(11);
    fireEvent.click(view.getByLabelText('Decrease Width'));
    expect(onChange).toHaveBeenLastCalledWith(9);

    const high = renderSlider(100);
    expect(high.view.container.querySelector<HTMLButtonElement>('button[aria-label="Increase Width"]')!.disabled).toBe(true);
    const low = renderSlider(0);
    expect(low.view.container.querySelector<HTMLButtonElement>('button[aria-label="Decrease Width"]')!.disabled).toBe(true);
  });

  it('supports arrows, PageUp/PageDown, Home, and End', () => {
    const { slider, onChange } = renderSlider(10);
    fireEvent.keyDown(slider, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith(11);
    fireEvent.keyDown(slider, { key: 'ArrowUp' });
    expect(onChange).toHaveBeenLastCalledWith(11);
    fireEvent.keyDown(slider, { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith(9);
    fireEvent.keyDown(slider, { key: 'ArrowDown' });
    expect(onChange).toHaveBeenLastCalledWith(9);
    fireEvent.keyDown(slider, { key: 'PageUp' });
    expect(onChange).toHaveBeenLastCalledWith(20);
    fireEvent.keyDown(slider, { key: 'PageDown' });
    expect(onChange).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(slider, { key: 'Home' });
    expect(onChange).toHaveBeenLastCalledWith(0);
    fireEvent.keyDown(slider, { key: 'End' });
    expect(onChange).toHaveBeenLastCalledWith(100);
  });

  it('writes a typed value through the mono input', () => {
    const { view, onChange } = renderSlider(10);
    const input = view.getByLabelText('Width value');
    expect(input.className).toContain('vernier-input');
    fireEvent.change(input, { target: { value: '42' } });
    expect(onChange).toHaveBeenLastCalledWith(42);
  });
});

describe('Inspector uses the vernier, not native ranges', () => {
  it('replaces the native range inputs with four VernierSlider rails', () => {
    const view = render(<Inspector />);
    expect(view.container.querySelectorAll('input[type="range"]')).toHaveLength(0);
    const sliders = view.container.querySelectorAll<HTMLElement>('[role="slider"]');
    expect(sliders).toHaveLength(4);
    for (const slider of sliders) {
      expect(slider.getAttribute('aria-valuemin')).not.toBeNull();
      expect(slider.getAttribute('aria-valuemax')).not.toBeNull();
      expect(slider.getAttribute('aria-valuenow')).not.toBeNull();
    }
  });

  it('drives the real parameter state from the keyboard', () => {
    const view = render(<Inspector />);
    const width = view.getByLabelText('Width (size_x) vernier slider');
    fireEvent.keyDown(width, { key: 'ArrowRight' });
    const parameters = getState().parameters as { size_x: number };
    expect(parameters.size_x).toBe(40.5);
  });
});
