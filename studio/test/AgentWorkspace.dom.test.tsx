// @vitest-environment happy-dom
// Vellum drafting-board workspace regression coverage.
//
// These drive the real shipped App through the real boot path (same mock
// surface as AppBoot.dom.test.tsx), so a regression that re-buries the agent
// or mounts a second WebGL canvas fails here.
//
// The 4-zone contract under test:
//   left chat · center drafting sheet + 48px ruler bar · right AuxView + journal
// Everything that worked before (chat, preview, estimates, export, provider
// keys) stays reachable; the workbench panels now live in the Options drawer.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  getSettings: vi.fn(),
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
  return container.querySelector<HTMLElement>('.studio-zone-left')!;
}

function center(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.studio-zone-center')!;
}

function right(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.studio-zone-right')!;
}

function sceneCanvases(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>('.viewer-canvas'));
}

function bottomButton(container: HTMLElement, label: string): HTMLButtonElement {
  return Array.from(container.querySelectorAll<HTMLButtonElement>('.bottom-bar button')).find(
    (button) => button.getAttribute('aria-label') === label || button.textContent?.includes(label),
  )!;
}

async function openOptions(container: HTMLElement): Promise<HTMLButtonElement> {
  const trigger = container.querySelector<HTMLButtonElement>('button.bottom-bar-options')!;
  trigger.focus();
  fireEvent.click(trigger);
  await waitFor(() => {
    expect(container.querySelector('[role="dialog"]')).toBeTruthy();
  });
  return trigger;
}

describe('Vellum 4-zone layout', () => {
  it('puts the agent chat in the left zone and the drafting sheet in the center', async () => {
    const view = await bootShell();

    expect(left(view.container).querySelector('.chat-panel')).toBeTruthy();
    expect(left(view.container).textContent).toContain('MECHANICAL DESIGN AGENT');

    const sheet = center(view.container).querySelector('.drafting-sheet');
    expect(sheet).toBeTruthy();
    expect(sheet!.querySelector('.viewer-canvas')).toBeTruthy();
    expect(center(view.container).querySelector('.bottom-bar')).toBeTruthy();

    expect(view.container.querySelector('.app-header')).toBeNull();
    expect(view.container.querySelector('.right-rail')).toBeNull();
  });

  it('keeps the AuxView and journal in the right zone, with real connection info', async () => {
    const view = await bootShell();
    expect(right(view.container).querySelector('.aux-view')).toBeTruthy();
    expect(right(view.container).querySelector('.journal-panel')).toBeTruthy();

    await waitFor(() => {
      expect(right(view.container).querySelector('.journal-panel-footer')?.textContent).toContain(
        BASE_URL,
      );
    });
    // No cryptographic ledger is reported by the backend: neutral session
    // stamp only, never a fake OK.
    expect(right(view.container).textContent).toContain('SESSION LOG');
    expect(right(view.container).textContent).not.toContain('HASH-CHAIN OK');
  });

  it('mounts exactly one SceneCanvas while the Options drawer opens and closes', async () => {
    const view = await bootShell();
    expect(sceneCanvases(view.container)).toHaveLength(1);

    await openOptions(view.container);
    expect(sceneCanvases(view.container)).toHaveLength(1);

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(view.container.querySelector('[role="dialog"]')).toBeNull());
    expect(sceneCanvases(view.container)).toHaveLength(1);
    expect(sceneCanvases(view.container)[0]!.closest('.drafting-sheet')).toBeTruthy();
  });

  it('shows the Kala Studio title block on the sheet', async () => {
    const view = await bootShell();
    const titleBlock = center(view.container).querySelector('.drafting-title-block');
    expect(titleBlock?.textContent).toContain('Kala Studio');
    expect(titleBlock?.textContent).toContain('STUDIO-BOX');
    expect(titleBlock?.textContent).toContain('METRIC (MM)');
  });
});

