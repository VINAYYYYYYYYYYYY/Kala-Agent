// @vitest-environment happy-dom
// Vellum drafting-board workspace regression coverage.
//
// These drive the real shipped App through the real boot path (same mock
// surface as AppBoot.dom.test.tsx), so a regression that re-buries the agent,
// duplicates a control or mounts a second WebGL canvas fails here.
//
// App-feel wave (2026-10-31), chat-first direction: the four-zone dashboard
// became an app frame:
//   top bar 44 · chat (centred, primary) · drawing split (closed by default)
//   · Details 360 optional · status 28
// The behaviour assertions are unchanged; only their one home moved (D3/D9):
//   render mode -> drawing panel header; view tools -> sheet rail;
//   Export -> Export dialog; settings -> Settings dialog;
//   geometry -> Details › Properties; AuxView -> Details › Views;
//   journal -> Details › Activity; connection and the measured readout ->
//   the status bar (the readout only while the drawing is open).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  getSettings: vi.fn(),
  getProviderModels: vi.fn(),
  postPreview: vi.fn(),
  postChat: vi.fn(),
  postSettings: vi.fn(),
  resolveConnectionFromBridge: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    getHealth: mocks.getHealth,
    getIntegrations: mocks.getIntegrations,
    getProviders: mocks.getProviders,
    getSettings: mocks.getSettings,
    getProviderModels: mocks.getProviderModels,
    postPreview: mocks.postPreview,
    postChat: mocks.postChat,
    postSettings: mocks.postSettings,
  };
});

vi.mock('../src/api/connection', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/connection')>();
  return {
    ...actual,
    resolveConnectionFromBridge: mocks.resolveConnectionFromBridge,
  };
});

import { App } from '../src/App';
import { getState, resetStore, THEME_STORAGE_KEY } from '../src/state/store';
import { openProviderPicker, pickProvider, providerOption } from './providerFixtures';

const BASE_URL = 'http://127.0.0.1:8765';

beforeEach(() => {
  try {
    localStorage.removeItem(THEME_STORAGE_KEY);
  } catch {
    // node environment without localStorage
  }
  resetStore();
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('unexpected fetch in unit test'))));
  mocks.resolveConnectionFromBridge.mockResolvedValue({
    baseUrl: BASE_URL,
    token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
    mode: 'desktop' as const,
  });
  mocks.getHealth.mockResolvedValue({
    status: 'ok',
    service: 'kala-studio',
    version: '0.1.0',
    offline: false,
    step_available: true,
    integrations: [],
    providers: [],
  });
  mocks.getIntegrations.mockResolvedValue({
    integrations: [
      {
        id: 'freecad',
        name: 'FreeCAD',
        category: 'cad',
        platforms: ['linux'],
        status: 'available_via_neutral_file',
        capabilities: ['stl', 'step'],
        notes: '',
      },
    ],
  });
  mocks.getProviders.mockResolvedValue({
    providers: [
      { id: 'local', name: 'Local helper', configured: true, notes: '' },
      { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
      { id: 'openrouter', name: 'OpenRouter', configured: false, notes: '' },
    ],
  });
  mocks.getSettings.mockResolvedValue({
    credentials: [
      { provider: 'openai', env_var: 'OPENAI_API_KEY', configured: true, source: 'stored' },
      { provider: 'openrouter', env_var: 'OPENROUTER_API_KEY', configured: false, source: 'none' },
    ],
    providers: [
      { id: 'local', name: 'Local helper', configured: true, notes: '' },
      { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
      { id: 'openrouter', name: 'OpenRouter', configured: false, notes: '' },
    ],
    offline: false,
  });
  mocks.getProviderModels.mockResolvedValue({
    provider: 'local',
    source: 'static',
    fetched_at: null,
    models: [],
  });
  mocks.postPreview.mockResolvedValue({
    positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
    indices: [0, 1, 2],
    bounds: { min: [0, 0, 0], max: [1, 1, 1] },
    engine: 'analytic',
    warnings: [],
  });
  mocks.postChat.mockResolvedValue({
    text: 'A 40 x 20 x 30 mm box has a nominal volume of 24000 mm3.',
    provider: 'local',
    model: null,
    offline: true,
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  mocks.getHealth.mockReset();
  mocks.getIntegrations.mockReset();
  mocks.getProviders.mockReset();
  mocks.getSettings.mockReset();
  mocks.getProviderModels.mockReset();
  mocks.postPreview.mockReset();
  mocks.postChat.mockReset();
  mocks.postSettings.mockReset();
  mocks.resolveConnectionFromBridge.mockReset();
  try {
    localStorage.removeItem(THEME_STORAGE_KEY);
    delete document.documentElement.dataset.theme;
  } catch {
    // ignore
  }
  resetStore();
});

async function bootShell() {
  const view = render(<App />);
  await waitFor(() => {
    expect(view.container.querySelector('.studio-shell')).toBeTruthy();
  });
  return view;
}

function left(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.studio-chat-column')!;
}

function center(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.studio-drawing-column')!;
}

async function openDrawing(container: HTMLElement): Promise<void> {
  fireEvent.click(topBarButton(container, 'Drawing'));
  await waitFor(() => expect(container.querySelector('.drafting-sheet')).toBeTruthy());
}

function details(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.details-panel')!;
}

function statusBar(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.bottom-bar')!;
}

function sceneCanvases(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>('.viewer-canvas'));
}

function topBarButton(container: HTMLElement, label: string): HTMLButtonElement {
  const button = Array.from(container.querySelectorAll<HTMLButtonElement>('.top-bar button')).find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!button) throw new Error(`the top bar has no "${label}" button`);
  return button;
}

