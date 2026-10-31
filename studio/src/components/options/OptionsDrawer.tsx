// Settings dialog.
//
// The old "Options & Engineering Drawer" was a four-card dashboard: a
// parametric inspector, the export panel, the provider keys and the loopback
// connection, all visible at once. The app-feel wave moves each of those to
// its one home:
//   * the primitive inspector lives in Details › Properties,
//   * export has its own focused dialog,
//   * this dialog keeps the four secondary sections:
//       Workspace, Models & keys, CAD, Connection.
//
// Accessibility: role="dialog", aria-modal, Escape closes, focus is trapped
// while open, and focus returns to the trigger on close.

import { useEffect, useRef } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactElement, ReactNode } from 'react';
import {
  appendJournal,
  dispatch,
  getState,
  useUiState,
  selectCadProviders,
} from '../../state/store';
import type { OptionsCard, ThemeName } from '../../types/studio';
import {
  getCadProviders,
  StudioAbortError,
  StudioApiError,
  StudioNetworkError,
} from '../../api/client';
import { IntegrationsPanel } from '../panels/IntegrationsPanel';
import { ProviderPanel } from '../panels/ProviderPanel';

export interface OptionsDrawerProps {
  open: boolean;
  /** The settings section to reveal when the dialog opens. */
  card: OptionsCard;
  onCardChange(card: OptionsCard): void;
  onClose(): void;
}

interface SectionEntry {
  id: OptionsCard;
  index: string;
  label: string;
  note: string;
}

const SECTIONS: ReadonlyArray<SectionEntry> = [
  {
    id: 'workspace',
    index: '01',
    label: 'Workspace',
    note: 'Choose the paper the whole studio is drawn on; the choice is remembered on this machine.',
  },
  {
    id: 'providers',
    index: '02',
    label: 'Models & keys',
    note: 'The model routes this studio can call, and the keys each one uses.',
  },
  {
    id: 'cad',
    index: '03',
    label: 'CAD',
    note: 'The CAD and analysis tools this backend registers, and its live probe of each provider.',
  },
  {
    id: 'connection',
    index: '04',
    label: 'Connection',
    note: 'The loopback bridge between this window and the Studio backend.',
  },
];

/** The eleven-pixel registration mark reused from the old drawer header. */
function SettingsMark(): ReactElement {
  return <span className="settings-mark" aria-hidden="true" />;
}

export function OptionsDrawer(props: OptionsDrawerProps): ReactElement | null {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeRef = useRef<HTMLButtonElement | null>(null);
  const tabRefs = useRef(new Map<OptionsCard, HTMLButtonElement>());
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(props.onClose);
  onCloseRef.current = props.onClose;
  const { open, onCardChange } = props;

  useEffect(() => {
    if (!open) return;
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
  }, [open]);

  if (!open) return null;

  const active = SECTIONS.find((entry) => entry.id === props.card) ?? SECTIONS[0]!;

  function onNavKeyDown(event: ReactKeyboardEvent<HTMLElement>): void {
    const ids = SECTIONS.map((entry) => entry.id);
    const current = ids.indexOf(active.id);
    let next: number | null = null;
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
      next = (current + 1) % ids.length;
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
      next = (current - 1 + ids.length) % ids.length;
    } else if (event.key === 'Home') {
      next = 0;
    } else if (event.key === 'End') {
      next = ids.length - 1;
    }
    if (next === null) return;
    event.preventDefault();
    const target = ids[next]!;
    onCardChange(target);
    tabRefs.current.get(target)?.focus();
  }

  return (
    <div
      className="app-modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) props.onClose();
      }}
    >
      <div
        ref={dialogRef}
        className="settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
      >
        <header className="settings-header">
          <div className="settings-heading">
            <SettingsMark />
            <h2 id="settings-title" className="settings-title">
              Settings
            </h2>
          </div>
          <button
            ref={closeRef}
            type="button"
            className="settings-close"
            aria-label="Close settings"
            onClick={props.onClose}
          >
            Close [Esc]
          </button>
        </header>

        <div className="settings-body">
          <nav
            className="settings-nav"
            role="tablist"
            aria-orientation="vertical"
            aria-label="Settings sections"
            onKeyDown={onNavKeyDown}
          >
            {SECTIONS.map((entry) => {
              const selected = entry.id === active.id;
              return (
                <button
                  key={entry.id}
                  ref={(node) => {
                    if (node) tabRefs.current.set(entry.id, node);
                    else tabRefs.current.delete(entry.id);
                  }}
                  type="button"
                  role="tab"
                  id={`settings-tab-${entry.id}`}
                  className={`settings-nav-item${selected ? ' is-active' : ''}`}
                  data-section={entry.id}
                  aria-selected={selected}
                  aria-controls={selected ? `settings-panel-${entry.id}` : undefined}
                  tabIndex={selected ? 0 : -1}
                  onClick={() => onCardChange(entry.id)}
                >
                  <span className="settings-nav-index" aria-hidden="true">
                    {entry.index}
                  </span>
                  <span>{entry.label}</span>
                </button>
              );
            })}
          </nav>

          <section
            key={active.id}
            className="settings-section"
            role="tabpanel"
            id={`settings-panel-${active.id}`}
            data-section={active.id}
            aria-labelledby={`settings-tab-${active.id}`}
          >
            <header className="settings-section-header">
              <h3 className="settings-section-heading">{active.label}</h3>
              <p className="settings-section-note">{active.note}</p>
            </header>
            <div className="settings-section-content">
              <SectionBody card={active.id} />
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function SectionBody(props: { card: OptionsCard }): ReactElement {
  switch (props.card) {
    case 'workspace':
      return <WorkspaceSection />;
    case 'providers':
      return <ProvidersSection />;
    case 'cad':
      return <CadSection />;
    case 'connection':
      return <ConnectionSection />;
  }
}

