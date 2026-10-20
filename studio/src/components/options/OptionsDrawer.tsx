// Options & Engineering drawer.
//
// A modal drafting sheet with four cards:
//   01 Parametric Inspector
//   02 Export & handoff + integration registry
//   03 Model gateway & API providers
//   04 Loopback connection and retry
//
// Accessibility: role="dialog", aria-modal, Escape closes, focus is trapped
// while open, and focus returns to the trigger on close.

import { useEffect, useMemo, useRef } from 'react';
import type { ReactElement } from 'react';
import {
  appendJournal,
  dispatch,
  useUiState,
  selectConnection,
  selectServer,
} from '../../state/store';
import type { OptionsCard, ThemeName } from '../../types/studio';
import { Inspector } from '../inspector/Inspector';
import { ExportPanel } from '../export/ExportPanel';
import { IntegrationsPanel } from '../panels/IntegrationsPanel';
import { ProviderPanel } from '../panels/ProviderPanel';

export interface OptionsDrawerProps {
  open: boolean;
  /** Card to reveal when the drawer opens (Export STEP opens card 02). */
  card: OptionsCard;
  onCardChange(card: OptionsCard): void;
  onClose(): void;
}

const CARDS: ReadonlyArray<{ id: OptionsCard; index: string; title: string }> = [
  { id: 'inspector', index: '01', title: 'Parametric Inspector & Primitive Controls' },
  { id: 'export', index: '02', title: 'Neutral-File Export & CAD/CAE Handoff' },
  { id: 'providers', index: '03', title: 'Model Gateway & API Providers' },
  { id: 'connection', index: '04', title: 'Loopback Boot Gate & Recovery' },
];

/** Graphite line icons for the four card headers (no icon font). */
const CARD_ICONS: Record<OptionsCard, ReactElement> = {
  inspector: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="2.2" />
      <path d="M8 2.2v1.4M8 12.4v1.4M2.2 8h1.4M12.4 8h1.4M3.9 3.9l1 1M11.1 11.1l1 1M12.1 3.9l-1 1M4.9 11.1l-1 1" />
    </svg>
  ),
  export: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M3 2.5h7l3 3v8H3zM10 2.5v3h3M5.5 10.5h5M8 6.5v4M6.5 9l1.5 1.5L9.5 9" />
    </svg>
  ),
  providers: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="5" cy="8" r="2.2" />
      <path d="M7.2 8h6.3M11 5.8 13.5 8 11 10.2M2.8 8H1" />
    </svg>
  ),
  connection: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M2 5.5h8.5L8.5 3.5M14 10.5H5.5l2 2M10.5 5.5l2 2M5.5 10.5l-2-2" />
    </svg>
  ),
};

