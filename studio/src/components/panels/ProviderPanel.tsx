import { useEffect, useState } from 'react';
import type { ReactElement } from 'react';
// Provider list + API key setup panel.
//
// Keys are sent once to `POST /api/studio/settings` over loopback and stored
// in the backend's owner-only `studio-settings.json`. The backend never
// echoes secret values back to the browser.
//
// The subscription CLIs (Claude Code, Codex, OpenCode) are listed here too.
// They carry no key: they use the session the operator's own CLI already
// holds, and Kala never sees or stores those credentials.

import {
  getSettings,
  postSettings,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { dispatch, useUiState } from '../../state/store';
import { describeProviderStatus, isSubscriptionProvider } from '../../providerPolicy';
import type { CredentialProviderId, CredentialSource, ProviderCredentialStatus } from '../../types/studio';

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
    // The credential source decides what "cleared" may claim. A key that
    // comes from the environment is not removed by clearing the stored
    // entry, so the message must not claim that it was.
    const wasStored = credentialSourceFor(providerId) === 'stored';
    try {
      const updated = await postSettings(connection.connection, {
        provider: providerId,
        api_key: rawKey,
      });
      setInputs((prev) => ({ ...prev, [providerId]: '' }));
      dispatch({ type: 'server/settings', settings: updated });
      setStatusByProvider((prev) => ({
        ...prev,
        [providerId]: rawKey
          ? 'API key saved on backend.'
          : wasStored
            ? 'Stored API key cleared.'
            : 'The stored API key entry was cleared; the key in the environment is still in force.',
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

  /** The credential source the backend reported for one provider. */
  function credentialSourceFor(providerId: string): CredentialSource | null {
    const entry = credentials.find((cred) => cred.provider === providerId);
    return entry?.source ?? null;
  }

  return (
    <ul className={compact ? 'panel-list panel-list-compact' : 'panel-list'}>
      {providers.map((provider) => {
        const isConfigurable = CONFIGURABLE_PROVIDERS.has(provider.id);
        const subscription = isSubscriptionProvider(provider.id);
        const status = describeProviderStatus(provider);
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
            data-provider-id={provider.id}
            data-provider-state={status.state}
            title={compact && provider.notes ? provider.notes : undefined}
          >
            <header>
              <span className="panel-list-name">{provider.name}</span>
              <span
                className={`panel-list-status panel-list-status-${status.state === 'configured' ? 'on' : 'off'}`}
              >
                {status.badge}
              </span>
            </header>
            {subscription ? (
              <p className="panel-list-notes">
                {provider.source
                  ? `Credential source: ${provider.source}.`
                  : `Uses the ${provider.name} session already signed in on this machine.`}{' '}
                Kala never sees or stores that credential.
              </p>
            ) : null}
            {status.reason ? (
              <p className="panel-list-notes" role="note">
                {status.reason}
              </p>
            ) : null}
            {provider.notes && !compact && !subscription ? (
              <p className="panel-list-notes">{provider.notes}</p>
            ) : null}
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
                  {/* Only a STORED key can be removed here. A key that comes
                      from the environment stays in force whatever this panel
                      does, so the panel must not offer to clear it. */}
                  {cred?.source === 'stored' ? (
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
                {cred?.source === 'environment' ? (
                  <p className="provider-key-source-note" role="note">
                    {`This key comes from the environment (${cred.env_var}) — it is not stored by ` +
                      'Studio and cannot be removed here. Clear it in the environment to revoke it.'}
                  </p>
                ) : null}
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
