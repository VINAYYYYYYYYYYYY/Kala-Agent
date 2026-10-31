// @vitest-environment happy-dom
// Browser manual-connection regression coverage.
//
// Reproduces the conductor's actual-browser finding: with no desktop
// bridge, the operator fills origin and token in the in-page form and
// clicks Connect. The shell must boot, all three registry requests must
// fire, and the initial box preview must be requested/rendered. The
// previous bridge-mocked tests did not exercise this path because they
// short-circuited on a successful resolveConnectionFromBridge.
//
// The manual path reuses the same lifecycle-owned registry handle as the
// bridge probe, so these tests also assert that unmounting mid-connect
// aborts the in-flight requests and that a retry returns to the form.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  postPreview: vi.fn(),
  resolveConnectionFromBridge: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    getHealth: mocks.getHealth,
    getIntegrations: mocks.getIntegrations,
    getProviders: mocks.getProviders,
    postPreview: mocks.postPreview,
  };
});

vi.mock('../src/api/connection', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/connection')>();
  return {
    ...actual,
    resolveConnectionFromBridge: mocks.resolveConnectionFromBridge,
  };
});

import { App } from '../src/App';
import { getState, resetStore } from '../src/state/store';

const QA_TOKEN = 'kala-studio-browser-qa-20261018';
const ORIGIN = 'http://127.0.0.1:8765';

const PREVIEW = {
  positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  indices: [0, 1, 2],
  bounds: { min: [0, 0, 0], max: [1, 1, 1] },
  engine: 'analytic',
  warnings: [],
};

beforeEach(() => {
  resetStore();
  vi.stubGlobal('fetch', () => Promise.reject(new Error('unexpected fetch in unit test')));
  // Bridge is unavailable in this scenario.
  mocks.resolveConnectionFromBridge.mockRejectedValue(new Error('desktop bridge unavailable'));
  mocks.getHealth.mockResolvedValue({
    status: 'ok',
    service: 'kala-studio',
    version: '0.1.0',
    offline: false,
    step_available: false,
    integrations: [],
    providers: [],
  });
  mocks.getIntegrations.mockResolvedValue({
    integrations: [
      {
        id: 'fusion',
        name: 'Fusion',
        category: 'cad',
        platforms: ['windows', 'macos'],
        status: 'available_via_neutral_file',
        capabilities: ['stl', 'step'],
        notes: '',
      },
    ],
  });
  mocks.getProviders.mockResolvedValue({
    providers: [{ id: 'local', name: 'Local helper', configured: true, notes: '' }],
  });
  mocks.postPreview.mockResolvedValue(PREVIEW);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  mocks.getHealth.mockReset();
  mocks.getIntegrations.mockReset();
  mocks.getProviders.mockReset();
  mocks.postPreview.mockReset();
  mocks.resolveConnectionFromBridge.mockReset();
  resetStore();
});

function waitForForm(container: HTMLElement) {
  return waitFor(() => {
    expect(container.querySelector('input[type="url"]')).toBeTruthy();
    expect(container.querySelector('input[type="password"]')).toBeTruthy();
  });
}

function submitCredentials(container: HTMLElement): void {
  fireEvent.change(container.querySelector<HTMLInputElement>('input[type="url"]')!, {
    target: { value: ORIGIN },
  });
  fireEvent.change(container.querySelector<HTMLInputElement>('input[type="password"]')!, {
    target: { value: QA_TOKEN },
  });
  fireEvent.click(container.querySelector<HTMLButtonElement>('button.connection-form-submit')!);
}

