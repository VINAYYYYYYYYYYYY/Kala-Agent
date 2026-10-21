// Tests for the HTTP client. We stub `fetch` so the assertions cover the
// URL construction, headers, body serialisation, redirect handling, and
// error classification paths without a real network round-trip.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  CHAT_TURN_TIMEOUT_MS,
  FALLBACK_CHAT_TURN_TIMEOUT_MS,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
  StudioTimeoutError,
  chatTurnTimeoutMs,
  getHealth,
  getIntegrations,
  getProviders,
  getSettings,
  postChat,
  postExport,
  postHandoff,
  postPreview,
  postSettings,
} from '../src/api/client';
import { dispatch, getState, resetStore } from '../src/state/store';
import type {
  ChatRequest,
  ExportRequest,
  HandoffRequest,
  PreviewRequest,
  StudioConnection,
} from '../src/types/studio';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

interface CapturedCall {
  url: string;
  init: RequestInit;
}

let captured: CapturedCall[] = [];
let responder: (init: RequestInit) => Response | Promise<Response> = () =>
  new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

beforeEach(() => {
  captured = [];
  resetStore();
  vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input.toString();
    captured.push({ url, init: init ?? {} });
    return Promise.resolve(responder(init ?? {}));
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('GET requests', () => {
  it('builds the absolute loopback URL and includes Accept header', async () => {
    responder = () => new Response(JSON.stringify({ status: 'ok', service: 'kala-studio', version: '0.0.1', offline: true, step_available: false, integrations: [], providers: [] }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    const response = await getHealth(connection);
    expect(response.status).toBe('ok');
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/health');
    expect(call.init.method).toBe('GET');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Accept']).toBe('application/json');
    expect(headers['Authorization']).toBeUndefined();
  });

  it('stores the backend design-turn budget and falls back when it is absent', async () => {
    responder = () =>
      new Response(
        JSON.stringify({
          status: 'ok',
          service: 'kala-studio',
          version: '0.1.0',
          offline: true,
          design_turn_budget_seconds: 3_960,
          step_available: false,
          integrations: [],
          providers: [],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    await getHealth(connection);
    expect(getState().server.designTurnBudgetSeconds).toBe(3_960);

    responder = () =>
      new Response(
        JSON.stringify({
          status: 'ok',
          service: 'kala-studio',
          version: '0.1.0',
          offline: true,
          step_available: false,
          integrations: [],
          providers: [],
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    await getHealth(connection);
    expect(getState().server.designTurnBudgetSeconds).toBeNull();
    expect(chatTurnTimeoutMs(getState().server.designTurnBudgetSeconds)).toBe(
      FALLBACK_CHAT_TURN_TIMEOUT_MS,
    );
  });

  it('refuses to follow redirects', async () => {    responder = () => new Response('redirected', { status: 302, headers: { Location: 'http://evil.example/' } });
    await expect(getHealth(connection)).rejects.toBeInstanceOf(StudioApiError);
    const call = captured[0]!;
    expect(call.init.redirect).toBe('error');
  });

  it('classifies 401 as StudioAuthError', async () => {
    responder = () => new Response(JSON.stringify({ code: 'unauthorized', message: 'token bad', details: [] }), { status: 401, headers: { 'Content-Type': 'application/json' } });
    await expect(getHealth(connection)).rejects.toBeInstanceOf(StudioAuthError);
  });
});

describe('POST requests', () => {
  it('sends Authorization header and JSON body', async () => {
    responder = (init) => new Response(JSON.stringify({ positions: [], normals: [], indices: [], bounds: { min: [0,0,0], max: [0,0,0] }, engine: 'analytic', warnings: [] }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    const payload: PreviewRequest = { kind: 'box', parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 } };
    await postPreview(connection, payload);
    const call = captured[0]!;
    expect(call.init.method).toBe('POST');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe('Bearer unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa');
    expect(headers['Content-Type']).toBe('application/json');
    expect(call.init.body).toBe(JSON.stringify(payload));
  });

  it('cancels via AbortSignal and converts DOMException into StudioAbortError', async () => {
    // Make the mock fetch honour the abort signal and reject with a
    // DOMException, mirroring real browser behaviour.
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        if (init?.signal?.aborted) {
          reject(new DOMException('aborted', 'AbortError'));
          return;
        }
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    });
    const controller = new AbortController();
    controller.abort();
    await expect(postPreview(connection, { kind: 'box', parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 } }, controller.signal)).rejects.toBeInstanceOf(StudioAbortError);
  });

  it('classifies server errors with code+message bodies as StudioApiError', async () => {
    responder = () => new Response(JSON.stringify({ code: 'invalid_dimension', message: 'size_x too small', details: ['hint'] }), { status: 422, headers: { 'Content-Type': 'application/json' } });
    try {
      await postPreview(connection, { kind: 'box', parameters: { size_x: 0, size_y: 1, size_z: 1, subdivisions: 1 } });
      throw new Error('expected throw');
    } catch (cause) {
      expect(cause).toBeInstanceOf(StudioApiError);
      if (cause instanceof StudioApiError) {
        expect(cause.code).toBe('invalid_dimension');
        expect(cause.message).toBe('size_x too small');
        expect(cause.status).toBe(422);
        expect(cause.details).toEqual(['hint']);
      }
    }
  });
});

describe('chat and settings requests', () => {
  it('chat sends the provider/model/prompt/allow_remote payload', async () => {
    responder = () => new Response(JSON.stringify({ text: 'hello', provider: 'local', model: null, offline: true }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    const payload: ChatRequest = { provider: 'local', model: 'helper-v1', prompt: 'hi', allow_remote: false };
    await postChat(connection, payload);
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/chat');
    expect(call.init.body).toBe(JSON.stringify(payload));
  });

  it('derives the chat deadline from the backend budget plus five minutes', () => {
    // The documented fallback when an older backend reports no budget.
    expect(FALLBACK_CHAT_TURN_TIMEOUT_MS).toBe(70 * 60_000);
    expect(chatTurnTimeoutMs(null)).toBe(70 * 60_000);
    expect(chatTurnTimeoutMs(undefined)).toBe(70 * 60_000);
    expect(chatTurnTimeoutMs(0)).toBe(70 * 60_000);
    expect(chatTurnTimeoutMs(Number.NaN)).toBe(70 * 60_000);
    // The real backend budget: 2 * (180 + 1800) = 3 960 s, plus the margin.
    expect(chatTurnTimeoutMs(3_960)).toBe(3_960_000 + 5 * 60_000);
    expect(CHAT_TURN_TIMEOUT_MS).toBe(FALLBACK_CHAT_TURN_TIMEOUT_MS);
  });

  it('uses the backend-reported budget as the chat deadline', async () => {
    vi.useFakeTimers();
    try {
      dispatch({ type: 'server/capabilities', designAvailable: true, designTurnBudgetSeconds: 1 });
      // A backend that never answers: the fetch ends only when the signal
      // aborts.
      vi.unstubAllGlobals();
      vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        });
      });

      let failure: unknown = null;
      void postChat(connection, {
        provider: 'local',
        model: null,
        prompt: 'hi',
        allow_remote: false,
      }).then(
        () => undefined,
        (cause: unknown) => {
          failure = cause;
        },
      );

      // The reported budget (1 s) plus the margin has not expired yet; the
      // old 30-minute/70-minute constants would still be pending too.
      await vi.advanceTimersByTimeAsync(1_000 + 5 * 60_000 - 10);
      expect(failure).toBeNull();
      await vi.advanceTimersByTimeAsync(20);
      expect(failure).toBeInstanceOf(StudioTimeoutError);
      expect((failure as StudioTimeoutError).path).toBe('/api/studio/chat');
    } finally {
      vi.useRealTimers();
    }
  });

  it('abandons a wedged chat turn at the fallback deadline', async () => {
    vi.useFakeTimers();
    try {
      resetStore();
      expect(getState().server.designTurnBudgetSeconds).toBeNull();
      // A backend that never answers: the fetch only ends when the signal
      // aborts.
      vi.unstubAllGlobals();
      vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        });
      });

      const pending = postChat(connection, {
        provider: 'local',
        model: null,
        prompt: 'hi',
        allow_remote: false,
      }).catch((cause: unknown) => cause);

      await vi.advanceTimersByTimeAsync(FALLBACK_CHAT_TURN_TIMEOUT_MS + 10);

      const failure = await pending;
      expect(failure).toBeInstanceOf(StudioTimeoutError);
      expect((failure as StudioTimeoutError).path).toBe('/api/studio/chat');
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps an operator cancellation a cancellation and not a timeout', async () => {
    vi.useFakeTimers();
    try {
      vi.unstubAllGlobals();
      vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
        return new Promise((_resolve, reject) => {
          init?.signal?.addEventListener('abort', () => {
            reject(new DOMException('aborted', 'AbortError'));
          });
        });
      });

      const controller = new AbortController();
      const pending = postChat(connection, {
        provider: 'local',
        model: null,
        prompt: 'hi',
        allow_remote: false,
      }, controller.signal).catch((cause: unknown) => cause);

      controller.abort();
      await vi.advanceTimersByTimeAsync(10);

      const failure = await pending;
      expect(failure).toBeInstanceOf(StudioAbortError);
      expect(failure).not.toBeInstanceOf(StudioTimeoutError);
    } finally {
      vi.useRealTimers();
    }
  });

  it('getSettings fetches /api/studio/settings without Authorization header', async () => {
    responder = () =>
      new Response(
        JSON.stringify({
          credentials: [{ provider: 'openai', env_var: 'OPENAI_API_KEY', configured: false, source: 'none' }],
          providers: [],
          offline: true,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    const settings = await getSettings(connection);
    expect(settings.offline).toBe(true);
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/settings');
    expect(call.init.method).toBe('GET');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBeUndefined();
  });

  it('postSettings sends bearer token and provider/api_key payload to /api/studio/settings', async () => {
    responder = () =>
      new Response(
        JSON.stringify({
          credentials: [{ provider: 'openai', env_var: 'OPENAI_API_KEY', configured: true, source: 'stored' }],
          providers: [{ id: 'openai', name: 'OpenAI', configured: true, notes: '' }],
          offline: false,
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    const payload = { provider: 'openai' as const, api_key: 'sk-test-key' };
    const settings = await postSettings(connection, payload);
    expect(settings.offline).toBe(false);
    const call = captured[0]!;
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/settings');
    expect(call.init.method).toBe('POST');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe('Bearer unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa');
    expect(call.init.body).toBe(JSON.stringify(payload));
  });
});

// The real backend streams the artefact in the body of the export/handoff
// POST (an attachment), so these tests use real binary response bodies and
// Content-Disposition headers instead of the invented JSON metadata shape.
describe('binary export and handoff', () => {
  const stlBytes = new Uint8Array([0x53, 0x54, 0x4c, 0x20, 1, 2, 3, 4]); // 8 bytes
  const box: ExportRequest = {
    kind: 'box',
    parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 },
    format: 'stl',
  };

  function binaryResponse(bytes: Uint8Array, headers: Record<string, string> = {}): Response {
    // Copy into a plain ArrayBuffer so the DOM `BodyInit` typing accepts it.
    const buffer = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(buffer).set(bytes);
    return new Response(buffer, { status: 200, headers: { 'Content-Type': 'model/stl', ...headers } });
  }

  it('direct export POSTs once and consumes the attachment bytes (no artifacts GET)', async () => {
    responder = () =>
      binaryResponse(stlBytes, {
        'Content-Disposition': 'attachment; filename="kala-studio-box.stl"',
      });

    const artifact = await postExport(connection, box);

    expect(artifact.bytes).toBe(stlBytes.byteLength);
    expect(artifact.blob.size).toBe(stlBytes.byteLength);
    expect(artifact.filename).toBe('kala-studio-box.stl');
    expect(artifact.format).toBe('stl');
    expect(artifact.integrationId).toBeNull();

    // Exactly one request; it is the POST itself, never a follow-up GET.
    expect(captured).toHaveLength(1);
    const call = captured[0]!;
    expect(call.init.method).toBe('POST');
    expect(call.url).toBe('http://127.0.0.1:8765/api/studio/export');
    expect(new URL(call.url).pathname).not.toContain('/artifacts/');
    const headers = call.init.headers as Record<string, string>;
    expect(headers['Authorization']).toBe('Bearer unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa');
    expect(headers['Content-Type']).toBe('application/json');
    expect(call.init.body).toBe(JSON.stringify(box));
    expect(call.init.redirect).toBe('error');
  });

  it('handoff reads the filename and integration id from the response headers', async () => {
    responder = () =>
      binaryResponse(stlBytes, {
        'Content-Type': 'application/step',
        'Content-Disposition': 'attachment; filename="solidworks.step"',
        'x-studio-handoff': 'solidworks;format=step;bytes=8',
      });
    const payload: HandoffRequest = {
      integration_id: 'solidworks',
      kind: 'box',
      parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 },
      format: 'step',
    };

    const artifact = await postHandoff(connection, payload);

    expect(artifact.integrationId).toBe('solidworks');
    expect(artifact.filename).toBe('solidworks.step');
    expect(artifact.format).toBe('step');
    expect(artifact.bytes).toBe(8);
    expect(captured).toHaveLength(1);
    expect(captured[0]!.init.body).toBe(JSON.stringify(payload));
  });

  it('uses a suggested filename when content-disposition is absent', async () => {
    responder = () => binaryResponse(stlBytes);
    const payload: ExportRequest = {
      kind: 'sphere',
      parameters: { radius: 1, rings: 8, sectors: 12 },
      format: 'stl',
    };
    const artifact = await postExport(connection, payload);
    expect(artifact.filename).toBe('kala-studio-sphere.stl');
  });

  it('reduces a path-traversing content-disposition to a basename', async () => {
    responder = () =>
      binaryResponse(stlBytes, { 'Content-Disposition': 'attachment; filename="../../etc/passwd"' });
    const artifact = await postExport(connection, box);
    expect(artifact.filename).toBe('passwd');
  });

  it('decodes an RFC 6266 filename* value', async () => {
    responder = () =>
      binaryResponse(stlBytes, { 'Content-Disposition': "attachment; filename*=UTF-8''kala%20box.stl" });
    const artifact = await postExport(connection, box);
    expect(artifact.filename).toBe('kala box.stl');
  });

  it('classifies 401 on the binary path as StudioAuthError', async () => {
    responder = () =>
      new Response(JSON.stringify({ code: 'unauthorized', message: 'token bad', details: [] }), {
        status: 401,
        headers: { 'Content-Type': 'application/json' },
      });
    await expect(postExport(connection, box)).rejects.toBeInstanceOf(StudioAuthError);
  });

  it('surfaces a 503 step_unavailable honestly', async () => {
    responder = () =>
      new Response(
        JSON.stringify({
          code: 'step_unavailable',
          message: 'STEP export requires the OCP bindings',
          details: ['install cadquery-ocp'],
        }),
        { status: 503, headers: { 'Content-Type': 'application/json' } },
      );
    try {
      await postExport(connection, { ...box, format: 'step' });
      throw new Error('expected throw');
    } catch (cause) {
      expect(cause).toBeInstanceOf(StudioApiError);
      if (cause instanceof StudioApiError) {
        expect(cause.code).toBe('step_unavailable');
        expect(cause.status).toBe(503);
      }
    }
  });

  it('surfaces a 409 unsupported handoff format', async () => {
    responder = () =>
      new Response(
        JSON.stringify({ code: 'handoff_format_unsupported', message: 'not supported', details: [] }),
        { status: 409, headers: { 'Content-Type': 'application/json' } },
      );
    try {
      await postHandoff(connection, {
        integration_id: 'solidworks',
        kind: 'box',
        parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 },
        format: 'stl',
      });
      throw new Error('expected throw');
    } catch (cause) {
      expect(cause).toBeInstanceOf(StudioApiError);
      if (cause instanceof StudioApiError) {
        expect(cause.code).toBe('handoff_format_unsupported');
      }
    }
  });

  it('refuses redirects on the binary path', async () => {
    responder = () =>
      new Response('nope', { status: 302, headers: { Location: 'http://evil.example/' } });
    await expect(postExport(connection, box)).rejects.toBeInstanceOf(StudioApiError);
    expect(captured[0]!.init.redirect).toBe('error');
  });

  it('converts an aborted binary request into StudioAbortError', async () => {
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
      return new Promise((_resolve, reject) => {
        if (init?.signal?.aborted) {
          reject(new DOMException('aborted', 'AbortError'));
          return;
        }
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    });
    const controller = new AbortController();
    controller.abort();
    await expect(postExport(connection, box, controller.signal)).rejects.toBeInstanceOf(
      StudioAbortError,
    );
  });
});

describe('network errors', () => {
  it('wraps network failures in StudioNetworkError', async () => {
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', () => Promise.reject(new TypeError('failed to fetch')));
    await expect(getHealth(connection)).rejects.toBeInstanceOf(StudioNetworkError);
  });
});
