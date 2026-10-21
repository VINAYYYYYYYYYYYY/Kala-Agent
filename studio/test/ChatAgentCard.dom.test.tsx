// @vitest-environment happy-dom
// The agent card metadata: the provider/model badge the backend reported
// and the client-measured send-to-reply time.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postChat: vi.fn(),
  getProviderModels: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    postChat: mocks.postChat,
    getProviderModels: mocks.getProviderModels,
  };
});

import { ChatPanel } from '../src/components/chat/ChatPanel';
import { dispatch, resetStore } from '../src/state/store';
import { pickProvider } from './providerFixtures';

const BASE_URL = 'http://127.0.0.1:8765';

beforeEach(() => {
  resetStore();
  dispatch({
    type: 'connection/ready',
    connection: {
      baseUrl: BASE_URL,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    },
  });
  dispatch({
    type: 'server/providers',
    providers: [
      { id: 'local', name: 'Local helper', configured: true, notes: '' },
      { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
    ],
    health: 'ok',
    healthMessage: 'ok',
    version: '0.1.0',
    stepAvailable: false,
  });
  mocks.getProviderModels.mockResolvedValue({
    provider: 'local',
    source: 'static',
    fetched_at: null,
    models: [],
  });
});

afterEach(() => {
  cleanup();
  mocks.postChat.mockReset();
  mocks.getProviderModels.mockReset();
  vi.restoreAllMocks();
});

function badge(container: HTMLElement): HTMLElement | null {
  return container.querySelector<HTMLElement>('.chat-panel-message-badge');
}

describe('agent card badge', () => {
  it('shows the offline local route and the measured reply time', async () => {
    // A controlled clock makes the client-side measurement deterministic.
    let now = 1_000_000;
    vi.spyOn(Date, 'now').mockImplementation(() => now);
    mocks.postChat.mockImplementation(() => {
      now += 1200;
      return Promise.resolve({
        text: 'The nominal envelope is 40 × 20 × 30 mm.',
        provider: 'local',
        model: null,
        offline: true,
      });
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    const textarea = view.container.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    fireEvent.change(textarea, { target: { value: 'size the bracket' } });

    const send = view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!;
    await waitFor(() => expect(send.disabled).toBe(false));
    fireEvent.click(send);

    await waitFor(() => expect(badge(view.container)).toBeTruthy());
    const agentBadge = badge(view.container)!;
    expect(agentBadge.textContent).toBe('local · offline');
    expect(agentBadge.getAttribute('data-offline')).toBe('yes');

    const elapsed = view.container.querySelector('.chat-panel-message-elapsed')!;
    expect(elapsed.textContent).toBe('(1.2 s)');
  });

  it('shows the provider and model the backend answered with', () => {
    dispatch({
      type: 'chat/response',
      response: {
        text: 'Sized to 40 × 20 × 30 mm.',
        provider: 'openai',
        model: 'gpt-4o-mini',
        offline: false,
        durationMs: 2500,
      },
    });

    const view = render(<ChatPanel />);
    const agentBadge = badge(view.container)!;
    expect(agentBadge.textContent).toBe('openai · gpt-4o-mini');
    expect(agentBadge.getAttribute('data-offline')).toBe('no');
    expect(view.container.querySelector('.chat-panel-message-elapsed')?.textContent).toBe('(2.5 s)');
  });

  it('shows a fast local reply in milliseconds instead of a rounded zero', () => {
    dispatch({
      type: 'chat/response',
      response: {
        text: 'Local answer.',
        provider: 'local',
        model: null,
        offline: true,
        durationMs: 43,
      },
    });
    const view = render(<ChatPanel />);
    expect(view.container.querySelector('.chat-panel-message-elapsed')?.textContent).toBe('(43 ms)');
  });

  it('never shows a badge on the operator turn', () => {
    dispatch({
      type: 'chat/send',
      request: { provider: 'local', model: null, prompt: 'hello', allowRemote: false },
    });
    const view = render(<ChatPanel />);
    expect(badge(view.container)).toBeNull();
    expect(view.container.querySelector('.chat-panel-message-elapsed')).toBeNull();
  });
});
