// @vitest-environment happy-dom
// The 2D pencil HLR drawing: four views, measured callouts, pan/zoom/reset,
// and the honest loading/error states.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { HlrDrawing } from '../src/viewer/HlrDrawing';
import { DESIGN_VIEW_BASES, designViewCaption, viewLookDirection } from '../src/viewer/hlr';
import type { HlrDrawingProps } from '../src/viewer/HlrDrawing';
import type { DesignViewName, DrawingResponse, DrawingView } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  bracketDesignResult,
  bracketSection,
  notReportedDesignResult,
} from './designFixtures';

afterEach(cleanup);

function view(
  name: DesignViewName,
  visible: ReadonlyArray<ReadonlyArray<[number, number]>>,
  hidden: ReadonlyArray<ReadonlyArray<[number, number]>> = [],
  truncated = false,
): DrawingView {
  const asPolyline = (points: ReadonlyArray<[number, number]>) => ({
    points,
    closed: true,
    point_count: points.length,
  });
  return {
    name,
    visible: visible.map(asPolyline),
    hidden: hidden.map(asPolyline),
    truncated,
    point_count: 42,
    polyline_count: visible.length + hidden.length,
  };
}

const drawing: DrawingResponse = {
  design_id: BRACKET_DESIGN_ID,
  views: [
    view(
      'iso',
      [
        [[-30, 0], [30, 0], [30, 40], [-30, 40]],
        [[30, 0], [50, -14], [50, 26], [30, 40]],
      ],
      [[[0, 12], [20, 2], [20, 22], [0, 32]]],
    ),
    view(
      'front',
      [
        [[-30, 0], [30, 0], [30, 40], [-30, 40]],
        [[-5, 19], [-5, 29], [5, 29], [5, 19]],
      ],
      [[[30, 0], [30, 8]]],
    ),
    view('top', [[[-30, 0], [30, 0], [30, 45], [-30, 45]]]),
    view('right', [[[0, 0], [45, 0], [45, 40], [0, 40]]]),
  ],
  chord_tolerance_mm: 0.05,
  max_points: 20000,
  point_count: 168,
};

function props(overrides: Partial<HlrDrawingProps> = {}): HlrDrawingProps {
  return {
    design: bracketDesignResult,
    drawing,
    status: 'ready',
    error: null,
    showDimensions: true,
    viewResetToken: 0,
    ...overrides,
  };
}

describe('HLR view layout', () => {
  it('draws the iso view and the three third-angle small views', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    expect(container.querySelectorAll('.hlr-view')).toHaveLength(4);
    for (const name of ['iso', 'front', 'top', 'right']) {
      expect(container.querySelector(`.hlr-view[data-view="${name}"]`)).toBeTruthy();
    }
    expect(container.textContent).toContain('ISO · FROM (+X −Y +Z)');
    expect(container.textContent).toContain('TOP · LOOKING −Z');
    expect(container.textContent).toContain('FRONT · LOOKING +Y');
    expect(container.textContent).toContain('RIGHT · LOOKING −X');
    expect(container.textContent).toContain('THIRD ANGLE');
    expect(container.textContent).toContain('TOP ABOVE FRONT');
    expect(container.textContent).toContain('RIGHT OF FRONT');
  });

  it('uses separate visible and hidden line classes', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    expect(container.querySelectorAll('path.hlr-visible-line').length).toBeGreaterThan(0);
    const hidden = container.querySelectorAll('path.hlr-hidden-line');
    expect(hidden.length).toBeGreaterThan(0);
    expect(hidden[0]!.getAttribute('data-line')).toBe('hidden');
  });

  it('names a view the backend did not return instead of inventing it', () => {
    const partial: DrawingResponse = {
      ...drawing,
      views: drawing.views.filter((entry) => entry.name !== 'iso'),
    };
    const { container } = render(<HlrDrawing {...props({ drawing: partial })} />);
    expect(container.textContent).toContain('ISO VIEW NOT RETURNED');
  });

  it('reports a truncated polyline budget', () => {
    const truncated: DrawingResponse = {
      ...drawing,
      views: drawing.views.map((entry) =>
        entry.name === 'front' ? { ...entry, truncated: true } : entry,
      ),
    };
    const { container } = render(<HlrDrawing {...props({ drawing: truncated })} />);
    expect(container.textContent).toContain('POLYLINE BUDGET TRUNCATED');
    expect(container.textContent).toContain('TRUNCATED');
  });
});

