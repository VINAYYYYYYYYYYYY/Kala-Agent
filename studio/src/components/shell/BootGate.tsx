import type { ReactElement } from 'react';
// Connection bootstrap gate.
//
// Two entry points, one lifecycle-owned handle:
//   1. The bridge path runs from the effect below on mount. It tries
//      `resolveConnectionFromBridge`; on success it dispatches the
//      connection and runs the same registry load against it.
//   2. The manual path runs from the form's `onSubmit`. It validates
//      the inputs, dispatches the connection, and then runs the same
//      registry load against the SAME handle, so it never re-runs the
//      bridge probe and remains cancellable on unmount/retry.
//
// `handleRef` holds exactly one `RegistryHandle` per mounted gate (a
// fresh one per effect run). Every async callback closes over that
// handle and bails once it is no longer alive or has been superseded by
// a newer load on the same handle. React StrictMode's double-mount runs
// the effect twice, so the first handle is cancelled and the second is
// used; there is no shared mutable state to leak.

import { useEffect, useRef, useState } from 'react';
import {
  resolveConnectionFromBridge,
  validateConnection,
} from '../../api/connection';
import {
  getHealth,
  getIntegrations,
  getProviders,
  StudioApiError,
  StudioNetworkError,
} from '../../api/client';
import { dispatch, appendJournal, useUiState } from '../../state/store';
import type { StudioConnection } from '../../types/studio';
import { ConnectionForm } from './ConnectionForm';

type Phase =
  | { kind: 'connecting'; message: string }
  | { kind: 'awaiting-browser' }
  | { kind: 'error'; message: string };

// Each boot session owns exactly one `RegistryHandle`. Both the bridge
// probe (run by the effect on mount / retry) and the manual form
// submission run their registry loads against the handle stored in
// `handleRef`. Because the manual path reuses the same handle the effect
// created, a React unmount or a retry tears the manual fetches down too:
// the effect cleanup flips `alive` and aborts every controller the handle
// owns. No async callback can outlive the component or touch a superseded
// handle. StrictMode's double-mount simply produces a second handle whose
// cleanup cancels the first. All registry fetches are bounded by an
// abort-on-timeout controller.

export interface RegistryHandle {
  alive: boolean;
  controllers: AbortController[];
  // Identity of the load currently allowed to publish results. A new
  // load (or a cancel) replaces it, so a superseded load's callbacks
  // bail instead of racing the winner.
  activeLoad: object | null;
}

export function createRegistryHandle(): RegistryHandle {
  return { alive: true, controllers: [], activeLoad: null };
}

// Abort every in-flight request without retiring the handle, and clear
// the active load so its callbacks are ignored. Used when a manual
// submission supersedes an earlier load on the same handle.
export function abortControllers(handle: RegistryHandle): void {
  handle.activeLoad = null;
  for (const controller of handle.controllers) controller.abort();
  handle.controllers = [];
}

// Retire the handle: mark it dead and abort everything it owns. After
// this, every guarded callback bails before dispatching or setting state.
export function cancelHandle(handle: RegistryHandle): void {
  handle.alive = false;
  abortControllers(handle);
}

