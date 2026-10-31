// Mechanical engineering agent chat panel — the bound engineer's calculation
// pad and the app's centred, primary surface (chat-first direction,
// 2026-10-31).
//
// The operator sizes a part by talking to it. The drawing is a secondary
// split panel that appears after a CAD-built design; it never crowds the
// conversation by default.
//
// Honest about three things:
//   1. The text shown is exactly what the backend returned; we do not
//      simulate streaming, do not invent partial answers, and do not claim
//      a model is "thinking" when one isn't. The backend's ChatResponse has
//      no structured parameter/tool-call field, so no inline result tables
//      or digest chips are fabricated here.
//   2. Remote providers only run if the operator ticks the explicit consent
//      stamp. The state machine below refuses to send until consent is on.
//   3. For the local helper, model is optional (the backend ignores it).
//      For API routes, model is required.
//
// App-feel wave (2026-10-31): the column is only the thread and the
// composer. The brand wordmark lives in the top bar, the live connection in
// the status bar, the API keys in Settings › Models & keys, the active
// geometry in Details › Properties, and the setup pickers behind the two
// chips under the prompt.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import {
  postChat,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
  StudioTimeoutError,
} from '../../api/client';
import type { ConnectionStatus } from '../../state/store';
import { appendJournal, dispatch, useUiState } from '../../state/store';
import type { ChatMessage } from '../../state/store';
import { CadTargetSelector, cadProviderDisplayName } from './CadTargetSelector';
import { ComposerPopover } from './ComposerPopover';
import { DesignAgentCard } from './DesignAgentCard';
import { designSupport } from './designCard';
import { ModelPicker } from './ModelPicker';
import { ProviderPicker } from './ProviderPicker';
import { readStoredModel } from '../../modelCatalog';
import {
  defaultModelLabel,
  describeProviderStatus,
  requiresModel,
  requiresRemoteConsent,
} from '../../providerPolicy';
import type { CadProviderRecord, ChatRequest } from '../../types/studio';
import '../../styles/chat.css';
import '../../styles/models.css';

// Starter prompts seed the composer; they never auto-send, so a stray click
// can never spend a remote-provider call without the operator pressing the
// drafting button.
const STARTER_PROMPTS: ReadonlyArray<{ id: string; label: string; prompt: string }> = [
  {
    id: 'size-part',
    label: 'Size L-Bracket',
    prompt:
      'Size an L-bracket for a 40 mm mounting plate: legs of 60 mm and 45 mm, 8 mm wall thickness. Give me the bounding envelope and the nominal volume I should expect before I model it in CAD.',
  },
  {
    id: 'size-shaft',
    label: 'Size Stepped Shaft',
    prompt:
      'Size a stepped shaft spacer: 25 mm diameter by 120 mm long, with a 15 mm diameter shoulder 30 mm long. Give me the nominal volume, surface area, and envelope for a quick mass and packaging estimate.',
  },
  {
    id: 'estimate-housing',
    label: 'Housing Envelope',
    prompt:
      'I am quoting a cylindrical housing, 80 mm outer diameter and 60 mm tall. Give me the nominal OpenCascade volume and surface area so I can sanity-check the material cost before the detailed model exists.',
  },
  {
    id: 'plan-export',
    label: 'STEP Handoff',
    prompt:
      'Plan the handoff for the current primitive: which neutral file (STEP ISO-10303-21 or STL) should I export, what should the downstream CAD/CAE package check first, and what geometry numbers should travel with the file?',
  },
];

