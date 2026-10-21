// The composer's provider picker.
//
// The rows are the backend's own provider registry (`GET /api/studio/providers`
// and the same list in `GET /api/studio/settings`). The UI renders what the
// backend reported and nothing more:
//
//   * a badge per row: configured, not configured, or unavailable (the
//     backend's availability probe said the provider cannot run here),
//   * the reason the backend gave for an unavailable provider,
//   * where the credential comes from (a stored key, an environment
//     variable, or the operator's own CLI subscription).
//
// An unavailable row cannot be selected — the backend said the provider is
// not usable on this host. A row without a credential CAN be selected: the
// operator needs to see why it is unusable and where the key goes, and the
// composer refuses to send until the backend reports it configured.

import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import {
  canonicalProviderId,
  describeProviderStatus,
  isSubscriptionProvider,
  providerFallbackName,
  PROVIDER_ALIASES,
} from '../../providerPolicy';
import type { ProviderState } from '../../providerPolicy';
import type { ProviderCredentialStatus, ProviderRecord } from '../../types/studio';

export interface ProviderPickerProps {
  providers: ReadonlyArray<ProviderRecord>;
  credentials: ReadonlyArray<ProviderCredentialStatus>;
  /** The selected provider id. Empty means "no provider selected". */
  value: string;
  onChange(providerId: string): void;
  disabled?: boolean;
}

interface ProviderView {
  id: string;
  name: string;
  state: ProviderState;
  badge: string;
  reason: string | null;
  subscription: boolean;
  credential: string | null;
  selectable: boolean;
  /** What an empty model field means for this route, when it is optional. */
  modelNote: string | null;
}

