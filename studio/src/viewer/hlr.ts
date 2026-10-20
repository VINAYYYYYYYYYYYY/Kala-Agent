// Pure geometry for the 2D pencil HLR drawing.
//
// The B1 drawing contract returns polylines in *view-plane millimetres*: the
// pair is (horizontal, vertical) for the named view. This module owns the
// three things that must be shared by the view, the tests, and the
// section panel so none of them invents a second layout rule:
//
//   * fitting one or more polyline groups into a sheet cell, preserving
//     aspect ratio and flipping the vertical axis for SVG;
//   * building deterministic SVG path strings (visible and hidden lines);
//   * the pan/zoom transform, including the reset state and cursor-centred
//     wheel zoom.
//
// Nothing here computes a measured number: the callouts read the kernel's
// `measured.bounds`, and the polylines are the backend's HLR output.

import type {
  DesignMeasured,
  DesignViewName,
  DrawingPolyline,
  SectionPlane,
  SectionResponse,
} from '../types/studio';
import { fitToBox } from './draftingGeometry';

/** A rectangular region of the drawing sheet, in SVG units. */
export interface DrawingCell {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The view-plane extent of a polyline group, in millimetres. */
export interface PolylineExtent {
  minU: number;
  minV: number;
  maxU: number;
  maxV: number;
  width: number;
  height: number;
}

/** The fitted result of one or more polyline groups in one cell. */
export interface PolylineFit {
  extent: PolylineExtent;
  /** Millimetres → SVG units. */
  scale: number;
  /** SVG x of `extent.minU`. */
  originX: number;
  /** SVG y of `extent.maxV` (the vertical axis is flipped for SVG). */
  originY: number;
  /** One path per input group, in the same order. */
  paths: string[];
}

/** The pan/zoom state of the drawing. */
export interface DrawingTransform {
  zoom: number;
  /** Screen-space translation in SVG units, applied after the scale. */
  panX: number;
  panY: number;
}

export const MIN_DRAWING_ZOOM = 0.5;
export const MAX_DRAWING_ZOOM = 8;

export const IDENTITY_TRANSFORM: DrawingTransform = { zoom: 1, panX: 0, panY: 0 };

/**
 * The view basis the Python backend uses (`drawing.py`, `_VIEW_BASES`).
 *
 * `viewer` is the unit direction *from the part towards the viewer*; the
 * backend hands OpenCascade the opposite direction as the projection
 * direction. `horizontal` is the view-plane u axis; the vertical axis is
 * `viewer × horizontal`, so every basis is right-handed and the local frame
 * matches the drawing sheet.
 *
 * The reading labels on the HLR sheet are derived from this object, not
 * written by hand, so a caption can never name a different direction from
 * the polylines beside it.
 */
export interface DesignViewBasis {
  viewer: readonly [number, number, number];
  horizontal: string;
  vertical: string;
}

const ISO_VIEWER: readonly [number, number, number] = [
  1 / Math.sqrt(3),
  -1 / Math.sqrt(3),
  1 / Math.sqrt(3),
];

export const DESIGN_VIEW_BASES: Readonly<Record<DesignViewName, DesignViewBasis>> = {
  // front looks from -Y towards +Y; X is horizontal, Z is vertical.
  front: { viewer: [0, -1, 0], horizontal: 'X', vertical: 'Z' },
  // top looks from +Z towards -Z; X is horizontal, Y is vertical.
  top: { viewer: [0, 0, 1], horizontal: 'X', vertical: 'Y' },
  // right looks from +X towards -X; Y is horizontal, Z is vertical.
  right: { viewer: [1, 0, 0], horizontal: 'Y', vertical: 'Z' },
  // iso looks from (+X, -Y, +Z) towards the origin.
  iso: { viewer: ISO_VIEWER, horizontal: 'X+Y', vertical: 'X−Y+2Z' },
};

const AXIS_NAMES = ['X', 'Y', 'Z'] as const;

function signedAxis(value: number, axis: number, tolerance = 1e-9): string | null {
  if (Math.abs(value) <= tolerance) return null;
  return `${value > 0 ? '+' : '−'}${AXIS_NAMES[axis]}`;
}

/** `+Y`, `−Z`, …: the axis and sign the viewer looks along (-viewer). */
export function viewLookDirection(viewer: readonly [number, number, number]): string | null {
  const parts: string[] = [];
  for (let axis = 0; axis < 3; axis += 1) {
    const label = signedAxis(-viewer[axis]!, axis);
    if (label !== null) parts.push(label);
  }
  return parts.length === 0 ? null : parts.join(' ');
}

/** `+X −Y +Z`: where the viewer stands, used for an axonometric view. */
export function viewViewerPosition(viewer: readonly [number, number, number]): string | null {
  const parts: string[] = [];
  for (let axis = 0; axis < 3; axis += 1) {
    const label = signedAxis(viewer[axis]!, axis);
    if (label !== null) parts.push(label);
  }
  return parts.length === 0 ? null : parts.join(' ');
}

/**
 * The caption under a view, derived from the same basis the backend used.
 *
 * Orthographic views read `FRONT · LOOKING +Y` (the front viewer stands at
 * −Y and looks towards +Y); the axonometric view reads
 * `ISO · FROM (+X −Y +Z)` because a three-axis look direction is not a
 * single signed axis.
 */
export function designViewCaption(name: DesignViewName): string {
  const basis = DESIGN_VIEW_BASES[name]!;
  if (name === 'iso') {
    const position = viewViewerPosition(basis.viewer);
    return position ? `ISO · FROM (${position})` : 'ISO · HLR';
  }
  const direction = viewLookDirection(basis.viewer);
  return direction ? `${name.toUpperCase()} · LOOKING ${direction}` : name.toUpperCase();
}


/**
 * The extent of every finite point in every supplied group. Returns null when
 * there is nothing drawable, so the caller can render an honest empty state.
 */
export function polylinesExtent(
  groups: ReadonlyArray<ReadonlyArray<DrawingPolyline>>,
): PolylineExtent | null {
  let minU = Number.POSITIVE_INFINITY;
  let minV = Number.POSITIVE_INFINITY;
  let maxU = Number.NEGATIVE_INFINITY;
  let maxV = Number.NEGATIVE_INFINITY;
  let seen = false;
  for (const group of groups) {
    for (const polyline of group) {
      for (const point of polyline.points) {
        const u = point[0];
        const v = point[1];
        if (!Number.isFinite(u) || !Number.isFinite(v)) continue;
        seen = true;
        if (u < minU) minU = u;
        if (u > maxU) maxU = u;
        if (v < minV) minV = v;
        if (v > maxV) maxV = v;
      }
    }
  }
  if (!seen) return null;
  return {
    minU,
    minV,
    maxU,
    maxV,
    width: maxU - minU,
    height: maxV - minV,
  };
}

/**
 * Fit polyline groups into a cell. All groups share one transform so the
 * visible and hidden lines of a view cannot drift apart.
 */
export function fitPolylineGroups(
  groups: ReadonlyArray<ReadonlyArray<DrawingPolyline>>,
  cell: DrawingCell,
  padding: number,
): PolylineFit | null {
  const extent = polylinesExtent(groups);
  if (!extent) return null;
  if (!(cell.width > 0) || !(cell.height > 0)) return null;
  // A perfectly straight line has a zero extent on one axis. Give the fit a
  // microscopic second dimension so the line still gets a real scale.
  const contentWidth = Math.max(extent.width, 1e-6);
  const contentHeight = Math.max(extent.height, 1e-6);
  const fit = fitToBox(cell.width, cell.height, contentWidth, contentHeight, padding);
  if (!(fit.scale > 0)) return null;
  const originX = cell.x + fit.x;
  const originY = cell.y + fit.y;
  const map = (u: number, v: number): [number, number] => [
    originX + (u - extent.minU) * fit.scale,
    originY + (extent.maxV - v) * fit.scale,
  ];
  return {
    extent,
    scale: fit.scale,
    originX,
    originY,
    paths: groups.map((group) =>
      group
        .map((polyline) => polylinePath(polyline, map))
        .filter((path) => path.length > 0)
        .join(' '),
    ),
  };
}

/** Build one SVG path from a polyline, with the vertical axis flipped. */
export function polylinePath(
  polyline: DrawingPolyline,
  map: (u: number, v: number) => [number, number],
): string {
  const points = polyline.points;
  if (points.length === 0) return '';
  const parts: string[] = [];
  let started = false;
  for (const point of points) {
    const u = point[0];
    const v = point[1];
    if (!Number.isFinite(u) || !Number.isFinite(v)) continue;
    const [x, y] = map(u, v);
    parts.push(`${started ? 'L' : 'M'} ${formatPathNumber(x)} ${formatPathNumber(y)}`);
    started = true;
  }
  if (!started) return '';
  if (polyline.closed) parts.push('Z');
  return parts.join(' ');
}

function formatPathNumber(value: number): string {
  const rounded = Math.round(value * 1000) / 1000;
  return (rounded === 0 ? 0 : rounded).toString();
}

/** Clamp the drawing zoom to its honest bounds. */
export function clampDrawingZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return Math.min(MAX_DRAWING_ZOOM, Math.max(MIN_DRAWING_ZOOM, zoom));
}

