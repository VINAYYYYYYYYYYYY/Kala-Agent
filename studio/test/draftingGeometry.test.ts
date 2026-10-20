// Pure drafting-geometry coverage for Section A-A and the orthographic
// three-view. These assertions pin real parameter/bounds arithmetic, not
// rendered markup.

import { describe, expect, it } from 'vitest';
import {
  DRAFTING_SCALE,
  boundsKey,
  chainDashSegments,
  computeCentreLineAxes,
  computeOrthoViews,
  computeSectionView,
  fitToBox,
  formatMmDimension,
  formatMmValue,
  formatSectionPlaneLabel,
  isCircularExtent,
  sectionPlaneZ,
  shouldReframeBounds,
} from '../src/viewer/draftingGeometry';
import type { BBox, PrimitiveKind } from '../src/types/studio';

const box = { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 };
const cylinder = { radius: 20, height: 40, radial_segments: 48, height_segments: 1 };
const sphere = { radius: 25, rings: 24, sectors: 32 };

function bounds(width: number, height: number, depth: number): BBox {
  return {
    min: [-width / 2, -height / 2, -depth / 2],
    max: [width / 2, height / 2, depth / 2],
  };
}

describe('Section A-A math', () => {
  it('cuts a box on the Z mid-plane into a width × height rectangle', () => {
    const section = computeSectionView('box', box);
    expect(section.kind).toBe('rectangle');
    expect(section.widthMm).toBe(40);
    expect(section.heightMm).toBe(20);
    expect(section.planeLabel).toContain('Z = 0.00 mm');
    expect(section.planeLabel).toContain('mid-plane');
    expect(section.reason).toBeUndefined();
  });

  it('cuts a cylinder normal to its axis into a real circle', () => {
    const section = computeSectionView('cylinder', cylinder);
    expect(section.kind).toBe('circle');
    expect(section.widthMm).toBe(40);
    expect(section.heightMm).toBe(40);
    expect(section.planeLabel).toContain('normal to axis');
  });

  it('cuts a sphere on the mid-plane into a great circle', () => {
    const section = computeSectionView('sphere', sphere);
    expect(section.kind).toBe('circle');
    expect(section.widthMm).toBe(50);
    expect(section.heightMm).toBe(50);
    expect(section.planeLabel).toContain('great circle');
  });

  it('is honest for an unsupported primitive instead of inventing a profile', () => {
    const section = computeSectionView('torus' as PrimitiveKind, box);
    expect(section.kind).toBe('unavailable');
    expect(section.reason).toContain('unavailable');
  });

  it('refuses non-positive parameters', () => {
    const section = computeSectionView('box', { ...box, size_x: 0 });
    expect(section.kind).toBe('unavailable');
    expect(section.reason).toContain('not valid');
  });
});

describe('section cutting plane', () => {
  it('cuts at the real bounds mid-plane when the mesh is not centred', () => {
    const offset: BBox = { min: [-20, -10, 0], max: [20, 10, 30] };
    expect(sectionPlaneZ(offset)).toBe(15);

    const section = computeSectionView('box', box, offset);
    expect(section.planeLabel).toBe('Z = 15.00 mm · mid-plane');
    expect(section.planeZMm).toBe(15);
    // The profile itself is unchanged: the box is still 40 × 20 mm.
    expect(section.kind).toBe('rectangle');
    expect(section.widthMm).toBe(40);
    expect(section.heightMm).toBe(20);
  });

  it('labels a cylinder and a sphere cut at the real mid-plane', () => {
    const offset: BBox = { min: [-20, -20, -5], max: [20, 20, 25] };
    expect(computeSectionView('cylinder', cylinder, offset).planeLabel).toBe(
      'Z = 10.00 mm · normal to axis',
    );
    expect(computeSectionView('sphere', sphere, offset).planeLabel).toBe(
      'Z = 10.00 mm · great circle',
    );
  });

  it('falls back to the centred contract when no bounds exist', () => {
    expect(sectionPlaneZ(null)).toBe(0);
    expect(computeSectionView('box', box).planeLabel).toBe('Z = 0.00 mm · mid-plane');
    expect(computeSectionView('box', box, null).planeLabel).toBe('Z = 0.00 mm · mid-plane');
  });

  it('never prints a negative zero plane', () => {
    expect(formatSectionPlaneLabel(-0, 'mid-plane')).toBe('Z = 0.00 mm · mid-plane');
    expect(formatSectionPlaneLabel(-0.001, 'mid-plane')).toBe('Z = 0.00 mm · mid-plane');
    expect(formatSectionPlaneLabel(-12.5, 'mid-plane')).toBe('Z = -12.50 mm · mid-plane');
  });
});

