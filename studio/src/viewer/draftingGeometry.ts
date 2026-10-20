// Pure drafting geometry for the auxiliary Section A-A and orthographic
// views.
//
// Everything here is derived from the frozen primitive contract: either the
// active parameters (Section A-A) or the real mesh bounds (orthographic
// projections). Nothing is inferred from chat text, and no tolerance or
// verification value exists in this module.
//
// Conventions used by both the local tessellators and the viewport:
//   * millimetres, right-handed, +Y up, origin at the solid centre;
//   * box: size_x (X) × size_y (Y) × size_z (Z);
//   * cylinder: axis along Z, radius in the XY plane, ±height/2 in Z;
//   * sphere: centred on the origin with the poles on Z.
//
// Section A-A is the plane at the real Z mid-plane of the mesh bounds,
// `z = (z0 + z1) / 2`. Every primitive this studio builds is centred on the
// origin, so that mid-plane is `z = 0`; the value is still read from the
// bounds so a mesh that is *not* centred is cut and labelled at its own
// mid-plane instead of a hard-coded zero. For a box that is a width ×
// height rectangle, for a cylinder it is the circular cross-section normal
// to its axis, and for a sphere it is a great circle. No other primitive has
// a section defined here, so the caller must render an explicit unavailable
// state instead of guessing.
//
// The camera-framing helpers at the bottom of this module answer a different
// question — whether a new mesh justifies re-framing the operator's view —
// and are pure so the viewport and the test suite can share one rule.
//
// Orthographic views always use the AABB (`bounds`) that the renderer and
// backend already report:
//   front: looking along -Z → X horizontal, Y vertical
//   top:   looking along -Y → X horizontal, Z vertical
//   right: looking along +X → Z horizontal, Y vertical

import type {
  BBox,
  BoxParameters,
  CylinderParameters,
  PrimitiveKind,
  SphereParameters,
  StudioParameters,
} from '../types/studio';
import { boundsDimensions } from './localPrimitives';

export interface SectionView {
  /** Drawable profile kind. `unavailable` has no dimensions to draw. */
  kind: 'rectangle' | 'circle' | 'unavailable';
  /** Profile width in millimetres (X extent for a rectangle). */
  widthMm: number;
  /** Profile height in millimetres (Y extent for a rectangle). */
  heightMm: number;
  /** Real cutting-plane description, e.g. `Z = 15.00 mm · mid-plane`. */
  planeLabel: string;
  /** The cutting plane's Z coordinate in millimetres (the bounds mid-plane). */
  planeZMm?: number;
  /** Why the profile is unavailable. */
  reason?: string;
}

/**
 * The drawing scale of the sheet.
 *
 * The viewport frames the mesh to fit the canvas (see `frameOnBounds` in
 * `SceneCanvas`), so the sheet has no fixed model-to-paper ratio. Claiming
 * `1:1` would be false, so the sheet and the bottom bar both state `FIT`.
 */
export const DRAFTING_SCALE = 'FIT';

/** A point in model millimetres. */
export interface Vec3Mm {
  x: number;
  y: number;
  z: number;
}

/**
 * One drafted centre line: a chain-dash axis through the part centre.
 */
export interface CentreLineAxis {
  id: 'x' | 'y';
  /** Start of the line, in model millimetres. */
  from: Vec3Mm;
  /** End of the line, in model millimetres. */
  to: Vec3Mm;
  /** Drafting label, e.g. `+X (WIDTH)`. */
  label: string;
}

/**
 * The two drafted centre lines through the part centre: the X axis (the
 * width) and the Y axis (the height), both on the mid-plane of the bounds.
 * `pad` extends each line past the part so the chain-dash reads as a centre
 * line and not as an edge.
 */
