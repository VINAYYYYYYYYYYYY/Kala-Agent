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
import { useUiState, dispatch } from './state/store';
import { createPreviewController, type PreviewController } from './state/previewController';
import { defaultParameters } from './viewer/localPrimitives';
import type { StudioConnection } from './types/studio';
import type { PrimitiveKind, StudioParameters } from './types/studio';

export function App(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);

  const controllerRef = useRef<PreviewController | null>(null);

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