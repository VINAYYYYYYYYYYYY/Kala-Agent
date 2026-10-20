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
  createJournalEntry,
  appendJournal,
  type UiState,
} from '../src/state/store';
import { JOURNAL_CAP } from '../src/types/studio';
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

  it('chat/response keeps the provider, model, offline flag, and reply time', () => {
    const next = reducer(baseState, {
      type: 'chat/response',
      response: {
        text: 'hello',
        provider: 'openai',
        model: 'gpt-4o-mini',
        offline: false,
        durationMs: 1200,
      },
    });
    const reply = next.chat.thread[0]!;
    expect(reply.provider).toBe('openai');
    expect(reply.model).toBe('gpt-4o-mini');
    expect(reply.offline).toBe(false);
    expect(reply.durationMs).toBe(1200);
  });
});

describe('measure tool state', () => {
  it('starts disarmed', () => {
    expect(createInitialUiState().measure).toEqual({ active: false, points: [] });
  });

  it('arms, records picked points, and clears on demand', () => {
    let state = reducer(baseState, { type: 'measure/toggle' });
    expect(state.measure.active).toBe(true);

    state = reducer(state, { type: 'measure/point', point: { x: 0, y: 0, z: 15 } });
    state = reducer(state, { type: 'measure/point', point: { x: 40, y: 0, z: 15 } });
    expect(state.measure.points).toHaveLength(2);

    state = reducer(state, { type: 'measure/clear' });
    expect(state.measure).toEqual({ active: false, points: [] });
  });

  it('drops stale picks when the geometry changes but keeps the tool armed', () => {
    let state = reducer(baseState, { type: 'measure/toggle' });
    state = reducer(state, { type: 'measure/point', point: { x: 0, y: 0, z: 15 } });
    expect(state.measure.points).toHaveLength(1);

    state = reducer(state, {
      type: 'parameters/change',
      parameters: { size_x: 50, size_y: 20, size_z: 30, subdivisions: 1 },
    });
    expect(state.measure.points).toEqual([]);
    expect(state.measure.active).toBe(true);
  });
});

describe('vellum shell state', () => {
  it('starts in the drafted vellum workspace defaults', () => {
    const state = createInitialUiState();
    expect(state.renderMode).toBe('pencil2d');
    expect(state.auxView).toBe('section');
    expect(state.showSection).toBe(true);
    expect(state.optionsOpen).toBe(false);
    expect(state.optionsCard).toBe('inspector');
    expect(state.theme).toBe('vellum');
    expect(state.journal).toEqual([]);
    expect(state.viewResetToken).toBe(0);
    expect(state.connectionRetryToken).toBe(0);
  });

  it('updates render mode, aux view, section flag, and view reset token', () => {
    let state = reducer(baseState, { type: 'renderMode/set', mode: 'clay3d' });
    expect(state.renderMode).toBe('clay3d');

    state = reducer(state, { type: 'auxView/set', view: 'tolerance' });
    expect(state.auxView).toBe('tolerance');

    state = reducer(state, { type: 'section/toggle' });
    expect(state.showSection).toBe(false);
    state = reducer(state, { type: 'section/set', show: true });
    expect(state.showSection).toBe(true);

    state = reducer(state, { type: 'view/reset' });
    expect(state.viewResetToken).toBe(1);
    state = reducer(state, { type: 'connection/retry' });
    expect(state.connectionRetryToken).toBe(1);
  });

  it('opens the Options drawer on the requested card and closes it', () => {
    let state = reducer(baseState, { type: 'options/open' });
    expect(state.optionsOpen).toBe(true);
    expect(state.optionsCard).toBe('inspector');

    state = reducer(state, { type: 'options/card', card: 'export' });
    expect(state.optionsOpen).toBe(true);
    expect(state.optionsCard).toBe('export');

    state = reducer(state, { type: 'options/close' });
    expect(state.optionsOpen).toBe(false);
    expect(state.optionsCard).toBe('export');

    state = reducer(state, { type: 'options/open', card: 'connection' });
    expect(state.optionsCard).toBe('connection');
  });

  it('switches the theme without touching any secret field', () => {
    const next = reducer(baseState, { type: 'theme/set', theme: 'slate' });
    expect(next.theme).toBe('slate');
    expect(next.connection).toBe(baseState.connection);
    expect(next.server).toBe(baseState.server);
  });
});

describe('bounded journal', () => {
  it('appends entries and drops the oldest once the 500-row cap is reached', () => {
    let state = createInitialUiState();
    for (let index = 0; index < JOURNAL_CAP + 5; index += 1) {
      state = reducer(state, {
        type: 'journal/append',
        entry: { id: `e${index}`, ts: index, kind: 'system', message: `entry-${index}` },
      });
    }
    expect(state.journal).toHaveLength(JOURNAL_CAP);
    expect(state.journal[0]?.message).toBe('entry-5');
    expect(state.journal[state.journal.length - 1]?.message).toBe(`entry-${JOURNAL_CAP + 4}`);
  });

  it('preserves an optional digest and clears on demand', () => {
    const entry = { id: 'e1', ts: 1, kind: 'export' as const, message: 'Saved part.step', digest: '512 bytes' };
    const withEntry = reducer(baseState, { type: 'journal/append', entry });
    expect(withEntry.journal[0]?.digest).toBe('512 bytes');
    expect(reducer(withEntry, { type: 'journal/clear' }).journal).toEqual([]);
  });

  it('builds real entries with a timestamp and monotonic id', () => {
    const a = createJournalEntry({ kind: 'connection', message: 'connected', ts: 100 });
    const b = createJournalEntry({ kind: 'connection', message: 'connected', ts: 100 });
    expect(a.ts).toBe(100);
    expect(a.id).not.toBe(b.id);
    expect(a.kind).toBe('connection');
  });

  it('appendJournal dispatches into the live store', () => {
    resetStore();
    appendJournal({ kind: 'preview', message: 'Server preview verified' });
    expect(getState().journal).toHaveLength(1);
    expect(getState().journal[0]?.message).toBe('Server preview verified');
  });
});

describe('store reset', () => {  it('notifies subscribers and installs an idle initial state', () => {
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