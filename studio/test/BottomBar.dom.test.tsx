// @vitest-environment happy-dom
// The status bar readout.
//
// The bar is read-only (D5): it states the loopback connection, the measured
// readout, the FreeCAD integration row and the backend version. The view
// toggles, the render mode, Export and Settings live on the top bar and in
// the sheet's tool rail; this file asserts that none of them came back.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { BottomBar, describeConnectionStatus } from '../src/components/bottombar/BottomBar';
import { dispatch, getState, resetStore } from '../src/state/store';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
  // The readout describes the drawing, so it is shown only while the
  // drawing is open (chat-first direction).
  dispatch({ type: 'chrome/drawing-open' });
});

function readout(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.bottom-bar-readout')!;
}

describe('status bar readout', () => {
  it('shows the readout only while the drawing is open', () => {
    dispatch({ type: 'chrome/drawing-close' });
    const { container } = render(<BottomBar />);
    expect(container.querySelector('.bottom-bar-readout')).toBeNull();
    expect(container.querySelector('.bottom-bar-source')).toBeNull();
    // The always-on facts stay.
    expect(container.querySelector('.bottom-bar-connection')).toBeTruthy();
    expect(container.querySelector('.bottom-bar-freecad')).toBeTruthy();
    expect(container.querySelector('.bottom-bar-version')).toBeTruthy();
  });

  it('states the honest scale and never claims 1:1', () => {
    const { container } = render(<BottomBar />);
    const text = readout(container).textContent ?? '';
    expect(text).toContain('SCALE');
    expect(text).toContain('FIT');
    expect(text).not.toContain('1:1');
  });

  it('keeps the nominal readout and its source badge', () => {
    const { container } = render(<BottomBar />);
    const text = readout(container).textContent ?? '';
    expect(text).toContain('ENV');
    expect(text).toContain('VOL');
    expect(text).toContain('AREA');
    // Without a mesh the readout says so instead of inventing numbers.
    expect(text).toContain('—');
    expect(container.querySelector('.bottom-bar-source')?.getAttribute('data-source')).toBe('—');
  });

  it('keeps the fixed readout order ENV · VOL · AREA · SCALE · source', () => {
    const { container } = render(<BottomBar />);
    const text = readout(container).textContent ?? '';
    const order = ['ENV', 'VOL', 'AREA', 'SCALE'];
    const positions = order.map((label) => text.indexOf(label));
    expect(positions.every((position) => position >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(readout(container).lastElementChild?.classList.contains('bottom-bar-source')).toBe(true);
  });

  it('is read-only: the view toggles, render mode, Export and Settings are gone', () => {
    const { container } = render(<BottomBar />);
    for (const label of ['Grid', 'Axes', 'Dims', 'Section A-A', 'Reset View', 'Render mode']) {
      expect(container.querySelector(`[aria-label="${label}"]`)).toBeNull();
    }
    expect(container.querySelector('.bottom-bar-export')).toBeNull();
    expect(container.querySelector('.bottom-bar-options')).toBeNull();
    // The one control is the connection label.
    const buttons = Array.from(container.querySelectorAll('button'));
    expect(buttons).toHaveLength(1);
    expect(buttons[0]!.classList.contains('bottom-bar-connection')).toBe(true);
  });
});

describe('status bar connection', () => {
  it('names the desktop connection and opens Settings › Connection', () => {
    dispatch({
      type: 'connection/ready',
      connection: { baseUrl: 'http://127.0.0.1:8791', token: 't', mode: 'desktop' },
    });
    const { container } = render(<BottomBar />);
    const label = container.querySelector<HTMLButtonElement>('.bottom-bar-connection')!;
    expect(label.textContent).toContain('Connected · Desktop');
    expect(label.getAttribute('data-state')).toBe('desktop');

    fireEvent.click(label);
    expect(getState().optionsOpen).toBe(true);
    expect(getState().optionsCard).toBe('connection');
  });

  it('names the browser fallback rather than calling it a desktop', () => {
    dispatch({
      type: 'connection/ready',
      connection: { baseUrl: 'http://127.0.0.1:8791', token: 't', mode: 'browser-manual' },
    });
    const { container } = render(<BottomBar />);
    const label = container.querySelector<HTMLElement>('.bottom-bar-connection')!;
    expect(label.textContent).toContain('Connected · Browser fallback');
    expect(label.getAttribute('data-state')).toBe('browser');
  });

  it('reports the real state when it is not connected', () => {
    expect(describeConnectionStatus({ kind: 'idle' })).toMatchObject({
      label: 'Not connected',
      state: 'idle',
    });
    expect(describeConnectionStatus({ kind: 'connecting' })).toMatchObject({
      label: 'Connecting',
      state: 'connecting',
    });
    expect(describeConnectionStatus({ kind: 'error', message: 'refused' })).toMatchObject({
      label: 'Connection failed',
      state: 'error',
      title: 'refused',
    });

    const { container } = render(<BottomBar />);
    expect(container.querySelector('.bottom-bar-connection')?.textContent).toContain(
      'Not connected',
    );
  });

  it('reports the backend version from the store', () => {
    dispatch({
      type: 'server/providers',
      providers: [],
      health: 'ok',
      healthMessage: '',
      version: '0.1.0',
      stepAvailable: false,
    });
    const { container } = render(<BottomBar />);
    expect(container.querySelector('.bottom-bar-version')?.textContent).toBe('backend v0.1.0');

    cleanup();
    resetStore();
    const empty = render(<BottomBar />);
    expect(empty.container.querySelector('.bottom-bar-version')?.textContent).toBe('backend v—');
  });

  it('keeps the FreeCAD status dot', () => {
    dispatch({
      type: 'server/integrations',
      integrations: [
        {
          id: 'freecad',
          name: 'FreeCAD',
          category: 'cad',
          platforms: ['linux'],
          status: 'available_via_neutral_file',
          capabilities: ['step'],
          notes: 'neutral STEP handoff',
        },
      ],
    });
    const { container } = render(<BottomBar />);
    const freecad = container.querySelector<HTMLElement>('.bottom-bar-freecad')!;
    expect(freecad.getAttribute('data-status')).toBe('available_via_neutral_file');
    expect(freecad.querySelector('.bottom-bar-freecad-dot')).toBeTruthy();
    expect(freecad.getAttribute('aria-label')).toBe('FreeCAD: neutral');
  });
});
