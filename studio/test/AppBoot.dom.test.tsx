// @vitest-environment happy-dom
// React DOM tests that exercise the actual shipped components under
// StrictMode to confirm the conductor's reproduction of the BootGate
// double-mount bug stays fixed.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  resolveConnectionFromBridge: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    getHealth: mocks.getHealth,
    getIntegrations: mocks.getIntegrations,
    getProviders: mocks.getProviders,
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

beforeEach(() => {
  vi.stubGlobal('fetch', () => Promise.reject(new Error('unexpected fetch in unit test')));
  mocks.resolveConnectionFromBridge.mockResolvedValue({
    baseUrl: 'http://127.0.0.1:8765',
    token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    mode: 'desktop' as const,
  });
  mocks.getHealth.mockResolvedValue({
    status: 'ok',
    service: 'kala-studio',
    version: '0.1.0',
    offline: false,
    step_available: false,
    integrations: [],
    providers: [],
  });
  mocks.getIntegrations.mockResolvedValue({ integrations: [
    {
      id: 'fusion',
      name: 'Fusion',
      category: 'cad',
      platforms: ['windows', 'macos'],
      status: 'available_via_neutral_file',
      capabilities: ['stl', 'step'],
      notes: '',
    },
  ] });
  mocks.getProviders.mockResolvedValue({ providers: [
    {
      id: 'local',
      name: 'Local helper',
      configured: true,
      notes: '',
    },
  ] });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  mocks.getHealth.mockReset();
  mocks.getIntegrations.mockReset();
  mocks.getProviders.mockReset();
  mocks.resolveConnectionFromBridge.mockReset();
});

describe('App boot under StrictMode', () => {
  it('reaches the shell with health, integrations, and providers populated', async () => {
    const view = render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    await waitFor(() => {
      expect(mocks.getHealth).toHaveBeenCalled();
      expect(mocks.getIntegrations).toHaveBeenCalled();
      expect(mocks.getProviders).toHaveBeenCalled();
    });

    await waitFor(() => {
      expect(view.container.textContent).toContain('Kala Studio');
    });
  });

  it('does not hang on retry when the first attempt fails', async () => {
    let attempts = 0;
    mocks.getHealth.mockImplementation(async () => {
      attempts += 1;
      if (attempts === 1) throw new Error('boom');
      return {
        status: 'ok',
        service: 'kala-studio',
        version: '0.1.0',
        offline: false,
        step_available: false,
        integrations: [],
        providers: [],
      };
    });

    const view = render(
      <StrictMode>
        <App />
      </StrictMode>,
    );

    await waitFor(() => {
      // The retry button appears when bootstrap fails; clicking it
      // re-runs bootstrap. After clicking, the shell eventually renders.
      const retry = view.container.querySelector('button.boot-gate-retry');
      expect(retry).toBeTruthy();
    });

    const retry = view.container.querySelector<HTMLButtonElement>('button.boot-gate-retry');
    retry?.click();

    await waitFor(() => {
      expect(view.container.textContent).toContain('Kala Studio');
    });
  });
});