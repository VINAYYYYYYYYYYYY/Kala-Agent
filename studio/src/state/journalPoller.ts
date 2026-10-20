// The journal poller.
//
// The Studio backend keeps a hash-chained journal of the session. The
// poller reads it in bounded pages and hands each page to the store.
//
// The cadence follows the work:
//   - 1 s while a chat or design request is in flight (a turn is running),
//   - 5 s while the studio is idle,
//   - paused entirely while the document is hidden.
//
// A failing backend backs off exponentially (1 s, 2 s, 4 s, 8 s, 16 s,
// 30 s, 30 s …) so a dead server is not hammered. The chain is verified
// after every new design and every 30 s of idle time; the HASH-CHAIN OK
// stamp is lit only by a real `/journal/verify` response.
//
// The controller owns its timers and its abort controllers: `stop()` cancels
// both, and `start()` after a `stop()` builds a fresh loop, so React
// StrictMode's double mount cannot leave a second poller running.

import {
  getJournal,
  getJournalVerify,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
  StudioTimeoutError,
} from '../api/client';
import type { JournalPage, JournalVerify, StudioConnection } from '../types/studio';
import { JOURNAL_LIMIT_DEFAULT } from '../api/client';
import { dispatch } from './store';

/** The poll cadence while a turn is running. */
export const POLL_BUSY_MS = 1_000;
/** The poll cadence while the studio is idle. */
export const POLL_IDLE_MS = 5_000;
/** How long the chain is left unverified while the studio is idle. */
export const VERIFY_IDLE_MS = 30_000;
/** The first backoff step after a failed poll. */
export const BACKOFF_BASE_MS = 1_000;
/** The longest backoff step. */
export const BACKOFF_MAX_MS = 30_000;
/**
 * How many pages the poller may read back to back while the backend keeps
 * saying "there is more". The cap keeps a broken backend from turning the
 * poller into a tight loop.
 */
export const MAX_DRAIN_PAGES = 20;

export interface JournalPollerOptions {
  connection: StudioConnection;
  /** The sequence the next page should start after. */
  getAfter(): number;
  /** True while a chat or design request is pending. */
  isBusy(): boolean;
  /** The design id of the newest design, so the chain is verified after it. */
  getLastDesignId(): string | null;
  /** True while the document is hidden; polling pauses. */
  isHidden?(): boolean;
  /** The page size to ask for. */
  limit?: number;
}

export interface JournalPoller {
  /** Start polling. Calling it twice is a no-op. */
  start(): void;
  /** Stop polling, cancel the timer, and abort any in-flight request. */
  stop(): void;
  /**
   * Read the journal now instead of waiting for the cadence. Called when a
   * design lands: its provider/build/artifact events are already stored, and
   * the idle cadence must not leave the ledger without them.
   */
  wake(): void;
  isRunning(): boolean;
  /** Test-only hooks. The runtime never calls these. */
  __test?: {
    pollCount(): number;
    verifyCount(): number;
    pendingTimer(): boolean;
    nextDelayMs(): number | null;
    backoffStep(): number;
  };
}

function defaultIsHidden(): boolean {
  if (typeof document === 'undefined') return false;
  return document.hidden === true;
}

function describeFailure(cause: unknown): string {
  if (cause instanceof StudioAbortError) return 'cancelled';
  if (cause instanceof StudioTimeoutError) return `timeout: ${cause.path}`;
  if (cause instanceof StudioAuthError) return `auth: ${cause.message} (${cause.status})`;
  if (cause instanceof StudioApiError) return `${cause.code}: ${cause.message}`;
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'unknown error';
}

/**
 * The backoff for the n-th consecutive failure: 1 s, 2 s, 4 s, 8 s, 16 s,
 * then capped at 30 s.
 */
export function backoffDelay(failures: number): number {
  const step = Math.max(1, Math.floor(failures));
  const delay = BACKOFF_BASE_MS * 2 ** (step - 1);
  return Math.min(delay, BACKOFF_MAX_MS);
}