export function ProviderPicker(props: ProviderPickerProps): ReactElement {
  const { providers, credentials, value } = props;
  const listboxId = useId();
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const listboxRef = useRef<HTMLUListElement | null>(null);

  // The backend registry carries the historical alias `codex` next to the
  // canonical `codex_cli`. Two rows that name the same provider look like a
  // bug, so the alias row is hidden whenever its canonical route is listed.
  // A stored selection that still names the alias keeps working: it is shown
  // as the canonical route.
  const listedIds = new Set(providers.map((provider) => provider.id));
  const visibleProviders = providers.filter((provider) => {
    const canonical = PROVIDER_ALIASES[provider.id];
    return !(canonical && listedIds.has(canonical));
  });
  const selectedId = (() => {
    const canonical = PROVIDER_ALIASES[value];
    if (canonical && listedIds.has(canonical)) return canonical;
    return value;
  })();

  const views: ReadonlyArray<ProviderView> = visibleProviders.map((provider) =>
    describeProvider(provider, credentials),
  );
  const selectedIndex = views.findIndex((view) => view.id === selectedId);
  const selected = selectedIndex >= 0 ? views[selectedIndex] : null;

  // A click outside the popup closes it. The listener is removed as soon as
  // the popup closes, so nothing stays bound to the document.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent): void {
      const root = rootRef.current;
      if (!root) return;
      if (event.target instanceof Node && root.contains(event.target)) return;
      setOpen(false);
    }
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  // Opening the popup moves the DOM focus into the listbox, which then owns
  // the active option through `aria-activedescendant`.
  useEffect(() => {
    if (!open) return;
    const node = listboxRef.current;
    if (!node) return;
    node.focus();
    // The selected row is the active one, unless the backend says that row
    // cannot be used: the active row is always one the operator may pick.
    const start =
      selectedIndex >= 0 && views[selectedIndex]?.selectable
        ? selectedIndex
        : firstSelectable(views);
    setActiveIndex(start);
  }, [open]);

  function close(returnFocus: boolean): void {
    setOpen(false);
    if (returnFocus) triggerRef.current?.focus();
  }

  function openList(startIndex?: number): void {
    setOpen(true);
    if (startIndex !== undefined) setActiveIndex(startIndex);
  }

  function moveActive(delta: number): void {
    if (views.length === 0) return;
    const start = activeIndex < 0 ? (delta > 0 ? -1 : views.length) : activeIndex;
    let next = start;
    for (let step = 0; step < views.length; step += 1) {
      next += delta;
      if (next < 0) next = views.length - 1;
      if (next >= views.length) next = 0;
      if (views[next]?.selectable) {
        setActiveIndex(next);
        return;
      }
    }
  }

  function choose(index: number): void {
    const view = views[index];
    if (!view || !view.selectable) return;
    props.onChange(view.id);
    close(true);
  }

  function onTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>): void {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const start = selectedIndex >= 0 ? selectedIndex : 0;
      openList(start);
      return;
    }
    if (event.key === 'Enter' || event.key === ' ' || event.key === 'Spacebar') {
      event.preventDefault();
      openList(selectedIndex >= 0 ? selectedIndex : 0);
    }
  }

  function onListKeyDown(event: KeyboardEvent<HTMLUListElement>): void {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        moveActive(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        moveActive(-1);
        break;
      case 'Home':
        event.preventDefault();
        setActiveIndex(firstSelectable(views));
        break;
      case 'End':
        event.preventDefault();
        setActiveIndex(lastSelectable(views));
        break;
      case 'Enter':
      case ' ':
      case 'Spacebar':
        event.preventDefault();
        if (activeIndex >= 0) choose(activeIndex);
        break;
      case 'Escape':
        event.preventDefault();
        close(true);
        break;
      case 'Tab':
        close(false);
        break;
      default:
        break;
    }
  }

  const triggerLabel = selected
    ? selected.name
    : value.length > 0
      ? providerFallbackName(canonicalProviderId(value))
      : 'Select a provider';
  const triggerState: ProviderState = selected?.state ?? 'not-configured';
  const triggerBadge = selected?.badge ?? 'No provider';

  return (
    <div className="provider-picker" ref={rootRef} data-provider-picker>
      <button
        type="button"
        ref={triggerRef}
        className="provider-picker-trigger"
        id="chat-provider-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={listboxId}
        disabled={props.disabled === true}
        onClick={() => (open ? close(false) : openList(selectedIndex >= 0 ? selectedIndex : 0))}
        onKeyDown={onTriggerKeyDown}
      >
        <span className="provider-picker-trigger-name">{triggerLabel}</span>
        <span className="provider-picker-badge" data-state={triggerState}>
          {triggerBadge}
        </span>
      </button>
      {selected && selected.reason ? (
        <p className="provider-picker-reason" role="note">
          {selected.reason}
        </p>
      ) : null}
      {open ? (
        <ul
          className="provider-picker-list"
          id={listboxId}
          role="listbox"
          tabIndex={-1}
          ref={listboxRef}
          aria-label="Provider"
          aria-activedescendant={activeIndex >= 0 ? `${listboxId}-option-${activeIndex}` : undefined}
          onKeyDown={onListKeyDown}
        >
          {views.map((view, index) => (
            <li
              key={view.id}
              id={`${listboxId}-option-${index}`}
              role="option"
              className="provider-picker-option"
              aria-selected={view.id === selectedId}
              aria-disabled={view.selectable ? undefined : 'true'}
              data-provider-id={view.id}
              data-provider-state={view.state}
              data-active={index === activeIndex ? 'yes' : 'no'}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => choose(index)}
            >
              <span className="provider-picker-option-head">
                <span className="provider-picker-option-name">{view.name}</span>
                <span className="provider-picker-option-id">{view.id}</span>
                <span className="provider-picker-badge" data-state={view.state}>
                  {view.badge}
                </span>
                {view.subscription ? (
                  <span className="provider-picker-badge" data-state="subscription">
                    Subscription (CLI)
                  </span>
                ) : null}
              </span>
              {view.credential ? (
                <span className="provider-picker-option-credential">{view.credential}</span>
              ) : null}
              {view.modelNote ? (
                <span className="provider-picker-option-model">{view.modelNote}</span>
              ) : null}
              {view.reason ? (
                <span className="provider-picker-option-reason">{view.reason}</span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/** The first row the operator may pick, or -1 when every row is disabled. */
function firstSelectable(views: ReadonlyArray<ProviderView>): number {
  const index = views.findIndex((view) => view.selectable);
  return index;
}

/** The last row the operator may pick, or -1 when every row is disabled. */
function lastSelectable(views: ReadonlyArray<ProviderView>): number {
  for (let index = views.length - 1; index >= 0; index -= 1) {
    if (views[index]?.selectable) return index;
  }
  return -1;
}

/**
 * What the UI knows about one provider row.
 *
 * The availability verdict is the backend's own: a provider it probed and
 * found unavailable is not selectable, and the reason it gave is shown
 * verbatim.
 */
function describeProvider(
  provider: ProviderRecord,
  credentials: ReadonlyArray<ProviderCredentialStatus>,
): ProviderView {
  const status = describeProviderStatus(provider);
  const credential = credentials.find((entry) => entry.provider === provider.id);
  const subscription = isSubscriptionProvider(provider.id);
  return {
    id: provider.id,
    name: provider.name,
    state: status.state,
    badge: status.badge,
    reason: status.reason,
    subscription,
    credential: describeCredential(provider.id, credential, provider.source),
    selectable: status.selectable,
    modelNote: subscription
      ? 'Model optional — the CLI runs with its own default model unless you pin one in the model picker.'
      : null,
  };
}

/**
 * Where the credential of this provider comes from, in the operator's
 * words. The backend's `source` is authoritative; nothing here is invented.
 *
 * A subscription CLI has no credential row in the settings (it has no key at
 * all): the backend names its source on the provider record instead
 * ("subscription (CLI)"), and that is what the row says.
 */
function describeCredential(
  providerId: string,
  credential: ProviderCredentialStatus | undefined,
  source: string | null | undefined,
): string | null {
  if (source && source.toLowerCase().includes('subscription')) {
    return 'Uses the subscription session already signed in on this machine.';
  }
  if (!credential) return null;
  if (credential.source === 'subscription') {
    return 'Uses the CLI subscription already signed in on this machine.';
  }
  if (credential.source === 'stored') {
    return `Key stored in Studio settings (${credential.env_var}).`;
  }
  if (credential.source === 'environment') {
    return `Key loaded from the environment (${credential.env_var}).`;
  }
  if (providerId === 'openai') return 'Set OPENAI_API_KEY to enable this route.';
  if (providerId === 'openrouter') return 'Set OPENROUTER_API_KEY to enable this route.';
  return null;
}
