// Reducer store with `useSyncExternalStore` integration.
//
// State is split into connection status, editor (primitive + parameters),
// preview (local + server), chat thread, server registries, and viewport
// preferences. Reducers are pure; the store applies them and notifies
// subscribers synchronously. The React tree reads through `useUiState`,
// which guarantees tear-free snapshots.

import { useSyncExternalStore } from 'react';
import {
  BoxParameters,
  CylinderParameters,
  IntegrationRecord,
  PreviewResponse,
  PrimitiveKind,
  ProviderCredentialStatus,
  ProviderId,
  ProviderRecord,
  SphereParameters,
  StudioConnection,
  StudioLocalPreview,
  StudioParameters,
  StudioSettingsResponse,
} from '../types/studio';

export type ConnectionStatus =
  | { kind: 'idle' }
  | { kind: 'connecting' }
  | { kind: 'ready'; connection: StudioConnection }
  | { kind: 'error'; message: string };

export interface PreviewState {
  local: StudioLocalPreview | null;
  server: { preview: PreviewResponse; receivedAt: number } | null;
  inFlight: boolean;
  lastError: string | null;
  // Distinguishes whether the most recent failure was an explicit cancel
  // (the operator changed parameters or navigated away) or a real error
  // (timeout, network, server refusal).
  lastErrorKind: 'none' | 'cancelled' | 'timeout' | 'network' | 'server';
}

export interface ChatMessage {
  role: 'user' | 'assistant';
  text: string;
  at: number;
}

export interface ChatState {
  thread: ReadonlyArray<ChatMessage>;
  inFlight: boolean;
  lastError: string | null;
  lastErrorKind: 'none' | 'cancelled' | 'timeout' | 'network' | 'server';
}

export interface ServerState {
  integrations: ReadonlyArray<IntegrationRecord>;
  providers: ReadonlyArray<ProviderRecord>;
  credentials: ReadonlyArray<ProviderCredentialStatus>;
  health: 'unknown' | 'ok' | 'unreachable';
  healthMessage: string;
  stepAvailable: boolean;
  version: string;
}

export type MaterialKind = 'default' | 'matte' | 'wireframe';
export type BackgroundKind = 'studio' | 'paper' | 'dark';

export interface ViewportPreferences {
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
}

export interface UiState {
  connection: ConnectionStatus;
  primitive: PrimitiveKind;
  parameters: StudioParameters;
  preview: PreviewState;
  chat: ChatState;
  server: ServerState;
  viewport: ViewportPreferences;
  lastSyncAt: number;
}

type Listener = () => void;

export type Action =
  | { type: 'connection/connecting' }
  | { type: 'connection/ready'; connection: StudioConnection }
  | { type: 'connection/error'; message: string }
  | { type: 'primitive/change'; kind: PrimitiveKind; parameters: StudioParameters }
  | { type: 'parameters/change'; parameters: StudioParameters }
  | { type: 'parameters/reset'; parameters: StudioParameters }
  | { type: 'preview/local'; preview: StudioLocalPreview }
  | { type: 'preview/request-start' }
  | { type: 'preview/request-success'; preview: PreviewResponse }
  | { type: 'preview/request-error'; message: string; kind: PreviewState['lastErrorKind'] }
  | { type: 'preview/request-cancelled' }
  | { type: 'preview/clear-server' }
  | { type: 'viewport/set'; patch: Partial<ViewportPreferences> }
  | { type: 'server/integrations'; integrations: ReadonlyArray<IntegrationRecord> }
  | { type: 'server/providers'; providers: ReadonlyArray<ProviderRecord>; health: 'ok' | 'unreachable'; healthMessage: string; version: string; stepAvailable: boolean }
  | { type: 'server/settings'; settings: StudioSettingsResponse }
  | { type: 'server/reset' }
  | { type: 'chat/send'; request: { provider: ProviderId; model: string | null; prompt: string; allowRemote: boolean } }
  | { type: 'chat/response'; response: { text: string; provider: ProviderId; model: string | null } }
  | { type: 'chat/error'; message: string; kind: ChatState['lastErrorKind'] }
  | { type: 'chat/clear' };

// Build a brand-new initial state. Every call allocates a fresh object
// graph; nothing is shared between the returned value and any prior
// state. `resetStore` uses this factory rather than re-using a single
// module-level constant so a reset can never alias (and therefore never
// resurrect) a mutated object.
export function createInitialUiState(): UiState {
  return {
    connection: { kind: 'idle' },
    primitive: 'box',
    parameters: { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 } satisfies BoxParameters,
    preview: { local: null, server: null, inFlight: false, lastError: null, lastErrorKind: 'none' },
    chat: { thread: [], inFlight: false, lastError: null, lastErrorKind: 'none' },
    server: {
      integrations: [],
      providers: [],
      credentials: [],
      health: 'unknown',
      healthMessage: '',
      stepAvailable: false,
      version: '',
    },
    viewport: {
      showGrid: true,
      showAxes: true,
      showDimensions: true,
      material: 'default',
      background: 'studio',
    },
    lastSyncAt: 0,
  };
}

// Convenience constant for callers that only need a default snapshot.
// The store itself never re-uses this object across resets.
export const initialUiState: UiState = createInitialUiState();

