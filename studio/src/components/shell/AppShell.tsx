// Root layout: the Vellum drafting board 4-zone shell.
//
//   | LEFT 340px: agent chat | CENTER flex: drafting sheet + 48px bar | RIGHT 320px |
//   |                        |                                        | AuxView / Journal |
//
// There is no header bar and no app footer. Connection information lives in
// the journal footer and in Options card 04. Exactly one SceneCanvas is
// mounted at any time, inside the center drafting sheet.

import { useEffect, useMemo } from 'react';
import type { ReactElement } from 'react';
import { BootGate } from './BootGate';
import { ChatPanel } from '../chat/ChatPanel';
import { DraftingSheet } from '../drafting/DraftingSheet';
import type { TitleBlockData } from '../drafting/DraftingSheet';
import { AuxView } from '../drafting/AuxView';
import { useDesignController } from '../drafting/useDesignController';
import { JournalPanel } from '../journal/JournalPanel';
import { BottomBar } from '../bottombar/BottomBar';
import { OptionsDrawer } from '../options/OptionsDrawer';
import {
  dispatch,
  persistTheme,
  useUiState,
  selectAuxView,
  selectChainStatus,
  selectConnection,
  selectJournal,
  selectOptionsCard,
  selectOptionsOpen,
  selectPrimitive,
  selectRenderMode,
  selectShowSection,
  selectTheme,
  selectViewport,
  selectViewResetToken,
} from '../../state/store';
import {
  BoxParameters,
  CylinderParameters,
  SphereParameters,
  StudioLocalPreview,
  StudioParameters,
  PrimitiveKind,
} from '../../types/studio';
import { DRAFTING_SCALE } from '../../viewer/draftingGeometry';

export function AppShell(): ReactElement {
  const serverMesh = useUiState((s) => s.preview.server);
  const localMesh = useUiState((s) => s.preview.local);
  const viewport = useUiState(selectViewport);
  const primitive = useUiState(selectPrimitive);
  const parameters = useUiState((s) => s.parameters);
  const connection = useUiState(selectConnection);
  const renderMode = useUiState(selectRenderMode);
  const auxView = useUiState(selectAuxView);
  const showSection = useUiState(selectShowSection);
  const optionsOpen = useUiState(selectOptionsOpen);
  const optionsCard = useUiState(selectOptionsCard);
  const theme = useUiState(selectTheme);
  const journal = useUiState(selectJournal);
  // The hash-chain verdict of the backend's own session journal, and the
  // first event whose digest does not verify. Both come from the store:
  // the ledger may only claim a verified chain when the backend verified it.
  const chainStatus = useUiState(selectChainStatus);
  const firstBadSequence = useUiState((s) => s.sessionJournal.firstBadSequence);
  const viewResetToken = useUiState(selectViewResetToken);
  // One shared drawing/section controller: the sheet and the AuxView read
  // the same per-parameter caches, so two controllers would race the same
  // request. The hook disposes it with the connection.
  const designController = useDesignController();

  // The mesh handed to the viewport must be referentially stable. The shell
  // re-renders for plenty of unrelated reasons (a journal row, a tab, the
  // options drawer, a theme switch); building a new mesh object on every
  // render would rebuild the GPU geometry and snap the camera back to the
  // default framing each time. The memo is keyed on the mesh sources and the
  // driving parameters only.
  const displayMesh = useMemo<StudioLocalPreview | null>(() => {
    if (!serverMesh) return localMesh;
    return {
      kind: primitive,
      parameters,
      positions: new Float32Array(serverMesh.preview.positions),
      normals: new Float32Array(serverMesh.preview.normals),
      indices: new Uint32Array(serverMesh.preview.indices),
      bounds: serverMesh.preview.bounds,
      source: 'server',
      generatedAt: serverMesh.receivedAt,
      engine: serverMesh.preview.engine,
      warnings: serverMesh.preview.warnings,
    };
  }, [serverMesh, localMesh, primitive, parameters]);
  const meshSource: 'local' | 'server' | 'none' = serverMesh ? 'server' : localMesh ? 'local' : 'none';
  const bounds = serverMesh?.preview.bounds ?? localMesh?.bounds ?? null;
  // The drafting sheet follows the workspace theme: paper-toned canvas on
  // vellum, dark slate canvas at night. The store keeps the explicit
  // viewport preference for P2's future manual background control.
  const canvasBackground = theme === 'slate' ? 'dark' : 'paper';

  const titleBlock = useMemo<TitleBlockData>(
    () => ({
      sheet: 'SHEET 01',
      partId: `STUDIO-${primitive.toUpperCase()}`,
      title: `${primitive} · ${describeDrivingDimensions(primitive, parameters)}`,
      // SceneCanvas frames the mesh to fit the sheet; claiming 1:1 would be
      // a false scale statement. The bottom ruler bar reads the same value.
      scale: DRAFTING_SCALE,
      units: 'mm',
      projection: '3RD ANGLE',
      tolerance: '—',
      revision: 'DRAFT',
      author: 'Kala Studio',
    }),
    [primitive, parameters],
  );

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    persistTheme(theme);
  }, [theme]);

  useEffect(() => {
    document.title = connection.kind === 'ready'
      ? `Kala Studio · ${connection.connection.mode === 'desktop' ? 'Desktop' : 'Browser fallback'}`
      : 'Kala Studio · Connecting';
  }, [connection]);

  return (
    <BootGate>
      <div className="studio-shell" data-render-mode={renderMode}>
        <aside className="studio-zone studio-zone-left" aria-label="Agent chat">
          <ChatPanel />
        </aside>
        <main className="studio-zone studio-zone-center" aria-label="Drafting worksheet">
          <DraftingSheet
            mesh={displayMesh}
            meshSource={meshSource}
            showGrid={viewport.showGrid}
            showAxes={viewport.showAxes}
            showDimensions={viewport.showDimensions}
            material={viewport.material}
            background={canvasBackground}
            renderMode={renderMode}
            showSection={showSection}
            titleBlock={titleBlock}
            units="mm"
            viewResetToken={viewResetToken}
            designController={designController}
          />
          <BottomBar onOpenOptions={(card) => dispatch({ type: 'options/open', card })} />
        </main>
        <aside className="studio-zone studio-zone-right" aria-label="Auxiliary view and event journal">
          <AuxView
            auxView={auxView}
            onAuxViewChange={(view) => dispatch({ type: 'auxView/set', view })}
            primitive={primitive}
            parameters={parameters}
            bounds={bounds}
            designController={designController}
          />
          <JournalPanel
            entries={journal}
            connectionStatus={connection}
            hashChainStatus={chainStatus}
            firstBadSequence={firstBadSequence}
          />
        </aside>
        <OptionsDrawer
          open={optionsOpen}
          card={optionsCard}
          onCardChange={(card) => dispatch({ type: 'options/card', card })}
          onClose={() => dispatch({ type: 'options/close' })}
        />
      </div>
    </BootGate>
  );
}

function describeDrivingDimensions(kind: PrimitiveKind, parameters: StudioParameters): string {
  if (kind === 'box') {
    const p = parameters as BoxParameters;
    return `${p.size_x.toFixed(2)} × ${p.size_y.toFixed(2)} × ${p.size_z.toFixed(2)} mm`;
  }
  if (kind === 'cylinder') {
    const p = parameters as CylinderParameters;
    return `⌀${(p.radius * 2).toFixed(2)} × ${p.height.toFixed(2)} mm`;
  }
  const p = parameters as SphereParameters;
  return `⌀${(p.radius * 2).toFixed(2)} mm`;
}