export function OptionsDrawer(props: OptionsDrawerProps): ReactElement | null {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeRef = useRef<HTMLButtonElement | null>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(props.onClose);
  onCloseRef.current = props.onClose;
  const theme = useUiState((s) => s.theme);
  // Card notes must not claim a port or a capability the loopback connection
  // has not reported. Both values come from the live store, with an explicit
  // empty state before the registries answer.
  const connection = useUiState(selectConnection);
  const server = useUiState(selectServer);
  const preview = useUiState((s) => s.preview);
  const inspectorNote = preview.server
    ? 'SERVER VERIFIED'
    : preview.local
      ? 'LOCAL PREVIEW'
      : 'NO MESH';
  const connectionNote = useMemo(() => {
    if (connection.kind !== 'ready') return 'NO LOOPBACK';
    try {
      const url = new URL(connection.connection.baseUrl);
      const port = url.port || (url.protocol === 'https:' ? '443' : '80');
      return `PORT ${port}`;
    } catch {
      return 'LOOPBACK';
    }
  }, [connection]);
  const exportNote =
    server.health === 'unknown'
      ? 'STEP —'
      : server.stepAvailable
        ? 'ISO 10303 READY'
        : 'STEP UNAVAILABLE';

  useEffect(() => {
    if (!props.open) return;
    const previous = document.activeElement;
    returnFocusRef.current = previous instanceof HTMLElement ? previous : null;
    closeRef.current?.focus();

    function focusables(): HTMLElement[] {
      const dialog = dialogRef.current;
      if (!dialog) return [];
      return Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.getAttribute('aria-hidden') !== 'true');
    }

    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const items = focusables();
      if (items.length === 0) return;
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === dialogRef.current)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('keydown', onKeyDown, true);
      returnFocusRef.current?.focus();
      returnFocusRef.current = null;
    };
  }, [props.open]);

  useEffect(() => {
    if (!props.open) return;
    const element = document.getElementById(`options-card-${props.card}`);
    if (element && typeof element.scrollIntoView === 'function') {
      // Align the card header with the top of the drawer's scroll viewport.
      // `block: 'nearest'` is wrong here: the grid rows are as tall as the
      // tallest card, so a short card in a tall row can be "partly visible"
      // while its actual content is entirely above the fold.
      element.scrollIntoView({ block: 'start' });
    }
  }, [props.open, props.card]);

  if (!props.open) return null;

  return (
    <div
      className="options-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) props.onClose();
      }}
    >
      <div
        className="options-drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="options-drawer-title"
        ref={dialogRef}
      >
        <header className="options-drawer-header">
          <div className="options-drawer-heading">
            <span className="options-drawer-mark" aria-hidden="true" />
            <div>
              <h2 id="options-drawer-title">Kala Studio // Options &amp; Engineering Drawer</h2>
              <p>Secondary controls, kernel overrides, connection recovery, and handoff protocols.</p>
            </div>
          </div>
          <div className="options-drawer-actions">
            <ThemeSwitch theme={theme} />
            <button ref={closeRef} type="button" className="options-drawer-close" onClick={props.onClose}>
              ✕ Close / Dock to Scale Ruler [Esc]
            </button>
          </div>
        </header>

        <nav className="options-drawer-nav" aria-label="Options cards">
          {CARDS.map((entry) => (
            <button
              key={entry.id}
              type="button"
              className={`options-drawer-nav-item${props.card === entry.id ? ' is-active' : ''}`}
              aria-pressed={props.card === entry.id}
              onClick={() => props.onCardChange(entry.id)}
            >
              {entry.index} // {entry.title}
            </button>
          ))}
        </nav>

        <div className="options-drawer-grid">
          <article
            className={`options-card${props.card === 'inspector' ? ' is-focused' : ''}`}
            id="options-card-inspector"
            data-card="inspector"
            aria-labelledby="options-card-inspector-title"
          >
            <CardCorners />
            <CardHeader card="inspector" index="01" title="Parametric Inspector & Primitive Controls" note={inspectorNote} />
            <div className="options-card-body">
              <Inspector showHeading={false} />
            </div>
          </article>

          <article
            className={`options-card${props.card === 'export' ? ' is-focused' : ''}`}
            id="options-card-export"
            data-card="export"
            aria-labelledby="options-card-export-title"
          >
            <CardCorners />
            <CardHeader card="export" index="02" title="Neutral-File Export & CAD/CAE Handoff" note={exportNote} />
            <div className="options-card-body">
              <ExportPanel showHeading={false} />
              <section className="options-card-registry" aria-label="Integration registry">
                <h3>Integration registry</h3>
                <IntegrationsPanel compact />
              </section>
            </div>
          </article>

          <article
            className={`options-card${props.card === 'providers' ? ' is-focused' : ''}`}
            id="options-card-providers"
            data-card="providers"
            aria-labelledby="options-card-providers-title"
          >
            <CardCorners />
            <CardHeader card="providers" index="03" title="Model Gateway & API Providers" note="[LOCAL VAULT]" />
            <div className="options-card-body">
              <p className="options-card-note">
                Keys are sent once to the local Studio backend and stored server-side only. They are
                never echoed back to the browser and never enter localStorage.
              </p>
              <ProviderPanel compact />
            </div>
          </article>

          <article
            className={`options-card${props.card === 'connection' ? ' is-focused' : ''}`}
            id="options-card-connection"
            data-card="connection"
            aria-labelledby="options-card-connection-title"
          >
            <CardCorners />
            <CardHeader card="connection" index="04" title="Loopback Boot Gate & Recovery" note={connectionNote} />
            <div className="options-card-body">
              <ConnectionCard />
              <p className="options-card-threat">
                The bearer token is held in memory for this session only. It is never persisted to
                disk, never logged, and never written to localStorage.
              </p>
            </div>
          </article>
        </div>

        <footer className="options-drawer-title-block" aria-label="Options sheet title block">
          <div className="options-title-cell">
            <span>STUDIO</span>
            <strong>KALA STUDIO CAD/CAE</strong>
          </div>
          <div className="options-title-cell">
            <span>SHEET ID</span>
            <strong>KALA-UI-OPT-04</strong>
          </div>
          <div className="options-title-cell options-title-cell-wide">
            <span>DRAWING TITLE</span>
            <strong>Options &amp; Engineering Drawer</strong>
          </div>
          <div className="options-title-cell" title="Not to scale — this is an on-screen controls sheet.">
            <span>SCALE</span>
            <strong>NTS</strong>
          </div>
          <div className="options-title-cell">
            <span>BACKEND</span>
            <strong>{server.version ? `v${server.version}` : '—'}</strong>
          </div>
          <div className="options-title-cell options-title-cell-wide" title={connection.kind === 'ready' ? connection.connection.baseUrl : 'Not connected'}>
            <span>CONNECTION ORIGIN</span>
            <strong>{connection.kind === 'ready' ? connection.connection.baseUrl : '—'}</strong>
          </div>
        </footer>
      </div>
    </div>
  );
}

