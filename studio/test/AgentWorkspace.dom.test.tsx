// @vitest-environment happy-dom
// Chat-first workspace regression coverage.
//
// The pivot under test: Kala Studio's main `.app-shell-viewport` belongs to
// the Mechanical Engineering Agent, and the OpenCascade workbench (3D
// preview, parametric sizing, nominal geometry estimates, STEP/STL export)
// is live in the right rail. The toolbar's view switch flips into "3D Canvas
// Focus" for close mesh inspection.
//
// These drive the REAL shipped App through the real boot path (same mock
// surface as AppBoot.dom.test.tsx) rather than a hand-assembled tree, so a
// regression that re-buries the agent in a rail tab fails here.
//
// The WebGL discipline is asserted directly: exactly one `SceneCanvas` is
// mounted at any moment, and which container holds it flips with the view.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  getHealth: vi.fn(),
  getIntegrations: vi.fn(),
  getProviders: vi.fn(),
  getSettings: vi.fn(),
  postPreview: vi.fn(),
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
import { resetStore } from '../src/state/store';

beforeEach(() => {
  resetStore();
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('unexpected fetch in unit test'))));
  mocks.resolveConnectionFromBridge.mockResolvedValue({
    baseUrl: 'http://127.0.0.1:8765',
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
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  mocks.getHealth.mockReset();
  mocks.getIntegrations.mockReset();
  mocks.getProviders.mockReset();
  mocks.getSettings.mockReset();
  mocks.postPreview.mockReset();
  mocks.postSettings.mockReset();
  mocks.resolveConnectionFromBridge.mockReset();
  resetStore();
});

async function bootShell() {
  const view = render(<App />);
  // Wait for BootGate to release the shell.
  await waitFor(() => {
    expect(view.container.querySelector('.app-shell-viewport')).toBeTruthy();
  });
  return view;
}

function viewport(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.app-shell-viewport')!;
}

function rail(container: HTMLElement): HTMLElement {
  return container.querySelector<HTMLElement>('.right-rail')!;
}

// One SceneCanvas == one `.viewer-canvas` host element.
function sceneCanvases(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>('.viewer-canvas'));
}

function viewButton(container: HTMLElement, mode: string): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>(`button[data-view-mode="${mode}"]`)!;
}

