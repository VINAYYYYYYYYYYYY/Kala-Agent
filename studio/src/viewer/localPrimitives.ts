// Local triangle-mesh generator for the three studio primitives.
//
// The generator matches the backend's parametric intent:
//   box:        size_x, size_y, size_z, subdivisions
//   cylinder:   radius, height, radial_segments, height_segments
//   sphere:     radius, rings, sectors
//
// Every primitive is centred on the origin, units are millimetres, triangles
// are wound counter-clockwise when viewed from outside (so the geometric
// normal agrees with the per-vertex normal), normals are unit length, and
// the bounds match the vertex extents to within rounding. A validator
// rejects out-of-range numbers using the same limits the backend applies.
//
// Alongside the tessellators it exports `computeGeometryEstimate`, the
// closed-form nominal volume / surface-area / bounding-envelope values the
// inspector, the agent chat, and the CAD handoff panel all read.
//
// This is a pure module with no DOM or network dependencies. The same
// exports back the renderer, the preview controller, and the test suite.

import {
  BBox,
  BoxParameters,
  CylinderParameters,
  MAX_BOX_SUBDIVISIONS,
  MAX_CYLINDER_HEIGHT_SEGMENTS,
  MAX_CYLINDER_RADIAL_SEGMENTS,
  MAX_DIMENSION_MM,
  MAX_MESH_VERTICES,
  MAX_SPHERE_RINGS,
  MAX_SPHERE_SECTORS,
  MIN_BOX_SUBDIVISIONS,
  MIN_CYLINDER_HEIGHT_SEGMENTS,
  MIN_CYLINDER_RADIAL_SEGMENTS,
  MIN_DIMENSION_MM,
  MIN_SPHERE_RINGS,
  MIN_SPHERE_SECTORS,
  PrimitiveKind,
  SphereParameters,
  StudioLocalPreview,
  StudioParameters,
} from '../types/studio';

export const TWO_PI = Math.PI * 2;

export const LOCAL_ENGINE = 'analytic-local';

export function defaultBox(): BoxParameters {
  return { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 };
}

export function defaultCylinder(): CylinderParameters {
  return { radius: 20, height: 40, radial_segments: 48, height_segments: 1 };
}

export function defaultSphere(): SphereParameters {
  return { radius: 25, rings: 24, sectors: 32 };
}

export function defaultParameters(kind: PrimitiveKind): StudioParameters {
  if (kind === 'box') return defaultBox();
  if (kind === 'cylinder') return defaultCylinder();
  return defaultSphere();
}

export interface ValidationError {
  code: string;
  message: string;
}

export function validateBox(p: BoxParameters): ValidationError | null {
  for (const [name, value] of [
    ['size_x', p.size_x],
    ['size_y', p.size_y],
    ['size_z', p.size_z],
  ] as ReadonlyArray<[string, number]>) {
    if (!Number.isFinite(value)) {
      return { code: 'invalid_dimension', message: `${name} must be a finite number (millimetres)` };
    }
    if (value < MIN_DIMENSION_MM || value > MAX_DIMENSION_MM) {
      return {
        code: 'dimension_out_of_range',
        message: `${name} must be between ${MIN_DIMENSION_MM} and ${MAX_DIMENSION_MM} mm`,
      };
    }
  }
  if (!Number.isInteger(p.subdivisions) || p.subdivisions < MIN_BOX_SUBDIVISIONS || p.subdivisions > MAX_BOX_SUBDIVISIONS) {
    return {
      code: 'invalid_subdivisions',
      message: `subdivisions must be an integer in [${MIN_BOX_SUBDIVISIONS}, ${MAX_BOX_SUBDIVISIONS}]`,
    };
  }
  return null;
}

