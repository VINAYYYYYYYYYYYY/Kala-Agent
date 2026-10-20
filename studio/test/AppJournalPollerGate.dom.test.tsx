// @vitest-environment happy-dom
// Regression coverage for the journal-poller capability gate.
//
// The poller must start ONLY when `/health` explicitly reports
// `design_available: true`. A backend that answers `false` (the Python
// backend without OCP) serves the journal routes with 503, so polling it
// would fill the ledger with failed requests; `null` means the flag has
// not arrived yet and is treated as "not available".

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  getCadProviders: vi.fn(),
  resolveConnectionFromBridge: vi.fn(),
  createJournalPoller: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    getHealth: mocks.getHealth,
    getIntegrations: mocks.getIntegrations,
    getProviders: mocks.getProviders,
    getCadProviders: mocks.getCadProviders,
  };
});

vi.mock('../src/api/connection', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/connection')>();
  return {
    ...actual,
    resolveConnectionFromBridge: mocks.resolveConnectionFromBridge,
  };
});

vi.mock('../src/state/journalPoller', () => ({
  createJournalPoller: mocks.createJournalPoller,
}));

import { App } from '../src/App';
import { dispatch, resetStore } from '../src/state/store';

const poller = {
  start: vi.fn(),
  stop: vi.fn(),
  isRunning: () => false,
};

function health(designAvailable: boolean | undefined): Record<string, unknown> {
  return {
    status: 'ok',
    service: 'kala-studio',
    version: '0.1.0',
    offline: false,
    step_available: false,
    integrations: [],
    providers: [],
    ...(designAvailable === undefined ? {} : { design_available: designAvailable }),
  };
}

// The real `getHealth` records the capability in the store; this mocked
// client keeps that side effect so the App gate sees the same state.
function respondWithHealth(designAvailable: boolean | undefined): Record<string, unknown> {
  dispatch({ type: 'server/capabilities', designAvailable: designAvailable ?? null });
  return health(designAvailable);
}

beforeEach(() => {
  resetStore();
  vi.stubGlobal('fetch', () => Promise.reject(new Error('unexpected fetch in unit test')));
  poller.start.mockClear();
  poller.stop.mockClear();
  mocks.createJournalPoller.mockReset();
  mocks.createJournalPoller.mockReturnValue(poller);
  mocks.resolveConnectionFromBridge.mockResolvedValue({
    baseUrl: 'http://127.0.0.1:8765',
    token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    mode: 'desktop' as const,
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
  mocks.getCadProviders.mockResolvedValue({ providers: [] });
});

afterEach(() => {
  cleanup();
  resetStore();
  vi.unstubAllGlobals();
  mocks.getHealth.mockReset();
  mocks.getIntegrations.mockReset();
  mocks.getProviders.mockReset();
  mocks.getCadProviders.mockReset();
  mocks.resolveConnectionFromBridge.mockReset();
});

async function renderUntilShell(): Promise<HTMLElement> {
  const view = render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
  await waitFor(() => {
    expect(view.container.querySelector('.studio-shell')).toBeTruthy();
  });
  return view.container;
}

describe('journal poller capability gate', () => {
  it('does not start the poller when design_available is false', async () => {
    mocks.getHealth.mockImplementation(async () => respondWithHealth(false));
    await renderUntilShell();
    // Give the capability effect a chance to run after the shell renders.
    await waitFor(() => {
      expect(mocks.getHealth).toHaveBeenCalled();
    });
    expect(mocks.createJournalPoller).not.toHaveBeenCalled();
    expect(poller.start).not.toHaveBeenCalled();
  });

  it('does not start the poller when design_available is absent', async () => {
    mocks.getHealth.mockImplementation(async () => respondWithHealth(undefined));
    await renderUntilShell();
    await waitFor(() => {
      expect(mocks.getHealth).toHaveBeenCalled();
    });
    expect(mocks.createJournalPoller).not.toHaveBeenCalled();
  });

  it('starts the poller when design_available is true', async () => {
    mocks.getHealth.mockImplementation(async () => respondWithHealth(true));
    await renderUntilShell();
    await waitFor(() => {
      expect(mocks.createJournalPoller).toHaveBeenCalled();
      expect(poller.start).toHaveBeenCalled();
    });
  });
});
