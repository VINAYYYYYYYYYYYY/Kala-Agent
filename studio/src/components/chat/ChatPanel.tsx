// Mechanical engineering agent chat panel — the bound engineer's calculation
// pad in the left zone.
//
// This is the primary workspace, not a side drawer: the operator sizes a
// part by talking to it, and the OpenCascade workbench in the drafting sheet
// answers with the geometry that talk produces.
//
// Honest about three things:
//   1. The text shown is exactly what the backend returned; we do not
//      simulate streaming, do not invent partial answers, and do not claim
//      a model is "thinking" when one isn't. The backend's ChatResponse has
//      no structured parameter/tool-call field, so no inline result tables
//      or digest chips are fabricated here. The mono table at the top is the
//      real active-geometry context the workbench is holding.
//   2. Remote providers only run if the operator ticks the explicit consent
//      stamp. The state machine below refuses to send until consent is on.
//   3. For the local helper, model is optional (the backend ignores it).
//      For API routes, model is required.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import {
  postChat,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import type { ConnectionStatus } from '../../state/store';
import { appendJournal, dispatch, selectActiveDesign, useUiState } from '../../state/store';
import type { ChatMessage } from '../../state/store';
import {
  computeGeometryEstimate,
  formatEnvelopeEstimate,
  formatVolumeEstimate,
} from '../../viewer/localPrimitives';
import { ProviderPanel } from '../panels/ProviderPanel';
import { CadTargetSelector } from './CadTargetSelector';
import { DesignAgentCard } from './DesignAgentCard';
import { designSupport, measuredByLabel } from './designCard';
import type { ChatRequest } from '../../types/studio';
import type {
  BoxParameters,
  CylinderParameters,
  PrimitiveKind,
  SphereParameters,
  StudioParameters,
} from '../../types/studio';
import '../../styles/chat.css';

