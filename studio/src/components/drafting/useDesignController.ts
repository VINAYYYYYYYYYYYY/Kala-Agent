// One design-resource controller for the drafting sheet and the aux view.
//
// The controller caches drawing and section responses in the store, so the
// sheet and the auxiliary view must share one instance: two controllers
// would race the same cache key and send the same request twice. The hook
// creates it in an effect (not a memo) so React StrictMode's double mount
// disposes the first controller and builds a fresh one, and a connection
// change disposes the old controller rather than leaving requests running.

import { useEffect, useState } from 'react';
import { createDesignController, type DesignController } from '../../state/designController';
import { selectConnection, useUiState } from '../../state/store';

export function useDesignController(): DesignController | null {
  const connection = useUiState(selectConnection);
  const [controller, setController] = useState<DesignController | null>(null);
  const readyConnection = connection.kind === 'ready' ? connection.connection : null;

  useEffect(() => {
    if (!readyConnection) {
      setController(null);
      return;
    }
    const next = createDesignController(readyConnection);
    setController(next);
    return () => {
      next.dispose();
      setController((current) => (current === next ? null : current));
    };
  }, [readyConnection]);

  return controller;
}
