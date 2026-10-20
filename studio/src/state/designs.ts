// The design slice of the studio store.
//
// A design turn produces a `DesignResult`: a provider-built, provider-measured
// part with its own id, digests, and the OpenCascade view imported from the
// provider's STEP. The store keeps the most recent designs in a bounded LRU
// (16 entries) so a long session cannot grow without limit, and keeps the
// per-design drawing and section responses in their own caches keyed by the
// request parameters.
//
// Nothing in this module invents a number. The view is the OpenCascade import
// of the provider's STEP, the measurements are the provider's measurements,
// and the caches only ever hold a response that passed the client-side
// contract check.

import type {
  BBox,
  DesignResult,
  DesignViewName,
  DrawingResponse,
  SectionPlane,
  SectionResponse,
} from '../types/studio';
import { DESIGN_LRU_CAP } from '../types/studio';

/** The lifecycle of one cached backend resource. */
export type ResourceStatus = 'idle' | 'loading' | 'ready' | 'error';

/** One cached drawing or section response. */
export interface DesignResource<T> {
  status: ResourceStatus;
  data: T | null;
  error: string | null;
}

export interface DesignState {
  /** design_id -> the measured design, bounded to `DESIGN_LRU_CAP` entries. */
  readonly designs: ReadonlyMap<string, DesignResult>;
  /** The LRU order of `designs`: least recently used first. */
  readonly order: ReadonlyArray<string>;
  /** The design the sheet, the aux view, and the export act on. */
  readonly activeDesignId: string | null;
  /** Drawing responses, keyed by `drawingCacheKey`. */
  readonly drawings: ReadonlyMap<string, DesignResource<DrawingResponse>>;
  /** Section responses, keyed by `sectionCacheKey`. */
  readonly sections: ReadonlyMap<string, DesignResource<SectionResponse>>;
  /** The last design error (a refused turn, a failed load). */
  readonly lastError: string | null;
}

export type DesignAction =
  | { type: 'design/received'; design: DesignResult }
  | { type: 'design/activate'; designId: string | null }
  | { type: 'design/clear-active' }
  | { type: 'design/drawing-start'; key: string }
  | { type: 'design/drawing-success'; key: string; drawing: DrawingResponse }
  | { type: 'design/drawing-error'; key: string; message: string }
  | { type: 'design/section-start'; key: string }
  | { type: 'design/section-success'; key: string; section: SectionResponse }
  | { type: 'design/section-error'; key: string; message: string };

export function createDesignState(): DesignState {
  return {
    designs: new Map<string, DesignResult>(),
    order: [],
    activeDesignId: null,
    drawings: new Map<string, DesignResource<DrawingResponse>>(),
    sections: new Map<string, DesignResource<SectionResponse>>(),
    lastError: null,
  };
}

/** The cache key of a drawing request. */
export function drawingCacheKey(designId: string, views: ReadonlyArray<DesignViewName>): string {
  return `${designId}|${views.length === 0 ? 'all' : [...views].join(',')}`;
}

/** The cache key of a section request. */
export function sectionCacheKey(designId: string, plane: SectionPlane, offsetMm?: number): string {
  return `${designId}|${plane}|${offsetMm === undefined ? 'default' : offsetMm}`;
}

/** Move a design to the most-recently-used end of the LRU order. */
function touch(order: ReadonlyArray<string>, designId: string): ReadonlyArray<string> {
  return [...order.filter((id) => id !== designId), designId];
}

function resource<T>(status: ResourceStatus, data: T | null, error: string | null): DesignResource<T> {
  return { status, data, error };
}

/**
 * The pure design-slice reducer. It never mutates the state it is given;
 * every branch returns a new object graph.
 */
