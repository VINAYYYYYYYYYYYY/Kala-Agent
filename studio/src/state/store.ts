// Reducer store with `useSyncExternalStore` integration.
//
// State is split into connection status, editor (primitive + parameters),
// preview (local + server), chat thread, server registries, and viewport
// preferences. Reducers are pure; the store applies them and notifies
// subscribers synchronously. The React tree reads through `useUiState`,
// which guarantees tear-free snapshots.

import { useSyncExternalStore } from 'react';
import {
  AuxViewKind,
  BoxParameters,
  CadProviderId,
  CadProviderRecord,
  CylinderParameters,
  DesignResult,
  DrawingResponse,
  HashChainStatus,
  IntegrationRecord,
  JOURNAL_CAP,
  JournalEntry,
  JournalKind,
  OptionsCard,
  DetailsTab,
  PreviewResponse,
  PrimitiveKind,
  ProviderCredentialStatus,
  ProviderId,
  ProviderRecord,
  RenderMode,
  SectionPlane,
  SectionResponse,
  SphereParameters,
  StudioConnection,
  StudioLocalPreview,
  StudioParameters,
  StudioSettingsResponse,
  ThemeName,
} from '../types/studio';
import { CAD_PROVIDER_IDS, SECTION_PLANES } from '../types/studio';
import {
  addMeasurePoint,
  clearMeasure,
  createMeasureState,
  toggleMeasure,
} from '../viewer/measure';
import type { MeasurePoint, MeasureState } from '../viewer/measure';
import {
  createDesignState,
  designReducer,
  selectActiveDesignFrom,
  selectDrawingFrom,
  selectSectionFrom,
} from './designs';
import type { DesignAction, DesignResource, DesignState } from './designs';
import {
  createMergedJournalSelector,
  createSessionJournalState,
  sessionJournalReducer,
} from './journal';
import { DEFAULT_DESIGN_SECTION_CUT, defaultSectionCutForPlane, type SectionCut } from './sectionCut';
import type { MergedJournalEntry, SessionJournalAction, SessionJournalState } from './journal';

export type {
  AuxViewKind,
  JournalEntry,
  JournalKind,
  OptionsCard,
  DetailsTab,
  RenderMode,
  ThemeName,
} from '../types/studio';
export type { MeasurePoint, MeasureState } from '../viewer/measure';
export { JOURNAL_CAP } from '../types/studio';
export type { DesignAction, DesignResource, DesignState } from './designs';
export type { MergedJournalEntry, SessionJournalEvent, SessionJournalState } from './journal';
export {
  createDesignMeshMemo,
  designToDisplayMesh,
  drawingCacheKey,
  sectionCacheKey,
  selectActiveDesignFrom,
  selectDrawingFrom,
  selectSectionFrom,
} from './designs';
export { mergeJournalEntries, summarisePayload } from './journal';