export function ChatPanel(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const server = useUiState((s) => s.server);
  const chat = useUiState((s) => s.chat);
  const cadProvider = useUiState((s) => s.cadProvider);
  const [providerId, setProviderId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [prompt, setPrompt] = useState<string>('');
  const [allowRemote, setAllowRemote] = useState<boolean>(false);
  // The live elapsed clock of the turn in flight. `turnStartedAt` is the
  // instant the request left the browser; `now` is the tick that keeps the
  // readout honest while the backend works.
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null);
  const [now, setNow] = useState<number>(() => Date.now());
  const controllerRef = useRef<AbortController | null>(null);
  const promptRef = useRef<HTMLTextAreaElement | null>(null);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const threadPinnedRef = useRef<boolean>(true);

  // The honest note for a backend that cannot compile a design. It appears
  // only once the health probe has answered, so the panel never flashes a
  // claim it cannot make yet.
  const support = useMemo(() => designSupport(server), [server]);
  const connectionState = useMemo(() => describeConnection(connection), [connection]);

  useEffect(() => {
    if (!providerId && server.providers[0]) {
      const first = server.providers[0].id;
      setProviderId(first);
      // The first provider of the session starts on the model the operator
      // last used with it, when there is one.
      setModel(readStoredModel(first) ?? '');
    }
  }, [server.providers, providerId]);

  useEffect(() => () => {
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
  }, []);

  // The live elapsed clock of a turn in flight. It ticks once a second and
  // is torn down with the turn (and with the component), so no timer
  // outlives the request it measures. The tick is a text update, not an
  // animation, so it honours `prefers-reduced-motion` without a special
  // case.
  useEffect(() => {
    if (turnStartedAt === null) return;
    const tick = (): void => setNow(Date.now());
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, [turnStartedAt]);

  // Keep the newest turn in view unless the operator has scrolled up to read
  // an earlier answer. A cleared thread re-pins for the next turn. Layout
  // effect so the measurement happens after the new card is laid out.
  useLayoutEffect(() => {
    if (chat.thread.length === 0) threadPinnedRef.current = true;
    if (!threadPinnedRef.current) return;
    const node = threadRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [chat.thread.length, chat.inFlight, chat.lastError, chat.lastErrorKind]);

  function onThreadScroll(): void {
    const node = threadRef.current;
    if (!node) return;
    const distance = node.scrollHeight - node.scrollTop - node.clientHeight;
    threadPinnedRef.current = distance <= 24;
  }

  const provider = server.providers.find((p) => p.id === providerId);
  const configured = provider?.configured === true;
  // The provider policy decides what a turn means: whether it leaves the
  // machine (and therefore needs the consent stamp) and whether a model id
  // is mandatory. The subscription CLIs need the stamp like any remote
  // route, but their model id is optional.
  const needsConsent = requiresRemoteConsent(providerId);
  const modelRequired = requiresModel(providerId);
  const providerLabel = provider?.name ?? (providerId.length > 0 ? providerId : 'No provider');
  // The route stamp states the backend's own verdict: a provider it probed
  // and found unavailable is not "configured", it is unavailable.
  const providerState = provider ? describeProviderStatus(provider) : null;
  // The chip is the whole route in two words: the provider and the model it
  // will run. An empty model is the provider's own default, never a guess.
  const modelLabel = model.trim().length > 0 ? model.trim() : 'default';
  const modelChipLabel = `Model: ${providerLabel} · ${modelLabel}`;

  const cadRecord = cadProvider === null
    ? null
    : server.cadProviders.find((record) => record.id === cadProvider) ?? null;
  const cadLabel = cadProvider === null
    ? 'choose'
    : cadRecord
      ? cadProviderDisplayName(cadRecord)
      : cadProvider;
  // The visible chip carries only the short target name; the parenthetical
  // status (opt-in) and the provider version stay in the popover and in the
  // chip's title. The accessible name keeps the full identity.
  const cadChipValue = cadProvider === null
    ? 'choose'
    : cadRecord
      ? cadProviderShortName(cadRecord)
      : cadProvider;
  const cadChipTitle = cadProvider === null
    ? 'CAD target: choose — pick the provider that builds this design'
    : cadRecord
      ? `CAD target: ${cadLabel}${cadRecord.version ? ` ${cadRecord.version}` : ''}`
      : `CAD target: ${cadProvider}`;
  const cadChipLabel = `CAD target: ${cadLabel}`;

  const canSend =
    connection.kind === 'ready' &&
    !chat.inFlight &&
    prompt.trim().length > 0 &&
    providerId.length > 0 &&
    configured &&
    (!needsConsent || allowRemote) &&
    (!modelRequired || model.trim().length > 0);

  async function onSend(): Promise<void> {
    if (!canSend || connection.kind !== 'ready') return;
    if (controllerRef.current) controllerRef.current.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    const trimmedPrompt = prompt.trim();
    const trimmedModel = model.trim();
    const payload: ChatRequest = {
      provider: providerId as ChatRequest['provider'],
      model: trimmedModel.length > 0 ? trimmedModel : null,
      prompt: trimmedPrompt,
      allow_remote: allowRemote,
      cad_provider: cadProvider,
    };
    setPrompt('');
    dispatch({
      type: 'chat/send',
      request: {
        provider: payload.provider,
        model: payload.model,
        prompt: payload.prompt,
        allowRemote: payload.allow_remote,
      },
    });
    appendJournal({ kind: 'chat', message: `You · ${truncate(trimmedPrompt, 160)}` });
    appendJournal({
      kind: 'tool',
      message: `POST /api/studio/chat · ${providerId}${trimmedModel ? ` · ${trimmedModel}` : ''}${cadProvider ? ` · CAD ${cadProvider}` : ' · CAD none'}`,
    });
    // The reply time is measured here, in the browser, from the moment the
    // request left to the moment the answer arrived. It is a client-side
    // measurement of a real round trip, not a backend-reported figure.
    const startedAt = Date.now();
    setTurnStartedAt(startedAt);
    try {
      const response = await postChat(connection.connection, payload, controller.signal);
      if (!controller.signal.aborted) {
        dispatch({
          type: 'chat/response',
          response: {
            text: response.text,
            provider: response.provider,
            model: response.model,
            offline: response.offline,
            durationMs: Date.now() - startedAt,
            // A turn that compiled a design says which one. The design
            // itself lives in the store, so the card never shows a stale
            // copy of a measurement.
            ...(response.design ? { designId: response.design.design_id } : {}),
          },
        });
        appendJournal({ kind: 'chat', message: `Agent · ${truncate(response.text, 160)}` });
      }
    } catch (cause) {
      if (controller.signal.aborted) {
        // Operator cancelled: the browser closes the request, which drops the
        // backend's handler future; the backend cancels the turn's token and
        // stops the CAD build. The line below says exactly that, and nothing
        // about a design that is still running.
        dispatch({ type: 'chat/error', message: 'Cancelled', kind: 'cancelled' });
        appendJournal({ kind: 'chat', message: 'Turn cancelled — the backend stopped it' });
        return;
      }
      const desc = describeError(cause);
      dispatch({ type: 'chat/error', message: desc.message, kind: desc.kind });
      appendJournal({ kind: 'chat', message: `Chat failed · ${desc.message}` });
    } finally {
      if (controllerRef.current === controller) controllerRef.current = null;
      // Only the turn that owns the clock may stop it.
      setTurnStartedAt((current) => (current === startedAt ? null : current));
    }
  }

  function onCancel(): void {
    if (controllerRef.current) controllerRef.current.abort();
  }

  /**
   * A new provider is a new route: the consent stamp is cleared (it was
   * granted for one provider only) and the model id falls back to the one
   * the operator last used with THAT provider.
   */
  function onProviderChange(next: string): void {
    setProviderId(next);
    setAllowRemote(false);
    setModel(readStoredModel(next) ?? '');
  }

  function onNewChat(): void {
    if (controllerRef.current) controllerRef.current.abort();
    dispatch({ type: 'chat/clear' });
  }

  function onPromptKeyDown(event: KeyboardEvent<HTMLTextAreaElement>): void {
    // Enter drafts; Shift+Enter keeps the newline the operator typed. An
    // active IME composition is never treated as a send.
    if (event.key !== 'Enter' || event.shiftKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    if (canSend) void onSend();
  }

  const supportNotice = support.unavailable && support.note ? (
    <section className="chat-panel-agent-card" aria-label="Design turn support">
      <p className="chat-panel-agent-card-note" role="note">
        {support.note}
      </p>
    </section>
  ) : null;

  // The composer is one block in both states: centred in the column while
  // the thread is empty, docked at the bottom once it has messages.
  const composer = (
    <form
      className="chat-panel-form"
      onSubmit={(event) => {
        event.preventDefault();
        void onSend();
      }}
    >
      <div className="chat-panel-frame chat-panel-form-frame">
        <textarea
          className="chat-panel-prompt"
          aria-label="Prompt"
          ref={promptRef}
          value={prompt}
          rows={3}
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={onPromptKeyDown}
          placeholder="Describe geometry, tolerance, or prompt…"
        />
        <div className="chat-panel-chips">
          <ComposerPopover
            name="model"
            label={modelChipLabel}
            title={modelChipLabel}
            panel={
              <>
                <ProviderPicker
                  providers={server.providers}
                  credentials={server.credentials}
                  value={providerId}
                  onChange={onProviderChange}
                />
                <ModelPicker
                  providerId={providerId}
                  providerName={providerLabel}
                  value={model}
                  onChange={setModel}
                  configured={configured}
                  providerAvailable={provider?.available ?? null}
                  providerReason={provider?.reason ?? null}
                  allowRemote={allowRemote}
                  onGrantConsent={() => setAllowRemote(true)}
                  onOpenKeySettings={() => dispatch({ type: 'options/open', card: 'providers' })}
                  optional={!modelRequired}
                />
              </>
            }
          >
            <span
              className="composer-chip-dot"
              data-state={providerState?.state ?? 'not-configured'}
              aria-hidden="true"
            />
            <span className="composer-chip-value">
              {providerLabel} · {modelLabel}
            </span>
            <span className="composer-chip-caret" aria-hidden="true">▾</span>
          </ComposerPopover>
          <ComposerPopover
            name="cad"
            label={cadChipLabel}
            title={cadChipTitle}
            attention={cadProvider === null}
            panel={<CadTargetSelector />}
          >
            <span className="composer-chip-prefix">CAD:</span>
            <span className="composer-chip-value">{cadChipValue}</span>
            <span className="composer-chip-caret" aria-hidden="true">▾</span>
          </ComposerPopover>
        </div>
        {needsConsent && !allowRemote ? (
          <label className="chat-panel-consent" data-consent="required">
            <input
              type="checkbox"
              checked={allowRemote}
              onChange={(event) => setAllowRemote(event.target.checked)}
            />
            <span className="chat-panel-consent-copy">
              <span className="chat-panel-consent-stamp">
                Remote egress consent · {providerLabel}
              </span>
              <span className="chat-panel-consent-text">
                This prompt leaves the loopback backend for the selected remote provider only while
                this consent stamp is ticked.
              </span>
            </span>
          </label>
        ) : null}
        <div className="chat-panel-actions">
          <button type="submit" className="chat-panel-send" disabled={!canSend}>
            {chat.inFlight ? 'Drafting…' : '[Draft Part Geometry ↵]'}
          </button>
          {chat.inFlight ? (
            <button type="button" className="chat-panel-cancel" onClick={onCancel}>
              Cancel
            </button>
          ) : null}
          {providerState && providerState.state !== 'configured' && providerId ? (
            <span className="chat-panel-warning">
              {providerState.state === 'unavailable'
                ? `Provider unavailable${providerState.reason ? `: ${providerState.reason}` : '.'}`
                : 'Provider is not configured server-side.'}
            </span>
          ) : null}
        </div>
      </div>
    </form>
  );

  const starters = (
    <div className="chat-panel-starters" role="group" aria-label="Engineering starter prompts">
      {STARTER_PROMPTS.map((starter) => (
        <button
          key={starter.id}
          type="button"
          className="chat-panel-starter"
          data-starter={starter.id}
          onClick={() => {
            setPrompt(starter.prompt);
            promptRef.current?.focus();
          }}
        >
          {starter.label}
        </button>
      ))}
    </div>
  );

  return (
    <section
      className="chat-panel"
      data-connection={connectionState}
      data-empty={chat.thread.length === 0 ? 'yes' : 'no'}
      aria-label="Mechanical engineering agent chat"
    >
      <header className="chat-panel-header">
        <div className="chat-panel-frame chat-panel-header-frame">
          <span className="chat-panel-header-label">Agent</span>
          {chat.thread.length > 0 ? (
            <button type="button" className="chat-panel-new" onClick={onNewChat}>
              New chat
            </button>
          ) : null}
        </div>
      </header>
      {chat.thread.length === 0 ? (
        // A new chat: one short mono-caps label, the composer around the
        // middle of the column, and the starter chips under it.
        <div className="chat-panel-center">
          <div className="chat-panel-frame chat-panel-empty">
            <p className="chat-panel-empty-label">New design</p>
          </div>
          {supportNotice ? <div className="chat-panel-frame">{supportNotice}</div> : null}
          {composer}
          <div className="chat-panel-frame">{starters}</div>
        </div>
      ) : (
        <>
          <div
            className="chat-panel-thread"
            ref={threadRef}
            onScroll={onThreadScroll}
            role="log"
            aria-live="polite"
          >
            <div className="chat-panel-frame chat-panel-thread-frame">
              {supportNotice}
              {chat.thread.map((entry, index) => (
                <article
                  key={`${entry.role}-${entry.at}-${index}`}
                  className={`chat-panel-message chat-panel-message-${entry.role}`}
                >
                  <header className="chat-panel-message-meta">
                    <span className="chat-panel-message-role">
                      {entry.role === 'user' ? 'You' : 'Agent'}
                    </span>
                    {entry.role === 'assistant' ? (
                      <span
                        className="chat-panel-message-badge"
                        data-offline={entry.offline === true ? 'yes' : 'no'}
                        title="The provider, model, and reply time the backend reported for this turn"
                      >
                        {chatBadgeText(entry)}
                      </span>
                    ) : null}
                    {entry.role === 'assistant' && typeof entry.durationMs === 'number' ? (
                      <span className="chat-panel-message-elapsed">
                        ({formatElapsed(entry.durationMs)})
                      </span>
                    ) : null}
                    <time className="chat-panel-message-time" dateTime={isoTime(entry.at)}>
                      {formatTime(entry.at)}
                    </time>
                  </header>
                  <p className="chat-panel-message-text">{entry.text}</p>
                  {entry.role === 'assistant' && entry.designId ? (
                    <DesignAgentCard designId={entry.designId} />
                  ) : null}
                </article>
              ))}
              {chat.inFlight ? (
                <p className="chat-panel-pending" role="status">
                  <span className="chat-panel-pending-dot" aria-hidden="true" />
                  Awaiting response…
                  {turnStartedAt !== null ? (
                    // The clock is hidden from assistive technology: the status
                    // region must announce "Awaiting response…" once, not once a
                    // second. The text itself is the operator's elapsed readout.
                    <span
                      className="chat-panel-elapsed-live"
                      data-elapsed-live="yes"
                      aria-hidden="true"
                    >
                      {formatElapsed(Math.max(0, now - turnStartedAt))}
                    </span>
                  ) : null}
                </p>
              ) : null}
              {chat.lastError && chat.lastErrorKind !== 'cancelled' ? (
                <p className="chat-panel-error" role="alert">{chat.lastError}</p>
              ) : null}
              {chat.lastErrorKind === 'cancelled' ? (
                <p className="chat-panel-pending" role="status">
                  Cancelled — the backend stopped the turn.
                </p>
              ) : null}
            </div>
          </div>
          {composer}
        </>
      )}
    </section>
  );
}

function describeConnection(status: ConnectionStatus): string {
  if (status.kind === 'ready') {
    return status.connection.mode === 'desktop' ? 'desktop' : 'browser';
  }
  if (status.kind === 'connecting') return 'connecting';
  if (status.kind === 'error') return 'error';
  return 'disconnected';
}

function formatTime(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '—';
  return date.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
}

function isoTime(ts: number): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return '';
  return date.toISOString();
}

// The badge states what the backend actually answered with: the provider
// and model from the response, or the offline local helper. No model name
// is invented when the response carried none.
//
// A subscription CLI that ran with its OWN default model reports the literal
// id `default` (or nothing at all). The CLI does not say which model it
// picked, so the badge says that instead of naming a model.
function chatBadgeText(entry: ChatMessage): string {
  if (entry.offline === true) return 'local · offline';
  const provider = entry.provider ?? 'local';
  const model = entry.model ?? null;
  const cliDefault = defaultModelLabel(provider, model);
  if (cliDefault) return cliDefault;
  return model ? `${provider} · ${model}` : provider;
}

// Client-measured send-to-reply time, e.g. `1.2 s`. A reply that came back
// in well under a tenth of a second is shown in milliseconds so the chip
// never reads like a stuck value; a turn that ran for a minute or more is
// shown as `m:ss`, so the live clock and the post-reply chip always read
// the same way.
export function formatElapsed(durationMs: number): string {
  if (!Number.isFinite(durationMs) || durationMs < 0) return '—';
  if (durationMs < 100) return `${Math.round(durationMs)} ms`;
  if (durationMs < 60_000) return `${(durationMs / 1000).toFixed(1)} s`;
  const totalSeconds = Math.floor(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${String(seconds).padStart(2, '0')}`;
}

function truncate(text: string, limit: number): string {
  const compact = text.replace(/\s+/g, ' ').trim();
  return compact.length > limit ? `${compact.slice(0, limit - 1)}…` : compact;
}

/**
 * The short name of a CAD target for the composer chip. The backend's
 * display name carries the opt-in status in a parenthetical suffix
 * ("OpenCascade (opt-in)"); that identity belongs in the popover and the
 * chip title, not in the 360 px chip row.
 */
export function cadProviderShortName(provider: CadProviderRecord): string {
  const short = provider.name.replace(/\s*\([^)]*\)\s*$/, '').trim();
  return short.length > 0 ? short : provider.name;
}

function describeError(cause: unknown): { message: string; kind: 'cancelled' | 'timeout' | 'network' | 'server' } {
  if (cause instanceof StudioAbortError) return { message: 'cancelled', kind: 'cancelled' };
  // The client's own deadline expired. This is NOT a cancellation: the
  // backend may still be working on the turn, and the operator must be told
  // exactly that.
  if (cause instanceof StudioTimeoutError) {
    return {
      message:
        `timeout: ${cause.path} did not answer within the client deadline — ` +
        'the backend may still be working on this turn. Press Cancel to stop waiting.',
      kind: 'timeout',
    };
  }
  if (cause instanceof StudioAuthError) return { message: `auth: ${cause.message} (${cause.status})`, kind: 'server' };
  if (cause instanceof StudioApiError) {
    // A provider refusal carries its reason and the available provider ids
    // in `details`. Rendering only the generic message would hide the exact
    // facts the operator needs to pick a different target.
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return { message: `${cause.code}: ${cause.message}${details}`, kind: 'server' };
  }
  if (cause instanceof StudioNetworkError) {
    if (cause.cause instanceof DOMException && cause.cause.name === 'AbortError') {
      return { message: 'cancelled', kind: 'cancelled' };
    }
    return { message: cause.message, kind: 'network' };
  }
  if (cause instanceof Error) return { message: cause.message, kind: 'network' };
  return { message: 'unknown error', kind: 'network' };
}