function WorkspaceSection(): ReactElement {
  const theme = useUiState((s) => s.theme);
  return (
    <div className="settings-stack">
      <div className="settings-row">
        <div className="settings-row-label">
          <span>Theme</span>
          <strong>{theme === 'slate' ? 'Slate' : 'Vellum'}</strong>
        </div>
        <ThemeSwitch theme={theme} />
      </div>
    </div>
  );
}

function ProvidersSection(): ReactElement {
  return (
    <div className="settings-stack">
      <p className="settings-honesty">
        Keys are sent once to the local Studio backend and stored server-side only. They are
        never echoed back to the browser and never enter localStorage.
      </p>
      <ProviderPanel compact />
    </div>
  );
}

function CadSection(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const providers = useUiState(selectCadProviders);
  const status = useUiState((s) => s.server.cadProvidersStatus);
  const error = useUiState((s) => s.server.cadProvidersError);

  // The composer owns the CAD pick, but it only mounts its provider selector
  // when its chip is opened. Settings must not show "not reported yet" on a
  // live backend just because nobody opened that popover, so the first visit
  // asks the same read-only probe the composer uses. An existing result —
  // including an error — is never re-fetched here; the composer owns retry.
  useEffect(() => {
    if (connection.kind !== 'ready' || designAvailable !== true) return;
    if (getState().server.cadProvidersStatus !== 'idle') return;
    const controller = new AbortController();
    dispatch({ type: 'server/cad-providers-start' });
    getCadProviders(connection.connection, controller.signal)
      .then((response) => {
        if (controller.signal.aborted) return;
        dispatch({ type: 'server/cad-providers', providers: response.providers });
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted || cause instanceof StudioAbortError) return;
        dispatch({ type: 'server/cad-providers-error', message: describeProbeError(cause) });
      });
    return () => controller.abort();
  }, [connection, designAvailable]);

  const emptyLabel =
    status === 'loading'
      ? 'Probing the CAD providers…'
      : status === 'error'
        ? `CAD provider probe failed${error ? `: ${error}` : ''}`
        : 'The backend has not reported its CAD providers yet.';
  return (
    <div className="settings-stack">
      <SettingsBlock title="Integration registry">
        <IntegrationsPanel compact />
      </SettingsBlock>
      <SettingsBlock title="Live CAD providers">
        {providers.length === 0 ? (
          <p className="panel-empty">{emptyLabel}</p>
        ) : (
          <ul className="panel-list panel-list-compact" aria-label="Live CAD providers">
            {providers.map((provider) => (
              <li
                key={provider.id}
                className="panel-list-item"
                data-cad-provider={provider.id}
                data-cad-available={provider.available ? 'true' : 'false'}
              >
                <header>
                  <span className="panel-list-name">{provider.name}</span>
                  <span
                    className={`panel-list-status ${
                      provider.available ? 'panel-list-status-on' : 'panel-list-status-off'
                    }`}
                  >
                    {provider.available ? 'Available' : 'Unavailable'}
                  </span>
                </header>
                <p className="panel-list-meta">
                  Version {provider.version ?? '—'}
                </p>
                {!provider.available && provider.reason ? (
                  <p className="panel-list-notes" role="note">
                    {provider.reason}
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </SettingsBlock>
    </div>
  );
}

function describeProbeError(cause: unknown): string {
  if (cause instanceof StudioApiError) {
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return `${cause.code}: ${cause.message}${details}`;
  }
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'the CAD provider probe failed';
}

function SettingsBlock(props: { title: string; children: ReactNode }): ReactElement {
  return (
    <section className="settings-block">
      <h4 className="settings-subheading">{props.title}</h4>
      {props.children}
    </section>
  );
}

function ConnectionSection(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const server = useUiState((s) => s.server);
  const ready = connection.kind === 'ready';
  return (
    <div className="settings-stack">
      <dl className="settings-connection-facts">
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
          <dd data-health={server.health}>
            {server.health === 'ok' ? 'OK' : server.health === 'unreachable' ? 'Unreachable' : 'Unknown'}
          </dd>
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
        className="settings-connection-retry"
        onClick={() => {
          appendJournal({ kind: 'connection', message: 'Reconnect / retry requested from Settings' });
          dispatch({ type: 'connection/retry' });
        }}
      >
        Reconnect / Retry Bridge
      </button>
      <p className="settings-connection-note">
        If the desktop bridge is unavailable, reload the app to reach the in-page loopback form.
        Manual browser connect never persists the token.
      </p>
      <p className="settings-connection-note settings-connection-note-boxed">
        The bearer token is held in memory for this session only. It is never persisted to
        disk, never logged, and never written to localStorage.
      </p>
    </div>
  );
}

function ThemeSwitch(props: { theme: ThemeName }): ReactElement {
  const options: ReadonlyArray<{ id: ThemeName; label: string }> = [
    { id: 'vellum', label: 'Vellum' },
    { id: 'slate', label: 'Slate' },
  ];
  return (
    <div className="settings-theme-switch" role="group" aria-label="Workspace theme">
      {options.map((option) => (
        <button
          key={option.id}
          type="button"
          className={`settings-theme-button${props.theme === option.id ? ' is-active' : ''}`}
          aria-pressed={props.theme === option.id}
          onClick={() => dispatch({ type: 'theme/set', theme: option.id })}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
