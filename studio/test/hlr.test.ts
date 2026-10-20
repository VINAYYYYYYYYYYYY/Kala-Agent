// Pure 2D HLR drawing geometry: fit, path generation, and pan/zoom.

import { describe, expect, it } from 'vitest';
import {
  IDENTITY_TRANSFORM,
  MAX_DRAWING_ZOOM,
  VIEW_MEASURED_AXES,
  clampDrawingZoom,
  drawingTransformAttribute,
  fitPolylineGroups,
  measuredAxisValue,
  measuredEnvelope,
  panDrawing,
  polylinesExtent,
  zoomDrawingAt,
} from '../src/viewer/hlr';
import type { DrawingPolyline } from '../src/types/studio';
import { bracketDesignResult, notReportedDesignResult } from './designFixtures';

function polyline(
  points: ReadonlyArray<[number, number]>,
  closed = false,
): DrawingPolyline {
  return { points, closed, point_count: points.length };
}

describe('polyline extents', () => {
  it('spans every finite point in every group', () => {
    const extent = polylinesExtent([
      [polyline([[-30, 0], [-30, 8]])],
      [polyline([[0, 19], [4.99, 29]])],
    ]);
    expect(extent).toEqual({
      minU: -30,
      minV: 0,
      maxU: 4.99,
      maxV: 29,
      width: 34.99,
      height: 29,
    });
  });

  it('is null when there is nothing drawable', () => {
    expect(polylinesExtent([])).toBeNull();
    expect(polylinesExtent([[polyline([])]])).toBeNull();
    expect(polylinesExtent([[polyline([[Number.NaN, 1]])]])).toBeNull();
  });
});

describe('fitting polylines to a cell', () => {
  it('preserves aspect ratio and flips the vertical axis for SVG', () => {
    const fit = fitPolylineGroups(
      [[polyline([[0, 0], [10, 0], [10, 5]])]],
      { x: 0, y: 0, width: 100, height: 100 },
      0,
    )!;
    expect(fit.scale).toBeCloseTo(10, 6);
    // 10 x 5 mm in a 100 x 100 cell: scale 10, centred vertically.
    expect(fit.originX).toBeCloseTo(0, 6);
    expect(fit.originY).toBeCloseTo(25, 6);
    // (0, 0) is the bottom-left of the part, so it lands low in the cell.
    expect(fit.paths[0]).toBe('M 0 75 L 100 75 L 100 25');
  });

  it('closes a closed loop with Z', () => {
    const fit = fitPolylineGroups(
      [[polyline([[0, 0], [10, 0], [10, 5]], true)]],
      { x: 0, y: 0, width: 100, height: 100 },
      0,
    )!;
    expect(fit.paths[0]!.endsWith('Z')).toBe(true);
  });

  it('keeps visible and hidden groups under one transform', () => {
    const fit = fitPolylineGroups(
      [
        [polyline([[-30, 0], [-30, 40]])],
        [polyline([[30, 0], [30, 8]])],
      ],
      { x: 10, y: 10, width: 200, height: 200 },
      10,
    )!;
    expect(fit.paths).toHaveLength(2);
    expect(fit.paths[0]).not.toBe(fit.paths[1]);
    expect(fit.paths[0]).toContain('M ');
  });

  it('fits a perfectly straight line instead of dividing by zero', () => {
    const fit = fitPolylineGroups(
      [[polyline([[5, 0], [5, 10]])]],
      { x: 0, y: 0, width: 100, height: 100 },
      0,
    )!;
    expect(Number.isFinite(fit.scale)).toBe(true);
    expect(fit.scale).toBeGreaterThan(0);
    const coordinates = fit.paths[0]!.match(/-?\d+(?:\.\d+)?/g) ?? [];
    expect(coordinates.every((value) => Number.isFinite(Number(value)))).toBe(true);
  });

  it('returns null for an empty set or a degenerate cell', () => {
    expect(fitPolylineGroups([], { x: 0, y: 0, width: 10, height: 10 }, 1)).toBeNull();
    expect(
      fitPolylineGroups(
        [[polyline([[0, 0], [1, 1]])]],
        { x: 0, y: 0, width: 0, height: 10 },
        1,
      ),
    ).toBeNull();
  });
});

describe('pan and zoom', () => {
  it('keeps the cursor point fixed while zooming', () => {
    const point = { x: 100, y: 50 };
    const next = zoomDrawingAt(IDENTITY_TRANSFORM, point, 2);
    expect(next.zoom).toBe(2);
    // p_screen = pan + zoom * p_original
    expect(next.panX + next.zoom * point.x).toBeCloseTo(point.x, 6);
    expect(next.panY + next.zoom * point.y).toBeCloseTo(point.y, 6);
  });

  it('clamps zoom to the honest bounds', () => {
    expect(clampDrawingZoom(0.01)).toBe(0.5);
    expect(clampDrawingZoom(1000)).toBe(MAX_DRAWING_ZOOM);
    expect(clampDrawingZoom(Number.NaN)).toBe(1);
    expect(zoomDrawingAt(IDENTITY_TRANSFORM, { x: 0, y: 0 }, 1000).zoom).toBe(MAX_DRAWING_ZOOM);
  });

  it('translates without changing the zoom', () => {
    const next = panDrawing({ zoom: 2, panX: 1, panY: 2 }, 10, -4);
    expect(next).toEqual({ zoom: 2, panX: 11, panY: -2 });
    expect(panDrawing(next, 0, 0)).toBe(next);
  });

  it('serialises the SVG transform in translate-then-scale order', () => {
    expect(drawingTransformAttribute({ zoom: 1.5, panX: -20, panY: 8 })).toBe(
      'translate(-20 8) scale(1.5)',
    );
  });
});

describe('measured callout axes', () => {
  it('reads the provider-measured bounds, not declared envelopes', () => {
    expect(measuredEnvelope(bracketDesignResult.measured)).toEqual({ x: 60, y: 45, z: 40 });
  });

  it('returns null when the provider reported no bounds', () => {
    expect(measuredEnvelope(notReportedDesignResult.measured)).toBeNull();
  });

  it('maps each orthographic view to the axes A2 publishes', () => {
    const envelope = measuredEnvelope(bracketDesignResult.measured)!;
    expect(VIEW_MEASURED_AXES.front).toEqual({ horizontal: 'X', vertical: 'Z' });
    expect(VIEW_MEASURED_AXES.top).toEqual({ horizontal: 'X', vertical: 'Y' });
    expect(VIEW_MEASURED_AXES.right).toEqual({ horizontal: 'Y', vertical: 'Z' });
    expect(measuredAxisValue(envelope, VIEW_MEASURED_AXES.front.horizontal)).toBe(60);
    expect(measuredAxisValue(envelope, VIEW_MEASURED_AXES.front.vertical)).toBe(40);
    expect(measuredAxisValue(envelope, VIEW_MEASURED_AXES.top.vertical)).toBe(45);
  });
});
