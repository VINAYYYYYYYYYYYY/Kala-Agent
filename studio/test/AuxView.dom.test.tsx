// @vitest-environment happy-dom
// AuxView tab behaviour and the real section/ortho/tolerance content.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { useState } from 'react';
import { AuxView } from '../src/components/drafting/AuxView';
import type { AuxViewProps } from '../src/components/drafting/AuxView';
import type { AuxViewKind, PrimitiveKind } from '../src/types/studio';
import {
  dispatch,
  getState,
  resetStore,
  sectionCacheKey,
  drawingCacheKey,
} from '../src/state/store';
import type { DesignController } from '../src/state/designController';
import type { DrawingResponse, SectionResponse } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  bracketDesignResult,
  bracketDrawing,
  bracketSection,
} from './designFixtures';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

const box = { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 };
const cylinder = { radius: 20, height: 40, radial_segments: 48, height_segments: 1 };
const sphere = { radius: 25, rings: 24, sectors: 32 };
const bounds = { min: [-20, -10, -15] as [number, number, number], max: [20, 10, 15] as [number, number, number] };

function Harness(props: Partial<AuxViewProps>): ReturnType<typeof AuxView> {
  const [auxView, setAuxView] = useState<AuxViewKind>(props.auxView ?? 'section');
  const controller = props.designController;
  return (
    <AuxView
      auxView={auxView}
      onAuxViewChange={(view) => {
        props.onAuxViewChange?.(view);
        setAuxView(view);
      }}
      primitive={props.primitive ?? 'box'}
      parameters={props.parameters ?? box}
      bounds={props.bounds === undefined ? bounds : props.bounds}
      {...(controller !== undefined ? { designController: controller } : {})}
    />
  );
}

function tabs(container: HTMLElement): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll<HTMLButtonElement>('.aux-view-tab'));
}

