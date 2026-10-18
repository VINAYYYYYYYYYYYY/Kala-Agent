// Preview controller.
//
// Owns the local-vs-server preview pipeline:
//   - Always renders a deterministic local mesh immediately on parameter
//     change, so the viewport never sits empty.
//   - Debounces a server request; on every new edit, the in-flight request
//     is aborted and a new one fires.
//   - Out-of-order responses are dropped via a per-request generation
//     counter (a closure, not a captured boolean).
//   - On disposal, all timers abort and any pending request aborts.

import { postPreview, StudioAbortError, StudioApiError, StudioAuthError, StudioNetworkError } from '../api/client';
import { PreviewResponse, StudioConnection, StudioParameters, PrimitiveKind } from '../types/studio';
import { buildLocal, validateParameters } from '../viewer/localPrimitives';
import { dispatch } from './store';

const DEBOUNCE_MS = 220;
const REQUEST_TIMEOUT_MS = 8_000;

function fingerprint(kind: PrimitiveKind, p: StudioParameters): string {
  return `${kind}|${JSON.stringify(p)}`;
}

function describeError(cause: unknown): { message: string; kind: 'cancelled' | 'timeout' | 'network' | 'server' } {
  if (cause instanceof StudioAbortError) return { message: 'cancelled', kind: 'cancelled' };
  if (cause instanceof StudioAuthError) return { message: `auth: ${cause.message} (${cause.status})`, kind: 'server' };
  if (cause instanceof StudioApiError) return { message: `${cause.code}: ${cause.message}`, kind: 'server' };
  if (cause instanceof StudioNetworkError) {
    if (cause.cause instanceof DOMException && cause.cause.name === 'AbortError') {
      return { message: 'cancelled', kind: 'cancelled' };
    }
    return { message: cause.message, kind: 'network' };
  }
  if (cause instanceof Error) return { message: cause.message, kind: 'network' };
  return { message: 'unknown error', kind: 'network' };
}

export interface PreviewController {
  trigger(kind: PrimitiveKind, p: StudioParameters): void;
  cancel(): void;
  dispose(): void;
  isDisposed(): boolean;
  // Test-only hooks. The runtime never calls these.
  __test?: {
    activeRequestCount(): number;
    hasPendingTimer(): boolean;
    generation(): number;
  };
}

export function createPreviewController(connection: StudioConnection): PreviewController {
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let generation = 0;
  let disposed = false;
  // Whether we have pushed `preview/request-start` (store `inFlight:
  // true`) and not yet resolved it. Explicit cancel/dispose use this to
  // clear the loading state exactly once, quietly.
  let storeLoading = false;
  const inFlight = new Set<AbortController>();
  const lastFingerprint: { current: string | null } = { current: null };

  function clearDebounce(): void {
    if (debounceTimer !== null) {
      clearTimeout(debounceTimer);
      debounceTimer = null;
    }
  }

  function abortAll(): void {
    for (const controller of inFlight) controller.abort();
    inFlight.clear();
  }

  // Retire the current generation so any in-flight completion (including
  // a non-cooperative mock or a slow network response that ignores the
  // abort) is dropped instead of overwriting newer state.
  function invalidate(): void {
    generation += 1;
    lastFingerprint.current = null;
  }

  function clearLoadingQuietly(): void {
    if (!storeLoading) return;
    storeLoading = false;
    dispatch({ type: 'preview/request-cancelled' });
  }

  function localUpdate(kind: PrimitiveKind, p: StudioParameters): void {
    const validation = validateParameters(kind, p);
    if (validation) {
      dispatch({
        type: 'preview/local',
        preview: {
          kind,
          parameters: p,
          positions: new Float32Array(0),
          normals: new Float32Array(0),
          indices: new Uint32Array(0),
          bounds: { min: [0, 0, 0], max: [0, 0, 0] },
          source: 'local',
          generatedAt: Date.now(),
          engine: 'invalid',
          warnings: [validation.message],
        },
      });
      return;
    }
    try {
      const preview = buildLocal(kind, p);
      dispatch({ type: 'preview/local', preview });
    } catch (cause) {
      const desc = describeError(cause);
      dispatch({ type: 'preview/request-error', message: desc.message, kind: desc.kind });
    }
  }

  async function requestServer(
    kind: PrimitiveKind,
    p: StudioParameters,
    fp: string,
    myGeneration: number,
  ): Promise<void> {
    if (disposed) return;
    const controller = new AbortController();
    inFlight.add(controller);
    let timedOut = false;
    const timeout = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, REQUEST_TIMEOUT_MS);
    dispatch({ type: 'preview/request-start' });
    storeLoading = true;
    try {
      const response: PreviewResponse = await postPreview(connection, { kind, parameters: p }, controller.signal);
      if (disposed || myGeneration !== generation || fp !== lastFingerprint.current) return;
      storeLoading = false;
      dispatch({ type: 'preview/request-success', preview: response });
    } catch (cause) {
      if (disposed || myGeneration !== generation) return;
      // The controller's own 8-second timeout MUST complete the request
      // regardless of how the rejection is classified: an aborted fetch
      // surfaces as `StudioAbortError` (`kind: 'cancelled'`), which the
      // user-cancel suppression below would otherwise swallow, leaving
      // the store stuck on "loading" forever.
      if (timedOut) {
        storeLoading = false;
        dispatch({ type: 'preview/request-error', message: 'request timed out', kind: 'timeout' });
        return;
      }
      // A deliberate abort (cancel/dispose/trigger superseding this
      // request) is not an error. The generation guard above already
      // dropped superseded requests; this is the narrow "aborted but
      // same generation" case, which must stay silent.
      if (controller.signal.aborted) return;
      const desc = describeError(cause);
      if (desc.kind === 'cancelled') return;
      storeLoading = false;
      dispatch({ type: 'preview/request-error', message: desc.message, kind: desc.kind });
    } finally {
      clearTimeout(timeout);
      inFlight.delete(controller);
    }
  }

  function trigger(kind: PrimitiveKind, p: StudioParameters): void {
    if (disposed) return;
    const fp = fingerprint(kind, p);
    lastFingerprint.current = fp;
    generation += 1;
    const myGeneration = generation;
    abortAll();
    // A superseding edit keeps the loading flag set: the new request (or
    // its debounce) owns the indicator now. Only cancel/dispose clear it.
    localUpdate(kind, p);
    clearDebounce();
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      void requestServer(kind, p, fp, myGeneration);
    }, DEBOUNCE_MS);
  }

  function cancel(): void {
    if (disposed) return;
    invalidate();
    abortAll();
    clearDebounce();
    clearLoadingQuietly();
  }

  function dispose(): void {
    if (disposed) return;
    disposed = true;
    invalidate();
    abortAll();
    clearDebounce();
    clearLoadingQuietly();
  }

  return {
    trigger,
    cancel,
    dispose,
    isDisposed: () => disposed,
    __test: {
      activeRequestCount: () => inFlight.size,
      hasPendingTimer: () => debounceTimer !== null,
      generation: () => generation,
    },
  };
}