function sheetTool(container: HTMLElement, label: string): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>(
    `.drafting-tools button[aria-label="${label}"]`,
  );
  if (!button) throw new Error(`the sheet rail has no "${label}" tool`);
  return button;
}

function detailsTab(container: HTMLElement, label: string): HTMLButtonElement {
  const tab = Array.from(
    details(container).querySelectorAll<HTMLButtonElement>('.details-panel-tab'),
  ).find((candidate) => candidate.textContent === label);
  if (!tab) throw new Error(`Details has no "${label}" tab`);
  return tab;
}

async function openDetails(container: HTMLElement, tab?: 'Properties' | 'Views' | 'Activity') {
  fireEvent.click(topBarButton(container, 'Details'));
  await waitFor(() => expect(container.querySelector('.details-panel')).toBeTruthy());
  if (tab) fireEvent.click(detailsTab(container, tab));
}

function settingsDialog(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.settings-dialog')!;
}

async function openSettings(container: HTMLElement): Promise<HTMLButtonElement> {
  const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="Settings"]')!;
  trigger.focus();
  fireEvent.click(trigger);
  await waitFor(() => expect(container.querySelector('.settings-dialog')).toBeTruthy());
  return trigger;
}

function settingsNav(container: HTMLElement, label: string): HTMLButtonElement {
  const item = Array.from(
    settingsDialog(container).querySelectorAll<HTMLButtonElement>('.settings-nav-item'),
  ).find((candidate) => candidate.textContent?.replace(/^\d+/, '').trim() === label);
  if (!item) throw new Error(`Settings has no "${label}" section`);
  return item;
}

