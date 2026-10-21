// @vitest-environment happy-dom
// The six-column session ledger.
//
// The ledger merges the backend's hash-chained journal events with the
// UI-local activity. These tests pin the rules that make it honest:
//
//   - the six columns are SEQ, UTC, TYPE, MESSAGE, DIGEST, CHAIN;
//   - a local row has no sequence and no chain state ("—" and "local");
//   - the chain stamp is "HASH-CHAIN OK" only for a verified chain, "CHAIN
//     BROKEN @ n" when the backend names the first bad event, and the
//     neutral "SESSION LOG" when nothing has been verified;
//   - the digest column shows a short digest and copies the full one;
//   - the newest-first toggle reverses the ledger order;
//   - a backend without the design routes says so instead of pretending.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import fs from 'node:fs';
import path from 'node:path';

import { JournalPanel } from '../src/components/journal/JournalPanel';
import { dispatch, getState, resetStore } from '../src/state/store';
import { summarisePayload } from '../src/state/journal';
import type { JournalEntry, JournalPage } from '../src/types/studio';
import {
  bracketJournalPage,
  bracketJournalB2Page,
  bracketJournalBroken,
  bracketJournalVerified,
} from './designFixtures';

const BASE_URL = 'http://127.0.0.1:8765';

const readyConnection = {
  kind: 'ready' as const,
  connection: { baseUrl: BASE_URL, token: 'x'.repeat(32), mode: 'desktop' as const },
};

let writeText: ReturnType<typeof vi.fn>;

beforeEach(() => {
  resetStore();
  writeText = vi.fn(async () => undefined);
  Object.defineProperty(globalThis.navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  });
});

afterEach(() => {
  cleanup();
  resetStore();
  vi.restoreAllMocks();
});

function localEntry(overrides: Partial<JournalEntry> = {}): JournalEntry {
  return {
    id: 'local-1',
    ts: Date.UTC(2026, 9, 19, 16, 0, 0),
    kind: 'chat',
    message: 'Agent · sized the bracket',
    ...overrides,
  };
}

function seedServerPage(): void {
  act(() => {
    dispatch({ type: 'journal/server-page', page: bracketJournalPage });
  });
}

function seedVerified(): void {
  act(() => {
    dispatch({ type: 'journal/server-verify', verify: bracketJournalVerified });
  });
}

function seedBroken(): void {
  act(() => {
    dispatch({ type: 'journal/server-verify', verify: bracketJournalBroken });
  });
}

/** A fourth event, so the broken-chain marking can be observed. */
const fourthEventPage: JournalPage = {
  events: [
    {
      sequence: 4,
      event_type: 'design.measured',
      aggregate_id: '88a7ec139868e36bd23c81f5688869bf9de5f23a7c72928db4014c9eb0afebae',
      payload: { volume_mm3: 35571.416, surface_area_mm2: 11889.102, solid_count: 1 },
      digest: 'aa11bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66',
      occurred_at: '2026-10-19T16:47:00.000000Z',
      previous_event_digest: '01509e146a3763fb7b7704e2acde3d44e391ede304fdb387a6e3f2a50859a20f',
      event_digest: 'aa11bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66',
    },
  ],
  next_after: 4,
  has_more: false,
  limit: 200,
};

