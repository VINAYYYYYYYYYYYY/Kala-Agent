// @vitest-environment happy-dom
// Integration coverage for the Options drawer details that are easy to
// regress: the card-header notes must come from the live store (never the
// Stitch literal "PORT 8765"), and switching cards must scroll the selected
// card to the top of the drawer viewport rather than leaving it above the
// fold.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render } from '@testing-library/react';
import { OptionsDrawer } from '../src/components/options/OptionsDrawer';
import { dispatch, resetStore, THEME_STORAGE_KEY } from '../src/state/store';
import type { OptionsCard } from '../src/types/studio';

const originalScrollIntoView = Element.prototype.scrollIntoView;

function renderDrawer(card: OptionsCard, onCardChange = () => {}) {
  return render(
    <OptionsDrawer open card={card} onCardChange={onCardChange} onClose={() => {}} />,
  );
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
  Element.prototype.scrollIntoView = originalScrollIntoView;
  vi.unstubAllGlobals();
});

describe('OptionsDrawer drafted sheet', () => {
  it('renders four card headers with icons and four corner ticks each', () => {
    const view = renderDrawer('inspector');
    expect(view.container.querySelectorAll('.options-card-icon svg')).toHaveLength(4);
    expect(view.container.querySelectorAll('.options-card-tick')).toHaveLength(16);
    for (const corner of ['tl', 'tr', 'bl', 'br']) {
      expect(view.container.querySelectorAll(`.options-card-tick-${corner}`)).toHaveLength(4);
    }
  });

  it('does not duplicate the inner Inspector and Export headings', () => {
    const view = renderDrawer('inspector');
    expect(view.container.querySelector('#options-card-inspector h2')).toBeNull();
    expect(view.container.querySelector('#options-card-export h2')).toBeNull();
    expect(view.container.querySelector('#options-card-inspector .inspector')).toBeTruthy();
    expect(view.container.querySelector('#options-card-export .export-panel')).toBeTruthy();
  });

  it('prints only real data in the footer title block', () => {
    dispatch({
      type: 'connection/ready',
      connection: {
        baseUrl: 'http://127.0.0.1:9911',
        token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
        mode: 'browser-manual',
      },
    });
    act(() => {
      dispatch({
        type: 'server/providers',
        providers: [],
        health: 'ok',
        healthMessage: 'ok',
        version: '9.8.7',
        stepAvailable: true,
      });
    });
    const view = renderDrawer('connection');
    const block = view.container.querySelector('.options-drawer-title-block')!;
    const text = block.textContent ?? '';
    expect(text).toContain('SHEET ID');
    expect(text).toContain('KALA-UI-OPT-04');
    expect(text).toContain('DRAWING TITLE');
    expect(text).toContain('Options & Engineering Drawer');
    expect(text).toContain('SCALE');
    expect(text).toContain('NTS');
    expect(text).toContain('BACKEND');
    expect(text).toContain('v9.8.7');
    expect(text).toContain('CONNECTION ORIGIN');
    expect(text).toContain('http://127.0.0.1:9911');
    expect(text).not.toContain('1:1 NOMINAL');
  });
});

describe('OptionsDrawer card notes', () => {
  it('reports the real loopback port instead of the Stitch literal', () => {
    dispatch({
      type: 'connection/ready',
      connection: {
        baseUrl: 'http://127.0.0.1:9911',
        token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
        mode: 'browser-manual',
      },
    });
    const view = renderDrawer('connection');
    expect(view.getByText('PORT 9911')).toBeTruthy();
    expect(view.queryByText('PORT 8765')).toBeNull();
  });

  it('says NO LOOPBACK before a connection exists', () => {
    const view = renderDrawer('connection');
    expect(view.getByText('NO LOOPBACK')).toBeTruthy();
  });

  it('follows the backend STEP capability, with an explicit unknown state', () => {
    const view = renderDrawer('export');
    expect(view.getByText('STEP —')).toBeTruthy();
    act(() => {
      dispatch({
        type: 'server/providers',
        providers: [],
        health: 'ok',
        healthMessage: 'ok',
        version: '0.1.0',
        stepAvailable: true,
      });
    });
    expect(view.getByText('ISO 10303 READY')).toBeTruthy();
  });
});

describe('OptionsDrawer card navigation', () => {
  it('scrolls the selected card to the top of the drawer viewport', () => {
    const scrollSpy = vi.fn();
    Element.prototype.scrollIntoView = scrollSpy;
    const view = renderDrawer('inspector');
    expect(scrollSpy).toHaveBeenCalledWith({ block: 'start' });
    scrollSpy.mockClear();
    act(() => {
      view.rerender(
        <OptionsDrawer open card="providers" onCardChange={() => {}} onClose={() => {}} />,
      );
    });
    expect(scrollSpy).toHaveBeenCalledWith({ block: 'start' });
  });
});
