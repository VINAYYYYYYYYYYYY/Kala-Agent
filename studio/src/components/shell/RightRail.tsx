import type { ReactElement } from 'react';
// OpenCascade geometry, estimation, and CAD-handoff workbench.
//
// The rail used to stack Inspector, Export, and Chat vertically, which
// pushed the chat panel below the visible viewport on a 1280x800 window.
// They now live behind tabs and the active panel owns the rail space, so
// every panel is scrollable on its own.
//
// Since the chat-first pivot the chat is no longer a rail tab by default:
// the rail is the *workbench*, and the docked preview here plus the
// Inspector's parametric sizing and nominal geometry estimates are what the
// agent talks about. The `Assistant` tab is still reachable so a secondary
// view of the thread stays one click away.
//
// WebGL discipline: the rail mounts a docked `SceneCanvas` only while the
// main viewport is showing the agent. In 3D-canvas-focus mode the single
// `SceneCanvas` lives in the main viewport and the rail does not mount a
// second one, so the app never holds more than one WebGL context.

import { useState } from 'react';
import { Inspector } from '../inspector/Inspector';
import { ExportPanel } from '../export/ExportPanel';
import { ChatPanel } from '../chat/ChatPanel';
import { SceneCanvas } from '../../viewer/SceneCanvas';
import type { ViewportPreferences } from '../../state/store';
import type { PreviewResponse, StudioLocalPreview } from '../../types/studio';
import type { WorkspaceViewMode } from './workspaceView';

type RailTab = 'workbench' | 'export' | 'assistant';

const TABS: ReadonlyArray<{ id: RailTab; label: string }> = [
  { id: 'workbench', label: 'Workbench' },
  { id: 'export', label: 'Export' },
  { id: 'assistant', label: 'Assistant' },
];

export interface RightRailProps {
  viewMode: WorkspaceViewMode;
  mesh: StudioLocalPreview | PreviewResponse | null;
  meshSource: 'local' | 'server' | 'none';
  viewport: ViewportPreferences;
}

export function RightRail(props: RightRailProps): ReactElement {
  const [tab, setTab] = useState<RailTab>('workbench');
  // Exactly one SceneCanvas exists at a time: docked here while the main
  // viewport shows the agent, or in the viewport itself in focus mode.
  const docked = props.viewMode === 'agent';

  return (
    <aside
      className={`right-rail${docked ? ' has-preview' : ''}`}
      aria-label="OpenCascade geometry and CAD workbench"
    >
      <div className="right-rail-tabs" role="tablist">
        {TABS.map((entry) => (
          <button
            key={entry.id}
            role="tab"
            aria-selected={tab === entry.id}
            data-rail-tab={entry.id}
            className={`right-rail-tab${tab === entry.id ? ' is-active' : ''}`}
            onClick={() => setTab(entry.id)}
            type="button"
          >
            {entry.label}
          </button>
        ))}
      </div>
      {docked ? (
        <div className="right-rail-preview" aria-label="Docked 3D preview">
          <SceneCanvas
            mesh={props.mesh}
            meshSource={props.meshSource}
            showGrid={props.viewport.showGrid}
            showAxes={props.viewport.showAxes}
            showDimensions={props.viewport.showDimensions}
            material={props.viewport.material}
            background={props.viewport.background}
          />
        </div>
      ) : null}
      <div className={`right-rail-panel right-rail-panel-${tab}`} role="tabpanel">
        {tab === 'workbench' ? <Inspector /> : null}
        {tab === 'export' ? <ExportPanel /> : null}
        {tab === 'assistant' ? <ChatPanel /> : null}
      </div>
    </aside>
  );
}