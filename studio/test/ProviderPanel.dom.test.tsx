// @vitest-environment happy-dom
// The API-key panel of the composer and of Options card 03.
//
// The rules under test:
//   * a STORED key can be cleared here, and the panel says so,
//   * a key that comes from the ENVIRONMENT is not stored by Studio and stays
//     in force whatever the panel does: the panel must not offer to clear it
//     and must not claim that it cleared anything,
//   * the source line always names where the key comes from.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  postSettings: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return { ...actual, getSettings: mocks.getSettings, postSettings: mocks.postSettings };
});

import { ProviderPanel } from '../src/components/panels/ProviderPanel';
import { dispatch, resetStore } from '../src/state/store';
import type { ProviderCredentialStatus, ProviderRecord } from '../src/types/studio';

const openai: ProviderRecord = {
  id: 'openai',
  name: 'OpenAI (Chat Completions)',
  configured: true,
  notes: 'API-key route.',
};

function credential(
  source: ProviderCredentialStatus['source'],
): ProviderCredentialStatus {
  return {
    provider: 'openai',
    env_var: 'OPENAI_API_KEY',
    configured: source !== 'none',
    source,
  };
}

function settings(source: ProviderCredentialStatus['source']) {
  return { credentials: [credential(source)], providers: [openai], offline: false };
}

function seed(): void {
  dispatch({ type: 'connection/ready', connection: {
    baseUrl: 'http://127.0.0.1:8765',
    token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    mode: 'desktop',
  } });
  dispatch({ type: 'server/providers', providers: [openai], health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
}

beforeEach(() => {
  resetStore();
  seed();
  mocks.getSettings.mockResolvedValue(settings('stored'));
  mocks.postSettings.mockResolvedValue(settings('none'));
});

afterEach(() => {
  cleanup();
  resetStore();
  mocks.getSettings.mockReset();
  mocks.postSettings.mockReset();
});

describe('the API-key panel', () => {
  it('offers "Clear key" for a stored key and reports the clearing honestly', async () => {
    const view = render(<ProviderPanel />);

    const clear = await waitFor(() => {
      const button = view.container.querySelector<HTMLButtonElement>('.provider-key-clear');
      expect(button).toBeTruthy();
      return button!;
    });

    fireEvent.click(clear);

    await waitFor(() => expect(mocks.postSettings).toHaveBeenCalledTimes(1));
    expect(mocks.postSettings.mock.calls[0]![1]).toEqual({
      provider: 'openai',
      api_key: null,
    });
    const status = await waitFor(() => {
      const node = view.container.querySelector('.provider-key-status');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(status.textContent).toBe('Stored API key cleared.');
  });

  it('does not offer "Clear key" for a key that comes from the environment', async () => {
    mocks.getSettings.mockResolvedValue(settings('environment'));

    const view = render(<ProviderPanel />);

    await waitFor(() =>
      expect(view.container.querySelector('.provider-key-source')?.textContent).toContain(
        'Key loaded from environment (OPENAI_API_KEY)',
      ),
    );
    // A key in the environment is not in the panel's hands.
    expect(view.container.querySelector('.provider-key-clear')).toBeNull();

    const note = view.container.querySelector('.provider-key-source-note');
    expect(note?.textContent).toContain('OPENAI_API_KEY');
    expect(note?.textContent).toContain('cannot be removed here');
  });

  it('never claims a clearing it did not perform', async () => {
    mocks.getSettings.mockResolvedValue(settings('environment'));

    const view = render(<ProviderPanel />);
    await waitFor(() =>
      expect(view.container.querySelector('.provider-key-source-note')).toBeTruthy(),
    );

    expect(view.container.textContent).not.toContain('API key cleared.');
  });

  it('offers no "Clear key" when the backend did not name a source', async () => {
    mocks.getSettings.mockResolvedValue(settings('none'));

    const view = render(<ProviderPanel />);

    await waitFor(() =>
      expect(view.container.querySelector('.provider-key-source')).toBeTruthy(),
    );
    expect(view.container.querySelector('.provider-key-clear')).toBeNull();
  });
});