export function validateCylinder(p: CylinderParameters): ValidationError | null {
  if (!Number.isFinite(p.radius) || p.radius < MIN_DIMENSION_MM || p.radius > MAX_DIMENSION_MM) {
    return {
      code: 'dimension_out_of_range',
      message: `radius must be between ${MIN_DIMENSION_MM} and ${MAX_DIMENSION_MM} mm`,
    };
  }
  if (!Number.isFinite(p.height) || p.height < MIN_DIMENSION_MM || p.height > MAX_DIMENSION_MM) {
    return {
      code: 'dimension_out_of_range',
      message: `height must be between ${MIN_DIMENSION_MM} and ${MAX_DIMENSION_MM} mm`,
    };
  }
  if (!Number.isInteger(p.radial_segments) || p.radial_segments < MIN_CYLINDER_RADIAL_SEGMENTS || p.radial_segments > MAX_CYLINDER_RADIAL_SEGMENTS) {
    return {
      code: 'invalid_segments',
      message: `radial_segments must be an integer in [${MIN_CYLINDER_RADIAL_SEGMENTS}, ${MAX_CYLINDER_RADIAL_SEGMENTS}]`,
    };
  }
  if (!Number.isInteger(p.height_segments) || p.height_segments < MIN_CYLINDER_HEIGHT_SEGMENTS || p.height_segments > MAX_CYLINDER_HEIGHT_SEGMENTS) {
    return {
      code: 'invalid_segments',
      message: `height_segments must be an integer in [${MIN_CYLINDER_HEIGHT_SEGMENTS}, ${MAX_CYLINDER_HEIGHT_SEGMENTS}]`,
    };
  }
  return null;
}

export function validateSphere(p: SphereParameters): ValidationError | null {
  if (!Number.isFinite(p.radius) || p.radius < MIN_DIMENSION_MM || p.radius > MAX_DIMENSION_MM) {
    return {
      code: 'dimension_out_of_range',
      message: `radius must be between ${MIN_DIMENSION_MM} and ${MAX_DIMENSION_MM} mm`,
    };
  }
  if (!Number.isInteger(p.rings) || p.rings < MIN_SPHERE_RINGS || p.rings > MAX_SPHERE_RINGS) {
    return {
      code: 'invalid_segments',
      message: `rings must be an integer in [${MIN_SPHERE_RINGS}, ${MAX_SPHERE_RINGS}]`,
    };
  }
  if (!Number.isInteger(p.sectors) || p.sectors < MIN_SPHERE_SECTORS || p.sectors > MAX_SPHERE_SECTORS) {
    return {
      code: 'invalid_segments',
      message: `sectors must be an integer in [${MIN_SPHERE_SECTORS}, ${MAX_SPHERE_SECTORS}]`,
    };
  }
  return null;
}

export function validateParameters(kind: PrimitiveKind, p: StudioParameters): ValidationError | null {
  if (kind === 'box') return validateBox(p as BoxParameters);
  if (kind === 'cylinder') return validateCylinder(p as CylinderParameters);
  return validateSphere(p as SphereParameters);
}

export function boundsFromPositions(positions: Float32Array): BBox {
  if (positions.length < 3) {
    return { min: [0, 0, 0], max: [0, 0, 0] };
  }
  let minX = positions[0]!;
  let minY = positions[1]!;
  let minZ = positions[2]!;
  let maxX = minX;
  let maxY = minY;
  let maxZ = minZ;
  for (let i = 3; i + 2 < positions.length; i += 3) {
    const x = positions[i]!;
    const y = positions[i + 1]!;
    const z = positions[i + 2]!;
    if (x < minX) minX = x; else if (x > maxX) maxX = x;
    if (y < minY) minY = y; else if (y > maxY) maxY = y;
    if (z < minZ) minZ = z; else if (z > maxZ) maxZ = z;
  }
  return { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] };
}

function emptyPreview(kind: PrimitiveKind, p: StudioParameters): StudioLocalPreview {
  return {
    kind,
    parameters: p,
    positions: new Float32Array(0),
    normals: new Float32Array(0),
    indices: new Uint32Array(0),
    bounds: { min: [0, 0, 0], max: [0, 0, 0] },
    source: 'local',
    generatedAt: Date.now(),
    engine: LOCAL_ENGINE,
    warnings: ['parameters failed validation; no mesh emitted'],
  };
}

