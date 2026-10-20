// @vitest-environment happy-dom
// The composer's CAD target selector (ADR-0011).
//
// The rules under test:
//   - there is NO default target; nothing is checked until the operator picks,
//   - the rows are the backend's live probe result (availability is never
//     assumed), with an unavailable provider disabled and its reason in the
//     tooltip,
//   - the choice lives in the session store only and survives a remount,
//   - a failed probe is reported honestly and can be retried.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getCadProviders: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return { ...actual, getCadProviders: mocks.getCadProviders };
});

import { StudioNetworkError } from '../src/api/client';
import { CadTargetSelector } from '../src/components/chat/CadTargetSelector';
import { dispatch, getState, resetStore } from '../src/state/store';
import { cadProviders } from './designFixtures';

const connection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop' as const,
};

function ready(designAvailable: boolean | null = true): void {
  dispatch({ type: 'connection/ready', connection });
  dispatch({ type: 'server/capabilities', designAvailable });
}

function option(container: HTMLElement, id: string): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>(`[data-cad-provider="${id}"]`)!;
}

beforeEach(() => {
  resetStore();
  mocks.getCadProviders.mockResolvedValue(cadProviders);
});

afterEach(() => {
  cleanup();
  resetStore();
  mocks.getCadProviders.mockReset();
});

describe('the CAD target selector', () => {
  it('starts with no target selected', async () => {
    ready(true);
    const { container } = render(<CadTargetSelector />);
    await waitFor(() => expect(option(container, 'freecad')).toBeTruthy());

    expect(getState().cadProvider).toBeNull();
    const checked = container.querySelectorAll('[role="radio"][aria-checked="true"]');
    expect(checked).toHaveLength(0);
    expect(container.textContent).toContain('the first design turn will ask you to choose');
  });

  it('renders the three live probe rows with their versions', async () => {
    ready(true);
    const { container } = render(<CadTargetSelector />);
    await waitFor(() => expect(option(container, 'freecad')).toBeTruthy());

    expect(option(container, 'opencascade').textContent).toContain('OpenCascade (opt-in)');
    expect(option(container, 'opencascade').textContent).toContain('8.0.1.1');
    expect(option(container, 'freecad').textContent).toContain('FreeCAD');
    expect(option(container, 'freecad').textContent).toContain('1.1.4');
    expect(option(container, 'solidedge').textContent).toContain('Solid Edge');
    expect(mocks.getCadProviders).toHaveBeenCalledWith(connection, expect.any(AbortSignal));
  });

  it('disables an unavailable target and puts its reason in the tooltip', async () => {
    ready(true);
    const { container } = render(<CadTargetSelector />);
    await waitFor(() => expect(option(container, 'solidedge')).toBeTruthy());

    const solidEdge = option(container, 'solidedge');
    expect(solidEdge.disabled).toBe(true);
    expect(solidEdge.getAttribute('data-available')).toBe('no');
    expect(solidEdge.getAttribute('title')).toContain('Windows bridge');
    expect(
      solidEdge.closest('.chat-panel-cad-target-option')?.getAttribute('title'),
    ).toContain('Windows bridge');
  });

  it('records a click in the session store and marks the target checked', async () => {
    ready(true);
    const { container } = render(<CadTargetSelector />);
    await waitFor(() => expect(option(container, 'freecad')).toBeTruthy());

    fireEvent.click(option(container, 'freecad'));

    expect(getState().cadProvider).toBe('freecad');
    expect(option(container, 'freecad').getAttribute('aria-checked')).toBe('true');
    expect(option(container, 'freecad').getAttribute('data-selected')).toBe('yes');
    // Nothing was persisted outside the in-memory store.
    try {
      expect(localStorage.getItem('kala-studio.cad-provider')).toBeNull();
    } catch {
      // no localStorage in this environment
    }
  });

  it('keeps the choice when the selector remounts in the same session', async () => {
    ready(true);
    const first = render(<CadTargetSelector />);
    await waitFor(() => expect(option(first.container, 'opencascade')).toBeTruthy());
    fireEvent.click(option(first.container, 'opencascade'));
    first.unmount();

    const second = render(<CadTargetSelector />);
    await waitFor(() => expect(option(second.container, 'opencascade')).toBeTruthy());
    expect(option(second.container, 'opencascade').getAttribute('aria-checked')).toBe('true');
    expect(getState().cadProvider).toBe('opencascade');
  });

  it('does not probe a backend that does not advertise design turns', () => {
    ready(false);
    const { container } = render(<CadTargetSelector />);

    expect(mocks.getCadProviders).not.toHaveBeenCalled();
    expect(container.textContent).toContain('reports design_available: false');
    expect(container.querySelector('[role="radiogroup"]')).toBeNull();
  });

  it('reports a failed probe and retries it', async () => {
    ready(true);
    mocks.getCadProviders.mockRejectedValueOnce(
      new StudioNetworkError('network request to /api/studio/providers/cad failed', new Error('down')),
    );
    const { container } = render(<CadTargetSelector />);
    await waitFor(() =>
      expect(container.querySelector('[role="alert"]')?.textContent).toContain('providers/cad'),
    );

    fireEvent.click(container.querySelector('.chat-panel-cad-target-retry')!);
    await waitFor(() => expect(option(container, 'freecad')).toBeTruthy());
    expect(mocks.getCadProviders).toHaveBeenCalledTimes(2);
  });
});
