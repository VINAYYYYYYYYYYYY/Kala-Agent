// The ledger view model.
//
// The journal has two honest sources: the backend's stored, hash-chained
// events (the server rows) and the UI-local activity the shell records while
// the operator works (the local rows). They are different things: a local
// row is not a chain event and must never be presented as one.
//
// This module turns both sources into the flat rows the six-column ledger
// renders. It is pure: no clock, no store, no side effects. Every number it
// shows was read from the event that carried it.
//
// The chain verdict is the only thing that may mark a server row "ok", and
// it comes from a `/journal/verify` response. A local row is always "local"
// and never claims a chain state it cannot have.

import type { HashChainStatus, JournalEntry, JournalKind } from '../../types/studio';
import type { SessionJournalEvent } from '../../state/journal';

/** The chain state of one ledger row. */
export type LedgerChain = 'ok' | 'broken' | 'pending' | 'local';

/** One row of the six-column ledger. */
export interface LedgerRow {
  /** Stable key for React. */
  id: string;
  source: 'local' | 'server';
  /** The local category of the row (preview, chat, tool, …). */
  kind: JournalKind;
  /** The TYPE cell: a local tag such as `[PREVIEW]` or an event type. */
  type: string;
  /** The MESSAGE cell. */
  message: string;
  /** Epoch milliseconds. */
  ts: number;
  /** The backend sequence, or null for a local row. */
  sequence: number | null;
  /** The full digest, or null when the event carries none. */
  digest: string | null;
  /** The CHAIN cell. */
  chain: LedgerChain;
}

/** The local kind tags, shared with the activity list. */
export const KIND_LABEL: Record<JournalKind, string> = {
  preview: 'PREVIEW',
  chat: 'CHAT',
  tool: 'TOOL',
  export: 'EXPORT',
  connection: 'BRIDGE',
  system: 'SYSTEM',
};

/**
 * Build the ledger rows.
 *
 * The order is by time, oldest first, unless `newestFirst` is set. Rows that
 * share a timestamp keep the backend row first: it is the authoritative
 * ledger row, and the local row is only a note about the same instant.
 */
export function buildLedgerRows(
  local: ReadonlyArray<JournalEntry>,
  server: ReadonlyArray<SessionJournalEvent>,
  chainStatus: HashChainStatus,
  firstBadSequence: number | null,
  newestFirst = false,
): LedgerRow[] {
  const rows: LedgerRow[] = [];
  // Server rows first so a stable sort keeps them ahead of a local row that
  // carries the same millisecond.
  for (const event of server) {
    rows.push({
      id: `server-${event.sequence}`,
      source: 'server',
      kind: journalKindFor(event.eventType),
      type: event.eventType,
      message: event.summary,
      ts: event.ts,
      sequence: event.sequence,
      digest: event.digest,
      chain: chainFor(event.sequence, chainStatus, firstBadSequence),
    });
  }
  for (const entry of local) {
    rows.push({
      id: entry.id,
      source: 'local',
      kind: entry.kind,
      type: `[${KIND_LABEL[entry.kind]}]`,
      message: entry.message,
      ts: entry.ts,
      sequence: null,
      digest: entry.digest ?? null,
      chain: 'local',
    });
  }
  rows.sort((a, b) => a.ts - b.ts);
  return newestFirst ? rows.reverse() : rows;
}

/**
 * The chain state of one stored event.
 *
 * `verified` is the only verdict that marks a row "ok", and it comes from a
 * verification response that said so. When the chain is broken, the rows
 * before the first bad sequence are still covered by an intact prefix; the
 * bad row and everything after it are marked broken.
 */
export function chainFor(
  sequence: number,
  chainStatus: HashChainStatus,
  firstBadSequence: number | null,
): LedgerChain {
  if (chainStatus === 'unknown') return 'pending';
  if (chainStatus === 'verified') return 'ok';
  if (firstBadSequence === null) return 'broken';
  return sequence >= firstBadSequence ? 'broken' : 'ok';
}

/** The CHAIN cell text. */
export function chainLabel(chain: LedgerChain): string {
  if (chain === 'local') return 'local';
  if (chain === 'ok') return 'ok';
  if (chain === 'broken') return 'broken';
  return '—';
}

/** The CHAIN cell title: what the verdict is based on. */
export function chainTitle(chain: LedgerChain): string {
  if (chain === 'local') return 'Local UI activity — not a chain event';
  if (chain === 'ok') return 'Covered by a verified hash chain';
  if (chain === 'broken') return 'The chain does not verify from this event on';
  return 'No hash-chain verification has been reported yet';
}

/**
 * The short digest shown in the ledger. A short value (a byte count, a
 * label) is shown as it is; a sha256 is cut to its first eight characters.
 * The full value always stays available in the cell's title and in the copy
 * button.
 */
export function shortDigest(digest: string): string {
  const compact = digest.replace(/\s+/g, ' ').trim();
  if (compact.length <= 12) return compact;
  return `${compact.slice(0, 8)}…`;
}

/** The local ledger category a backend event belongs to. */
export function journalKindFor(eventType: string): JournalKind {
  if (eventType.startsWith('chat.')) return 'chat';
  if (eventType.startsWith('export.')) return 'export';
  if (eventType.startsWith('design.') || eventType.startsWith('artifact.')) return 'tool';
  if (eventType.startsWith('provider.') || eventType.startsWith('view.')) return 'tool';
  if (eventType.startsWith('preview.')) return 'preview';
  return 'system';
}
