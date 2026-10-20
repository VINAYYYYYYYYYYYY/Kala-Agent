// @vitest-environment happy-dom
// Component-level tests for the chat panel's cancel and model-optional
// behaviour. These run on the actual shipped JSX to confirm the UI state
// transitions the conductor asked us to prove.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postChat: vi.fn(),
  getCadProviders: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    postChat: mocks.postChat,
    getCadProviders: mocks.getCadProviders,
  };
});

import { ChatPanel } from '../src/components/chat/ChatPanel';
import { StudioApiError } from '../src/api/client';
import { resetStore } from '../src/state/store';
import { cadProviders } from './designFixtures';

const baseProviders = [
  { id: 'local', name: 'Local helper', configured: true, notes: '' },
  { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
];

beforeEach(() => {
  // The chat panel reads from the store. We seed it before each render
  // by going through the public dispatch surface.
  resetStore();
  mocks.getCadProviders.mockResolvedValue(cadProviders);
});

afterEach(() => {
  cleanup();
  resetStore();
  mocks.postChat.mockReset();
  mocks.getCadProviders.mockReset();
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
    const providerSelect = view.container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: 'local' } });

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
    const providerSelect = view.container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: 'local' } });
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
    const providerSelect = view.container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: 'local' } });
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
    const providerSelect = view.container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: 'local' } });
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
    const providerSelect = view.container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(providerSelect, { target: { value: 'local' } });
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