describe('the real B2 journal page', () => {
  it('merges all ten server events with sequence, digest, and chain, interleaved by time', () => {
    act(() => {
      dispatch({ type: 'journal/server-page', page: bracketJournalB2Page });
      dispatch({ type: 'journal/server-verify', verify: bracketJournalVerified });
      dispatch({
        type: 'journal/append',
        entry: {
          id: 'local-a',
          ts: Date.parse('2026-10-19T20:18:59.700Z'),
          kind: 'chat',
          message: 'Local note between the build request and the build',
        },
      });
      dispatch({
        type: 'journal/append',
        entry: {
          id: 'local-b',
          ts: Date.parse('2026-10-19T20:19:00.500Z'),
          kind: 'tool',
          message: 'Local note after the turn',
        },
      });
    });

    const view = render(
      <JournalPanel entries={getState().journal} connectionStatus={readyConnection} />,
    );
    const rows = Array.from(view.container.querySelectorAll<HTMLElement>('tr.journal-entry'));
    expect(rows).toHaveLength(12);
    const serverRows = rows.filter((row) => row.getAttribute('data-source') === 'server');
    expect(serverRows).toHaveLength(10);
    expect(serverRows.map((row) => row.querySelector('.journal-entry-seq')?.textContent)).toEqual(
      ['1', '2', '3', '4', '5', '6', '7', '8', '9', '10'],
    );
    for (const row of serverRows) {
      expect(row.querySelector('.journal-entry-digest')?.textContent?.length).toBeGreaterThan(4);
      expect(row.querySelector('.journal-entry-digest')?.getAttribute('title')).toHaveLength(64);
      expect(row.querySelector('.journal-entry-chain')?.textContent).toBe('ok');
      expect(row.querySelector('.journal-entry-type')?.textContent).toMatch(
        /^(design\.|provider\.|artifact\.|view\.|chat\.)/,
      );
    }
    // The two local rows sit between the server rows at their real times;
    // they keep SEQ "—" and CHAIN "local" and are never dressed as events.
    const labels = rows.map((row) =>
      row.getAttribute('data-source') === 'server'
        ? row.querySelector('.journal-entry-seq')?.textContent
        : row.querySelector('.journal-entry-message')?.textContent,
    );
    expect(labels[0]).toBe('1');
    expect(labels[1]).toBe('2');
    expect(labels[2]).toContain('between the build request');
    expect(labels[3]).toBe('3');
    expect(labels[11]).toContain('after the turn');
    const localRows = rows.filter((row) => row.getAttribute('data-source') === 'local');
    for (const row of localRows) {
      expect(row.querySelector('.journal-entry-seq')?.textContent).toBe('—');
      expect(row.querySelector('.journal-entry-chain')?.textContent).toBe('local');
    }
  });
});

describe('ledger columns', () => {
  it('renders the six ledger columns', () => {
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const headers = Array.from(
      view.container.querySelectorAll('.journal-ledger thead th'),
    ).map((node) => node.textContent);
    expect(headers).toEqual(['SEQ', 'UTC', 'TYPE', 'MESSAGE', 'DIGEST', 'CHAIN']);
  });

  it('keeps UTC and CHAIN in each row title for the compact rail', () => {
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const rows = Array.from(view.container.querySelectorAll('tr.journal-entry'));
    expect(rows.length).toBeGreaterThan(1);
    for (const row of rows) {
      const title = row.getAttribute('title') ?? '';
      expect(title).toMatch(
        /^UTC \d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z · CHAIN (local|ok|broken|—)/,
      );
    }
    const localRow = rows.find((row) => row.getAttribute('data-source') === 'local')!;
    expect(localRow.getAttribute('title')).toContain(
      `UTC ${new Date(localEntry().ts).toISOString()} · CHAIN local`,
    );
  });

  it('gives a local row no sequence and no chain state', () => {
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const row = view.container.querySelector('.journal-entry')!;
    expect(row.getAttribute('data-source')).toBe('local');
    expect(row.querySelector('.journal-entry-seq')?.textContent).toBe('—');
    expect(row.querySelector('.journal-entry-chain')?.textContent).toBe('local');
    // A local row never carries a digest it did not earn.
    expect(row.querySelector('.journal-entry-digest')).toBeNull();
  });

  it('gives a stored event its sequence, digest, and chain state', () => {
    seedServerPage();
    seedVerified();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const rows = view.container.querySelectorAll('.journal-entry');
    expect(rows).toHaveLength(3);

    const serverRow = rows[1]!;
    expect(serverRow.getAttribute('data-source')).toBe('server');
    expect(serverRow.querySelector('.journal-entry-seq')?.textContent).toBe('1');
    expect(serverRow.querySelector('.journal-entry-type')?.textContent).toContain(
      'design.intent',
    );
    expect(serverRow.querySelector('.journal-entry-message')?.textContent).toContain(
      'template l_bracket',
    );
    // The digest is short in the cell; the full value stays in the title.
    const digest = serverRow.querySelector<HTMLElement>('.journal-entry-digest')!;
    expect(digest.textContent).toBe('7e613498…');
    expect(digest.getAttribute('title')).toBe(
      '7e6134981f8033d8b3de85ff8cc8d2c10d12210c99ffb901fa22a741e2ec0dbb',
    );
    expect(serverRow.querySelector('.journal-entry-chain')?.textContent).toBe('ok');
  });

  it('shows the UTC time of a stored event', () => {
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const time = view.container.querySelectorAll('time')[1]!;
    expect(time.getAttribute('dateTime')).toBe('2026-10-19T16:46:57.617Z');
    expect(time.textContent).toBe('16:46:57');
  });
});

