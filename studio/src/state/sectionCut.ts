// The shared Section A–A cutting plane.
//
// One plane serves the drafting sheet, the redline A–A trace drawn in the
// HLR views, the auxiliary section panel, and the clay viewport's cutting
// plane. Keeping the plane and its offset in the store means those four
// surfaces cannot disagree: changing the plane in the AuxView changes the
// label on the sheet and the line in the drawing in the same render.
//
// The offset is measured along the plane normal, exactly like the backend:
//   XY → normal +Z, offset is z
//   XZ → normal +Y, offset is y
//   YZ → normal +X, offset is x
//
// `offsetMm: null` means "the backend's documented default": the mid-plane
// of the measured bounds normal to the plane. The UI never invents an offset
// for that case; it reads the mid-plane from the bounds it already holds.
//
// Nothing here is a measurement: the label prints the plane the operator
// selected and the offset the backend was asked for. The measured area and
// loop counts stay in the `/section` response.

import type { BBox, SectionPlane } from '../types/studio';

/** A principal cutting plane and its offset along the plane normal. */
export interface SectionCut {
  plane: SectionPlane;
  /** Millimetres along the plane normal; null asks for the mid-plane. */
  offsetMm: number | null;
}

/**
 * The design default. ADR-0009 criterion 4 cuts the L-bracket on XZ at
 * y = T/2 = 4 mm: through the upright wall and the base, clear of the holes.
 */
export const DEFAULT_DESIGN_SECTION_CUT: SectionCut = { plane: 'XZ', offsetMm: 4 };

/**
 * The documented cut for a plane chosen in the AuxView. XZ keeps the
 * criterion-4 y = 4 mm offset; XY and YZ use the backend's mid-plane default
 * (null), which the label resolves from the measured bounds.
 */
export function defaultSectionCutForPlane(plane: SectionPlane): SectionCut {
  return plane === 'XZ' ? { plane, offsetMm: 4 } : { plane, offsetMm: null };
}

/**
 * The primitive default. Every primitive is built on the origin, and the
 * existing drafting convention cuts them on the XY plane at the Z mid-plane.
 */
export const PRIMITIVE_SECTION_CUT: SectionCut = { plane: 'XY', offsetMm: null };

/** The index of the coordinate the plane normal points along. */
export const SECTION_PLANE_AXIS: Readonly<Record<SectionPlane, 0 | 1 | 2>> = {
  XY: 2,
  XZ: 1,
  YZ: 0,
};

/** The axis name the plane fixes: XY fixes Z, XZ fixes Y, YZ fixes X. */
export const SECTION_PLANE_AXIS_NAME: Readonly<Record<SectionPlane, 'X' | 'Y' | 'Z'>> = {
  XY: 'Z',
  XZ: 'Y',
  YZ: 'X',
};

/** The unit normal of each plane in the backend's order (X, Y, Z). */
export const SECTION_PLANE_NORMAL: Readonly<
  Record<SectionPlane, readonly [number, number, number]>
> = {
  XY: [0, 0, 1],
  XZ: [0, 1, 0],
  YZ: [1, 0, 0],
};

/** The offset the backend uses when `offsetMm` is null, or null without bounds. */
export function sectionCutOffset(cut: SectionCut, bounds: BBox | null | undefined): number | null {
  if (cut.offsetMm !== null) {
    return Number.isFinite(cut.offsetMm) ? cut.offsetMm : null;
  }
  if (!bounds) return null;
  const axis = SECTION_PLANE_AXIS[cut.plane];
  const low = bounds.min[axis];
  const high = bounds.max[axis];
  if (!Number.isFinite(low) || !Number.isFinite(high)) return null;
  return (low + high) / 2;
}

/** Fixed two-decimal drafting number. */
function formatCutValue(value: number): string {
  const rounded = Math.round(value * 100) / 100;
  return (rounded === 0 ? 0 : rounded).toFixed(2);
}

/**
 * The cutting-plane label, e.g. `XZ · Y = 4.00 mm` for the explicit ADR cut,
 * or `XY · Z = 20.00 mm · mid-plane` for a default cut. The value is the
 * offset along the normal, read from the shared cut state (never inferred
 * from a different plane or a stale design).
 */
export function sectionCutLabel(cut: SectionCut, bounds: BBox | null | undefined): string {
  const axis = SECTION_PLANE_AXIS_NAME[cut.plane];
  const offset = sectionCutOffset(cut, bounds);
  if (offset === null) return `${cut.plane} · ${axis} = —`;
  if (cut.offsetMm === null) {
    return `${cut.plane} · ${axis} = ${formatCutValue(offset)} mm · mid-plane`;
  }
  return `${cut.plane} · ${axis} = ${formatCutValue(offset)} mm`;
}

/** The full sheet/heading note, e.g. `CUT A–A · XZ · Y = 4.00 mm`. */
export function sectionCutNote(cut: SectionCut, bounds: BBox | null | undefined): string {
  return `CUT A–A · ${sectionCutLabel(cut, bounds)}`;
}

/**
 * The four corners of the cutting plane, padded past the part, in loop
 * order. The clay viewport draws these as its redline plane; the same
 * ordering is used for every plane so the line is continuous.
 */
export function sectionCutCorners(
  bounds: BBox,
  cut: SectionCut,
  pad: number,
): Array<[number, number, number]> {
  const margin = Number.isFinite(pad) ? Math.max(pad, 0) : 0;
  const [x0, y0, z0] = bounds.min;
  const [x1, y1, z1] = bounds.max;
  const offset = sectionCutOffset(cut, bounds);
  const value = offset ?? 0;
  if (cut.plane === 'XY') {
    return [
      [x0 - margin, y0 - margin, value],
      [x1 + margin, y0 - margin, value],
      [x1 + margin, y1 + margin, value],
      [x0 - margin, y1 + margin, value],
    ];
  }
  if (cut.plane === 'XZ') {
    return [
      [x0 - margin, value, z0 - margin],
      [x1 + margin, value, z0 - margin],
      [x1 + margin, value, z1 + margin],
      [x0 - margin, value, z1 + margin],
    ];
  }
  return [
    [value, y0 - margin, z0 - margin],
    [value, y1 + margin, z0 - margin],
    [value, y1 + margin, z1 + margin],
    [value, y0 - margin, z1 + margin],
  ];
}

/** True when two cuts describe the same plane and offset. */
export function sameSectionCut(left: SectionCut, right: SectionCut): boolean {
  return left.plane === right.plane && left.offsetMm === right.offsetMm;
}
