// Real regression coverage for the preview request timeout / cancel
// lifecycle. These tests drive the ACTUAL `createPreviewController`
// against the ACTUAL store and a stubbed `fetch`, using fake timers so
// the controller's own 8-second timeout and 220ms debounce are exercised
// end to end. The previous tests only asserted "does not throw"; they
// never checked that the store's `inFlight` flag or `lastErrorKind`
// settled, which is how the stuck-loading bug slipped through.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPreviewController } from '../src/state/previewController';
import { getState, resetStore } from '../src/state/store';
import type { StudioConnection, StudioParameters, PreviewResponse } from '../src/types/studio';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

const DEBOUNCE_MS = 220;
const REQUEST_TIMEOUT_MS = 8_000;

function boxParams(size_x: number): StudioParameters {
  return { size_x, size_y: 20, size_z: 30, subdivisions: 1 };
}

function okResponse(): Response {
  const preview: PreviewResponse = {
    positions: new Array(9).fill(0),
    normals: new Array(9).fill(0),
    indices: [0, 1, 2],
    bounds: { min: [-10, -10, -10], max: [10, 10, 10] },
    engine: 'analytic',
    warnings: [],
  };
  return new Response(JSON.stringify(preview), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

interface CapturedCall {
  signal: AbortSignal | null;
  resolve(value: Response): void;
  reject(cause: unknown): void;
}

// A fetch stub that rejects with the browser's AbortError when the
// signal aborts. This mirrors real `fetch` behaviour, so the controller's
// timeout (which aborts the signal) is what actually completes the
// request.
function abortAwareFetch(): CapturedCall[] {
  const calls: CapturedCall[] = [];
  vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
    return new Promise<Response>((resolve, reject) => {
      const signal = init?.signal ?? null;
      calls.push({ signal, resolve, reject });
      if (signal) {
        const onAbort = () => reject(new DOMException('aborted', 'AbortError'));
        if (signal.aborted) onAbort();
        else signal.addEventListener('abort', onAbort, { once: true });
      }
    });
  });
  return calls;
}

// A fetch stub that IGNORES the abort signal entirely: it stands in for
// a non-cooperative backend / browser that resolves after cancellation.
function nonCooperativeFetch(): CapturedCall[] {
  const calls: CapturedCall[] = [];
  vi.stubGlobal('fetch', () => {
    return new Promise<Response>((resolve, reject) => {
      calls.push({ signal: null, resolve, reject });
    });
  });
  return calls;
}

async function settle(times = 12): Promise<void> {
  for (let i = 0; i < times; i += 1) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  resetStore();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  resetStore();
});

describe('preview timeout lifecycle', () => {
  it('completes the 8s timeout: clears loading, records a timeout error, then retries', async () => {
    const calls = abortAwareFetch();
    const controller = createPreviewController(connection);

    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    expect(calls).toHaveLength(1);
    expect(getState().preview.inFlight).toBe(true);

    // The controller's own timeout fires and aborts the request. Even
    // though the abort surfaces as a cancellation, the store must stop
    // loading and record a timeout.
    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 5);
    await settle();

    expect(calls[0]?.signal?.aborted).toBe(true);
    expect(getState().preview.inFlight).toBe(false);
    expect(getState().preview.lastErrorKind).toBe('timeout');
    expect(getState().preview.lastError).toBe('request timed out');

    // A retry must be able to succeed and clear the timeout.
    vi.stubGlobal('fetch', vi.fn(async () => okResponse()));
    controller.trigger('box', boxParams(20));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    await settle();

    expect(getState().preview.inFlight).toBe(false);
    expect(getState().preview.lastErrorKind).toBe('none');
    expect(getState().preview.lastError).toBeNull();
    expect(getState().preview.server).not.toBeNull();
    controller.dispose();
  });

  it('does not leave the store loading when a timeout rejection is classified as cancelled', async () => {
    // The precise regression: an aborted fetch becomes StudioAbortError
    // (kind "cancelled"); the old code returned before handling the
    // timeout, leaving `inFlight` stuck true forever.
    const calls = abortAwareFetch();
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);

    await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS + 1);
    await settle();

    expect(calls).toHaveLength(1);
    expect(getState().preview.inFlight).toBe(false);
    controller.dispose();
  });
});

describe('preview cancel lifecycle', () => {
  it('explicit cancel clears loading quietly and a stale completion cannot overwrite a newer preview', async () => {
    const calls = nonCooperativeFetch();
    const controller = createPreviewController(connection);

    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    expect(calls).toHaveLength(1);
    expect(getState().preview.inFlight).toBe(true);

    controller.cancel();
    expect(getState().preview.inFlight).toBe(false);
    expect(getState().preview.lastError).toBeNull();
    expect(getState().preview.lastErrorKind).toBe('none');

    // The aborted request is non-cooperative and resolves anyway. It must
    // be dropped by the generation guard, not overwrite the store.
    calls[0]?.resolve(okResponse());
    await settle();
    expect(getState().preview.server).toBeNull();

    // A brand-new preview can then complete normally.
    vi.stubGlobal('fetch', vi.fn(async () => okResponse()));
    controller.trigger('box', boxParams(30));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    await settle();
    expect(getState().preview.inFlight).toBe(false);
    expect(getState().preview.server).not.toBeNull();
    controller.dispose();
  });

  it('dispose clears the loading flag without surfacing an error', async () => {
    abortAwareFetch();
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    expect(getState().preview.inFlight).toBe(true);

    controller.dispose();
    expect(getState().preview.inFlight).toBe(false);
    expect(getState().preview.lastError).toBeNull();
    expect(getState().preview.lastErrorKind).toBe('none');
    expect(controller.__test?.activeRequestCount()).toBe(0);
  });

  it('a superseded request cannot clobber a newer success', async () => {
    const calls = abortAwareFetch();
    const controller = createPreviewController(connection);

    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    expect(calls).toHaveLength(1);

    // Second edit supersedes the first: the first controller aborts.
    controller.trigger('box', boxParams(20));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);
    expect(calls).toHaveLength(2);

    // The newer request completes first.
    calls[1]?.resolve(okResponse());
    await settle();
    const serverAfterNewer = getState().preview.server;
    expect(serverAfterNewer).not.toBeNull();
    expect(getState().preview.inFlight).toBe(false);

    // The superseded first request resolves late; the store must keep the
    // newer mesh.
    calls[0]?.resolve(okResponse());
    await settle();
    expect(getState().preview.server).toBe(serverAfterNewer);
    controller.dispose();
  });

  it('keeps the local fallback visible while a request is cancelled', async () => {
    abortAwareFetch();
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await vi.advanceTimersByTimeAsync(DEBOUNCE_MS + 5);

    const localBefore = getState().preview.local;
    expect(localBefore).not.toBeNull();
    controller.cancel();
    // The quiet cancel must not clear the local mesh.
    expect(getState().preview.local).toBe(localBefore);
    expect(getState().preview.inFlight).toBe(false);
    controller.dispose();
  });
});
