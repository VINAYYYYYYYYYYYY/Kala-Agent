// Measure tool: the distance between two picked points on the mesh.
//
// The tool is a pure state machine plus pure distance maths so both can be
// unit tested without a WebGL context. The viewport owns the picking: it
// raycasts the face mesh and hands the real intersection point over in
// model millimetres. Screen coordinates never enter this module — a
// measurement is a distance between two points *in the model*, so the
// number on the sheet is a real length, not a screen-space approximation.
//
// Rules:
//   * the tool is armed with `toggleMeasure`; arming it drops any stale
//     picks so a new measurement never mixes points from another one;
//   * the first pick arms the second point, the second pick completes the
//     measurement;
//   * a third pick starts a new measurement (the pick becomes its first
//     point) so the operator can re-measure without re-arming the tool;
//   * `clearMeasure` disarms the tool and drops the picks — the sheet uses
//     it for Escape and for a second click on the tool button.

/** A point on the mesh, in model millimetres. */
export interface MeasurePoint {
  x: number;
  y: number;
  z: number;
}

/** The measure tool state. */
export interface MeasureState {
  /** True while the tool is armed and accepting picks. */
  active: boolean;
  /** The picked points, at most two. */
  points: MeasurePoint[];
}

/** A measurement needs two points. */
export const MAX_MEASURE_POINTS = 2;

/** A fresh, disarmed measure state. */
export function createMeasureState(): MeasureState {
  return { active: false, points: [] };
}

/**
 * Straight-line distance between two model points, in millimetres.
 * Non-finite input yields `NaN`, which the callers render as `—`.
 */
export function distanceMm(a: MeasurePoint, b: MeasurePoint): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const dz = b.z - a.z;
  return Math.sqrt(dx * dx + dy * dy + dz * dz);
}

/** Arm or disarm the tool. Arming or disarming always drops stale picks. */
export function toggleMeasure(state: MeasureState): MeasureState {
  return { active: !state.active, points: [] };
}

/** Disarm the tool and drop the picks. */
export function clearMeasure(): MeasureState {
  return { active: false, points: [] };
}

/**
 * Record a picked point. Ignored when the tool is not armed or the point is
 * not a finite model coordinate. The third pick starts a new measurement.
 */
export function addMeasurePoint(state: MeasureState, point: MeasurePoint): MeasureState {
  if (!state.active) return state;
  if (!isFinitePoint(point)) return state;
  if (state.points.length >= MAX_MEASURE_POINTS) {
    return { active: true, points: [point] };
  }
  return { active: true, points: [...state.points, point] };
}

/** The measured distance, or `null` while the measurement is incomplete. */
export function measureDistanceMm(state: MeasureState): number | null {
  if (state.points.length < MAX_MEASURE_POINTS) return null;
  const first = state.points[0];
  const second = state.points[1];
  if (!first || !second) return null;
  return distanceMm(first, second);
}

/** Drafting number for a distance, e.g. `40.00 mm`. */
export function formatMeasureDistance(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return `${value.toFixed(2)} mm`;
}

/** The state of the tool, for the UI. */
export type MeasureStatus = 'inactive' | 'awaiting-first' | 'awaiting-second' | 'complete';

/** The status of the current tool state. */
export function measureStatus(state: MeasureState): MeasureStatus {
  if (!state.active) return 'inactive';
  if (state.points.length === 0) return 'awaiting-first';
  if (state.points.length === 1) return 'awaiting-second';
  return 'complete';
}

/**
 * A short honest note for the sheet: what the tool is waiting for, or the
 * measured distance. No value is invented while the measurement is
 * incomplete.
 */
export function measureStatusLabel(state: MeasureState): string {
  switch (measureStatus(state)) {
    case 'awaiting-first':
      return 'MEASURE · PICK POINT 1 ON THE MESH';
    case 'awaiting-second':
      return 'MEASURE · PICK POINT 2 ON THE MESH';
    case 'complete': {
      const distance = measureDistanceMm(state);
      return distance === null ? 'MEASURE · —' : `MEASURE · ${formatMeasureDistance(distance)}`;
    }
    case 'inactive':
    default:
      return 'MEASURE · OFF';
  }
}

function isFinitePoint(point: MeasurePoint): boolean {
  return (
    Number.isFinite(point.x) && Number.isFinite(point.y) && Number.isFinite(point.z)
  );
}
