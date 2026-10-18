// In-page connection form.
import type { ReactElement } from 'react';
//
// Replaces the previous `window.prompt` bootstrap. The form:
//   - Prefills `baseUrl` from `window.location.origin` (when that origin is
//     a valid loopback URL).
//   - Holds the token in a `type="password"` input. The token stays in
//     component state and is handed to the connection store on submit; it
//     is never written to localStorage, never echoed in the DOM, and never
//     logged.
//   - Rejects non-loopback origins, URLs with credentials, URLs with a
//     query string, and missing tokens before calling `onSubmit`.
//   - Shows an inline error if the operator-supplied values fail
//     validation so the operator can correct them without a page reload.

import { useState, FormEvent } from 'react';
import { currentOriginOrEmpty, normalizeLoopbackOrigin, validateConnection } from '../../api/connection';
import type { StudioConnection } from '../../types/studio';

interface ConnectionFormProps {
  onSubmit(connection: StudioConnection): void;
  errorMessage?: string | null;
}

export function ConnectionForm(props: ConnectionFormProps): ReactElement {
  const [baseUrl, setBaseUrl] = useState<string>(currentOriginOrEmpty());
  const [token, setToken] = useState<string>('');
  const [validationError, setValidationError] = useState<string | null>(null);

  function onSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault();
    setValidationError(null);
    try {
      const normalised = normalizeLoopbackOrigin(baseUrl);
      if (!normalised) {
        setValidationError(
          'Origin must be a loopback URL with a port and no credentials or query (e.g. http://127.0.0.1:8765).',
        );
        return;
      }
      const connection = validateConnection(baseUrl, token, 'browser-manual');
      props.onSubmit(connection);
    } catch (cause) {
      setValidationError(cause instanceof Error ? cause.message : 'invalid connection');
    }
  }

  const error = validationError ?? props.errorMessage ?? null;

  return (
    <form className="connection-form" onSubmit={onSubmit} aria-labelledby="connection-form-title">
      <h2 id="connection-form-title">Connect to Kala Studio backend</h2>
      <p className="connection-form-help">
        The browser fallback keeps the token in this tab only. It is never persisted, logged, or
        echoed back to the server. Reloading the page clears it.
      </p>
      <label className="connection-form-field">
        <span>Backend origin</span>
        <input
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          value={baseUrl}
          onChange={(event) => setBaseUrl(event.target.value.trim())}
          placeholder="http://127.0.0.1:8765"
          aria-invalid={error ? 'true' : 'false'}
          required
        />
      </label>
      <label className="connection-form-field">
        <span>Bearer token</span>
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={token}
          onChange={(event) => setToken(event.target.value)}
          placeholder="Paste the per-launch KALA_STUDIO_TOKEN"
          aria-invalid={error ? 'true' : 'false'}
          required
        />
      </label>
      {error ? (
        <p className="connection-form-error" role="alert">
          {error}
        </p>
      ) : null}
      <button type="submit" className="connection-form-submit">Connect</button>
    </form>
  );
}