// --- Box -----------------------------------------------------------------
//
// Six flat-shaded faces. Each face is a regular subdivision grid that
// emits CCW triangles when viewed from outside (so the face normal
// agrees with the geometric triangle normal). The triangle winding
// `(i0, i1, i2)` and `(i1, i3, i2)` produces a geometric normal of
// `+u × +v`; we choose `u` and `v` so that direction matches the face
// outward direction.

export function buildBox(p: BoxParameters): StudioLocalPreview {
  const err = validateBox(p);
  if (err) return emptyPreview('box', p);
  const hx = p.size_x / 2;
  const hy = p.size_y / 2;
  const hz = p.size_z / 2;
  const n = p.subdivisions;

  type Face = { normal: readonly [number, number, number]; uAxis: 0 | 1 | 2; vAxis: 0 | 1 | 2; constant: 0 | 1 | 2; offset: number };
  // For each axis sign we pick `u`, `v` so that `+u × +v` equals the
  // outward direction. The axis order is a right-handed cycle, so we
  // pair each constant axis with the next two in (X, Y, Z) order, with
  // the order swapped when the outward sign is negative.
  const faces: readonly Face[] = [
    { normal: [1, 0, 0], uAxis: 1, vAxis: 2, constant: 0, offset: hx },
    { normal: [-1, 0, 0], uAxis: 2, vAxis: 1, constant: 0, offset: -hx },
    { normal: [0, 1, 0], uAxis: 2, vAxis: 0, constant: 1, offset: hy },
    { normal: [0, -1, 0], uAxis: 0, vAxis: 2, constant: 1, offset: -hy },
    { normal: [0, 0, 1], uAxis: 0, vAxis: 1, constant: 2, offset: hz },
    { normal: [0, 0, -1], uAxis: 1, vAxis: 0, constant: 2, offset: -hz },
  ];

  const halfExtents: readonly [number, number, number] = [hx, hy, hz];
  const positions: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];
  for (const face of faces) {
    const base = positions.length / 3;
    for (let row = 0; row <= n; row++) {
      const v = (row / n) * 2 - 1;
      for (let col = 0; col <= n; col++) {
        const u = (col / n) * 2 - 1;
        const coord: [number, number, number] = [0, 0, 0];
        coord[face.uAxis] = u * halfExtents[face.uAxis]!;
        coord[face.vAxis] = v * halfExtents[face.vAxis]!;
        coord[face.constant] = face.offset;
        positions.push(coord[0], coord[1], coord[2]);
        normals.push(face.normal[0], face.normal[1], face.normal[2]);
      }
    }
    for (let row = 0; row < n; row++) {
      for (let col = 0; col < n; col++) {
        const i0 = base + row * (n + 1) + col;
        const i1 = i0 + 1;
        const i2 = i0 + (n + 1);
        const i3 = i2 + 1;
        // Geometric normal of `(i0, i1, i2)` is `+u × +v`, which by the
        // face table equals the outward direction.
        indices.push(i0, i1, i2);
        indices.push(i1, i3, i2);
      }
    }
  }
  return finishPreview('box', p, positions, normals, indices);
}

// --- Cylinder ------------------------------------------------------------
//
// Side wall tessellated into radial_segments × height_segments quads with
// shared vertices giving smooth per-vertex normals; two end caps with their
// own pole-aligned normals. CCW when viewed from outside everywhere.

