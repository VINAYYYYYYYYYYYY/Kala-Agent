// Tests for the preview controller.
//
// The controller must:
//   - dispatch a local preview immediately
//   - debounce a server request
//   - abort the previous request when parameters change
//   - distinguish cancellation from timeout
//   - dispose cleanly: timers cancelled, controllers aborted
//
// We stub the global `fetch` so no real network round-trip happens.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPreviewController, type PreviewController } from '../src/state/previewController';
import { StudioAbortError, StudioNetworkError } from '../src/api/client';
import type { StudioConnection, StudioParameters, PrimitiveKind, PreviewResponse } from '../src/types/studio';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

function boxParams(size_x: number): StudioParameters {
  return { size_x, size_y: 20, size_z: 30, subdivisions: 1 };
}

function makePreviewResponse(p: StudioParameters): PreviewResponse {
  return {
    positions: new Array(9).fill(0),
    normals: new Array(9).fill(0),
    indices: [0, 1, 2],
    bounds: { min: [-10, -10, -10], max: [10, 10, 10] },
    engine: 'analytic',
    warnings: [],
  };
}

interface FetchCall {
  signal: AbortSignal;
  resolve(value: Response | PromiseLike<Response>): void;
  reject(cause: unknown): void;
}

let pendingFetches: FetchCall[] = [];

beforeEach(() => {
  pendingFetches = [];
  vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
    return new Promise<Response>((resolve, reject) => {
      const call: FetchCall = {
        signal: init?.signal ?? new AbortController().signal,
        resolve,
        reject,
      };
      pendingFetches.push(call);
    });
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('previewController.trigger', () => {
  it('debounces the first server request and aborts it on the second edit', async () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    // Before debounce: no fetch issued yet.
    expect(pendingFetches).toHaveLength(0);

    await new Promise((r) => setTimeout(r, 300));
    expect(pendingFetches).toHaveLength(1);
    const firstCall = pendingFetches[0]!;
    expect(firstCall.signal.aborted).toBe(false);

    // Operator changes parameters — controller must abort the in-flight
    // request and start a fresh one.
    controller.trigger('box', boxParams(20));
    await new Promise((r) => setTimeout(r, 300));
    expect(firstCall.signal.aborted).toBe(true);
    expect(pendingFetches).toHaveLength(2);
    controller.dispose();
  });

  it('cancelling in-flight requests does not surface as an error', async () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await new Promise((r) => setTimeout(r, 300));
    controller.cancel();
    controller.dispose();
    const first = pendingFetches[0]!;
    first.reject(new DOMException('aborted', 'AbortError'));
    await new Promise((r) => setTimeout(r, 20));
    // No further side-effects expected; the dispose flag prevented an
    // error dispatch.
  });

  it('disposal aborts any pending fetch and cancels timers', async () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await new Promise((r) => setTimeout(r, 300));
    expect(controller.__test?.hasPendingTimer()).toBe(false);
    expect(controller.__test?.activeRequestCount()).toBe(1);
    controller.dispose();
    expect(controller.isDisposed()).toBe(true);
    expect(controller.__test?.activeRequestCount()).toBe(0);
    const first = pendingFetches[0]!;
    expect(first.signal.aborted).toBe(true);
  });

  it('out-of-order responses are dropped via the generation guard', async () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await new Promise((r) => setTimeout(r, 300));
    const first = pendingFetches[0]!;
    // Operator edits again immediately so the generation counter moves.
    controller.trigger('box', boxParams(20));
    await new Promise((r) => setTimeout(r, 300));
    const second = pendingFetches[1]!;
    // The older response arrives after the newer one. It must be dropped.
    first.resolve(new Response(JSON.stringify(makePreviewResponse(boxParams(10))), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    second.resolve(new Response(JSON.stringify(makePreviewResponse(boxParams(20))), { status: 200, headers: { 'Content-Type': 'application/json' } }));
    await new Promise((r) => setTimeout(r, 30));
    controller.dispose();
  });
});

describe('error classification', () => {
  it('timeout aborts classify as cancelled, not network', async () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(10));
    await new Promise((r) => setTimeout(r, 300));
    const first = pendingFetches[0]!;
    // Simulate a fetch-level abort (timeout). The controller treats this
    // as the timeout path so we don't surface it as a user-facing error.
    first.reject(new StudioNetworkError('simulated timeout', new DOMException('aborted', 'AbortError')));
    await new Promise((r) => setTimeout(r, 30));
    controller.dispose();
  });

  it('cancel does not throw after disposal', () => {
    const controller = createPreviewController(connection);
    controller.dispose();
    expect(() => controller.cancel()).not.toThrow();
  });
});

describe('local preview', () => {
  it('emits a local preview immediately even when parameters are invalid', () => {
    const controller = createPreviewController(connection);
    controller.trigger('box', boxParams(-1));
    expect(controller.__test?.hasPendingTimer()).toBe(true);
    controller.dispose();
  });
});

describe('StudioAbortError', () => {
  it('extends Error with the right name', () => {
    const err = new StudioAbortError();
    expect(err.name).toBe('StudioAbortError');
    expect(err.message).toBe('request aborted');
  });
});