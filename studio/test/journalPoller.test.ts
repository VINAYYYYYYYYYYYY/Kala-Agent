// Tests for the journal poller.
//
// The poller must:
//   - read a bounded page right after it starts,
//   - poll every second while a turn is running and every five when idle,
//   - back off exponentially on failures and never wait longer than 30 s,
//   - stop polling entirely while the document is hidden,
//   - ask for the events after the last sequence it has,
//   - verify the chain after a new design and every 30 s of idle time,
//   - cancel its timer and its request on stop, and survive a
//     stop/start cycle (React StrictMode's double mount).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  backoffDelay,
  createJournalPoller,
  POLL_BUSY_MS,
  POLL_IDLE_MS,
  type JournalPollerOptions,
} from '../src/state/journalPoller';
import { getState, resetStore } from '../src/state/store';
import type { JournalPage, StudioConnection } from '../src/types/studio';
import {
  BRACKET_DESIGN_ID,
  bracketJournalB2Page,
  bracketJournalPage,
  bracketJournalVerified,
} from './designFixtures';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'unit-test-token-aaaaaaaaaaaaaaaaaaaaaaaa',
  mode: 'desktop',
};

interface Recorded {
  url: string;
  signal: AbortSignal | null;
}

let recorded: Recorded[] = [];
let journalResponder: () => Response = () =>
  new Response(JSON.stringify({ ...bracketJournalPage, has_more: false }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
let verifyResponder: () => Response = () =>
  new Response(JSON.stringify(bracketJournalVerified), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });

function stubFetch(): void {
  vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === 'string' ? input : input.toString();
    recorded.push({ url, signal: init?.signal ?? null });
    if (url.includes('/journal/verify')) return Promise.resolve(verifyResponder());
    return Promise.resolve(journalResponder());
  });
}

function baseOptions(overrides: Partial<JournalPollerOptions> = {}): JournalPollerOptions {
  return {
    connection,
    getAfter: () => 0,
    isBusy: () => false,
    getLastDesignId: () => null,
    isHidden: () => false,
    ...overrides,
  };
}

async function advance(ms: number): Promise<void> {
  await vi.advanceTimersByTimeAsync(ms);
}