describe('view labels follow the backend basis', () => {
  it('pins every caption to the direction the backend projects from', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    const caption = (name: string): string =>
      container.querySelector(`.hlr-view[data-view="${name}"] .hlr-view-caption`)?.textContent ?? '';

    // drawing.py: front viewer -Y, top +Z, right +X, iso (+X, -Y, +Z).
    expect(DESIGN_VIEW_BASES.front.viewer).toEqual([0, -1, 0]);
    expect(DESIGN_VIEW_BASES.top.viewer).toEqual([0, 0, 1]);
    expect(DESIGN_VIEW_BASES.right.viewer).toEqual([1, 0, 0]);
    expect(DESIGN_VIEW_BASES.iso.viewer[0]).toBeCloseTo(1 / Math.sqrt(3), 12);
    expect(DESIGN_VIEW_BASES.iso.viewer[1]).toBeCloseTo(-1 / Math.sqrt(3), 12);
    expect(DESIGN_VIEW_BASES.iso.viewer[2]).toBeCloseTo(1 / Math.sqrt(3), 12);

    expect(caption('front')).toBe('FRONT · LOOKING +Y');
    expect(caption('top')).toBe('TOP · LOOKING −Z');
    expect(caption('right')).toBe('RIGHT · LOOKING −X');
    expect(caption('iso')).toBe('ISO · FROM (+X −Y +Z)');
    // The label and the basis cannot drift apart: the rendered caption is
    // the helper's output, and the helper reads -viewer.
    for (const name of ['front', 'top', 'right'] as const) {
      expect(caption(name)).toContain(viewLookDirection(DESIGN_VIEW_BASES[name].viewer)!);
      expect(designViewCaption(name)).toBe(caption(name));
    }
    // The third-angle placement is unchanged: top above front, right of front.
    const top = container.querySelector<SVGGElement>('.hlr-view[data-view="top"]')!;
    const front = container.querySelector<SVGGElement>('.hlr-view[data-view="front"]')!;
    const right = container.querySelector<SVGGElement>('.hlr-view[data-view="right"]')!;
    const topRect = top.querySelector('rect')!;
    const frontRect = front.querySelector('rect')!;
    const rightRect = right.querySelector('rect')!;
    expect(Number(topRect.getAttribute('y'))).toBeLessThan(Number(frontRect.getAttribute('y')));
    expect(Number(rightRect.getAttribute('x'))).toBeGreaterThan(Number(frontRect.getAttribute('x')));
  });

  it('keeps the cut label in the heading, out of the SVG caption band', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    const cut = container.querySelector('.hlr-heading-cut')!;
    expect(cut.textContent).toBe('CUT A–A · XZ · Y = 4.00 mm');
    expect(cut.closest('svg')).toBeNull();
  });
});

describe('the shared A–A trace', () => {
  it('draws the measured cut in the iso and front views at the plane', () => {
    const { container } = render(
      <HlrDrawing
        {...props()}
        section={bracketSection}
        cut={{ plane: 'XZ', offsetMm: 4 }}
      />,
    );
    const front = container.querySelector('.hlr-view[data-view="front"] .hlr-section-trace');
    const iso = container.querySelector('.hlr-view[data-view="iso"] .hlr-section-trace');
    expect(front).toBeTruthy();
    expect(iso).toBeTruthy();
    expect(front?.getAttribute('data-section-plane')).toBe('XZ');
    // XZ is coplanar with front, not with top or right.
    expect(container.querySelector('.hlr-view[data-view="top"] .hlr-section-trace')).toBeNull();
    expect(container.querySelector('.hlr-view[data-view="right"] .hlr-section-trace')).toBeNull();
    // The A flags mark the ends of the front trace.
    expect(front?.querySelectorAll('.hlr-section-flag')).toHaveLength(2);
  });

  it('draws no trace when the measured section is for another plane', () => {
    const { container } = render(
      <HlrDrawing
        {...props()}
        section={bracketSection}
        cut={{ plane: 'XY', offsetMm: null }}
      />,
    );
    expect(container.querySelectorAll('.hlr-section-trace')).toHaveLength(0);
    expect(container.querySelector('.hlr-heading-cut')?.textContent).toBe(
      'CUT A–A · XY · Z = 20.00 mm · mid-plane',
    );
  });
});

