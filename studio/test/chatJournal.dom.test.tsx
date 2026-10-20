// @vitest-environment happy-dom
// P2b chat-journal coverage: taped starter tabs, Enter vs Shift+Enter,
// real connection stamps, ledger rendering, sticky autoscroll, and the
// honest hash-chain / SESSION LOG stamp logic.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, createEvent, fireEvent, render, waitFor } from '@testing-library/react';

const mocks = vi.hoisted(() => ({
  postChat: vi.fn(),
}));

vi.mock('../src/api/client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/api/client')>();
  return {
    ...actual,
    postChat: mocks.postChat,
  };
});

import { ChatPanel } from '../src/components/chat/ChatPanel';
import { JournalPanel } from '../src/components/journal/JournalPanel';
import type { JournalPanelProps } from '../src/components/journal/JournalPanel';
import {
  dispatch,
  getState,
  resetStore,
} from '../src/state/store';
import type { JournalEntry } from '../src/types/studio';

const BASE_URL = 'http://127.0.0.1:8765';

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
  mocks.postChat.mockReset();
  resetStore();
});

function seedProviders(version = '0.1.0'): void {
  dispatch({
    type: 'connection/ready',
    connection: {
      baseUrl: BASE_URL,
      token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
      mode: 'desktop',
    },
  });
  dispatch({
    type: 'server/providers',
    providers: [
      { id: 'local', name: 'Local helper', configured: true, notes: '' },
      { id: 'openai', name: 'OpenAI', configured: true, notes: '' },
    ],
    health: 'ok',
    healthMessage: 'ok',
    version,
    stepAvailable: true,
  });
}

function entry(overrides: Partial<JournalEntry> & Pick<JournalEntry, 'id' | 'kind' | 'message'>): JournalEntry {
  return { ts: Date.UTC(2026, 0, 2, 12, 34, 56), ...overrides };
}

const readyConnection: JournalPanelProps['connectionStatus'] = {
  kind: 'ready',
  connection: { baseUrl: BASE_URL, token: 'x'.repeat(32), mode: 'desktop' },
};

describe('starter tabs', () => {
  it('renders the four taped paper tabs and only seeds the composer', () => {
    const view = render(<ChatPanel />);
    const starters = Array.from(
      view.container.querySelectorAll<HTMLButtonElement>('button.chat-panel-starter'),
    );
    expect(starters.map((button) => button.textContent)).toEqual([
      'Size L-Bracket',
      'Size Stepped Shaft',
      'Housing Envelope',
      'STEP Handoff',
    ]);

    const shaft = view.container.querySelector<HTMLButtonElement>('button[data-starter="size-shaft"]')!;
    fireEvent.click(shaft);

    const textarea = view.container.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    expect(textarea.value).toContain('stepped shaft');
    expect(textarea.value).toContain('shaft spacer');
    expect(textarea.value).toContain('25 mm diameter by 120 mm long');

    // Seeding must never auto-send or create a thread entry.
    expect(mocks.postChat).not.toHaveBeenCalled();
    expect(getState().chat.thread).toHaveLength(0);
    expect(getState().journal).toHaveLength(0);
  });
});