describe('bottom ruler bar', () => {
  it('shows the real nominal readout and its source', async () => {
    const view = await bootShell();
    await waitFor(() => {
      const readout = center(view.container).querySelector('.bottom-bar-readout');
      expect(readout?.textContent).toContain('24.0 cm³');
      expect(readout?.textContent).toContain('52.0 cm²');
      expect(readout?.textContent).toContain('40×20×30 mm');
    });
    const source = center(view.container).querySelector('.bottom-bar-source');
    expect(['local est.', 'server']).toContain(source?.getAttribute('data-source'));

    await waitFor(() => {
      expect(center(view.container).querySelector('.bottom-bar-source')?.getAttribute('data-source')).toBe('server');
    });
  });

  it('wires Grid, Axes, Dims, Section, render mode, and Reset View to real state', async () => {
    const view = await bootShell();
    const bar = center(view.container);

    const grid = bottomButton(bar, 'Grid');
    expect(grid.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(grid);
    await waitFor(() => expect(getState().viewport.showGrid).toBe(false));
    expect(bottomButton(bar, 'Grid').getAttribute('aria-checked')).toBe('false');

    fireEvent.click(bottomButton(bar, 'Axes'));
    fireEvent.click(bottomButton(bar, 'Dims'));
    expect(getState().viewport.showAxes).toBe(false);
    expect(getState().viewport.showDimensions).toBe(false);

    fireEvent.click(bar.querySelector<HTMLButtonElement>('button[aria-label="Section A-A"]')!);
    await waitFor(() => expect(getState().showSection).toBe(false));
    expect(center(view.container).querySelector('.drafting-sheet')?.getAttribute('data-section')).toBe('off');

    fireEvent.click(bar.querySelector<HTMLButtonElement>('button[data-render-mode="clay3d"]')!);
    await waitFor(() => expect(getState().renderMode).toBe('clay3d'));
    expect(center(view.container).querySelector('.drafting-sheet')?.getAttribute('data-render-mode')).toBe('clay3d');

    const beforeReset = getState().viewResetToken;
    fireEvent.click(bar.querySelector<HTMLButtonElement>('button[aria-label="Reset View"]')!);
    expect(getState().viewResetToken).toBe(beforeReset + 1);
  });

  it('reports only the real FreeCAD integration status', async () => {
    const view = await bootShell();
    await waitFor(() => {
      const pill = center(view.container).querySelector('.bottom-bar-freecad');
      expect(pill?.textContent).toContain('FreeCAD');
      expect(pill?.getAttribute('aria-label')).toContain('neutral');
      expect(pill?.getAttribute('title')).toContain('neutral');
      expect(pill?.getAttribute('data-status')).toBe('available_via_neutral_file');
    });
  });

  it('opens the Options drawer on card 02 from Export STEP', async () => {
    const view = await bootShell();
    const trigger = center(view.container).querySelector<HTMLButtonElement>('.bottom-bar-export')!;
    fireEvent.click(trigger);

    await waitFor(() => {
      expect(view.container.querySelector('#options-card-export')?.classList.contains('is-focused')).toBe(true);
    });
    expect(getState().optionsOpen).toBe(true);
    expect(getState().optionsCard).toBe('export');
  });
});

describe('Options drawer', () => {
  it('renders the four drafted cards with the existing panels reachable', async () => {
    const view = await bootShell();
    await openOptions(view.container);

    const dialog = view.container.querySelector('[role="dialog"]')!;
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.querySelectorAll('.options-card')).toHaveLength(4);
    expect(dialog.querySelector('#options-card-inspector .inspector')).toBeTruthy();
    expect(dialog.querySelector('#options-card-export .export-panel')).toBeTruthy();
    expect(dialog.querySelector('#options-card-export .panel-list')).toBeTruthy();
    expect(dialog.querySelector('#options-card-providers .panel-list')).toBeTruthy();
    expect(dialog.querySelector('#options-card-connection')?.textContent).toContain(BASE_URL);
    expect(dialog.querySelector('#options-card-connection')?.textContent).toContain(
      'In memory only (not displayed)',
    );
  });

  it('closes on Escape and returns focus to the trigger', async () => {
    const view = await bootShell();
    const trigger = await openOptions(view.container);
    await waitFor(() => {
      expect(document.activeElement?.classList.contains('options-drawer-close')).toBe(true);
    });

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => {
      expect(view.container.querySelector('[role="dialog"]')).toBeNull();
    });
    expect(document.activeElement).toBe(trigger);
  });

  it('traps Tab inside the dialog', async () => {
    const view = await bootShell();
    await openOptions(view.container);
    const dialog = view.container.querySelector<HTMLElement>('[role="dialog"]')!;
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
    await openOptions(view.container);

    const slate = Array.from(
      view.container.querySelectorAll<HTMLButtonElement>('.options-theme-button'),
    ).find((button) => button.textContent === 'Slate')!;
    fireEvent.click(slate);

    await waitFor(() => expect(getState().theme).toBe('slate'));
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe('slate'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('slate');

    const vellum = Array.from(
      view.container.querySelectorAll<HTMLButtonElement>('.options-theme-button'),
    ).find((button) => button.textContent === 'Vellum')!;
    fireEvent.click(vellum);
    await waitFor(() => expect(getState().theme).toBe('vellum'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('vellum');
  });

  it('requests a real reconnect when Retry is pressed', async () => {
    const view = await bootShell();
    await openOptions(view.container);
    const before = getState().connectionRetryToken;
    const retry = Array.from(
      view.container.querySelectorAll<HTMLButtonElement>('#options-card-connection button'),
    ).find((button) => button.textContent?.includes('Reconnect'))!;
    fireEvent.click(retry);
    expect(getState().connectionRetryToken).toBe(before + 1);
    expect(getState().journal.some((entry) => entry.message.includes('Reconnect / retry requested'))).toBe(true);
  });
});

describe('AuxView tabs', () => {
  it('switches the aux view in the store and marks the tab selected', async () => {
    const view = await bootShell();
    const tabs = right(view.container).querySelectorAll<HTMLButtonElement>('.aux-view-tab');
    expect(Array.from(tabs).map((tab) => tab.textContent)).toEqual([
      'Section A-A',
      'Ortho 3-View',
      'Tolerance',
    ]);
    expect(tabs[0]!.getAttribute('aria-selected')).toBe('true');

    fireEvent.click(tabs[1]!);
    await waitFor(() => expect(getState().auxView).toBe('ortho'));
    expect(tabs[1]!.getAttribute('aria-selected')).toBe('true');
    expect(right(view.container).querySelectorAll('.aux-ortho-view')).toHaveLength(3);
    expect(right(view.container).textContent).toContain('THIRD-ANGLE');
  });
});

describe('OpenCascade geometry estimates in the Options drawer', () => {
  it('shows nominal volume, area, and envelope for the active primitive', async () => {
    const view = await bootShell();
    await openOptions(view.container);
    const estimates = view.container.querySelector<HTMLElement>('.inspector-estimates');
    expect(estimates).toBeTruthy();

    // Default box is 40 × 20 × 30 mm: V = 24000 mm³, A = 5200 mm².
    const text = estimates!.textContent ?? '';
    expect(text).toContain('Volume');
    expect(text).toContain('24000.00 mm³ (24.000 cm³)');
    expect(text).toContain('Surface area');
    expect(text).toContain('5200.00 mm² (52.000 cm²)');
    expect(text).toContain('Bounding envelope');
    expect(text).toContain('40.00 × 20.00 × 30.00 mm');
  });

  it('states that the estimates are nominal geometry, not simulation output', async () => {
    const view = await bootShell();
    await openOptions(view.container);
    const note = view.container.querySelector('.inspector-estimates-note');
    const text = note?.textContent ?? '';
    expect(text).toContain('Closed-form nominal geometry for sizing & handoff.');
    expect(text).toContain('Not a certified FEA solver result.');
    expect(text).toContain('CAD/CAE');
    expect(text).toContain('No stress, deflection, or tolerance');
  });

  it('recomputes the estimate and the chat banner when the primitive changes', async () => {
    const view = await bootShell();
    await openOptions(view.container);
    const kindGroup = view.container.querySelector<HTMLElement>('[aria-label="Primitive kind"]')!;
    const cylinder = Array.from(kindGroup.querySelectorAll('button')).find(
      (button) => button.textContent === 'Cylinder',
    )!;
    fireEvent.click(cylinder);

    await waitFor(() => {
      expect(view.container.querySelector('.inspector-estimates')?.textContent).toContain(
        '50265.48 mm³ (50.265 cm³)',
      );
    });
    expect(view.container.querySelector('.inspector-estimates')?.textContent).toContain(
      '40.00 × 40.00 × 40.00 mm',
    );

    const context = left(view.container).querySelector<HTMLElement>('.chat-panel-context');
    expect(context!.textContent).toContain('cylinder');
    expect(context!.textContent).toContain('envelope 40.00 × 40.00 × 40.00 mm');
    expect(context!.textContent).toContain('volume 50265.48 mm³ (50.265 cm³)');
  });
});

describe('agent composer and geometry context', () => {
  it('shows the active OpenCascade primitive, dimensions, and nominal volume', async () => {
    const view = await bootShell();
    const context = left(view.container).querySelector<HTMLElement>('.chat-panel-context');
    expect(context).toBeTruthy();

    const text = context!.textContent ?? '';
    expect(text).toContain('Active geometry');
    expect(text).toContain('box');
    expect(text).toContain('40.00 mm × 20.00 mm × 30.00 mm');
    expect(text).toContain('envelope 40.00 × 20.00 × 30.00 mm');
    expect(text).toContain('volume 24000.00 mm³ (24.000 cm³)');
  });

  it('seeds the composer from an engineering starter prompt without sending it', async () => {
    const view = await bootShell();
    const textarea = left(view.container).querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    expect(textarea.value).toBe('');

    const starter = left(view.container).querySelector<HTMLButtonElement>('button[data-starter="size-shaft"]')!;
    fireEvent.click(starter);

    expect(textarea.value).toContain('shaft spacer');
    expect(textarea.value).toContain('25 mm diameter by 120 mm long');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('keeps the documented composer selectors in the left zone', async () => {
    const view = await bootShell();
    const panel = left(view.container);

    expect(panel.querySelector('select#chat-provider-select')).toBeTruthy();
    expect(panel.querySelector('input#chat-model-input[type="text"]')).toBeTruthy();
    expect(panel.querySelector('textarea.chat-panel-prompt')).toBeTruthy();
    expect(panel.querySelector('textarea.chat-panel-prompt')!.getAttribute('placeholder'))
      .toContain('Describe geometry, tolerance, or prompt');
    expect(panel.querySelector('button.chat-panel-send')!.textContent).toContain(
      'Draft Part Geometry',
    );
    expect(panel.querySelector('button.chat-panel-clear')!.textContent).toBe('Clear');
    expect(panel.querySelector('.chat-panel-empty')).toBeTruthy();
    expect(panel.querySelector('select')!.id).toBe('chat-provider-select');
    expect(panel.querySelector('textarea')).toBe(panel.querySelector('textarea.chat-panel-prompt'));
  });

  it('requires explicit remote consent before a remote provider can send', async () => {
    const view = await bootShell();
    const panel = left(view.container);

    fireEvent.change(panel.querySelector<HTMLSelectElement>('#chat-provider-select')!, {
      target: { value: 'openai' },
    });
    await waitFor(() => {
      expect(panel.querySelector('input[type="checkbox"]')).toBeTruthy();
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

    fireEvent.click(panel.querySelector<HTMLInputElement>('input[type="checkbox"]')!);
    await waitFor(() => {
      expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false);
    });
    expect(fetch).not.toHaveBeenCalled();
  });

  it('lets the operator configure or clear provider API keys from the chat panel', async () => {
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

    const openrouterOption = panel.querySelector<HTMLOptionElement>(
      'select#chat-provider-select option[value="openrouter"]',
    )!;
    expect(openrouterOption.disabled).toBe(true);

    const toggle = panel.querySelector<HTMLButtonElement>('button.chat-panel-settings-toggle')!;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');

    const settingsSection = panel.querySelector<HTMLElement>('.chat-panel-settings')!;
    const keyInput = settingsSection.querySelector<HTMLInputElement>(
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
        panel.querySelector<HTMLOptionElement>(
          'select#chat-provider-select option[value="openrouter"]',
        )!.disabled,
      ).toBe(false);
    });
  });
});

describe('journal appenders', () => {
  it('records connection, preview tool calls, preview results, and chat turns', async () => {
    const view = await bootShell();

    await waitFor(() => {
      const text = right(view.container).querySelector('.journal-list')?.textContent ?? '';
      expect(text).toContain('Loopback connected');
      expect(text).toContain('POST /api/studio/preview');
      expect(text).toContain('Server preview verified');
    });

    const panel = left(view.container);
    fireEvent.change(panel.querySelector<HTMLSelectElement>('#chat-provider-select')!, {
      target: { value: 'local' },
    });
    fireEvent.change(panel.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!, {
      target: { value: 'check journal' },
    });
    await waitFor(() => {
      expect(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false);
    });
    fireEvent.click(panel.querySelector<HTMLButtonElement>('button.chat-panel-send')!);

    await waitFor(() => {
      expect(mocks.postChat).toHaveBeenCalled();
      const text = right(view.container).querySelector('.journal-list')?.textContent ?? '';
      expect(text).toContain('You · check journal');
      expect(text).toContain('POST /api/studio/chat · local');
      expect(text).toContain('Agent · A 40 x 20 x 30 mm box');
    });

    expect(getState().journal.length).toBeGreaterThan(0);
  });
});
