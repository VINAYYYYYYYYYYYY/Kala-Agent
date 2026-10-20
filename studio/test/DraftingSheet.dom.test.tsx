// @vitest-environment happy-dom
// DraftingSheet render contract: drafted ticks, real title block, real tool
// palette actions, and the single viewport mount.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { DraftingSheet, DESIGN_STARTER_PROMPT } from '../src/components/drafting/DraftingSheet';
import type { DraftingSheetProps, TitleBlockData } from '../src/components/drafting/DraftingSheet';
import { buildLocal, defaultBox } from '../src/viewer/localPrimitives';
import {
  dispatch,
  drawingCacheKey,
  getState,
  resetStore,
} from '../src/state/store';
import type { DesignResult, DrawingResponse, StudioLocalPreview } from '../src/types/studio';
import { BRACKET_DESIGN_ID, bracketDesignResult, bracketDrawing, notReportedDesignResult } from './designFixtures';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

const titleBlock: TitleBlockData = {
  sheet: 'SHEET 01',
  partId: 'STUDIO-BOX',
  title: 'box · 40.00 × 20.00 × 30.00 mm',
  scale: 'FIT',
  units: 'mm',
  projection: '3RD ANGLE',
  tolerance: '—',
  revision: 'DRAFT',
  author: 'Kala Studio',
};

function props(overrides: Partial<DraftingSheetProps> = {}): DraftingSheetProps {
  return {
    mesh: null,
    meshSource: 'none',
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default',
    background: 'paper',
    renderMode: 'pencil2d',
    showSection: true,
    titleBlock,
    units: 'mm',
    viewResetToken: 0,
    ...overrides,
  };
}

function titleValues(container: HTMLElement): Record<string, string> {
  const entries = Array.from(container.querySelectorAll<HTMLElement>('.drafting-title-cell')).map(
    (cell) => [
      cell.querySelector('.drafting-title-label')?.textContent ?? '',
      cell.querySelector('.drafting-title-value')?.textContent ?? '',
    ],
  );
  return Object.fromEntries(entries);
}

describe('drafted sheet frame', () => {
  it('renders A–F and 1–8 coordinate ticks with real boundary marks', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    expect(
      Array.from(container.querySelectorAll('.drafting-coord-top')).map((node) => node.textContent),
    ).toEqual(['1', '2', '3', '4', '5', '6', '7', '8']);
    expect(
      Array.from(container.querySelectorAll('.drafting-coord-bottom')).map((node) => node.textContent),
    ).toEqual(['1', '2', '3', '4', '5', '6', '7', '8']);
    expect(
      Array.from(container.querySelectorAll('.drafting-coord-left')).map((node) => node.textContent),
    ).toEqual(['A', 'B', 'C', 'D', 'E', 'F']);
    expect(
      Array.from(container.querySelectorAll('.drafting-coord-right')).map((node) => node.textContent),
    ).toEqual(['A', 'B', 'C', 'D', 'E', 'F']);
    // One more tick than zones on every edge.
    expect(container.querySelectorAll('.drafting-tick-top')).toHaveLength(9);
    expect(container.querySelectorAll('.drafting-tick-left')).toHaveLength(7);
    expect(container.querySelectorAll('.drafting-pin')).toHaveLength(4);
  });

  it('mounts exactly one viewport inside the sheet stage', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    const canvases = container.querySelectorAll('.viewer-canvas');
    expect(canvases).toHaveLength(1);
    expect(canvases[0]!.closest('.drafting-sheet-stage')).toBeTruthy();
    expect(container.querySelector('.drafting-sheet')!.getAttribute('data-render-mode')).toBe(
      'pencil2d',
    );
    expect(container.querySelector('.drafting-sheet')!.getAttribute('data-section')).toBe('on');
  });

  it('shows the real title block values and explicit unknown fallbacks', () => {
    const { container } = render(
      <DraftingSheet
        {...props()}
        titleBlock={{ ...titleBlock, revision: '', author: '', tolerance: '' }}
      />,
    );
    const values = titleValues(container);
    expect(values['SUBJECT // PART ID']).toBe('STUDIO-BOX');
    expect(values.DRAWING).toContain('40.00 × 20.00 × 30.00 mm');
    expect(values.SCALE).toBe('FIT');
    expect(values.UNITS).toBe('METRIC (MM)');
    expect(values.PROJECTION).toBe('3RD ANGLE');
    expect(values.SHEET).toBe('SHEET 01');
    expect(values.TOLERANCE).toBe('—');
    expect(values.REV).toBe('—');
    expect(values.AUTHOR).toBe('—');
  });
});