describe('composer keyboard handling', () => {
  it('sends on Enter and keeps the newline on Shift+Enter', async () => {
    seedProviders();
    mocks.postChat.mockResolvedValue({
      text: 'The nominal envelope is 40 × 20 × 30 mm.',
      provider: 'local',
      model: null,
      offline: true,
    });

    const view = render(<ChatPanel />);
    const provider = view.container.querySelector<HTMLSelectElement>('#chat-provider-select')!;
    fireEvent.change(provider, { target: { value: 'local' } });

    const textarea = view.container.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    fireEvent.change(textarea, { target: { value: 'Size the active bracket.' } });

    const send = view.container.querySelector<HTMLButtonElement>('button.chat-panel-send')!;
    await waitFor(() => expect(send.disabled).toBe(false));

    const shiftEnter = createEvent.keyDown(textarea, { key: 'Enter', shiftKey: true });
    fireEvent(textarea, shiftEnter);
    expect(shiftEnter.defaultPrevented).toBe(false);
    expect(mocks.postChat).not.toHaveBeenCalled();
    expect(getState().chat.thread).toHaveLength(0);

    fireEvent.keyDown(textarea, { key: 'Enter' });
    await waitFor(() => expect(mocks.postChat).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      expect(getState().chat.thread).toHaveLength(2);
      expect(getState().chat.thread[0]?.role).toBe('user');
      expect(getState().chat.thread[1]?.role).toBe('assistant');
    });
  });

  it('does not send Enter when the provider is not configured', async () => {
    seedProviders();
    const view = render(<ChatPanel />);
    const provider = view.container.querySelector<HTMLSelectElement>('#chat-provider-select')!;
    fireEvent.change(provider, { target: { value: 'local' } });

    const textarea = view.container.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt')!;
    // Empty prompt: Enter must not fire a request even though the provider is ready.
    fireEvent.keyDown(textarea, { key: 'Enter' });
    expect(mocks.postChat).not.toHaveBeenCalled();
  });

  it('pins the thread to the newest turn unless the operator scrolled up', () => {
    seedProviders();
    const view = render(<ChatPanel />);
    const thread = view.container.querySelector<HTMLDivElement>('.chat-panel-thread')!;
    Object.defineProperty(thread, 'scrollHeight', { configurable: true, value: 600 });
    Object.defineProperty(thread, 'clientHeight', { configurable: true, value: 200 });

    // Parked at the bottom: a new response keeps the newest card visible.
    thread.scrollTop = 400;
    fireEvent.scroll(thread);
    act(() => {
      dispatch({
        type: 'chat/response',
        response: { text: 'First answer', provider: 'local', model: null },
      });
    });
    expect(thread.scrollTop).toBe(600);

    // Scrolled up to re-read: a later response must not yank the viewport.
    thread.scrollTop = 0;
    fireEvent.scroll(thread);
    act(() => {
      dispatch({
        type: 'chat/response',
        response: { text: 'Second answer', provider: 'local', model: null },
      });
    });
    expect(thread.scrollTop).toBe(0);
  });
});

describe('connection stamp', () => {
  it('reports the real desktop bridge state', async () => {
    seedProviders();
    const view = render(<ChatPanel />);
    const stamp = view.container.querySelector<HTMLElement>('.chat-panel-connection')!;
    expect(stamp.textContent).toContain('Desktop bridge');
    expect(stamp.closest('.chat-panel')?.getAttribute('data-connection')).toBe('desktop');
  });

  it('reports the real browser fallback and disconnected states', async () => {
    dispatch({
      type: 'connection/ready',
      connection: { baseUrl: BASE_URL, token: 'x'.repeat(32), mode: 'browser-manual' },
    });
    const browser = render(<ChatPanel />);
    expect(browser.container.querySelector('.chat-panel-connection')?.textContent).toContain(
      'Browser fallback',
    );
    cleanup();

    resetStore();
    const disconnected = render(<ChatPanel />);
    expect(disconnected.container.querySelector('.chat-panel-connection')?.textContent).toContain(
      'Disconnected',
    );
    expect(
      disconnected.container.querySelector('.chat-panel')?.getAttribute('data-connection'),
    ).toBe('disconnected');
  });
});

