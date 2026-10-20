// Center Vellum drafting sheet.
//
// The sheet owns the app's single SceneCanvas and the drafted furniture
// around it: a double frame with A–F / 1–8 coordinate ticks, a pencil grid
// and paper grain (CSS/SVG only), a floating 2D/3D mode pill, a small tool
// palette wired to real store actions, the redline A–A flags, and a title
// block built exclusively from the values the shell supplies.
//
// There are no decorative controls: every palette button dispatches an
// action that exists in the store (reset view, dimensions, section, grid).

import { useEffect, useMemo, useState } from 'react';
import type { ReactElement } from 'react';
import { SceneCanvas } from '../../viewer/SceneCanvas';
import { HlrDrawing } from '../../viewer/HlrDrawing';
import { DRAFTING_SCALE } from '../../viewer/draftingGeometry';
import { measureStatusLabel } from '../../viewer/measure';
import type { MeasurePoint } from '../../viewer/measure';
import {
  activateDesign,
  clearActiveDesign,
  designToDisplayMesh,
  dispatch,
  drawingCacheKey,
  getState,
  sectionCacheKey,
  selectActiveDesign,
  selectDesigns,
  selectDrawing,
  selectSection,
  selectSectionCut,
  useUiState,
} from '../../state/store';
import type { BackgroundKind, MaterialKind } from '../../state/store';
import type { DesignController } from '../../state/designController';
import { PRIMITIVE_SECTION_CUT, sectionCutNote } from '../../state/sectionCut';
import { measuredByLabel } from '../chat/designCard';
import type {
  BBox,
  PreviewResponse,
  PrimitiveKind,
  RenderMode,
  StudioLocalPreview,
  StudioParameters,
  StudioUnits,
} from '../../types/studio';
import '../../styles/drafting.css';

/**
 * Real title-block values. Every field is derived from workspace state or
 * is an explicit empty state — no revision, tolerance, or verification is
 * invented.
 */
export interface TitleBlockData {
  sheet: string;
  partId: string;
  title: string;
  scale: string;
  units: string;
  projection: string;
  tolerance: string;
  revision: string;
  author: string;
}

/**
 * FINAL prop contract for the center drafting sheet.
 *
 * `viewResetToken` increments on the bottom-bar Reset View action and is
 * forwarded to SceneCanvas, which re-frames the live camera without
 * remounting the canvas.
 */
export interface DraftingSheetProps {
  /** What SceneCanvas needs: the current mesh and its provenance. */
  mesh: StudioLocalPreview | PreviewResponse | null;
  meshSource: 'local' | 'server' | 'none';
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
  /** 2D pencil drafting mode or 3D clay solid mode. */
  renderMode: RenderMode;
  /** Whether the Section A-A cut plane is shown. */
  showSection: boolean;
  /** Real title-block labels for the active part. */
  titleBlock: TitleBlockData;
  /** Millimetres; the backend primitive contract is metric. */
  units: StudioUnits;
  /** Increments when the operator asks to re-frame the view. */
  viewResetToken: number;
  /**
   * The shared drawing/section controller. When absent (a DOM test or a
   * disconnected shell) the sheet still renders whatever the store already
   * holds; it just cannot request a drawing.
   */
  designController?: DesignController | null;
  /** The active primitive, used only to shape the clay design mesh. */
  primitive?: PrimitiveKind;
  /** The active parameters, used only to shape the clay design mesh. */
  parameters?: StudioParameters;
}

const HORIZONTAL_TICKS = ['1', '2', '3', '4', '5', '6', '7', '8'] as const;
const VERTICAL_TICKS = ['A', 'B', 'C', 'D', 'E', 'F'] as const;

