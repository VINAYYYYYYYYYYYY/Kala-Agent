// @vitest-environment happy-dom
// The top bar: the app's one permanent command row.
//
// Chat-first direction (2026-10-31): the wordmark owns the left, the centre
// is empty (the render mode moved into the drawing panel header), and the
// right carries the Drawing split toggle, Details, Export and Settings.
// Every assertion is against real store state, never a local mirror.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { TopBar } from '../src/components/shell/TopBar';
import { getState, resetStore } from '../src/state/store';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

describe('top bar', () => {
  it('renders the banner and the wordmark, with no render mode and no chat toggle', () => {
    const { container } = render(<TopBar />);
    expect(container.querySelector('header[role="banner"]')).toBeTruthy();
    const wordmark = container.querySelector('.top-bar-wordmark')?.textContent ?? '';
    expect(wordmark).toContain('KALA');
    expect(wordmark).toContain('STUDIO');
    // The render mode has one home: the drawing panel header.
    expect(container.querySelector('[aria-label="Render mode"]')).toBeNull();
    expect(container.querySelector('[data-render-mode]')).toBeNull();
    // The chat is always shown, so there is nothing to toggle.
    expect(container.querySelector('[aria-label="Toggle chat"]')).toBeNull();
    expect(container.querySelector('.top-bar-chat-toggle')).toBeNull();
    expect(container.querySelector('.top-bar-center')?.textContent).toBe('');
  });

  it('toggles the drawing split panel and mirrors the state in aria-pressed', () => {
    const { container } = render(<TopBar />);
    const toggle = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent === 'Drawing',
    )!;
    expect(toggle.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(toggle);
    expect(getState().drawingOpen).toBe(true);
    expect(toggle.getAttribute('aria-pressed')).toBe('true');
    expect(toggle.classList.contains('is-active')).toBe(true);

    fireEvent.click(toggle);
    expect(getState().drawingOpen).toBe(false);
    expect(toggle.getAttribute('aria-pressed')).toBe('false');
  });

  it('toggles Details and mirrors the state in aria-pressed', () => {
    const { container } = render(<TopBar />);
    const details = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent === 'Details',
    )!;
    expect(details.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(details);
    expect(getState().detailsOpen).toBe(true);
    expect(details.getAttribute('aria-pressed')).toBe('true');
    expect(details.classList.contains('is-active')).toBe(true);

    fireEvent.click(details);
    expect(getState().detailsOpen).toBe(false);
    expect(details.getAttribute('aria-pressed')).toBe('false');
  });

  it('opens the Export dialog', () => {
    const { container } = render(<TopBar />);
    const exportButton = Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent === 'Export',
    )!;
    expect(exportButton.getAttribute('aria-haspopup')).toBe('dialog');
    fireEvent.click(exportButton);
    expect(getState().exportOpen).toBe(true);
    expect(getState().optionsOpen).toBe(false);
  });

  it('opens Settings from the gear', () => {
    const { container } = render(<TopBar />);
    const settings = container.querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!;
    expect(settings.getAttribute('aria-haspopup')).toBe('dialog');
    fireEvent.click(settings);
    expect(getState().optionsOpen).toBe(true);
  });
});