describe('Manual Connect without desktop bridge', () => {
  it('falls through to the in-page form when the bridge is missing', async () => {
    const view = render(<App />);
    await waitForForm(view.container);
  });

  it('after Connect, registry requests fire, the shell renders, and the initial box preview is requested', async () => {
    const view = render(<App />);
    await waitForForm(view.container);

    submitCredentials(view.container);

    await waitFor(() => {
      expect(mocks.getHealth).toHaveBeenCalled();
      expect(mocks.getIntegrations).toHaveBeenCalled();
      expect(mocks.getProviders).toHaveBeenCalled();
    });

    await waitFor(() => {
      // The app frame renders with the real loopback origin in the status
      // bar; the journal now lives in Details › Activity, closed by default.
      const connection = view.container.querySelector('.bottom-bar-connection');
      expect(connection?.getAttribute('title')).toContain(ORIGIN);
      expect(connection?.textContent).toContain('Browser fallback');
    });

    // Chat-first: the shell is up with the chat centred and no drawing
    // mounted. The preview is requested for the primitive, not rendered on
    // a sheet nobody asked for.
    expect(view.container.querySelector('.chat-panel')).toBeTruthy();
    expect(view.container.querySelector('.drafting-sheet')).toBeNull();

    // The connection was authenticated with the QA token.
    const call = mocks.getHealth.mock.calls[0]!;
    const connection = call[0] as { baseUrl: string; token: string };
    expect(connection.token).toBe(QA_TOKEN);
    expect(connection.baseUrl).toBe(ORIGIN);

    // The initial box preview is built locally and requested from the
    // backend after the debounce.
    expect(getState().preview.local?.kind).toBe('box');
    await waitFor(() => {
      expect(mocks.postPreview).toHaveBeenCalled();
    });
    const request = mocks.postPreview.mock.calls[0]!;
    expect((request[1] as { kind: string }).kind).toBe('box');
  });

  it('reports the registry failure to the operator when the server is unreachable', async () => {
    mocks.getHealth.mockRejectedValue(new Error('network unreachable'));
    mocks.getIntegrations.mockRejectedValue(new Error('network unreachable'));
    mocks.getProviders.mockRejectedValue(new Error('network unreachable'));

    const view = render(<App />);
    await waitForForm(view.container);
    submitCredentials(view.container);

    await waitFor(() => {
      expect(view.container.querySelector('button.boot-gate-retry')).toBeTruthy();
    });
  });

  it('retry after a failed manual connect returns to the form and can connect', async () => {
    // First health probe fails; subsequent calls succeed.
    mocks.getHealth.mockRejectedValueOnce(new Error('boom'));

    const view = render(<App />);
    await waitForForm(view.container);
    submitCredentials(view.container);

    await waitFor(() => {
      expect(view.container.querySelector('button.boot-gate-retry')).toBeTruthy();
    });
    fireEvent.click(view.container.querySelector<HTMLButtonElement>('button.boot-gate-retry')!);

    // The bridge probe runs again, fails, and the manual form returns.
    await waitForForm(view.container);
    submitCredentials(view.container);

    await waitFor(() => {
      // The app frame renders with the real loopback origin in the status
      // bar; the journal now lives in Details › Activity, closed by default.
      const connection = view.container.querySelector('.bottom-bar-connection');
      expect(connection?.getAttribute('title')).toContain(ORIGIN);
      expect(connection?.textContent).toContain('Browser fallback');
    });
  });

  it('aborts an in-flight manual connect when the component unmounts', async () => {
    let capturedSignal: AbortSignal | null = null;
    mocks.getHealth.mockImplementation((_connection: unknown, signal: AbortSignal) => {
      capturedSignal = signal;
      return new Promise(() => undefined);
    });
    mocks.getIntegrations.mockImplementation(() => new Promise(() => undefined));
    mocks.getProviders.mockImplementation(() => new Promise(() => undefined));

    const view = render(<App />);
    await waitForForm(view.container);
    submitCredentials(view.container);

    await waitFor(() => {
      expect(capturedSignal).not.toBeNull();
    });
    expect(capturedSignal!.aborted).toBe(false);

    view.unmount();

    await waitFor(() => {
      expect(capturedSignal!.aborted).toBe(true);
    });
  });
});
