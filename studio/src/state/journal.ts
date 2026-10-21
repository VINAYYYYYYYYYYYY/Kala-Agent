// The Studio session journal slice.
//
// The backend keeps its own hash-chained SQLite journal of design turns
// (`/journal`, `/journal/verify`). The UI keeps its own short-lived ledger
// of local activity (a preview request, a chat turn, an export receipt).
// The two live apart on purpose: a local row is not a chain event and must
// never be presented as one.
//
// The server events are stored in their own list, deduped by sequence, and
// merged with the local rows in a selector that orders by time. The chain
// verdict comes only from a successful `/journal/verify` response; nothing
// else may light the HASH-CHAIN OK stamp.

import type { HashChainStatus, JournalKind, JournalPage, JournalVerify } from '../types/studio';
import type { JournalEntry } from '../types/studio';
import { SESSION_JOURNAL_CAP } from '../types/studio';

/** One stored backend journal event, flattened for the ledger. */
export interface SessionJournalEvent {
  sequence: number;
  eventType: string;
  aggregateId: string;
  /** The raw ISO timestamp the backend stored. */
  occurredAt: string;
  /** The same instant in epoch milliseconds, for ordering. */
  ts: number;
  /** A short, honest one-line summary of the payload. */
  summary: string;
  /** The SHA-256 of the stored payload text. */
  digest: string;
  /** The chain digest of this event. */
  eventDigest: string;
  previousEventDigest: string;
  payload: Record<string, unknown> | null;
}

export interface SessionJournalState {
  /** The stored events, ordered by sequence, deduped by sequence. */
  readonly events: ReadonlyArray<SessionJournalEvent>;
  /** The highest sequence the store has seen; the next poll asks after it. */
  readonly lastSequence: number;
  /** True while the backend still has events after `lastSequence`. */
  readonly hasMore: boolean;
  /** The last verification verdict. `'unknown'` until a verify succeeds. */
  readonly chainStatus: HashChainStatus;
  /** The first sequence whose digest does not match, when the chain broke. */
  readonly firstBadSequence: number | null;
  /** The chain head digest reported by the last verification. */
  readonly headDigest: string | null;
  /** The number of events the last verification counted. */
  readonly eventCount: number | null;
  /** The last polling error, if any. */
  readonly lastError: string | null;
}

export type SessionJournalAction =
  | { type: 'journal/server-page'; page: JournalPage }
  | { type: 'journal/server-verify'; verify: JournalVerify }
  | { type: 'journal/server-error'; message: string };

export function createSessionJournalState(): SessionJournalState {
  return {
    events: [],
    lastSequence: 0,
    hasMore: false,
    chainStatus: 'unknown',
    firstBadSequence: null,
    headDigest: null,
    eventCount: null,
    lastError: null,
  };
}

/** The pure session-journal reducer. */
export function sessionJournalReducer(
  state: SessionJournalState,
  action: SessionJournalAction,
): SessionJournalState {
  switch (action.type) {
    case 'journal/server-page': {
      const page = action.page;
      // Dedupe by sequence: a page may repeat an event the store already
      // holds (a re-poll after a restart, an overlapping page).
      const bySequence = new Map<number, SessionJournalEvent>();
      for (const event of state.events) bySequence.set(event.sequence, event);
      let lastSequence = state.lastSequence;
      for (const event of page.events) {
        if (bySequence.has(event.sequence)) continue;
        const stored = toSessionEvent(event);
        bySequence.set(stored.sequence, stored);
        if (stored.sequence > lastSequence) lastSequence = stored.sequence;
      }
      const events = [...bySequence.values()].sort((a, b) => a.sequence - b.sequence);
      return {
        ...state,
        events: events.length > SESSION_JOURNAL_CAP
          ? events.slice(events.length - SESSION_JOURNAL_CAP)
          : events,
        lastSequence,
        hasMore: page.has_more,
        lastError: null,
      };
    }
    case 'journal/server-verify': {
      const verify = action.verify;
      return {
        ...state,
        // 'verified' is only ever set from a verify response that says ok.
        chainStatus: verify.ok ? 'verified' : 'broken',
        firstBadSequence: verify.ok ? null : verify.first_bad_sequence,
        headDigest: verify.head_digest,
        eventCount: verify.event_count,
        // A verification says nothing about the polling, so the last
        // polling error stays until a page succeeds.
      };
    }
    case 'journal/server-error': {
      // A failed poll does not change the last known verdict; it only
      // records that the ledger could not be refreshed.
      return { ...state, lastError: action.message };
    }
  }
}

function toSessionEvent(event: JournalPage['events'][number]): SessionJournalEvent {
  return {
    sequence: event.sequence,
    eventType: event.event_type,
    aggregateId: event.aggregate_id,
    occurredAt: event.occurred_at,
    ts: Date.parse(event.occurred_at),
    summary: summarisePayload(event.event_type, event.payload),
    digest: event.digest,
    eventDigest: event.event_digest,
    previousEventDigest: event.previous_event_digest,
    payload: event.payload,
  };
}

