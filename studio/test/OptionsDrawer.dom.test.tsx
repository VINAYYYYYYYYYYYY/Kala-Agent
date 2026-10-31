// @vitest-environment happy-dom
// Settings dialog (the old Options drawer).
//
// The app-feel wave turned the four-card Options dashboard into a focused
// Settings dialog with a left nav and one section at a time. These tests
// pin the structure that replaced it: every nav item reveals its section,
// the theme switch and the reconnect action still work, Escape and the
// close button close it, and the primitive inspector and export are gone.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { OptionsDrawer } from '../src/components/options/OptionsDrawer';
import { dispatch, getState, resetStore, THEME_STORAGE_KEY } from '../src/state/store';
import type {
  CadProviderRecord,
  IntegrationRecord,
  OptionsCard,
  ProviderRecord,
} from '../src/types/studio';

const CONNECTION = {
  baseUrl: 'http://127.0.0.1:9911',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'browser-manual' as const,
};

const PROVIDERS: ReadonlyArray<ProviderRecord> = [
  { id: 'local', name: 'Local Deterministic Helper', configured: true, notes: '' },
  { id: 'openai', name: 'OpenAI (Chat Completions)', configured: false, notes: 'API-key route.' },
];

const INTEGRATIONS: ReadonlyArray<IntegrationRecord> = [
  {
    id: 'freecad',
    name: 'FreeCAD',
    category: 'cad',
    platforms: ['linux', 'open-source'],
    status: 'available_via_neutral_file',
    capabilities: ['step', 'stl'],
    notes: '',
  },
];

const CAD_PROVIDERS: ReadonlyArray<CadProviderRecord> = [
  {
    id: 'opencascade',
    name: 'OpenCascade',
    available: true,
    version: '7.9.3',
    reason: null,
    measured_by: 'OpenCascade',
    mode: 'in-process',
  },
  {
    id: 'freecad',
    name: 'FreeCAD',
    available: false,
    version: null,
    reason: 'FreeCAD binary not found on PATH',
    measured_by: 'FreeCAD',
    mode: 'subprocess',
  },
];

function renderDrawer(
  card: OptionsCard,
  onCardChange: (card: OptionsCard) => void = () => {},
  onClose: () => void = () => {},
) {
  return render(
    <OptionsDrawer open card={card} onCardChange={onCardChange} onClose={onClose} />,
  );
}

function seedServer(): void {
  act(() => {
    dispatch({ type: 'connection/ready', connection: CONNECTION });
    dispatch({
      type: 'server/providers',
      providers: PROVIDERS,
      health: 'ok',
      healthMessage: 'ok',
      version: '9.8.7',
      stepAvailable: true,
    });
    dispatch({ type: 'server/integrations', integrations: INTEGRATIONS });
    dispatch({ type: 'server/cad-providers', providers: CAD_PROVIDERS });
  });
}

