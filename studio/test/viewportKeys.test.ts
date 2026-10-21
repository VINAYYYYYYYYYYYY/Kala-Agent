// The keyboard contract of the 3D viewport.
//
// The mapping is pure, so it can be pinned without a WebGL context:
//   * the arrow keys orbit,
//   * `+` / `-` zoom,
//   * `0` resets,
//   * every other key is left to the browser.

import { describe, expect, it } from 'vitest';

import {
  orbitCamera,
  VIEWPORT_KEY_HELP,
  VIEWPORT_MAX_POLAR_RAD,
  VIEWPORT_MIN_POLAR_RAD,
  VIEWPORT_ORBIT_STEP_RAD,
  VIEWPORT_ZOOM_FACTOR,
  viewportCommand,
  zoomCamera,
} from '../src/viewer/viewportKeys';

describe('the viewport key map', () => {
  it('maps the four arrow keys to an orbit', () => {
    expect(viewportCommand('ArrowLeft')).toEqual({
      kind: 'orbit',
      azimuth: -VIEWPORT_ORBIT_STEP_RAD,
      polar: 0,
    });
    expect(viewportCommand('ArrowRight')).toEqual({
      kind: 'orbit',
      azimuth: VIEWPORT_ORBIT_STEP_RAD,
      polar: 0,
    });
    expect(viewportCommand('ArrowUp')).toEqual({
      kind: 'orbit',
      azimuth: 0,
      polar: -VIEWPORT_ORBIT_STEP_RAD,
    });
    expect(viewportCommand('ArrowDown')).toEqual({
      kind: 'orbit',
      azimuth: 0,
      polar: VIEWPORT_ORBIT_STEP_RAD,
    });
  });

  it('maps + and - to a zoom and 0 to a reset', () => {
    expect(viewportCommand('+')).toEqual({ kind: 'zoom', factor: VIEWPORT_ZOOM_FACTOR });
    expect(viewportCommand('=')).toEqual({ kind: 'zoom', factor: VIEWPORT_ZOOM_FACTOR });
    expect(viewportCommand('-')).toEqual({ kind: 'zoom', factor: 1 / VIEWPORT_ZOOM_FACTOR });
    expect(viewportCommand('_')).toEqual({ kind: 'zoom', factor: 1 / VIEWPORT_ZOOM_FACTOR });
    expect(viewportCommand('0')).toEqual({ kind: 'reset' });
  });

  it('leaves every other key to the browser', () => {
    for (const key of ['a', 'Enter', 'Tab', 'Escape', ' ', 'F1', 'ArrowUpLeft']) {
      expect(viewportCommand(key)).toBeNull();
    }
  });

  it('names every key it owns in the help sentence', () => {
    expect(VIEWPORT_KEY_HELP).toContain('arrow keys orbit');
    expect(VIEWPORT_KEY_HELP).toContain('+ and - zoom');
    expect(VIEWPORT_KEY_HELP).toContain('0 resets the view');
  });
});

describe('the viewport camera maths', () => {
  const camera = { azimuth: 0.5, polar: 1.2, distance: 400 };

  it('adds the orbit step to the azimuth and the polar angle', () => {
    const turned = orbitCamera(camera, VIEWPORT_ORBIT_STEP_RAD, 0);
    expect(turned.azimuth).toBeCloseTo(0.5 + VIEWPORT_ORBIT_STEP_RAD, 10);
    expect(turned.polar).toBeCloseTo(1.2, 10);
    expect(turned.distance).toBe(400);
  });

  it('never lets the polar angle reach a pole', () => {
    const up = orbitCamera({ ...camera, polar: VIEWPORT_MIN_POLAR_RAD }, 0, -1);
    expect(up.polar).toBe(VIEWPORT_MIN_POLAR_RAD);
    const down = orbitCamera({ ...camera, polar: VIEWPORT_MAX_POLAR_RAD }, 0, 1);
    expect(down.polar).toBe(VIEWPORT_MAX_POLAR_RAD);
  });

  it('scales the distance on a zoom and clamps it to the controls range', () => {
    const closer = zoomCamera(camera, VIEWPORT_ZOOM_FACTOR, 1, 10_000);
    expect(closer.distance).toBeCloseTo(400 * VIEWPORT_ZOOM_FACTOR, 10);

    const tooClose = zoomCamera(camera, 0.0001, 10, 10_000);
    expect(tooClose.distance).toBe(10);

    const tooFar = zoomCamera(camera, 1000, 10, 10_000);
    expect(tooFar.distance).toBe(10_000);
  });

  it('keeps the azimuth and the polar angle on a zoom', () => {
    const zoomed = zoomCamera(camera, 0.5, 1, 10_000);
    expect(zoomed.azimuth).toBe(camera.azimuth);
    expect(zoomed.polar).toBe(camera.polar);
  });
});
