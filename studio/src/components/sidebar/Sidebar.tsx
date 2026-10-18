// Sidebar with three workspaces: CAD, Analysis, Providers.
//
// Selecting a workspace reveals a panel below the navigation that lists
// the relevant registry items so the operator doesn't have to find them
// inside a separate tab.
//
// One unified engineering workflow throughout: the same OpenCascade
// workbench — 3D preview, parametric sizing, nominal geometry estimates,
// STEP/STL export — serves both a complete standalone build and a quick
// estimate that gets handed off to an external CAD/CAE package. Nothing in
// this UI separates one operator from another.

import type { ReactElement } from 'react';
import { useState } from 'react';
import { useUiState, dispatch } from '../../state/store';
import { IntegrationsPanel } from '../panels/IntegrationsPanel';
import { ProviderPanel } from '../panels/ProviderPanel';

export type Workspace = 'cad' | 'analysis' | 'providers';

export function Sidebar(): ReactElement {
  const [workspace, setWorkspace] = useState<Workspace>('cad');
  const primitive = useUiState((s) => s.primitive);
  const integrationsCount = useUiState((s) => s.server.integrations.filter((i) => i.category === 'cad').length);
  const analysisCount = useUiState((s) => s.server.integrations.filter((i) => i.category === 'analysis').length);
  const providersCount = useUiState((s) => s.server.providers.length);
  const configuredCount = useUiState((s) => s.server.providers.filter((p) => p.configured).length);

  return (
    <nav className="app-sidebar" aria-label="Studio workspaces">
      <ul className="app-sidebar-list">
        <WorkspaceItem id="cad" current={workspace} label="CAD" count={integrationsCount} description="Parametric primitives and preview" onSelect={setWorkspace} />
        <WorkspaceItem id="analysis" current={workspace} label="Analysis" count={analysisCount} description="Handoff to FEA / CAE registries" onSelect={setWorkspace} />
        <WorkspaceItem id="providers" current={workspace} label="Providers" count={providersCount} configured={`${configuredCount}/${providersCount}`} description="API keys & chat providers" onSelect={setWorkspace} />
      </ul>
      <SidebarPanel workspace={workspace} primitive={primitive} />
    </nav>
  );
}

interface WorkspaceItemProps {
  id: Workspace;
  current: Workspace;
  label: string;
  description: string;
  count?: number;
  configured?: string;
  onSelect(next: Workspace): void;
}

function WorkspaceItem(props: WorkspaceItemProps): ReactElement {
  const isCurrent = props.id === props.current;
  return (
    <li>
      <button
        type="button"
        className={`app-sidebar-button${isCurrent ? ' is-current' : ''}`}
        aria-pressed={isCurrent}
        onClick={() => props.onSelect(props.id)}
      >
        <span className="app-sidebar-button-label">{props.label}</span>
        <span className="app-sidebar-button-meta">
          {props.count !== undefined ? `${props.count} integration${props.count === 1 ? '' : 's'}` : null}
          {props.configured ? props.configured : null}
        </span>
        <span className="app-sidebar-button-description">{props.description}</span>
      </button>
    </li>
  );
}

function SidebarPanel(props: { workspace: Workspace; primitive: string }): ReactElement {
  if (props.workspace === 'cad') {
    return (
      <aside className="app-sidebar-context" aria-label="CAD workspace summary">
        <header>
          <h2>CAD primitives</h2>
        </header>
        <p className="app-sidebar-context-line">Current shape: <code>{props.primitive}</code></p>
        <p className="app-sidebar-context-line">
          The OpenCascade workbench in the right rail previews the shape, sizes it parametrically,
          and reports nominal volume, surface area, and bounding envelope. STEP/STL export writes
          a neutral artefact for an external CAD/CAE package; native connectors are not run from
          Kala Studio.
        </p>
        <button
          type="button"
          className="app-sidebar-secondary"
          onClick={() => dispatch({ type: 'parameters/reset', parameters: defaultFor(props.primitive) })}
        >
          Reset primitive
        </button>
        <IntegrationsPanel compact category="cad" />
      </aside>
    );
  }
  if (props.workspace === 'analysis') {
    return (
      <aside className="app-sidebar-context" aria-label="Analysis workspace summary">
        <header>
          <h2>Analysis registry</h2>
        </header>
        <p className="app-sidebar-context-line">
          Native solver integration is not run from Kala Studio; this list shows the analysis
          targets that can consume a neutral artefact.
        </p>
        <IntegrationsPanel compact category="analysis" />
      </aside>
    );
  }
  return (
    <aside className="app-sidebar-context" aria-label="Providers workspace summary">
      <header>
        <h2>Provider &amp; API key settings</h2>
      </header>
      <p className="app-sidebar-context-line">
        Save your OpenAI or OpenRouter API key to the local backend settings store (or set it via
        environment variables). Keys are stored server-side only and are never echoed back to the
        browser.
      </p>
      <ProviderPanel compact />
    </aside>
  );
}

function defaultFor(kind: string) {
  if (kind === 'box') return { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 };
  if (kind === 'cylinder') return { radius: 20, height: 40, radial_segments: 48, height_segments: 1 };
  return { radius: 25, rings: 24, sectors: 32 };
}