describe('mode pill and tool palette', () => {
  it('binds the 2D/3D pill to real render-mode state', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    const pencil = container.querySelector<HTMLButtonElement>(
      '.drafting-mode-pill button[data-render-mode="pencil2d"]',
    )!;
    const clay = container.querySelector<HTMLButtonElement>(
      '.drafting-mode-pill button[data-render-mode="clay3d"]',
    )!;
    expect(pencil.textContent).toContain('2D Pencil Vector');
    expect(clay.textContent).toContain('3D Clay');
    expect(pencil.getAttribute('aria-pressed')).toBe('true');
    expect(clay.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(clay);
    expect(getState().renderMode).toBe('clay3d');
    fireEvent.click(pencil);
    expect(getState().renderMode).toBe('pencil2d');
  });

  it('offers only tools backed by real store actions', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    const tools = Array.from(container.querySelectorAll<HTMLButtonElement>('.drafting-tool'));
    expect(tools).toHaveLength(5);
    for (const tool of tools) {
      expect(tool.getAttribute('aria-label')).toBeTruthy();
    }

    fireEvent.click(tools.find((tool) => tool.getAttribute('aria-label') === 'Toggle grid')!);
    expect(getState().viewport.showGrid).toBe(false);

    fireEvent.click(tools.find((tool) => tool.getAttribute('aria-label') === 'Toggle dimensions')!);
    expect(getState().viewport.showDimensions).toBe(false);

    fireEvent.click(tools.find((tool) => tool.getAttribute('aria-label') === 'Toggle Section A-A')!);
    expect(getState().showSection).toBe(false);

    fireEvent.click(tools.find((tool) => tool.getAttribute('aria-label') === 'Reset view')!);
    expect(getState().viewResetToken).toBe(1);
  });

  it('disables the measure tool while there is no mesh', () => {
    const { container } = render(<DraftingSheet {...props()} />);
    const measure = container.querySelector<HTMLButtonElement>(
      '.drafting-tool[aria-label="Measure distance"]',
    )!;
    expect(measure.disabled).toBe(true);
  });
});

describe('section flags and mesh wiring', () => {
  it('shows redline A–A flags and the real cut plane when section is on', () => {
    const mesh = buildLocal('box', defaultBox());
    const { container } = render(
      <DraftingSheet {...props({ mesh, meshSource: 'local', showSection: true })} />,
    );
    expect(container.querySelectorAll('.drafting-section-flag')).toHaveLength(2);
    expect(container.textContent).toContain('CUT A–A · XY · Z = 0.00 mm · mid-plane');
    expect(container.textContent).toContain('Local preview');
  });

  it('hides the flags and cut note without a section or mesh', () => {
    const { container } = render(
      <DraftingSheet {...props({ showSection: false, mesh: null })} />,
    );
    expect(container.querySelector('.drafting-section-mark')).toBeNull();
    expect(container.querySelector('.drafting-cut-note')).toBeNull();
    expect(container.textContent).toContain('No mesh');
  });

  it('names the real bounds mid-plane when the mesh is not centred', () => {
    const mesh: StudioLocalPreview = {
      ...buildLocal('box', defaultBox()),
      bounds: { min: [-20, -10, 0], max: [20, 10, 30] },
    };
    const { container } = render(
      <DraftingSheet {...props({ mesh, meshSource: 'server', showSection: true })} />,
    );
    expect(container.querySelector('.drafting-cut-note')?.textContent).toBe(
      'CUT A–A · XY · Z = 15.00 mm · mid-plane',
    );
  });
});

// --- Design turns -----------------------------------------------------------

const SECOND_DESIGN_ID = 'b'.repeat(64);

