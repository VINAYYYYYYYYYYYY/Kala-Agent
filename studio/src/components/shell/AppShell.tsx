// Root layout: header, sidebar, viewport, right rail.
//
// Chat-first: the Mechanical Engineering Agent (ChatPanel) owns the main
// `.app-shell-viewport` because talking to the agent is the primary way to
// drive a design. The OpenCascade workbench — docked 3D preview, parametric
// Inspector with nominal geometry estimates, and the STEP/STL export panel —
// lives in the right rail and stays live while the agent has the viewport.
//
// The toolbar's view switch flips that arrangement into "3D Canvas Focus",
// where the canvas takes the viewport and the rail drops its docked preview.
// Exactly one `SceneCanvas` is mounted at any moment.

import { useEffect, useState } from 'react';
import type { ReactElement } from 'react';
import { SceneCanvas } from '../../viewer/SceneCanvas';
import { useUiState } from '../../state/store';
import { Header } from '../shell/Header';
import { Sidebar } from '../sidebar/Sidebar';
import { Toolbar } from '../toolbar/Toolbar';
import { BootGate } from '../shell/BootGate';
import { ChatPanel } from '../chat/ChatPanel';
import { RightRail } from './RightRail';
import { DEFAULT_VIEW_MODE } from './workspaceView';
import type { WorkspaceViewMode } from './workspaceView';

export function AppShell(): ReactElement {
  const serverMesh = useUiState((s) => s.preview.server);
  const localMesh = useUiState((s) => s.preview.local);
  const viewport = useUiState((s) => s.viewport);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const connection = useUiState((s) => s.connection);
  const [viewMode, setViewMode] = useState<WorkspaceViewMode>(DEFAULT_VIEW_MODE);

  const displayMesh = serverMesh
    ? {
        kind: primitive,
        parameters,
        positions: new Float32Array(serverMesh.preview.positions),
        normals: new Float32Array(serverMesh.preview.normals),
        indices: new Uint32Array(serverMesh.preview.indices),
        bounds: serverMesh.preview.bounds,
        source: 'server' as const,
        generatedAt: serverMesh.receivedAt,
        engine: serverMesh.preview.engine,
        warnings: serverMesh.preview.warnings,
      }
    : localMesh;
  const meshSource: 'local' | 'server' | 'none' = serverMesh ? 'server' : localMesh ? 'local' : 'none';

  useEffect(() => {
    document.title = connection.kind === 'ready'
      ? `Kala Studio · ${connection.connection.mode === 'desktop' ? 'Desktop' : 'Browser fallback'}`
      : 'Kala Studio · Connecting';
  }, [connection]);

  return (
    <BootGate>
      <div className="app-shell">
        <Header />
        <div className="app-shell-body">
          <Sidebar />
          <main className="app-shell-main" aria-label="Studio workspace">
            <Toolbar viewMode={viewMode} onViewModeChange={setViewMode} />
            <div className="app-shell-viewport" data-view-mode={viewMode}>
              {viewMode === 'agent' ? (
                <ChatPanel />
              ) : (
                <SceneCanvas
                  mesh={displayMesh}
                  meshSource={meshSource}
                  showGrid={viewport.showGrid}
                  showAxes={viewport.showAxes}
                  showDimensions={viewport.showDimensions}
                  material={viewport.material}
                  background={viewport.background}
                />
              )}
            </div>
            <RightRail
              viewMode={viewMode}
              mesh={displayMesh}
              meshSource={meshSource}
              viewport={viewport}
            />
          </main>
        </div>
        <FooterOnReady />
      </div>
    </BootGate>
  );
}

function FooterOnReady(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const server = useUiState((s) => s.server);
  if (connection.kind !== 'ready') return <div className="app-shell-footer" aria-hidden="true" />;
  return (
    <footer className="app-shell-footer">
      <span>
        Connected to <code>{connection.connection.baseUrl}</code>{' '}
        ({connection.connection.mode === 'desktop' ? 'desktop shell' : 'browser fallback'})
      </span>
      <span className="app-shell-footer-secondary">
        Backend v{server.version || 'unknown'} · step {server.stepAvailable ? 'available' : 'unavailable'} · token in memory only
      </span>
    </footer>
  );
}