describe('chain stamp', () => {
  it('is neutral until a backend verification arrives', () => {
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const stamp = view.container.querySelector('.journal-panel-stamp')!;
    expect(stamp.textContent).toBe('SESSION LOG');
    expect(stamp.getAttribute('data-chain')).toBe('unknown');
    expect(view.container.textContent).not.toContain('HASH-CHAIN OK');
  });

  it('claims HASH-CHAIN OK only for a verified chain', () => {
    seedServerPage();
    seedVerified();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const stamp = view.container.querySelector('.journal-panel-stamp')!;
    expect(stamp.textContent).toBe('HASH-CHAIN OK');
    expect(stamp.getAttribute('data-chain')).toBe('verified');
  });

  it('names the first bad event when the chain is broken', () => {
    seedServerPage();
    seedBroken();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const stamp = view.container.querySelector('.journal-panel-stamp')!;
    expect(stamp.textContent).toBe('CHAIN BROKEN @ 4');
    expect(stamp.getAttribute('data-chain')).toBe('broken');
    expect(view.container.textContent).not.toContain('HASH-CHAIN OK');
  });

  it('marks the rows from the first bad event on as broken', () => {
    seedServerPage();
    act(() => {
      dispatch({ type: 'journal/server-page', page: fourthEventPage });
    });
    seedBroken();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const rows = view.container.querySelectorAll('.journal-entry');
    // Sequence 1 and 2 sit before the break and stay covered; sequence 4 is
    // the first event the chain does not verify.
    expect(rows[1]?.querySelector('.journal-entry-chain')?.textContent).toBe('ok');
    expect(rows[3]?.querySelector('.journal-entry-chain')?.textContent).toBe('broken');
  });
});

describe('ledger order', () => {
  it('toggles between oldest-first and newest-first', () => {
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const toggle = view.container.querySelector<HTMLButtonElement>(
      '.journal-order-toggle',
    )!;
    expect(toggle.getAttribute('aria-pressed')).toBe('false');

    const firstSequence = (): string | null =>
      view.container.querySelector('.journal-entry-seq')?.textContent ?? null;

    expect(firstSequence()).toBe('—');
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-pressed')).toBe('true');
    expect(firstSequence()).toBe('2');
    fireEvent.click(toggle);
    expect(firstSequence()).toBe('—');
  });
});

describe('digest copy', () => {
  it('copies the full digest of a stored event', async () => {
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const copy = view.container.querySelector<HTMLButtonElement>(
      'button[aria-label="Copy digest"]',
    )!;
    fireEvent.click(copy);
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        '7e6134981f8033d8b3de85ff8cc8d2c10d12210c99ffb901fa22a741e2ec0dbb',
      ),
    );
    await waitFor(() =>
      expect(copy.getAttribute('data-state')).toBe('copied'),
    );
  });

  it('reports a refused clipboard instead of pretending', async () => {
    writeText.mockRejectedValueOnce(new Error('denied'));
    seedServerPage();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const copy = view.container.querySelector<HTMLButtonElement>(
      'button[aria-label="Copy digest"]',
    )!;
    fireEvent.click(copy);
    await waitFor(() => expect(copy.getAttribute('data-state')).toBe('failed'));
    expect(copy.getAttribute('aria-label')).toContain('refused');
  });
});

describe('backend without design support', () => {
  it('says so and keeps the local ledger', () => {
    act(() => {
      dispatch({ type: 'server/capabilities', designAvailable: false });
    });
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const note = view.container.querySelector('.journal-panel-note');
    expect(note?.textContent).toContain('design_available: false');
    // The local activity is still recorded.
    expect(view.container.querySelectorAll('.journal-entry')).toHaveLength(1);
    // No chain claim: nothing verified the journal.
    expect(view.container.querySelector('.journal-panel-stamp')?.textContent).toBe(
      'SESSION LOG',
    );
  });

  it('keeps the note away when the backend never said', () => {
    expect(getState().server.designAvailable).toBeNull();
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    expect(view.container.querySelector('.journal-panel-note')).toBeNull();
  });
});

