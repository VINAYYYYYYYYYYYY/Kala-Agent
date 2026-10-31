// The composer's CAD target selector (ADR-0011).
//
// The backend probes the host and answers `GET /providers/cad`; this control
// renders exactly those rows. There is NO default target: nothing is checked
// until the operator clicks one, so the first design turn produces the
// backend's "choose a provider" reply instead of silently building with a
// provider the operator did not pick.
//
// An unavailable provider is disabled and carries its backend reason in the
// tooltip. The selection lives in `state.cadProvider` only: it is never
// written to localStorage and disappears with the session.

import { useEffect, useState } from 'react';
import type { ReactElement } from 'react';
import {
  getCadProviders,
  StudioAbortError,
  StudioApiError,
  StudioNetworkError,
} from '../../api/client';
import { dispatch, setCadProvider, useUiState } from '../../state/store';
import type { CadProviderRecord } from '../../types/studio';

export function CadTargetSelector(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const health = useUiState((s) => s.server.health);
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const providers = useUiState((s) => s.server.cadProviders);
  const status = useUiState((s) => s.server.cadProvidersStatus);
  const error = useUiState((s) => s.server.cadProvidersError);
  const selected = useUiState((s) => s.cadProvider);
  const [retryToken, setRetryToken] = useState(0);

  useEffect(() => {
    if (connection.kind !== 'ready') return;
    // The probe belongs to the design-turn surface. An older backend (or one
    // that cannot import OCP) has no provider route, so it is not called.
    if (designAvailable !== true) return;
    const controller = new AbortController();
    dispatch({ type: 'server/cad-providers-start' });
    getCadProviders(connection.connection, controller.signal)
      .then((response) => {
        if (controller.signal.aborted) return;
        dispatch({ type: 'server/cad-providers', providers: response.providers });
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted || cause instanceof StudioAbortError) return;
        dispatch({ type: 'server/cad-providers-error', message: describeError(cause) });
      });
    return () => controller.abort();
  }, [
    connection.kind === 'ready' ? connection.connection : null,
    designAvailable,
    retryToken,
  ]);

  if (designAvailable !== true) {
    return (
      <section className="chat-panel-cad-target" aria-label="CAD target">
        <span className="chat-panel-cad-target-label">CAD target</span>
        <p className="chat-panel-cad-target-note" role={designAvailable === false ? 'note' : 'status'}>
          {designAvailable === false
            ? 'CAD providers need the Python geometry backend; this backend reports design_available: false.'
            : health === 'ok'
              ? 'CAD providers need the Python geometry backend; this backend did not report design_available.'
              : 'CAD providers load once the backend reports design-turn support.'}
        </p>
      </section>
    );
  }

  const selectedRecord = selected === null
    ? null
    : providers.find((record) => record.id === selected) ?? null;

  return (
    <section className="chat-panel-cad-target" aria-label="CAD target">
      <span className="chat-panel-cad-target-label">CAD target</span>
      {status === 'loading' || status === 'idle' ? (
        <p className="chat-panel-cad-target-note" role="status">
          Probing CAD providers on this host…
        </p>
      ) : null}
      {status === 'error' ? (
        <p className="chat-panel-cad-target-error" role="alert">
          {error ?? 'The CAD provider probe failed.'}{' '}
          <button
            type="button"
            className="chat-panel-cad-target-retry"
            onClick={() => setRetryToken((token) => token + 1)}
          >
            Retry
          </button>
        </p>
      ) : null}
      {status === 'ready' && providers.length === 0 ? (
        <p className="chat-panel-cad-target-note" role="note">
          The backend reported no CAD providers.
        </p>
      ) : null}
      {providers.length > 0 ? (
        <div className="chat-panel-cad-target-options" role="radiogroup" aria-label="CAD target">
          {providers.map((provider) => (
            <ProviderOption
              key={provider.id}
              provider={provider}
              selected={selected === provider.id}
            />
          ))}
        </div>
      ) : null}
      {selectedRecord?.available === false ? (
        <p className="chat-panel-cad-target-error" role="alert">
          {cadProviderDisplayName(selectedRecord)} is not available:{' '}
          {selectedRecord.reason ?? 'no reason reported'}.
        </p>
      ) : null}
      {status === 'ready' && selected !== null && selectedRecord === null ? (
        <p className="chat-panel-cad-target-error" role="alert">
          The backend no longer reports the selected CAD target ({selected}).
        </p>
      ) : null}
      {status === 'ready' && selected === null ? (
        <p className="chat-panel-cad-target-hint" role="note">
          No CAD target selected — the first design turn will ask you to choose.
        </p>
      ) : null}
    </section>
  );
}

function ProviderOption(props: {
  provider: CadProviderRecord;
  selected: boolean;
}): ReactElement {
  const { provider } = props;
  const unavailable = !provider.available;
  const name = cadProviderDisplayName(provider);
  const version = provider.version?.trim();
  const label = version ? `${name} ${version}` : name;
  const tooltip = unavailable
    ? `${name} is unavailable: ${provider.reason ?? 'no reason reported'}`
    : `${label} — builds and measures this design`;
  return (
    <span className="chat-panel-cad-target-option" title={tooltip}>
      <button
        type="button"
        role="radio"
        aria-checked={props.selected}
        aria-label={unavailable ? `${label} (unavailable)` : label}
        className={`chat-panel-cad-target-button${props.selected ? ' is-active' : ''}`}
        data-cad-provider={provider.id}
        data-available={provider.available ? 'yes' : 'no'}
        data-selected={props.selected ? 'yes' : 'no'}
        disabled={unavailable}
        title={tooltip}
        onClick={() => setCadProvider(provider.id)}
      >
        <span className="chat-panel-cad-target-name">{name}</span>
        {version ? <span className="chat-panel-cad-target-version">{version}</span> : null}
        {unavailable ? <span className="chat-panel-cad-target-unavailable">unavailable</span> : null}
      </button>
    </span>
  );
}

/**
 * The opt-in status is part of OpenCascade's identity (ADR-0011): it is
 * never the default builder. The backend name already carries it, but the
 * label is enforced here so a front door that omits the suffix cannot make
 * the opt-in provider look automatic.
 */
export function cadProviderDisplayName(provider: CadProviderRecord): string {
  if (provider.id === 'opencascade' && !/opt-in/i.test(provider.name)) {
    return `${provider.name} (opt-in)`;
  }
  return provider.name;
}

function describeError(cause: unknown): string {
  if (cause instanceof StudioApiError) {
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return `${cause.code}: ${cause.message}${details}`;
  }
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'the CAD provider probe failed';
}
