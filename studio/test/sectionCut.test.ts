// The shared Section A–A state: one plane and offset for the sheet, the HLR
// redline, the AuxView, and the clay viewport.

import { describe, expect, it } from 'vitest';
import {
  DEFAULT_DESIGN_SECTION_CUT,
  PRIMITIVE_SECTION_CUT,
  defaultSectionCutForPlane,
  sameSectionCut,
  sectionCutCorners,
  sectionCutLabel,
  sectionCutNote,
  sectionCutOffset,
} from '../src/state/sectionCut';
import { createInitialUiState, reducer } from '../src/state/store';
import { bracketDesignResult } from './designFixtures';

const bounds = bracketDesignResult.view.bounds;

describe('the shared cutting plane', () => {
  it('defaults the design to the ADR criterion-4 XZ cut at y = 4 mm', () => {
    expect(DEFAULT_DESIGN_SECTION_CUT).toEqual({ plane: 'XZ', offsetMm: 4 });
    expect(sectionCutLabel(DEFAULT_DESIGN_SECTION_CUT, bounds)).toBe('XZ · Y = 4.00 mm');
    expect(sectionCutNote(DEFAULT_DESIGN_SECTION_CUT, bounds)).toBe(
      'CUT A–A · XZ · Y = 4.00 mm',
    );
  });

  it('reads a default plane offset from the measured bounds, not a constant', () => {
    expect(sectionCutOffset({ plane: 'XY', offsetMm: null }, bounds)).toBe(20);
    expect(sectionCutOffset({ plane: 'YZ', offsetMm: null }, bounds)).toBe(0);
    expect(sectionCutLabel({ plane: 'XY', offsetMm: null }, bounds)).toBe(
      'XY · Z = 20.00 mm · mid-plane',
    );
    expect(sectionCutLabel({ plane: 'YZ', offsetMm: null }, bounds)).toBe(
      'YZ · X = 0.00 mm · mid-plane',
    );
  });

  it('keeps the primitive cut on the Z mid-plane', () => {
    expect(PRIMITIVE_SECTION_CUT).toEqual({ plane: 'XY', offsetMm: null });
    expect(sectionCutLabel(PRIMITIVE_SECTION_CUT, bounds)).toBe(
      'XY · Z = 20.00 mm · mid-plane',
    );
  });

  it('maps the documented plane default for every AuxView tab', () => {
    expect(defaultSectionCutForPlane('XZ')).toEqual({ plane: 'XZ', offsetMm: 4 });
    expect(defaultSectionCutForPlane('XY')).toEqual({ plane: 'XY', offsetMm: null });
    expect(defaultSectionCutForPlane('YZ')).toEqual({ plane: 'YZ', offsetMm: null });
  });

  it('puts every clay plane corner on the shared plane', () => {
    const corners = sectionCutCorners(bounds, DEFAULT_DESIGN_SECTION_CUT, 5);
    expect(corners).toHaveLength(4);
    for (const [, y] of corners) expect(y).toBe(4);
    expect(Math.min(...corners.map(([x]) => x))).toBe(-35);
    expect(Math.max(...corners.map(([, , z]) => z))).toBe(45);

    const mid = sectionCutCorners(bounds, { plane: 'XY', offsetMm: null }, 0);
    for (const [, , z] of mid) expect(z).toBe(20);
  });

  it('is one store state, not a per-component copy', () => {
    const initial = createInitialUiState();
    expect(initial.sectionCut).toEqual(DEFAULT_DESIGN_SECTION_CUT);
    const next = reducer(initial, {
      type: 'section/cut-plane',
      plane: 'XY',
      offsetMm: null,
    });
    expect(next.sectionCut).toEqual({ plane: 'XY', offsetMm: null });
    // A bad dispatch cannot put a plane on the sheet the backend never cut.
    expect(
      reducer(next, {
        type: 'section/cut-plane',
        plane: 'AB' as 'XY',
        offsetMm: null,
      }),
    ).toBe(next);
    expect(
      reducer(next, {
        type: 'section/cut-plane',
        plane: 'XZ',
        offsetMm: Number.NaN,
      }),
    ).toBe(next);
    expect(sameSectionCut(next.sectionCut, { plane: 'XY', offsetMm: null })).toBe(true);
  });
});