// The merged ledger is memoised: `useSyncExternalStore` requires a stable
// snapshot, and a selector that built a new array on every call would
// re-render forever.
const mergedJournalSelector = createMergedJournalSelector();

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
  /** The provider the backend actually answered with (assistant turns). */
  provider?: ProviderId;
  /** The model the backend actually answered with, when it reported one. */
  model?: string | null;
  /** True when the backend answered from the offline local helper. */
  offline?: boolean;
  /** Client-measured send-to-reply time in milliseconds (assistant turns). */
  durationMs?: number;
  /**
   * The design this turn compiled, when it compiled one (ADR-0009). The
   * design itself stays in the design slice; the message only names it, so
   * the agent card always reads the store's copy of the measurement.
   */
  designId?: string;
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
  /**
   * Whether the backend carries the design-turn routes. `null` means the
   * backend did not say (an older build), so the journal poller stays off.
   */
  designAvailable: boolean | null;
  /**
   * The backend's own worst-case design-turn budget, from `/health`.
   *
   * `null` means the backend did not report one (an older build), so the
   * client uses its documented fallback deadline.
   */
  designTurnBudgetSeconds: number | null;
  /**
   * The live CAD provider probe (`GET /providers/cad`). Empty until the
   * design-turn backend answers; the composer never guesses an entry.
   */
  cadProviders: ReadonlyArray<CadProviderRecord>;
  cadProvidersStatus: 'idle' | 'loading' | 'ready' | 'error';
  cadProvidersError: string | null;
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
  // --- Vellum shell state -----------------------------------------------------
  renderMode: RenderMode;
  auxView: AuxViewKind;
  showSection: boolean;
  optionsOpen: boolean;
  optionsCard: OptionsCard;
  // --- App chrome (app-feel wave) ----------------------------------------------
  // The chat is the primary, always-shown surface. The drawing (DraftingSheet)
  // is a secondary split panel: closed by default, opened automatically when a
  // CAD-built design arrives (`design/received`), and toggled from the top bar.
  drawingOpen: boolean;
  // The right-hand Details panel (Properties / Views / Activity) is closed by
  // default so the sheet is the app; the top bar toggles it.
  detailsOpen: boolean;
  detailsTab: DetailsTab;
  // The Export dialog, opened from the top bar.
  exportOpen: boolean;
  theme: ThemeName;
  journal: ReadonlyArray<JournalEntry>;
  // The compiled designs of the session (bounded LRU) and their caches.
  designs: DesignState;
  // The backend's stored session journal, kept apart from the local ledger.
  sessionJournal: SessionJournalState;
  // Incremented by the bottom-bar "Reset View" action. DraftingSheet passes it
  // to the viewport so the camera can be framed again.
  viewResetToken: number;
  // Incremented by the Options drawer "Reconnect / Retry" action. BootGate
  // watches it and re-runs the bridge probe without persisting any token.
  connectionRetryToken: number;
  // The measure tool: armed or not, plus the picked points (model mm).
  measure: MeasureState;
  /**
   * The CAD target the operator picked for this session (ADR-0011). It is
   * deliberately not persisted to localStorage and has NO default: `null`
   * means the first design turn asks the operator to choose. A choice lives
   * as long as the store does.
   */
  cadProvider: CadProviderId | null;
  /**
   * The one Section A–A cutting plane shared by the sheet, the HLR redline,
   * the AuxView section panel, and the clay viewport. The default is the
   * ADR-0009 criterion-4 XZ cut at y = 4 mm; selecting another plane in the
   * AuxView changes this state, and every surface follows it.
   */
  sectionCut: SectionCut;
}

type Listener = () => void;

export type Action =
  | { type: 'connection/connecting' }
  | { type: 'connection/ready'; connection: StudioConnection }
  | { type: 'connection/error'; message: string }
  | { type: 'connection/retry' }
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
  | { type: 'renderMode/set'; mode: RenderMode }
  | { type: 'auxView/set'; view: AuxViewKind }
  | { type: 'section/toggle' }
  | { type: 'section/set'; show: boolean }
  | { type: 'section/cut-plane'; plane: SectionPlane; offsetMm: number | null }
  | { type: 'options/open'; card?: OptionsCard }
  | { type: 'options/close' }
  | { type: 'options/card'; card: OptionsCard }
  | { type: 'chrome/drawing-open' }
  | { type: 'chrome/drawing-close' }
  | { type: 'chrome/drawing-toggle' }
  | { type: 'chrome/details-open'; tab?: DetailsTab }
  | { type: 'chrome/details-close' }
  | { type: 'chrome/details-toggle' }
  | { type: 'chrome/export-open' }
  | { type: 'chrome/export-close' }
  | { type: 'theme/set'; theme: ThemeName }
  | { type: 'journal/append'; entry: JournalEntry }
  | { type: 'journal/clear' }
  | { type: 'view/reset' }
  | { type: 'server/integrations'; integrations: ReadonlyArray<IntegrationRecord> }
  | { type: 'server/providers'; providers: ReadonlyArray<ProviderRecord>; health: 'ok' | 'unreachable'; healthMessage: string; version: string; stepAvailable: boolean }
  | { type: 'server/settings'; settings: StudioSettingsResponse }
  | { type: 'server/capabilities'; designAvailable: boolean | null; designTurnBudgetSeconds?: number | null }
  | { type: 'server/cad-providers-start' }
  | { type: 'server/cad-providers'; providers: ReadonlyArray<CadProviderRecord> }
  | { type: 'server/cad-providers-error'; message: string }
  | { type: 'cad/provider-set'; provider: CadProviderId | null }
  | { type: 'server/reset' }
  | { type: 'chat/send'; request: { provider: ProviderId; model: string | null; prompt: string; allowRemote: boolean } }
  | { type: 'chat/response'; response: { text: string; provider: ProviderId; model: string | null; offline?: boolean; durationMs?: number; designId?: string } }
  | { type: 'chat/error'; message: string; kind: ChatState['lastErrorKind'] }
  | { type: 'chat/clear' }
  | { type: 'measure/toggle' }
  | { type: 'measure/point'; point: MeasurePoint }
  | { type: 'measure/clear' }
  | DesignAction
  | SessionJournalAction;