describe('measured redline callouts', () => {
  it('labels the drawing as a view of the imported provider STEP', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    expect(container.textContent).toContain('view: OpenCascade import (view-only)');
    const label = container.querySelector<HTMLElement>('.hlr-heading-view')!;
    expect(label.getAttribute('data-view-provider')).toBe('freecad');
  });

  it('reads the provider-measured envelope from its bounds', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    expect(container.textContent).toContain('MEASURED ENVELOPE · X 60.00 · Y 45.00 · Z 40.00 mm');
    const labels = Array.from(container.querySelectorAll('.hlr-dim')).map(
      (node) => node.getAttribute('data-dim'),
    );
    expect(labels).toEqual(['X 60.00', 'Y 45.00', 'X 60.00', 'Z 40.00', 'Y 45.00', 'Z 40.00']);
    expect(container.querySelectorAll('.hlr-dim-line')).toHaveLength(6);
  });

  it('draws no callouts when the provider reported no envelope', () => {
    const { container } = render(
      <HlrDrawing {...props({ design: notReportedDesignResult })} />,
    );
    expect(container.querySelectorAll('.hlr-dim')).toHaveLength(0);
    expect(container.querySelector('.hlr-envelope-note')).toBeNull();
    // The HLR linework still renders: the imported view exists even though
    // the provider reported no measurements.
    expect(container.querySelectorAll('path.hlr-visible-line').length).toBeGreaterThan(0);
  });

  it('hides every callout when dimensions are off', () => {
    const { container } = render(<HlrDrawing {...props({ showDimensions: false })} />);
    expect(container.querySelectorAll('.hlr-dim')).toHaveLength(0);
    expect(container.querySelector('.hlr-envelope-note')).toBeNull();
    // The HLR linework stays.
    expect(container.querySelectorAll('path.hlr-visible-line').length).toBeGreaterThan(0);
  });
});

describe('pan, zoom, and reset', () => {
  it('zooms with the wheel and shows the zoom percentage', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    const svg = container.querySelector('svg.hlr-svg')!;
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('100%');
    fireEvent.wheel(svg, { deltaY: -120 });
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('120%');
    fireEvent.wheel(svg, { deltaY: 120 });
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('100%');
  });

  it('zooms with the controls and resets to the identity transform', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    const group = container.querySelector('svg.hlr-svg g')!;
    expect(group.getAttribute('transform')).toBe('translate(0 0) scale(1)');
    fireEvent.click(container.querySelector('[aria-label="Zoom in"]')!);
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('125%');
    fireEvent.click(container.querySelector('[aria-label="Reset drawing view"]')!);
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('100%');
    expect(group.getAttribute('transform')).toBe('translate(0 0) scale(1)');
  });

  it('pans with a pointer drag', () => {
    const { container } = render(<HlrDrawing {...props()} />);
    const svg = container.querySelector('svg.hlr-svg')!;
    const group = container.querySelector('svg.hlr-svg g')!;
    fireEvent.pointerDown(svg, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(svg, { clientX: 140, clientY: 120, pointerId: 1 });
    fireEvent.pointerUp(svg, { clientX: 140, clientY: 120, pointerId: 1 });
    expect(group.getAttribute('transform')).toBe('translate(40 20) scale(1)');
  });

  it('resets when the sheet view-reset token changes', () => {
    const { container, rerender } = render(<HlrDrawing {...props()} />);
    fireEvent.wheel(container.querySelector('svg.hlr-svg')!, { deltaY: -120 });
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('120%');
    rerender(<HlrDrawing {...props({ viewResetToken: 1 })} />);
    expect(container.querySelector('.hlr-zoom-value')?.textContent).toBe('100%');
  });
});

