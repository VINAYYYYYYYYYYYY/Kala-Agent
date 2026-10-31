// @vitest-environment happy-dom
// The app frame, chat-first: top bar / [chat | drawing split] / status bar,
// and the keyboard shortcuts that move between them.
//
// The chat is always shown and centred; the drawing is a secondary split
// panel that is closed at boot, opens when a CAD-built design arrives and
// toggles from the top bar or Ctrl/Cmd+B. The viewport, boot gate and the
// two dialogs are replaced by recorders so this file tests only the shell's
// own layout and dispatch.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import type { ReactNode } from 'react';

vi.mock('../src/viewer/SceneCanvas', async () => {
  const { createElement } = await import('react');
  return {
    SceneCanvas: () => createElement('div', { className: 'viewer-canvas' }),
  };
});

vi.mock('../src/components/shell/BootGate', async () => {
  const { createElement, Fragment } = await import('react');
  return {
    BootGate: ({ children }: { children: ReactNode }) => createElement(Fragment, null, children),
  };
});

vi.mock('../src/components/options/OptionsDrawer', async () => {
  const { createElement } = await import('react');
  return {
    OptionsDrawer: (props: { open: boolean }) =>
      props.open ? createElement('div', { className: 'mock-options-dialog' }) : null,
  };
});

vi.mock('../src/components/export/ExportDialog', async () => {
  const { createElement } = await import('react');
  return {
    ExportDialog: (props: { open: boolean }) =>
      props.open ? createElement('div', { className: 'mock-export-dialog' }) : null,
  };
});

import { AppShell } from '../src/components/shell/AppShell';
import { dispatch, getState, resetStore } from '../src/state/store';
import { BRACKET_DESIGN_ID, bracketDesignResult } from './designFixtures';

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
  resetStore();
});

describe('app frame', () => {
  it('boots as the centred chat alone, with no sheet, no canvas and no readout', () => {
    const { container } = render(<AppShell />);
    expect(container.querySelector('.top-bar')).toBeTruthy();
    expect(container.querySelector('.studio-chat-column')).toBeTruthy();
    expect(container.querySelector('.chat-panel')).toBeTruthy();
    // The drawing is a secondary surface: not mounted at boot.
    expect(container.querySelector('.studio-drawing-column')).toBeNull();
    expect(container.querySelector('.drafting-sheet')).toBeNull();
    expect(container.querySelector('.viewer-canvas')).toBeNull();
    // Details is closed by default too.
    expect(container.querySelector('[aria-label="Details"]')).toBeNull();
    expect(container.querySelector('.bottom-bar')).toBeTruthy();
    expect(container.querySelector('.bottom-bar-readout')).toBeNull();
    expect(container.querySelector('.studio-main')!.getAttribute('data-drawing')).toBe('closed');
    expect(container.querySelector('.studio-main')!.getAttribute('data-details')).toBe('closed');
  });

  it('opens the primitive sheet from the Drawing toggle and closes it again', () => {
    const { container } = render(<AppShell />);
    const drawing = Array.from(container.querySelectorAll<HTMLButtonElement>('.top-bar button')).find(
      (button) => button.textContent === 'Drawing',
    )!;
    expect(drawing.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(drawing);
    expect(container.querySelector('.studio-main')!.getAttribute('data-drawing')).toBe('open');
    expect(container.querySelector('.studio-drawing-column .drafting-sheet')).toBeTruthy();
    expect(container.querySelectorAll('.viewer-canvas')).toHaveLength(1);
    expect(drawing.getAttribute('aria-pressed')).toBe('true');
    expect(container.querySelector('.drawing-panel-name')?.textContent).toBe('BOX');

    fireEvent.click(container.querySelector<HTMLButtonElement>('button[aria-label="Close drawing"]')!);
    expect(container.querySelector('.studio-drawing-column')).toBeNull();
    expect(container.querySelector('.viewer-canvas')).toBeNull();
    expect(drawing.getAttribute('aria-pressed')).toBe('false');
  });

  it('opens the drawing automatically when a CAD-built design arrives', () => {
    const { container } = render(<AppShell />);
    expect(getState().drawingOpen).toBe(false);

    act(() => {
      dispatch({ type: 'design/received', design: bracketDesignResult });
    });

    expect(getState().drawingOpen).toBe(true);
    expect(container.querySelector('.studio-drawing-column .drafting-sheet')).toBeTruthy();
    expect(container.querySelector('.drawing-panel-name')?.textContent).toBe(
      bracketDesignResult.label,
    );

    fireEvent.click(container.querySelector<HTMLButtonElement>('button[aria-label="Close drawing"]')!);
    expect(container.querySelector('.studio-drawing-column')).toBeNull();
  });

  it('renders the Details column in the grid when it is open', () => {
    const { container } = render(<AppShell />);
    act(() => {
      dispatch({ type: 'chrome/details-open', tab: 'properties' });
    });
    const details = container.querySelector('[aria-label="Details"]')!;
    expect(details).toBeTruthy();
    // It is a column of the shell grid, not an overlay.
    expect(details.closest('.studio-main')).toBeTruthy();
    expect(container.querySelector('.studio-main')!.getAttribute('data-details')).toBe('open');
  });

  it('toggles the drawing with Ctrl/Cmd+B outside text fields', () => {
    render(<AppShell />);
    fireEvent.keyDown(window, { key: 'b', ctrlKey: true });
    expect(getState().drawingOpen).toBe(true);

    fireEvent.keyDown(window, { key: 'B', metaKey: true });
    expect(getState().drawingOpen).toBe(false);

    const textarea = document.createElement('textarea');
    document.body.appendChild(textarea);
    textarea.focus();
    fireEvent.keyDown(textarea, { key: 'b', ctrlKey: true });
    expect(getState().drawingOpen).toBe(false);
    document.body.removeChild(textarea);
  });

  it('opens Settings with Ctrl/Cmd+, outside text fields', () => {
    render(<AppShell />);
    fireEvent.keyDown(window, { key: ',', ctrlKey: true });
    expect(getState().optionsOpen).toBe(true);

    act(() => {
      dispatch({ type: 'options/close' });
    });
    fireEvent.keyDown(window, { key: ',', metaKey: true });
    expect(getState().optionsOpen).toBe(true);
  });

  it('activates a design and opens the drawing from the card summary', () => {
    const { container } = render(<AppShell />);
    act(() => {
      dispatch({ type: 'design/received', design: bracketDesignResult });
      dispatch({ type: 'chat/response', response: {
        text: 'Bracket built.',
        provider: 'local',
        model: null,
        offline: true,
        designId: BRACKET_DESIGN_ID,
      } });
      dispatch({ type: 'chrome/drawing-close' });
    });
    expect(container.querySelector('.studio-drawing-column')).toBeNull();

    const show = container.querySelector<HTMLButtonElement>('.chat-panel-design-show')!;
    expect(show.textContent).toBe('Show drawing');
    fireEvent.click(show);

    expect(getState().drawingOpen).toBe(true);
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    expect(container.querySelector('.studio-drawing-column')).toBeTruthy();
  });
});
