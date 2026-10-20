// Top-level wiring for the studio UI.
import type { ReactElement } from 'react';
//
// The preview controller is created in an effect, not `useMemo`, so React
// StrictMode's double-mount does not pin a disposed controller in the
// React closure. On unmount the controller is disposed; on remount a
// fresh controller is built. The effect also fires `trigger()` whenever
// primitive/parameters change, so the viewport always renders the right
// shape.

import { useEffect, useRef } from 'react';
import { AppShell } from './components/shell/AppShell';
import { useUiState, dispatch, getState } from './state/store';
import { createPreviewController, type PreviewController } from './state/previewController';
import { createJournalPoller, type JournalPoller } from './state/journalPoller';
import { defaultParameters } from './viewer/localPrimitives';
import type { StudioConnection } from './types/studio';
import type { PrimitiveKind, StudioParameters } from './types/studio';

export function App(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  // `null` means the backend did not report the design-turn routes, so the
  // journal poller stays off instead of polling a route that is not there.
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const lastDesignId = useUiState((s) => {
    const order = s.designs.order;
    return order.length > 0 ? order[order.length - 1]! : null;
  });

  const controllerRef = useRef<PreviewController | null>(null);
  const pollerRef = useRef<JournalPoller | null>(null);

  useEffect(() => {
    if (connection.kind !== 'ready') {
      if (controllerRef.current) {
        controllerRef.current.dispose();
        controllerRef.current = null;
      }
      return;
    }
    const next = createPreviewController(connection.connection);
    controllerRef.current = next;
    next.trigger(primitive, parameters);
    return () => {
      next.dispose();
      if (controllerRef.current === next) controllerRef.current = null;
    };
  }, [connection.kind === 'ready' ? connection.connection : null]);

  // The session-journal poller. It is created in an effect (not a memo) so
  // StrictMode's double mount cannot pin a stopped poller in the closure:
  // the cleanup stops the poller and the remount builds a fresh one.
  // It starts ONLY when the backend explicitly advertises design turns
  // (`design_available === true`). A backend that reports `false` answers
  // the journal routes with 503, so polling it would only fill the ledger
  // with failures; `null` means the flag has not arrived yet.
  useEffect(() => {
    if (connection.kind !== 'ready' || designAvailable !== true) {
      if (pollerRef.current) {
        pollerRef.current.stop();
        pollerRef.current = null;
      }
      return;
    }
    const poller = createJournalPoller({
      connection: connection.connection,
      getAfter: () => getState().sessionJournal.lastSequence,
      isBusy: () => getState().chat.inFlight,
      getLastDesignId: () => {
        const designs = getState().designs;
        const last = designs.order[designs.order.length - 1];
        return last ?? null;
      },
    });
    pollerRef.current = poller;
    poller.start();
    return () => {
      poller.stop();
      if (pollerRef.current === poller) pollerRef.current = null;
    };
  }, [connection.kind === 'ready' ? connection.connection : null, designAvailable]);

  // A compiled design is already journalled on the backend when the store
  // receives it (design.intent, provider.build.*, artifact.stored,
  // view.imported, design.measured, design.checked, chat.turn). Wake the
  // poller immediately instead of letting the five-second idle cadence
  // leave the ledger showing only local rows while HASH-CHAIN OK is lit.
  useEffect(() => {
    if (lastDesignId === null) return;
    pollerRef.current?.wake();
  }, [lastDesignId]);

  useEffect(() => {
    const controller = controllerRef.current;
    if (!controller) return;
    controller.trigger(primitive, parameters);
  }, [primitive, parameters]);

  return <AppShell />;
}

export function changePrimitive(kind: PrimitiveKind): void {
  dispatch({ type: 'primitive/change', kind, parameters: defaultParameters(kind) });
}

export type { StudioConnection, StudioParameters };