export function designReducer(state: DesignState, action: DesignAction): DesignState {
  switch (action.type) {
    case 'design/received': {
      const design = action.design;
      const designs = new Map(state.designs);
      designs.set(design.design_id, design);
      const order = [...touch(state.order, design.design_id)];
      // Evict the least recently used designs beyond the cap.
      while (order.length > DESIGN_LRU_CAP) {
        const oldest = order[0];
        if (oldest === undefined) break;
        order.shift();
        designs.delete(oldest);
      }
      return {
        ...state,
        designs,
        order,
        // A fresh design is the one the operator is looking at.
        activeDesignId: design.design_id,
        lastError: null,
      };
    }
    case 'design/activate': {
      if (action.designId === null) {
        return state.activeDesignId === null ? state : { ...state, activeDesignId: null };
      }
      // Activating a design the store does not hold is refused: the sheet
      // tabs are built from the LRU, so this cannot happen in the UI.
      if (!state.designs.has(action.designId)) return state;
      return {
        ...state,
        activeDesignId: action.designId,
        order: touch(state.order, action.designId),
      };
    }
    case 'design/clear-active': {
      return state.activeDesignId === null ? state : { ...state, activeDesignId: null };
    }
    case 'design/drawing-start': {
      const drawings = new Map(state.drawings);
      drawings.set(action.key, resource<DrawingResponse>('loading', null, null));
      return { ...state, drawings };
    }
    case 'design/drawing-success': {
      const drawings = new Map(state.drawings);
      drawings.set(action.key, resource<DrawingResponse>('ready', action.drawing, null));
      return { ...state, drawings, lastError: null };
    }
    case 'design/drawing-error': {
      const drawings = new Map(state.drawings);
      drawings.set(action.key, resource<DrawingResponse>('error', null, action.message));
      return { ...state, drawings, lastError: action.message };
    }
    case 'design/section-start': {
      const sections = new Map(state.sections);
      sections.set(action.key, resource<SectionResponse>('loading', null, null));
      return { ...state, sections };
    }
    case 'design/section-success': {
      const sections = new Map(state.sections);
      sections.set(action.key, resource<SectionResponse>('ready', action.section, null));
      return { ...state, sections, lastError: null };
    }
    case 'design/section-error': {
      const sections = new Map(state.sections);
      sections.set(action.key, resource<SectionResponse>('error', null, action.message));
      return { ...state, sections, lastError: action.message };
    }
  }
}

/** The design the store is currently showing, if any. */
export function selectActiveDesignFrom(state: DesignState): DesignResult | null {
  if (state.activeDesignId === null) return null;
  return state.designs.get(state.activeDesignId) ?? null;
}

/** The cached drawing for a key, or null when it was never requested. */
export function selectDrawingFrom(
  state: DesignState,
  key: string,
): DesignResource<DrawingResponse> | null {
  return state.drawings.get(key) ?? null;
}

/** The cached section for a key, or null when it was never requested. */
export function selectSectionFrom(
  state: DesignState,
  key: string,
): DesignResource<SectionResponse> | null {
  return state.sections.get(key) ?? null;
}

// --- The viewer mesh ----------------------------------------------------------

/**
 * The mesh the viewport renders for a design: the OpenCascade import of the
 * provider's exact STEP bytes. The arrays are typed so the three.js buffer
 * attributes can be built without copying twice, and the bounds are the
 * imported B-Rep's optimal box (`view.bounds`), not the tessellation's
 * extents and not a re-computed envelope.
 */
export interface DesignDisplayMesh {
  designId: string;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  bounds: BBox;
  vertexCount: number;
  triangleCount: number;
  /** The number of solids in the imported B-Rep. */
  solids: number;
  engine: string;
  warnings: ReadonlyArray<string>;
  source: 'design';
}

/**
 * Convert a design's imported view into the mesh the viewport renders. The
 * conversion is pure: the same design always yields the same mesh. It reads
 * `design.view`, the B2 field; the removed B1 `design.mesh` is never used.
 */
export function designToDisplayMesh(design: DesignResult): DesignDisplayMesh {
  const view = design.view;
  return {
    designId: design.design_id,
    positions: new Float32Array(view.positions),
    normals: new Float32Array(view.normals),
    indices: new Uint32Array(view.indices),
    bounds: {
      min: [view.bounds.min[0], view.bounds.min[1], view.bounds.min[2]],
      max: [view.bounds.max[0], view.bounds.max[1], view.bounds.max[2]],
    },
    vertexCount: view.vertex_count,
    triangleCount: view.triangle_count,
    solids: view.solids,
    engine: view.engine,
    warnings: [...design.warnings, ...view.warnings],
    source: 'design',
  };
}

/** A bounded, memoising view of `designToDisplayMesh`. */
export interface DesignMeshMemo {
  get(design: DesignResult): DesignDisplayMesh;
  size(): number;
  clear(): void;
}

/**
 * Memoise the mesh conversion by design id. The memo is bounded, so a long
 * session cannot leak a typed array per design.
 */
export function createDesignMeshMemo(cap: number = DESIGN_LRU_CAP): DesignMeshMemo {
  const cache = new Map<string, DesignDisplayMesh>();
  const limit = Math.max(1, Math.floor(cap));
  return {
    get(design: DesignResult): DesignDisplayMesh {
      const hit = cache.get(design.design_id);
      if (hit) {
        // Refresh the LRU position.
        cache.delete(design.design_id);
        cache.set(design.design_id, hit);
        return hit;
      }
      const mesh = designToDisplayMesh(design);
      cache.set(design.design_id, mesh);
      while (cache.size > limit) {
        const oldest = cache.keys().next();
        if (oldest.done) break;
        cache.delete(oldest.value);
      }
      return mesh;
    },
    size(): number {
      return cache.size;
    },
    clear(): void {
      cache.clear();
    },
  };
}