export function computeCentreLineAxes(bounds: BBox, pad: number): CentreLineAxis[] {
  const centreX = (bounds.min[0] + bounds.max[0]) / 2;
  const centreY = (bounds.min[1] + bounds.max[1]) / 2;
  const centreZ = (bounds.min[2] + bounds.max[2]) / 2;
  const margin = Number.isFinite(pad) ? Math.max(pad, 0) : 0;
  return [
    {
      id: 'x',
      from: { x: bounds.min[0] - margin, y: centreY, z: centreZ },
      to: { x: bounds.max[0] + margin, y: centreY, z: centreZ },
      label: '+X (WIDTH)',
    },
    {
      id: 'y',
      from: { x: centreX, y: bounds.min[1] - margin, z: centreZ },
      to: { x: centreX, y: bounds.max[1] + margin, z: centreZ },
      label: '+Y (HEIGHT)',
    },
  ];
}

/**
 * Chain-dash (— · —) segments along a straight line.
 *
 * The pattern alternates dash and gap lengths, starting with a dash:
 * `[dash, gap, dash, gap]`. A dash of 14 and a gap of 3 with a second dash
 * of 3 gives the drafted `14, 3, 3, 3` chain-dash used for centre lines.
 * The returned pairs are the visible dash segments; the gaps are the space
 * between them.
 */
export function chainDashSegments(
  from: Vec3Mm,
  to: Vec3Mm,
  pattern: ReadonlyArray<number>,
): Array<[Vec3Mm, Vec3Mm]> {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const dz = to.z - from.z;
  const length = Math.sqrt(dx * dx + dy * dy + dz * dz);
  if (!Number.isFinite(length) || length <= 0) return [];
  const steps = pattern.filter((value) => Number.isFinite(value) && value > 0);
  if (steps.length === 0) return [];
  const ux = dx / length;
  const uy = dy / length;
  const uz = dz / length;
  const segments: Array<[Vec3Mm, Vec3Mm]> = [];
  let travelled = 0;
  let index = 0;
  // Guard against a pattern that sums to zero: the loop always advances.
  while (travelled < length && segments.length < 4096) {
    const step = steps[index % steps.length]!;
    const isDash = index % 2 === 0;
    if (isDash) {
      const end = Math.min(travelled + step, length);
      segments.push([
        { x: from.x + ux * travelled, y: from.y + uy * travelled, z: from.z + uz * travelled },
        { x: from.x + ux * end, y: from.y + uy * end, z: from.z + uz * end },
      ]);
      travelled = end;
    } else {
      travelled = Math.min(travelled + step, length);
    }
    index += 1;
  }
  return segments;
}

export interface OrthoView {
  id: 'front' | 'top' | 'right';
  /** Display label, e.g. `FRONT`. */
  label: string;
  /** Direction the eye looks from, as drafting convention text. */
  direction: string;
  /** Horizontal extent of the projected silhouette in millimetres. */
  widthMm: number;
  /** Vertical extent of the projected silhouette in millimetres. */
  heightMm: number;
  kind: 'rect' | 'circle';
}

const EPSILON_MM = 0.001;

/** True when two extents describe a circle to drafting tolerance. */
export function isCircularExtent(a: number, b: number): boolean {
  if (!Number.isFinite(a) || !Number.isFinite(b)) return false;
  return Math.abs(a - b) <= Math.max(EPSILON_MM, Math.max(a, b) * 0.001);
}

/**
 * Z coordinate of the Section A-A cutting plane: the real mid-plane of the
 * mesh bounds, `z = (z0 + z1) / 2`. A mesh that is centred on the origin
 * reports `0`; a mesh that is not centred reports its own mid-plane.
 * Missing or non-finite bounds fall back to `0`, which is the centre of
 * every primitive in the frozen primitive contract.
 */
export function sectionPlaneZ(bounds: BBox | null | undefined): number {
  if (!bounds) return 0;
  const z0 = bounds.min[2];
  const z1 = bounds.max[2];
  if (!Number.isFinite(z0) || !Number.isFinite(z1)) return 0;
  return (z0 + z1) / 2;
}