describe('app frame', () => {
  it('boots as the centred agent chat, with the drawing closed and no canvas mounted', async () => {
    const view = await bootShell();

    const topBar = view.container.querySelector('.top-bar');
    expect(topBar).toBeTruthy();
    expect(topBar!.textContent).toContain('KALA');
    expect(topBar!.textContent).toContain('STUDIO');
    // The render mode has one home: the drawing panel header.
    expect(topBar!.textContent).not.toContain('2D Pencil');
    expect(topBar!.textContent).not.toContain('3D Clay');
    expect(topBar!.querySelector('[data-render-mode]')).toBeNull();
    // The chat is always shown; there is no chat toggle.
    expect(topBar!.querySelector('.top-bar-chat-toggle')).toBeNull();
    expect(topBarButton(view.container, 'Drawing').getAttribute('aria-pressed')).toBe('false');

    expect(left(view.container).querySelector('.chat-panel')).toBeTruthy();
    expect(left(view.container).textContent).toContain('Agent');
    // A new chat: one short label above the composer, the starters under it.
    expect(left(view.container).querySelector('.chat-panel-empty-label')?.textContent).toBe(
      'New design',
    );
    expect(left(view.container).querySelectorAll('.chat-panel-starter')).toHaveLength(4);

    // The drawing is secondary: no sheet, no canvas, no readout at boot.
    expect(center(view.container)).toBeNull();
    expect(view.container.querySelector('.drafting-sheet')).toBeNull();
    expect(sceneCanvases(view.container)).toHaveLength(0);
    expect(statusBar(view.container).querySelector('.bottom-bar-readout')).toBeNull();
    expect(view.container.querySelector('.studio-main')!.getAttribute('data-drawing')).toBe(
      'closed',
    );

    // The status bar is the app's footer, not a row inside a sheet column.
    expect(statusBar(view.container)).toBeTruthy();

    // Details is closed by default; none of the old dashboard chrome exists.
    expect(view.container.querySelector('.details-panel')).toBeNull();
    expect(view.container.querySelector('.app-header')).toBeNull();
    expect(view.container.querySelector('.right-rail')).toBeNull();
  });

  it('opens the primitive drawing from the top bar, and closes it again', async () => {
    const view = await bootShell();
    await openDrawing(view.container);

    const sheet = center(view.container).querySelector('.drafting-sheet');
    expect(sheet).toBeTruthy();
    expect(sheet!.querySelector('.viewer-canvas')).toBeTruthy();
    expect(view.container.querySelector('.drawing-panel-name')?.textContent).toBe('BOX');
    expect(view.container.querySelector('.studio-main')!.getAttribute('data-drawing')).toBe('open');
    // The readout belongs to the drawing and appears with it.
    await waitFor(() => {
      expect(statusBar(view.container).querySelector('.bottom-bar-readout')).toBeTruthy();
    });

    fireEvent.click(view.container.querySelector<HTMLButtonElement>('button[aria-label="Close drawing"]')!);
    await waitFor(() => expect(view.container.querySelector('.drafting-sheet')).toBeNull());
    expect(sceneCanvases(view.container)).toHaveLength(0);
    expect(statusBar(view.container).querySelector('.bottom-bar-readout')).toBeNull();
  });

  it('keeps the AuxView and journal in Details, with real connection info in the status bar', async () => {
    const view = await bootShell();

    await waitFor(() => {
      const connection = statusBar(view.container).querySelector('.bottom-bar-connection');
      expect(connection?.textContent).toContain('Connected · Desktop');
      expect(connection?.getAttribute('title')).toContain(BASE_URL);
    });

    await openDetails(view.container, 'Views');
    expect(details(view.container).querySelector('.aux-view')).toBeTruthy();
    expect(details(view.container).querySelector('.journal-panel')).toBeNull();

    fireEvent.click(detailsTab(view.container, 'Activity'));
    await waitFor(() => {
      expect(details(view.container).querySelector('.journal-panel')).toBeTruthy();
    });
    expect(details(view.container).querySelector('.aux-view')).toBeNull();
    // No cryptographic ledger is reported by the backend: neutral session
    // stamp only, never a fake OK.
    expect(details(view.container).textContent).toContain('SESSION LOG');
    expect(details(view.container).textContent).not.toContain('HASH-CHAIN OK');
    // The footer is not repeated inside Details: the status bar owns it.
    expect(details(view.container).querySelector('.journal-panel-footer')).toBeNull();
    await waitFor(() => {
      expect(details(view.container).querySelector('.journal-list')?.textContent).toContain(
        'Loopback connected',
      );
    });
  });

  it('mounts exactly one SceneCanvas while Settings and Export open and close, and while Details toggles', async () => {
    const view = await bootShell();
    // The drawing is closed at boot, so no canvas exists yet.
    expect(sceneCanvases(view.container)).toHaveLength(0);
    await openDrawing(view.container);
    expect(sceneCanvases(view.container)).toHaveLength(1);

    await openSettings(view.container);
    expect(sceneCanvases(view.container)).toHaveLength(1);
    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(view.container.querySelector('.settings-dialog')).toBeNull());
    expect(sceneCanvases(view.container)).toHaveLength(1);

    fireEvent.click(topBarButton(view.container, 'Export'));
    await waitFor(() => expect(view.container.querySelector('.export-dialog')).toBeTruthy());
    expect(sceneCanvases(view.container)).toHaveLength(1);
    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(view.container.querySelector('.export-dialog')).toBeNull());
    expect(sceneCanvases(view.container)).toHaveLength(1);

    fireEvent.click(topBarButton(view.container, 'Details'));
    await waitFor(() => expect(view.container.querySelector('.details-panel')).toBeTruthy());
    expect(sceneCanvases(view.container)).toHaveLength(1);
    fireEvent.click(topBarButton(view.container, 'Details'));
    await waitFor(() => expect(view.container.querySelector('.details-panel')).toBeNull());
    expect(sceneCanvases(view.container)).toHaveLength(1);

    expect(sceneCanvases(view.container)[0]!.closest('.drafting-sheet')).toBeTruthy();
  });

  it('shows the Kala Studio title block on the sheet', async () => {
    const view = await bootShell();
    await openDrawing(view.container);
    const titleBlock = center(view.container).querySelector('.drafting-title-block');
    expect(titleBlock?.textContent).toContain('Kala Studio');
    expect(titleBlock?.textContent).toContain('STUDIO-BOX');
    expect(titleBlock?.textContent).toContain('METRIC (MM)');
  });
});

