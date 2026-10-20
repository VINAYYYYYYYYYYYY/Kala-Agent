// Pure coverage for the measure tool: the distance maths and the tool
// state machine. No DOM, no WebGL — the picks themselves are real raycast
// intersections and are covered by `raycast.test.ts`.

import { describe, expect, it } from 'vitest';
import {
  MAX_MEASURE_POINTS,
  addMeasurePoint,
  clearMeasure,
  createMeasureState,
  distanceMm,
  formatMeasureDistance,
  measureDistanceMm,
  measureStatus,
  measureStatusLabel,
  toggleMeasure,
} from '../src/viewer/measure';
import type { MeasureState } from '../src/viewer/measure';

const origin = { x: 0, y: 0, z: 0 };
const alongX = { x: 40, y: 0, z: 0 };
const alongY = { x: 0, y: 20, z: 0 };
const diagonal = { x: 3, y: 4, z: 12 };

describe('measure distance maths', () => {
  it('measures along a single axis', () => {
    expect(distanceMm(origin, alongX)).toBe(40);
    expect(distanceMm(origin, alongY)).toBe(20);
  });

  it('measures a true 3D distance, not a screen-space approximation', () => {
    // 3² + 4² + 12² = 169 → 13 mm.
    expect(distanceMm(origin, diagonal)).toBe(13);
  });

  it('is symmetric', () => {
    expect(distanceMm(diagonal, origin)).toBe(distanceMm(origin, diagonal));
  });

  it('is zero for the same point', () => {
    expect(distanceMm(origin, { ...origin })).toBe(0);
  });

  it('formats a drafting number with the unit', () => {
    expect(formatMeasureDistance(13)).toBe('13.00 mm');
    expect(formatMeasureDistance(12.3456)).toBe('12.35 mm');
    expect(formatMeasureDistance(Number.NaN)).toBe('—');
  });
});

describe('measure state machine', () => {
  it('starts disarmed with no points', () => {
    const state = createMeasureState();
    expect(state.active).toBe(false);
    expect(state.points).toEqual([]);
    expect(measureStatus(state)).toBe('inactive');
  });

  it('arming the tool clears stale picks', () => {
    const armed = toggleMeasure(createMeasureState());
    expect(armed.active).toBe(true);

    const withPoint = addMeasurePoint(armed, origin);
    expect(withPoint.points).toHaveLength(1);

    const rearmed = toggleMeasure(withPoint);
    expect(rearmed.active).toBe(false);
    expect(rearmed.points).toEqual([]);

    const armedAgain = toggleMeasure(rearmed);
    expect(armedAgain.active).toBe(true);
    expect(armedAgain.points).toEqual([]);
  });

  it('ignores picks while the tool is not armed', () => {
    const state = addMeasurePoint(createMeasureState(), origin);
    expect(state.points).toEqual([]);
    expect(state.active).toBe(false);
  });

  it('ignores a pick that is not a finite model point', () => {
    const armed = toggleMeasure(createMeasureState());
    const state = addMeasurePoint(armed, { x: Number.NaN, y: 0, z: 0 });
    expect(state.points).toEqual([]);
  });

  it('needs two points before it reports a distance', () => {
    const armed = toggleMeasure(createMeasureState());
    const first = addMeasurePoint(armed, origin);
    expect(first.points).toHaveLength(1);
    expect(measureDistanceMm(first)).toBeNull();
    expect(measureStatus(first)).toBe('awaiting-second');

    const second = addMeasurePoint(first, alongX);
    expect(second.points).toHaveLength(MAX_MEASURE_POINTS);
    expect(measureDistanceMm(second)).toBe(40);
    expect(measureStatus(second)).toBe('complete');
  });

  it('starts a new measurement on a third pick', () => {
    const armed = toggleMeasure(createMeasureState());
    const complete = addMeasurePoint(addMeasurePoint(armed, origin), alongX);
    const restarted = addMeasurePoint(complete, alongY);
    expect(restarted.points).toEqual([alongY]);
    expect(restarted.active).toBe(true);
    expect(measureDistanceMm(restarted)).toBeNull();
  });

  it('clear disarms the tool and drops the picks', () => {
    const armed = toggleMeasure(createMeasureState());
    const complete = addMeasurePoint(addMeasurePoint(armed, origin), alongX);
    const cleared = clearMeasure();
    expect(cleared).toEqual({ active: false, points: [] });
    expect(complete.active).toBe(true);
  });

  it('describes what the tool is waiting for, and never invents a value', () => {
    const armed: MeasureState = toggleMeasure(createMeasureState());
    expect(measureStatusLabel(armed)).toBe('MEASURE · PICK POINT 1 ON THE MESH');
    const first = addMeasurePoint(armed, origin);
    expect(measureStatusLabel(first)).toBe('MEASURE · PICK POINT 2 ON THE MESH');
    const second = addMeasurePoint(first, alongX);
    expect(measureStatusLabel(second)).toBe('MEASURE · 40.00 mm');
    expect(measureStatusLabel(createMeasureState())).toBe('MEASURE · OFF');
  });
});