describe('AuxView tabs', () => {
  it('renders the three drafted tabs and selects the active one', () => {
    const { container } = render(<Harness />);
    expect(tabs(container).map((tab) => tab.textContent)).toEqual([
      'Section A-A',
      'Ortho 3-View',
      'Tolerance',
    ]);
    expect(tabs(container)[0]!.getAttribute('aria-selected')).toBe('true');
    expect(tabs(container)[0]!.tabIndex).toBe(0);
    expect(tabs(container)[1]!.tabIndex).toBe(-1);
  });

  it('reports the chosen tab and updates the panel', () => {
    const onAuxViewChange = vi.fn();
    const { container } = render(<Harness onAuxViewChange={onAuxViewChange} />);
    fireEvent.click(tabs(container)[1]!);
    expect(onAuxViewChange).toHaveBeenCalledWith('ortho');
    expect(tabs(container)[1]!.getAttribute('aria-selected')).toBe('true');
    expect(container.textContent).toContain('FRONT');
    expect(container.textContent).toContain('THIRD-ANGLE');
  });

  it('supports arrow-key tab navigation', () => {
    const { container } = render(<Harness />);
    fireEvent.keyDown(tabs(container)[0]!, { key: 'ArrowRight' });
    expect(tabs(container)[1]!.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(tabs(container)[1]!, { key: 'End' });
    expect(tabs(container)[2]!.getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(tabs(container)[2]!, { key: 'Home' });
    expect(tabs(container)[0]!.getAttribute('aria-selected')).toBe('true');
  });
});

describe('Section A-A panel', () => {
  it('draws a hatched rectangle with real box dimensions', () => {
    const { container } = render(<Harness />);
    const profile = container.querySelector('rect.aux-profile');
    expect(profile).toBeTruthy();
    expect(profile!.getAttribute('fill')).toMatch(/^url\(#.+\)$/);
    expect(container.querySelector('pattern')).toBeTruthy();
    expect(container.querySelectorAll('.aux-dim-line')).toHaveLength(2);
    expect(container.textContent).toContain('W 40.00');
    expect(container.textContent).toContain('H 20.00');
    expect(container.textContent).toContain('Z = 0.00 mm · mid-plane');
    expect(container.textContent).toContain('SECTION A–A');
  });

  it('cuts and labels the section on the real mesh bounds', () => {
    const { container } = render(
      <Harness bounds={{ min: [-20, -10, 0], max: [20, 10, 30] }} />,
    );
    expect(container.textContent).toContain('Z = 15.00 mm · mid-plane');
  });

  it('states the 45° ANSI cut convention in the header, not only in the footer', () => {
    const { container } = render(<Harness />);
    const header = container.querySelector('.aux-view-header')!;
    expect(header.querySelector('.aux-view-cut-note')?.textContent).toBe('CUT: 45° ANSI');
    // The footer keeps the real plane and nothing else.
    const footer = container.querySelector('.aux-view-footer')!;
    expect(footer.textContent).toContain('PLANE: Z = 0.00 mm · mid-plane');
    expect(footer.textContent).not.toContain('CUT: 45° ANSI');
  });

  it('draws a circle for a cylinder cut normal to its axis', () => {
    const { container } = render(<Harness primitive="cylinder" parameters={cylinder} />);
    expect(container.querySelector('circle.aux-profile')).toBeTruthy();
    expect(container.querySelector('rect.aux-profile')).toBeNull();
    expect(container.textContent).toContain('⌀ 40.00');
    expect(container.textContent).toContain('normal to axis');
  });

  it('draws a great circle for a sphere', () => {
    const { container } = render(<Harness primitive="sphere" parameters={sphere} />);
    expect(container.querySelector('circle.aux-profile')).toBeTruthy();
    expect(container.textContent).toContain('⌀ 50.00');
    expect(container.textContent).toContain('great circle');
  });

  it('shows an honest unavailable state for an unsupported primitive', () => {
    const { container } = render(
      <Harness primitive={'torus' as PrimitiveKind} parameters={box} />,
    );
    expect(container.querySelector('.aux-profile')).toBeNull();
    expect(container.textContent).toContain('Section A–A unavailable');
    expect(container.textContent).toContain('torus');
    expect(container.textContent).not.toContain('ANSI 45° HATCH');
  });
});

describe('Ortho 3-View panel', () => {
  it('draws front/top/right projections from real bounds', () => {
    const { container } = render(<Harness auxView="ortho" />);
    const views = container.querySelectorAll('.aux-ortho-view');
    expect(views).toHaveLength(3);
    expect(container.querySelector('.aux-ortho-front')).toBeTruthy();
    expect(container.querySelector('.aux-ortho-top')).toBeTruthy();
    expect(container.querySelector('.aux-ortho-right')).toBeTruthy();
    expect(container.textContent).toContain('FRONT');
    expect(container.textContent).toContain('TOP');
    expect(container.textContent).toContain('RIGHT');
    expect(container.textContent).toContain('40.00 × 20.00 mm');
    expect(container.textContent).toContain('40.00 × 30.00 mm');
    expect(container.textContent).toContain('30.00 × 20.00 mm');
    expect(container.textContent).toContain('BOUNDS: LIVE');
  });

  it('shows a real empty state before bounds exist', () => {
    const { container } = render(<Harness auxView="ortho" bounds={null} />);
    expect(container.querySelectorAll('.aux-ortho-view')).toHaveLength(0);
    expect(container.textContent).toContain('No mesh bounds yet');
    expect(container.textContent).toContain('BOUNDS: —');
  });
});

describe('Tolerance panel', () => {
  it('states that no tolerance data exists instead of inventing limits', () => {
    const { container } = render(<Harness auxView="tolerance" />);
    expect(container.textContent).toContain('Tolerance — no model');
    expect(container.textContent).toContain('no tolerance stack');
    expect(container.textContent).toContain('VALUES: NONE');
    expect(container.textContent).toContain('Not provided');
    const values = Array.from(
      container.querySelectorAll<HTMLElement>('.aux-empty-facts dd'),
    ).map((node) => node.textContent);
    expect(values).toEqual(['—', '—', 'Not provided']);
    expect(container.textContent).not.toContain('0.05');
  });
});

// --- Design-backed panels ---------------------------------------------------

function controllerMock(): DesignController {
  return {
    loadDrawing: vi.fn(async () => undefined),
    loadSection: vi.fn(async () => undefined),
    dispose: vi.fn(),
    isDisposed: () => false,
  };
}

function seedDesign(available: boolean | null = true): void {
  dispatch({ type: 'server/capabilities', designAvailable: available });
  dispatch({ type: 'design/received', design: bracketDesignResult });
}

function seedSection(section: SectionResponse = bracketSection): void {
  dispatch({
    type: 'design/section-success',
    key: sectionCacheKey(BRACKET_DESIGN_ID, section.plane, 4),
    section,
  });
}

function fullDrawing(): DrawingResponse {
  const polyline = (points: ReadonlyArray<[number, number]>) => ({
    points,
    closed: false,
    point_count: points.length,
  });
  return {
    ...bracketDrawing,
    views: [
      { name: 'iso', visible: [polyline([[-30, 0], [30, 0], [30, 40]])], hidden: [], truncated: false, point_count: 3, polyline_count: 1 },
      { name: 'front', visible: [polyline([[-30, 0], [30, 0], [30, 40], [-30, 40]])], hidden: [], truncated: false, point_count: 4, polyline_count: 1 },
      { name: 'top', visible: [polyline([[-30, 0], [30, 0], [30, 45], [-30, 45]])], hidden: [], truncated: false, point_count: 4, polyline_count: 1 },
      { name: 'right', visible: [polyline([[0, 0], [45, 0], [45, 40], [0, 40]])], hidden: [], truncated: false, point_count: 4, polyline_count: 1 },
    ],
  };
}

function seedDrawing(): void {
  dispatch({
    type: 'design/drawing-success',
    key: drawingCacheKey(BRACKET_DESIGN_ID, []),
    drawing: fullDrawing(),
  });
}

describe('design Section A–A', () => {
  it('draws the backend loops as one evenodd hatched path with the exact area', () => {
    seedDesign();
    seedSection();
    const { container } = render(<Harness auxView="section" />);

    const area = container.querySelector('.aux-section-area')!;
    expect(area.getAttribute('fill-rule')).toBe('evenodd');
    expect(area.getAttribute('fill')).toMatch(/^url\(#.+\)$/);
    expect(container.querySelector('pattern')).toBeTruthy();
    expect(container.textContent).toContain('AREA 2,321.46 mm²');
    expect(container.textContent).toContain('1 OUTER · 1 INNER LOOP');
    expect(container.textContent).toContain('KERNEL AREA');
    expect(container.textContent).toContain('CUT A–A · XZ · Y = 4.00 mm');
    expect(container.textContent).toContain('AREA: 2,321.46 mm²');
    expect(container.querySelectorAll('.aux-section-plane-button')).toHaveLength(3);
  });

  it('writes the plane to the one shared Section A–A state', () => {
    seedDesign();
    seedSection();
    const { container } = render(<Harness auxView="section" />);

    // The default is the ADR criterion-4 cut.
    expect(getState().sectionCut).toEqual({ plane: 'XZ', offsetMm: 4 });
    expect(container.textContent).toContain('CUT A–A · XZ · Y = 4.00 mm');

    const xy = Array.from(
      container.querySelectorAll<HTMLButtonElement>('.aux-section-plane-button'),
    ).find((button) => button.textContent === 'XY')!;
    fireEvent.click(xy);

    // One state: the footer names the same plane the sheet will print, and
    // the offset is the backend mid-plane because XY has no explicit offset.
    expect(getState().sectionCut).toEqual({ plane: 'XY', offsetMm: null });
    expect(container.textContent).toContain('CUT A–A · XY · Z = 20.00 mm · mid-plane');
    expect(container.textContent).not.toContain('CUT A–A · XZ · Y = 4.00 mm');
  });

  it('marks an approximate area with ≈ instead of presenting it as exact', () => {
    seedDesign();
    seedSection({ ...bracketSection, area_exact: false, area_mm2: 2321.4 });
    const { container } = render(<Harness auxView="section" />);
    expect(container.textContent).toContain('≈ AREA 2,321.40 mm²');
    expect(container.textContent).toContain('POLYLINE AREA');
    expect(container.textContent).not.toContain('KERNEL AREA');
  });

  it('requests the selected plane from the shared controller', () => {
    seedDesign();
    seedSection();
    const controller = controllerMock();
    const { container } = render(
      <Harness auxView="section" designController={controller} />,
    );
    fireEvent.click(
      Array.from(container.querySelectorAll<HTMLButtonElement>('.aux-section-plane-button')).find(
        (button) => button.textContent === 'YZ',
      )!,
    );
    expect(controller.loadSection).toHaveBeenCalledWith(BRACKET_DESIGN_ID, 'YZ', undefined);
  });

  it('shows a loading state naming the plane', () => {
    seedDesign();
    dispatch({
      type: 'design/section-start',
      key: sectionCacheKey(BRACKET_DESIGN_ID, 'XZ', 4),
    });
    const { container } = render(<Harness auxView="section" />);
    expect(container.querySelector('.aux-loading')?.getAttribute('role')).toBe('status');
    expect(container.textContent).toContain('Loading section XZ');
    expect(container.querySelector('.aux-section-area')).toBeNull();
  });

  it('shows the real error and retries the same plane', () => {
    seedDesign();
    dispatch({
      type: 'design/section-error',
      key: sectionCacheKey(BRACKET_DESIGN_ID, 'XZ', 4),
      message: '503: design_unavailable',
    });
    const controller = controllerMock();
    const { container } = render(
      <Harness auxView="section" designController={controller} />,
    );
    expect(container.querySelector('.aux-error')?.getAttribute('role')).toBe('alert');
    expect(container.textContent).toContain('503: design_unavailable');
    fireEvent.click(container.querySelector('.aux-retry')!);
    expect(controller.loadSection).toHaveBeenCalledWith(BRACKET_DESIGN_ID, 'XZ', 4);
  });
});

describe('design Ortho 3-View', () => {
  it('draws the front/top/right HLR views in third-angle arrangement', () => {
    seedDesign();
    seedDrawing();
    const { container } = render(<Harness auxView="ortho" />);
    expect(container.querySelectorAll('.aux-design-ortho-view')).toHaveLength(3);
    expect(container.querySelector('.aux-design-ortho-front')).toBeTruthy();
    expect(container.querySelector('.aux-design-ortho-top')).toBeTruthy();
    expect(container.querySelector('.aux-design-ortho-right')).toBeTruthy();
    expect(container.querySelectorAll('path.aux-ortho-visible').length).toBeGreaterThan(0);
    expect(container.textContent).toContain('3RD ANGLE · OCP HLR');
    expect(container.textContent).toContain('SOURCE: OCP HLR · VIEW: IMPORT');
  });

  it('shows a loading state and an error state', () => {
    seedDesign();
    dispatch({
      type: 'design/drawing-start',
      key: drawingCacheKey(BRACKET_DESIGN_ID, []),
    });
    const { container } = render(<Harness auxView="ortho" />);
    expect(container.textContent).toContain('Loading the HLR ortho views');
    act(() => {
      dispatch({
        type: 'design/drawing-error',
        key: drawingCacheKey(BRACKET_DESIGN_ID, []),
        message: 'timeout: /drawing',
      });
    });
    expect(container.querySelector('.aux-error')?.getAttribute('role')).toBe('alert');
    expect(container.textContent).toContain('timeout: /drawing');
  });
});

describe('design Tolerance', () => {
  it('lists the declared tolerances and never claims analysis', () => {
    seedDesign();
    const { container } = render(<Harness auxView="tolerance" />);
    expect(container.textContent).toContain('declared — not analysed');
    const items = Array.from(
      container.querySelectorAll<HTMLElement>('.aux-tolerance-list li'),
    ).map((node) => node.textContent);
    expect(items).toEqual(bracketDesignResult.tolerances);
    expect(container.textContent).toContain('Material: aluminium (declared)');
    expect(container.textContent).toContain('computes no fits');
    expect(container.textContent).toContain('SOURCE: DECLARED');
    expect(container.textContent).toContain('measured by FreeCAD 1.1.4');
    expect(container.textContent).not.toContain('HASH-CHAIN');
  });
});

describe('backend without design support', () => {
  it('shows the honest note with design_available: false', () => {
    dispatch({ type: 'server/capabilities', designAvailable: false });
    const { container } = render(<Harness auxView="tolerance" />);
    expect(container.querySelector('.aux-backend-note')?.getAttribute('role')).toBe('status');
    expect(container.textContent).toContain(
      'Design turns need the Python geometry backend; this backend reports design_available: false',
    );
    // The primitive tolerance feature still works.
    expect(container.textContent).toContain('Tolerance — no model');
    expect(container.textContent).toContain('VALUES: NONE');
  });

  it('does not claim the backend reported false when it did not report at all', () => {
    dispatch({ type: 'server/capabilities', designAvailable: null });
    const { container } = render(<Harness auxView="tolerance" />);
    expect(container.textContent).toContain('this backend did not report design_available');
    expect(container.textContent).not.toContain('reports design_available: false');
  });

  it('keeps the primitive section and ortho views available', () => {
    dispatch({ type: 'server/capabilities', designAvailable: false });
    const { container } = render(<Harness auxView="section" />);
    expect(container.querySelector('rect.aux-profile')).toBeTruthy();
    expect(container.textContent).toContain('SECTION A–A');
    expect(container.querySelector('.aux-backend-note')).toBeNull();
  });
});