describe('status bar', () => {
  it('shows the real nominal readout and its source while the drawing is open', async () => {
    const view = await bootShell();
    // At boot there is no drawing and therefore no readout.
    expect(statusBar(view.container).querySelector('.bottom-bar-readout')).toBeNull();
    await openDrawing(view.container);
    await waitFor(() => {
      const readout = statusBar(view.container).querySelector('.bottom-bar-readout');
      expect(readout?.textContent).toContain('24.0 cm³');
      expect(readout?.textContent).toContain('52.0 cm²');
      expect(readout?.textContent).toContain('40×20×30 mm');
    });
    const source = statusBar(view.container).querySelector('.bottom-bar-source');
    expect(['local est.', 'server']).toContain(source?.getAttribute('data-source'));

    await waitFor(() => {
      expect(
        statusBar(view.container).querySelector('.bottom-bar-source')?.getAttribute('data-source'),
      ).toBe('server');
    });

    // The controls that used to share this bar have their own homes now.
    expect(statusBar(view.container).querySelector('button.bottom-bar-options')).toBeNull();
    expect(statusBar(view.container).querySelector('[data-render-mode]')).toBeNull();
  });

  it('wires Grid, Axes, Dims, Section, render mode and Reset View to real state', async () => {
    const view = await bootShell();
    await openDrawing(view.container);

    const grid = sheetTool(view.container, 'Toggle grid');
    expect(grid.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(grid);
    await waitFor(() => expect(getState().viewport.showGrid).toBe(false));
    expect(sheetTool(view.container, 'Toggle grid').getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(sheetTool(view.container, 'Toggle axes'));
    fireEvent.click(sheetTool(view.container, 'Toggle dimensions'));
    expect(getState().viewport.showAxes).toBe(false);
    expect(getState().viewport.showDimensions).toBe(false);

    fireEvent.click(sheetTool(view.container, 'Toggle Section A-A'));
    await waitFor(() => expect(getState().showSection).toBe(false));
    expect(center(view.container).querySelector('.drafting-sheet')?.getAttribute('data-section')).toBe(
      'off',
    );

    // Render mode is the drawing panel header's control, and only there.
    expect(view.container.querySelector('.top-bar [data-render-mode]')).toBeNull();
    fireEvent.click(
      view.container.querySelector<HTMLButtonElement>(
        '.drawing-panel-header [data-render-mode="clay3d"]',
      )!,
    );
    await waitFor(() => expect(getState().renderMode).toBe('clay3d'));
    expect(
      center(view.container).querySelector('.drafting-sheet')?.getAttribute('data-render-mode'),
    ).toBe('clay3d');

    const beforeReset = getState().viewResetToken;
    fireEvent.click(sheetTool(view.container, 'Reset view'));
    expect(getState().viewResetToken).toBe(beforeReset + 1);
  });

  it('reports only the real FreeCAD integration status', async () => {
    const view = await bootShell();
    await waitFor(() => {
      const pill = statusBar(view.container).querySelector('.bottom-bar-freecad');
      expect(pill?.textContent).toContain('FreeCAD');
      expect(pill?.getAttribute('aria-label')).toContain('neutral');
      expect(pill?.getAttribute('title')).toContain('neutral');
      expect(pill?.getAttribute('data-status')).toBe('available_via_neutral_file');
    });
  });

  it('opens the focused Export dialog from the top bar', async () => {
    const view = await bootShell();
    fireEvent.click(topBarButton(view.container, 'Export'));

    await waitFor(() => {
      expect(view.container.querySelector('.export-dialog')).toBeTruthy();
    });
    const dialog = modalDialog(view.container);
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.textContent).toContain('Export');
    expect(dialog.querySelector('.export-panel')).toBeTruthy();
    expect(getState().exportOpen).toBe(true);

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(view.container.querySelector('.export-dialog')).toBeNull());
    expect(getState().exportOpen).toBe(false);
  });
});

