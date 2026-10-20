// Picking helper for the measure tool.
//
// The raycast itself is a Three.js concern, so it lives here rather than in
// the pure measure module: this file is the only place that knows about the
// renderer, and the pure state machine (`viewer/measure.ts`) never imports
// Three.js.
//
// A pick returns the intersection point in *model* millimetres — the same
// coordinate space the mesh was built in — so the measured distance is a
// real length and not a screen-space approximation.

import * as THREE from 'three';
import type { MeasurePoint } from './measure';

/**
 * Convert a pointer position inside the canvas rect to normalised device
 * coordinates (-1 … 1 on both axes, +Y up).
 */
export function pointerToNdc(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
): THREE.Vector2 {
  const x = rect.width > 0 ? ((clientX - rect.left) / rect.width) * 2 - 1 : 0;
  const y = rect.height > 0 ? -((clientY - rect.top) / rect.height) * 2 + 1 : 0;
  return new THREE.Vector2(x, y);
}

/**
 * Raycast the given object and return the first hit in model millimetres.
 * Returns `null` when the ray misses, so a click on empty space never
 * invents a measurement.
 */
export function raycastFacePoint(
  camera: THREE.Camera,
  target: THREE.Object3D | null,
  ndc: THREE.Vector2,
): MeasurePoint | null {
  if (!target) return null;
  const raycaster = new THREE.Raycaster();
  raycaster.setFromCamera(ndc, camera);
  const hits = raycaster.intersectObject(target, false);
  const hit = hits[0];
  if (!hit) return null;
  return { x: hit.point.x, y: hit.point.y, z: hit.point.z };
}