/**
 * The design and session-journal actions live in their own slices; these
 * guards route them without widening the store's action union.
 */
function isDesignAction(action: Action): action is DesignAction {
  return action.type.startsWith('design/');
}

function isSessionJournalAction(action: Action): action is SessionJournalAction {
  return action.type.startsWith('journal/server-');
}

export const THEME_STORAGE_KEY = 'kala-studio.theme';

/**
 * Read the persisted workspace theme. Only the theme name is stored; no
 * credential, token, connection origin, or chat content ever touches
 * localStorage.
 */
export function readStoredTheme(): ThemeName {
  try {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') return 'vellum';
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    return stored === 'slate' ? 'slate' : 'vellum';
  } catch {
    return 'vellum';
  }
}

export function persistTheme(theme: ThemeName): void {
  try {
    if (typeof window === 'undefined' || typeof localStorage === 'undefined') return;
    localStorage.setItem(THEME_STORAGE_KEY, theme);
  } catch {
    // A locked-down browser profile can refuse storage; the in-memory
    // theme still applies for this session.
  }
}

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
      designAvailable: null,
      designTurnBudgetSeconds: null,
      cadProviders: [],
      cadProvidersStatus: 'idle',
      cadProvidersError: null,
      version: '',
    },
    viewport: {
      showGrid: true,
      showAxes: true,
      showDimensions: true,
      material: 'default',
      background: 'paper',
    },
    lastSyncAt: 0,
    renderMode: 'pencil2d',
    auxView: 'section',
    showSection: true,
    optionsOpen: false,
    optionsCard: 'workspace',
    drawingOpen: false,
    detailsOpen: false,
    detailsTab: 'properties',
    exportOpen: false,
    theme: readStoredTheme(),
    journal: [],
    designs: createDesignState(),
    sessionJournal: createSessionJournalState(),
    viewResetToken: 0,
    connectionRetryToken: 0,
    measure: createMeasureState(),
    cadProvider: null,
    sectionCut: { ...DEFAULT_DESIGN_SECTION_CUT },
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
    case 'connection/retry':
      return { ...state, connectionRetryToken: state.connectionRetryToken + 1 };
    case 'primitive/change':
      return {
        ...state,
        primitive: action.kind,
        parameters: action.parameters,
        preview: { local: null, server: null, inFlight: false, lastError: null, lastErrorKind: 'none' },
        // The picked points belong to the old solid: drop them, keep the
        // tool armed so the operator can re-measure on the new part.
        measure: { ...state.measure, points: [] },
      };
    case 'parameters/change':
      return {
        ...state,
        parameters: action.parameters,
        preview: { ...state.preview, server: null, lastError: null, lastErrorKind: 'none' },
        measure: { ...state.measure, points: [] },
      };
    case 'parameters/reset':
      return {
        ...state,
        parameters: action.parameters,
        preview: { local: null, server: null, inFlight: false, lastError: null, lastErrorKind: 'none' },
        measure: { ...state.measure, points: [] },
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
    case 'renderMode/set':
      return { ...state, renderMode: action.mode };
    case 'auxView/set':
      return { ...state, auxView: action.view };
    case 'section/toggle':
      return { ...state, showSection: !state.showSection };
    case 'section/set':
      return { ...state, showSection: action.show };
    case 'section/cut-plane': {
      // The plane belongs to the closed set and the offset must be finite.
      // A bad dispatch leaves the shared cut untouched rather than putting a
      // plane on the sheet that the backend never cut.
      if (!SECTION_PLANES.includes(action.plane)) return state;
      if (action.offsetMm !== null && !Number.isFinite(action.offsetMm)) return state;
      if (
        state.sectionCut.plane === action.plane &&
        state.sectionCut.offsetMm === action.offsetMm
      ) {
        return state;
      }
      return {
        ...state,
        sectionCut: { plane: action.plane, offsetMm: action.offsetMm },
      };
    }
    case 'options/open':
      return { ...state, optionsOpen: true, optionsCard: action.card ?? state.optionsCard };
    case 'options/close':
      return { ...state, optionsOpen: false };
    case 'options/card':
      return { ...state, optionsCard: action.card, optionsOpen: true };
    case 'chrome/drawing-open':
      return { ...state, drawingOpen: true };
    case 'chrome/drawing-close':
      return { ...state, drawingOpen: false };
    case 'chrome/drawing-toggle':
      return { ...state, drawingOpen: !state.drawingOpen };
    case 'chrome/details-open':
      return { ...state, detailsOpen: true, detailsTab: action.tab ?? state.detailsTab };
    case 'chrome/details-close':
      return { ...state, detailsOpen: false };
    case 'chrome/details-toggle':
      return { ...state, detailsOpen: !state.detailsOpen };
    case 'chrome/export-open':
      return { ...state, exportOpen: true };
    case 'chrome/export-close':
      return { ...state, exportOpen: false };
    case 'theme/set':
      return { ...state, theme: action.theme };
    case 'journal/append': {
      const next = [...state.journal, action.entry];
      // Bounded ledger: oldest rows are dropped first so the array never
      // grows without limit during a long session.
      return {
        ...state,
        journal: next.length > JOURNAL_CAP ? next.slice(next.length - JOURNAL_CAP) : next,
      };
    }
    case 'journal/clear':
      return { ...state, journal: [] };
    case 'view/reset':
      return { ...state, viewResetToken: state.viewResetToken + 1 };
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
    case 'server/capabilities':
      return {
        ...state,
        server: {
          ...state.server,
          designAvailable: action.designAvailable,
          designTurnBudgetSeconds:
            action.designTurnBudgetSeconds !== undefined
              ? action.designTurnBudgetSeconds
              : state.server.designTurnBudgetSeconds,
        },
      };
    case 'server/cad-providers-start':
      return {
        ...state,
        server: {
          ...state.server,
          cadProvidersStatus: 'loading',
          cadProvidersError: null,
        },
      };
    case 'server/cad-providers':
      return {
        ...state,
        server: {
          ...state.server,
          cadProviders: action.providers,
          cadProvidersStatus: 'ready',
          cadProvidersError: null,
        },
      };
    case 'server/cad-providers-error':
      return {
        ...state,
        server: {
          ...state.server,
          cadProvidersStatus: 'error',
          cadProvidersError: action.message,
        },
      };
    case 'cad/provider-set': {
      // The reducer enforces the closed set so a bad dispatch cannot make the
      // composer send a provider the backend does not know.
      if (
        action.provider !== null &&
        !CAD_PROVIDER_IDS.includes(action.provider)
      ) {
        return state;
      }
      if (state.cadProvider === action.provider) return state;
      return { ...state, cadProvider: action.provider };
    }
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
          designAvailable: null,
          designTurnBudgetSeconds: null,
          cadProviders: [],
          cadProvidersStatus: 'idle',
          cadProvidersError: null,
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
    case 'chat/response': {
      const message: ChatMessage = {
        role: 'assistant',
        text: action.response.text,
        at: Date.now(),
        provider: action.response.provider,
        model: action.response.model,
      };
      if (action.response.offline !== undefined) message.offline = action.response.offline;
      if (action.response.durationMs !== undefined) message.durationMs = action.response.durationMs;
      if (action.response.designId !== undefined) message.designId = action.response.designId;
      return {
        ...state,
        chat: {
          thread: [...state.chat.thread, message],
          inFlight: false,
          lastError: null,
          lastErrorKind: 'none',
        },
        lastSyncAt: Date.now(),
      };
    }
    case 'chat/error':
      return {
        ...state,
        chat: { ...state.chat, inFlight: false, lastError: action.message, lastErrorKind: action.kind },
      };
    case 'chat/clear':
      return { ...state, chat: { thread: [], inFlight: false, lastError: null, lastErrorKind: 'none' } };
    case 'measure/toggle':
      return { ...state, measure: toggleMeasure(state.measure) };
    case 'measure/point':
      return { ...state, measure: addMeasurePoint(state.measure, action.point) };
    case 'measure/clear':
      return { ...state, measure: clearMeasure() };
    default: {
      // The design slice and the session-journal slice own their actions.
      if (isDesignAction(action)) {
        const designs = designReducer(state.designs, action);
        // A design built by the CAD tool is what the drawing is for: show it.
        if (action.type === 'design/received' && !state.drawingOpen) {
          return { ...state, designs, drawingOpen: true };
        }
        return designs === state.designs ? state : { ...state, designs };
      }
      if (isSessionJournalAction(action)) {
        const sessionJournal = sessionJournalReducer(state.sessionJournal, action);
        return sessionJournal === state.sessionJournal ? state : { ...state, sessionJournal };
      }
      return state;
    }
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

// --- Journal helpers ---------------------------------------------------------

let journalSequence = 0;

export interface JournalInput {
  kind: JournalKind;
  message: string;
  digest?: string;
  ts?: number;
  id?: string;
}

/**
 * Build a journal entry. The timestamp and monotonic id are created here,
 * outside the reducer, so `reducer` stays a pure function. A caller that
 * already has a real backend digest can pass `digest`.
 */
export function createJournalEntry(input: JournalInput): JournalEntry {
  const ts = input.ts ?? Date.now();
  const entry: JournalEntry = {
    id: input.id ?? `j-${ts.toString(36)}-${(journalSequence++).toString(36)}`,
    ts,
    kind: input.kind,
    message: input.message,
  };
  if (input.digest !== undefined) entry.digest = input.digest;
  return entry;
}

/** Append one real event to the bounded journal. */
export function appendJournal(input: JournalInput): void {
  dispatch({ type: 'journal/append', entry: createJournalEntry(input) });
}

// --- Selectors ---------------------------------------------------------------
//
// P2 can import these instead of repeating inline lambdas. They are plain
// functions so they work with `useUiState(selectX)` and with `getState()`.

export const selectConnection = (state: UiState): ConnectionStatus => state.connection;
export const selectPrimitive = (state: UiState): PrimitiveKind => state.primitive;
export const selectParameters = (state: UiState): StudioParameters => state.parameters;
export const selectPreview = (state: UiState): PreviewState => state.preview;
export const selectViewport = (state: UiState): ViewportPreferences => state.viewport;
export const selectRenderMode = (state: UiState): RenderMode => state.renderMode;
export const selectAuxView = (state: UiState): AuxViewKind => state.auxView;
export const selectShowSection = (state: UiState): boolean => state.showSection;
export const selectOptionsOpen = (state: UiState): boolean => state.optionsOpen;
export const selectOptionsCard = (state: UiState): OptionsCard => state.optionsCard;
export const selectDrawingOpen = (state: UiState): boolean => state.drawingOpen;
export const selectDetailsOpen = (state: UiState): boolean => state.detailsOpen;
export const selectDetailsTab = (state: UiState): DetailsTab => state.detailsTab;
export const selectExportOpen = (state: UiState): boolean => state.exportOpen;
export const selectTheme = (state: UiState): ThemeName => state.theme;
export const selectJournal = (state: UiState): ReadonlyArray<JournalEntry> => state.journal;
export const selectServer = (state: UiState): ServerState => state.server;
export const selectChat = (state: UiState): ChatState => state.chat;
export const selectViewResetToken = (state: UiState): number => state.viewResetToken;
export const selectConnectionRetryToken = (state: UiState): number => state.connectionRetryToken;
export const selectMeasure = (state: UiState): MeasureState => state.measure;
/** The CAD target the operator picked for this session, or null. */
export const selectCadProvider = (state: UiState): CadProviderId | null => state.cadProvider;
/** The live CAD provider probe rows. */
export const selectCadProviders = (state: UiState): ReadonlyArray<CadProviderRecord> =>
  state.server.cadProviders;
/** The shared Section A–A cutting plane. */
export const selectSectionCut = (state: UiState): SectionCut => state.sectionCut;
/** The record of the picked CAD target, or null when it is not selected. */
export const selectCadProviderRecord = (state: UiState): CadProviderRecord | null => {
  if (state.cadProvider === null) return null;
  return state.server.cadProviders.find((record) => record.id === state.cadProvider) ?? null;
};

// --- Design turns (ADR-0009) --------------------------------------------------

/** Where the viewport's mesh comes from. */
export type DisplayMeshSource = 'design' | 'server' | 'local' | 'none';

export const selectDesigns = (state: UiState): DesignState => state.designs;
export const selectActiveDesign = (state: UiState): DesignResult | null =>
  selectActiveDesignFrom(state.designs);
export const selectSessionJournal = (state: UiState): SessionJournalState => state.sessionJournal;
export const selectChainStatus = (state: UiState): HashChainStatus => state.sessionJournal.chainStatus;

/** The cached drawing for a cache key, or null when it was never requested. */
export const selectDrawing = (state: UiState, key: string): DesignResource<DrawingResponse> | null =>
  selectDrawingFrom(state.designs, key);

/** The cached section for a cache key, or null when it was never requested. */
export const selectSection = (state: UiState, key: string): DesignResource<SectionResponse> | null =>
  selectSectionFrom(state.designs, key);

/**
 * The local ledger and the stored backend events, merged and ordered by
 * time. The backend rows keep their sequence and digests; the local rows
 * stay marked as local. The selector is memoised so the snapshot stays
 * referentially stable for `useSyncExternalStore`.
 */
export const selectMergedJournal = (state: UiState): ReadonlyArray<MergedJournalEntry> =>
  mergedJournalSelector(state.journal, state.sessionJournal.events, state.sessionJournal.chainStatus);

/**
 * Which mesh the viewport shows. A compiled design wins over the analytic
 * preview; the preview only fills the sheet when there is no design.
 */
export function selectDisplayMeshSource(state: UiState): DisplayMeshSource {
  const design = selectActiveDesignFrom(state.designs);
  if (design && design.view.positions.length > 0) return 'design';
  if (state.preview.server) return 'server';
  if (state.preview.local) return 'local';
  return 'none';
}

// --- Design actions -----------------------------------------------------------

/**
 * Record a design that arrived from a chat turn or from `POST /design`.
 * The design becomes the active one: it is what the operator just asked
 * for.
 */
export function designReceived(design: DesignResult): void {
  dispatch({ type: 'design/received', design });
}

/** Make a stored design the active one. An unknown id is refused. */
export function activateDesign(designId: string | null): void {
  dispatch({ type: 'design/activate', designId });
}

/** Stop showing a design; the viewport falls back to the analytic preview. */
export function clearActiveDesign(): void {
  dispatch({ type: 'design/clear-active' });
}

/**
 * Pick the CAD target for this session. `null` clears the choice; there is
 * no default and nothing is persisted outside the in-memory store.
 */
export function setCadProvider(provider: CadProviderId | null): void {
  dispatch({ type: 'cad/provider-set', provider });
}

/**
 * Select the shared Section A–A plane. The offset follows the documented
 * default for that plane (XZ at y = 4 mm, XY/YZ at the backend mid-plane).
 */
export function setSectionPlane(plane: SectionPlane): void {
  dispatch({ type: 'section/cut-plane', ...defaultSectionCutForPlane(plane) });
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