// --- Merging the two ledgers --------------------------------------------------

/** One row of the merged ledger. */
export interface MergedJournalEntry {
  id: string;
  /** Epoch milliseconds. */
  ts: number;
  source: 'local' | 'server';
  kind: JournalKind;
  message: string;
  digest?: string;
  /** The backend sequence, for server rows. */
  sequence?: number;
  /** The backend event type, for server rows. */
  eventType?: string;
  /** The chain verdict at the time the row is rendered. */
  chain: HashChainStatus;
}

/**
 * Merge the local ledger and the stored backend events into one list
 * ordered by time. Rows with the same timestamp keep the backend event
 * first: it is the authoritative ledger row.
 */
export function mergeJournalEntries(
  local: ReadonlyArray<JournalEntry>,
  server: ReadonlyArray<SessionJournalEvent>,
  chainStatus: HashChainStatus,
): ReadonlyArray<MergedJournalEntry> {
  const merged: MergedJournalEntry[] = [];
  for (const entry of local) {
    merged.push({
      id: entry.id,
      ts: entry.ts,
      source: 'local',
      kind: entry.kind,
      message: entry.message,
      chain: chainStatus,
      ...(entry.digest === undefined ? {} : { digest: entry.digest }),
    });
  }
  for (const event of server) {
    merged.push({
      id: `server-${event.sequence}`,
      ts: event.ts,
      source: 'server',
      kind: journalKindFor(event.eventType),
      message: `${event.eventType} · ${event.summary}`,
      digest: event.digest,
      sequence: event.sequence,
      eventType: event.eventType,
      chain: chainStatus,
    });
  }
  merged.sort((a, b) => {
    if (a.ts !== b.ts) return a.ts - b.ts;
    if (a.source !== b.source) return a.source === 'server' ? -1 : 1;
    if (a.sequence !== undefined && b.sequence !== undefined) return a.sequence - b.sequence;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
  return merged;
}

/**
 * A memoising wrapper around `mergeJournalEntries`.
 *
 * The React store is read through `useSyncExternalStore`, which requires
 * the snapshot to be referentially stable: a selector that built a new
 * array on every call would loop forever. The memo keeps the last result
 * and returns it again while the three inputs are unchanged.
 */
export type MergedJournalSelector = (
  local: ReadonlyArray<JournalEntry>,
  server: ReadonlyArray<SessionJournalEvent>,
  chainStatus: HashChainStatus,
) => ReadonlyArray<MergedJournalEntry>;

export function createMergedJournalSelector(): MergedJournalSelector {
  let lastLocal: ReadonlyArray<JournalEntry> | null = null;
  let lastServer: ReadonlyArray<SessionJournalEvent> | null = null;
  let lastChain: HashChainStatus | null = null;
  let lastResult: ReadonlyArray<MergedJournalEntry> = [];
  return (local, server, chainStatus) => {
    if (local === lastLocal && server === lastServer && chainStatus === lastChain) {
      return lastResult;
    }
    lastResult = mergeJournalEntries(local, server, chainStatus);
    lastLocal = local;
    lastServer = server;
    lastChain = chainStatus;
    return lastResult;
  };
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

// --- Payload summaries --------------------------------------------------------

function numberText(value: unknown, digits = 3): string | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  return value.toFixed(digits);
}

function shortId(value: unknown): string | null {
  if (typeof value !== 'string' || value.length === 0) return null;
  return value.length > 8 ? value.slice(0, 8) : value;
}

/**
 * A one-line summary of a journal payload. Every number in the summary is
 * read from the payload itself; nothing is computed or rounded into
 * existence here.
 */
export function summarisePayload(
  eventType: string,
  payload: Record<string, unknown> | null,
): string {
  if (!payload) return '';
  const parts: string[] = [];
  switch (eventType) {
    case 'design.intent': {
      const template = typeof payload['template'] === 'string' ? payload['template'] : null;
      const provider = typeof payload['provider'] === 'string' ? payload['provider'] : null;
      const label = typeof payload['label'] === 'string' ? payload['label'] : null;
      if (template) parts.push(`template ${template}`);
      if (provider) parts.push(`provider ${provider}`);
      if (label) parts.push(label);
      const id = shortId(payload['design_id']);
      if (id) parts.push(`design ${id}`);
      break;
    }
    case 'design.compiled': {
      const components = payload['components'];
      if (typeof components === 'number') parts.push(`${components} components`);
      if (payload['fused'] === true) parts.push('fused');
      const id = shortId(payload['design_id']);
      if (id) parts.push(`design ${id}`);
      break;
    }
    case 'design.measured': {
      const provider = typeof payload['provider'] === 'string' ? payload['provider'] : null;
      const measuredBy = typeof payload['measured_by'] === 'string' ? payload['measured_by'] : null;
      if (measuredBy) parts.push(`measured by ${measuredBy}`);
      else if (provider) parts.push(`provider ${provider}`);
      const volume = numberText(payload['volume_mm3']);
      const area = numberText(payload['surface_area_mm2']);
      if (volume !== null) parts.push(`volume ${volume} mm^3`);
      if (area !== null) parts.push(`area ${area} mm^2`);
      if (typeof payload['solid_count'] === 'number') {
        parts.push(`${payload['solid_count']} solid${payload['solid_count'] === 1 ? '' : 's'}`);
      }
      break;
    }
    case 'design.checked': {
      parts.push(payload['verified'] === true ? 'verified' : 'not verified');
      const checks = payload['checks'];
      if (Array.isArray(checks)) {
        let failed = 0;
        let notReported = 0;
        for (const check of checks) {
          if (!isRecord(check)) continue;
          if (check['status'] === 'not_reported') {
            notReported += 1;
          } else if (check['status'] === 'failed' || (check['status'] === undefined && check['passed'] === false)) {
            // A pre-B2 event has no status; `passed: false` is then the only
            // failure signal and is treated as a failure, not a pass.
            failed += 1;
          }
        }
        if (failed === 0 && notReported === 0) {
          parts.push(`${checks.length} checks passed`);
        } else {
          if (failed > 0) parts.push(`${failed} checks failed`);
          if (notReported > 0) parts.push(`${notReported} not reported`);
        }
      }
      break;
    }
    case 'provider.build.completed': {
      const provider = typeof payload['provider'] === 'string' ? payload['provider'] : null;
      const version = typeof payload['provider_version'] === 'string'
        ? payload['provider_version']
        : null;
      if (provider) parts.push(version ? `${provider} ${version}` : provider);
      parts.push('build completed');
      const step = shortId(payload['step_sha256']);
      if (step) parts.push(`step ${step}`);
      break;
    }
    case 'provider.build.failed': {
      const provider = typeof payload['provider'] === 'string' ? payload['provider'] : null;
      const reason = typeof payload['reason'] === 'string' ? payload['reason'] : null;
      if (provider) parts.push(provider);
      parts.push(reason ? `refused: ${reason}` : 'build refused');
      break;
    }
    case 'view.imported': {
      const triangles = payload['triangles'];
      const solids = payload['solids'];
      parts.push('view imported');
      if (typeof solids === 'number') parts.push(`${solids} solid${solids === 1 ? '' : 's'}`);
      if (typeof triangles === 'number') parts.push(`${triangles} triangles`);
      break;
    }
    case 'design.rejected': {
      const reason = typeof payload['reason'] === 'string' ? payload['reason'] : null;
      parts.push(reason ? `refused: ${reason}` : 'refused');
      break;
    }
    case 'artifact.stored': {
      const kind = typeof payload['kind'] === 'string' ? payload['kind'] : null;
      if (kind) parts.push(kind);
      const size = payload['size_bytes'];
      if (typeof size === 'number' && Number.isFinite(size)) parts.push(`${size} bytes`);
      const digest = shortId(payload['sha256']);
      if (digest) parts.push(`sha ${digest}`);
      break;
    }
    case 'export.completed': {
      const format = typeof payload['format'] === 'string' ? payload['format'] : null;
      if (format) parts.push(format);
      const size = payload['size_bytes'];
      if (typeof size === 'number' && Number.isFinite(size)) parts.push(`${size} bytes`);
      const digest = shortId(payload['sha256']);
      if (digest) parts.push(`sha ${digest}`);
      break;
    }
    case 'chat.turn': {
      const provider = typeof payload['provider'] === 'string' ? payload['provider'] : null;
      const cadProvider = typeof payload['cad_provider'] === 'string'
        ? payload['cad_provider']
        : null;
      // A turn that failed carries the reason it failed. It must not read
      // like a turn that was merely requested: the ledger is the audit trail
      // of the session, and a turn that died with `provider_timeout` has to
      // say so.
      const failure = typeof payload['failure'] === 'string' ? payload['failure'] : null;
      if (provider) parts.push(provider);
      if (cadProvider) parts.push(`CAD ${cadProvider}`);
      if (payload['offline'] === true) parts.push('offline');
      if (failure && failure.trim().length > 0) {
        parts.push(`failed: ${failure.trim()}`);
      } else if (payload['design_requested'] === true) {
        parts.push('design requested');
      }
      break;
    }
    default:
      break;
  }
  if (parts.length > 0) return parts.join(' · ');
  // A type we do not know yet still gets an honest, bounded summary of the
  // first few scalar payload values.
  const fallback: string[] = [];
  for (const [key, value] of Object.entries(payload)) {
    if (fallback.length >= 3) break;
    if (typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string') {
      fallback.push(`${key}=${String(value).slice(0, 24)}`);
    }
  }
  return fallback.join(' · ');
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