const REMOTE_PROVIDERS: ReadonlySet<string> = new Set(['openai', 'openrouter', 'codex']);

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
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const activeDesign = useUiState(selectActiveDesign);
  const cadProvider = useUiState((s) => s.cadProvider);
  const [providerId, setProviderId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [prompt, setPrompt] = useState<string>('');
  const [allowRemote, setAllowRemote] = useState<boolean>(false);
  const [showKeySettings, setShowKeySettings] = useState<boolean>(false);
  const controllerRef = useRef<AbortController | null>(null);
  const promptRef = useRef<HTMLTextAreaElement | null>(null);
  const threadRef = useRef<HTMLDivElement | null>(null);
  const threadPinnedRef = useRef<boolean>(true);

  // The active geometry the OpenCascade workbench is holding right now. The
  // agent quotes these numbers back to the operator, so chat and workbench
  // never drift apart.
  //
  // When a compiled design is active the sheet shows that design, so the
  // block quotes the design's PROVIDER-MEASURED values and says which
  // provider reported them. A property the provider did not report is "—",
  // never a closed-form substitute.
  const geometryContext = useMemo(() => {
    if (activeDesign) {
      const measured = activeDesign.measured;
      const bounds = measured.bounds;
      return {
        primitive: 'design',
        dimensions: activeDesign.label,
        envelope: bounds
          ? `${(bounds.max[0] - bounds.min[0]).toFixed(2)} × ${(bounds.max[1] - bounds.min[1]).toFixed(2)} × ${(bounds.max[2] - bounds.min[2]).toFixed(2)} mm`
          : '—',
        volume: measured.volume_mm3 === null
          ? '—'
          : `${measured.volume_mm3.toFixed(2)} mm³ (${(measured.volume_mm3 / 1000).toFixed(3)} cm³)`,
        source: measuredByLabel(activeDesign),
      };
    }
    const estimate = computeGeometryEstimate(primitive, parameters);
    return {
      primitive,
      dimensions: describeParameters(primitive, parameters),
      envelope: formatEnvelopeEstimate(estimate),
      volume: formatVolumeEstimate(estimate),
      source: 'nominal estimate (closed-form)',
    };
  }, [activeDesign, primitive, parameters]);

  const connectionView = useMemo(() => describeConnection(connection), [connection]);

  // The honest note for a backend that cannot compile a design. It appears
  // only once the health probe has answered, so the panel never flashes a
  // claim it cannot make yet.
  const support = useMemo(() => designSupport(server), [server]);

  useEffect(() => {
    if (!providerId && server.providers[0]) {
      setProviderId(server.providers[0].id);
    }
  }, [server.providers, providerId]);

  useEffect(() => () => {
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
  }, []);

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
  const isLocal = providerId === 'local' || providerId === 'local-helper';
  const requiresRemoteConsent = REMOTE_PROVIDERS.has(providerId);
  const requiresModel = !isLocal;
  const providerLabel = provider?.name ?? (providerId.length > 0 ? providerId : 'No provider');
  const providerStatusText = providerId.length === 0
    ? 'No provider selected'
    : `${providerLabel} · ${configured ? 'configured' : 'not configured'}${isLocal ? ' · loopback only' : ''}`;

  const canSend =
    connection.kind === 'ready' &&
    !chat.inFlight &&
    prompt.trim().length > 0 &&
    providerId.length > 0 &&
    configured &&
    (!requiresRemoteConsent || allowRemote) &&
    (!requiresModel || model.trim().length > 0);

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
        // Operator cancelled: clear the in-flight state without an error.
        dispatch({ type: 'chat/error', message: 'Cancelled', kind: 'cancelled' });
        return;
      }
      const desc = describeError(cause);
      dispatch({ type: 'chat/error', message: desc.message, kind: desc.kind });
      appendJournal({ kind: 'chat', message: `Chat failed · ${desc.message}` });
    } finally {
      if (controllerRef.current === controller) controllerRef.current = null;
    }
  }

  function onCancel(): void {
    if (controllerRef.current) controllerRef.current.abort();
  }

  function onClear(): void {
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

  return (
    <section
      className="chat-panel"
      data-connection={connectionView.state}
      aria-label="Mechanical engineering agent chat"
    >
      <header className="chat-panel-header">
        <div className="chat-panel-stamp">
          <h2 className="chat-panel-stamp-title">
            KALA <span className="chat-panel-stamp-slash" aria-hidden="true">//</span> MECHANICAL DESIGN AGENT
          </h2>
          <span className="chat-panel-connection" data-state={connectionView.state} role="status" title={connectionView.title}>
            <span className="chat-panel-connection-dot" aria-hidden="true" />
            {connectionView.label}
          </span>
        </div>
        <div className="chat-panel-header-actions">
          <button
            type="button"
            className="chat-panel-settings-toggle"
            aria-expanded={showKeySettings}
            onClick={() => setShowKeySettings((open) => !open)}
          >
            {showKeySettings ? 'Hide API Keys' : 'API Keys'}
          </button>
          <button
            type="button"
            className="chat-panel-clear"
            onClick={onClear}
            disabled={chat.thread.length === 0}
          >
            Clear
          </button>
        </div>
      </header>
      {showKeySettings ? (
        <section className="chat-panel-settings" aria-label="API key settings">
          <header className="chat-panel-settings-header">
            <h3>Configure Provider API Keys</h3>
            <p>
              Keys are stored on your local Studio backend (<code>studio-settings.json</code>, mode{' '}
              <code>0600</code>) and never echoed back to the browser.
            </p>
          </header>
          <ProviderPanel compact />
        </section>
      ) : null}
      <div className="chat-panel-intro">
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
        <section className="chat-panel-context" aria-label="Active OpenCascade geometry context">
          <div className="chat-panel-context-head">
            <span className="chat-panel-context-label">Active geometry</span>
            <code className="chat-panel-context-kind">{geometryContext.primitive}</code>
          </div>
          <dl className="chat-panel-context-table">
            <div className="chat-panel-context-row">
              <dt>dims</dt>{' '}
              <dd className="chat-panel-context-dims">{geometryContext.dimensions}</dd>
            </div>
            <div className="chat-panel-context-row">
              <dt>envelope</dt>{' '}
              <dd className="chat-panel-context-envelope">{geometryContext.envelope}</dd>
            </div>
            <div className="chat-panel-context-row">
              <dt>volume</dt>{' '}
              <dd className="chat-panel-context-volume">{geometryContext.volume}</dd>
            </div>
            <div className="chat-panel-context-row">
              <dt>source</dt>{' '}
              <dd className="chat-panel-context-source">{geometryContext.source}</dd>
            </div>
          </dl>
        </section>
        {support.unavailable && support.note ? (
          <section className="chat-panel-agent-card" aria-label="Design turn support">
            <p className="chat-panel-agent-card-note" role="note">
              {support.note}
            </p>
          </section>
        ) : null}
      </div>
      <div
        className="chat-panel-thread"
        ref={threadRef}
        onScroll={onThreadScroll}
        role="log"
        aria-live="polite"
      >
        {chat.thread.length === 0 ? (
          <p className="chat-panel-empty">
            Pick a configured provider, type a prompt, and send. Remote providers require the consent
            stamp below.
          </p>
        ) : null}
        {chat.thread.map((entry, index) => (
          <article
            key={`${entry.role}-${entry.at}-${index}`}
            className={`chat-panel-message chat-panel-message-${entry.role}`}
          >
            <header className="chat-panel-message-meta">
              <span className="chat-panel-message-role">{entry.role === 'user' ? 'You' : 'Agent'}</span>
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
                <span className="chat-panel-message-elapsed">({formatElapsed(entry.durationMs)})</span>
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
          </p>
        ) : null}
        {chat.lastError && chat.lastErrorKind !== 'cancelled' ? (
          <p className="chat-panel-error" role="alert">{chat.lastError}</p>
        ) : null}
        {chat.lastErrorKind === 'cancelled' ? (
          <p className="chat-panel-pending" role="status">Request cancelled</p>
        ) : null}
      </div>
      <form
        className="chat-panel-form"
        onSubmit={(event) => {
          event.preventDefault();
          void onSend();
        }}
      >
        <div className="chat-panel-route">
          <label className="chat-panel-field">
            <span>Provider</span>
            <select
              id="chat-provider-select"
              value={providerId}
              onChange={(event) => {
                setProviderId(event.target.value);
                setAllowRemote(false);
              }}
            >
              <option value="" disabled>Select a provider</option>
              {server.providers.map((provider) => (
                <option key={provider.id} value={provider.id} disabled={!provider.configured}>
                  {provider.name}{provider.configured ? '' : ' (not configured)'}
                </option>
              ))}
            </select>
          </label>
          <label className="chat-panel-field">
            <span>
              Model{isLocal ? ' (optional for local helper)' : ''}
            </span>
            <input
              id="chat-model-input"
              type="text"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder={isLocal ? 'optional' : 'e.g. gpt-4o-mini'}
            />
          </label>
        </div>
        <CadTargetSelector />
        {requiresRemoteConsent ? (
          <label
            className="chat-panel-consent"
            data-consent={allowRemote ? 'granted' : 'required'}
          >
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
        ) : (
          <p
            className="chat-panel-provider-status"
            data-configured={configured ? 'yes' : 'no'}
            data-route={isLocal ? 'local' : 'loopback'}
          >
            <span className="chat-panel-provider-status-label">Route</span>
            <span className="chat-panel-provider-status-value" title={providerStatusText}>
              {providerStatusText}
            </span>
          </p>
        )}
        <label className="chat-panel-field chat-panel-field-grow">
          <span>Prompt</span>
          <textarea
            className="chat-panel-prompt"
            ref={promptRef}
            value={prompt}
            rows={3}
            onChange={(event) => setPrompt(event.target.value)}
            onKeyDown={onPromptKeyDown}
            placeholder="Describe geometry, tolerance, or prompt…"
          />
        </label>
        <div className="chat-panel-actions">
          <button type="submit" className="chat-panel-send" disabled={!canSend}>
            {chat.inFlight ? 'Drafting…' : '[Draft Part Geometry ↵]'}
          </button>
          {chat.inFlight ? (
            <button type="button" className="chat-panel-cancel" onClick={onCancel}>
              Cancel
            </button>
          ) : null}
          {!configured && providerId ? (
            <span className="chat-panel-warning">
              Provider is not configured server-side.
            </span>
          ) : null}
        </div>
      </form>
    </section>
  );
}

interface ConnectionView {
  label: string;
  state: 'desktop' | 'browser' | 'connecting' | 'disconnected' | 'error';
  title: string;
}

function describeConnection(status: ConnectionStatus): ConnectionView {
  if (status.kind === 'ready') {
    const desktop = status.connection.mode === 'desktop';
    return {
      label: desktop ? 'Desktop bridge' : 'Browser fallback',
      state: desktop ? 'desktop' : 'browser',
      title: `Connected to ${status.connection.baseUrl}`,
    };
  }
  if (status.kind === 'connecting') {
    return { label: 'Connecting', state: 'connecting', title: 'Loopback connection in progress' };
  }
  if (status.kind === 'error') {
    return { label: 'Disconnected', state: 'error', title: status.message };
  }
  return { label: 'Disconnected', state: 'disconnected', title: 'No loopback connection' };
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
function chatBadgeText(entry: ChatMessage): string {
  if (entry.offline === true) return 'local · offline';
  const provider = entry.provider ?? 'local';
  const model = entry.model ?? null;
  return model ? `${provider} · ${model}` : provider;
}

// Client-measured send-to-reply time, e.g. `1.2 s`. A reply that came back
// in well under a tenth of a second is shown in milliseconds so the chip
// never reads like a stuck value.
function formatElapsed(durationMs: number): string {
  if (!Number.isFinite(durationMs) || durationMs < 0) return '—';
  if (durationMs < 100) return `${Math.round(durationMs)} ms`;
  return `${(durationMs / 1000).toFixed(1)} s`;
}

// One-line, dimension-first summary of the active primitive. Only the
// driving dimensions are listed — segment counts are tessellation detail and
// belong in the inspector, not in a chat banner.
function describeParameters(
  primitive: PrimitiveKind,
  parameters: StudioParameters,
): string {
  if (primitive === 'box') {
    const p = parameters as BoxParameters;
    return `${mm(p.size_x)} × ${mm(p.size_y)} × ${mm(p.size_z)}`;
  }
  if (primitive === 'cylinder') {
    const p = parameters as CylinderParameters;
    return `⌀${mm(p.radius * 2)} × ${mm(p.height)}`;
  }
  const p = parameters as SphereParameters;
  return `⌀${mm(p.radius * 2)}`;
}

function mm(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return `${value.toFixed(2)} mm`;
}

function truncate(text: string, limit: number): string {
  const compact = text.replace(/\s+/g, ' ').trim();
  return compact.length > limit ? `${compact.slice(0, limit - 1)}…` : compact;
}

function describeError(cause: unknown): { message: string; kind: 'cancelled' | 'timeout' | 'network' | 'server' } {
  if (cause instanceof StudioAbortError) return { message: 'cancelled', kind: 'cancelled' };
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