describe('chat-first workspace layout', () => {
  it('puts the Mechanical Engineering Agent in the primary viewport by default', async () => {
    const view = await bootShell();
    const main = viewport(view.container);

    expect(main.getAttribute('data-view-mode')).toBe('agent');
    expect(main.querySelector('.chat-panel')).toBeTruthy();
    // The agent has the viewport: the 3D canvas is NOT mounted there.
    expect(main.querySelector('.viewer-canvas')).toBeNull();
    expect(main.textContent).toContain('Mechanical Engineering Agent');
  });

  it('keeps the OpenCascade workbench live in the right rail at the same time', async () => {
    const view = await bootShell();

    expect(rail(view.container).getAttribute('aria-label')).toBe('OpenCascade geometry and CAD workbench');
    expect(rail(view.container).querySelector('.right-rail-preview')).toBeTruthy();
    expect(rail(view.container).querySelector('.inspector')).toBeTruthy();
    // Exactly one WebGL context exists while the agent owns the viewport.
    expect(sceneCanvases(view.container)).toHaveLength(1);
    expect(sceneCanvases(view.container)[0]!.closest('.right-rail-preview')).toBeTruthy();
  });

  it('opens the rail on the Workbench tab and still exposes Export and Assistant', async () => {
    const view = await bootShell();
    const tabs = rail(view.container).querySelectorAll<HTMLButtonElement>('button.right-rail-tab');
    expect(Array.from(tabs).map((t) => t.textContent)).toEqual(['Workbench', 'Export', 'Assistant']);
    expect(tabs[0]!.getAttribute('aria-selected')).toBe('true');

    // Export tab → the STEP/STL handoff panel.
    fireEvent.click(tabs[1]!);
    await waitFor(() => {
      expect(rail(view.container).querySelector('.export-panel')).toBeTruthy();
    });
    expect(rail(view.container).textContent).toContain('Download');

    // Assistant tab → a secondary view of the same agent thread.
    fireEvent.click(tabs[2]!);
    await waitFor(() => {
      expect(rail(view.container).querySelector('.chat-panel')).toBeTruthy();
    });
  });

  it('expands the 3D canvas into the viewport and keeps exactly one SceneCanvas mounted', async () => {
    const view = await bootShell();

    fireEvent.click(viewButton(view.container, 'canvas'));
    await waitFor(() => {
      expect(viewport(view.container).getAttribute('data-view-mode')).toBe('canvas');
    });

    const main = viewport(view.container);
    expect(main.querySelector('.viewer-canvas')).toBeTruthy();
    expect(main.querySelector('.chat-panel')).toBeNull();
    // The rail drops its docked preview so the WebGL context count stays 1.
    expect(rail(view.container).querySelector('.right-rail-preview')).toBeNull();
    expect(sceneCanvases(view.container)).toHaveLength(1);

    // …and back again, with no canvas leak on either transition.
    fireEvent.click(viewButton(view.container, 'agent'));
    await waitFor(() => {
      expect(viewport(view.container).querySelector('.chat-panel')).toBeTruthy();
    });
    expect(sceneCanvases(view.container)).toHaveLength(1);
  });

  it('marks the active view mode for assistive tech, not colour alone', async () => {
    const view = await bootShell();
    const agent = viewButton(view.container, 'agent');
    const canvas = viewButton(view.container, 'canvas');
    expect(agent.getAttribute('role')).toBe('radio');
    expect(agent.getAttribute('aria-checked')).toBe('true');
    expect(canvas.getAttribute('aria-checked')).toBe('false');

    fireEvent.click(canvas);
    await waitFor(() => {
      expect(viewButton(view.container, 'canvas').getAttribute('aria-checked')).toBe('true');
    });
    expect(viewButton(view.container, 'agent').getAttribute('aria-checked')).toBe('false');
  });
});

describe('OpenCascade geometry estimates in the workbench', () => {
  it('shows nominal volume, area, and envelope for the active primitive', async () => {
    const view = await bootShell();
    const estimates = rail(view.container).querySelector<HTMLElement>('.inspector-estimates');
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
    const note = rail(view.container).querySelector('.inspector-estimates-note');
    const text = note?.textContent ?? '';
    expect(text).toContain('nominal');
    expect(text).toContain('CAD/CAE');
    // Never dress a closed-form estimate up as an FEA result.
    expect(text).toContain('No stress, deflection, or tolerance');
  });

  it('recomputes the estimate and the agent banner when the primitive changes', async () => {
    const view = await bootShell();
    const kindGroup = view.container.querySelector<HTMLElement>('[aria-label="Primitive kind"]')!;
    const cylinder = Array.from(kindGroup.querySelectorAll('button')).find((b) => b.textContent === 'Cylinder')!;
    fireEvent.click(cylinder);

    // Default cylinder is r = 20, h = 40: V = πr²h = 16000π, envelope 40³.
    await waitFor(() => {
      expect(rail(view.container).querySelector('.inspector-estimates')?.textContent).toContain(
        '50265.48 mm³ (50.265 cm³)',
      );
    });
    expect(rail(view.container).querySelector('.inspector-estimates')?.textContent).toContain(
      '40.00 × 40.00 × 40.00 mm',
    );

    // The agent's active-geometry banner tracks the same switch, so the
    // conversation never quotes geometry the workbench is not holding.
    const context = viewport(view.container).querySelector<HTMLElement>('.chat-panel-context');
    expect(context!.textContent).toContain('cylinder');
    expect(context!.textContent).toContain('envelope 40.00 × 40.00 × 40.00 mm');
    expect(context!.textContent).toContain('volume 50265.48 mm³ (50.265 cm³)');
  });
});