export function BootGate({ children }: { children: React.ReactNode }): ReactElement {
  const [phase, setPhase] = useState<Phase>({ kind: 'connecting', message: 'Connecting to Kala Studio backend…' });
  const [retrySeed, setRetrySeed] = useState(0);
  const handleRef = useRef<RegistryHandle | null>(null);
  const connectionRetryToken = useUiState((s) => s.connectionRetryToken);

  useEffect(() => {
    const handle = createRegistryHandle();
    handleRef.current = handle;
    void attemptBridge(handle, setPhase);
    return () => {
      cancelHandle(handle);
      if (handleRef.current === handle) handleRef.current = null;
    };
  }, [retrySeed, connectionRetryToken]);

  const connection = useUiState((s) => s.connection);
  const health = useUiState((s) => s.server.health);
  const integrationsCount = useUiState((s) => s.server.integrations.length);
  const providersCount = useUiState((s) => s.server.providers.length);

  const isReady =
    connection.kind === 'ready' &&
    health !== 'unknown' &&
    integrationsCount > 0 &&
    providersCount > 0;

  if (isReady) {
    return <>{children}</>;
  }

  if (phase.kind === 'awaiting-browser') {
    return (
      <div className="boot-gate" role="status" aria-busy="true">
        <div className="boot-gate-card">
          <p className="boot-gate-kicker">KALA STUDIO // LOOPBACK BOOT GATE</p>
          <h1>Kala Studio</h1>
          <ConnectionForm
            onSubmit={(submitted) => {
              const handle = handleRef.current;
              // The form only renders while the bridge probe is in the
              // awaiting-browser phase, so the handle is normally alive
              // here. If it is not, refuse silently rather than fire an
              // unowned request.
              if (!handle || !handle.alive) return;
              attemptManual(handle, submitted, setPhase);
            }}
          />
        </div>
      </div>
    );
  }

  if (phase.kind === 'error') {
    return (
      <div className="boot-gate" role="alertdialog" aria-busy="false">
        <div className="boot-gate-card">
          <p className="boot-gate-kicker">KALA STUDIO // LOOPBACK BOOT GATE</p>
          <h1>Kala Studio</h1>
          <p className="boot-gate-message">{phase.message}</p>
          <button
            type="button"
            className="boot-gate-retry"
            onClick={() => {
              appendJournal({ kind: 'connection', message: 'Retry requested from boot gate' });
              dispatch({ type: 'server/reset' });
              setPhase({ kind: 'connecting', message: 'Reconnecting…' });
              setRetrySeed((n) => n + 1);
            }}
          >
            Retry
          </button>
          <p className="boot-gate-help">
            If you launched Kala Studio from the desktop app, the token is held in the desktop process
            and is not visible in the browser. Re-launch the desktop app to reset, or fall back to the
            in-page form to paste a token printed elsewhere.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="boot-gate" role="status" aria-busy="true">
      <div className="boot-gate-card">
        <p className="boot-gate-kicker">KALA STUDIO // LOOPBACK BOOT GATE</p>
        <h1>Kala Studio</h1>
        <p className="boot-gate-message">{phase.message}</p>
      </div>
    </div>
  );
}

async function attemptBridge(
  handle: RegistryHandle,
  setPhase: (next: Phase) => void,
): Promise<void> {
  let connection: StudioConnection;
  try {
    connection = await resolveConnectionFromBridge();
  } catch (cause) {
    if (!handle.alive) return;
    if (isMissingBridgeError(cause)) {
      setPhase({ kind: 'awaiting-browser' });
      return;
    }
    const message = describeError(cause);
    setPhase({ kind: 'error', message });
    dispatch({ type: 'connection/error', message });
    appendJournal({ kind: 'connection', message: `Connection failed · ${message}` });
    return;
  }
  if (!handle.alive) return;
  dispatch({ type: 'connection/ready', connection });
  appendJournal({
    kind: 'connection',
    message: `Loopback connected · ${connection.mode === 'desktop' ? 'desktop shell' : 'browser fallback'} · ${connection.baseUrl}`,
  });
  await loadRegistries(handle, connection, setPhase);
}

// Manual form submission: validate, dispatch, then run the same registry
// load — against the SAME lifecycle-owned handle the bridge probe used —
// so the load is cancellable on unmount and on retry. The form has
// already validated the inputs; we re-validate so a malformed connection
// can never be dispatched.
function attemptManual(
  handle: RegistryHandle,
  submitted: StudioConnection,
  setPhase: (next: Phase) => void,
): void {
  // Cancel any request still in flight on this handle before starting a
  // new load, so a double submit cannot leave two requests racing.
  abortControllers(handle);
  let connection: StudioConnection;
  try {
    connection = validateConnection(submitted.baseUrl, submitted.token, 'browser-manual');
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : 'invalid connection';
    setPhase({ kind: 'error', message });
    dispatch({ type: 'connection/error', message });
    return;
  }
  dispatch({ type: 'connection/ready', connection });
  appendJournal({
    kind: 'connection',
    message: `Loopback connected · browser fallback · ${connection.baseUrl}`,
  });
  setPhase({ kind: 'connecting', message: 'Loading registries…' });
  void loadRegistries(handle, connection, setPhase);
}

async function loadRegistries(
  handle: RegistryHandle,
  connection: StudioConnection,
  setPhase: (next: Phase) => void,
): Promise<void> {
  // Claim this handle for the new load. Any earlier load's callbacks now
  // fail the identity check and bail.
  const loadId: object = {};
  handle.activeLoad = loadId;
  const healthController = new AbortController();
  const integrationsController = new AbortController();
  const providersController = new AbortController();
  handle.controllers.push(healthController, integrationsController, providersController);

  const timers: ReturnType<typeof setTimeout>[] = [];
  const timeout = (controller: AbortController, ms: number) => {
    const timer = setTimeout(() => controller.abort(), ms);
    timers.push(timer);
  };
  timeout(healthController, 5_000);
  timeout(integrationsController, 5_000);
  timeout(providersController, 5_000);

  const superseded = () => !handle.alive || handle.activeLoad !== loadId;

  try {
    const [health, integrations, providers] = await Promise.all([
      getHealth(connection, healthController.signal),
      getIntegrations(connection, integrationsController.signal),
      getProviders(connection, providersController.signal),
    ]);
    if (superseded()) return;
    dispatch({
      type: 'server/providers',
      providers: providers.providers,
      health: 'ok',
      healthMessage: health.status,
      version: health.version,
      stepAvailable: health.step_available,
    });
    dispatch({ type: 'server/integrations', integrations: integrations.integrations });
    appendJournal({
      kind: 'system',
      message: `Registries loaded · ${integrations.integrations.length} integrations · ${providers.providers.length} providers`,
    });
  } catch (cause) {
    if (superseded()) return;
    const message = describeError(cause);
    dispatch({
      type: 'server/providers',
      providers: [],
      health: 'unreachable',
      healthMessage: message,
      version: '',
      stepAvailable: false,
    });
    appendJournal({ kind: 'connection', message: `Registry load failed · ${message}` });
    setPhase({ kind: 'error', message });
  } finally {
    for (const timer of timers) clearTimeout(timer);
    handle.controllers = handle.controllers.filter(
      (c) => c !== healthController && c !== integrationsController && c !== providersController,
    );
  }
}

function isMissingBridgeError(cause: unknown): boolean {
  return cause instanceof Error && cause.message.includes('desktop bridge unavailable');
}

function describeError(cause: unknown): string {
  if (cause instanceof StudioApiError) return `${cause.code}: ${cause.message}`;
  if (cause instanceof StudioNetworkError) return cause.message;
  if (cause instanceof Error) return cause.message;
  return 'unknown error';
}