// @vitest-environment happy-dom
// The Details panel: tabs, close, and the three sections.
//
// Properties carries the active-geometry summary and, while no design is
// active, the primitive inspector. Views mounts the AuxView; Activity mounts
// the journal. The panel is bound to the store's `detailsTab`, so the top bar
// and the panel can never disagree.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { DetailsPanel } from '../src/components/shell/DetailsPanel';
import type { DetailsPanelProps } from '../src/components/shell/DetailsPanel';
import { dispatch, getState, resetStore } from '../src/state/store';
import { bracketDesignResult } from './designFixtures';

afterEach(cleanup);

beforeEach(() => {
  resetStore();
});

function props(overrides: Partial<DetailsPanelProps> = {}): DetailsPanelProps {
  return {
    auxView: 'section',
    primitive: 'box',
    parameters: { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 },
    bounds: { min: [-20, -10, -15], max: [20, 10, 15] },
    ...overrides,
  };
}

function tabs(container: HTMLElement): HTMLButtonElement[] {
  return Array.from(container.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
}

function geometryRow(container: HTMLElement, label: string): string {
  const row = Array.from(container.querySelectorAll('.details-geometry-row')).find(
    (candidate) => candidate.querySelector('dt')?.textContent === label,
  );
  return row?.querySelector('dd')?.textContent ?? '';
}

describe('details panel chrome', () => {
  it('renders the three tabs bound to the store tab', () => {
    const { container } = render(<DetailsPanel {...props()} />);
    expect(container.querySelector('aside')!.getAttribute('role')).toBe('complementary');
    expect(container.querySelector('aside')!.getAttribute('aria-label')).toBe('Details');
    expect(tabs(container).map((tab) => tab.textContent)).toEqual([
      'Properties',
      'Views',
      'Activity',
    ]);
    expect(tabs(container)[0]!.getAttribute('aria-selected')).toBe('true');
    expect(tabs(container)[1]!.getAttribute('tabindex')).toBe('-1');
    expect(container.querySelector('[role="tabpanel"]')!.getAttribute('aria-labelledby')).toBe(
      'details-tab-properties',
    );
  });

  it('changes the visible section with the tab strip', () => {
    const { container } = render(<DetailsPanel {...props()} />);
    fireEvent.click(tabs(container)[1]!);
    expect(getState().detailsTab).toBe('views');
    expect(tabs(container)[1]!.getAttribute('aria-selected')).toBe('true');
    expect(container.querySelector('.aux-view')).toBeTruthy();
    expect(container.querySelector('.details-geometry')).toBeNull();

    fireEvent.click(tabs(container)[2]!);
    expect(getState().detailsTab).toBe('activity');
    expect(container.querySelector('.journal-panel')).toBeTruthy();
    expect(container.querySelector('.aux-view')).toBeNull();
    // The status bar already reports the origin and backend version, so the
    // journal footer is not repeated inside Details.
    expect(container.querySelector('.journal-panel-footer')).toBeNull();
  });

  it('closes the panel from the close button', () => {
    dispatch({ type: 'chrome/details-open', tab: 'properties' });
    const { container } = render(<DetailsPanel {...props()} />);
    fireEvent.click(container.querySelector('button[aria-label="Close details"]')!);
    expect(getState().detailsOpen).toBe(false);
  });

  it('closes on Escape while the focus is inside the panel', () => {
    dispatch({ type: 'chrome/details-open', tab: 'properties' });
    const { container } = render(<DetailsPanel {...props()} />);
    fireEvent.keyDown(tabs(container)[0]!, { key: 'Escape' });
    expect(getState().detailsOpen).toBe(false);
  });

  it('keeps the Views tab wired to the shared AuxView state', () => {
    dispatch({ type: 'chrome/details-open', tab: 'views' });
    const { container } = render(<DetailsPanel {...props()} />);
    const ortho = Array.from(container.querySelectorAll<HTMLButtonElement>('.aux-view-tab')).find(
      (tab) => tab.textContent === 'Ortho 3-View',
    )!;
    fireEvent.click(ortho);
    expect(getState().auxView).toBe('ortho');
  });
});

describe('details properties', () => {
  it('shows the nominal geometry and the primitive inspector without a design', () => {
    const { container } = render(<DetailsPanel {...props()} />);
    expect(container.querySelector('.details-geometry')).toBeTruthy();
    expect(geometryRow(container, 'DIMS')).toContain('40.00 mm');
    expect(geometryRow(container, 'ENVELOPE')).toBe('40.00 × 20.00 × 30.00 mm');
    expect(geometryRow(container, 'VOLUME')).toContain('mm³');
    expect(geometryRow(container, 'SURFACE AREA')).toContain('5200.00 mm² (52.000 cm²)');
    expect(geometryRow(container, 'SOURCE')).toBe('nominal estimate (closed-form)');
    expect(container.querySelector('.inspector')).toBeTruthy();
    // The summary is the one home of the numbers: the inspector's estimate
    // card and bounds block are hidden here.
    expect(container.querySelector('.inspector-estimates')).toBeNull();
    expect(container.querySelector('.inspector-bounds')).toBeNull();
    // The nominal-geometry sentence appears once, under the summary.
    const note = container.querySelector('.details-geometry-note');
    expect(note?.textContent).toContain('Closed-form nominal geometry');
    expect(container.querySelectorAll('.inspector-estimates-note')).toHaveLength(0);
  });

  it('quotes the provider measurements and drops the primitive inspector with a design', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<DetailsPanel {...props()} />);
    expect(geometryRow(container, 'DIMS')).toBe('L-bracket 60 x 45 x 40');
    expect(geometryRow(container, 'ENVELOPE')).toBe('60.00 × 45.00 × 40.00 mm');
    expect(geometryRow(container, 'SURFACE AREA')).toContain('11889.10 mm² (11.889 cm²)');
    expect(geometryRow(container, 'SOURCE')).toBe('measured by FreeCAD 1.1.4');
    expect(container.querySelector('.inspector')).toBeNull();
    // The nominal sentence belongs to the primitive only.
    expect(container.querySelector('.details-geometry-note')).toBeNull();
  });
});
