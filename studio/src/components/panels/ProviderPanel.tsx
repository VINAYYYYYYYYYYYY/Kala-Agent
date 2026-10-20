import { useEffect, useState } from 'react';
import type { ReactElement } from 'react';
// Provider list + API key setup panel.
//
// Keys are sent once to `POST /api/studio/settings` over loopback and stored
// in the backend's owner-only `studio-settings.json`. The backend never
// echoes secret values back to the browser.

import {
  getSettings,
  postSettings,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { dispatch, useUiState } from '../../state/store';
import type { CredentialProviderId, ProviderCredentialStatus } from '../../types/studio';

const CONFIGURABLE_PROVIDERS: ReadonlySet<string> = new Set(['openai', 'openrouter']);

export function ProviderPanel({ compact = false }: { compact?: boolean }): ReactElement {
  const connection = useUiState((s) => s.connection);
  const providers = useUiState((s) => s.server.providers);
  const credentials = useUiState((s) => s.server.credentials);
  const [inputs, setInputs] = useState<Record<string, string>>({});
  const [busyProvider, setBusyProvider] = useState<string | null>(null);
  const [statusByProvider, setStatusByProvider] = useState<Record<string, string>>({});
  const [errorByProvider, setErrorByProvider] = useState<Record<string, string>>({});

  useEffect(() => {
    if (connection.kind !== 'ready') return;
    const controller = new AbortController();
    getSettings(connection.connection, controller.signal)
      .then((settings) => {
        if (!controller.signal.aborted && settings && Array.isArray(settings.credentials)) {
          dispatch({ type: 'server/settings', settings });
        }
      })
      .catch(() => {
        // Older or unit-test mocks that do not implement /api/studio/settings
        // gracefully fall back to the /api/studio/providers snapshot.
      });
    return () => controller.abort();
  }, [connection]);

  if (providers.length === 0) {
    return <p className="panel-empty">No providers registered on the backend yet.</p>;
  }

  async function handleSaveKey(providerId: CredentialProviderId, rawKey: string | null): Promise<void> {
    if (connection.kind !== 'ready' || busyProvider !== null) return;
    setBusyProvider(providerId);
    setStatusByProvider((prev) => ({ ...prev, [providerId]: '' }));
    setErrorByProvider((prev) => ({ ...prev, [providerId]: '' }));
    try {
      const updated = await postSettings(connection.connection, {
        provider: providerId,
        api_key: rawKey,
      });
      setInputs((prev) => ({ ...prev, [providerId]: '' }));
      dispatch({ type: 'server/settings', settings: updated });
      setStatusByProvider((prev) => ({
        ...prev,
        [providerId]: rawKey ? 'API key saved on backend.' : 'API key cleared.',
      }));
    } catch (cause) {
      setErrorByProvider((prev) => ({
        ...prev,
        [providerId]: formatError(cause),
      }));
    } finally {
      setBusyProvider(null);
    }
  }

  return (
    <ul className={compact ? 'panel-list panel-list-compact' : 'panel-list'}>
      {providers.map((provider) => {
        const isConfigurable = CONFIGURABLE_PROVIDERS.has(provider.id);
        const cred: ProviderCredentialStatus | undefined = credentials.find(
          (c) => c.provider === provider.id,
        );
        const inputValue = inputs[provider.id] ?? '';
        const isBusy = busyProvider === provider.id;
        const statusMsg = statusByProvider[provider.id] ?? '';
        const errorMsg = errorByProvider[provider.id] ?? '';

        return (
          <li
            key={provider.id}
            className="panel-list-item"
            title={compact && provider.notes ? provider.notes : undefined}
          >
            <header>
              <span className="panel-list-name">{provider.name}</span>
              <span
                className={`panel-list-status panel-list-status-${provider.configured ? 'on' : 'off'}`}
              >
                {provider.configured ? 'Configured server-side' : 'Not configured'}
              </span>
            </header>
            {provider.notes && !compact ? <p className="panel-list-notes">{provider.notes}</p> : null}
            {isConfigurable ? (
              <form
                className="provider-key-form"
                onSubmit={(event) => {
                  event.preventDefault();
                  const trimmed = inputValue.trim();
                  if (!trimmed) return;
                  void handleSaveKey(provider.id as CredentialProviderId, trimmed);
                }}
              >
                <div className="provider-key-meta">
                  <span className="provider-key-source">
                    {cred?.source === 'stored'
                      ? `Key stored in Studio settings (${cred.env_var})`
                      : cred?.source === 'environment'
                        ? `Key loaded from environment (${cred.env_var})`
                        : `Set ${provider.id === 'openai' ? 'OPENAI_API_KEY' : 'OPENROUTER_API_KEY'}`}
                  </span>
                </div>
                <div className="provider-key-controls">
                  <input
                    type="password"
                    className="provider-key-input"
                    autoComplete="off"
                    spellCheck={false}
                    aria-label={`${provider.name} API key`}
                    placeholder={provider.id === 'openai' ? 'sk-...' : 'sk-or-...'}
                    value={inputValue}
                    disabled={isBusy}
                    onChange={(event) =>
                      setInputs((prev) => ({ ...prev, [provider.id]: event.target.value }))
                    }
                  />
                  <button
                    type="submit"
                    className="provider-key-save"
                    disabled={isBusy || inputValue.trim().length === 0}
                  >
                    {isBusy ? 'Saving…' : 'Save key'}
                  </button>
                  {provider.configured ? (
                    <button
                      type="button"
                      className="provider-key-clear"
                      disabled={isBusy}
                      onClick={() => {
                        void handleSaveKey(provider.id as CredentialProviderId, null);
                      }}
                    >
                      Clear key
                    </button>
                  ) : null}
                </div>
                {statusMsg ? (
                  <p className="provider-key-status" role="status">
                    {statusMsg}
                  </p>
                ) : null}
                {errorMsg ? (
                  <p className="provider-key-error" role="alert">
                    {errorMsg}
                  </p>
                ) : null}
              </form>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}

function formatError(cause: unknown): string {
  if (cause instanceof StudioAuthError) return `auth: ${cause.message} (${cause.status})`;
  if (cause instanceof StudioApiError) return `${cause.code}: ${cause.message}`;
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'failed to update API key settings';
}