export function buildCylinder(p: CylinderParameters): StudioLocalPreview {
  const err = validateCylinder(p);
  if (err) return emptyPreview('cylinder', p);
  const radial = p.radial_segments;
  const rings = p.height_segments + 1;
  const halfH = p.height / 2;

  const positions: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];

  for (let ringIdx = 0; ringIdx < rings; ringIdx++) {
    const z = -halfH + (ringIdx / p.height_segments) * p.height;
    for (let seg = 0; seg <= radial; seg++) {
      const theta = (seg / radial) * TWO_PI;
      const cosT = Math.cos(theta);
      const sinT = Math.sin(theta);
      positions.push(p.radius * cosT, p.radius * sinT, z);
      normals.push(cosT, sinT, 0);
    }
  }
  for (let ringIdx = 0; ringIdx < p.height_segments; ringIdx++) {
    for (let seg = 0; seg < radial; seg++) {
      const i0 = ringIdx * (radial + 1) + seg;
      const i1 = i0 + 1;
      const i2 = i0 + (radial + 1);
      const i3 = i2 + 1;
      // CCW from outside the wall. Vertex grid is col-major over
      // (theta, z). With theta increasing first and z increasing second,
      // the geometric normal of (i0, i1, i2) is +theta × +z which points
      // radially outward at every quad.
      indices.push(i0, i1, i2);
      indices.push(i1, i3, i2);
    }
  }
  // Top cap: pole at +halfH.
  const topPole = positions.length / 3;
  positions.push(0, 0, halfH);
  normals.push(0, 0, 1);
  const topRingStart = positions.length / 3;
  for (let seg = 0; seg <= radial; seg++) {
    const theta = (seg / radial) * TWO_PI;
    const cosT = Math.cos(theta);
    const sinT = Math.sin(theta);
    positions.push(p.radius * cosT, p.radius * sinT, halfH);
    normals.push(0, 0, 1);
  }
  for (let seg = 0; seg < radial; seg++) {
    const i0 = topRingStart + seg;
    const i1 = i0 + 1;
    // CCW from above (+Z looking down at the cap): pole, then i0, then i1.
    indices.push(topPole, i0, i1);
  }
  // Bottom cap: pole at -halfH, winding reversed.
  const bottomPole = positions.length / 3;
  positions.push(0, 0, -halfH);
  normals.push(0, 0, -1);
  const bottomRingStart = positions.length / 3;
  for (let seg = 0; seg <= radial; seg++) {
    const theta = (seg / radial) * TWO_PI;
    const cosT = Math.cos(theta);
    const sinT = Math.sin(theta);
    positions.push(p.radius * cosT, p.radius * sinT, -halfH);
    normals.push(0, 0, -1);
  }
  for (let seg = 0; seg < radial; seg++) {
    const i0 = bottomRingStart + seg;
    const i1 = i0 + 1;
    indices.push(bottomPole, i1, i0);
  }

  return finishPreview('cylinder', p, positions, normals, indices);
}

// --- Sphere --------------------------------------------------------------

export function buildSphere(p: SphereParameters): StudioLocalPreview {
  const err = validateSphere(p);
  if (err) return emptyPreview('sphere', p);
  const rings = p.rings;
  const sectors = p.sectors;
  const positions: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];

  // Vertex layout: rings 0..rings inclusive. Ring 0 is the north pole, ring
// rings is the south pole, and the rings in between each have `sectors+1`
// longitude vertices. The poles share a single vertex index each so the
// fan triangles around them stay non-degenerate.
const northPoleIndex = 0;
positions.push(0, 0, p.radius);
normals.push(0, 0, 1);

for (let ringIdx = 1; ringIdx < rings; ringIdx++) {
    const phi = (Math.PI * ringIdx) / rings;
    const sinPhi = Math.sin(phi);
    const cosPhi = Math.cos(phi);
    for (let sec = 0; sec <= sectors; sec++) {
      const theta = (TWO_PI * sec) / sectors;
      const cosT = Math.cos(theta);
      const sinT = Math.sin(theta);
      const x = p.radius * sinPhi * cosT;
      const y = p.radius * sinPhi * sinT;
      const z = p.radius * cosPhi;
      positions.push(x, y, z);
      normals.push(x / p.radius, y / p.radius, z / p.radius);
    }
}

const southPoleIndex = positions.length / 3;
positions.push(0, 0, -p.radius);
normals.push(0, 0, -1);