export function reducer(state: UiState, action: Action): UiState {
  switch (action.type) {
    case 'connection/connecting':
      return { ...state, connection: { kind: 'connecting' } };
    case 'connection/ready':
      return { ...state, connection: { kind: 'ready', connection: action.connection }, lastSyncAt: Date.now() };
    case 'connection/error':
      return { ...state, connection: { kind: 'error', message: action.message } };
    case 'primitive/change':
      return {
        ...state,
        primitive: action.kind,
        parameters: action.parameters,
        preview: { local: null, server: null, inFlight: false, lastError: null, lastErrorKind: 'none' },
      };
    case 'parameters/change':
      return {
        ...state,
        parameters: action.parameters,
        preview: { ...state.preview, server: null, lastError: null, lastErrorKind: 'none' },
      };
    case 'parameters/reset':
      return {
        ...state,
        parameters: action.parameters,
        preview: { local: null, server: null, inFlight: false, lastError: null, lastErrorKind: 'none' },
      };
    case 'preview/local':
      return { ...state, preview: { ...state.preview, local: action.preview } };
    case 'preview/request-start':
      return { ...state, preview: { ...state.preview, inFlight: true, lastError: null, lastErrorKind: 'none' } };
    case 'preview/request-success':
      return {
        ...state,
        preview: {
          ...state.preview,
          inFlight: false,
          lastError: null,
          lastErrorKind: 'none',
          server: { preview: action.preview, receivedAt: Date.now() },
        },
        lastSyncAt: Date.now(),
      };
    case 'preview/request-error':
      return {
        ...state,
        preview: {
          ...state.preview,
          inFlight: false,
          lastError: action.message,
          lastErrorKind: action.kind,
        },
      };
    // Quiet cancellation: clear the loading flag and any prior error
    // without surfacing a user-facing message. The local preview (and
    // any last-good server mesh) is preserved, so the viewport keeps
    // showing a shape.
    case 'preview/request-cancelled':
      return {
        ...state,
        preview: {
          ...state.preview,
          inFlight: false,
          lastError: null,
          lastErrorKind: 'none',
        },
      };
    case 'preview/clear-server':
      return { ...state, preview: { ...state.preview, server: null } };
    case 'viewport/set':
      return { ...state, viewport: { ...state.viewport, ...action.patch } };
    case 'server/integrations':
      return { ...state, server: { ...state.server, integrations: action.integrations } };
    case 'server/providers':
      return {
        ...state,
        server: {
          ...state.server,
          providers: action.providers,
          health: action.health,
          healthMessage: action.healthMessage,
          stepAvailable: action.stepAvailable,
          version: action.version,
        },
        lastSyncAt: Date.now(),
      };
    case 'server/settings':
      return {
        ...state,
        server: {
          ...state.server,
          credentials: action.settings.credentials,
          providers: action.settings.providers,
        },
        lastSyncAt: Date.now(),
      };
    case 'server/reset':
      return {
        ...state,
        server: {
          ...state.server,
          integrations: [],
          providers: [],
          credentials: [],
          health: 'unknown',
          healthMessage: '',
        },
      };
    case 'chat/send':
      return {
        ...state,
        chat: {
          thread: [...state.chat.thread, { role: 'user', text: action.request.prompt, at: Date.now() }],
          inFlight: true,
          lastError: null,
          lastErrorKind: 'none',
        },
      };
    case 'chat/response':
      return {
        ...state,
        chat: {
          thread: [...state.chat.thread, { role: 'assistant', text: action.response.text, at: Date.now() }],
          inFlight: false,
          lastError: null,
          lastErrorKind: 'none',
        },
        lastSyncAt: Date.now(),
      };
    case 'chat/error':
      return {
        ...state,
        chat: { ...state.chat, inFlight: false, lastError: action.message, lastErrorKind: action.kind },
      };
    case 'chat/clear':
      return { ...state, chat: { thread: [], inFlight: false, lastError: null, lastErrorKind: 'none' } };
  }
}

class Store {
  private state: UiState = createInitialUiState();
  private listeners: Set<Listener> = new Set();

  getState(): UiState {
    return this.state;
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  dispatch(action: Action): void {
    const next = reducer(this.state, action);
    if (next !== this.state) {
      this.state = next;
      for (const listener of this.listeners) listener();
    }
  }

  // Store-owned reset. Installs a fresh initial state and notifies every
  // subscriber so React re-reads through `useSyncExternalStore`. The
  // private field is never reached from outside the class.
  reset(): void {
    this.state = createInitialUiState();
    for (const listener of this.listeners) listener();
  }
}

export const store = new Store();

export function resetStore(): void {
  store.reset();
}

export function useUiState<T>(selector: (state: UiState) => T): T {
  return useSyncExternalStore(
    (listener) => store.subscribe(listener),
    () => selector(store.getState()),
    () => selector(store.getState()),
  );
}

export function getState(): UiState {
  return store.getState();
}

export function dispatch(action: Action): void {
  store.dispatch(action);
}

export function isBoxParameters(p: StudioParameters): p is BoxParameters {
  return (
    typeof (p as Partial<BoxParameters>).size_x === 'number' &&
    typeof (p as Partial<BoxParameters>).size_y === 'number' &&
    typeof (p as Partial<BoxParameters>).size_z === 'number'
  );
}

export function isCylinderParameters(p: StudioParameters): p is CylinderParameters {
  return (
    typeof (p as Partial<CylinderParameters>).radius === 'number' &&
    typeof (p as Partial<CylinderParameters>).height === 'number' &&
    typeof (p as Partial<CylinderParameters>).radial_segments === 'number'
  );
}

export function isSphereParameters(p: StudioParameters): p is SphereParameters {
  return (
    typeof (p as Partial<SphereParameters>).radius === 'number' &&
    typeof (p as Partial<SphereParameters>).rings === 'number' &&
    typeof (p as Partial<SphereParameters>).sectors === 'number'
  );
}