export function DraftingSheet(props: DraftingSheetProps): ReactElement {
  const activeDesign = useUiState(selectActiveDesign);
  const designState = useUiState(selectDesigns);
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const designId = activeDesign?.design_id ?? null;
  const drawingKey = designId ? drawingCacheKey(designId, []) : null;
  const drawingResource = useUiState((state) =>
    drawingKey ? selectDrawing(state, drawingKey) : null,
  );
  const [closedDesignTabs, setClosedDesignTabs] = useState<ReadonlySet<string>>(
    () => new Set<string>(),
  );

  // A design is a static drawing, not a three.js primitive: it has no
  // primitive section profile. The clay mode keeps the imported design view.
  const measure = useUiState((s) => s.measure);
  // One cutting plane for the whole workspace. The sheet note, the redline
  // trace in the HLR views, the AuxView section request, and the clay plane
  // all read this state; a design defaults to XZ at y = 4 mm, a primitive to
  // the Z mid-plane (XY with the backend's default offset).
  const sharedCut = useUiState(selectSectionCut);
  const cut = activeDesign ? sharedCut : PRIMITIVE_SECTION_CUT;
  // The view bounds are always present, even when the provider reported no
  // measurements of its own. They are the envelope the mid-plane falls back
  // to for a default cut.
  const cutBounds: BBox | null = activeDesign?.view.bounds ?? props.mesh?.bounds ?? null;
  const cutNote = sectionCutNote(cut, cutBounds);
  const unitsLabel = `METRIC (${props.units.toUpperCase()})`;
  const visibleDesignIds = designState.order.filter((id) => !closedDesignTabs.has(id));

  // The sheet's redline is the measured section itself, so the section at
  // the shared cut is requested alongside the drawing. The AuxView and the
  // sheet use the same cache key, so only one request is made.
  const sectionKey = designId
    ? sectionCacheKey(designId, cut.plane, cut.offsetMm ?? undefined)
    : null;
  const sectionResource = useUiState((state) =>
    sectionKey ? selectSection(state, sectionKey) : null,
  );

  // Request the HLR drawing when the static sheet is actually showing it.
  // The drawing is cached per design, so switching modes back and forth does
  // not re-request it.
  useEffect(() => {
    if (!designId || !props.designController) return;
    if (props.renderMode !== 'pencil2d') return;
    if (designAvailable !== true) return;
    void props.designController.loadDrawing(designId);
    if (props.showSection) {
      void props.designController.loadSection(designId, cut.plane, cut.offsetMm ?? undefined);
    }
  }, [
    designId,
    props.designController,
    props.renderMode,
    designAvailable,
    props.showSection,
    cut.plane,
    cut.offsetMm,
  ]);

  // A design that becomes active is shown even if its tab was closed: the
  // tab strip is a view, the store's LRU is the session record.
  useEffect(() => {
    if (!designId) return;
    setClosedDesignTabs((previous) => {
      if (!previous.has(designId)) return previous;
      const next = new Set(previous);
      next.delete(designId);
      return next;
    });
  }, [designId]);

  const designTitle = useMemo(() => {
    if (!activeDesign) return null;
    return {
      partId: activeDesign.design_id.slice(0, 8),
      drawing: activeDesign.label,
      measuredBy: measuredByLabel(activeDesign),
      material: activeDesign.material && activeDesign.material.trim().length > 0
        ? activeDesign.material
        : '—',
      tolerances: activeDesign.tolerances.length > 0
        ? activeDesign.tolerances.join(' · ')
        : '—',
      scale: DRAFTING_SCALE,
      units: unitsLabel,
      projection: '3RD ANGLE',
      revision: '—',
    };
  }, [activeDesign, unitsLabel]);

  // The clay viewport shows the design's own measured mesh. The object is
  // memoised so an unrelated shell re-render never rebuilds the GPU
  // geometry (the same stability rule as the primitive viewport).
  const designMesh = useMemo<StudioLocalPreview | null>(() => {
    if (!activeDesign) return null;
    const converted = designToDisplayMesh(activeDesign);
    return {
      kind: props.primitive ?? 'box',
      parameters: props.parameters ?? { size_x: 1, size_y: 1, size_z: 1, subdivisions: 1 },
      positions: converted.positions,
      normals: converted.normals,
      indices: converted.indices,
      bounds: converted.bounds,
      source: 'local',
      generatedAt: 0,
      engine: converted.engine,
      warnings: converted.warnings,
    };
  }, [activeDesign, props.primitive, props.parameters]);

  const showHlrDrawing = props.renderMode === 'pencil2d' && activeDesign !== null;
  const viewportMesh = activeDesign ? designMesh : props.mesh;
  const viewportMeshSource: 'local' | 'server' | 'design' | 'none' = activeDesign
    ? 'design'
    : props.meshSource;
  const scaleLabel = activeDesign ? DRAFTING_SCALE : props.titleBlock.scale;

  function closeDesignTab(id: string): void {
    const remaining = visibleDesignIds.filter((candidate) => candidate !== id);
    setClosedDesignTabs((previous) => {
      if (previous.has(id)) return previous;
      const next = new Set(previous);
      next.add(id);
      return next;
    });
    // Closing the active tab moves the sheet to the nearest remaining tab.
    // The design itself stays in the store's LRU either way.
    if (designId !== id) return;
    const nextActive = remaining[remaining.length - 1] ?? null;
    if (nextActive) activateDesign(nextActive);
    else clearActiveDesign();
  }

  // Escape cancels the measure tool. The Options drawer owns Escape while
  // it is open, so the sheet only reacts when no drawer is showing.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      if (event.key !== 'Escape') return;
      const state = getState();
      if (state.optionsOpen) return;
      if (!state.measure.active) return;
      dispatch({ type: 'measure/clear' });
    }
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, []);

  // A pick is a real raycast hit in model millimetres. A click that missed
  // the mesh reports `null` and is ignored: a measurement is never invented.
  function onMeasurePoint(point: MeasurePoint | null): void {
    if (!point) return;
    dispatch({ type: 'measure/point', point });
  }

  return (
    <section
      className="drafting-sheet"
      data-render-mode={props.renderMode}
      data-section={props.showSection ? 'on' : 'off'}
      aria-label="Vellum drafting sheet"
    >
      <div className="drafting-sheet-frame" aria-hidden="true">
        {HORIZONTAL_TICKS.map((label, index) => (
          <span
            key={`top-${label}`}
            className="drafting-coord drafting-coord-top"
            style={{ left: `${((index + 0.5) / HORIZONTAL_TICKS.length) * 100}%` }}
          >
            {label}
          </span>
        ))}
        {HORIZONTAL_TICKS.map((label, index) => (
          <span
            key={`bottom-${label}`}
            className="drafting-coord drafting-coord-bottom"
            style={{ left: `${((index + 0.5) / HORIZONTAL_TICKS.length) * 100}%` }}
          >
            {label}
          </span>
        ))}
        {VERTICAL_TICKS.map((label, index) => (
          <span
            key={`left-${label}`}
            className="drafting-coord drafting-coord-left"
            style={{ top: `${((index + 0.5) / VERTICAL_TICKS.length) * 100}%` }}
          >
            {label}
          </span>
        ))}
        {VERTICAL_TICKS.map((label, index) => (
          <span
            key={`right-${label}`}
            className="drafting-coord drafting-coord-right"
            style={{ top: `${((index + 0.5) / VERTICAL_TICKS.length) * 100}%` }}
          >
            {label}
          </span>
        ))}
        {Array.from({ length: HORIZONTAL_TICKS.length + 1 }, (_, index) => (
          <span
            key={`tick-top-${index}`}
            className="drafting-tick drafting-tick-top"
            style={{ left: `${(index / HORIZONTAL_TICKS.length) * 100}%` }}
          />
        ))}
        {Array.from({ length: HORIZONTAL_TICKS.length + 1 }, (_, index) => (
          <span
            key={`tick-bottom-${index}`}
            className="drafting-tick drafting-tick-bottom"
            style={{ left: `${(index / HORIZONTAL_TICKS.length) * 100}%` }}
          />
        ))}
        {Array.from({ length: VERTICAL_TICKS.length + 1 }, (_, index) => (
          <span
            key={`tick-left-${index}`}
            className="drafting-tick drafting-tick-left"
            style={{ top: `${(index / VERTICAL_TICKS.length) * 100}%` }}
          />
        ))}
        {Array.from({ length: VERTICAL_TICKS.length + 1 }, (_, index) => (
          <span
            key={`tick-right-${index}`}
            className="drafting-tick drafting-tick-right"
            style={{ top: `${(index / VERTICAL_TICKS.length) * 100}%` }}
          />
        ))}
        <span className="drafting-pin drafting-pin-tl">+</span>
        <span className="drafting-pin drafting-pin-tr">+</span>
        <span className="drafting-pin drafting-pin-bl">+</span>
        <span className="drafting-pin drafting-pin-br">+</span>
      </div>

      <div className="drafting-sheet-tabline">
        <div className="drafting-sheet-tabs">
          {designAvailable === true ? (
            <>
              {visibleDesignIds.map((id) => {
                const design = designState.designs.get(id);
                const label = design?.label ?? 'Design';
                const active = id === designId;
                return (
                  <span
                    key={id}
                    className={`drafting-sheet-tab drafting-design-tab${active ? ' is-active' : ''}`}
                    data-design-id={id}
                  >
                    <button
                      type="button"
                      className="drafting-design-tab-select"
                      aria-pressed={active}
                      title={`${label} · ${id}`}
                      onClick={() => activateDesign(id)}
                    >
                      <span className="drafting-design-tab-label">{label}</span>
                      <span className="drafting-design-tab-id">{id.slice(0, 8)}</span>
                    </button>
                    <button
                      type="button"
                      className="drafting-design-tab-close"
                      aria-label={`Close design tab: ${label}`}
                      title="Remove from the tab strip; the design stays in the session"
                      onClick={() => closeDesignTab(id)}
                    >
                      ×
                    </button>
                  </span>
                );
              })}
              <button
                type="button"
                className="drafting-sheet-tab drafting-design-tab drafting-design-tab-add"
                aria-label="New design sheet"
                title="New design sheet — focuses the agent composer with a starter"
                onClick={() => prefillComposer(DESIGN_STARTER_PROMPT)}
              >
                +
              </button>
            </>
          ) : (
            <span className="drafting-sheet-tab is-active" title="Active drawing sheet">
              {props.titleBlock.sheet}
            </span>
          )}
          {activeDesign === null ? (
            <span className="drafting-sheet-scale">SCALE {scaleLabel}</span>
          ) : null}
        </div>
        <div className="drafting-mode-pill" role="group" aria-label="Render mode">
          <button
            type="button"
            className={`drafting-mode-button${props.renderMode === 'pencil2d' ? ' is-active' : ''}`}
            aria-pressed={props.renderMode === 'pencil2d'}
            title="2D pencil vector drafting"
            data-render-mode="pencil2d"
            onClick={() => dispatch({ type: 'renderMode/set', mode: 'pencil2d' })}
          >
            ✎ 2D Pencil Vector
          </button>
          <span className="drafting-mode-divider" aria-hidden="true">|</span>
          <button
            type="button"
            className={`drafting-mode-button${props.renderMode === 'clay3d' ? ' is-active' : ''}`}
            aria-pressed={props.renderMode === 'clay3d'}
            title="3D warm clay solid"
            data-render-mode="clay3d"
            onClick={() => dispatch({ type: 'renderMode/set', mode: 'clay3d' })}
          >
            ⬡ 3D Clay
          </button>
        </div>
      </div>

      <div className="drafting-tools-wrap">
        <div className="drafting-tools" role="toolbar" aria-label="Drafting tools">
          <button
            type="button"
            className="drafting-tool"
            aria-label="Reset view"
            title="Re-frame the current part"
            onClick={() => dispatch({ type: 'view/reset' })}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M3 8a5 5 0 1 0 1.6-3.7" />
              <path d="M3 3.5V7h3.5" />
            </svg>
          </button>
          <button
            type="button"
            className={`drafting-tool${props.showDimensions ? ' is-active' : ''}`}
            aria-label="Toggle dimensions"
            aria-pressed={props.showDimensions}
            title="Toggle redline dimensions"
            onClick={() =>
              dispatch({ type: 'viewport/set', patch: { showDimensions: !props.showDimensions } })
            }
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M2.5 5v6M13.5 5v6M2.5 8h11" />
              <path d="M5 6.5 2.5 8 5 9.5M11 6.5 13.5 8 11 9.5" />
            </svg>
          </button>
          <button
            type="button"
            className={`drafting-tool${measure.active ? ' is-active' : ''}`}
            aria-label="Measure distance"
            aria-pressed={measure.active}
            disabled={!props.mesh || activeDesign !== null}
            title={
              activeDesign !== null
                ? 'Measure is available on the 3D clay viewport, not the static HLR sheet'
                : props.mesh
                  ? 'Measure a real distance between two points on the mesh'
                  : 'Measure needs a mesh: run a preview first'
            }
            onClick={() => dispatch({ type: 'measure/toggle' })}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M2 11.2 11.2 2l2.8 2.8L4.8 14z" />
              <path d="M4.9 8.3 6.1 9.5M6.7 6.5 7.9 7.7M8.5 4.7 9.7 5.9" />
            </svg>
          </button>
          <button
            type="button"
            className={`drafting-tool drafting-tool-section${props.showSection ? ' is-active' : ''}`}
            aria-label="Toggle Section A-A"
            aria-pressed={props.showSection}
            title="Toggle the A–A cutting plane"
            onClick={() => dispatch({ type: 'section/toggle' })}
          >
            A–A
          </button>
          <button
            type="button"
            className={`drafting-tool${props.showGrid ? ' is-active' : ''}`}
            aria-label="Toggle grid"
            aria-pressed={props.showGrid}
            title="Toggle the pencil grid"
            onClick={() => dispatch({ type: 'viewport/set', patch: { showGrid: !props.showGrid } })}
          >
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path d="M2.5 2.5h11v11h-11zM2.5 6.5h11M2.5 10.5h11M6.5 2.5v11M10.5 2.5v11" />
            </svg>
          </button>
        </div>
        {measure.active ? (
          <p className="drafting-measure-note" role="status">
            {measureStatusLabel(measure)}
          </p>
        ) : null}
      </div>

      <div className="drafting-sheet-stage">
        {showHlrDrawing && activeDesign ? (
          <HlrDrawing
            design={activeDesign}
            drawing={drawingResource?.data ?? null}
            status={drawingResource?.status ?? 'idle'}
            error={drawingResource?.error ?? null}
            showDimensions={props.showDimensions}
            viewResetToken={props.viewResetToken}
            section={sectionResource?.data ?? null}
            cut={cut}
            onRetry={() => {
              void props.designController?.loadDrawing(activeDesign.design_id);
            }}
          />
        ) : (
          <SceneCanvas
            mesh={viewportMesh}
            meshSource={viewportMeshSource}
            designMesh={activeDesign !== null}
            designProvider={activeDesign?.provider ?? null}
            showGrid={props.showGrid}
            showAxes={props.showAxes}
            showDimensions={props.showDimensions}
            material={props.material}
            background={props.background}
            renderMode={props.renderMode}
            showSection={props.showSection}
            sectionCut={cut}
            viewResetToken={props.viewResetToken}
            measure={measure}
            onMeasurePoint={onMeasurePoint}
          />
        )}
      </div>

      {props.showSection && activeDesign === null ? (
        <div className="drafting-section-mark" aria-hidden="true">
          <span className="drafting-section-flag">A</span>
          <span className="drafting-section-rule" />
          <span className="drafting-section-flag">A</span>
        </div>
      ) : null}

      <div className="drafting-stage-note" aria-hidden="true">
        {showHlrDrawing ? 'DRAG PAN · SCROLL ZOOM' : 'DRAG ORBIT · SCROLL ZOOM'}
      </div>

      {props.showSection && (props.mesh || activeDesign) && !showHlrDrawing ? (
        <div
          className={`drafting-cut-note${activeDesign ? ' is-design' : ''}`}
          data-cut-plane={cut.plane}
          data-cut-offset={cut.offsetMm === null ? 'default' : String(cut.offsetMm)}
          aria-hidden="true"
        >
          {cutNote}
        </div>
      ) : null}

      <div className="drafting-title-block" data-title-block={designTitle ? 'design' : 'primitive'}>
        {designTitle && activeDesign ? (
          <>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">SUBJECT // DESIGN ID</span>
              <span className="drafting-title-value" title={activeDesign.design_id}>
                {designTitle.partId}
              </span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">DRAWING</span>
              <span className="drafting-title-value" title={designTitle.drawing}>
                {designTitle.drawing}
              </span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">MEASURED BY</span>
              <span className="drafting-title-value" title={designTitle.measuredBy}>
                {designTitle.measuredBy}
              </span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">MATERIAL (DECLARED)</span>
              <span className="drafting-title-value">{designTitle.material}</span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">TOLERANCES (DECLARED)</span>
              <span className="drafting-title-value" title={designTitle.tolerances}>
                {designTitle.tolerances}
              </span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">SCALE</span>
              <span className="drafting-title-value">{designTitle.scale}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">UNITS</span>
              <span className="drafting-title-value">{designTitle.units}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">PROJECTION</span>
              <span className="drafting-title-value">{designTitle.projection}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">REV</span>
              <span className="drafting-title-value">{designTitle.revision}</span>
            </div>
          </>
        ) : (
          <>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">SUBJECT // PART ID</span>
              <span className="drafting-title-value">{props.titleBlock.partId}</span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-wide">
              <span className="drafting-title-label">DRAWING</span>
              <span className="drafting-title-value">{props.titleBlock.title}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">SCALE</span>
              <span className="drafting-title-value">{props.titleBlock.scale}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">UNITS</span>
              <span className="drafting-title-value">{unitsLabel}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">PROJECTION</span>
              <span className="drafting-title-value">{props.titleBlock.projection || '—'}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">SHEET</span>
              <span className="drafting-title-value">{props.titleBlock.sheet}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">TOLERANCE</span>
              <span className="drafting-title-value">{props.titleBlock.tolerance || '—'}</span>
            </div>
            <div className="drafting-title-cell">
              <span className="drafting-title-label">REV</span>
              <span className="drafting-title-value">{props.titleBlock.revision || '—'}</span>
            </div>
            <div className="drafting-title-cell drafting-title-cell-author">
              <span className="drafting-title-label">AUTHOR</span>
              <span className="drafting-title-value">{props.titleBlock.author || '—'}</span>
            </div>
          </>
        )}
      </div>
    </section>
  );
}

/**
 * The starter a "+" sheet tab offers. It is a real design prompt for the
 * local parser, not a fake sheet: clicking "+" only focuses the composer.
 */
export const DESIGN_STARTER_PROMPT =
  'Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes.';

/**
 * Put a starter into the chat composer and focus it. The composer is a
 * React-controlled textarea, so the native value setter is used to keep
 * React's value tracker in sync before dispatching the input event.
 */
export function prefillComposer(prompt: string): void {
  if (typeof document === 'undefined') return;
  const textarea = document.querySelector<HTMLTextAreaElement>('textarea.chat-panel-prompt');
  if (!textarea) return;
  textarea.focus();
  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(textarea) as object,
    'value',
  )?.set;
  if (typeof setter === 'function') {
    setter.call(textarea, prompt);
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
  } else {
    textarea.value = prompt;
  }
}
