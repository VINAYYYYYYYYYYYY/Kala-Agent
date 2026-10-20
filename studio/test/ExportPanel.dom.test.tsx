// @vitest-environment happy-dom
// Export & handoff panel regression tests.
//
// The conductor found the real 401/503/409 wire contract was mis-modeled:
// the backend streams the artefact in the body of a single authenticated
// POST and there is no `GET /artifacts/...` route, yet the panel expected a
// JSON metadata body plus a follow-up download. These tests drive the real
// panel with a mocked client that returns the actual `ExportArtifact`
// shape, and assert the direct button works, capability-supported neutral
// handoff works even when native automation is unavailable, unsupported
// capabilities are refused, errors are surfaced, and no second GET is made.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postExport: vi.fn(),
  postHandoff: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return { ...actual, postExport: mocks.postExport, postHandoff: mocks.postHandoff };
});

import { StudioApiError, StudioAuthError } from '../src/api/client';
import { ExportPanel } from '../src/components/export/ExportPanel';
import { dispatch, getState, resetStore } from '../src/state/store';
import type { BoxParameters, ExportFormat, ExportArtifact, IntegrationRecord } from '../src/types/studio';

const connection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop' as const,
};

const BOX: BoxParameters = { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 };

const FREECAD: IntegrationRecord = {
  id: 'freecad',
  name: 'FreeCAD',
  category: 'cad',
  platforms: ['linux', 'open-source'],
  status: 'available_via_neutral_file',
  capabilities: ['step', 'stl'],
  notes: '',
};

const SOLIDWORKS: IntegrationRecord = {
  id: 'solidworks',
  name: 'SOLIDWORKS',
  category: 'cad',
  platforms: ['windows'],
  status: 'native_api_unavailable',
  capabilities: ['step'],
  notes: 'Native automation requires a licence.',
};

const OPENSCAD: IntegrationRecord = {
  id: 'openscad',
  name: 'OpenSCAD',
  category: 'cad',
  platforms: ['linux'],
  status: 'not_configured',
  capabilities: ['stl'],
  notes: '',
};

function makeArtifact(
  bytes: number,
  filename: string,
  format: ExportFormat,
  integrationId: string | null,
): ExportArtifact {
  return { blob: new Blob([new Uint8Array(bytes)]), filename, bytes, format, integrationId };
}

// happy-dom may not implement object URLs; provide them so saveBlob runs.
if (typeof (URL as { createObjectURL?: unknown }).createObjectURL !== 'function') {
  (URL as unknown as Record<string, unknown>).createObjectURL = () => 'blob:mock';
}
if (typeof (URL as { revokeObjectURL?: unknown }).revokeObjectURL !== 'function') {
  (URL as unknown as Record<string, unknown>).revokeObjectURL = () => undefined;
}

let clickSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  resetStore();
  dispatch({ type: 'connection/ready', connection: connection });
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock');
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => undefined);
  clickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
  // Any direct fetch would be a stray request (e.g. a bogus GET artifact).
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('unexpected direct fetch'))));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  mocks.postExport.mockReset();
  mocks.postHandoff.mockReset();
  resetStore();
});

function selects(container: HTMLElement): [HTMLSelectElement, HTMLSelectElement] {
  const nodes = container.querySelectorAll<HTMLSelectElement>('select');
  return [nodes[0]!, nodes[1]!];
}

function downloadButton(container: HTMLElement): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>('button.export-panel-button')!;
}

// Seed the store before mounting so the target options exist by the time a
// test selects one (a post-render dispatch would not flush before the change).
function renderPanel(rows: IntegrationRecord[]): ReturnType<typeof render> {
  dispatch({ type: 'server/integrations', integrations: rows });
  return render(<ExportPanel />);
}

describe('ExportPanel — direct export', () => {
  it('enables and performs a direct STL export in a single POST', async () => {
    mocks.postExport.mockResolvedValue(makeArtifact(84, 'kala-studio-box.stl', 'stl', null));
    const view = renderPanel([]);

    const button = downloadButton(view.container);
    expect(button.disabled).toBe(false);
    expect(button.textContent).toContain('Download STL');

    fireEvent.click(button);

    await waitFor(() => expect(mocks.postExport).toHaveBeenCalledTimes(1));
    expect(mocks.postExport).toHaveBeenCalledWith(
      connection,
      { kind: 'box', parameters: BOX, format: 'stl' },
    );
    expect(mocks.postHandoff).not.toHaveBeenCalled();
    expect(clickSpy).toHaveBeenCalled();

    await waitFor(() => {
      expect(view.container.querySelector('.export-panel-result')?.textContent).toContain('84');
    });
    // The real byte count lands in the bounded journal as an export receipt.
    const receipt = getState().journal.find((entry) => entry.kind === 'export');
    expect(receipt?.message).toContain('kala-studio-box.stl');
    expect(receipt?.digest).toContain('84 bytes');
    // No stray GET followed the export.
    expect(fetch).not.toHaveBeenCalled();
  });

  it('performs a direct STEP export', async () => {
    mocks.postExport.mockResolvedValue(makeArtifact(512, 'kala-studio-box.step', 'step', null));
    const view = renderPanel([]);
    const [, formatSelect] = selects(view.container);

    fireEvent.change(formatSelect, { target: { value: 'step' } });
    await waitFor(() => expect(downloadButton(view.container).textContent).toContain('STEP'));

    fireEvent.click(downloadButton(view.container));
    await waitFor(() => expect(mocks.postExport).toHaveBeenCalledTimes(1));
    expect(mocks.postExport).toHaveBeenCalledWith(
      connection,
      { kind: 'box', parameters: BOX, format: 'step' },
    );
  });
});

