// Agent & Kernel Journal — the right-bottom engineer's ledger.
//
// The journal is the app's real event ledger: preview requests/results, chat
// turns, export receipts, and connection events are appended by the store
// helpers. Beside them the backend keeps its own hash-chained session
// journal (`/journal`), whose events carry a sequence number and a digest.
//
// The ledger shows both, in one six-column table:
//
//   SEQ | UTC | TYPE | MESSAGE | DIGEST | CHAIN
//
// A local UI row has no sequence and no chain state, so it shows "—" and
// "local"; it is never dressed up as a chain event. The header stamp is the
// chain verdict and it comes from a real `/journal/verify` response only:
// "HASH-CHAIN OK" when the chain verifies, "CHAIN BROKEN @ n" when the
// backend names the first bad event, and the neutral "SESSION LOG" when no
// verification has been reported.
//
// The list sticks to the newest row unless the operator has scrolled up. An
// aria-live status region announces new rows at most once per interval so a
// burst of preview events does not flood a screen reader.
//
// Every MESSAGE cell is one line: the ledger is a scan surface, not a payload
// viewer. The full text stays in the cell (so a screen reader still reads it)
// and in its title tooltip. A digest's copy button shares the DIGEST cell with
// the chip, so it can never paint over CHAIN.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import type { HashChainStatus, JournalEntry } from '../../types/studio';
import type { ConnectionStatus } from '../../state/store';
import { useUiState } from '../../state/store';
import { CopyButton } from './CopyButton';
import {
  buildLedgerRows,
  chainLabel,
  chainTitle,
  shortDigest,
} from './ledgerRows';
import '../../styles/journal.css';

/**
 * FINAL prop contract for the right-hand journal slot.
 *
 * The backend version rendered in the footer is read from the shared store
 * (`server.version`, reported by the real health/providers fetch).
 */
export interface JournalPanelProps {
  /** Bounded journal rows from `useUiState(selectJournal)`. */
  entries: ReadonlyArray<JournalEntry>;
  /** Real loopback connection state, rendered in the ledger footer. */
  connectionStatus: ConnectionStatus;
  /**
   * Hash-chain verification reported by a real backend. The shell passes the
   * store's verdict; when the prop is absent the store's own verdict is
   * used. Never derive "OK" from local state.
   */
  hashChainStatus?: HashChainStatus;
  /**
   * The first event whose digest does not verify. The store's value is used
   * when the prop is absent.
   */
  firstBadSequence?: number | null;
  /**
   * Draw the footer with the origin and the backend version. Details ›
   * Activity turns it off: the status bar already reports both. Other
   * callers keep the default.
   */
  showFooter?: boolean;
}

// Distance from the bottom, in pixels, that still counts as "stuck".
const STICK_THRESHOLD_PX = 24;

// Coalesce screen-reader announcements of rapid journal bursts.
const ANNOUNCE_INTERVAL_MS = 500;

