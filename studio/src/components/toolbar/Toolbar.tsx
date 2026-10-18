import type { ReactElement } from 'react';
// Floating toolbar above the viewport.
//
// Owns the workspace view switch (agent vs. expanded 3D canvas), the
// primitive kind picker, viewport toggles (grid, axes, dimensions),
// material/background selectors, and a reset action. The current selection
// is shown with `aria-pressed`/`aria-checked` rather than colour alone.
//
// The view switch lives here, inside the `toolbar` grid area, so the main
// grid keeps its two rows (`auto minmax(0, 1fr)`) — the layout invariant
// that stops the shell growing past the window.

import { useUiState, dispatch } from '../../state/store';
import type { MaterialKind, BackgroundKind, ViewportPreferences } from '../../state/store';
import type { PrimitiveKind } from '../../types/studio';
import { VIEW_MODE_OPTIONS } from '../shell/workspaceView';
import type { WorkspaceViewMode } from '../shell/workspaceView';

const KIND_OPTIONS: ReadonlyArray<{ id: PrimitiveKind; label: string }> = [
  { id: 'box', label: 'Box' },
  { id: 'cylinder', label: 'Cylinder' },
  { id: 'sphere', label: 'Sphere' },
];

const MATERIAL_OPTIONS: ReadonlyArray<MaterialKind> = ['default', 'matte', 'wireframe'];
const BACKGROUND_OPTIONS: ReadonlyArray<BackgroundKind> = ['studio', 'paper', 'dark'];

export interface ToolbarProps {
  viewMode: WorkspaceViewMode;
  onViewModeChange(next: WorkspaceViewMode): void;
}

export function Toolbar(props: ToolbarProps): ReactElement {
  const primitive = useUiState((s) => s.primitive);
  const viewport = useUiState((s) => s.viewport);
  const preview = useUiState((s) => s.preview);

  function onPrimitiveChange(next: PrimitiveKind): void {
    if (next === primitive) return;
    dispatch({
      type: 'primitive/change',
      kind: next,
      parameters: next === 'box'
        ? { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 }
        : next === 'cylinder'
          ? { radius: 20, height: 40, radial_segments: 48, height_segments: 1 }
          : { radius: 25, rings: 24, sectors: 32 },
    });
  }

  function patchViewport(patch: Partial<ViewportPreferences>): void {
    dispatch({ type: 'viewport/set', patch });
  }

  return (
    <div className="viewport-toolbar" role="toolbar" aria-label="Viewport controls">
      <div className="viewport-toolbar-view-switch" role="radiogroup" aria-label="Workspace view">
        <span className="viewport-toolbar-view-label">View</span>
        {VIEW_MODE_OPTIONS.map((option) => (
          <button
            key={option.id}
            type="button"
            role="radio"
            aria-checked={option.id === props.viewMode}
            data-view-mode={option.id}
            title={option.description}
            className={`viewport-toolbar-button${option.id === props.viewMode ? ' is-active' : ''}`}
            onClick={() => props.onViewModeChange(option.id)}
          >
            {option.label}
          </button>
        ))}
      </div>
      <div className="viewport-toolbar-group" role="radiogroup" aria-label="Primitive kind">
        {KIND_OPTIONS.map((option) => (
          <button
            key={option.id}
            type="button"
            role="radio"
            aria-checked={option.id === primitive}
            className={`viewport-toolbar-button${option.id === primitive ? ' is-active' : ''}`}
            onClick={() => onPrimitiveChange(option.id)}
          >
            {option.label}
          </button>
        ))}
      </div>
      <ViewportToggle label="Grid" active={viewport.showGrid} onToggle={() => patchViewport({ showGrid: !viewport.showGrid })} />
      <ViewportToggle label="Axes" active={viewport.showAxes} onToggle={() => patchViewport({ showAxes: !viewport.showAxes })} />
      <ViewportToggle label="Dimensions" active={viewport.showDimensions} onToggle={() => patchViewport({ showDimensions: !viewport.showDimensions })} />
      <div className="viewport-toolbar-group" role="radiogroup" aria-label="Material">
        {MATERIAL_OPTIONS.map((material) => (
          <button
            key={material}
            type="button"
            role="radio"
            aria-checked={viewport.material === material}
            className={`viewport-toolbar-button${viewport.material === material ? ' is-active' : ''}`}
            onClick={() => patchViewport({ material })}
          >
            {material}
          </button>
        ))}
      </div>
      <div className="viewport-toolbar-group" role="radiogroup" aria-label="Background">
        {BACKGROUND_OPTIONS.map((background) => (
          <button
            key={background}
            type="button"
            role="radio"
            aria-checked={viewport.background === background}
            className={`viewport-toolbar-button${viewport.background === background ? ' is-active' : ''}`}
            onClick={() => patchViewport({ background })}
          >
            {background}
          </button>
        ))}
      </div>
      <button
        type="button"
        className="viewport-toolbar-button viewport-toolbar-button-warning"
        onClick={() => dispatch({
          type: 'parameters/reset',
          parameters: primitive === 'box'
            ? { size_x: 40, size_y: 20, size_z: 30, subdivisions: 1 }
            : primitive === 'cylinder'
              ? { radius: 20, height: 40, radial_segments: 48, height_segments: 1 }
              : { radius: 25, rings: 24, sectors: 32 },
        })}
      >
        Reset
      </button>
      <div className="viewport-toolbar-status" aria-live="polite">
        {preview.inFlight
          ? 'Verifying…'
          : preview.lastError
            ? preview.lastErrorKind === 'timeout'
              ? `Timeout: ${preview.lastError}`
              : preview.lastErrorKind === 'cancelled'
                ? 'Cancelled'
                : `Error: ${preview.lastError}`
            : ''}
      </div>
    </div>
  );
}

function ViewportToggle(props: { label: string; active: boolean; onToggle(): void }): ReactElement {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.active}
      className={`viewport-toolbar-button${props.active ? ' is-active' : ''}`}
      onClick={props.onToggle}
    >
      {props.label}
    </button>
  );
}