describe('journal ledger', () => {
  it('renders tagged mono rows, real times, and only real digests', () => {
    seedProviders('0.1.0');
    const entries: JournalEntry[] = [
      entry({ id: 'p1', kind: 'preview', message: 'Server preview verified · analytic', digest: '512 bytes' }),
      entry({ id: 'e1', kind: 'export', message: 'Saved bracket.step' }),
    ];
    const view = render(
      <JournalPanel entries={entries} connectionStatus={readyConnection} hashChainStatus="unknown" />,
    );

    expect(view.container.querySelector('.journal-panel-header')?.textContent).toContain(
      'AGENT & KERNEL JOURNAL',
    );

    const rows = view.container.querySelectorAll('.journal-entry');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.getAttribute('data-kind')).toBe('preview');
    expect(rows[0]?.querySelector('.journal-entry-tag')?.textContent).toBe('[PREVIEW]');
    expect(rows[0]?.querySelector('.journal-entry-message')?.textContent).toContain(
      'Server preview verified',
    );
    expect(rows[0]?.querySelector('.journal-entry-digest')?.textContent).toBe('512 bytes');
    // No digest was recorded for the export row: no chip is rendered.
    expect(rows[1]?.querySelector('.journal-entry-digest')).toBeNull();
    expect(rows[0]?.querySelector('time')?.getAttribute('dateTime')).toBe(
      new Date(Date.UTC(2026, 0, 2, 12, 34, 56)).toISOString(),
    );
  });

  it('shows the real loopback origin and backend version in the footer', () => {
    seedProviders('1.4.2');
    const view = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} hashChainStatus="unknown" />,
    );
    const footer = view.container.querySelector('.journal-panel-footer')!;
    expect(footer.textContent).toContain(BASE_URL);
    expect(footer.textContent).toContain('backend v1.4.2');
  });

  it('shows an honest empty state and a neutral SESSION LOG stamp', () => {
    seedProviders('0.1.0');
    const view = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} hashChainStatus="unknown" />,
    );
    expect(view.container.querySelector('.journal-empty')).toBeTruthy();
    const stamp = view.container.querySelector('.journal-panel-stamp')!;
    expect(stamp.textContent).toBe('SESSION LOG');
    expect(stamp.getAttribute('data-chain')).toBe('unknown');
    expect(view.container.textContent).not.toContain('HASH-CHAIN OK');
  });

  it('only claims HASH-CHAIN OK when the backend reports verification', () => {
    seedProviders();
    const unknown = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} hashChainStatus="unknown" />,
    );
    expect(unknown.container.textContent).not.toContain('HASH-CHAIN OK');
    expect(unknown.container.textContent).toContain('SESSION LOG');
    cleanup();

    const verified = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} hashChainStatus="verified" />,
    );
    expect(verified.container.querySelector('.journal-panel-stamp')?.textContent).toBe(
      'HASH-CHAIN OK',
    );
    cleanup();

    const broken = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} hashChainStatus="broken" />,
    );
    expect(broken.container.querySelector('.journal-panel-stamp')?.textContent).toBe(
      'HASH-CHAIN BROKEN',
    );
  });

  it('announces the newest row through a polite live region', async () => {
    seedProviders();
    const view = render(
      <JournalPanel
        entries={[entry({ id: 'a1', kind: 'chat', message: 'Agent · sized the bracket' })]}
        connectionStatus={readyConnection}
        hashChainStatus="unknown"
      />,
    );
    const live = view.container.querySelector<HTMLElement>('.journal-live')!;
    expect(live.getAttribute('aria-live')).toBe('polite');
    await waitFor(
      () => expect(live.textContent).toContain('Agent · sized the bracket'),
      { timeout: 2000 },
    );
  });
});

describe('journal autoscroll stickiness', () => {
  it('sticks to the newest row while pinned and offers a jump when scrolled up', async () => {
    seedProviders();
    const first = entry({ id: 'j1', kind: 'preview', message: 'first row' });
    const second = entry({ id: 'j2', kind: 'preview', message: 'second row' });
    const third = entry({ id: 'j3', kind: 'preview', message: 'third row' });

    const view = render(
      <JournalPanel entries={[first]} connectionStatus={readyConnection} hashChainStatus="unknown" />,
    );
    const list = view.container.querySelector<HTMLOListElement>('.journal-list')!;
    Object.defineProperty(list, 'scrollHeight', { configurable: true, value: 1000 });
    Object.defineProperty(list, 'clientHeight', { configurable: true, value: 200 });

    // Pinned at the bottom: a new row keeps the ledger at the newest line.
    list.scrollTop = 800;
    fireEvent.scroll(list);
    view.rerender(
      <JournalPanel
        entries={[first, second]}
        connectionStatus={readyConnection}
        hashChainStatus="unknown"
      />,
    );
    expect(list.scrollTop).toBeGreaterThan(0);
    expect(view.container.querySelector('.journal-jump')).toBeNull();

    // Scrolled up: appends must not yank the operator back down.
    list.scrollTop = 0;
    fireEvent.scroll(list);
    await waitFor(() => expect(view.container.querySelector('.journal-jump')).toBeTruthy());

    view.rerender(
      <JournalPanel
        entries={[first, second, third]}
        connectionStatus={readyConnection}
        hashChainStatus="unknown"
      />,
    );
    expect(list.scrollTop).toBe(0);
    expect(view.container.querySelector('.journal-jump')).toBeTruthy();

    // Jump control re-pins and the control disappears.
    fireEvent.click(view.container.querySelector<HTMLButtonElement>('.journal-jump')!);
    expect(list.scrollTop).toBeGreaterThan(0);
    await waitFor(() => expect(view.container.querySelector('.journal-jump')).toBeNull());
  });
});
