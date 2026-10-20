// Tests for the design slice, the viewer mesh conversion, the journal
// merge, and the chain verdict.
//
// The rules under test:
//   - the store keeps a bounded LRU of designs (16) and evicts the oldest,
//   - a design that arrives becomes the active one,
//   - the viewport mesh source is honest about where the geometry came from,
//   - the backend journal events are deduped by sequence and merged with
//     the local ledger by time,
//   - `verified` is only ever set by a real verification response.

import { beforeEach, describe, expect, it } from 'vitest';
import {
  activateDesign,
  clearActiveDesign,
  createInitialUiState,
  designReceived,
  dispatch,
  getState,
  reducer,
  resetStore,
  selectActiveDesign,
  selectCadProvider,
  selectCadProviderRecord,
  selectChainStatus,
  selectDisplayMeshSource,
  selectMergedJournal,
  setCadProvider,
  type UiState,
} from '../src/state/store';
import {
  createDesignMeshMemo,
  designToDisplayMesh,
  drawingCacheKey,
  sectionCacheKey,
} from '../src/state/designs';
import { mergeJournalEntries, summarisePayload } from '../src/state/journal';
import type { DesignResult, JournalPage } from '../src/types/studio';
import { DESIGN_LRU_CAP } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  bracketDesignResult,
  bracketJournalBroken,
  bracketJournalPage,
  bracketJournalVerified,
  cadProviders,
  notReportedDesignResult,
} from './designFixtures';

function designWithId(designId: string): DesignResult {
  return { ...bracketDesignResult, design_id: designId };
}

function journalEvent(sequence: number, occurredAt: string, type = 'design.measured') {
  return {
    sequence,
    event_type: type,
    aggregate_id: BRACKET_DESIGN_ID,
    payload: { volume_mm3: 35571.416, surface_area_mm2: 11889.1, solid_count: 1 },
    digest: 'd'.repeat(64),
    occurred_at: occurredAt,
    previous_event_digest: '0'.repeat(64),
    event_digest: 'e'.repeat(64),
  };
}

beforeEach(() => {
  resetStore();
});

