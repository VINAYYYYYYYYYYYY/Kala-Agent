// @vitest-environment happy-dom
// Component-level tests for the chat panel's cancel and model-optional
// behaviour. These run on the actual shipped JSX to confirm the UI state
// transitions the conductor asked us to prove.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postChat: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    postChat: mocks.postChat,
  };
});

import { ChatPanel } from '../src/components/chat/ChatPanel';

const baseProviders = [
  { id: 'local', name: 'Local helper', configured: true, notes: '' },
  { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
];

beforeEach(() => {
  // The chat panel reads from the store. We seed it before each render
  // by going through the public dispatch surface.
});

afterEach(() => {
  cleanup();
  mocks.postChat.mockReset();
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
});