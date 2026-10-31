// Root layout: the app frame, chat-first.
//
//   | top bar 44 px                                                       |
//   | chat (centred, primary) | drawing split panel (closed by default)    |
//   | Details 360 (optional, right)                                       |
//   | status bar 28 px (read-only)                                        |
//
// The chat is always shown. The drawing (DraftingSheet) is a secondary
// split panel: it is closed at boot, opens automatically when a CAD-built
// design arrives (`design/received`), and toggles from the top bar or
// Ctrl/Cmd+B. While it is closed no DraftingSheet or SceneCanvas is
// mounted; while it is open exactly one SceneCanvas is.

import { useEffect, useMemo } from 'react';
import type { ReactElement } from 'react';
import { BootGate } from './BootGate';
import { TopBar } from './TopBar';
import { DrawingPanel } from './DrawingPanel';
import { DetailsPanel } from './DetailsPanel';
import { ChatPanel } from '../chat/ChatPanel';
import type { TitleBlockData } from '../drafting/DraftingSheet';
import { useDesignController } from '../drafting/useDesignController';
import { BottomBar } from '../bottombar/BottomBar';
import { OptionsDrawer } from '../options/OptionsDrawer';
import { ExportDialog } from '../export/ExportDialog';
import {
  dispatch,
  persistTheme,
  useUiState,
  selectActiveDesign,
  selectAuxView,
  selectConnection,
  selectDetailsOpen,
  selectDrawingOpen,
  selectExportOpen,
  selectOptionsCard,
  selectOptionsOpen,
  selectPrimitive,
  selectRenderMode,
  selectShowSection,
  selectTheme,
  selectViewport,
  selectViewResetToken,
  setSectionPlane,
} from '../../state/store';
import { DEFAULT_DESIGN_SECTION_CUT } from '../../state/sectionCut';
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
  const activeDesignId = useUiState((s) => s.designs.activeDesignId);
  const renderMode = useUiState(selectRenderMode);
  const auxView = useUiState(selectAuxView);
  const showSection = useUiState(selectShowSection);
  const optionsOpen = useUiState(selectOptionsOpen);
  const optionsCard = useUiState(selectOptionsCard);
  const drawingOpen = useUiState(selectDrawingOpen);
  const detailsOpen = useUiState(selectDetailsOpen);
  const activeDesign = useUiState(selectActiveDesign);
  const exportOpen = useUiState(selectExportOpen);
  const theme = useUiState(selectTheme);
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

  // A new design gets the documented default cut (XZ at y = 4 mm). This
  // lived in the always-mounted AuxView before the app-feel wave; keeping it
  // in the always-mounted shell means it still runs while Details is closed,
  // and remounting the Views tab no longer resets a plane the operator chose.
  useEffect(() => {
    if (!activeDesignId) return;
    setSectionPlane(DEFAULT_DESIGN_SECTION_CUT.plane);
  }, [activeDesignId]);

  useEffect(() => {
    document.title = connection.kind === 'ready'
      ? `Kala Studio · ${connection.connection.mode === 'desktop' ? 'Desktop' : 'Browser fallback'}`
      : 'Kala Studio · Connecting';
  }, [connection]);

  // Global app shortcuts. Text entry keeps its own keys: Ctrl/Cmd+B must
  // never steal a keystroke from a prompt or a numeric field.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (event.defaultPrevented) return;
      if (!event.metaKey && !event.ctrlKey) return;
      if (event.altKey) return;
      if (isTextEntry(event.target)) return;
      if (event.key === ',' || event.code === 'Comma') {
        event.preventDefault();
        dispatch({ type: 'options/open' });
        return;
      }
      if (event.key.toLowerCase() === 'b') {
        event.preventDefault();
        dispatch({ type: 'chrome/drawing-toggle' });
      }
    }
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  return (
    <BootGate>
      <div className="studio-shell" data-render-mode={renderMode}>
        <TopBar />
        <div
          className="studio-main"
          data-drawing={drawingOpen ? 'open' : 'closed'}
          data-details={detailsOpen ? 'open' : 'closed'}
        >
          <aside className="studio-chat-column" aria-label="Agent chat">
            <ChatPanel />
          </aside>
          {drawingOpen ? (
            <DrawingPanel
              name={activeDesign?.label ?? primitive.toUpperCase()}
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
          ) : null}
          {detailsOpen ? (
            <DetailsPanel
              auxView={auxView}
              primitive={primitive}
              parameters={parameters}
              bounds={bounds}
              designController={designController}
            />
          ) : null}
        </div>
        <BottomBar />
        <OptionsDrawer
          open={optionsOpen}
          card={optionsCard}
          onCardChange={(card) => dispatch({ type: 'options/card', card })}
          onClose={() => dispatch({ type: 'options/close' })}
        />
        <ExportDialog
          open={exportOpen}
          onClose={() => dispatch({ type: 'chrome/export-close' })}
        />
      </div>
    </BootGate>
  );
}

/** True while the keystroke belongs to a text field rather than the app. */
function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
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
