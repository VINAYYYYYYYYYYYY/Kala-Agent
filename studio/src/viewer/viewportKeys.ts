// The keyboard contract of the 3D viewport.
//
// The viewport is a three.js canvas: the pointer orbits it, but a keyboard
// user must be able to reach it too. The mapping lives here, in a pure
// module, so the keys are testable without a WebGL context and so the
// camera maths has one home.
//
// The contract:
//   * the arrow keys orbit (left/right around the vertical axis,
//     up/down over it),
//   * `+` / `-` zoom in and out,
//   * `0` re-frames the part.
//
// Every command is expressed in the camera's own spherical coordinates, so
// the same numbers drive the pointer and the keyboard.

/** One step of the keyboard orbit, in radians (15 degrees). */
export const VIEWPORT_ORBIT_STEP_RAD = Math.PI / 12;

/**
 * One step of the keyboard zoom, as a factor of the current camera
 * distance. Below 1 is closer to the part.
 */
export const VIEWPORT_ZOOM_FACTOR = 0.8;

/** The polar angle never reaches the poles: the part would be edge-on. */
export const VIEWPORT_MIN_POLAR_RAD = 0.05;
export const VIEWPORT_MAX_POLAR_RAD = Math.PI - 0.05;

/** One command of the viewport keyboard. */
export type ViewportCommand =
  | { kind: 'orbit'; azimuth: number; polar: number }
  | { kind: 'zoom'; factor: number }
  | { kind: 'reset' };

/**
 * The command of one key, or `null` when the key is not a viewport key.
 *
 * An unknown key is left to the browser: the viewport never swallows a key
 * it does not own.
 */
export function viewportCommand(key: string): ViewportCommand | null {
  switch (key) {
    case 'ArrowLeft':
      return { kind: 'orbit', azimuth: -VIEWPORT_ORBIT_STEP_RAD, polar: 0 };
    case 'ArrowRight':
      return { kind: 'orbit', azimuth: VIEWPORT_ORBIT_STEP_RAD, polar: 0 };
    case 'ArrowUp':
      return { kind: 'orbit', azimuth: 0, polar: -VIEWPORT_ORBIT_STEP_RAD };
    case 'ArrowDown':
      return { kind: 'orbit', azimuth: 0, polar: VIEWPORT_ORBIT_STEP_RAD };
    case '+':
    case '=':
      return { kind: 'zoom', factor: VIEWPORT_ZOOM_FACTOR };
    case '-':
    case '_':
      return { kind: 'zoom', factor: 1 / VIEWPORT_ZOOM_FACTOR };
    case '0':
      return { kind: 'reset' };
    default:
      return null;
  }
}

/** The spherical camera state the commands act on. */
export interface ViewportCamera {
  /** The azimuth around the vertical axis, in radians. */
  azimuth: number;
  /** The polar angle from the vertical axis, in radians. */
  polar: number;
  /** The distance to the part, in millimetres. */
  distance: number;
}

/**
 * The camera after one orbit command. The polar angle is clamped so the
 * part is never viewed exactly edge-on from the pole.
 */
export function orbitCamera(
  camera: ViewportCamera,
  azimuth: number,
  polar: number,
): ViewportCamera {
  return {
    azimuth: camera.azimuth + azimuth,
    polar: Math.min(
      VIEWPORT_MAX_POLAR_RAD,
      Math.max(VIEWPORT_MIN_POLAR_RAD, camera.polar + polar),
    ),
    distance: camera.distance,
  };
}

/** The camera after one zoom command, clamped to the controls' own range. */
export function zoomCamera(
  camera: ViewportCamera,
  factor: number,
  minDistance: number,
  maxDistance: number,
): ViewportCamera {
  const next = camera.distance * factor;
  return {
    azimuth: camera.azimuth,
    polar: camera.polar,
    distance: Math.min(maxDistance, Math.max(minDistance, next)),
  };
}

/**
 * The sentence the viewport announces. It names every key it owns, so a
 * keyboard user learns the contract from the control itself.
 */
export const VIEWPORT_KEY_HELP =
  'Keyboard: the arrow keys orbit, + and - zoom, 0 resets the view.';