/** Drafting text for a cutting plane, e.g. `Z = 15.00 mm · mid-plane`. */
export function formatSectionPlaneLabel(planeZ: number, note: string): string {
  return `Z = ${formatPlaneValue(planeZ)} mm · ${note}`;
}

/**
 * Section A-A for the active primitive. The plane is the real Z mid-plane of
 * the supplied mesh bounds. When no bounds are available the primitive
 * contract is used instead: every solid this studio builds is centred on the
 * origin, so the mid-plane is `z = 0`.
 */
export function computeSectionView(
  kind: PrimitiveKind,
  parameters: StudioParameters,
  bounds?: BBox | null,
): SectionView {
  const planeZ = sectionPlaneZ(bounds);
  const unavailable = (reason: string): SectionView => ({
    kind: 'unavailable',
    widthMm: 0,
    heightMm: 0,
    planeLabel: formatSectionPlaneLabel(planeZ, 'mid-plane'),
    planeZMm: planeZ,
    reason,
  });

  if (kind === 'box') {
    const p = parameters as BoxParameters;
    if (!positive(p.size_x) || !positive(p.size_y)) {
      return unavailable('Box parameters are not valid, so no section is drawn.');
    }
    return {
      kind: 'rectangle',
      widthMm: p.size_x,
      heightMm: p.size_y,
      planeLabel: formatSectionPlaneLabel(planeZ, 'mid-plane'),
      planeZMm: planeZ,
    };
  }

  if (kind === 'cylinder') {
    const p = parameters as CylinderParameters;
    const diameter = p.radius * 2;
    if (!positive(diameter)) {
      return unavailable('Cylinder parameters are not valid, so no section is drawn.');
    }
    return {
      kind: 'circle',
      widthMm: diameter,
      heightMm: diameter,
      planeLabel: formatSectionPlaneLabel(planeZ, 'normal to axis'),
      planeZMm: planeZ,
    };
  }

  if (kind === 'sphere') {
    const p = parameters as SphereParameters;
    const diameter = p.radius * 2;
    if (!positive(diameter)) {
      return unavailable('Sphere parameters are not valid, so no section is drawn.');
    }
    return {
      kind: 'circle',
      widthMm: diameter,
      heightMm: diameter,
      planeLabel: formatSectionPlaneLabel(planeZ, 'great circle'),
      planeZMm: planeZ,
    };
  }

  // Runtime guard for a primitive this build does not know how to cut.
  return unavailable(`Section A–A is unavailable for primitive "${String(kind)}".`);
}

/**
 * Three real orthographic silhouettes from the current mesh bounds. Returns
 * `null` when no mesh bounds exist yet (nothing to project) or when the
 * bounds are degenerate/non-finite. Callers must render an honest empty
 * state rather than a placeholder shape.
 */
export function computeOrthoViews(
  kind: PrimitiveKind,
  parameters: StudioParameters,
  bounds: BBox | null,
): OrthoView[] | null {
  if (!bounds) return null;
  const dims = boundsDimensions(bounds);
  if (
    !positive(dims.width) ||
    !positive(dims.height) ||
    !positive(dims.depth)
  ) {
    return null;
  }

  void parameters;
  const front = orthoShape(kind, 'front', dims.width, dims.height);
  const top = orthoShape(kind, 'top', dims.width, dims.depth);
  const right = orthoShape(kind, 'right', dims.depth, dims.height);

  return [
    {
      id: 'front',
      label: 'FRONT',
      direction: 'LOOKING −Z',
      widthMm: dims.width,
      heightMm: dims.height,
      kind: front,
    },
    {
      id: 'top',
      label: 'TOP',
      direction: 'LOOKING −Y',
      widthMm: dims.width,
      heightMm: dims.depth,
      kind: top,
    },
    {
      id: 'right',
      label: 'RIGHT',
      direction: 'LOOKING +X',
      widthMm: dims.depth,
      heightMm: dims.height,
      kind: right,
    },
  ];
}