describe('the design LRU', () => {
  it('records a design and makes it the active one', () => {
    designReceived(bracketDesignResult);
    const state = getState();
    expect(state.designs.designs.size).toBe(1);
    expect(state.designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    expect(selectActiveDesign(state)?.label).toBe('L-bracket 60 x 45 x 40');
  });

  it('evicts the least recently used design beyond the cap', () => {
    const ids = Array.from({ length: DESIGN_LRU_CAP + 1 }, (_, index) =>
      index.toString(16).padStart(64, '0'),
    );
    let state = createInitialUiState();
    for (const id of ids) {
      state = reducer(state, { type: 'design/received', design: designWithId(id) });
    }
    expect(state.designs.designs.size).toBe(DESIGN_LRU_CAP);
    // The first design is the one that was evicted.
    expect(state.designs.designs.has(ids[0]!)).toBe(false);
    expect(state.designs.designs.has(ids[1]!)).toBe(true);
    expect(state.designs.order).toHaveLength(DESIGN_LRU_CAP);
  });

  it('refreshes a design when it is activated, so it survives the next eviction', () => {
    const ids = Array.from({ length: DESIGN_LRU_CAP + 1 }, (_, index) =>
      index.toString(16).padStart(64, '0'),
    );
    let state = createInitialUiState();
    for (const id of ids) {
      state = reducer(state, { type: 'design/received', design: designWithId(id) });
    }
    // Re-activate the oldest survivor; the next arrival must evict the one
    // that has been used least recently instead.
    state = reducer(state, { type: 'design/activate', designId: ids[1]! });
    state = reducer(state, {
      type: 'design/received',
      design: designWithId('f'.repeat(64)),
    });
    expect(state.designs.designs.has(ids[1]!)).toBe(true);
    expect(state.designs.designs.has(ids[2]!)).toBe(false);
  });

  it('activates a stored design and refuses an unknown one', () => {
    designReceived(bracketDesignResult);
    dispatch({ type: 'design/received', design: designWithId('b'.repeat(64)) });
    activateDesign(BRACKET_DESIGN_ID);
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    activateDesign('c'.repeat(64));
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
  });

  it('clears the active design without forgetting the stored ones', () => {
    designReceived(bracketDesignResult);
    clearActiveDesign();
    expect(getState().designs.activeDesignId).toBeNull();
    expect(getState().designs.designs.size).toBe(1);
    expect(selectActiveDesign(getState())).toBeNull();
  });
});

describe('the CAD target selection', () => {
  it('starts with no default provider', () => {
    expect(selectCadProvider(createInitialUiState())).toBeNull();
    expect(selectCadProviderRecord(createInitialUiState())).toBeNull();
  });

  it('records a provider choice in the session store and resolves its probe row', () => {
    let state = reducer(createInitialUiState(), {
      type: 'server/cad-providers',
      providers: cadProviders.providers,
    });
    state = reducer(state, { type: 'cad/provider-set', provider: 'freecad' });
    expect(selectCadProvider(state)).toBe('freecad');
    expect(selectCadProviderRecord(state)?.available).toBe(true);
    expect(selectCadProviderRecord(state)?.version).toBe('1.1.4');
  });

  it('refuses an unknown provider id', () => {
    let state = reducer(createInitialUiState(), { type: 'cad/provider-set', provider: 'freecad' });
    state = reducer(state, {
      type: 'cad/provider-set',
      provider: 'onshape' as unknown as 'freecad',
    });
    expect(selectCadProvider(state)).toBe('freecad');
  });

  it('clears the choice back to the no-default state', () => {
    setCadProvider('opencascade');
    expect(getState().cadProvider).toBe('opencascade');
    setCadProvider(null);
    expect(getState().cadProvider).toBeNull();
  });

  it('is not persisted to localStorage', () => {
    setCadProvider('freecad');
    try {
      expect(localStorage.getItem('kala-studio.cad-provider')).toBeNull();
    } catch {
      // A node test environment may not have localStorage at all.
    }
    resetStore();
    expect(getState().cadProvider).toBeNull();
  });

  it('records the provider probe status and error', () => {
    let state = reducer(createInitialUiState(), { type: 'server/cad-providers-start' });
    expect(state.server.cadProvidersStatus).toBe('loading');
    state = reducer(state, {
      type: 'server/cad-providers-error',
      message: '404: not found',
    });
    expect(state.server.cadProvidersStatus).toBe('error');
    expect(state.server.cadProvidersError).toBe('404: not found');
    state = reducer(state, {
      type: 'server/cad-providers',
      providers: cadProviders.providers,
    });
    expect(state.server.cadProvidersStatus).toBe('ready');
    expect(state.server.cadProviders).toHaveLength(3);
    expect(state.server.cadProvidersError).toBeNull();
  });
});

describe('the display mesh source', () => {
  it('is "none" while nothing is on the sheet', () => {
    expect(selectDisplayMeshSource(createInitialUiState())).toBe('none');
  });

  it('is "local" while only the analytic preview exists', () => {
    const state = reducer(createInitialUiState(), {
      type: 'preview/local',
      preview: {
        kind: 'box',
        parameters: { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 },
        positions: new Float32Array(9),
        normals: new Float32Array(9),
        indices: new Uint32Array(3),
        bounds: { min: [0, 0, 0], max: [1, 1, 1] },
        source: 'local',
        generatedAt: 0,
        engine: 'analytic-local',
        warnings: [],
      },
    });
    expect(selectDisplayMeshSource(state)).toBe('local');
  });

  it('is "server" once the server preview answered', () => {
    let state = reducer(createInitialUiState(), {
      type: 'preview/request-success',
      preview: {
        positions: [0, 0, 0],
        normals: [0, 0, 1],
        indices: [0, 0, 0],
        bounds: { min: [0, 0, 0], max: [1, 1, 1] },
        engine: 'analytic',
        warnings: [],
      },
    });
    expect(selectDisplayMeshSource(state)).toBe('server');
    state = reducer(state, { type: 'design/received', design: bracketDesignResult });
    expect(selectDisplayMeshSource(state)).toBe('design');
  });
});

describe('the design to view conversion', () => {
  it('produces the typed arrays the viewport renders from the imported view', () => {
    const mesh = designToDisplayMesh(bracketDesignResult);
    expect(mesh.designId).toBe(BRACKET_DESIGN_ID);
    expect(mesh.positions).toBeInstanceOf(Float32Array);
    expect(mesh.normals).toBeInstanceOf(Float32Array);
    expect(mesh.indices).toBeInstanceOf(Uint32Array);
    expect(Array.from(mesh.positions)).toEqual([-30, 0, 0, -30, 0, 8, 30, 0, 0]);
    expect(Array.from(mesh.indices)).toEqual([0, 1, 2]);
    // The bounds are the imported B-Rep's optimal box from `view.bounds`.
    expect(mesh.bounds.min).toEqual([-30, 0, 0]);
    expect(mesh.bounds.max).toEqual([30, 45, 40]);
    expect(mesh.vertexCount).toBe(3);
    expect(mesh.triangleCount).toBe(1);
    expect(mesh.solids).toBe(1);
    expect(mesh.source).toBe('design');
  });

  it('reads view bounds, not measured bounds, when they differ', () => {
    const different: DesignResult = {
      ...bracketDesignResult,
      measured: {
        ...bracketDesignResult.measured,
        bounds: { min: [0, 0, 0], max: [10, 10, 10] },
      },
      view: {
        ...bracketDesignResult.view,
        bounds: { min: [-31, -1, -2], max: [31, 46, 41] },
      },
    };
    const mesh = designToDisplayMesh(different);
    expect(mesh.bounds.min).toEqual([-31, -1, -2]);
    expect(mesh.bounds.max).toEqual([31, 46, 41]);
  });

  it('renders the imported view even when the provider reported no measurements', () => {
    const mesh = designToDisplayMesh(notReportedDesignResult);
    expect(Array.from(mesh.positions)).toEqual([-30, 0, 0, -30, 0, 8, 30, 0, 0]);
    expect(mesh.bounds).toEqual({ min: [-30, 0, 0], max: [30, 45, 40] });
  });

  it('is pure: the same design always yields the same mesh', () => {
    const a = designToDisplayMesh(bracketDesignResult);
    const b = designToDisplayMesh(bracketDesignResult);
    expect(a.positions).not.toBe(b.positions);
    expect(Array.from(a.positions)).toEqual(Array.from(b.positions));
  });

  it('memoises by design id and stays bounded', () => {
    const memo = createDesignMeshMemo(2);
    const first = memo.get(bracketDesignResult);
    expect(memo.get(bracketDesignResult)).toBe(first);
    expect(memo.size()).toBe(1);
    memo.get(designWithId('d'.repeat(64)));
    memo.get(designWithId('e'.repeat(64)));
    expect(memo.size()).toBe(2);
  });
});

describe('the drawing and section caches', () => {
  it('keys the caches by the request parameters', () => {
    expect(drawingCacheKey(BRACKET_DESIGN_ID, ['front', 'iso'])).toBe(`${BRACKET_DESIGN_ID}|front,iso`);
    expect(drawingCacheKey(BRACKET_DESIGN_ID, [])).toBe(`${BRACKET_DESIGN_ID}|all`);
    expect(sectionCacheKey(BRACKET_DESIGN_ID, 'XZ', 4)).toBe(`${BRACKET_DESIGN_ID}|XZ|4`);
    expect(sectionCacheKey(BRACKET_DESIGN_ID, 'XY')).toBe(`${BRACKET_DESIGN_ID}|XY|default`);
  });

  it('records the loading, ready, and error states', () => {
    const key = drawingCacheKey(BRACKET_DESIGN_ID, ['front']);
    let state = reducer(createInitialUiState(), { type: 'design/drawing-start', key });
    expect(state.designs.drawings.get(key)?.status).toBe('loading');
    state = reducer(state, {
      type: 'design/drawing-success',
      key,
      drawing: { design_id: BRACKET_DESIGN_ID, views: [], chord_tolerance_mm: 0.05, max_points: 1, point_count: 0 },
    });
    expect(state.designs.drawings.get(key)?.status).toBe('ready');
    expect(state.designs.drawings.get(key)?.data?.design_id).toBe(BRACKET_DESIGN_ID);
    state = reducer(state, { type: 'design/drawing-error', key, message: 'network' });
    expect(state.designs.drawings.get(key)?.status).toBe('error');
    expect(state.designs.drawings.get(key)?.error).toBe('network');
  });
});

describe('the journal merge', () => {
  it('keeps the backend events apart from the local ledger and orders them by time', () => {
    const state = reducer(createInitialUiState(), {
      type: 'journal/server-page',
      page: {
        events: [journalEvent(2, '2026-10-19T16:46:58.000Z')],
        next_after: 2,
        has_more: false,
        limit: 200,
      },
    });
    const withLocal = reducer(state, {
      type: 'journal/append',
      entry: { id: 'j-1', ts: Date.parse('2026-10-19T16:46:57.500Z'), kind: 'chat', message: 'You · hi' },
    });
    const merged = selectMergedJournal(withLocal);
    expect(merged).toHaveLength(2);
    expect(merged[0]?.source).toBe('local');
    expect(merged[1]?.source).toBe('server');
    expect(merged[1]?.sequence).toBe(2);
    expect(merged[1]?.eventType).toBe('design.measured');
    expect(merged[1]?.message).toContain('volume 35571.416 mm^3');
    expect(merged[0]?.sequence).toBeUndefined();
  });

  it('dedupes the backend events by sequence', () => {
    const page: JournalPage = {
      events: [journalEvent(1, '2026-10-19T16:46:57.000Z'), journalEvent(2, '2026-10-19T16:46:58.000Z')],
      next_after: 2,
      has_more: false,
      limit: 200,
    };
    let state = reducer(createInitialUiState(), { type: 'journal/server-page', page });
    expect(state.sessionJournal.events).toHaveLength(2);
    // The same page again must not double the ledger.
    state = reducer(state, { type: 'journal/server-page', page });
    expect(state.sessionJournal.events).toHaveLength(2);
    expect(state.sessionJournal.lastSequence).toBe(2);
  });

  it('merges a local and a backend row that share a timestamp with the backend row first', () => {    const ts = Date.parse('2026-10-19T16:46:57.000Z');
    const merged = mergeJournalEntries(
      [{ id: 'j-1', ts, kind: 'chat', message: 'You · hi' }],
      [
        {
          sequence: 1,
          eventType: 'chat.turn',
          aggregateId: BRACKET_DESIGN_ID,
          occurredAt: '2026-10-19T16:46:57.000Z',
          ts,
          summary: 'local · offline',
          digest: 'd'.repeat(64),
          eventDigest: 'e'.repeat(64),
          previousEventDigest: '0'.repeat(64),
          payload: null,
        },
      ],
      'unknown',
    );
    expect(merged).toHaveLength(2);
    expect(merged[0]?.source).toBe('server');
    expect(merged[1]?.source).toBe('local');
  });
  it('returns the same array for the same state, so the React snapshot stays stable', () => {
    const state = reducer(createInitialUiState(), {
      type: 'journal/server-page',
      page: bracketJournalPage,
    });
    expect(selectMergedJournal(state)).toBe(selectMergedJournal(state));
  });
});

describe('the chain verdict', () => {
  it('is unknown until a verification says otherwise', () => {
    const state = reducer(createInitialUiState(), {
      type: 'journal/server-page',
      page: bracketJournalPage,
    });
    expect(selectChainStatus(state)).toBe('unknown');
    expect(state.sessionJournal.firstBadSequence).toBeNull();
    expect(state.sessionJournal.headDigest).toBeNull();
  });

  it('becomes verified only from a successful verification', () => {
    const state = reducer(createInitialUiState(), {
      type: 'journal/server-verify',
      verify: bracketJournalVerified,
    });
    expect(selectChainStatus(state)).toBe('verified');
    expect(state.sessionJournal.headDigest).toBe(bracketJournalVerified.head_digest);
    expect(state.sessionJournal.eventCount).toBe(5);
    expect(state.sessionJournal.firstBadSequence).toBeNull();
  });

  it('becomes broken, with the first bad sequence, from a failed verification', () => {
    const state = reducer(createInitialUiState(), {
      type: 'journal/server-verify',
      verify: bracketJournalBroken,
    });
    expect(selectChainStatus(state)).toBe('broken');
    expect(state.sessionJournal.firstBadSequence).toBe(4);
    expect(state.sessionJournal.headDigest).toBe(bracketJournalBroken.head_digest);
  });

  it('never returns to verified without a new verification', () => {
    let state = reducer(createInitialUiState(), {
      type: 'journal/server-verify',
      verify: bracketJournalBroken,
    });
    expect(selectChainStatus(state)).toBe('broken');
    // A new page does not change the verdict.
    state = reducer(state, { type: 'journal/server-page', page: bracketJournalPage });
    expect(selectChainStatus(state)).toBe('broken');
    // A failed poll does not change it either.
    state = reducer(state, { type: 'journal/server-error', message: 'network' });
    expect(selectChainStatus(state)).toBe('broken');
    expect(state.sessionJournal.lastError).toBe('network');
  });
});

describe('the store keeps the design slice out of the local ledger', () => {
  it('does not turn a design into a local journal row', () => {
    const state: UiState = reducer(createInitialUiState(), {
      type: 'design/received',
      design: bracketDesignResult,
    });
    expect(state.journal).toHaveLength(0);
    expect(state.designs.designs.size).toBe(1);
  });
});

describe('the B2 journal summaries', () => {
  it('names the provider on a build and a measurement', () => {
    expect(
      summarisePayload('design.intent', { template: 'l_bracket', provider: 'freecad', design_id: BRACKET_DESIGN_ID }),
    ).toContain('provider freecad');
    expect(
      summarisePayload('design.measured', {
        provider: 'freecad',
        measured_by: 'FreeCAD',
        volume_mm3: 35571.416047,
        surface_area_mm2: 11889.101731,
        solid_count: 1,
      }),
    ).toContain('measured by FreeCAD');
    expect(
      summarisePayload('provider.build.completed', {
        provider: 'freecad',
        provider_version: '1.1.4',
        step_sha256: '24a5dca7bf55b1bcc2d4f33e796a739c210b2f85a89c964a7d59393f2bd58403',
      }),
    ).toContain('freecad 1.1.4');
  });

  it('does not count a not-reported check as passed or failed', () => {
    const summary = summarisePayload('design.checked', {
      verified: false,
      checks: [
        { id: 'brep_valid', status: 'not_reported', passed: false },
        { id: 'single_solid', status: 'not_reported', passed: false },
      ],
    });
    expect(summary).toContain('2 not reported');
    expect(summary).not.toContain('checks passed');
    expect(summary).not.toContain('checks failed');
  });

  it('reports a provider refusal with its reason', () => {
    const summary = summarisePayload('provider.build.failed', {
      provider: 'solidedge',
      reason: 'Solid Edge build requires the Windows bridge',
    });
    expect(summary).toContain('solidedge');
    expect(summary).toContain('Windows bridge');
  });
});
