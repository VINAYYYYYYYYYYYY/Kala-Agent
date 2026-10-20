// @vitest-environment happy-dom
// Export of the active design.
//
// ADR-0009 §7: exporting the active design writes a STEP or STL artefact and
// the panel shows a receipt with the filename, the byte count, and the
// sha256 the backend reported — with a copy control for the digest. A
// backend without the design routes never offers the export and says why.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  exportDesign: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return { ...actual, exportDesign: mocks.exportDesign };
});

import { StudioApiError } from '../src/api/client';
import { ExportPanel } from '../src/components/export/ExportPanel';
import { dispatch, getState, resetStore } from '../src/state/store';
import type { DesignExportArtifact } from '../src/types/studio';
import { bracketDesignResult, BRACKET_DESIGN_ID } from './designFixtures';

const connection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop' as const,
};

const DIGEST =
  '3f9a1c2b8e7d6054a3b2c1d0e9f8a7b6c5d4e3f2a1b0c9d8e7f6a5b4c3d2e1f00';

let writeText: ReturnType<typeof vi.fn>;

function artifact(overrides: Partial<DesignExportArtifact> = {}): DesignExportArtifact {
  return {
    blob: new Blob([new Uint8Array(2048)]),
    filename: 'kala-studio-88a7ec13.step',
    bytes: 2048,
    format: 'step',
    integrationId: null,
    designId: BRACKET_DESIGN_ID,
    digest: DIGEST,
    artifactKind: 'step',
    ...overrides,
  };
}

beforeEach(() => {
  resetStore();
  dispatch({ type: 'connection/ready', connection });
  writeText = vi.fn(async () => undefined);
  Object.defineProperty(globalThis.navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock');
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined);
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  resetStore();
  vi.restoreAllMocks();
  mocks.exportDesign.mockReset();
});

function seedDesign(): void {
  dispatch({ type: 'design/received', design: bracketDesignResult });
}

function designButton(container: HTMLElement): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>('.export-panel-design-button')!;
}

describe('ExportPanel — active design', () => {
  it('offers the design export and names the design it exports', () => {
    seedDesign();
    const view = render(<ExportPanel />);
    const section = view.container.querySelector('.export-panel-design')!;
    expect(section.getAttribute('aria-label')).toBe('Active design export');
    expect(section.textContent).toContain('L-bracket 60 x 45 x 40');
    expect(section.querySelector('.export-panel-design-id')?.textContent).toBe(
      'a9d04f8f',
    );
    expect(section.querySelector('.export-panel-design-provider')?.textContent).toBe(
      'measured by FreeCAD 1.1.4',
    );
    expect(section.querySelector('[data-artifact="step"]')?.textContent).toContain('24a5dca7');
    expect(section.querySelector('[data-artifact="native"]')?.textContent).toContain('FCStd');
    expect(section.querySelector('[data-artifact="native"]')?.textContent).toContain('e0f9fb92');
    expect(designButton(view.container).textContent).toContain('Export design STEP');
  });

  it('exports the active design and shows the receipt with the sha256', async () => {
    mocks.exportDesign.mockResolvedValue(artifact());
    seedDesign();
    const view = render(<ExportPanel />);

    fireEvent.click(designButton(view.container));

    await waitFor(() =>
      expect(mocks.exportDesign).toHaveBeenCalledWith(
        connection,
        BRACKET_DESIGN_ID,
        'step',
      ),
    );
    const receipt = view.container.querySelector('.export-panel-receipt')!;
    expect(receipt.textContent).toContain('kala-studio-88a7ec13.step');
    expect(receipt.textContent).toContain('2,048');
    expect(receipt.textContent).toContain(DIGEST);
    // The receipt is also a journal row: the export is a real event.
    expect(
      getState().journal.some((entry) => entry.message.includes('Saved kala-studio-88a7ec13.step')),
    ).toBe(true);
  });

  it('copies the sha256 of the receipt', async () => {
    mocks.exportDesign.mockResolvedValue(artifact());
    seedDesign();
    const view = render(<ExportPanel />);
    fireEvent.click(designButton(view.container));

    const copy = await waitFor(() => {
      const node = view.container.querySelector<HTMLButtonElement>(
        'button[aria-label="Copy sha256"]',
      );
      expect(node).toBeTruthy();
      return node!;
    });
    fireEvent.click(copy);
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(DIGEST));
  });

  it('exports in the format the operator picked', async () => {
    mocks.exportDesign.mockResolvedValue(artifact({ format: 'stl', artifactKind: 'stl' }));
    seedDesign();
    const view = render(<ExportPanel />);

    const designSelect = view.container.querySelector<HTMLSelectElement>(
      '.export-panel-design select',
    )!;
    fireEvent.change(designSelect, { target: { value: 'stl' } });
    await waitFor(() =>
      expect(designButton(view.container).textContent).toContain('Export design STL'),
    );
    fireEvent.click(designButton(view.container));
    await waitFor(() =>
      expect(mocks.exportDesign).toHaveBeenCalledWith(
        connection,
        BRACKET_DESIGN_ID,
        'stl',
      ),
    );
  });

  it('shows no digest when the backend reported none', async () => {
    mocks.exportDesign.mockResolvedValue(artifact({ digest: null }));
    seedDesign();
    const view = render(<ExportPanel />);
    fireEvent.click(designButton(view.container));

    const receipt = await waitFor(() => {
      const node = view.container.querySelector('.export-panel-receipt');
      expect(node).toBeTruthy();
      return node!;
    });
    expect(receipt.querySelector('.export-panel-receipt-digest')?.textContent).toBe('—');
    expect(view.container.querySelector('button[aria-label="Copy sha256"]')).toBeNull();
  });

  it('reports a failed design export', async () => {
    mocks.exportDesign.mockRejectedValue(
      new StudioApiError(503, {
        code: 'design_unavailable',
        message: 'The geometry backend is not available',
        details: [],
      }),
    );
    seedDesign();
    const view = render(<ExportPanel />);
    fireEvent.click(designButton(view.container));

    await waitFor(() => {
      expect(view.container.querySelector('.export-panel-error')?.textContent).toContain(
        'design_unavailable',
      );
    });
    expect(view.container.querySelector('.export-panel-receipt')).toBeNull();
  });
});

describe('ExportPanel — no design', () => {
  it('offers no design export when no design is active', () => {
    const view = render(<ExportPanel />);
    expect(view.container.querySelector('.export-panel-design')).toBeNull();
    expect(view.container.querySelector('.export-panel-note')).toBeNull();
  });

  it('says so when the backend has no design routes', () => {
    dispatch({ type: 'server/capabilities', designAvailable: false });
    const view = render(<ExportPanel />);
    const note = view.container.querySelector('.export-panel-note')!;
    expect(note.textContent).toContain('design_available: false');
    expect(note.textContent).toContain('Python geometry backend');
    expect(view.container.querySelector('.export-panel-design')).toBeNull();
  });
});