function CardCorners(): ReactElement {
  return (
    <>
      <span className="options-card-tick options-card-tick-tl" aria-hidden="true">+</span>
      <span className="options-card-tick options-card-tick-tr" aria-hidden="true">+</span>
      <span className="options-card-tick options-card-tick-bl" aria-hidden="true">+</span>
      <span className="options-card-tick options-card-tick-br" aria-hidden="true">+</span>
    </>
  );
}

function CardHeader(props: { card: OptionsCard; index: string; title: string; note: string }): ReactElement {
  return (
    <header className="options-card-header">
      <div className="options-card-heading">
        <span className="options-card-icon">{CARD_ICONS[props.card]}</span>
        <h3 id={`options-card-${props.card}-title`}>
          {props.index} // {props.title}
        </h3>
      </div>
      <span className="options-card-note-value">{props.note}</span>
    </header>
  );
}

function ThemeSwitch(props: { theme: ThemeName }): ReactElement {
  const options: ReadonlyArray<{ id: ThemeName; label: string }> = [
    { id: 'vellum', label: 'Vellum' },
    { id: 'slate', label: 'Slate' },
  ];
  return (
    <div className="options-theme-switch" role="group" aria-label="Workspace theme">
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          className={`options-theme-button${props.theme === option.id ? ' is-active' : ''}`}
          aria-pressed={props.theme === option.id}
          onClick={() => dispatch({ type: 'theme/set', theme: option.id })}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function ConnectionCard(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const server = useUiState((s) => s.server);
  const ready = connection.kind === 'ready';
  return (
    <div className="options-connection">
      <dl className="options-connection-facts">
        <div>
          <dt>Status</dt>
          <dd data-status={connection.kind}>
            {connection.kind === 'ready'
              ? 'Connected'
              : connection.kind === 'connecting'
                ? 'Connecting…'
                : connection.kind === 'error'
                  ? `Error · ${connection.message}`
                  : 'Not connected'}
          </dd>
        </div>
        <div>
          <dt>Origin</dt>
          <dd>{ready ? connection.connection.baseUrl : '—'}</dd>
        </div>
        <div>
          <dt>Mode</dt>
          <dd>{ready ? (connection.connection.mode === 'desktop' ? 'Desktop shell' : 'Browser fallback') : '—'}</dd>
        </div>
        <div>
          <dt>Backend</dt>
          <dd>{server.version ? `v${server.version}` : '—'}</dd>
        </div>
        <div>
          <dt>Health</dt>
          <dd data-health={server.health}>{server.health === 'ok' ? 'OK' : server.health === 'unreachable' ? 'Unreachable' : 'Unknown'}</dd>
        </div>
        <div>
          <dt>STEP export</dt>
          <dd>{server.health === 'unknown' ? 'Unknown' : server.stepAvailable ? 'Available' : 'Unavailable'}</dd>
        </div>
        <div>
          <dt>Token</dt>
          <dd>In memory only (not displayed)</dd>
        </div>
      </dl>
      <button
        type="button"
        className="options-connection-retry"
        onClick={() => {
          appendJournal({ kind: 'connection', message: 'Reconnect / retry requested from Options' });
          dispatch({ type: 'connection/retry' });
        }}
      >
        Reconnect / Retry Bridge
      </button>
      <p className="options-card-note">
        If the desktop bridge is unavailable, reload the app to reach the in-page loopback form.
        Manual browser connect never persists the token.
      </p>
    </div>
  );
}