export function JournalPanel(props: JournalPanelProps): ReactElement {
  const showFooter = props.showFooter ?? true;
  const backendVersion = useUiState((s) => s.server.version);
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const serverEvents = useUiState((s) => s.sessionJournal.events);
  const storeChainStatus = useUiState((s) => s.sessionJournal.chainStatus);
  const storeFirstBadSequence = useUiState((s) => s.sessionJournal.firstBadSequence);
  // The last polling failure. It is shown: a frozen ledger with a stale
  // "HASH-CHAIN OK" stamp is a lie the operator must not read. The note
  // clears itself as soon as a poll succeeds.
  const pollError = useUiState((s) => s.sessionJournal.lastError);
  const listRef = useRef<HTMLDivElement | null>(null);
  const announceTimerRef = useRef<number | null>(null);
  const pendingAnnouncementRef = useRef<string>('');
  const lastAnnouncementRef = useRef<string>('');
  const [pinned, setPinned] = useState(true);
  const [liveMessage, setLiveMessage] = useState('');
  const [newestFirst, setNewestFirst] = useState(false);

  const chainStatus = props.hashChainStatus ?? storeChainStatus;
  const firstBadSequence =
    props.firstBadSequence !== undefined ? props.firstBadSequence : storeFirstBadSequence;

  // The merged ledger: the local activity and the backend's stored events,
  // ordered by time. Memoised so the scroll and announcement effects do not
  // re-run on every render.
  const rows = useMemo(
    () => buildLedgerRows(props.entries, serverEvents, chainStatus, firstBadSequence, newestFirst),
    [props.entries, serverEvents, chainStatus, firstBadSequence, newestFirst],
  );

  // Auto-scroll only while the operator is already at the newest row. Layout
  // effect so the measurement happens after the new row is laid out.
  useLayoutEffect(() => {
    if (!pinned) return;
    const node = listRef.current;
    if (!node) return;
    // The newest row sits at the bottom in the ledger order and at the top
    // when the operator asked for newest-first.
    node.scrollTop = newestFirst ? 0 : node.scrollHeight;
  }, [rows, pinned, newestFirst]);

  // Throttled polite announcements: the first row in a burst is announced
  // after the interval, and anything that arrived meanwhile is represented by
  // the latest pending row.
  useEffect(() => {
    const latest = newestRow(rows, newestFirst);
    if (!latest) {
      if (announceTimerRef.current !== null) {
        window.clearTimeout(announceTimerRef.current);
        announceTimerRef.current = null;
      }
      pendingAnnouncementRef.current = '';
      lastAnnouncementRef.current = '';
      setLiveMessage('');
      return;
    }
    pendingAnnouncementRef.current = `${latest.type} · ${latest.message}`;
    if (announceTimerRef.current !== null) return;
    announceTimerRef.current = window.setTimeout(() => {
      announceTimerRef.current = null;
      const message = pendingAnnouncementRef.current;
      if (message !== lastAnnouncementRef.current) {
        lastAnnouncementRef.current = message;
        setLiveMessage(message);
      }
    }, ANNOUNCE_INTERVAL_MS);
  }, [rows, newestFirst]);

  useEffect(() => () => {
    if (announceTimerRef.current !== null) {
      window.clearTimeout(announceTimerRef.current);
      announceTimerRef.current = null;
    }
  }, []);

  function onScroll(): void {
    const node = listRef.current;
    if (!node) return;
    if (newestFirst) {
      // In newest-first order the newest row is at the top of the ledger.
      setPinned(node.scrollTop <= STICK_THRESHOLD_PX);
      return;
    }
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    setPinned(distance <= STICK_THRESHOLD_PX);
  }

  function jumpToLatest(): void {
    const node = listRef.current;
    if (!node) return;
    node.scrollTop = newestFirst ? 0 : node.scrollHeight;
    setPinned(true);
  }

  const origin = describeOrigin(props.connectionStatus);
  const showJump = !pinned && rows.length > 0;
  const degraded = designAvailable === false;

  return (
    <section className="journal-panel" aria-label="Agent and kernel journal">
      <header className="journal-panel-header">
        <h2>AGENT &amp; KERNEL JOURNAL</h2>
        <span
          className="journal-panel-stamp"
          data-chain={chainStatus}
          title={describeChainTitle(chainStatus, firstBadSequence)}
        >
          {describeChain(chainStatus, firstBadSequence)}
        </span>
      </header>
      <div className="journal-toolbar">
        <button
          type="button"
          className="journal-order-toggle"
          aria-pressed={newestFirst}
          data-order={newestFirst ? 'newest' : 'oldest'}
          title={
            newestFirst
              ? 'Ledger order: newest first. Toggle to read oldest first.'
              : 'Ledger order: oldest first. Toggle to read newest first.'
          }
          onClick={() => setNewestFirst((value) => !value)}
        >
          NEWEST FIRST
        </button>
        <span className="journal-row-count">
          {rows.length} {rows.length === 1 ? 'row' : 'rows'}
        </span>
      </div>
      <div className="journal-body">
        {pollError ? (
          <p className="journal-panel-note journal-panel-poll-error" role="status">
            {`Journal poll failed: ${pollError} — the ledger below may be stale.`}
          </p>
        ) : null}
        <div
          className="journal-list"
          ref={listRef}
          onScroll={onScroll}
          role="region"
          aria-label="Journal ledger"
        >
          {degraded ? (
            <p className="journal-panel-note">
              Design journal unavailable — this backend reports design_available: false, so only
              local activity is recorded here.
            </p>
          ) : null}
          {rows.length === 0 ? (
            <p className="journal-empty">
              No journal events yet. Preview, chat, export, and connection activity will be
              recorded here.
            </p>
          ) : (
            <table className="journal-ledger">
              <caption className="journal-ledger-caption">
                Session journal — SEQ, UTC, TYPE, MESSAGE, DIGEST, CHAIN
              </caption>
              <thead>
                <tr>
                  <th scope="col">SEQ</th>
                  <th scope="col">UTC</th>
                  <th scope="col">TYPE</th>
                  <th scope="col">MESSAGE</th>
                  <th scope="col">DIGEST</th>
                  <th scope="col">CHAIN</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((row) => (
                  <tr
                    key={row.id}
                    className="journal-entry"
                    data-kind={row.kind}
                    data-source={row.source}
                    data-chain={row.chain}
                    title={`UTC ${isoTime(row.ts)} · CHAIN ${chainLabel(row.chain)}${
                      row.sequence === null ? '' : ` · SEQ ${row.sequence}`
                    }`}
                  >
                    <td className="journal-entry-seq">{row.sequence ?? '—'}</td>
                    <td className="journal-entry-utc">
                      <time
                        className="journal-entry-time"
                        dateTime={isoTime(row.ts)}
                        title={isoTime(row.ts)}
                      >
                        {formatUtcTime(row.ts)}
                      </time>
                    </td>
                    <td className="journal-entry-type">
                      <span className="journal-entry-tag" title={row.type}>
                        {row.type}
                      </span>
                    </td>
                    <td className="journal-entry-message">
                      <span className="journal-entry-message-text" title={row.message}>
                        {row.message}
                      </span>
                    </td>
                    <td className="journal-entry-digest-cell">
                      {row.digest ? (
                        <span className="journal-entry-digest-line">
                          <code className="journal-entry-digest" title={row.digest}>
                            {shortDigest(row.digest)}
                          </code>
                          <CopyButton
                            value={row.digest}
                            label="Copy digest"
                            className="journal-digest-copy"
                          />
                        </span>
                      ) : null}
                    </td>
                    <td className="journal-entry-chain" title={chainTitle(row.chain)}>
                      {chainLabel(row.chain)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
        {showJump ? (
          <button
            type="button"
            className="journal-jump"
            aria-label="Jump to latest journal entry"
            onClick={jumpToLatest}
          >
            Jump to latest ↓
          </button>
        ) : null}
      </div>
      <p className="journal-live" aria-live="polite" aria-atomic="true" role="status">
        {liveMessage}
      </p>
      {showFooter ? (
        <footer className="journal-panel-footer">
          <span className="journal-panel-connection" title={origin.title}>
            {origin.text}
          </span>
          <span className="journal-panel-version">
            {backendVersion ? `backend v${backendVersion}` : 'backend v—'}
          </span>
        </footer>
      ) : null}
    </section>
  );
}

/** The newest row of the ordered ledger. */
function newestRow(
  rows: ReadonlyArray<{ type: string; message: string }>,
  newestFirst: boolean,
): { type: string; message: string } | null {
  if (rows.length === 0) return null;
  return newestFirst ? rows[0]! : rows[rows.length - 1]!;
}

/** The UTC wall-clock time of an instant, `HH:MM:SS`. */
function formatUtcTime(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '—';
  return date.toISOString().slice(11, 19);
}

function isoTime(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '';
  return date.toISOString();
}

function describeOrigin(status: ConnectionStatus): { text: string; title: string } {
  if (status.kind === 'ready') {
    return {
      text: status.connection.baseUrl,
      title: status.connection.mode === 'desktop' ? 'Desktop bridge origin' : 'Browser fallback origin',
    };
  }
  if (status.kind === 'connecting') {
    return { text: 'connecting…', title: 'Loopback connection in progress' };
  }
  if (status.kind === 'error') {
    return { text: 'not connected', title: status.message };
  }
  return { text: 'not connected', title: 'No loopback connection' };
}

/**
 * The chain stamp. "HASH-CHAIN OK" is printed only for a verified chain; a
 * broken chain names the first bad event when the backend reported it.
 */
function describeChain(status: HashChainStatus, firstBadSequence: number | null): string {
  if (status === 'verified') return 'HASH-CHAIN OK';
  if (status === 'broken') {
    return firstBadSequence === null ? 'HASH-CHAIN BROKEN' : `CHAIN BROKEN @ ${firstBadSequence}`;
  }
  // Neutral honest state: no verification has been reported for this session.
  return 'SESSION LOG';
}

function describeChainTitle(status: HashChainStatus, firstBadSequence: number | null): string {
  if (status === 'verified') return 'A real backend reports a verified hash chain';
  if (status === 'broken') {
    return firstBadSequence === null
      ? 'A real backend reports a broken hash chain'
      : `A real backend reports a broken hash chain from event ${firstBadSequence} on`;
  }
  return 'No backend hash-chain verification is reported for this session';
}