beforeEach(() => {
  resetStore();
  recorded = [];
  vi.useFakeTimers();
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('the poller cadence', () => {
  it('reads a page as soon as it starts', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(recorded.some((call) => call.url.includes('/api/studio/journal?'))).toBe(true);
    expect(poller.__test?.pollCount()).toBe(1);
    poller.stop();
  });

  it('polls every second while a turn is running', async () => {
    const poller = createJournalPoller(baseOptions({ isBusy: () => true }));
    poller.start();
    await advance(1);
    expect(poller.__test?.nextDelayMs()).toBe(POLL_BUSY_MS);
    await advance(POLL_BUSY_MS);
    expect(poller.__test?.pollCount()).toBe(2);
    await advance(POLL_BUSY_MS);
    expect(poller.__test?.pollCount()).toBe(3);
    poller.stop();
  });

  it('polls every five seconds while the studio is idle', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(poller.__test?.nextDelayMs()).toBe(POLL_IDLE_MS);
    await advance(POLL_IDLE_MS);
    expect(poller.__test?.pollCount()).toBe(2);
    poller.stop();
  });

  it('asks for the events after the last sequence it has', async () => {
    const poller = createJournalPoller(baseOptions({ getAfter: () => 2 }));
    poller.start();
    await advance(1);
    const call = recorded.find((entry) => entry.url.includes('/api/studio/journal?'));
    expect(call?.url).toContain('after=2');
    expect(call?.url).toContain('limit=200');
    poller.stop();
  });

  it('drains the rest of the journal without waiting for the next cadence', async () => {
    let pages = 0;
    journalResponder = () => {
      pages += 1;
      return new Response(JSON.stringify({ ...bracketJournalPage, has_more: pages === 1 }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    };
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    // The first page said there was more, so the next page is read right
    // away instead of after the five-second idle cadence.
    expect(poller.__test?.pollCount()).toBe(2);
    expect(poller.__test?.nextDelayMs()).toBe(POLL_IDLE_MS);
    poller.stop();
  });
});

describe('the poller backoff', () => {
  it('waits longer after every failure, up to thirty seconds', async () => {
    journalResponder = () =>
      new Response(JSON.stringify({ code: 'journal_unavailable', message: 'no journal', details: [] }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      });
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(poller.__test?.backoffStep()).toBe(1);
    expect(poller.__test?.nextDelayMs()).toBe(1_000);
    await advance(1_000);
    expect(poller.__test?.backoffStep()).toBe(2);
    expect(poller.__test?.nextDelayMs()).toBe(2_000);
    await advance(2_000);
    expect(poller.__test?.backoffStep()).toBe(3);
    expect(poller.__test?.nextDelayMs()).toBe(4_000);
    poller.stop();
  });

  it('never backs off for longer than thirty seconds', () => {
    expect(backoffDelay(1)).toBe(1_000);
    expect(backoffDelay(2)).toBe(2_000);
    expect(backoffDelay(3)).toBe(4_000);
    expect(backoffDelay(4)).toBe(8_000);
    expect(backoffDelay(5)).toBe(16_000);
    expect(backoffDelay(6)).toBe(30_000);
    expect(backoffDelay(20)).toBe(30_000);
  });

  it('returns to the normal cadence after a successful poll', async () => {
    let fail = true;
    journalResponder = () =>
      fail
        ? new Response(JSON.stringify({ code: 'journal_unavailable', message: 'no', details: [] }), {
            status: 500,
            headers: { 'Content-Type': 'application/json' },
          })
        : new Response(JSON.stringify({ ...bracketJournalPage, has_more: false }), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          });
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(poller.__test?.backoffStep()).toBe(1);
    fail = false;
    await advance(1_000);
    expect(poller.__test?.backoffStep()).toBe(0);
    expect(poller.__test?.nextDelayMs()).toBe(POLL_IDLE_MS);
    poller.stop();
  });
});

describe('the hidden tab', () => {
  it('does not poll while the document is hidden', async () => {
    let hidden = true;
    const poller = createJournalPoller(baseOptions({ isHidden: () => hidden }));
    poller.start();
    await advance(1);
    expect(recorded).toHaveLength(0);
    expect(poller.__test?.pollCount()).toBe(0);
    // It keeps its place and resumes as soon as the tab is visible again.
    hidden = false;
    await advance(POLL_IDLE_MS);
    expect(poller.__test?.pollCount()).toBe(1);
    poller.stop();
  });
});

describe('the chain verification', () => {
  it('verifies the chain after a new design', async () => {
    const poller = createJournalPoller(
      baseOptions({ getLastDesignId: () => 'a'.repeat(64) }),
    );
    poller.start();
    await advance(1);
    expect(poller.__test?.verifyCount()).toBe(1);
    expect(recorded.some((call) => call.url.includes('/journal/verify'))).toBe(true);
    // The same design does not trigger a second verification.
    await advance(POLL_IDLE_MS);
    expect(poller.__test?.verifyCount()).toBe(1);
    poller.stop();
  });

  it('verifies the chain every thirty idle seconds', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    const first = poller.__test?.verifyCount() ?? 0;
    await advance(10_000);
    expect(poller.__test?.verifyCount()).toBe(first);
    await advance(25_000);
    expect(poller.__test?.verifyCount()).toBe(first + 1);
    poller.stop();
  });

  it('records the verdict in the store', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    await advance(1);
    const state = getState();
    expect(state.sessionJournal.events.length).toBeGreaterThan(0);
    expect(state.sessionJournal.chainStatus).toBe('verified');
    expect(state.sessionJournal.headDigest).toBe(bracketJournalVerified.head_digest);
    poller.stop();
  });
});

