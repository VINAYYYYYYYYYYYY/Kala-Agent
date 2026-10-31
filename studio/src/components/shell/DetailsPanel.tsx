// Details panel — the right-hand inspector column (360 px, closed by default).
//
// One section is visible at a time, chosen from the tab strip and bound to
// the store's `detailsTab` (D2/D3):
//   Properties  the active-geometry summary, then the primitive inspector
//               while no design is active;
//   Views       the AuxView (Section / Ortho / Tolerance);
//   Activity    the Agent & Kernel journal.
//
// The panel sits in the shell grid beside the sheet; it never overlays the
// sheet. Escape closes it when the focus is inside it, and a close button
// does the same for pointer users.

import { useEffect, useRef } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import type {
  AuxViewKind,
  BBox,
  DetailsTab,
  PrimitiveKind,
  StudioParameters,
} from '../../types/studio';
import {
  dispatch,
  selectActiveDesign,
  selectChainStatus,
  selectConnection,
  selectDetailsTab,
  selectJournal,
  useUiState,
} from '../../state/store';
import type { DesignController } from '../../state/designController';
import { AuxView } from '../drafting/AuxView';
import { JournalPanel } from '../journal/JournalPanel';
import { ESTIMATE_NOTE, Inspector } from '../inspector/Inspector';
import { ActiveGeometrySummary } from './ActiveGeometrySummary';
import '../../styles/shell.css';

const TABS: ReadonlyArray<{ id: DetailsTab; label: string }> = [
  { id: 'properties', label: 'Properties' },
  { id: 'views', label: 'Views' },
  { id: 'activity', label: 'Activity' },
];

export interface DetailsPanelProps {
  /** Active auxiliary tab (the same state AppShell passed to AuxView). */
  auxView: AuxViewKind;
  /** Active primitive kind, for the AuxView silhouettes. */
  primitive: PrimitiveKind;
  /** Active parameters, in millimetres. */
  parameters: StudioParameters;
  /** Current real mesh bounds (local or server), or null before a preview. */
  bounds: BBox | null;
  /** Shared drawing/section loader; absent in a disconnected shell or a test. */
  designController?: DesignController | null;
}

export function DetailsPanel(props: DetailsPanelProps): ReactElement {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const tab = useUiState(selectDetailsTab);
  const activeDesign = useUiState(selectActiveDesign);
  const journal = useUiState(selectJournal);
  const connection = useUiState(selectConnection);
  const chainStatus = useUiState(selectChainStatus);
  const firstBadSequence = useUiState((s) => s.sessionJournal.firstBadSequence);

  // When the panel is opened with the keyboard shortcut or the top bar the
  // focus may still be on the trigger. The first tab is a stable landing
  // point inside the panel, so Escape and the arrow keys have somewhere to
  // start from.
  useEffect(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && panelRef.current?.contains(active)) return;
    panelRef.current?.querySelector<HTMLButtonElement>('[role="tab"]')?.focus();
  }, []);

  function onKeyDown(event: KeyboardEvent<HTMLElement>): void {
    if (event.key !== 'Escape') return;
    // Settings is a modal over the panel; its own Escape handler wins.
    if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
    event.stopPropagation();
    dispatch({ type: 'chrome/details-close' });
  }

  function selectTab(next: DetailsTab): void {
    if (next === tab) return;
    dispatch({ type: 'chrome/details-open', tab: next });
  }

  return (
    <aside
      className="details-panel studio-zone studio-zone-right"
      role="complementary"
      aria-label="Details"
      onKeyDown={onKeyDown}
    >
      <div className="details-panel-frame" ref={panelRef}>
        <header className="details-panel-header">
          <div className="details-panel-tabs" role="tablist" aria-label="Details sections">
            {TABS.map((entry) => (
              <button
                key={entry.id}
                type="button"
                role="tab"
                id={`details-tab-${entry.id}`}
                className={`details-panel-tab${tab === entry.id ? ' is-active' : ''}`}
                aria-selected={tab === entry.id}
                aria-controls={`details-panel-${entry.id}`}
                tabIndex={tab === entry.id ? 0 : -1}
                onClick={() => selectTab(entry.id)}
              >
                {entry.label}
              </button>
            ))}
          </div>
          <button
            type="button"
            className="details-panel-close"
            aria-label="Close details"
            title="Close Details (Esc)"
            onClick={() => dispatch({ type: 'chrome/details-close' })}
          >
            ✕
          </button>
        </header>

        <div
          className={`details-panel-body details-panel-body-${tab}`}
          id={`details-panel-${tab}`}
          role="tabpanel"
          aria-labelledby={`details-tab-${tab}`}
        >
          {tab === 'properties' ? (
            <>
              <ActiveGeometrySummary />
              {activeDesign === null ? (
                <>
                  <p className="details-geometry-note">{ESTIMATE_NOTE}</p>
                  <Inspector showHeading={false} showReadouts={false} />
                </>
              ) : null}
            </>
          ) : null}
          {tab === 'views' ? (
            <AuxView
              auxView={props.auxView}
              onAuxViewChange={(view) => dispatch({ type: 'auxView/set', view })}
              primitive={props.primitive}
              parameters={props.parameters}
              bounds={props.bounds}
              designController={props.designController ?? null}
            />
          ) : null}
          {tab === 'activity' ? (
            <JournalPanel
              entries={journal}
              connectionStatus={connection}
              hashChainStatus={chainStatus}
              firstBadSequence={firstBadSequence}
              showFooter={false}
            />
          ) : null}
        </div>
      </div>
    </aside>
  );
}
