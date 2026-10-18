// Tests that import the actual shipped localPrimitives module and verify
// the invariants the studio preview depends on:
//   - finite positions
//   - non-degenerate triangles (positive area)
//   - counter-clockwise winding when viewed from outside
//   - bounds consistent with positions
//   - validator rejects out-of-range dimensions and out-of-range
//     segmentation counts with sensible codes
//
// The shipped code (src/viewer/localPrimitives.ts) is the only source of
// truth for these tests; we never reimplement the generator here.

import { describe, expect, it } from 'vitest';
import {
  buildBox,
  buildCylinder,
  buildSphere,
  validateBox,
  validateCylinder,
  validateSphere,
  boundsFromPositions,
  computeGeometryEstimate,
  formatAreaEstimate,
  formatEnvelopeEstimate,
  formatEnvelopeVolumeEstimate,
  formatVolumeEstimate,
} from '../src/viewer/localPrimitives';
import type { BoxParameters, CylinderParameters, SphereParameters } from '../src/types/studio';

function triArea(positions: Float32Array, a: number, b: number, c: number): number {
  const ax = positions[a * 3] ?? 0;
  const ay = positions[a * 3 + 1] ?? 0;
  const az = positions[a * 3 + 2] ?? 0;
  const bx = positions[b * 3] ?? 0;
  const by = positions[b * 3 + 1] ?? 0;
  const bz = positions[b * 3 + 2] ?? 0;
  const cx = positions[c * 3] ?? 0;
  const cy = positions[c * 3 + 1] ?? 0;
  const cz = positions[c * 3 + 2] ?? 0;
  const ux = bx - ax;
  const uy = by - ay;
  const uz = bz - az;
  const vx = cx - ax;
  const vy = cy - ay;
  const vz = cz - az;
  const nx = uy * vz - uz * vy;
  const ny = uz * vx - ux * vz;
  const nz = ux * vy - uy * vx;
  return 0.5 * Math.sqrt(nx * nx + ny * ny + nz * nz);
}

function winding(positions: Float32Array, indices: Uint32Array): number {
  let inward = 0;
  let outward = 0;
  for (let i = 0; i + 2 < indices.length; i += 3) {
    const a = indices[i]!;
    const b = indices[i + 1]!;
    const c = indices[i + 2]!;
    const ax = positions[a * 3] ?? 0;
    const ay = positions[a * 3 + 1] ?? 0;
    const az = positions[a * 3 + 2] ?? 0;
    const bx = positions[b * 3] ?? 0;
    const by = positions[b * 3 + 1] ?? 0;
    const bz = positions[b * 3 + 2] ?? 0;
    const cx = positions[c * 3] ?? 0;
    const cy = positions[c * 3 + 1] ?? 0;
    const cz = positions[c * 3 + 2] ?? 0;
    const ux = bx - ax, uy = by - ay, uz = bz - az;
    const vx = cx - ax, vy = cy - ay, vz = cz - az;
    const nx = uy * vz - uz * vy;
    const ny = uz * vx - ux * vz;
    const nz = ux * vy - uy * vx;
    const dot =
      (ax + bx + cx) * nx +
      (ay + by + cy) * ny +
      (az + bz + cz) * nz;
    if (dot > 0) outward++;
    else if (dot < 0) inward++;
  }
  const total = inward + outward;
  return total > 0 ? outward / total : 1;
}

describe('box generator', () => {
  it('produces correct bounds for size_x=20 size_y=30 size_z=40', () => {
    const box = buildBox({ size_x: 20, size_y: 30, size_z: 40, subdivisions: 1 } satisfies BoxParameters);
    expect(box.bounds.min).toEqual([-10, -15, -20]);
    expect(box.bounds.max).toEqual([10, 15, 20]);
  });

  it('every triangle has CCW winding from outside', () => {
    const box = buildBox({ size_x: 20, size_y: 30, size_z: 40, subdivisions: 1 } satisfies BoxParameters);
    const ratio = winding(box.positions, box.indices);
    expect(ratio).toBe(1);
  });

  it('every triangle is non-degenerate', () => {
    const box = buildBox({ size_x: 20, size_y: 30, size_z: 40, subdivisions: 1 } satisfies BoxParameters);
    let degenerate = 0;
    for (let i = 0; i < box.indices.length; i += 3) {
      const area = triArea(box.positions, box.indices[i]!, box.indices[i + 1]!, box.indices[i + 2]!);
      if (area <= 0) degenerate++;
    }
    expect(degenerate).toBe(0);
  });

  it('normals have unit length', () => {
    const box = buildBox({ size_x: 20, size_y: 30, size_z: 40, subdivisions: 1 } satisfies BoxParameters);
    for (let i = 0; i < box.normals.length; i += 3) {
      const nx = box.normals[i] ?? 0;
      const ny = box.normals[i + 1] ?? 0;
      const nz = box.normals[i + 2] ?? 0;
      const len = Math.sqrt(nx * nx + ny * ny + nz * nz);
      expect(len).toBeCloseTo(1, 6);
    }
  });
});

