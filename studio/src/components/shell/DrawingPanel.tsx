// Drawing panel — the secondary split panel that appears beside the chat.
//
// App-feel wave, chat-first direction (2026-10-31): rendering is not the
// main surface. The chat is the centred, primary column; the drawing is a
// split panel that opens when a CAD-built design arrives or when the
// operator asks for it. It is the existing DraftingSheet under a slim panel
// header (the same mono-caps strip as Details):
//
//   DRAWING  <design or primitive name>   [2D Pencil | 3D Clay]   [✕]
//
// The header is the single home of the render mode (D3): the top bar no
// longer carries a second copy. The close button unmounts the whole panel,
// and with it the one SceneCanvas the sheet owns.

import type { ReactElement } from 'react';
import { dispatch, selectRenderMode, useUiState } from '../../state/store';
import type { RenderMode } from '../../types/studio';
import { DraftingSheet } from '../drafting/DraftingSheet';
import type { DraftingSheetProps } from '../drafting/DraftingSheet';
import '../../styles/shell.css';

const RENDER_MODES: ReadonlyArray<{ id: RenderMode; label: string; title: string }> = [
  { id: 'pencil2d', label: '2D Pencil', title: '2D pencil vector drafting' },
  { id: 'clay3d', label: '3D Clay', title: '3D warm clay solid' },
];

export interface DrawingPanelProps extends DraftingSheetProps {
  /**
   * The active design's label, or the primitive name when no design is
   * active. It is a real store value, never a placeholder.
   */
  name: string;
}

export function DrawingPanel(props: DrawingPanelProps): ReactElement {
  const { name, ...sheetProps } = props;
  const renderMode = useUiState(selectRenderMode);

  return (
    <section className="studio-drawing-column" aria-label="Drawing">
      <header className="drawing-panel-header">
        <div className="drawing-panel-ident">
          <span className="drawing-panel-label">DRAWING</span>
          <span className="drawing-panel-name" title={name}>
            {name}
          </span>
        </div>
        <div className="drawing-panel-mode" role="group" aria-label="Render mode">
          {RENDER_MODES.map((mode) => (
            <button
              key={mode.id}
              type="button"
              className={`drawing-panel-mode-button${renderMode === mode.id ? ' is-active' : ''}`}
              aria-pressed={renderMode === mode.id}
              data-render-mode={mode.id}
              title={mode.title}
              onClick={() => dispatch({ type: 'renderMode/set', mode: mode.id })}
            >
              {mode.label}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="drawing-panel-close"
          aria-label="Close drawing"
          title="Close the drawing (Ctrl/Cmd+B)"
          onClick={() => dispatch({ type: 'chrome/drawing-close' })}
        >
          ✕
        </button>
      </header>
      <DraftingSheet {...sheetProps} />
    </section>
  );
}
