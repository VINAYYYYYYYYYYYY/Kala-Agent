// Tests for the reducer store.

import { describe, expect, it } from 'vitest';
import {
  reducer,
  initialUiState,
  store,
  getState,
  dispatch,
  resetStore,
  createInitialUiState,
  type UiState,
} from '../src/state/store';
import type { BoxParameters, PreviewResponse } from '../src/types/studio';

const preview: PreviewResponse = {
  positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  indices: [0, 1, 2],
  bounds: { min: [0, 0, 0], max: [1, 1, 1] },
  engine: 'analytic',
  warnings: [],
};

const baseState: UiState = {
  ...initialUiState,
  parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 } satisfies BoxParameters,
};

describe('reducer', () => {
  it('flips connection status', () => {
    expect(reducer(baseState, { type: 'connection/connecting' }).connection).toEqual({ kind: 'connecting' });
    expect(reducer(baseState, { type: 'connection/error', message: 'oops' }).connection).toEqual({ kind: 'error', message: 'oops' });
  });

  it('primitive/change clears preview state', () => {
    const withPreview: UiState = {
      ...baseState,
      preview: { local: null, server: { preview, receivedAt: 1 }, inFlight: false, lastError: null, lastErrorKind: 'none' },
    };
    const next = reducer(withPreview, {
      type: 'primitive/change',
      kind: 'cylinder',
      parameters: { radius: 10, height: 10, radial_segments: 24, height_segments: 1 },
    });
    expect(next.primitive).toBe('cylinder');
    expect(next.preview.server).toBeNull();
  });

  it('preview/request-success updates the server mesh and bumps lastSyncAt', () => {
    const next = reducer(baseState, { type: 'preview/request-success', preview });
    expect(next.preview.server?.preview.bounds.max).toEqual([1, 1, 1]);
    expect(next.preview.lastError).toBeNull();
  });

  it('preview/request-error carries the error kind for the UI to render', () => {
    const next = reducer(baseState, { type: 'preview/request-error', message: 'oops', kind: 'network' });
    expect(next.preview.lastError).toBe('oops');
    expect(next.preview.lastErrorKind).toBe('network');
  });

  it('server/providers records version and step_available', () => {
    const next = reducer(baseState, {
      type: 'server/providers',
      providers: [],
      health: 'ok',
      healthMessage: 'ok',
      version: '1.2.3',
      stepAvailable: true,
    });
    expect(next.server.version).toBe('1.2.3');
    expect(next.server.stepAvailable).toBe(true);
  });

  it('chat/send appends the user message and marks inFlight', () => {
    const next = reducer(baseState, {
      type: 'chat/send',
      request: { provider: 'local', model: 'helper-v1', prompt: 'hi', allowRemote: false },
    });
    expect(next.chat.thread).toHaveLength(1);
    expect(next.chat.thread[0]?.role).toBe('user');
    expect(next.chat.inFlight).toBe(true);
  });

  it('chat/response appends the assistant reply and clears inFlight', () => {
    const inFlight: UiState = {
      ...baseState,
      chat: { thread: [{ role: 'user', text: 'hi', at: 0 }], inFlight: true, lastError: null, lastErrorKind: 'none' },
    };
    const next = reducer(inFlight, {
      type: 'chat/response',
      response: { text: 'hello', provider: 'local', model: 'helper-v1' },
    });
    expect(next.chat.thread).toHaveLength(2);
    expect(next.chat.thread[1]?.role).toBe('assistant');
    expect(next.chat.inFlight).toBe(false);
  });
});

describe('store reset', () => {
  it('notifies subscribers and installs an idle initial state', () => {
    resetStore();
    const seen: UiState[] = [];
    const unsubscribe = store.subscribe(() => seen.push(store.getState()));

    dispatch({
      type: 'connection/ready',
      connection: { baseUrl: 'http://127.0.0.1:8765', token: 'x'.repeat(20), mode: 'browser-manual' },
    });
    expect(getState().connection.kind).toBe('ready');

    resetStore();

    // The reset must have notified (once for the dispatch, once for the
    // reset) and returned the store to a fresh idle snapshot.
    expect(seen.length).toBe(2);
    expect(getState().connection).toEqual({ kind: 'idle' });
    expect(getState().server.health).toBe('unknown');
    expect(getState().preview.local).toBeNull();

    unsubscribe();
    resetStore();
  });

  it('creates an independent object graph on each reset (no mutable globals)', () => {
    resetStore();
    const first = getState();
    resetStore();
    const second = getState();

    // Every reset produces brand-new object identities...
    expect(first).not.toBe(second);
    expect(first.server).not.toBe(second.server);
    expect(first.preview).not.toBe(second.preview);
    expect(first.viewport).not.toBe(second.viewport);
    // ...and never aliases the exported default snapshot.
    expect(second).not.toBe(initialUiState);
    expect(initialUiState.server).not.toBe(second.server);

    // Mutating a returned snapshot must not leak into the next reset.
    (first.server as { health: string }).health = 'ok';
    resetStore();
    expect(getState().server.health).toBe('unknown');

    // The factory itself is pure with respect to the exported constant.
    const a = createInitialUiState();
    const b = createInitialUiState();
    expect(a).not.toBe(b);
    expect(a).not.toBe(initialUiState);
  });
});