/** The one modal dialog of the moment (Export or Settings). */
function modalDialog(container: HTMLElement): HTMLElement {
  const dialog = container.querySelector<HTMLElement>('[role="dialog"][aria-modal="true"]');
  if (!dialog) throw new Error('no modal dialog is open');
  return dialog;
}

describe('Settings dialog', () => {
  it('renders the four sections with the existing panels reachable', async () => {
    const view = await bootShell();
    await openSettings(view.container);

    const dialog = settingsDialog(view.container);
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(
      Array.from(dialog.querySelectorAll('.settings-nav-item')).map((item) =>
        item.textContent?.replace(/^\d+/, '').trim(),
      ),
    ).toEqual(['Workspace', 'Models & keys', 'CAD', 'Connection']);

    // Workspace: the theme switch.
    expect(dialog.querySelector('.settings-theme-switch')).toBeTruthy();

    // Models & keys: the provider panel and its honesty line.
    fireEvent.click(settingsNav(view.container, 'Models & keys'));
    await waitFor(() => {
      expect(
        dialog.querySelector('.settings-section[data-section="providers"] .panel-list'),
      ).toBeTruthy();
    });
    expect(dialog.textContent).toContain('never echoed back to the browser');

    // CAD: the integration registry and the live provider rows.
    fireEvent.click(settingsNav(view.container, 'CAD'));
    await waitFor(() => {
      expect(dialog.querySelector('.settings-section[data-section="cad"] .panel-list')).toBeTruthy();
    });

    // Connection: the real loopback facts.
    fireEvent.click(settingsNav(view.container, 'Connection'));
    await waitFor(() => {
      expect(dialog.textContent).toContain(BASE_URL);
    });
    expect(dialog.textContent).toContain('In memory only (not displayed)');
  });

  it('closes on Escape and returns focus to the trigger', async () => {
    const view = await bootShell();
    const trigger = await openSettings(view.container);
    await waitFor(() => {
      expect(document.activeElement?.classList.contains('settings-close')).toBe(true);
    });

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => {
      expect(view.container.querySelector('.settings-dialog')).toBeNull();
    });
    expect(document.activeElement).toBe(trigger);
  });

  it('traps Tab inside the dialog', async () => {
    const view = await bootShell();
    await openSettings(view.container);
    const dialog = settingsDialog(view.container);
    const focusables = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    );
    const first = focusables[0]!;
    const last = focusables[focusables.length - 1]!;
    last.focus();
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(document, { key: 'Tab' });
    expect(document.activeElement).toBe(first);
  });

  it('switches between Vellum and Slate themes and persists only the theme name', async () => {
    const view = await bootShell();
    await openSettings(view.container);

    const slate = Array.from(
      settingsDialog(view.container).querySelectorAll<HTMLButtonElement>('.settings-theme-button'),
    ).find((button) => button.textContent === 'Slate')!;
    fireEvent.click(slate);

    await waitFor(() => expect(getState().theme).toBe('slate'));
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe('slate'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('slate');

    const vellum = Array.from(
      settingsDialog(view.container).querySelectorAll<HTMLButtonElement>('.settings-theme-button'),
    ).find((button) => button.textContent === 'Vellum')!;
    fireEvent.click(vellum);
    await waitFor(() => expect(getState().theme).toBe('vellum'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('vellum');
  });

  it('requests a real reconnect when Retry is pressed', async () => {
    const view = await bootShell();
    await openSettings(view.container);
    fireEvent.click(settingsNav(view.container, 'Connection'));

    const before = getState().connectionRetryToken;
    const retry = settingsDialog(view.container).querySelector<HTMLButtonElement>(
      'button.settings-connection-retry',
    )!;
    fireEvent.click(retry);
    expect(getState().connectionRetryToken).toBe(before + 1);
    expect(
      getState().journal.some((entry) => entry.message.includes('Reconnect / retry requested')),
    ).toBe(true);
  });
});

describe('Details › Views', () => {
  it('switches the aux view in the store and marks the tab selected', async () => {
    const view = await bootShell();
    await openDetails(view.container, 'Views');

    const tabs = details(view.container).querySelectorAll<HTMLButtonElement>('.aux-view-tab');
    expect(Array.from(tabs).map((tab) => tab.textContent)).toEqual([
      'Section A-A',
      'Ortho 3-View',
      'Tolerance',
    ]);
    expect(tabs[0]!.getAttribute('aria-selected')).toBe('true');

    fireEvent.click(tabs[1]!);
    await waitFor(() => expect(getState().auxView).toBe('ortho'));
    expect(tabs[1]!.getAttribute('aria-selected')).toBe('true');
    expect(details(view.container).querySelectorAll('.aux-ortho-view')).toHaveLength(3);
    expect(details(view.container).textContent).toContain('THIRD-ANGLE');
  });
});

describe('Details › Properties', () => {
  it('shows nominal volume, area, and envelope for the active primitive', async () => {
    const view = await bootShell();
    await openDetails(view.container);
    const summary = details(view.container).querySelector<HTMLElement>('.details-geometry');
    expect(summary).toBeTruthy();

    // Default box is 40 × 20 × 30 mm: V = 24000 mm³, A = 5200 mm².
    const text = summary!.textContent ?? '';
    expect(text).toContain('VOLUME');
    expect(text).toContain('24000.00 mm³ (24.000 cm³)');
    expect(text).toContain('SURFACE AREA');
    expect(text).toContain('5200.00 mm² (52.000 cm²)');
    expect(text).toContain('ENVELOPE');
    expect(text).toContain('40.00 × 20.00 × 30.00 mm');
  });

  it('states that the estimates are nominal geometry, not simulation output', async () => {
    const view = await bootShell();
    await openDetails(view.container);
    const note = details(view.container).querySelector('.details-geometry-note');
    const text = note?.textContent ?? '';
    expect(text).toContain('Closed-form nominal geometry for sizing & handoff.');
    expect(text).toContain('Not a certified FEA solver result.');
    expect(text).toContain('CAD/CAE');
    expect(text).toContain('No stress, deflection, or tolerance');
    // Once, under the summary: the inspector's readout card is hidden here.
    expect(details(view.container).querySelectorAll('.details-geometry-note')).toHaveLength(1);
    expect(details(view.container).querySelector('.inspector-estimates')).toBeNull();
    expect(details(view.container).querySelector('.inspector-bounds')).toBeNull();
  });

  it('recomputes the estimate when the primitive changes and never duplicates it in the chat', async () => {
    const view = await bootShell();
    await openDetails(view.container);
    const kindGroup = details(view.container).querySelector<HTMLElement>(
      '[aria-label="Primitive kind"]',
    )!;
    const cylinder = Array.from(kindGroup.querySelectorAll('button')).find(
      (button) => button.textContent === 'Cylinder',
    )!;
    fireEvent.click(cylinder);

    await waitFor(() => {
      expect(
        details(view.container).querySelector('.details-geometry-volume')?.textContent,
      ).toContain('50265.48 mm³ (50.265 cm³)');
    });
    expect(
      details(view.container).querySelector('.details-geometry-envelope')?.textContent,
    ).toContain('40.00 × 40.00 × 40.00 mm');

    // The active geometry has one home: the chat carries no second table.
    expect(left(view.container).querySelector('.chat-panel-context')).toBeNull();
    expect(left(view.container).querySelector('.details-geometry')).toBeNull();
    expect(left(view.container).textContent).not.toContain('Active geometry');
  });

  it('shows the active OpenCascade primitive, dimensions, and nominal volume', async () => {
    const view = await bootShell();
    await openDetails(view.container);
    const summary = details(view.container).querySelector<HTMLElement>('.details-geometry');
    const text = summary!.textContent ?? '';
    expect(text).toContain('Active geometry');
    expect(text).toContain('box');
    expect(text).toContain('40.00 mm × 20.00 mm × 30.00 mm');
    expect(text).toContain('40.00 × 20.00 × 30.00 mm');
    expect(text).toContain('24000.00 mm³ (24.000 cm³)');
    expect(text).toContain('nominal estimate (closed-form)');
  });
});

describe('agent composer', () => {
  it('seeds the composer from an engineering starter prompt without sending it', async () => {
    const view = await bootShell();
    const textarea = left(view.container).querySelector<HTMLTextAreaElement>(
      'textarea.chat-panel-prompt',
    )!;
    expect(textarea.value).toBe('');

    const starter = left(view.container).querySelector<HTMLButtonElement>(
      'button[data-starter="size-shaft"]',
    )!;
    fireEvent.click(starter);

    expect(textarea.value).toContain('shaft spacer');
    expect(textarea.value).toContain('25 mm diameter by 120 mm long');
    expect(mocks.postChat).not.toHaveBeenCalled();
  });

  it('keeps the documented composer selectors in the left column behind the chips', async () => {
    const view = await bootShell();
    const panel = left(view.container);

    const textarea = panel.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    expect(textarea).toBeTruthy();
    expect(textarea.getAttribute('placeholder')).toContain('Describe geometry, tolerance, or prompt');
    expect(panel.querySelector('button.chat-panel-send')!.textContent).toContain(
      'Draft Part Geometry',
    );
    expect(panel.querySelector('.chat-panel-empty')).toBeTruthy();
    // The old always-on Clear and the key drawer are gone; New chat appears
    // only once the thread has messages.
    expect(panel.querySelector('button.chat-panel-clear')).toBeNull();
    expect(panel.querySelector('.chat-panel-settings-toggle')).toBeNull();
    expect(panel.querySelector('.chat-panel-settings')).toBeNull();
    expect(panel.querySelector('button.chat-panel-new')).toBeNull();

    const chip = panel.querySelector<HTMLButtonElement>('[data-chip="model"]')!;
    expect(chip.getAttribute('aria-haspopup')).toBe('dialog');
    fireEvent.click(chip);
    await waitFor(() => {
      expect(panel.querySelector('#chat-provider-trigger')).toBeTruthy();
    });
    expect(panel.querySelector('#chat-provider-trigger')!.getAttribute('aria-haspopup')).toBe(
      'listbox',
    );
    expect(panel.querySelector('input#chat-model-input[type="text"]')).toBeTruthy();
    expect(panel.querySelector('textarea')).toBe(textarea);
  });

  it('requires explicit remote consent before a remote provider can send', async () => {
    const view = await bootShell();
    const panel = left(view.container);

    pickProvider(panel, 'openai');
    await waitFor(() => {
      expect(panel.querySelector('.chat-panel-consent input[type="checkbox"]')).toBeTruthy();
    });

    const textarea = panel.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    fireEvent.change(textarea, { target: { value: 'estimate the envelope' } });

    expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(true);

    fireEvent.change(panel.querySelector<HTMLInputElement>('#chat-model-input')!, {
      target: { value: 'gpt-4o-mini' },
    });
    await waitFor(() => {
      expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(true);
    });

    fireEvent.click(
      panel.querySelector<HTMLInputElement>('.chat-panel-consent input[type="checkbox"]')!,
    );
    await waitFor(() => {
      expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false);
    });
    expect(mocks.postChat).not.toHaveBeenCalled();
  });

  it('lets the operator configure or clear provider API keys from Settings › Models & keys', async () => {
    mocks.postSettings.mockResolvedValue({
      credentials: [
        { provider: 'openai', env_var: 'OPENAI_API_KEY', configured: true, source: 'stored' },
        { provider: 'openrouter', env_var: 'OPENROUTER_API_KEY', configured: true, source: 'stored' },
      ],
      providers: [
        { id: 'local', name: 'Local helper', configured: true, notes: '' },
        { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
        { id: 'openrouter', name: 'OpenRouter', configured: true, notes: '' },
      ],
      offline: false,
    });

    const view = await bootShell();
    const panel = left(view.container);

    // The chat has no key drawer any more: the keys have one home.
    expect(panel.querySelector('.chat-panel-settings-toggle')).toBeNull();
    expect(panel.querySelector('.chat-panel-settings')).toBeNull();

    openProviderPicker(panel);
    expect(providerOption(panel, 'openrouter').getAttribute('data-provider-state')).toBe(
      'not-configured',
    );
    // Close the listbox and then its popover so Settings has the screen.
    fireEvent.keyDown(document, { key: 'Escape' });
    fireEvent.keyDown(document, { key: 'Escape' });

    await openSettings(view.container);
    fireEvent.click(settingsNav(view.container, 'Models & keys'));
    const section = settingsDialog(view.container).querySelector<HTMLElement>(
      '.settings-section[data-section="providers"]',
    )!;
    const keyInput = section.querySelector<HTMLInputElement>(
      'input[aria-label="OpenRouter API key"]',
    )!;
    fireEvent.change(keyInput, { target: { value: 'sk-or-test-key-123456' } });
    fireEvent.submit(keyInput.closest('form')!);

    await waitFor(() => {
      expect(mocks.postSettings).toHaveBeenCalledWith(
        expect.objectContaining({ baseUrl: BASE_URL }),
        { provider: 'openrouter', api_key: 'sk-or-test-key-123456' },
      );
    });

    await waitFor(() => {
      expect(keyInput.value).toBe('');
      expect(
        section
          .querySelector('[data-provider-id="openrouter"]')
          ?.getAttribute('data-provider-state'),
      ).toBe('configured');
    });

    const clear = section
      .querySelector('[data-provider-id="openrouter"]')
      ?.querySelector<HTMLButtonElement>('button.provider-key-clear')!;
    expect(clear).toBeTruthy();
    fireEvent.click(clear);
    await waitFor(() => {
      expect(mocks.postSettings).toHaveBeenLastCalledWith(
        expect.objectContaining({ baseUrl: BASE_URL }),
        { provider: 'openrouter', api_key: null },
      );
    });
  });
});

describe('journal appenders', () => {
  it('records connection, preview tool calls, preview results, and chat turns', async () => {
    const view = await bootShell();
    await openDetails(view.container, 'Activity');

    await waitFor(() => {
      const text = details(view.container).querySelector('.journal-list')?.textContent ?? '';
      expect(text).toContain('Loopback connected');
      expect(text).toContain('POST /api/studio/preview');
      expect(text).toContain('Server preview verified');
    });

    const panel = left(view.container);
    pickProvider(panel, 'local');
    fireEvent.change(panel.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!, {
      target: { value: 'check journal' },
    });
    await waitFor(() => {
      expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false);
    });
    fireEvent.click(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!);

    await waitFor(() => {
      expect(mocks.postChat).toHaveBeenCalled();
      const text = details(view.container).querySelector('.journal-list')?.textContent ?? '';
      expect(text).toContain('You · check journal');
      expect(text).toContain('POST /api/studio/chat · local');
      expect(text).toContain('Agent · A 40 x 20 x 30 mm box');
    });

    expect(getState().journal.length).toBeGreaterThan(0);
  });
});