describe('drafted centre lines', () => {
  const box40 = bounds(40, 20, 30);

  it('runs the X and Y centre lines through the part centre', () => {
    const axes = computeCentreLineAxes(box40, 6);
    expect(axes).toHaveLength(2);

    const x = axes[0]!;
    expect(x.id).toBe('x');
    expect(x.label).toBe('+X (WIDTH)');
    expect(x.from.y).toBe(0);
    expect(x.to.y).toBe(0);
    expect(x.from.x).toBe(-26);
    expect(x.to.x).toBe(26);

    const y = axes[1]!;
    expect(y.id).toBe('y');
    expect(y.label).toBe('+Y (HEIGHT)');
    expect(y.from.x).toBe(0);
    expect(y.to.x).toBe(0);
    expect(y.from.y).toBe(-16);
    expect(y.to.y).toBe(16);
  });

  it('follows a mesh that is not centred on the origin', () => {
    const offset: BBox = { min: [0, 0, 0], max: [40, 20, 30] };
    const axes = computeCentreLineAxes(offset, 0);
    expect(axes[0]!.from.y).toBe(10);
    expect(axes[0]!.to.y).toBe(10);
    expect(axes[0]!.from.z).toBe(15);
    expect(axes[1]!.from.x).toBe(20);
  });

  it('draws a chain-dash (— · —) along the line', () => {
    const segments = chainDashSegments(
      { x: 0, y: 0, z: 0 },
      { x: 40, y: 0, z: 0 },
      [10, 2, 2, 2],
    );
    expect(segments.length).toBeGreaterThan(1);
    // Every dash starts on the line and runs forwards.
    for (const [from, to] of segments) {
      expect(from.y).toBe(0);
      expect(to.y).toBe(0);
      expect(to.x).toBeGreaterThan(from.x);
    }
    // The pattern is long dash, gap, dot, gap: the first dash is the long one.
    expect(segments[0]![1].x).toBeCloseTo(10, 6);
    // The dashes never run past the end of the line.
    const last = segments[segments.length - 1]!;
    expect(last[1].x).toBeLessThanOrEqual(40);
  });

  it('has nothing to draw for a degenerate line', () => {
    expect(chainDashSegments({ x: 1, y: 1, z: 1 }, { x: 1, y: 1, z: 1 }, [10, 2, 2, 2])).toEqual([]);
    expect(chainDashSegments({ x: 0, y: 0, z: 0 }, { x: 10, y: 0, z: 0 }, [])).toEqual([]);
  });
});

describe('sheet scale', () => {
  it('states FIT because the viewport frames the part to the sheet', () => {
    expect(DRAFTING_SCALE).toBe('FIT');
  });
});

describe('camera framing predicate', () => {
  const box40 = bounds(40, 20, 30);

  it('re-frames the first mesh and any mesh whose bounds changed', () => {
    expect(shouldReframeBounds(null, box40)).toBe(true);
    expect(shouldReframeBounds(box40, bounds(40, 20, 31))).toBe(true);
    expect(shouldReframeBounds(box40, bounds(41, 20, 30))).toBe(true);
  });

  it('keeps the operator camera when the bounds are unchanged', () => {
    expect(shouldReframeBounds(box40, bounds(40, 20, 30))).toBe(false);
    // A rebuilt mesh with the same extents is not a new view.
    expect(shouldReframeBounds(box40, { ...box40 })).toBe(false);
  });

  it('ignores float noise below one micrometre', () => {
    const noisy: BBox = {
      min: [-20 + 1e-9, -10, -15],
      max: [20, 10, 15],
    };
    expect(shouldReframeBounds(box40, noisy)).toBe(false);
    expect(boundsKey(noisy)).toBe(boundsKey(box40));
  });

  it('has nothing to frame without a mesh', () => {
    expect(shouldReframeBounds(null, null)).toBe(false);
    expect(shouldReframeBounds(box40, null)).toBe(false);
  });
});

