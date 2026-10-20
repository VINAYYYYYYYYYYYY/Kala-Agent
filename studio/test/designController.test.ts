// Tests for the design resource controller.
//
// The controller is the only thing that fetches a drawing or a section. It
// records the loading, ready, and error states in the store's caches and
// never asks the backend twice for a response it already holds.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createDesignController } from '../src/state/designController';
import { drawingCacheKey, sectionCacheKey } from '../src/state/designs';
import { getState, resetStore } from '../src/state/store';
import type { StudioConnection } from '../src/types/studio';
import { BRACKET_DESIGN_ID, bracketDrawing, bracketSection } from './designFixtures';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

let captured: string[] = [];
let responder: (url: string) => Response = () =>
  new Response('{}', { status: 200, headers: { 'Content-Type': 'application/json' } });

beforeEach(() => {
  captured = [];
  resetStore();
  vi.stubGlobal('fetch', (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input.toString();
    captured.push(url);
    return Promise.resolve(responder(url));
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('the design controller', () => {
  it('loads a drawing and records it under its cache key', async () => {
    responder = () =>
      new Response(JSON.stringify(bracketDrawing), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    const controller = createDesignController(connection);
    await controller.loadDrawing(BRACKET_DESIGN_ID, ['front']);
    const key = drawingCacheKey(BRACKET_DESIGN_ID, ['front']);
    const entry = getState().designs.drawings.get(key);
    expect(entry?.status).toBe('ready');
    expect(entry?.data?.views[0]?.name).toBe('front');
    expect(captured[0]).toContain('/drawing?views=front');
  });

  it('does not ask the backend twice for a drawing it already holds', async () => {
    responder = () =>
      new Response(JSON.stringify(bracketDrawing), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    const controller = createDesignController(connection);
    await controller.loadDrawing(BRACKET_DESIGN_ID, ['front']);
    await controller.loadDrawing(BRACKET_DESIGN_ID, ['front']);
    expect(captured).toHaveLength(1);
  });

  it('records the failure of a drawing request', async () => {
    responder = () =>
      new Response(JSON.stringify({ code: 'design_not_found', message: 'no such design', details: [] }), {
        status: 404,
        headers: { 'Content-Type': 'application/json' },
      });
    const controller = createDesignController(connection);
    await controller.loadDrawing(BRACKET_DESIGN_ID, ['front']);
    const entry = getState().designs.drawings.get(drawingCacheKey(BRACKET_DESIGN_ID, ['front']));
    expect(entry?.status).toBe('error');
    expect(entry?.error).toContain('design_not_found');
  });

  it('loads a section and records it under its cache key', async () => {
    responder = () =>
      new Response(JSON.stringify(bracketSection), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    const controller = createDesignController(connection);
    await controller.loadSection(BRACKET_DESIGN_ID, 'XZ', 4);
    const entry = getState().designs.sections.get(sectionCacheKey(BRACKET_DESIGN_ID, 'XZ', 4));
    expect(entry?.status).toBe('ready');
    expect(entry?.data?.area_mm2).toBeCloseTo(2321.4601836602546, 9);
    expect(captured[0]).toContain('/section?plane=XZ&offset=4');
  });

  it('refuses to work after it is disposed', async () => {
    const controller = createDesignController(connection);
    controller.dispose();
    expect(controller.isDisposed()).toBe(true);
    await controller.loadDrawing(BRACKET_DESIGN_ID, ['front']);
    expect(captured).toHaveLength(0);
  });
});
