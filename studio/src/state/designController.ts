// The design resource controller.
//
// The store keeps the drawing and section responses of a design in caches
// keyed by the request parameters. This controller is the only thing that
// fetches them: it turns a cache key into a request, records the loading
// and error states, and keeps the cache honest (a warm cache is not
// re-fetched, a failed one can be retried).

import {
  getDesignDrawing,
  getDesignSection,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
  StudioTimeoutError,
} from '../api/client';
import type {
  DesignViewName,
  SectionPlane,
  StudioConnection,
} from '../types/studio';
import { drawingCacheKey, sectionCacheKey } from './designs';
import { dispatch, getState } from './store';

export interface DesignController {
  /** Load the HLR drawing of a design. A warm cache is not re-fetched. */
  loadDrawing(designId: string, views?: ReadonlyArray<DesignViewName>): Promise<void>;
  /** Load the section cut of a design. A warm cache is not re-fetched. */
  loadSection(designId: string, plane: SectionPlane, offsetMm?: number): Promise<void>;
  /** Abort every in-flight request and stop accepting new ones. */
  dispose(): void;
  isDisposed(): boolean;
}

function describeFailure(cause: unknown): string {
  if (cause instanceof StudioAbortError) return 'cancelled';
  if (cause instanceof StudioTimeoutError) return `timeout: ${cause.path}`;
  if (cause instanceof StudioAuthError) return `auth: ${cause.message} (${cause.status})`;
  if (cause instanceof StudioApiError) return `${cause.code}: ${cause.message}`;
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'unknown error';
}

export function createDesignController(connection: StudioConnection): DesignController {
  const inFlight = new Set<AbortController>();
  let disposed = false;

  function abortAll(): void {
    for (const controller of inFlight) controller.abort();
    inFlight.clear();
  }

  async function loadDrawing(
    designId: string,
    views: ReadonlyArray<DesignViewName> = [],
  ): Promise<void> {
    if (disposed) return;
    const key = drawingCacheKey(designId, views);
    const cached = getState().designs.drawings.get(key);
    if (cached && cached.status === 'ready' && cached.data) return;
    const controller = new AbortController();
    inFlight.add(controller);
    dispatch({ type: 'design/drawing-start', key });
    try {
      const drawing = await getDesignDrawing(connection, designId, views, controller.signal);
      if (disposed || controller.signal.aborted) return;
      dispatch({ type: 'design/drawing-success', key, drawing });
    } catch (cause) {
      if (disposed || controller.signal.aborted) return;
      dispatch({ type: 'design/drawing-error', key, message: describeFailure(cause) });
    } finally {
      inFlight.delete(controller);
    }
  }

  async function loadSection(
    designId: string,
    plane: SectionPlane,
    offsetMm?: number,
  ): Promise<void> {
    if (disposed) return;
    const key = sectionCacheKey(designId, plane, offsetMm);
    const cached = getState().designs.sections.get(key);
    if (cached && cached.status === 'ready' && cached.data) return;
    const controller = new AbortController();
    inFlight.add(controller);
    dispatch({ type: 'design/section-start', key });
    try {
      const section = await getDesignSection(connection, designId, plane, offsetMm, controller.signal);
      if (disposed || controller.signal.aborted) return;
      dispatch({ type: 'design/section-success', key, section });
    } catch (cause) {
      if (disposed || controller.signal.aborted) return;
      dispatch({ type: 'design/section-error', key, message: describeFailure(cause) });
    } finally {
      inFlight.delete(controller);
    }
  }

  return {
    loadDrawing,
    loadSection,
    dispose(): void {
      if (disposed) return;
      disposed = true;
      abortAll();
    },
    isDisposed: () => disposed,
  };
}