/**
 * Zoom around a point in SVG coordinates. The point under the cursor stays
 * under the cursor: `p = pan + zoom * original`.
 */
export function zoomDrawingAt(
  transform: DrawingTransform,
  point: { x: number; y: number },
  factor: number,
): DrawingTransform {
  if (!Number.isFinite(factor) || factor <= 0) return transform;
  const nextZoom = clampDrawingZoom(transform.zoom * factor);
  if (nextZoom === transform.zoom) return transform;
  const ratio = nextZoom / transform.zoom;
  return {
    zoom: nextZoom,
    panX: point.x - (point.x - transform.panX) * ratio,
    panY: point.y - (point.y - transform.panY) * ratio,
  };
}

/** Translate the drawing by an SVG-space delta. */
export function panDrawing(
  transform: DrawingTransform,
  dx: number,
  dy: number,
): DrawingTransform {
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return transform;
  if (dx === 0 && dy === 0) return transform;
  return { ...transform, panX: transform.panX + dx, panY: transform.panY + dy };
}

/** The SVG transform string for the pan/zoom group. */
export function drawingTransformAttribute(transform: DrawingTransform): string {
  return `translate(${formatPathNumber(transform.panX)} ${formatPathNumber(transform.panY)}) scale(${formatPathNumber(transform.zoom)})`;
}