export function createJournalPoller(options: JournalPollerOptions): JournalPoller {
  const limit = options.limit ?? JOURNAL_LIMIT_DEFAULT;
  const isHidden = options.isHidden ?? defaultIsHidden;

  let running = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let inFlight: AbortController | null = null;
  let failures = 0;
  let polls = 0;
  let verifies = 0;
  let drained = 0;
  let lastVerifyAt = 0;
  let verifiedDesignId: string | null = null;
  let nextDelay: number | null = null;
  let wakeRequested = false;

  function clearTimer(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    nextDelay = null;
  }

  function abortInFlight(): void {
    if (inFlight) {
      inFlight.abort();
      inFlight = null;
    }
  }

  function schedule(delayMs: number): void {
    if (!running) return;
    clearTimer();
    nextDelay = delayMs;
    timer = setTimeout(() => {
      timer = null;
      nextDelay = null;
      void tick();
    }, delayMs);
  }

  function shouldVerify(now: number): boolean {
    const designId = options.getLastDesignId();
    // A new design means new chain events: verify right away.
    if (designId !== null && designId !== verifiedDesignId) return true;
    // Otherwise verify on the idle cadence.
    if (options.isBusy()) return false;
    return now - lastVerifyAt >= VERIFY_IDLE_MS;
  }

  async function runVerify(now: number): Promise<void> {
    const controller = new AbortController();
    inFlight = controller;
    try {
      const verify: JournalVerify = await getJournalVerify(options.connection, controller.signal);
      verifies += 1;
      lastVerifyAt = now;
      verifiedDesignId = options.getLastDesignId();
      dispatch({ type: 'journal/server-verify', verify });
    } catch (cause) {
      if (!controller.signal.aborted) {
        dispatch({ type: 'journal/server-error', message: describeFailure(cause) });
      }
    } finally {
      if (inFlight === controller) inFlight = null;
    }
  }

  async function tick(): Promise<void> {
    if (!running) return;
    const now = Date.now();
    // A hidden document is not polled at all; the poller wakes up on the
    // next scheduled tick (or when the tab becomes visible again).
    if (isHidden()) {
      schedule(options.isBusy() ? POLL_BUSY_MS : POLL_IDLE_MS);
      return;
    }

    const controller = new AbortController();
    inFlight = controller;
    polls += 1;
    let page: JournalPage | null = null;
    try {
      page = await getJournal(options.connection, options.getAfter(), limit, controller.signal);
      dispatch({ type: 'journal/server-page', page });
      failures = 0;
    } catch (cause) {
      if (controller.signal.aborted) {
        // The poller was stopped (or the tab went away) mid-request.
        if (!running) return;
      } else {
        failures += 1;
        dispatch({ type: 'journal/server-error', message: describeFailure(cause) });
      }
    } finally {
      if (inFlight === controller) inFlight = null;
    }
    if (!running) return;

    if (page !== null && page.has_more && drained < MAX_DRAIN_PAGES) {
      // More pages are waiting: read them before the next scheduled poll.
      drained += 1;
      schedule(0);
      return;
    }
    drained = 0;

    if (shouldVerify(Date.now())) {
      await runVerify(Date.now());
      if (!running) return;
    }

    // A wake arrived while the page/verify were in flight. Read one more
    // page immediately: the design's events may have been stored between
    // the page request and the design's arrival.
    if (wakeRequested) {
      wakeRequested = false;
      schedule(0);
      return;
    }

    if (failures > 0) {
      schedule(backoffDelay(failures));
      return;
    }
    schedule(options.isBusy() ? POLL_BUSY_MS : POLL_IDLE_MS);
  }

  return {
    start(): void {
      if (running) return;
      running = true;
      failures = 0;
      // The first page is read immediately so the ledger is populated
      // before the operator looks at it.
      schedule(0);
    },
    stop(): void {
      if (!running) return;
      running = false;
      wakeRequested = false;
      clearTimer();
      abortInFlight();
    },
    wake(): void {
      if (!running) return;
      if (inFlight) {
        // The in-flight tick will honour the wake after it finishes.
        wakeRequested = true;
        return;
      }
      wakeRequested = false;
      schedule(0);
    },
    isRunning(): boolean {
      return running;
    },
    __test: {
      pollCount: () => polls,
      verifyCount: () => verifies,
      pendingTimer: () => timer !== null,
      nextDelayMs: () => nextDelay,
      backoffStep: () => failures,
    },
  };
}