describe('the design wake', () => {
  it('reads the backend page immediately when a design lands', async () => {
    let designId: string | null = null;
    let page: JournalPage = { events: [], next_after: 0, has_more: false, limit: 200 };
    journalResponder = () =>
      new Response(JSON.stringify(page), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    const poller = createJournalPoller(
      baseOptions({ getLastDesignId: () => designId }),
    );
    poller.start();
    await advance(1);
    expect(getState().sessionJournal.events).toHaveLength(0);
    expect(poller.__test?.nextDelayMs()).toBe(POLL_IDLE_MS);

    // The design is already journalled on the backend when the store gets
    // it. The idle cadence must not leave the ledger with local rows only.
    designId = BRACKET_DESIGN_ID;
    page = {
      events: [...bracketJournalB2Page.events],
      next_after: bracketJournalB2Page.next_after,
      has_more: false,
      limit: 200,
    };
    poller.wake();
    await advance(1);

    const state = getState();
    expect(state.sessionJournal.events).toHaveLength(10);
    expect(state.sessionJournal.lastSequence).toBe(10);
    expect(state.sessionJournal.chainStatus).toBe('verified');
    expect(poller.__test?.pollCount()).toBe(2);
    poller.stop();
  });

  it('honours a wake that arrives while a poll is in flight', async () => {
    let release: (() => void) | null = null;
    let calls = 0;
    vi.unstubAllGlobals();
    vi.stubGlobal('fetch', (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input.toString();
      calls += 1;
      if (url.includes('/journal/verify')) {
        return Promise.resolve(
          new Response(JSON.stringify(bracketJournalVerified), {
            status: 200,
            headers: { 'Content-Type': 'application/json' },
          }),
        );
      }
      if (calls === 1) {
        return new Promise((resolve) => {
          release = () =>
            resolve(
              new Response(JSON.stringify({ events: [], next_after: 0, has_more: false, limit: 200 }), {
                status: 200,
                headers: { 'Content-Type': 'application/json' },
              }),
            );
        });
      }
      return Promise.resolve(
        new Response(JSON.stringify(bracketJournalB2Page), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        }),
      );
    });

    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(release).not.toBeNull();

    poller.wake();
    release!();
    await advance(1);
    await advance(1);

    expect(getState().sessionJournal.events).toHaveLength(10);
    poller.stop();
  });
});

describe('the poller lifecycle', () => {
  it('cancels its timer and its request on stop', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(poller.__test?.pendingTimer()).toBe(true);
    poller.stop();
    expect(poller.__test?.pendingTimer()).toBe(false);
    expect(poller.isRunning()).toBe(false);
    const polls = poller.__test?.pollCount() ?? 0;
    await advance(30_000);
    expect(poller.__test?.pollCount()).toBe(polls);
  });

  it('is safe to start twice and to restart after a stop', async () => {
    const poller = createJournalPoller(baseOptions());
    poller.start();
    poller.start();
    await advance(1);
    const polls = poller.__test?.pollCount() ?? 0;
    expect(polls).toBe(1);

    // The StrictMode pattern: the first mount is torn down and a second one
    // is created. The second poller must not double the polling.
    poller.stop();
    poller.start();
    await advance(1);
    expect(poller.__test?.pollCount()).toBe(polls + 1);
    poller.stop();
  });

  it('aborts the request it has in flight when it stops', async () => {
    vi.unstubAllGlobals();
    const signals: AbortSignal[] = [];
    vi.stubGlobal('fetch', (_input: RequestInfo | URL, init?: RequestInit) => {
      if (init?.signal) signals.push(init.signal);
      return new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => {
          reject(new DOMException('aborted', 'AbortError'));
        });
      });
    });
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    expect(signals[0]?.aborted).toBe(false);
    poller.stop();
    expect(signals[0]?.aborted).toBe(true);
  });

  it('records a polling failure without inventing a chain verdict', async () => {
    journalResponder = () =>
      new Response(JSON.stringify({ code: 'journal_unavailable', message: 'no journal', details: [] }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      });
    verifyResponder = () =>
      new Response(JSON.stringify({ code: 'journal_unavailable', message: 'no journal', details: [] }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      });
    const poller = createJournalPoller(baseOptions());
    poller.start();
    await advance(1);
    const state = getState();
    expect(state.sessionJournal.lastError).toContain('journal_unavailable');
    // No verification answered, so no verdict is claimed.
    expect(state.sessionJournal.chainStatus).toBe('unknown');
    expect(state.sessionJournal.headDigest).toBeNull();
    poller.stop();
  });
});