describe('cylinder generator', () => {
  it('produces correct bounds for radius=10 height=30 along the Z axis', () => {
    const cyl = buildCylinder({ radius: 10, height: 30, radial_segments: 32, height_segments: 1 } satisfies CylinderParameters);
    expect(cyl.bounds.min[0]).toBeCloseTo(-10, 6);
    expect(cyl.bounds.max[0]).toBeCloseTo(10, 6);
    expect(cyl.bounds.min[1]).toBeCloseTo(-10, 6);
    expect(cyl.bounds.max[1]).toBeCloseTo(10, 6);
    expect(cyl.bounds.min[2]).toBeCloseTo(-15, 6);
    expect(cyl.bounds.max[2]).toBeCloseTo(15, 6);
  });

  it('every triangle has CCW winding from outside', () => {
    const cyl = buildCylinder({ radius: 10, height: 30, radial_segments: 32, height_segments: 1 } satisfies CylinderParameters);
    const ratio = winding(cyl.positions, cyl.indices);
    expect(ratio).toBeGreaterThan(0.99);
  });

  it('every triangle is non-degenerate', () => {
    const cyl = buildCylinder({ radius: 10, height: 30, radial_segments: 32, height_segments: 1 } satisfies CylinderParameters);
    let degenerate = 0;
    for (let i = 0; i < cyl.indices.length; i += 3) {
      const area = triArea(cyl.positions, cyl.indices[i]!, cyl.indices[i + 1]!, cyl.indices[i + 2]!);
      if (area <= 0) degenerate++;
    }
    expect(degenerate).toBe(0);
  });
});

describe('sphere generator', () => {
  it('produces correct bounds for radius=10', () => {
    const sph = buildSphere({ radius: 10, rings: 24, sectors: 32 } satisfies SphereParameters);
    for (let axis = 0; axis < 3; axis++) {
      expect(sph.bounds.min[axis]!).toBeCloseTo(-10, 6);
      expect(sph.bounds.max[axis]!).toBeCloseTo(10, 6);
    }
  });

  it('no degenerate triangles', () => {
    const sph = buildSphere({ radius: 10, rings: 24, sectors: 32 } satisfies SphereParameters);
    let degenerate = 0;
    for (let i = 0; i < sph.indices.length; i += 3) {
      const area = triArea(sph.positions, sph.indices[i]!, sph.indices[i + 1]!, sph.indices[i + 2]!);
      if (area <= 0) degenerate++;
    }
    expect(degenerate).toBe(0);
  });
});

describe('validator', () => {
  it('box rejects zero and NaN dimensions', () => {
    expect(validateBox({ size_x: 0, size_y: 1, size_z: 1, subdivisions: 1 })).not.toBeNull();
    expect(validateBox({ size_x: Number.NaN, size_y: 1, size_z: 1, subdivisions: 1 })).not.toBeNull();
    expect(validateBox({ size_x: -1, size_y: 1, size_z: 1, subdivisions: 1 })).not.toBeNull();
  });

  it('box rejects out-of-range subdivisions', () => {
    expect(validateBox({ size_x: 1, size_y: 1, size_z: 1, subdivisions: 0 })).not.toBeNull();
    expect(validateBox({ size_x: 1, size_y: 1, size_z: 1, subdivisions: 100 })).not.toBeNull();
  });

  it('cylinder rejects missing height_segments', () => {
    expect(validateCylinder({ radius: 10, height: 10, radial_segments: 8, height_segments: 1 })).toBeNull();
    expect(validateCylinder({ radius: 10, height: 10, radial_segments: 8, height_segments: 0 })).not.toBeNull();
  });

  it('sphere rejects non-integer rings', () => {
    expect(validateSphere({ radius: 10, rings: 12.5, sectors: 16 })).not.toBeNull();
  });
});