function orthoShape(
  kind: PrimitiveKind,
  view: OrthoView['id'],
  widthMm: number,
  heightMm: number,
): OrthoView['kind'] {
  if (kind === 'box') return 'rect';
  if (kind === 'sphere') {
    return isCircularExtent(widthMm, heightMm) ? 'circle' : 'rect';
  }
  if (kind === 'cylinder' && view === 'front') {
    // The cylinder axis is Z, so the front view looks straight down the
    // axis and the real silhouette is a circle.
    return isCircularExtent(widthMm, heightMm) ? 'circle' : 'rect';
  }
  return 'rect';
}

export interface FittedBox {
  x: number;
  y: number;
  width: number;
  height: number;
  /** Millimetres → SVG units for this fit. */
  scale: number;
}

/**
 * Fit a projected silhouette into a drawing cell while preserving aspect
 * ratio. Invalid input yields a zero-size box rather than NaN geometry.
 */
export function fitToBox(
  outerWidth: number,
  outerHeight: number,
  contentWidth: number,
  contentHeight: number,
  padding: number,
): FittedBox {
  const empty: FittedBox = { x: 0, y: 0, width: 0, height: 0, scale: 0 };
  if (!positive(outerWidth) || !positive(outerHeight)) return empty;
  if (!positive(contentWidth) || !positive(contentHeight)) return empty;
  const pad = Number.isFinite(padding) ? Math.max(0, padding) : 0;
  const availableWidth = outerWidth - pad * 2;
  const availableHeight = outerHeight - pad * 2;
  if (!positive(availableWidth) || !positive(availableHeight)) return empty;
  const scale = Math.min(availableWidth / contentWidth, availableHeight / contentHeight);
  const width = contentWidth * scale;
  const height = contentHeight * scale;
  return {
    x: (outerWidth - width) / 2,
    y: (outerHeight - height) / 2,
    width,
    height,
    scale,
  };
}

/** Fixed two-decimal drafting number, without a unit suffix. */
export function formatMmValue(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return value.toFixed(2);
}

/** Fixed two-decimal drafting number with an explicit `mm` unit. */
export function formatMmDimension(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return `${value.toFixed(2)} mm`;
}

// --- Camera framing ---------------------------------------------------------
//
// The viewport must not steal the operator's camera back every time the
// workspace re-renders. The mesh handed to the viewport is referentially
// stable, and a mesh that arrives with the *same* bounds as the one already
// on screen does not re-frame the camera either. These two pure helpers are
// the single rule both the viewport and the test suite use.

/** Tolerance of the bounds comparison, in millimetres. */
export const BOUNDS_EPSILON_MM = 1e-6;

/**
 * Stable key for a bounding box. Values are rounded to `BOUNDS_EPSILON_MM`
 * so that float noise is not read as a bounds change, while a real move of
 * the part is. `null` bounds map to an empty key.
 */
export function boundsKey(bounds: BBox | null | undefined): string {
  if (!bounds) return '';
  const values = [
    bounds.min[0],
    bounds.min[1],
    bounds.min[2],
    bounds.max[0],
    bounds.max[1],
    bounds.max[2],
  ];
  return values
    .map((value) => (Number.isFinite(value) ? value.toFixed(6) : 'non-finite'))
    .join('|');
}

/**
 * True when the camera must be re-framed.
 *
 *   * no new mesh → nothing to frame;
 *   * first mesh (no previously framed bounds) → frame it;
 *   * bounds changed → frame the new bounds;
 *   * identical bounds → keep the operator's camera.
 */
export function shouldReframeBounds(
  previous: BBox | null | undefined,
  next: BBox | null | undefined,
): boolean {
  if (!next) return false;
  if (!previous) return true;
  return boundsKey(previous) !== boundsKey(next);
}

function positive(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

function formatPlaneValue(value: number): string {
  if (!Number.isFinite(value)) return '—';
  const rounded = Math.round(value * 100) / 100;
  // Normalise -0 so the label never reads "-0.00 mm".
  return (rounded === 0 ? 0 : rounded).toFixed(2);
}