describe('ExportPanel — neutral handoff', () => {
  it('offers STL and STEP handoff for a capability-listed row (FreeCAD)', async () => {
    mocks.postHandoff.mockResolvedValue(makeArtifact(84, 'freecad.stl', 'stl', 'freecad'));
    const view = renderPanel([FREECAD]);
    const [targetSelect] = selects(view.container);

    fireEvent.change(targetSelect, { target: { value: 'freecad' } });
    await waitFor(() => expect(downloadButton(view.container).disabled).toBe(false));

    fireEvent.click(downloadButton(view.container));
    await waitFor(() => expect(mocks.postHandoff).toHaveBeenCalledTimes(1));
    expect(mocks.postHandoff).toHaveBeenCalledWith(connection, {
      integration_id: 'freecad',
      kind: 'box',
      parameters: BOX,
      format: 'stl',
    });
  });

  it('allows a capability-supported STEP handoff for a native_api_unavailable row (SOLIDWORKS)', async () => {
    mocks.postHandoff.mockResolvedValue(makeArtifact(1024, 'solidworks.step', 'step', 'solidworks'));
    const view = renderPanel([SOLIDWORKS]);
    const [targetSelect, formatSelect] = selects(view.container);

    fireEvent.change(targetSelect, { target: { value: 'solidworks' } });
    await waitFor(() => expect(formatSelect.value).toBe('step'));

    const button = downloadButton(view.container);
    expect(button.disabled).toBe(false);

    // Honest warning; never claims to launch or drive the native tool.
    const text = view.container.textContent ?? '';
    expect(text).toContain('Native automation is not available for SOLIDWORKS');
    expect(text).toContain('never launches or drives SOLIDWORKS');
    expect(text).not.toContain('native-only');

    fireEvent.click(button);
    await waitFor(() => expect(mocks.postHandoff).toHaveBeenCalledTimes(1));
    expect(mocks.postHandoff).toHaveBeenCalledWith(connection, {
      integration_id: 'solidworks',
      kind: 'box',
      parameters: BOX,
      format: 'step',
    });
  });

  it('refuses a format the target does not list (SOLIDWORKS has no STL)', async () => {
    const view = renderPanel([SOLIDWORKS]);
    const [targetSelect, formatSelect] = selects(view.container);

    fireEvent.change(targetSelect, { target: { value: 'solidworks' } });
    await waitFor(() => expect(formatSelect.value).toBe('step'));

    const stlOption = formatSelect.querySelector<HTMLOptionElement>('option[value="stl"]')!;
    expect(stlOption.disabled).toBe(true);
  });

  it('keeps an unconfigured target disabled', async () => {
    const view = renderPanel([OPENSCAD]);
    const [targetSelect] = selects(view.container);

    fireEvent.change(targetSelect, { target: { value: 'openscad' } });
    await waitFor(() => expect(view.container.textContent).toContain('not configured'));
    expect(downloadButton(view.container).disabled).toBe(true);
  });
});

describe('ExportPanel — failure reporting', () => {
  it('presents an authentication failure', async () => {
    mocks.postExport.mockRejectedValue(
      new StudioAuthError(401, { code: 'unauthorized', message: 'token bad', details: [] }),
    );
    const view = renderPanel([]);

    fireEvent.click(downloadButton(view.container));

    await waitFor(() => {
      const alert = view.container.querySelector('.export-panel-error');
      expect(alert?.textContent).toContain('auth');
      expect(alert?.textContent).toContain('401');
    });
  });

  it('presents a 503 step_unavailable honestly', async () => {
    mocks.postExport.mockRejectedValue(
      new StudioApiError(503, {
        code: 'step_unavailable',
        message: 'STEP export requires the OCP bindings',
        details: [],
      }),
    );
    const view = renderPanel([]);

    fireEvent.click(downloadButton(view.container));

    await waitFor(() => {
      expect(view.container.querySelector('.export-panel-error')?.textContent).toContain(
        'step_unavailable',
      );
    });
    // A failed export must not have produced a result line.
    expect(view.container.querySelector('.export-panel-result')).toBeNull();
  });
});