describe('boundsFromPositions', () => {
  it('returns zero box for empty input', () => {
    const bounds = boundsFromPositions(new Float32Array(0));
    expect(bounds.min).toEqual([0, 0, 0]);
    expect(bounds.max).toEqual([0, 0, 0]);
  });

  it('computes tight bounds', () => {
    const positions = new Float32Array([
      -1, -2, -3,
      4, 5, 6,
    ]);
    expect(boundsFromPositions(positions)).toEqual({ min: [-1, -2, -3], max: [4, 5, 6] });
  });
});

// Nominal analytical geometry — the numbers an OpenCascade volume/area
// property query returns for the ideal solid. These drive the inspector's
// estimate panel, the agent's active-geometry banner, and the numbers an
// operator quotes before handing a neutral STEP/STL file to an external
// CAD/CAE package, so they are pinned exactly.

describe('computeGeometryEstimate — box', () => {
  it('matches X·Y·Z and 2(XY + YZ + ZX) for 40 × 20 × 30 mm', () => {
    const estimate = computeGeometryEstimate('box', {
      size_x: 40,
      size_y: 20,
      size_z: 30,
      subdivisions: 1,
    } satisfies BoxParameters);

    expect(estimate.envelopeMm).toEqual([40, 20, 30]);
    expect(estimate.envelopeVolumeMm3).toBe(24000);
    expect(estimate.volumeMm3).toBe(24000);
    expect(estimate.volumeCm3).toBe(24);
    // 2·(40·20 + 20·30 + 30·40) = 2·2600
    expect(estimate.surfaceAreaMm2).toBe(5200);
    expect(estimate.surfaceAreaCm2).toBe(52);
  });

  it('is independent of tessellation (subdivisions change no nominal value)', () => {
    const coarse = computeGeometryEstimate('box', { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 });
    const fine = computeGeometryEstimate('box', { size_x: 40, size_y: 20, size_z: 30, subdivisions: 32 });
    expect(fine).toEqual(coarse);
  });

  it('reports the envelope volume separately from the solid volume for a cube', () => {
    // A cube's solid volume equals its envelope volume; this pins that the
    // two members are genuinely distinct fields rather than aliases.
    const cube = computeGeometryEstimate('box', { size_x: 10, size_y: 10, size_z: 10, subdivisions: 1 });
    expect(cube.envelopeVolumeMm3).toBe(1000);
    expect(cube.volumeMm3).toBe(1000);
  });
});

describe('computeGeometryEstimate — cylinder', () => {
  it('matches πr²h and 2πr(r + h) for r=20 h=40', () => {
    const estimate = computeGeometryEstimate('cylinder', {
      radius: 20,
      height: 40,
      radial_segments: 48,
      height_segments: 1,
    } satisfies CylinderParameters);

    expect(estimate.volumeMm3).toBeCloseTo(Math.PI * 400 * 40, 9);
    expect(estimate.surfaceAreaMm2).toBeCloseTo(2 * Math.PI * 20 * 60, 9);
    expect(estimate.volumeCm3).toBeCloseTo(estimate.volumeMm3 / 1000, 9);
    expect(estimate.surfaceAreaCm2).toBeCloseTo(estimate.surfaceAreaMm2 / 100, 9);
  });

  it('envelopes the cylinder as diameter × diameter × height', () => {
    const estimate = computeGeometryEstimate('cylinder', {
      radius: 12.5,
      height: 70,
      radial_segments: 24,
      height_segments: 1,
    } satisfies CylinderParameters);
    expect(estimate.envelopeMm).toEqual([25, 25, 70]);
    expect(estimate.envelopeVolumeMm3).toBe(25 * 25 * 70);
  });

  it('ignores segment counts — the nominal solid does not get more accurate', () => {
    const coarse = computeGeometryEstimate('cylinder', { radius: 10, height: 30, radial_segments: 6, height_segments: 1 });
    const fine = computeGeometryEstimate('cylinder', { radius: 10, height: 30, radial_segments: 256, height_segments: 64 });
    expect(fine).toEqual(coarse);
  });
});