// First non-pole ring starts at index 1; sector s of ring r is at
// `1 + (r - 1) * (sectors + 1) + s` for r in [1, rings-1].
for (let ringIdx = 0; ringIdx < rings; ringIdx++) {
    for (let sec = 0; sec < sectors; sec++) {
      if (ringIdx === 0) {
        // Top fan: north pole → ring1 sec → ring1 sec+1.
        const i0 = 1 + sec;
        const i1 = 1 + sec + 1;
        indices.push(northPoleIndex, i0, i1);
        continue;
      }
      if (ringIdx === rings - 1) {
        // Bottom fan: ringN sec+1 → ringN sec → south pole. The reverse
        // order keeps the geometric normal pointing -Z.
        const i0 = 1 + (ringIdx - 1) * (sectors + 1) + sec + 1;
        const i1 = 1 + (ringIdx - 1) * (sectors + 1) + sec;
        indices.push(southPoleIndex, i0, i1);
        continue;
      }
      const base = 1 + (ringIdx - 1) * (sectors + 1);
      const i0 = base + sec;
      const i1 = i0 + 1;
      const i2 = i0 + (sectors + 1);
      const i3 = i2 + 1;
      // CCW from outside: the geometric normal of (i0, i1, i2) is the
      // outward radial direction at that quad.
      indices.push(i0, i1, i2);
      indices.push(i1, i3, i2);
    }
  }

  return finishPreview('sphere', p, positions, normals, indices);
}

function finishPreview(
  kind: PrimitiveKind,
  p: StudioParameters,
  positions: ReadonlyArray<number>,
  normals: ReadonlyArray<number>,
  indices: ReadonlyArray<number>,
): StudioLocalPreview {
  if (positions.length / 3 > MAX_MESH_VERTICES) {
    return emptyPreview(kind, p);
  }
  const positionsF32 = new Float32Array(positions);
  const normalsF32 = new Float32Array(normals);
  const indicesU32 = new Uint32Array(indices);
  return {
    kind,
    parameters: p,
    positions: positionsF32,
    normals: normalsF32,
    indices: indicesU32,
    bounds: boundsFromPositions(positionsF32),
    source: 'local',
    generatedAt: Date.now(),
    engine: LOCAL_ENGINE,
    warnings: [],
  };
}

export function buildLocal(kind: PrimitiveKind, p: StudioParameters): StudioLocalPreview {
  if (kind === 'box') return buildBox(p as BoxParameters);
  if (kind === 'cylinder') return buildCylinder(p as CylinderParameters);
  return buildSphere(p as SphereParameters);
}

export function boundsDimensions(b: BBox): { width: number; height: number; depth: number } {
  return {
    width: b.max[0] - b.min[0],
    height: b.max[1] - b.min[1],
    depth: b.max[2] - b.min[2],
  };
}

export function formatMillimetres(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return `${value.toFixed(2)} mm`;
}

// --- Nominal geometry estimates -----------------------------------------
//
// Closed-form analytical values for the *ideal* solid each primitive
// represents — the same numbers an OpenCascade (OCP) volume/area property
// query returns for an unwrapped box, cylinder, or sphere of these
// dimensions. They are exact for the ideal solid (no fillet, draft, or
// tessellation error), which makes them useful for:
//
//   * early sizing and mass/volume sanity checks,
//   * quoting a bounding envelope to whoever does the detailed modelling,
//   * a neutral STEP/STL handoff into an external CAD/CAE package.
//
// They are deliberately NOT a simulation: nothing here is a stress, strain,
// deflection, or tolerance result. Callers must not present them as FEA.
//
// Every value is a magnitude, so a negative (already invalid) dimension
// still yields a well-formed extent rather than a negative volume. Range
// validation stays where it belongs — `validateParameters`.

export interface GeometryEstimate {
  /** Bounding-envelope extents ΔX, ΔY, ΔZ in millimetres. */
  envelopeMm: [number, number, number];
  /** Product of the envelope extents (ΔX · ΔY · ΔZ) in mm³. */
  envelopeVolumeMm3: number;
  /** Exact nominal solid volume in mm³. */
  volumeMm3: number;
  /** The same volume in cm³. */
  volumeCm3: number;
  /** Exact nominal surface area in mm². */
  surfaceAreaMm2: number;
  /** The same area in cm². */
  surfaceAreaCm2: number;
}

