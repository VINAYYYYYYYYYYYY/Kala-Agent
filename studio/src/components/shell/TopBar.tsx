// Top bar — the app's one permanent command row (44 px).
//
// Four things live here and nowhere else (D3):
//   left   the wordmark only;
//   centre empty — the render mode moved to the drawing panel header;
//   right  Drawing (the split-panel toggle), Details, Export, Settings.
//
// Every control is bound to real store state: the Drawing and Details
// toggles carry `aria-pressed` from the store, Export opens the Export
// dialog, and the gear opens Settings.

import type { ReactElement } from 'react';
import {
  dispatch,
  selectDetailsOpen,
  selectDrawingOpen,
  useUiState,
} from '../../state/store';
import '../../styles/shell.css';

export function TopBar(): ReactElement {
  const drawingOpen = useUiState(selectDrawingOpen);
  const detailsOpen = useUiState(selectDetailsOpen);

  return (
    <header className="top-bar" role="banner" aria-label="Kala Studio">
      <div className="top-bar-left">
        <span className="top-bar-wordmark">
          KALA <span className="top-bar-wordmark-slash" aria-hidden="true">//</span> STUDIO
        </span>
      </div>

      <div className="top-bar-center" aria-hidden="true" />

      <div className="top-bar-right">
        <button
          type="button"
          className={`top-bar-button${drawingOpen ? ' is-active' : ''}`}
          aria-pressed={drawingOpen}
          title={drawingOpen ? 'Hide the drawing' : 'Show the drawing (Ctrl/Cmd+B)'}
          onClick={() => dispatch({ type: 'chrome/drawing-toggle' })}
        >
          Drawing
        </button>
        <button
          type="button"
          className={`top-bar-button${detailsOpen ? ' is-active' : ''}`}
          aria-pressed={detailsOpen}
          title={detailsOpen ? 'Hide the Details panel' : 'Show Properties, Views and Activity'}
          onClick={() => dispatch({ type: 'chrome/details-toggle' })}
        >
          Details
        </button>
        <button
          type="button"
          className="top-bar-button"
          aria-haspopup="dialog"
          title="Export the active design or the primitive"
          onClick={() => dispatch({ type: 'chrome/export-open' })}
        >
          Export
        </button>
        <button
          type="button"
          className="top-bar-icon-button top-bar-settings"
          aria-label="Settings"
          aria-haspopup="dialog"
          title="Settings (Ctrl/Cmd+,)"
          onClick={() => dispatch({ type: 'options/open' })}
        >
          <svg viewBox="0 0 16 16" aria-hidden="true">
            <circle cx="8" cy="8" r="2.2" />
            <path d="M8 2.2v1.4M8 12.4v1.4M2.2 8h1.4M12.4 8h1.4M3.9 3.9l1 1M11.1 11.1l1 1M12.1 3.9l-1 1M4.9 11.1l-1 1" />
          </svg>
        </button>
      </div>
    </header>
  );
}
