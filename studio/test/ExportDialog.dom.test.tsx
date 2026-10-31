// @vitest-environment happy-dom
// The focused Export dialog.
//
// Export moved out of the Options dashboard into its own modal. With an
// active design the design export is the primary action and the primitive
// handoff sits behind a disclosure; with no design the primitive block is
// shown directly. Escape and the labelled close button close it, and the
// integration registry no longer appears here (it lives in Settings › CAD).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { ExportDialog } from '../src/components/export/ExportDialog';
import { dispatch, resetStore } from '../src/state/store';
import { bracketDesignResult } from './designFixtures';

const CONNECTION = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop' as const,
};

beforeEach(() => {
  resetStore();
  dispatch({ type: 'connection/ready', connection: CONNECTION });
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('no network in unit test'))));
});

afterEach(() => {
  cleanup();
  resetStore();
  vi.unstubAllGlobals();
});

function renderDialog(open = true, onClose: () => void = () => {}) {
  return render(<ExportDialog open={open} onClose={onClose} />);
}

describe('ExportDialog', () => {
  it('renders nothing while closed', () => {
    const view = renderDialog(false);
    expect(view.container.querySelector('[role="dialog"]')).toBeNull();
  });

  it('is a labelled modal titled Export', () => {
    const view = renderDialog();
    const dialog = view.getByRole('dialog');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe('export-dialog-title');
    expect(view.getByRole('heading', { name: 'Export' })).toBeTruthy();
  });

  it('shows the primitive export directly when no design is active', () => {
    const view = renderDialog();
    const panel = view.container.querySelector('.export-panel')!;
    expect(panel.querySelector('.export-panel-primitive')).toBeTruthy();
    expect(panel.querySelector('.export-panel-disclosure')).toBeNull();
    expect(panel.querySelectorAll('select')).toHaveLength(2);
    if (panel.querySelector('.export-panel-button')) {
      expect(panel.querySelector('.export-panel-button')!.textContent).toContain(
        'Download STL',
      );
    }
  });

  it('puts the active design first and the primitive behind a disclosure', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const view = renderDialog();
    const panel = view.container.querySelector('.export-panel')!;

    const design = panel.querySelector('.export-panel-design');
    const disclosure = panel.querySelector('.export-panel-disclosure');
    expect(design).toBeTruthy();
    expect(disclosure).toBeTruthy();
    expect(design!.compareDocumentPosition(disclosure!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(disclosure!.querySelector('summary')?.textContent).toBe(
      'Export the primitive instead',
    );
    expect(disclosure!.hasAttribute('open')).toBe(false);

    const designButton = panel.querySelector<HTMLButtonElement>('.export-panel-design-button')!;
    expect(designButton.textContent).toContain('Export design STEP');
    expect(designButton.disabled).toBe(false);
  });

  it('does not show the integration registry', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const view = renderDialog();
    expect(view.container.textContent).not.toContain('Integration registry');
    expect(view.container.querySelector('.panel-list')).toBeNull();
  });

  it('closes on Escape and from the labelled close button', () => {
    const onClose = vi.fn();
    const view = renderDialog(true, onClose);

    fireEvent.click(view.getByRole('button', { name: 'Close export' }));
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('returns focus to the opener when it closes', () => {
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();

    const view = renderDialog();
    expect(document.activeElement).toBe(
      view.getByRole('button', { name: 'Close export' }),
    );

    view.rerender(<ExportDialog open={false} onClose={() => {}} />);
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });
});