beforeEach(() => {
  resetStore();
  try {
    localStorage.removeItem(THEME_STORAGE_KEY);
  } catch {
    // environment without localStorage
  }
  vi.stubGlobal('fetch', vi.fn(() => Promise.reject(new Error('no network in unit test'))));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('Settings dialog shell', () => {
  it('is a labelled modal titled Settings', () => {
    const view = renderDrawer('workspace');
    const dialog = view.getByRole('dialog');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(dialog.getAttribute('aria-labelledby')).toBe('settings-title');
    expect(view.getByRole('heading', { name: 'Settings' })).toBeTruthy();
  });

  it('lists the four sections as a vertical tablist', () => {
    const view = renderDrawer('workspace');
    const tabs = view.getAllByRole('tab');
    expect(tabs.map((tab) => tab.textContent?.replace(/\d+/g, '').trim())).toEqual([
      'Workspace',
      'Models & keys',
      'CAD',
      'Connection',
    ]);
    expect(tabs[0]!.getAttribute('aria-selected')).toBe('true');
    expect(tabs[1]!.getAttribute('aria-selected')).toBe('false');

    const tablist = view.getByRole('tablist');
    expect(tablist.getAttribute('aria-orientation')).toBe('vertical');
    expect(tablist.getAttribute('aria-label')).toBe('Settings sections');
  });

  it('shows one section at a time and switches it from the nav', () => {
    const onCardChange = vi.fn();
    const view = renderDrawer('workspace', onCardChange);
    expect(view.getByRole('tabpanel', { name: 'Workspace' })).toBeTruthy();
    expect(view.queryByRole('tabpanel', { name: 'Models & keys' })).toBeNull();

    fireEvent.click(view.getByRole('tab', { name: /Models & keys/ }));
    expect(onCardChange).toHaveBeenCalledWith('providers');

    view.rerender(
      <OptionsDrawer open card="providers" onCardChange={onCardChange} onClose={() => {}} />,
    );
    expect(view.getByRole('tabpanel', { name: 'Models & keys' })).toBeTruthy();
    expect(view.queryByRole('tabpanel', { name: 'Workspace' })).toBeNull();
  });

  it('shows the selected section for every nav item', () => {
    const sections: ReadonlyArray<[OptionsCard, string]> = [
      ['workspace', 'Workspace'],
      ['providers', 'Models & keys'],
      ['cad', 'CAD'],
      ['connection', 'Connection'],
    ];
    const view = renderDrawer('workspace');
    for (const [id, label] of sections) {
      view.rerender(
        <OptionsDrawer open card={id} onCardChange={() => {}} onClose={() => {}} />,
      );
      expect(view.getByRole('heading', { name: label })).toBeTruthy();
      expect(view.getByRole('tabpanel', { name: label })).toBeTruthy();
      expect(view.getByRole('tabpanel').getAttribute('data-section')).toBe(id);
    }
  });

  it('moves the tab selection with the arrow keys', () => {
    const onCardChange = vi.fn();
    const view = renderDrawer('workspace', onCardChange);
    fireEvent.keyDown(view.getByRole('tablist'), { key: 'ArrowDown' });
    expect(onCardChange).toHaveBeenCalledWith('providers');
  });

  it('has no inspector or export inside', () => {
    const view = renderDrawer('providers');
    expect(view.container.querySelector('.inspector')).toBeNull();
    expect(view.container.querySelector('.export-panel')).toBeNull();
    expect(view.container.textContent).not.toContain('Parametric Inspector');
    expect(view.container.textContent).not.toContain('Export design');
  });

  it('closes on Escape and from the labelled close button', () => {
    const onClose = vi.fn();
    const view = renderDrawer('workspace', () => {}, onClose);

    const close = view.getByRole('button', { name: 'Close settings' });
    fireEvent.click(close);
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('returns focus to the opener when it closes', () => {
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();

    const view = renderDrawer('workspace');
    expect(document.activeElement).toBe(
      view.getByRole('button', { name: 'Close settings' }),
    );

    view.rerender(
      <OptionsDrawer open={false} card="workspace" onCardChange={() => {}} onClose={() => {}} />,
    );
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('renders nothing while closed', () => {
    const view = render(
      <OptionsDrawer open={false} card="workspace" onCardChange={() => {}} onClose={() => {}} />,
    );
    expect(view.container.querySelector('[role="dialog"]')).toBeNull();
  });
});

describe('Settings > Workspace', () => {
  it('switches the workspace theme and marks the live choice', () => {
    const view = renderDrawer('workspace');
    const group = view.getByRole('group', { name: 'Workspace theme' });
    const vellum = Array.from(group.querySelectorAll('button')).find(
      (button) => button.textContent === 'Vellum',
    )!;
    const slate = Array.from(group.querySelectorAll('button')).find(
      (button) => button.textContent === 'Slate',
    )!;
    expect(vellum.getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(slate);
    expect(getState().theme).toBe('slate');
    expect(slate.getAttribute('aria-pressed')).toBe('true');
    expect(vellum.getAttribute('aria-pressed')).toBe('false');

    fireEvent.click(vellum);
    expect(getState().theme).toBe('vellum');
  });
});

describe('Settings > Models & keys', () => {
  it('keeps the honesty line and lists the providers', () => {
    seedServer();
    const view = renderDrawer('providers');
    const text = view.container.textContent ?? '';
    expect(text).toContain('Keys are sent once to the local Studio backend');
    expect(text).toContain('never echoed back to the browser');
    expect(text).toContain('never enter localStorage');
    const rows = view.container.querySelectorAll('.panel-list-item');
    expect(rows.length).toBeGreaterThanOrEqual(2);
    expect(text).toContain('Local Deterministic Helper');
    expect(text).toContain('OpenAI (Chat Completions)');
  });
});

describe('Settings > CAD', () => {
  it('shows the integration registry and the live CAD providers read-only', () => {
    seedServer();
    const view = renderDrawer('cad');
    const text = view.container.textContent ?? '';
    expect(text).toContain('Integration registry');
    expect(text).toContain('FreeCAD');
    expect(text).toContain('Neutral file');
    expect(text).toContain('Live CAD providers');
    expect(text).toContain('OpenCascade');
    expect(text).toContain('Version 7.9.3');
    expect(text).toContain('Available');
    expect(text).toContain('FreeCAD binary not found on PATH');
    expect(text).toContain('Unavailable');

    const live = view.getByLabelText('Live CAD providers');
    expect(live.querySelectorAll('select, input, button, textarea')).toHaveLength(0);
  });

  it('says so before the backend has reported any CAD provider', () => {
    const view = renderDrawer('cad');
    expect(view.container.textContent).toContain(
      'The backend has not reported its CAD providers yet.',
    );
  });
});

describe('Settings > Connection', () => {
  it('shows the real connection facts and the token note', () => {
    seedServer();
    const view = renderDrawer('connection');
    const text = view.container.textContent ?? '';
    expect(text).toContain('Status');
    expect(text).toContain('Connected');
    expect(text).toContain('Origin');
    expect(text).toContain(CONNECTION.baseUrl);
    expect(text).toContain('Backend');
    expect(text).toContain('v9.8.7');
    expect(text).toContain('Health');
    expect(text).toContain('In memory only (not displayed)');
    expect(text).toContain('never written to localStorage');
  });

  it('requests a real reconnect when Retry is pressed', () => {
    seedServer();
    const view = renderDrawer('connection');
    const before = getState().connectionRetryToken;
    fireEvent.click(view.getByRole('button', { name: /Reconnect \/ Retry Bridge/i }));
    expect(getState().connectionRetryToken).toBe(before + 1);
    expect(
      getState().journal.some((entry) =>
        entry.message.includes('Reconnect / retry requested'),
      ),
    ).toBe(true);
  });
});