describe('message summaries', () => {
  it('renders a server message as one line with the full text in the title', () => {
    act(() => {
      dispatch({ type: 'journal/server-page', page: bracketJournalB2Page });
      dispatch({ type: 'journal/server-verify', verify: bracketJournalVerified });
    });
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const serverRows = Array.from(
      view.container.querySelectorAll<HTMLElement>('tr.journal-entry[data-source="server"]'),
    );
    expect(serverRows).toHaveLength(10);

    // The build row is the type-specific summary the ledger is meant to show,
    // not the raw payload and not a 4–5 line wrap.
    const buildRow = serverRows.find((row) =>
      row.querySelector('.journal-entry-type')?.textContent?.includes('provider.build.completed'),
    )!;
    const message = buildRow.querySelector('.journal-entry-message')!;
    const text = message.querySelector<HTMLElement>('.journal-entry-message-text')!;
    expect(text.textContent).toBe('freecad 1.1.4 · build completed · step 01e54555');
    expect(text.getAttribute('title')).toBe(text.textContent);

    for (const row of serverRows) {
      const body = row.querySelector<HTMLElement>('.journal-entry-message-text')!;
      expect(body.getAttribute('title')).toBe(body.textContent);
      expect(body.getAttribute('title')?.length).toBeGreaterThan(0);
    }

    // The full message remains the cell's text content, so the accessible
    // reading order still carries it; only the visual line is clipped.
    expect(message.textContent).toBe(text.textContent);
  });

  it('keeps the MESSAGE styling single-line and ellipsized', () => {
    const css = fs.readFileSync(
      path.resolve(__dirname, '../src/styles/journal.css'),
      'utf8',
    );
    const start = css.indexOf('.studio-zone-right .journal-panel .journal-entry-message-text {');
    expect(start).toBeGreaterThan(-1);
    const block = css.slice(start, css.indexOf('}', start));
    expect(block).toContain('white-space: nowrap');
    expect(block).toContain('overflow: hidden');
    expect(block).toContain('text-overflow: ellipsis');
  });
});

describe('a failed chat turn', () => {
  /** One `chat.turn` event whose payload carries the failure code. */
  const failedTurnPage: JournalPage = {
    events: [
      {
        sequence: 1,
        event_type: 'chat.turn',
        aggregate_id: 'studio',
        payload: {
          provider: 'codex_cli',
          cad_provider: 'freecad',
          design_requested: true,
          failure: 'provider_timeout',
        },
        digest: 'aa'.repeat(32),
        occurred_at: '2026-10-21T10:00:00.000000Z',
        previous_event_digest: '00'.repeat(32),
        event_digest: 'bb'.repeat(32),
      },
    ],
    next_after: 1,
    has_more: false,
    limit: 200,
  };

  it('shows the failure code instead of reading as a normal design request', () => {
    act(() => {
      dispatch({ type: 'journal/server-page', page: failedTurnPage });
    });

    const summary = summarisePayload('chat.turn', failedTurnPage.events[0]!.payload);
    expect(summary).toContain('failed: provider_timeout');
    // It must not read like a turn that was merely requested.
    expect(summary).not.toContain('design requested');

    const view = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} />,
    );
    const row = view.container.querySelector('tr.journal-entry')!;
    const message = row.querySelector('.journal-entry-message-text')!;
    expect(message.textContent).toContain('failed: provider_timeout');
    expect(message.textContent).not.toContain('design requested');
  });

  it('keeps the plain wording for a turn that did not fail', () => {
    const summary = summarisePayload('chat.turn', {
      provider: 'local',
      cad_provider: 'freecad',
      design_requested: true,
      offline: true,
    });
    expect(summary).toBe('local · CAD freecad · offline · design requested');
  });
});