describe('load states', () => {
  it('shows a loading status before the drawing arrives', () => {
    const { container } = render(
      <HlrDrawing {...props({ drawing: null, status: 'loading' })} />,
    );
    expect(container.querySelector('.hlr-empty')?.getAttribute('role')).toBe('status');
    expect(container.textContent).toContain('Loading the hidden-line drawing');
    expect(container.querySelector('svg.hlr-svg')).toBeNull();
  });

  it('shows the real error and offers a retry', () => {
    const onRetry = vi.fn();
    const { container } = render(
      <HlrDrawing
        {...props({ drawing: null, status: 'error', error: 'timeout: /drawing', onRetry })}
      />,
    );
    expect(container.querySelector('.hlr-empty')?.getAttribute('role')).toBe('alert');
    expect(container.textContent).toContain('timeout: /drawing');
    fireEvent.click(container.querySelector('.hlr-retry')!);
    expect(onRetry).toHaveBeenCalledTimes(1);
  });
});

describe('label bounding boxes', () => {
  interface Box {
    text: string;
    left: number;
    right: number;
    top: number;
    bottom: number;
  }

  /**
   * A text element's box, read from the DOM attributes the component
   * actually renders (x, y, anchor, text). The CSS label size is 9px; the
   * 0.62 em width is a conservative monospace/UI estimate, so a box can only
   * be too large. `scale` maps SVG units to the sheet's CSS pixels.
   */
  function textBox(element: Element, scale: number): Box {
    const text = element.textContent ?? '';
    const x = Number(element.getAttribute('x') ?? 0);
    const y = Number(element.getAttribute('y') ?? 0);
    const anchor = element.getAttribute('text-anchor');
    const fontSize = 9;
    const width = text.length * fontSize * 0.62;
    const height = fontSize * 1.1;
    const left = anchor === 'middle' ? x - width / 2 : anchor === 'end' ? x - width : x;
    return {
      text,
      left: left * scale,
      right: (left + width) * scale,
      top: (y - height) * scale,
      bottom: y * scale,
    };
  }

  function overlaps(a: Box, b: Box): boolean {
    return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
  }

  for (const [label, width] of [['1440', 780], ['1280', 740]] as const) {
    it(`places every sheet label without overlap at ${label}`, () => {
      const { container } = render(
        <HlrDrawing
          {...props()}
          section={bracketSection}
          cut={{ plane: 'XZ', offsetMm: 4 }}
        />,
      );
      const scale = width / 940;
      const elements = Array.from(
        container.querySelectorAll(
          '.hlr-view-caption, .hlr-projection-title, .hlr-projection-line, .hlr-envelope-note, .hlr-legend, .hlr-view-truncated, .hlr-section-flag',
        ),
      );
      expect(elements.length).toBeGreaterThan(10);
      const boxes = elements.map((element) => textBox(element, scale));
      const collisions: string[] = [];
      for (let i = 0; i < boxes.length; i += 1) {
        for (let j = i + 1; j < boxes.length; j += 1) {
          if (overlaps(boxes[i]!, boxes[j]!)) {
            collisions.push(`${boxes[i]!.text} ↔ ${boxes[j]!.text}`);
          }
        }
      }
      expect(collisions).toEqual([]);
      // The shared cut note is an HTML heading row, not a text label over the
      // drawing, so it cannot collide with the top-view caption.
      const cut = container.querySelector('.hlr-heading-cut')!;
      expect(cut.closest('svg')).toBeNull();
      expect(cut.textContent).toContain('XZ · Y = 4.00 mm');
    });
  }
});