/** The measured envelope, in millimetres, from the kernel's bounds. */
export interface MeasuredEnvelope {
  x: number;
  y: number;
  z: number;
}

export function measuredEnvelope(measured: DesignMeasured): MeasuredEnvelope | null {
  const bounds = measured.bounds;
  // A provider that reported no envelope leaves this null; the drawing then
  // carries no callouts rather than a substituted size.
  if (!bounds) return null;
  const min = bounds.min;
  const max = bounds.max;
  const values = [min[0], min[1], min[2], max[0], max[1], max[2]];
  if (values.some((value) => !Number.isFinite(value))) return null;
  return {
    x: max[0] - min[0],
    y: max[1] - min[1],
    z: max[2] - min[2],
  };
}

/**
 * The measured axis each orthographic view carries. This is A2's published
 * view basis, not a guess: front is X horizontal / Z vertical, top is X / Y,
 * and right is Y / Z.
 */
export const VIEW_MEASURED_AXES: Readonly<
  Record<'front' | 'top' | 'right', { horizontal: 'X' | 'Y' | 'Z'; vertical: 'X' | 'Y' | 'Z' }>
> = {
  front: { horizontal: 'X', vertical: 'Z' },
  top: { horizontal: 'X', vertical: 'Y' },
  right: { horizontal: 'Y', vertical: 'Z' },
};

/** The value of a measured axis by name. */
export function measuredAxisValue(
  envelope: MeasuredEnvelope,
  axis: 'X' | 'Y' | 'Z',
): number {
  if (axis === 'X') return envelope.x;
  if (axis === 'Y') return envelope.y;
  return envelope.z;
}

// --- The shared section trace ------------------------------------------------

/**
 * The orthographic view whose plane is parallel to a section plane, i.e. the
 * view where the cut profile appears in true shape:
 *   XY cut → top view (X horizontal, Y vertical)
 *   XZ cut → front view (X horizontal, Z vertical)
 *   YZ cut → right view (Y horizontal, Z vertical)
 * That is also the view whose basis is the section plane's own (u, v) frame.
 */
export const SECTION_VIEW_FOR_PLANE: Readonly<Record<SectionPlane, 'top' | 'front' | 'right'>> = {
  XY: 'top',
  XZ: 'front',
  YZ: 'right',
};

/** The iso projection used by the backend: u = (x+y)/√2, v = (−x+y+2z)/√6. */
export function projectToIso(point: readonly [number, number, number]): [number, number] {
  const [x, y, z] = point;
  return [
    (x + y) / Math.SQRT2,
    (-x + y + 2 * z) / Math.sqrt(6),
  ];
}

/** Map a plane coordinate (u, v) to model millimetres for a section plane. */
export function sectionPointToModel(
  plane: SectionPlane,
  u: number,
  v: number,
  offsetMm: number,
): [number, number, number] {
  if (plane === 'XY') return [u, v, offsetMm];
  if (plane === 'XZ') return [u, offsetMm, v];
  return [offsetMm, u, v];
}

/**
 * The section loops in the local coordinates of one HLR view.
 *
 * For the orthographic view coplanar with the cut, the section's (u, v)
 * coordinates *are* that view's coordinates. For the iso view the loops are
 * lifted back to model space at the measured offset and projected with the
 * backend's iso basis. A plane that is not the section's plane yields an
 * empty list: the drawing never draws a cut that was not measured.
 */
export function sectionLoopsInView(
  section: SectionResponse,
  view: DesignViewName,
): DrawingPolyline[] {
  if (section.plane !== 'XZ' && section.plane !== 'XY' && section.plane !== 'YZ') return [];
  const offset = section.offset_mm;
  if (!Number.isFinite(offset)) return [];
  const coplanar = SECTION_VIEW_FOR_PLANE[section.plane];
  if (view !== 'iso' && view !== coplanar) return [];
  const polylines: DrawingPolyline[] = [];
  for (const loop of section.loops) {
    const points: Array<[number, number]> = [];
    for (const point of loop.points) {
      if (!Number.isFinite(point[0]) || !Number.isFinite(point[1])) continue;
      if (view === 'iso') {
        points.push(projectToIso(sectionPointToModel(section.plane, point[0], point[1], offset)));
      } else {
        points.push([point[0], point[1]]);
      }
    }
    if (points.length < 2) continue;
    polylines.push({
      points,
      closed: true,
      point_count: points.length,
    });
  }
  return polylines;
}

/** The SVG point of a (u, v) pair under a fitted transform. */
export function fitPoint(fit: PolylineFit, u: number, v: number): [number, number] {
  return [
    fit.originX + (u - fit.extent.minU) * fit.scale,
    fit.originY + (fit.extent.maxV - v) * fit.scale,
  ];
}