function secondDesign(): DesignResult {
  return {
    ...bracketDesignResult,
    design_id: SECOND_DESIGN_ID,
    label: 'L-bracket 40 x 40 x 30',
    artifacts: { ...bracketDesignResult.artifacts, script_sha256: SECOND_DESIGN_ID },
  };
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

function seedDesign(design: DesignResult, withDrawing = false, available = true): void {
  dispatch({ type: 'server/capabilities', designAvailable: available });
  dispatch({ type: 'design/received', design });
  if (withDrawing) {
    dispatch({
      type: 'design/drawing-success',
      key: drawingCacheKey(design.design_id, []),
      drawing: { ...fullDrawing(), design_id: design.design_id },
    });
  }
}

describe('design drawing on the sheet', () => {
  it('replaces the three.js viewport with the static HLR SVG', () => {
    seedDesign(bracketDesignResult, true);
    const { container } = render(<DraftingSheet {...props({ renderMode: 'pencil2d' })} />);

    // Exactly one drawing surface: the WebGL canvas is unmounted.
    expect(container.querySelectorAll('.viewer-canvas')).toHaveLength(0);
    expect(container.querySelectorAll('.hlr-drawing')).toHaveLength(1);
    expect(container.querySelectorAll('.hlr-visible-line').length).toBeGreaterThan(0);
    expect(container.textContent).toContain('ISO · FROM (+X −Y +Z)');
    expect(container.querySelector('.drafting-sheet')!.getAttribute('data-render-mode')).toBe(
      'pencil2d',
    );
  });

  it('draws the imported provider view on the interactive clay viewport', () => {
    seedDesign(bracketDesignResult);
    const { container } = render(<DraftingSheet {...props({ renderMode: 'clay3d' })} />);

    expect(container.querySelectorAll('.hlr-drawing')).toHaveLength(0);
    const canvas = container.querySelector('.viewer-canvas')!;
    expect(canvas).toBeTruthy();
    expect(canvas.getAttribute('data-design-mesh')).toBe('true');
    expect(container.textContent).toContain('view: OpenCascade import (view-only)');
  });

  it('names the opt-in OpenCascade build as the view source when that provider built it', () => {
    seedDesign({ ...bracketDesignResult, provider: 'opencascade', measured_by: 'OpenCascade (opt-in)' });
    const { container } = render(<DraftingSheet {...props({ renderMode: 'clay3d' })} />);

    expect(container.textContent).toContain('view: OpenCascade (opt-in) build');
    expect(container.textContent).not.toContain('view: OpenCascade import (view-only)');
  });

  it('keeps the three.js pencil rendering without a design', () => {
    const { container } = render(<DraftingSheet {...props({ renderMode: 'pencil2d' })} />);
    expect(container.querySelectorAll('.viewer-canvas')).toHaveLength(1);
    expect(container.querySelectorAll('.hlr-drawing')).toHaveLength(0);
  });

  it('fills the title block from the design and labels declared metadata', () => {
    seedDesign(bracketDesignResult);
    const { container } = render(<DraftingSheet {...props()} />);
    const values = titleValues(container);
    expect(values['SUBJECT // DESIGN ID']).toBe(BRACKET_DESIGN_ID.slice(0, 8));
    expect(values.DRAWING).toBe('L-bracket 60 x 45 x 40');
    expect(values['MEASURED BY']).toBe('measured by FreeCAD 1.1.4');
    expect(values['MATERIAL (DECLARED)']).toBe('aluminium');
    expect(values['TOLERANCES (DECLARED)']).toContain('ISO 2768-m (declared)');
    expect(values.SCALE).toBe('FIT');
    expect(values.UNITS).toBe('METRIC (MM)');
    expect(values.PROJECTION).toBe('3RD ANGLE');
    expect(values.REV).toBe('—');
    // No invented checker or date; no primitive sheet cell is carried over.
    expect(values.AUTHOR).toBeUndefined();
    expect(values.SHEET).toBeUndefined();
    expect(container.querySelector('.drafting-title-block')!.getAttribute('data-title-block')).toBe(
      'design',
    );
  });

  it('still labels the measuring provider when it reported no measurements', () => {
    seedDesign(notReportedDesignResult);
    const { container } = render(<DraftingSheet {...props()} />);
    const values = titleValues(container);
    expect(values['MEASURED BY']).toBe('measured by FreeCAD 1.1.4');
    // The cut note uses the imported view bounds, which always exist.
    expect(container.querySelector('.hlr-heading-cut')?.textContent).toContain('Y = 4.00 mm');
  });

  it('takes the sheet label from the one shared cut plane', () => {
    seedDesign(bracketDesignResult, true);
    dispatch({ type: 'section/cut-plane', plane: 'XY', offsetMm: null });
    const { container } = render(<DraftingSheet {...props({ showSection: true })} />);
    expect(container.querySelector('.hlr-heading-cut')?.textContent).toBe(
      'CUT A–A · XY · Z = 20.00 mm · mid-plane',
    );
    expect(container.querySelector('.drafting-cut-note')).toBeNull();

    act(() => {
      dispatch({ type: 'section/cut-plane', plane: 'XZ', offsetMm: 4 });
    });
    expect(container.querySelector('.hlr-heading-cut')?.textContent).toBe(
      'CUT A–A · XZ · Y = 4.00 mm',
    );
  });

  it('shows the cut note at the measured bounds mid-plane', () => {
    seedDesign(bracketDesignResult, true);
    const { container } = render(<DraftingSheet {...props({ showSection: true })} />);
    expect(container.querySelector('.hlr-heading-cut')?.textContent).toBe(
      'CUT A–A · XZ · Y = 4.00 mm',
    );
  });
});

describe('sheet tabs are the design LRU', () => {
  it('renders one tab per design with its label and short id, active bound to the store', () => {
    seedDesign(bracketDesignResult, true);
    seedDesign(secondDesign(), true);
    const { container } = render(<DraftingSheet {...props()} />);
    const tabs = Array.from(container.querySelectorAll('.drafting-design-tab[data-design-id]'));
    expect(tabs).toHaveLength(2);
    expect(tabs[0]!.textContent).toContain('L-bracket 60 x 45 x 40');
    expect(tabs[0]!.textContent).toContain(BRACKET_DESIGN_ID.slice(0, 8));
    expect(tabs[1]!.textContent).toContain(SECOND_DESIGN_ID.slice(0, 8));
    expect(tabs[1]!.classList.contains('is-active')).toBe(true);
    // No fake "SHEET 01" tab when the backend supports designs.
    expect(container.querySelectorAll('.drafting-sheet-tab')).toHaveLength(3);

    fireEvent.click(tabs[0]!.querySelector('button')!);
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
  });

  it('closes a tab without dropping the design from the store LRU', () => {
    seedDesign(bracketDesignResult, true);
    seedDesign(secondDesign(), true);
    const { container } = render(<DraftingSheet {...props()} />);
    const activeTab = container.querySelector(
      `.drafting-design-tab[data-design-id="${SECOND_DESIGN_ID}"]`,
    )!;
    fireEvent.click(activeTab.querySelector('.drafting-design-tab-close')!);

    // The active tab was closed, so the sheet moves to the remaining design.
    expect(getState().designs.activeDesignId).toBe(BRACKET_DESIGN_ID);
    // The store keeps both LRU entries even though one tab is hidden.
    expect(getState().designs.order).toHaveLength(2);
    expect(getState().designs.designs.has(SECOND_DESIGN_ID)).toBe(true);
    expect(
      container.querySelector(`.drafting-design-tab[data-design-id="${SECOND_DESIGN_ID}"]`),
    ).toBeNull();
  });

  it('focuses the chat composer with a starter from the "+" tab and creates no sheet', () => {
    seedDesign(bracketDesignResult, true);
    const textarea = document.createElement('textarea');
    textarea.className = 'chat-panel-prompt';
    document.body.appendChild(textarea);
    const { container } = render(<DraftingSheet {...props()} />);
    const before = getState().designs.order.length;

    fireEvent.click(container.querySelector('.drafting-design-tab-add')!);

    expect(document.activeElement).toBe(textarea);
    expect(textarea.value).toBe(DESIGN_STARTER_PROMPT);
    expect(getState().designs.order).toHaveLength(before);
    document.body.removeChild(textarea);
  });

  it('hides the design tab strip on a backend without design support', () => {
    dispatch({ type: 'server/capabilities', designAvailable: false });
    const { container } = render(<DraftingSheet {...props()} />);
    expect(container.querySelectorAll('.drafting-design-tab')).toHaveLength(0);
    expect(container.querySelectorAll('.drafting-sheet-tab')).toHaveLength(1);
    expect(container.querySelector('.drafting-sheet-tab')?.textContent).toBe('SHEET 01');
    // Every primitive and preview feature still works.
    expect(container.querySelectorAll('.viewer-canvas')).toHaveLength(1);
    expect(container.querySelectorAll('.hlr-drawing')).toHaveLength(0);
  });
});
