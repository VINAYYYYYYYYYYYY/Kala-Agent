// @vitest-environment happy-dom
// The bottom ruler bar readout of a compiled design.
//
// With an active design the bar quotes the values the geometry kernel
// measured and names the source. Without one it keeps its primitive
// behaviour, so nothing regresses for the operator who never compiles a
// part.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/react';

import { BottomBar, describeDesignReadout } from '../src/components/bottombar/BottomBar';
import { dispatch, resetStore } from '../src/state/store';
import type { StudioLocalPreview } from '../src/types/studio';
import { bracketDesignResult, notReportedDesignResult } from './designFixtures';

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
});

function readout(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.bottom-bar-readout')!;
}

function source(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.bottom-bar-source')!;
}

function localPreview(): StudioLocalPreview {
  return {
    kind: 'box',
    parameters: { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 },
    positions: new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
    normals: new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
    indices: new Uint32Array([0, 1, 2]),
    bounds: { min: [0, 0, 0], max: [40, 20, 30] },
    source: 'local',
    generatedAt: 0,
    engine: 'local',
    warnings: [],
  };
}

describe('the bottom bar readout of a compiled design', () => {
  it('quotes the measured envelope, volume, and area', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    const text = readout(container).textContent ?? '';
    expect(text).toContain('60×45×40 mm');
    expect(text).toContain('35\u202F571 mm³');
    expect(text).toContain('11\u202F889 mm²');
  });

  it('names the provider that measured the numbers, with its version', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    expect(source(container).textContent).toBe('FreeCAD');
    expect(source(container).getAttribute('data-source')).toBe('FreeCAD');
    expect(source(container).getAttribute('title')).toContain('measured by FreeCAD 1.1.4');
    expect(source(container).getAttribute('data-source-kind')).toBe('provider');
    expect(source(container).getAttribute('data-provider')).toBe('freecad');
  });

  it('keeps the full measured precision in the item title tooltips', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    const titles = Array.from(
      container.querySelectorAll<HTMLElement>('.bottom-bar-readout-item'),
    ).map((item) => item.getAttribute('title') ?? '');
    expect(titles[0]).toContain('60 × 45 × 40 mm');
    expect(titles[1]).toContain('35571.416047 mm³');
    expect(titles[2]).toContain('11889.101731 mm²');
    expect(source(container).getAttribute('title')).toContain('measured by FreeCAD 1.1.4');
  });

  it('keeps the bar readout compact and the full values in the data', () => {
    const readoutData = describeDesignReadout(bracketDesignResult);
    expect(readoutData.envelope).toBe('60×45×40 mm');
    expect(readoutData.volume).toBe('35\u202F571 mm³');
    expect(readoutData.area).toBe('11\u202F889 mm²');
    expect(readoutData.source).toBe('FreeCAD');
    expect(readoutData.measuredBy).toBe('measured by FreeCAD 1.1.4');
    expect(readoutData.volumeTitle).toContain('35571.416047 mm³');
    expect(readoutData.areaTitle).toContain('11889.101731 mm²');
  });

  it('shows an em dash for values the provider did not report', () => {
    dispatch({ type: 'design/received', design: notReportedDesignResult });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    const text = readout(container).textContent ?? '';
    expect(text).toContain('ENV');
    expect(text).toContain('—');
    expect(text).not.toContain('0.00');
    expect(source(container).getAttribute('data-provider')).toBe('freecad');
  });

  it('keeps the fixed readout order and the honest scale', () => {
    dispatch({ type: 'design/received', design: bracketDesignResult });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    const text = readout(container).textContent ?? '';
    const order = ['ENV', 'VOL', 'AREA', 'SCALE'];
    const positions = order.map((label) => text.indexOf(label));
    expect(positions.every((position) => position >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(text).toContain('FIT');
    expect(text).not.toContain('1:1');
  });
});

describe('the bottom bar readout without a design', () => {
  it('keeps saying that there is no geometry', () => {
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    expect(source(container).getAttribute('data-source')).toBe('—');
    expect(source(container).getAttribute('data-source-kind')).toBe('none');
    expect(readout(container).textContent).toContain('—');
  });

  it('keeps the primitive estimate and its local source', () => {
    dispatch({ type: 'preview/local', preview: localPreview() });
    const { container } = render(<BottomBar onOpenOptions={() => {}} />);

    expect(source(container).getAttribute('data-source')).toBe('local est.');
    expect(source(container).getAttribute('data-source-kind')).toBe('local');
    expect(readout(container).textContent).toContain('cm³');
  });
});