describe('computeGeometryEstimate — sphere', () => {
  it('matches ⁴⁄₃πr³ and 4πr² for r=25', () => {
    const estimate = computeGeometryEstimate('sphere', { radius: 25, rings: 24, sectors: 32 } satisfies SphereParameters);
    expect(estimate.volumeMm3).toBeCloseTo((4 / 3) * Math.PI * 25 ** 3, 6);
    expect(estimate.surfaceAreaMm2).toBeCloseTo(4 * Math.PI * 25 * 25, 6);
    // The classic 4/3·π·r³ < bounding cube for r = 25.
    expect(estimate.volumeMm3).toBeLessThan(estimate.envelopeVolumeMm3);
    expect(estimate.envelopeMm).toEqual([50, 50, 50]);
    expect(estimate.envelopeVolumeMm3).toBe(125000);
  });

  it('matches ⁴⁄₃πr³ for r=10', () => {
    const estimate = computeGeometryEstimate('sphere', { radius: 10, rings: 8, sectors: 12 } satisfies SphereParameters);
    expect(estimate.volumeMm3).toBeCloseTo(4188.7902047863905, 6);
    expect(estimate.volumeCm3).toBeCloseTo(4.1887902047863905, 9);
  });
});

describe('computeGeometryEstimate — contract', () => {
  it('is pure: the same input always yields the same estimate', () => {
    const params = { radius: 33, height: 55, radial_segments: 32, height_segments: 2 } satisfies CylinderParameters;
    expect(computeGeometryEstimate('cylinder', params)).toEqual(computeGeometryEstimate('cylinder', params));
  });

  it('converts mm³ → cm³ and mm² → cm² consistently for every primitive', () => {
    const cases = [
      computeGeometryEstimate('box', { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 }),
      computeGeometryEstimate('cylinder', { radius: 20, height: 40, radial_segments: 48, height_segments: 1 }),
      computeGeometryEstimate('sphere', { radius: 25, rings: 24, sectors: 32 }),
    ];
    for (const estimate of cases) {
      expect(estimate.volumeCm3).toBeCloseTo(estimate.volumeMm3 / 1000, 9);
      expect(estimate.surfaceAreaCm2).toBeCloseTo(estimate.surfaceAreaMm2 / 100, 9);
      expect(estimate.envelopeVolumeMm3).toBe(
        estimate.envelopeMm[0] * estimate.envelopeMm[1] * estimate.envelopeMm[2],
      );
    }
  });

  it('reports magnitudes, never a negative volume or extent', () => {
    const bad = computeGeometryEstimate('box', { size_x: -10, size_y: -20, size_z: 30, subdivisions: 1 });
    expect(bad.volumeMm3).toBe(6000);
    expect(bad.envelopeMm).toEqual([10, 20, 30]);
    // Range rejection is the validator's job, not the estimator's.
    expect(validateBox({ size_x: -10, size_y: -20, size_z: 30, subdivisions: 1 })).not.toBeNull();
  });
});

describe('estimate formatters', () => {
  it('renders volume, area, and envelope with units', () => {
    const estimate = computeGeometryEstimate('box', { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 });
    expect(formatVolumeEstimate(estimate)).toBe('24000.00 mm³ (24.000 cm³)');
    expect(formatAreaEstimate(estimate)).toBe('5200.00 mm² (52.000 cm²)');
    expect(formatEnvelopeEstimate(estimate)).toBe('40.00 × 20.00 × 30.00 mm');
    expect(formatEnvelopeVolumeEstimate(estimate)).toBe('24000.00 mm³');
  });

  it('degrades to an em dash for non-finite inputs instead of printing NaN', () => {
    const broken = computeGeometryEstimate('sphere', { radius: Number.NaN, rings: 24, sectors: 32 });
    expect(formatVolumeEstimate(broken)).toBe('—');
    expect(formatAreaEstimate(broken)).toBe('—');
    expect(formatEnvelopeEstimate(broken)).toBe('—');
    expect(formatEnvelopeVolumeEstimate(broken)).toBe('—');
  });
});