describe('orthographic projection math', () => {
  it('projects a box from real bounds into front/top/right rectangles', () => {
    const views = computeOrthoViews('box', box, bounds(40, 20, 30));
    expect(views).not.toBeNull();
    expect(views!.map((view) => view.id)).toEqual(['front', 'top', 'right']);
    expect(views!.map((view) => view.kind)).toEqual(['rect', 'rect', 'rect']);
    expect(views![0]).toMatchObject({ label: 'FRONT', widthMm: 40, heightMm: 20 });
    expect(views![1]).toMatchObject({ label: 'TOP', widthMm: 40, heightMm: 30 });
    expect(views![2]).toMatchObject({ label: 'RIGHT', widthMm: 30, heightMm: 20 });
    expect(views![0]!.direction).toContain('−Z');
  });

  it('shows a cylinder front view as a circle because its axis is Z', () => {
    const views = computeOrthoViews('cylinder', cylinder, bounds(40, 40, 40))!;
    expect(views[0]!.kind).toBe('circle');
    expect(views[1]!.kind).toBe('rect');
    expect(views[2]!.kind).toBe('rect');
    expect(views[0]!.widthMm).toBe(40);
    expect(views[0]!.heightMm).toBe(40);
  });

  it('shows all three sphere views as circles', () => {
    const views = computeOrthoViews('sphere', sphere, bounds(50, 50, 50))!;
    expect(views.map((view) => view.kind)).toEqual(['circle', 'circle', 'circle']);
    expect(views[0]!.widthMm).toBe(50);
  });

  it('returns null before a preview or for degenerate bounds', () => {
    expect(computeOrthoViews('box', box, null)).toBeNull();
    expect(computeOrthoViews('box', box, bounds(0, 20, 30))).toBeNull();
    expect(
      computeOrthoViews('box', box, {
        min: [0, 0, 0],
        max: [Number.NaN, 20, 30],
      }),
    ).toBeNull();
  });

  it('does not pretend a non-circular extent is a circle', () => {
    expect(isCircularExtent(40, 40)).toBe(true);
    expect(isCircularExtent(40, 30)).toBe(false);
    const views = computeOrthoViews('sphere', sphere, bounds(50, 49, 50))!;
    expect(views[1]!.kind).toBe('circle');
    const warped = computeOrthoViews('sphere', sphere, bounds(50, 40, 50))!;
    expect(warped[0]!.kind).toBe('rect');
    expect(warped[1]!.kind).toBe('circle');
  });
});

describe('drawing fit', () => {
  it('fits a profile into a cell while preserving aspect ratio', () => {
    const fit = fitToBox(300, 120, 40, 20, 20);
    // The padded cell is 260 × 80, so height constrains the fit.
    expect(fit.scale).toBe(4);
    expect(fit.width).toBe(160);
    expect(fit.height).toBe(80);
    expect(fit.x).toBe(70);
    expect(fit.y).toBe(20);
    expect(fit.x + fit.width / 2).toBeCloseTo(150, 6);
    expect(fit.y + fit.height / 2).toBeCloseTo(60, 6);
  });

  it('returns a zero box for invalid cells or content', () => {
    expect(fitToBox(0, 100, 10, 10, 2)).toEqual({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      scale: 0,
    });
    expect(fitToBox(100, 100, 0, 10, 2).width).toBe(0);
  });
});

describe('drafting number formatting', () => {
  it('always uses two decimals and explicit millimetres', () => {
    expect(formatMmValue(40)).toBe('40.00');
    expect(formatMmValue(19.999)).toBe('20.00');
    expect(formatMmDimension(40)).toBe('40.00 mm');
    expect(formatMmValue(Number.NaN)).toBe('—');
    expect(formatMmDimension(Number.POSITIVE_INFINITY)).toBe('—');
  });
});