/**
 * Closed-form nominal geometry for a primitive. Pure: no DOM, no store, no
 * network. Units are millimetres throughout; the cm³/cm² members are simple
 * /1000 and /100 conversions so downstream CAD/CAE tools get both.
 */
export function computeGeometryEstimate(
  kind: PrimitiveKind,
  parameters: StudioParameters,
): GeometryEstimate {
  if (kind === 'box') {
    const p = parameters as BoxParameters;
    const x = Math.abs(p.size_x);
    const y = Math.abs(p.size_y);
    const z = Math.abs(p.size_z);
    // V = X · Y · Z   A = 2(XY + YZ + ZX)
    return finishEstimate(
      [x, y, z],
      x * y * z,
      2 * (x * y + y * z + z * x),
    );
  }
  if (kind === 'cylinder') {
    const p = parameters as CylinderParameters;
    const radius = Math.abs(p.radius);
    const height = Math.abs(p.height);
    const diameter = radius * 2;
    // V = π r² h   A = 2πr(r + h)   (the side wall plus both end caps)
    return finishEstimate(
      [diameter, diameter, height],
      Math.PI * radius * radius * height,
      2 * Math.PI * radius * (radius + height),
    );
  }
  const p = parameters as SphereParameters;
  const radius = Math.abs(p.radius);
  const diameter = radius * 2;
  // V = ⁴⁄₃ π r³   A = 4πr²
  return finishEstimate(
    [diameter, diameter, diameter],
    (4 / 3) * Math.PI * radius ** 3,
    4 * Math.PI * radius * radius,
  );
}

function finishEstimate(
  envelopeMm: [number, number, number],
  volumeMm3: number,
  surfaceAreaMm2: number,
): GeometryEstimate {
  return {
    envelopeMm,
    envelopeVolumeMm3: envelopeMm[0] * envelopeMm[1] * envelopeMm[2],
    volumeMm3,
    volumeCm3: volumeMm3 / 1000,
    surfaceAreaMm2,
    surfaceAreaCm2: surfaceAreaMm2 / 100,
  };
}

/** Human-readable volume pair, e.g. `24000.00 mm³ (24.000 cm³)`. */
export function formatVolumeEstimate(estimate: GeometryEstimate): string {
  if (!Number.isFinite(estimate.volumeMm3) || !Number.isFinite(estimate.volumeCm3)) return '—';
  return `${estimate.volumeMm3.toFixed(2)} mm³ (${estimate.volumeCm3.toFixed(3)} cm³)`;
}

/** Human-readable area pair, e.g. `5200.00 mm² (52.000 cm²)`. */
export function formatAreaEstimate(estimate: GeometryEstimate): string {
  if (!Number.isFinite(estimate.surfaceAreaMm2) || !Number.isFinite(estimate.surfaceAreaCm2)) return '—';
  return `${estimate.surfaceAreaMm2.toFixed(2)} mm² (${estimate.surfaceAreaCm2.toFixed(3)} cm²)`;
}

/** Human-readable bounding envelope, e.g. `40.00 × 20.00 × 30.00 mm`. */
export function formatEnvelopeEstimate(estimate: GeometryEstimate): string {
  const [x, y, z] = estimate.envelopeMm;
  if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) return '—';
  return `${x.toFixed(2)} × ${y.toFixed(2)} × ${z.toFixed(2)} mm`;
}

/** Human-readable bounding-envelope volume (ΔX · ΔY · ΔZ), e.g. `24000.00 mm³`. */
export function formatEnvelopeVolumeEstimate(estimate: GeometryEstimate): string {
  if (!Number.isFinite(estimate.envelopeVolumeMm3)) return '—';
  return `${estimate.envelopeVolumeMm3.toFixed(2)} mm³`;
}