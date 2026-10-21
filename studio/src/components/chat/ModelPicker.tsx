// The composer's model picker.
//
// The list comes from `GET /api/studio/providers/{id}/models`. Listing the
// models of a remote provider is remote egress, exactly like a chat turn,
// so the request carries the operator's consent stamp (`?allow_remote=…`).
// The UI NEVER sets that flag on its own: it passes the stamp the operator
// ticked, and the backend refuses the request when the flag is absent. A
// refusal arrives as a 403 and is shown as the consent-required state, with
// a button that ticks the stamp the operator already has in the composer.
//
// Every state is explicit:
//   * no provider selected,
//   * the provider has no credential yet,
//   * the list is loading,
//   * the backend refused without the consent stamp,
//   * the fetch failed (with the backend's own message and a retry),
//   * the provider reported no models,
//   * the search matched nothing,
//   * the list.
//
// The choice is remembered per provider in localStorage. Only the MODEL ID
// is stored — never a key, never a credential, never the prompt.

import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import {
  getProviderModels,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { useUiState } from '../../state/store';
import {
  catalogError,
  describeModel,
  describeModelDetail,
  filterModels,
  formatContextLength,
  writeStoredModel,
} from '../../modelCatalog';
import {
  DEFAULT_MODEL_ID,
  defaultModelLabel,
  isSubscriptionProvider,
  optionalModelNote,
} from '../../providerPolicy';
import type { ProviderModel, ProviderModelCatalog } from '../../types/studio';

export interface ModelPickerProps {
  /** The selected provider id; empty means "no provider selected". */
  providerId: string;
  /** The provider's display name, for the labels. */
  providerName: string;
  /** The current model id (controlled by the composer). */
  value: string;
  onChange(modelId: string): void;
  /** Whether the backend reports this provider as configured. */
  configured: boolean;
  /**
   * The backend's own availability probe for this provider. `null` means
   * the backend did not probe it — never "available".
   */
  providerAvailable?: boolean | null;
  /**
   * Why the backend reported this provider as unusable. Shown verbatim: it
   * is the only true reason the provider has no model list.
   */
  providerReason?: string | null;
  /** The operator's remote-egress consent stamp. */
  allowRemote: boolean;
  /** Tick the consent stamp (the composer's own checkbox). */
  onGrantConsent(): void;
  /** Open the API-key section so the operator can add a key. */
  onOpenKeySettings?: (() => void) | undefined;
  /** Whether the model id may be left empty. */
  optional: boolean;
}

type PickerStatus = 'idle' | 'loading' | 'ready' | 'error' | 'consent';

export function ModelPicker(props: ModelPickerProps): ReactElement {
  const connection = useUiState((s) => s.connection);
  const listId = useId();
  // The catalog is carried together with the provider id it was fetched
  // for. A catalog of another provider is never shown under this one, not
  // even for the render before the reset effect runs. The id is the one
  // the UI ASKED for, because the backend canonicalises the alias `codex`
  // to `codex_cli` in its envelope.
  const [catalog, setCatalog] = useState<{ providerId: string; listing: ProviderModelCatalog } | null>(
    null,
  );
  const [status, setStatus] = useState<PickerStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');
  const [freeOnly, setFreeOnly] = useState(false);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [reloadToken, setReloadToken] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const rootRef = useRef<HTMLElement | null>(null);

  const providerId = props.providerId;

  // A click outside the control closes the list. A click INSIDE it (on a
  // row, on the filter, on the reload button) does not: the list must not
  // disappear between the pointer press and the pointer release that picks
  // a row.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent): void {
      const root = rootRef.current;
      if (!root) return;
      if (event.target instanceof Node && root.contains(event.target)) return;
      setOpen(false);
      setActiveIndex(-1);
    }
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  // The catalog belongs to one provider. A new provider starts from an
  // empty list so the previous provider's models are never shown under the
  // new name.
  useEffect(() => {
    setCatalog(null);
    setStatus('idle');
    setError(null);
    setQuery('');
    setOpen(false);
    setActiveIndex(-1);
  }, [providerId]);

  useEffect(() => {
    if (connection.kind !== 'ready') return;
    if (providerId.length === 0) return;
    // A provider without a credential has no catalog to ask for; the
    // backend would refuse the request anyway.
    if (!props.configured) {
      setStatus('idle');
      setCatalog(null);
      return;
    }
    const controller = new AbortController();
    setStatus('loading');
    setError(null);
    getProviderModels(connection.connection, providerId, props.allowRemote, controller.signal)
      .then((result) => {
        if (controller.signal.aborted) return;
        const failure = catalogError(result);
        if (failure) {
          setCatalog({ providerId, listing: result });
          setStatus('error');
          setError(failure);
          return;
        }
        setCatalog({ providerId, listing: result });
        setStatus('ready');
      })
      .catch((cause: unknown) => {
        if (controller.signal.aborted || cause instanceof StudioAbortError) return;
        if (isConsentRefusal(cause)) {
          setStatus('consent');
          setError(describeError(cause));
          return;
        }
        setStatus('error');
        setError(describeError(cause));
      });
    return () => controller.abort();
  }, [
    connection.kind === 'ready' ? connection.connection : null,
    providerId,
    props.configured,
    props.allowRemote,
    reloadToken,
  ]);

  const models: ReadonlyArray<ProviderModel> =
    catalog && catalog.providerId === providerId ? catalog.listing.models : [];
  const visible = useMemo(
    () => filterModels(models, query, freeOnly),
    [models, query, freeOnly],
  );

  useEffect(() => {
    setActiveIndex(visible.length > 0 ? 0 : -1);
  }, [visible]);

  function choose(model: ProviderModel): void {
    props.onChange(model.id);
    writeStoredModel(providerId, model.id);
    // The filter is cleared so re-opening the list shows the whole catalog
    // again; the input keeps the chosen id.
    setQuery('');
    setOpen(false);
    setActiveIndex(-1);
    inputRef.current?.focus();
  }

  function onInputChange(next: string): void {
    // The raw text is what the operator typed: trimming it here would make
    // a space impossible to type. The chat request trims it when it is
    // sent, and the remembered value is the trimmed id.
    setQuery(next);
    props.onChange(next);
    writeStoredModel(providerId, next.trim());
    setOpen(true);
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (visible.length === 0) return;
      event.preventDefault();
      setOpen(true);
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      setActiveIndex((current) => {
        const start = current < 0 ? (delta > 0 ? -1 : visible.length) : current;
        const next = start + delta;
        if (next < 0) return visible.length - 1;
        if (next >= visible.length) return 0;
        return next;
      });
      return;
    }
    if (event.key === 'Enter') {
      if (open && activeIndex >= 0 && visible[activeIndex]) {
        event.preventDefault();
        choose(visible[activeIndex]);
      }
      return;
    }
    if (event.key === 'Escape') {
      if (open) {
        event.preventDefault();
        setOpen(false);
        setActiveIndex(-1);
      }
    }
  }

  const label = providerId.length > 0 ? props.providerName : 'No provider';
  const optionalNote = props.optional ? ' (optional)' : '';
  // What an EMPTY model field means for this provider, and how to pin one.
  const defaultModelNote = optionalModelNote(providerId, label);
  const activeOptionId =
    open && activeIndex >= 0 && visible[activeIndex]
      ? `${listId}-option-${activeIndex}`
      : undefined;

  return (
    <section className="model-picker" aria-label="Model" ref={rootRef}>
      <div className="model-picker-head">
        <label className="model-picker-label" htmlFor="chat-model-input">
          Model{optionalNote}
        </label>
        {status === 'ready' ? (
          <span
            className="model-picker-count"
            role="status"
            title={
              catalog && catalog.providerId === providerId
                ? `Source: ${catalog.listing.source || 'unspecified'}${
                    catalog.listing.fetched_at ? ` · fetched ${String(catalog.listing.fetched_at)}` : ''
                  }`
                : undefined
            }
          >
            {visible.length === models.length
              ? `${models.length} model${models.length === 1 ? '' : 's'}`
              : `${visible.length} of ${models.length} models`}
          </span>
        ) : null}
      </div>
      <div className="model-picker-control">
        <input
          id="chat-model-input"
          ref={inputRef}
          type="text"
          className="model-picker-input"
          role="combobox"
          autoComplete="off"
          spellCheck={false}
          aria-expanded={open && visible.length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={activeOptionId}
          placeholder={providerId.length > 0 ? 'Search models…' : 'Select a provider first'}
          value={props.value}
          disabled={providerId.length === 0}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={onKeyDown}
          onFocus={() => setOpen(true)}
        />
        <label className="model-picker-free">
          <input
            type="checkbox"
            checked={freeOnly}
            onChange={(event) => setFreeOnly(event.target.checked)}
          />
          <span>Free only</span>
        </label>
        <button
          type="button"
          className="model-picker-reload"
          onClick={() => setReloadToken((token) => token + 1)}
          disabled={providerId.length === 0 || !props.configured || status === 'loading'}
        >
          Reload
        </button>
      </div>
      {defaultModelNote ? (
        <p className="model-picker-note model-picker-note-default" role="note">
          {defaultModelNote}
        </p>
      ) : null}
      {open && visible.length > 0 ? (
        <ul className="model-picker-list" id={listId} role="listbox" aria-label="Models">
          {visible.map((model, index) => (
            <li
              key={model.id}
              id={`${listId}-option-${index}`}
              role="option"
              className="model-picker-option"
              aria-selected={model.id === props.value}
              data-model-id={model.id}
              data-free={model.free ? 'yes' : 'no'}
              data-active={index === activeIndex ? 'yes' : 'no'}
              title={describeModelDetail(model)}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => choose(model)}
            >
              <span className="model-picker-option-head">
                <span className="model-picker-option-name">{modelName(model, providerId)}</span>
                <span className="model-picker-option-id">{model.id}</span>
                {model.free ? (
                  <span className="model-picker-badge" data-state="free">
                    Free
                  </span>
                ) : null}
              </span>
              {describeModel(model) ? (
                <span className="model-picker-option-meta">{describeModel(model)}</span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      <ModelPickerState
        status={status}
        error={error}
        providerId={providerId}
        providerName={label}
        configured={props.configured}
        providerAvailable={props.providerAvailable ?? null}
        providerReason={props.providerReason ?? null}
        modelCount={models.length}
        visibleCount={visible.length}
        query={query}
        freeOnly={freeOnly}
        onGrantConsent={props.onGrantConsent}
        onOpenKeySettings={props.onOpenKeySettings}
        onRetry={() => setReloadToken((token) => token + 1)}
      />
    </section>
  );
}

/**
 * The name of one catalog row.
 *
 * The `default` row of a subscription CLI is NOT a model called "default":
 * it is the CLI's own default model, and the CLI does not report which
 * model that is. The row therefore says so honestly.
 */
function modelName(model: ProviderModel, providerId: string): string {
  if (model.id === DEFAULT_MODEL_ID) {
    const honest = defaultModelLabel(providerId, model.id);
    if (honest) return honest;
  }
  return model.name;
}

/**
 * The state line under the control: one honest sentence per state, with
 * the action that moves the operator forward.
 */
function ModelPickerState(props: {
  status: PickerStatus;
  error: string | null;
  providerId: string;
  providerName: string;
  configured: boolean;
  providerAvailable: boolean | null;
  providerReason: string | null;
  modelCount: number;
  visibleCount: number;
  query: string;
  freeOnly: boolean;
  onGrantConsent(): void;
  onOpenKeySettings?: (() => void) | undefined;
  onRetry(): void;
}): ReactElement | null {
  if (props.providerId.length === 0) {
    return (
      <p className="model-picker-note" role="note">
        Choose a provider to list its models.
      </p>
    );
  }
  if (!props.configured) {
    // A subscription CLI has no API key: the real reason it has no model
    // list is the backend's own — the CLI was not found on this host or is
    // unavailable. Sending the operator to the API-key form would be wrong.
    if (isSubscriptionProvider(props.providerId)) {
      const reason = props.providerReason?.trim() ?? '';
      return (
        <p className="model-picker-note model-picker-note-unavailable" role="note" data-provider-reason="cli">
          {reason.length > 0
            ? `${props.providerName} has no model list: ${reason}.`
            : `${props.providerName} was not found on this machine or is unavailable, so it has no model list.`}
        </p>
      );
    }
    return (
      <p className="model-picker-note model-picker-note-unavailable" role="note">
        {props.providerName} is not configured server-side, so it has no model list yet.{' '}
        {props.onOpenKeySettings ? (
          <button type="button" className="model-picker-action" onClick={props.onOpenKeySettings}>
            Open API keys
          </button>
        ) : null}
      </p>
    );
  }
  if (props.status === 'loading') {
    return (
      <p className="model-picker-note" role="status">
        Loading the model list for {props.providerName}…
      </p>
    );
  }
  if (props.status === 'consent') {
    return (
      <p className="model-picker-consent" role="alert">
        <span className="model-picker-consent-stamp">Remote egress consent required</span>
        <span className="model-picker-consent-text">
          {props.error ?? 'Listing the models of this provider leaves the machine.'} The consent
          stamp below the composer grants it for {props.providerName}.
        </span>
        <button type="button" className="model-picker-action" onClick={props.onGrantConsent}>
          Grant consent and load models
        </button>
      </p>
    );
  }
  if (props.status === 'error') {
    return (
      <p className="model-picker-error" role="alert">
        {props.error ?? 'The model list could not be loaded.'}{' '}
        <button type="button" className="model-picker-action" onClick={props.onRetry}>
          Retry
        </button>
      </p>
    );
  }
  if (props.status === 'ready' && props.modelCount === 0) {
    return (
      <p className="model-picker-note" role="note">
        {props.providerName} reported no models.
      </p>
    );
  }
  if (props.status === 'ready' && props.visibleCount === 0 && props.modelCount > 0) {
    // The provider has models, but the filter hides all of them. The two
    // reasons are named separately: a search that matches nothing, and the
    // "Free only" toggle that hides every paid model.
    return (
      <p className="model-picker-note" role="status">
        {props.freeOnly
          ? `No free model matches “${props.query.trim()}”. Turn off “Free only” to see the other ${props.modelCount} models.`
          : `No model matches “${props.query.trim()}”.`}
      </p>
    );
  }
  return null;
}

/**
 * Whether a refusal is the consent gate rather than an origin or bearer
 * problem. The backend's own code and message decide; the UI does not
 * treat every 403 as a consent request.
 */
function isConsentRefusal(cause: unknown): boolean {
  if (!(cause instanceof StudioAuthError)) return false;
  if (cause.status !== 403) return false;
  const haystack = `${cause.code} ${cause.message}`.toLowerCase();
  return (
    haystack.includes('consent') ||
    haystack.includes('egress') ||
    haystack.includes('allow_remote')
  );
}

function describeError(cause: unknown): string {
  if (cause instanceof StudioAuthError) {
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return `${cause.code}: ${cause.message} (${cause.status})${details}`;
  }
  if (cause instanceof StudioApiError) {
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return `${cause.code}: ${cause.message}${details}`;
  }
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'the model list could not be loaded';
}

/** The context window of a model, for tests and tooltips. */
export { formatContextLength };
