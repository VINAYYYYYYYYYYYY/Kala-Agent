// Mechanical engineering agent chat panel.
//
// This is the primary workspace, not a side drawer: the operator sizes a
// part by talking to it, and the OpenCascade workbench in the right rail
// answers with the geometry that talk produces.
//
// Honest about three things:
//   1. The text shown is exactly what the backend returned; we do not
//      simulate streaming, do not invent partial answers, and do not claim
//      a model is "thinking" when one isn't.
//   2. Remote providers only run if the operator ticks the explicit consent
//      checkbox. The state machine below refuses to send until consent is on.
//   3. For the local helper, model is optional (the backend ignores it).
//      For API routes, model is required.

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  postChat,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { useUiState, dispatch } from '../../state/store';
import {
  computeGeometryEstimate,
  formatEnvelopeEstimate,
  formatVolumeEstimate,
} from '../../viewer/localPrimitives';
import { ProviderPanel } from '../panels/ProviderPanel';
import type { ChatRequest } from '../../types/studio';
import type {
  BoxParameters,
  CylinderParameters,
  PrimitiveKind,
  SphereParameters,
  StudioParameters,
} from '../../types/studio';
import type { ReactElement } from 'react';

const REMOTE_PROVIDERS: ReadonlySet<string> = new Set(['openai', 'openrouter', 'codex']);

// Starter prompts seed the composer; they never auto-send, so a stray click
// can never spend a remote-provider call without the operator pressing Send.
const STARTER_PROMPTS: ReadonlyArray<{ id: string; label: string; prompt: string }> = [
  {
    id: 'size-part',
    label: 'Size a bracket',
    prompt:
      'Size an L-bracket for a 40 mm mounting plate: legs of 60 mm and 45 mm, 8 mm wall thickness. Give me the bounding envelope and the nominal volume I should expect before I model it in CAD.',
  },
  {
    id: 'size-shaft',
    label: 'Size a shaft',
    prompt:
      'I need a shaft spacer that is 25 mm diameter by 120 mm long. What is the nominal volume, the surface area, and the envelope for a quick mass and packaging estimate?',
  },
  {
    id: 'estimate-housing',
    label: 'Estimate a housing',
    prompt:
      'I am quoting a cylindrical housing, 80 mm outer diameter and 60 mm tall. Give me the nominal OpenCascade volume and surface area so I can sanity-check the material cost before the detailed model exists.',
  },
  {
    id: 'plan-export',
    label: 'Plan a STEP/STL handoff',
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
  const [providerId, setProviderId] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [prompt, setPrompt] = useState<string>('');
  const [allowRemote, setAllowRemote] = useState<boolean>(false);
  const [showKeySettings, setShowKeySettings] = useState<boolean>(false);
  const controllerRef = useRef<AbortController | null>(null);
  const promptRef = useRef<HTMLTextAreaElement | null>(null);

  // The active geometry the OpenCascade workbench is holding right now. The
  // agent quotes these numbers back to the operator, so chat and workbench
  // never drift apart.
  const geometryContext = useMemo(() => {
    const estimate = computeGeometryEstimate(primitive, parameters);
    return {
      primitive,
      dimensions: describeParameters(primitive, parameters),
      envelope: formatEnvelopeEstimate(estimate),
      volume: formatVolumeEstimate(estimate),
    };
  }, [primitive, parameters]);

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

  const provider = server.providers.find((p) => p.id === providerId);
  const configured = provider?.configured === true;
  const isLocal = providerId === 'local' || providerId === 'local-helper';
  const requiresRemoteConsent = REMOTE_PROVIDERS.has(providerId);
  const requiresModel = !isLocal;

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
    try {
      const response = await postChat(connection.connection, payload, controller.signal);
      if (!controller.signal.aborted) {
        dispatch({
          type: 'chat/response',
          response: { text: response.text, provider: response.provider, model: response.model },
        });
      }
    } catch (cause) {
      if (controller.signal.aborted) {
        // Operator cancelled: clear the in-flight state without an error.
        dispatch({ type: 'chat/error', message: 'Cancelled', kind: 'cancelled' });
        return;
      }
      const desc = describeError(cause);
      dispatch({ type: 'chat/error', message: desc.message, kind: desc.kind });
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

  return (
    <section className="chat-panel" aria-label="Mechanical engineering agent chat">
      <header className="chat-panel-header">
        <div className="chat-panel-heading">
          <h2>Mechanical Engineering Agent</h2>
          <p className="chat-panel-subtitle">
            Conversational mechanical sizing, parametric geometry estimation, and STEP/STL
            workflow planning.
          </p>
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
        <section className="chat-panel-context" aria-label="Active OpenCascade geometry context">
          <span className="chat-panel-context-label">Active geometry</span>
          <code className="chat-panel-context-kind">{geometryContext.primitive}</code>
          <span className="chat-panel-context-dims">{geometryContext.dimensions}</span>
          <span className="chat-panel-context-envelope">
            envelope {geometryContext.envelope}
          </span>
          <span className="chat-panel-context-volume">volume {geometryContext.volume}</span>
        </section>
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
      </div>
      <div className="chat-panel-thread" role="log" aria-live="polite">
        {chat.thread.length === 0 ? (
          <p className="chat-panel-empty">
            Pick a configured provider, type a prompt, and send. Remote providers require the consent toggle.
          </p>
        ) : null}
        {chat.thread.map((entry, index) => (
          <article
            key={`${entry.role}-${entry.at}-${index}`}
            className={`chat-panel-message chat-panel-message-${entry.role}`}
          >
            <span className="chat-panel-message-role">{entry.role === 'user' ? 'You' : 'Assistant'}</span>
            <p>{entry.text}</p>
          </article>
        ))}
        {chat.inFlight ? <p className="chat-panel-pending">Awaiting response…</p> : null}
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
        {requiresRemoteConsent ? (
          <label className="chat-panel-consent">
            <input
              type="checkbox"
              checked={allowRemote}
              onChange={(event) => setAllowRemote(event.target.checked)}
            />
            <span>
              I consent to this prompt being sent to a remote provider. Without this tick the request
              will not leave the loopback backend.
            </span>
          </label>
        ) : null}
        <label className="chat-panel-field chat-panel-field-grow">
          <span>Prompt</span>
          <textarea
            className="chat-panel-prompt"
            ref={promptRef}
            value={prompt}
            rows={3}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="Ask a short question about the current primitive, its envelope, or the STEP/STL handoff..."
          />
        </label>
        <div className="chat-panel-actions">
          <button type="submit" className="chat-panel-send" disabled={!canSend}>
            {chat.inFlight ? 'Sending…' : 'Send'}
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

function describeError(cause: unknown): { message: string; kind: 'cancelled' | 'timeout' | 'network' | 'server' } {
  if (cause instanceof StudioAbortError) return { message: 'cancelled', kind: 'cancelled' };
  if (cause instanceof StudioAuthError) return { message: `auth: ${cause.message} (${cause.status})`, kind: 'server' };
  if (cause instanceof StudioApiError) return { message: `${cause.code}: ${cause.message}`, kind: 'server' };
  if (cause instanceof StudioNetworkError) {
    if (cause.cause instanceof DOMException && cause.cause.name === 'AbortError') {
      return { message: 'cancelled', kind: 'cancelled' };
    }
    return { message: cause.message, kind: 'network' };
  }
  if (cause instanceof Error) return { message: cause.message, kind: 'network' };
  return { message: 'unknown error', kind: 'network' };
}
