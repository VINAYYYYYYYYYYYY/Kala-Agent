import type { ReactElement } from 'react';
// Header bar with connection status, mode, and live/offline chip.

import { useUiState } from '../../state/store';

export function Header(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const server = useUiState((s) => s.server);
  const status: 'online' | 'offline' | 'unknown' =
    connection.kind === 'ready' && server.health === 'ok'
      ? 'online'
      : connection.kind === 'ready'
        ? 'offline'
        : 'unknown';
  const statusLabel = status === 'online' ? 'Live' : status === 'offline' ? 'Offline' : 'Connecting';

  return (
    <header className="app-header" role="banner">
      <div className="app-header-brand">
        <span className="app-header-mark" aria-hidden="true">⚙</span>
        <h1>Kala Studio</h1>
        <span className="app-header-tagline">chat-first mechanical engineering agent</span>
      </div>
      <div className="app-header-status" aria-live="polite">
        <span className={`app-header-chip app-header-chip-${status}`} data-status={status}>
          <span className="app-header-chip-dot" aria-hidden="true" />
          {statusLabel}
        </span>
        <span className="app-header-mode">
          {connection.kind === 'ready'
            ? connection.connection.mode === 'desktop'
              ? 'Desktop shell'
              : 'Browser fallback (token in memory)'
            : 'Awaiting connection'}
        </span>
        {server.version ? <span className="app-header-mode-detail">v{server.version}</span> : null}
      </div>
    </header>
  );
}