describe('agent composer and geometry context', () => {
  it('shows the active OpenCascade primitive, dimensions, and nominal volume', async () => {
    const view = await bootShell();
    const context = viewport(view.container).querySelector<HTMLElement>('.chat-panel-context');
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
    const textarea = viewport(view.container).querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    expect(textarea.value).toBe('');

    const starter = viewport(view.container).querySelector<HTMLButtonElement>('button[data-starter="size-shaft"]')!;
    fireEvent.click(starter);

    expect(textarea.value).toContain('shaft spacer');
    expect(textarea.value).toContain('25 mm diameter by 120 mm long');
    // Seeding is not sending: no request may leave the machine.
    expect(fetch).not.toHaveBeenCalled();
  });

  it('keeps the documented composer selectors in the viewport', async () => {
    const view = await bootShell();
    const main = viewport(view.container);

    expect(main.querySelector('select#chat-provider-select')).toBeTruthy();
    expect(main.querySelector('input#chat-model-input[type="text"]')).toBeTruthy();
    expect(main.querySelector('textarea.chat-panel-prompt')).toBeTruthy();
    expect(main.querySelector('textarea.chat-panel-prompt')!.getAttribute('placeholder'))
      .toContain('Ask a short question about the current primitive');
    expect(main.querySelector('button.chat-panel-send')!.textContent).toBe('Send');
    expect(main.querySelector('button.chat-panel-clear')!.textContent).toBe('Clear');
    expect(main.querySelector('.chat-panel-empty')).toBeTruthy();

    // The consent checkbox only exists for a remote provider, and the first
    // select / first textarea must still be the provider and the prompt.
    const select = main.querySelector('select')!;
    expect(select.id).toBe('chat-provider-select');
    expect(main.querySelector('textarea')).toBe(main.querySelector('textarea.chat-panel-prompt'));
  });

  it('requires explicit remote consent before a remote provider can send', async () => {
    const view = await bootShell();
    const main = viewport(view.container);

    fireEvent.change(main.querySelector<HTMLSelectElement>('#chat-provider-select')!, {
      target: { value: 'openai' },
    });
    await waitFor(() => {
      expect(main.querySelector('input[type="checkbox"]')).toBeTruthy();
    });

    const textarea = main.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    fireEvent.change(textarea, { target: { value: 'estimate the envelope' } });

    // No model and no consent yet — the send stays locked.
    expect(main.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(true);

    fireEvent.change(main.querySelector<HTMLInputElement>('#chat-model-input')!, {
      target: { value: 'gpt-4o-mini' },
    });
    await waitFor(() => {
      expect(main.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(true);
    });

    fireEvent.click(main.querySelector<HTMLInputElement>('input[type="checkbox"]')!);
    await waitFor(() => {
      expect(main.querySelector<HTMLButtonElement>('button.chat-panel-send')!.disabled).toBe(false);
    });
    expect(fetch).not.toHaveBeenCalled();
  });

  it('lets the operator configure or clear provider API keys from the API Keys drawer', async () => {
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
    const main = viewport(view.container);

    // OpenRouter starts unconfigured in the dropdown
    const openrouterOption = main.querySelector<HTMLOptionElement>(
      'select#chat-provider-select option[value="openrouter"]',
    )!;
    expect(openrouterOption.disabled).toBe(true);

    // Toggle API Keys drawer open
    const toggle = main.querySelector<HTMLButtonElement>('button.chat-panel-settings-toggle')!;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');

    const settingsSection = main.querySelector<HTMLElement>('.chat-panel-settings')!;
    expect(settingsSection).toBeTruthy();

    const keyInput = settingsSection.querySelector<HTMLInputElement>(
      'input[aria-label="OpenRouter API key"]',
    )!;
    fireEvent.change(keyInput, { target: { value: 'sk-or-test-key-123456' } });

    const form = keyInput.closest('form')!;
    fireEvent.submit(form);

    await waitFor(() => {
      expect(mocks.postSettings).toHaveBeenCalledWith(
        expect.objectContaining({ baseUrl: 'http://127.0.0.1:8765' }),
        { provider: 'openrouter', api_key: 'sk-or-test-key-123456' },
      );
    });

    // Secret is cleared from input state and OpenRouter is now enabled in the dropdown
    await waitFor(() => {
      expect(keyInput.value).toBe('');
      expect(
        main.querySelector<HTMLOptionElement>(
          'select#chat-provider-select option[value="openrouter"]',
        )!.disabled,
      ).toBe(false);
    });
  });
});