describe('a failing journal poll', () => {
  it('shows the real error and clears it when a poll succeeds again', () => {
    act(() => {
      dispatch({
        type: 'journal/server-error',
        message: 'request to /api/studio/journal timed out',
      });
    });

    const view = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} />,
    );
    const note = view.container.querySelector('.journal-panel-poll-error');
    expect(note?.textContent).toContain('request to /api/studio/journal timed out');
    expect(note?.textContent).toContain('the ledger below may be stale');

    // A successful page clears the note: the poll recovered.
    act(() => {
      dispatch({ type: 'journal/server-page', page: bracketJournalPage });
    });
    expect(view.container.querySelector('.journal-panel-poll-error')).toBeNull();
  });

  it('says nothing when no poll has failed', () => {
    const view = render(
      <JournalPanel entries={[]} connectionStatus={readyConnection} />,
    );
    expect(view.container.querySelector('.journal-panel-poll-error')).toBeNull();
  });
});

describe('ledger cell layout', () => {  const journalCss = fs.readFileSync(
    path.resolve(__dirname, '../src/styles/journal.css'),
    'utf8',
  );

  function columnWidth(name: 'seq' | 'utc' | 'type' | 'digest' | 'chain'): number {
    const match = new RegExp(`--journal-col-${name}:\\s*(\\d+)px`).exec(journalCss);
    if (!match) throw new Error(`journal.css does not declare --journal-col-${name}`);
    return Number(match[1]);
  }

  const COLUMN = {
    SEQ: columnWidth('seq'),
    UTC: columnWidth('utc'),
    TYPE: columnWidth('type'),
    DIGEST: columnWidth('digest'),
    CHAIN: columnWidth('chain'),
  } as const;

  // Pinned to the sibling rules in journal.css (thead th / tbody td padding,
  // the digest chip's padding and border, the copy button, and the gap).
  const CELL_PADDING = 3 * 2;
  const HEADER_TRACKING = 9 * 0.08;
  const CHAIN_TRACKING = 9 * 0.04;
  const FONT_SIZE = 9;
  // A conservative monospace advance: JetBrains Mono is 0.6em, so every box
  // this test builds can only be too wide, never too narrow.
  const GLYPH = FONT_SIZE * 0.62;
  const CHIP_PADDING = 2 * 2;
  const CHIP_BORDER = 1;
  const DIGEST_GAP = 2;
  const COPY_BUTTON = 16;

  function textWidth(text: string, tracking = 0): number {
    return text.length * (GLYPH + tracking);
  }

  interface Box {
    name: string;
    left: number;
    right: number;
  }

  function cellBoxes(railWidth: number, compact: boolean): Box[] {
    const hidden = new Set(compact ? ['UTC', 'CHAIN'] : []);
    const names = ['SEQ', 'UTC', 'TYPE', 'MESSAGE', 'DIGEST', 'CHAIN'].filter(
      (name) => !hidden.has(name),
    );
    // The ledger is the rail minus the left rule and the sticky header's
    // 6px right gutter; use 8px so the model stays on the tight side.
    const ledgerWidth = railWidth - 8;
    const fixed = names
      .filter((name) => name !== 'MESSAGE')
      .reduce((total, name) => total + COLUMN[name as keyof typeof COLUMN], 0);
    const message = ledgerWidth - fixed;
    const boxes: Box[] = [];
    let left = 0;
    for (const name of names) {
      const width = name === 'MESSAGE' ? message : COLUMN[name as keyof typeof COLUMN];
      boxes.push({ name, left, right: left + width });
      left += width;
    }
    return boxes;
  }

  it.each([
    ['1440', 320, false],
    ['1280', 260, true],
  ] as const)('keeps every cell inside its column at %s', (label, railWidth, compact) => {
    act(() => {
      dispatch({ type: 'journal/server-page', page: bracketJournalB2Page });
      dispatch({ type: 'journal/server-verify', verify: bracketJournalVerified });
    });
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const headerCells = Array.from(
      view.container.querySelectorAll<HTMLElement>('.journal-ledger thead th'),
    );
    expect(headerCells.map((cell) => cell.textContent)).toEqual([
      'SEQ',
      'UTC',
      'TYPE',
      'MESSAGE',
      'DIGEST',
      'CHAIN',
    ]);

    const serverRow = view.container.querySelector<HTMLElement>(
      'tr.journal-entry[data-source="server"]',
    )!;
    const digestText =
      serverRow.querySelector('.journal-entry-digest')?.textContent ?? '';
    const chainText = serverRow.querySelector('.journal-entry-chain')?.textContent ?? '';

    const boxes = cellBoxes(railWidth, compact);
    expect(boxes.map((box) => box.name)).toEqual(
      compact
        ? ['SEQ', 'TYPE', 'MESSAGE', 'DIGEST']
        : ['SEQ', 'UTC', 'TYPE', 'MESSAGE', 'DIGEST', 'CHAIN'],
    );

    const content = new Map<string, number>();
    content.set('SEQ', Math.max(textWidth('SEQ', HEADER_TRACKING), textWidth('999')));
    content.set('UTC', textWidth('16:47:57'));
    content.set('TYPE', textWidth('TYPE', HEADER_TRACKING));
    content.set('MESSAGE', textWidth('MESSAGE', HEADER_TRACKING));
    content.set(
      'DIGEST',
      textWidth(digestText) + CHIP_PADDING + CHIP_BORDER + DIGEST_GAP + COPY_BUTTON,
    );
    content.set(
      'CHAIN',
      Math.max(textWidth('CHAIN', HEADER_TRACKING), textWidth('broken', CHAIN_TRACKING)),
    );

    for (const [index, box] of boxes.entries()) {
      const innerWidth = box.right - box.left - CELL_PADDING;
      expect(innerWidth, `${box.name} column at ${label}`).toBeGreaterThan(0);
      const needed = content.get(box.name) ?? 0;
      // A cell's content box must end before the next column starts. MESSAGE
      // is deliberately ellipsized, so it is allowed to fill its column.
      expect(needed, `${box.name} content at ${label}`).toBeLessThanOrEqual(innerWidth + 0.01);
      const next = boxes[index + 1];
      if (next) {
        const contentRight = box.left + CELL_PADDING / 2 + Math.min(needed, innerWidth);
        expect(contentRight, `${box.name} → ${next.name} at ${label}`).toBeLessThanOrEqual(
          next.left,
        );
      }
    }

    // The MESSAGE column keeps room for the leading provider/verb tokens
    // before its ellipsis, at both reference widths.
    const messageBox = boxes.find((box) => box.name === 'MESSAGE')!;
    expect(messageBox.right - messageBox.left).toBeGreaterThanOrEqual(compact ? 100 : 70);

    // CHAIN is not clipped in the six-column ledger: "ok" and "local" are
    // short, and "broken" must fit the same column.
    expect(chainText).toBe('ok');
    if (!compact) {
      expect(COLUMN.CHAIN - CELL_PADDING).toBeGreaterThanOrEqual(
        textWidth('broken', CHAIN_TRACKING),
      );
    }
  });

  it('keeps the copy button inside the DIGEST cell, not over CHAIN', () => {
    act(() => {
      dispatch({ type: 'journal/server-page', page: bracketJournalB2Page });
      dispatch({ type: 'journal/server-verify', verify: bracketJournalVerified });
    });
    const view = render(
      <JournalPanel entries={[localEntry()]} connectionStatus={readyConnection} />,
    );
    const row = view.container.querySelector<HTMLElement>(
      'tr.journal-entry[data-source="server"]',
    )!;
    const digestCell = row.querySelector('.journal-entry-digest-cell')!;
    const copy = digestCell.querySelector('button[aria-label="Copy digest"]')!;
    expect(copy).not.toBeNull();
    // Structural containment: the chip and the button share the digest line.
    expect(digestCell.querySelector('.journal-entry-digest-line')?.contains(copy)).toBe(true);
    expect(row.querySelector('.journal-entry-chain')?.contains(copy)).toBe(false);

    // The six-column DIGEST budget still fits an 8-character digest, the
    // chip frame, the gap, and the 16px button.
    const chip = digestCell.querySelector('.journal-entry-digest')!;
    const needed =
      textWidth(chip.textContent ?? '') +
      CHIP_PADDING +
      CHIP_BORDER +
      DIGEST_GAP +
      COPY_BUTTON;
    expect(needed).toBeLessThanOrEqual(COLUMN.DIGEST - CELL_PADDING + 0.01);
  });
});

