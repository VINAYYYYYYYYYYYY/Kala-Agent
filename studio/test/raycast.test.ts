// Raycast coverage for the measure tool.
//
// The pick is a real Three.js raycast against the face mesh, so the test
// builds a real camera and a real box geometry and checks that the
// intersection point comes back in model millimetres. A miss returns
// `null` so the sheet never invents a measurement.

import { describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { pointerToNdc, raycastFacePoint } from '../src/viewer/raycast';

function boxMesh(size = 40): THREE.Mesh {
  const geometry = new THREE.BoxGeometry(size, size, size);
  return new THREE.Mesh(geometry, new THREE.MeshBasicMaterial());
}

function cameraLookingAt(
  position: [number, number, number],
  target: [number, number, number] = [0, 0, 0],
): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 1000);
  camera.position.set(...position);
  camera.lookAt(new THREE.Vector3(...target));
  camera.updateMatrixWorld();
  return camera;
}

describe('pointerToNdc', () => {
  it('maps the canvas centre to the origin', () => {
    const ndc = pointerToNdc(100, 50, { left: 0, top: 0, width: 200, height: 100 });
    expect(ndc.x).toBeCloseTo(0, 6);
    expect(ndc.y).toBeCloseTo(0, 6);
  });

  it('maps the corners to the unit square and keeps +Y up', () => {
    const rect = { left: 0, top: 0, width: 200, height: 100 };
    const bottomLeft = pointerToNdc(0, 100, rect);
    expect(bottomLeft.x).toBeCloseTo(-1, 6);
    expect(bottomLeft.y).toBeCloseTo(-1, 6);
    const topRight = pointerToNdc(200, 0, rect);
    expect(topRight.x).toBeCloseTo(1, 6);
    expect(topRight.y).toBeCloseTo(1, 6);
  });

  it('is safe on a zero-sized rect', () => {
    const ndc = pointerToNdc(10, 10, { left: 0, top: 0, width: 0, height: 0 });
    expect(Number.isFinite(ndc.x)).toBe(true);
    expect(Number.isFinite(ndc.y)).toBe(true);
  });
});

describe('raycastFacePoint', () => {
  it('returns the model-space hit point in millimetres', () => {
    const mesh = boxMesh(40);
    const camera = cameraLookingAt([0, 0, 120]);
    const hit = raycastFacePoint(camera, mesh, new THREE.Vector2(0, 0));
    expect(hit).not.toBeNull();
    // The camera looks down -Z at the box, so the hit is the +Z face.
    expect(hit!.x).toBeCloseTo(0, 4);
    expect(hit!.y).toBeCloseTo(0, 4);
    expect(hit!.z).toBeCloseTo(20, 4);
  });

  it('returns a point on the surface for an off-centre pick', () => {
    const mesh = boxMesh(40);
    const camera = cameraLookingAt([0, 0, 120]);
    const hit = raycastFacePoint(camera, mesh, new THREE.Vector2(0.2, 0));
    expect(hit).not.toBeNull();
    expect(hit!.z).toBeCloseTo(20, 4);
    expect(hit!.x).toBeGreaterThan(0);
    expect(hit!.x).toBeLessThan(20);
  });

  it('returns null when the ray misses the mesh', () => {
    const mesh = boxMesh(40);
    const camera = cameraLookingAt([0, 0, 120]);
    const miss = raycastFacePoint(camera, mesh, new THREE.Vector2(0, 0.99));
    expect(miss).toBeNull();
  });

  it('returns null when there is no mesh to hit', () => {
    const camera = cameraLookingAt([0, 0, 120]);
    expect(raycastFacePoint(camera, null, new THREE.Vector2(0, 0))).toBeNull();
  });
});
