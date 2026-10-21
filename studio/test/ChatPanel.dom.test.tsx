// @vitest-environment happy-dom
// Component-level tests for the chat panel's cancel and model-optional
// behaviour. These run on the actual shipped JSX to confirm the UI state
// transitions the conductor asked us to prove.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postChat: vi.fn(),
  getCadProviders: vi.fn(),
  getProviderModels: vi.fn(),
  getSettings: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    postChat: mocks.postChat,
    getCadProviders: mocks.getCadProviders,
    getProviderModels: mocks.getProviderModels,
    getSettings: mocks.getSettings,
  };
});

import { ChatPanel } from '../src/components/chat/ChatPanel';
import { StudioApiError, StudioTimeoutError } from '../src/api/client';
import { dispatch, resetStore } from '../src/state/store';
import { cadProviders } from './designFixtures';
import { pickProvider } from './providerFixtures';

const baseProviders = [
  { id: 'local', name: 'Local helper', configured: true, notes: '' },
  { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
];

beforeEach(() => {
  // The chat panel reads from the store. We seed it before each render
  // by going through the public dispatch surface.
  resetStore();
  mocks.getCadProviders.mockResolvedValue(cadProviders);
  mocks.getProviderModels.mockResolvedValue({
    provider: 'local',
    source: 'static',
    fetched_at: null,
    models: [],
  });
  mocks.getSettings.mockResolvedValue({
    credentials: [],
    providers: [],
    offline: false,
  });
});

afterEach(() => {
  cleanup();
  resetStore();
  mocks.postChat.mockReset();
  mocks.getCadProviders.mockReset();
  mocks.getProviderModels.mockReset();
  mocks.getSettings.mockReset();
});

describe('ChatPanel state transitions', () => {
  it('enables Send for the local helper without a model', async () => {
    // Pre-populate the store.
    const { dispatch } = await import('../src/state/store');
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });

    const view = render(<ChatPanel />);

    // Select local helper.
    pickProvider(view.container, 'local');

    const prompt = view.container.querySelector('textarea') as HTMLTextAreaElement;
    fireEvent.change(prompt, { target: { value: 'summarise this' } });

    // Model input is empty, but the local helper ignores model so the
    // Send button is enabled.
    const sendButton = view.container.querySelector('button.chat-panel-send') as HTMLButtonElement;
    await waitFor(() => expect(sendButton.disabled).toBe(false));
  });

  it('cancelling an in-flight request clears inFlight and shows cancelled state', async () => {
    const { dispatch, getState } = await import('../src/state/store');
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });

    // A postChat that never resolves until we abort it.
    mocks.postChat.mockImplementation((_connection, _payload, signal) => new Promise((_resolve, reject) => {
      // When the operator cancels we reject with an AbortError so the
      // catch path in ChatPanel runs and inFlight clears.
      if (signal) {
        const onAbort = () => {
          const err = new Error('aborted');
          err.name = 'AbortError';
          reject(err);
        };
        if (signal.aborted) onAbort();
        else signal.addEventListener('abort', onAbort, { once: true });
      }
    }));

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    const prompt = view.container.querySelector('textarea') as HTMLTextAreaElement;
    fireEvent.change(prompt, { target: { value: 'long task' } });
    const sendButton = view.container.querySelector('button.chat-panel-send') as HTMLButtonElement;
    fireEvent.click(sendButton);

    // After click, inFlight is true. The Cancel button appears.
    await waitFor(() => {
      expect(view.container.querySelector('button.chat-panel-cancel')).toBeTruthy();
    });
    expect(getState().chat.inFlight).toBe(true);

    // Click cancel.
    const cancelButton = view.container.querySelector('button.chat-panel-cancel') as HTMLButtonElement;
    fireEvent.click(cancelButton);

    // Wait for inFlight to be cleared.
    await waitFor(() => {
      expect(getState().chat.inFlight).toBe(false);
    }, { timeout: 5000 });

    // After cancel the in-flight state is cleared. We type a new
    // prompt to verify the button re-enables.
    fireEvent.change(prompt, { target: { value: 'follow-up' } });
    await waitFor(() => {
      expect(sendButton.disabled).toBe(false);
    });
  });

  it('sends the selected CAD target as cad_provider', async () => {
    const { dispatch } = await import('../src/state/store');
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
    dispatch({ type: 'server/capabilities', designAvailable: true });
    dispatch({ type: 'server/cad-providers', providers: cadProviders.providers });
    dispatch({ type: 'cad/provider-set', provider: 'freecad' });
    mocks.postChat.mockResolvedValue({
      text: 'Compiled with FreeCAD.',
      provider: 'local',
      model: null,
      offline: true,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    const prompt = view.container.querySelector('textarea') as HTMLTextAreaElement;
    fireEvent.change(prompt, { target: { value: 'make me a bracket' } });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    await waitFor(() => expect(mocks.postChat).toHaveBeenCalledTimes(1));
    const payload = mocks.postChat.mock.calls[0]![1] as Record<string, unknown>;
    expect(payload['cad_provider']).toBe('freecad');
  });

  it('sends cad_provider null when no CAD target has been chosen', async () => {
    const { dispatch } = await import('../src/state/store');
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
    dispatch({ type: 'server/capabilities', designAvailable: true });
    dispatch({ type: 'server/cad-providers', providers: cadProviders.providers });
    mocks.postChat.mockResolvedValue({
      text: 'Choose a CAD provider before I build this design.',
      provider: 'local',
      model: null,
      offline: true,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    const prompt = view.container.querySelector('textarea') as HTMLTextAreaElement;
    fireEvent.change(prompt, { target: { value: 'make me a bracket' } });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    await waitFor(() => expect(mocks.postChat).toHaveBeenCalledTimes(1));
    const payload = mocks.postChat.mock.calls[0]![1] as Record<string, unknown>;
    expect(payload['cad_provider']).toBeNull();
  });

  it('renders the reason and available providers of a provider_unavailable refusal', async () => {
    const { dispatch } = await import('../src/state/store');
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
    mocks.postChat.mockRejectedValue(
      new StudioApiError(503, {
        code: 'provider_unavailable',
        message: 'the solidedge CAD provider is not available',
        details: [
          'the solidedge CAD provider is not available',
          'reason: Solid Edge build requires the Windows bridge (not reachable)',
          'available: opencascade, freecad',
        ],
      }),
    );

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    const error = await waitFor(() => {
      const node = view.container.querySelector('.chat-panel-error');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(error.textContent).toContain('provider_unavailable');
    expect(error.textContent).toContain('Windows bridge');
    expect(error.textContent).toContain('available: opencascade, freecad');
  });
});

describe('the composer route', () => {
  const subscriptionProviders = [
    ...baseProviders,
    { id: 'claude_code', name: 'Claude Code CLI', configured: true, notes: '' },
    { id: 'opencode_cli', name: 'OpenCode CLI', configured: true, notes: '' },
  ];

  function seed(providers = baseProviders): void {
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
  }

  it('sends the model the operator picked as the model of the turn', async () => {
    seed();
    mocks.getProviderModels.mockResolvedValue({
      provider: 'openai',
      source: 'openai',
      fetched_at: '2026-10-20T12:00:00Z',
      models: [{ id: 'gpt-4o-mini', name: 'GPT-4o mini', free: false }],
    });
    mocks.postChat.mockResolvedValue({
      text: 'Sized.',
      provider: 'openai',
      model: 'gpt-4o-mini',
      offline: false,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'openai');
    await waitFor(() => expect(mocks.getProviderModels).toHaveBeenCalled());

    fireEvent.focus(view.container.querySelector<HTMLInputElement>('#chat-model-input')!);
    fireEvent.click(view.container.querySelector<HTMLElement>('[data-model-id="gpt-4o-mini"]')!);
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    fireEvent.click(view.container.querySelector<HTMLInputElement>('.chat-panel-consent input')!);

    await waitFor(() =>
      expect(view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false),
    );
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    await waitFor(() => expect(mocks.postChat).toHaveBeenCalledTimes(1));
    const payload = mocks.postChat.mock.calls[0]![1] as Record<string, unknown>;
    expect(payload['model']).toBe('gpt-4o-mini');
    expect(payload['provider']).toBe('openai');
    expect(payload['allow_remote']).toBe(true);
  });

  it('remembers the model of each provider separately', async () => {
    seed();
    const view = render(<ChatPanel />);

    pickProvider(view.container, 'openai');
    fireEvent.change(view.container.querySelector<HTMLInputElement>('#chat-model-input')!, {
      target: { value: 'gpt-4o-mini' },
    });
    pickProvider(view.container, 'local');
    expect(view.container.querySelector<HTMLInputElement>('#chat-model-input')!.value).toBe('');

    pickProvider(view.container, 'openai');
    expect(view.container.querySelector<HTMLInputElement>('#chat-model-input')!.value).toBe('gpt-4o-mini');
  });

  it('lets a subscription CLI send without a model id and still needs the consent stamp', async () => {
    seed(subscriptionProviders);
    mocks.postChat.mockResolvedValue({
      text: 'Built.',
      provider: 'claude_code',
      model: null,
      offline: false,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'claude_code');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });

    const send = view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!;
    // The consent stamp is still required: the CLI talks to a remote service.
    await waitFor(() => expect(send.disabled).toBe(true));
    fireEvent.click(view.container.querySelector<HTMLInputElement>('.chat-panel-consent input')!);
    await waitFor(() => expect(send.disabled).toBe(false));

    fireEvent.click(send);
    await waitFor(() => expect(mocks.postChat).toHaveBeenCalledTimes(1));
    const payload = mocks.postChat.mock.calls[0]![1] as Record<string, unknown>;
    expect(payload['provider']).toBe('claude_code');
    expect(payload['model']).toBeNull();
    expect(payload['allow_remote']).toBe(true);
  });

  it('clears the consent stamp when the route changes', async () => {
    seed();
    const view = render(<ChatPanel />);

    pickProvider(view.container, 'openai');
    const consent = view.container.querySelector<HTMLInputElement>('.chat-panel-consent input')!;
    fireEvent.click(consent);
    expect(consent.checked).toBe(true);

    pickProvider(view.container, 'local');
    // The local route needs no consent, so the stamp is gone with it.
    expect(view.container.querySelector('.chat-panel-consent')).toBeNull();
  });

  it('opens the API-key section from the model picker of an unconfigured route', async () => {
    seed([{ id: 'openrouter', name: 'OpenRouter', configured: false, notes: '' }]);
    const view = render(<ChatPanel />);

    expect(view.container.querySelector('.chat-panel-settings')).toBeNull();
    pickProvider(view.container, 'openrouter');

    await waitFor(() =>
      expect(view.container.textContent).toContain('is not configured server-side'),
    );
    fireEvent.click(view.container.querySelector<HTMLButtonElement>('.model-picker-action')!);

    expect(view.container.querySelector('.chat-panel-settings')).toBeTruthy();
  });
});

describe('the live elapsed clock', () => {
  function seed(providers = baseProviders): void {
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
  }

  it('counts the turn up while it is in flight and stops when the reply lands', async () => {
    vi.useFakeTimers();
    try {
      seed();
      let release: (value: unknown) => void = () => {};
      mocks.postChat.mockImplementation(
        () => new Promise((resolve) => {
          release = resolve;
        }),
      );

      const view = render(<ChatPanel />);
      pickProvider(view.container, 'local');
      fireEvent.change(view.container.querySelector('textarea')!, {
        target: { value: 'long task' },
      });
      fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

      // The clock is there while the turn runs.
      const clock = view.container.querySelector('.chat-panel-elapsed-live');
      expect(clock).toBeTruthy();
      expect(clock!.textContent).toBe('0 ms');

      // It ticks once a second.
      await vi.advanceTimersByTimeAsync(3000);
      expect(view.container.querySelector('.chat-panel-elapsed-live')!.textContent).toBe('3.0 s');

      // The reply ends the clock: no timer is left behind.
      release({ text: 'Sized.', provider: 'local', model: null, offline: true });
      await vi.advanceTimersByTimeAsync(1);
      expect(view.container.querySelector('.chat-panel-elapsed-live')).toBeNull();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('cleans its timer up when the panel is unmounted mid-turn', async () => {
    vi.useFakeTimers();
    try {
      seed();
      mocks.postChat.mockImplementation(() => new Promise(() => {}));

      const view = render(<ChatPanel />);
      pickProvider(view.container, 'local');
      fireEvent.change(view.container.querySelector('textarea')!, {
        target: { value: 'long task' },
      });
      fireEvent.click(view.container.querySelector('button.chat-panel-send')!);
      expect(view.container.querySelector('.chat-panel-elapsed-live')).toBeTruthy();

      view.unmount();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('reads the same way as the post-reply elapsed chip', async () => {
    // The live clock and the chip after the reply must not disagree: the
    // chip of a turn that took 1.2 s reads "(1.2 s)".
    seed();
    mocks.postChat.mockResolvedValue({
      text: 'Sized.',
      provider: 'local',
      model: null,
      offline: true,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'quick task' },
    });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    const chip = await waitFor(() => {
      const node = view.container.querySelector('.chat-panel-message-elapsed');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(chip.textContent).toMatch(/^\(\d+(\.\d+)? (ms|s)\)$/);
  });
});

describe('the client deadline of a chat turn', () => {
  function seed(): void {
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers: baseProviders, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
  }

  it('reports an expired deadline as a timeout and not as a cancellation', async () => {
    seed();
    mocks.postChat.mockRejectedValue(new StudioTimeoutError('/api/studio/chat'));

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    const error = await waitFor(() => {
      const node = view.container.querySelector('.chat-panel-error');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(error.getAttribute('role')).toBe('alert');
    expect(error.textContent).toContain('timeout: /api/studio/chat');
    // A timeout is NOT a cancellation: the cancel line stays away.
    expect(view.container.textContent).not.toContain('the backend stopped the turn');
  });

  it('still reports an operator cancellation as a cancellation', async () => {
    seed();
    mocks.postChat.mockImplementation((_connection, _payload, signal) => new Promise((_resolve, reject) => {
      if (!signal) return;
      const onAbort = (): void => {
        const error = new Error('aborted');
        error.name = 'AbortError';
        reject(error);
      };
      if (signal.aborted) onAbort();
      else signal.addEventListener('abort', onAbort, { once: true });
    }));

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'local');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    await waitFor(() =>
      expect(view.container.querySelector('button.chat-panel-cancel')).toBeTruthy(),
    );
    fireEvent.click(view.container.querySelector('button.chat-panel-cancel')!);

    await waitFor(() =>
      expect(view.container.textContent).toContain(
        'Cancelled — the backend stopped the turn.',
      ),
    );
    expect(view.container.querySelector('.chat-panel-error')).toBeNull();
  });
});

describe('the reply badge and the CLI default model', () => {
  function seed(providers = baseProviders): void {
    dispatch({ type: 'connection/ready', connection: {
      baseUrl: 'http://127.0.0.1:8765',
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    } });
    dispatch({ type: 'server/providers', providers, health: 'ok', healthMessage: 'ok', version: '0.1.0', stepAvailable: false });
  }

  const cliProviders = [
    ...baseProviders,
    { id: 'claude_code', name: 'Claude Code CLI', configured: true, notes: '' },
    { id: 'codex_cli', name: 'Codex CLI', configured: true, notes: '' },
    { id: 'opencode_cli', name: 'OpenCode CLI', configured: true, notes: '' },
  ];

  async function replyWith(model: string | null, provider = 'codex_cli'): Promise<HTMLElement> {
    seed(cliProviders);
    mocks.postChat.mockResolvedValue({
      text: 'Built.',
      provider,
      model,
      offline: false,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'codex_cli');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    // The CLI route needs the consent stamp.
    const consent = view.container.querySelector<HTMLInputElement>('.chat-panel-consent input');
    if (consent) fireEvent.click(consent);
    await waitFor(() =>
      expect(view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false),
    );
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    return waitFor(() => {
      const badge = view.container.querySelector<HTMLElement>('.chat-panel-message-badge');
      expect(badge).toBeTruthy();
      return badge!;
    });
  }

  it('says honestly that the CLI ran with its own default model', async () => {
    const badge = await replyWith('default');
    expect(badge.textContent).toBe('Codex CLI default model (chosen by Codex)');
    // It never invents a model name.
    expect(badge.textContent).not.toContain('gpt-');
  });

  it('says the same when the backend reported no model at all', async () => {
    const badge = await replyWith(null);
    expect(badge.textContent).toBe('Codex CLI default model (chosen by Codex)');
  });

  it('names the model the operator pinned', async () => {
    seed(cliProviders);
    mocks.postChat.mockResolvedValue({
      text: 'Built.',
      provider: 'codex_cli',
      model: 'gpt-5-codex',
      offline: false,
    });

    const view = render(<ChatPanel />);
    pickProvider(view.container, 'codex_cli');
    fireEvent.change(view.container.querySelector('textarea')!, {
      target: { value: 'make me a bracket' },
    });
    const consent = view.container.querySelector<HTMLInputElement>('.chat-panel-consent input');
    if (consent) fireEvent.click(consent);
    await waitFor(() =>
      expect(view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false),
    );
    fireEvent.click(view.container.querySelector('button.chat-panel-send')!);

    const badge = await waitFor(() => {
      const node = view.container.querySelector('.chat-panel-message-badge');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(badge.textContent).toBe('codex_cli · gpt-5-codex');
  });
});
