// Right-top auxiliary drafted view.
//
// Three tabs, all backed by real inputs:
//   Section A–A — with a design: the backend's /section loops as an evenodd
//                 path with a 45° hatch, plus the exact measured area. With a
//                 primitive: the profile computed from the active parameters.
//   Ortho 3-View — with a design: the /drawing front/top/right HLR views in
//                 third-angle arrangement. With a primitive: the three
//                 silhouettes projected from the live mesh bounds.
//   Tolerance    — with a design: the DECLARED list, labelled "declared —
//                 not analysed". With a primitive: an explicit empty state.
//                 A backend without design support gets the honest note.
//
// The SVG hatching is an ANSI 45° symbolic fill; it is not a material claim.

import { useEffect, useId } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import type {
  AuxViewKind,
  BBox,
  DesignResult,
  DrawingResponse,
  DrawingView,
  PrimitiveKind,
  SectionPlane,
  SectionResponse,
  StudioParameters,
} from '../../types/studio';
import { SECTION_PLANES } from '../../types/studio';
import {
  computeOrthoViews,
  computeSectionView,
  fitToBox,
  formatMmValue,
  type OrthoView,
  type SectionView,
} from '../../viewer/draftingGeometry';
import { fitPolylineGroups, type DrawingCell } from '../../viewer/hlr';
import {
  drawingCacheKey,
  sectionCacheKey,
  selectActiveDesign,
  selectDrawing,
  selectSection,
  selectSectionCut,
  setSectionPlane,
  useUiState,
} from '../../state/store';
import type { DesignResource } from '../../state/store';
import type { DesignController } from '../../state/designController';
import { DEFAULT_DESIGN_SECTION_CUT, sectionCutNote } from '../../state/sectionCut';
import { measuredByLabel } from '../chat/designCard';
import '../../styles/drafting.css';

/**
 * FINAL prop contract for the right-top auxiliary view.
 */
export interface AuxViewProps {
  /** Active aux tab. */
  auxView: AuxViewKind;
  /** Called when the operator picks another tab. */
  onAuxViewChange(view: AuxViewKind): void;
  /** Active primitive kind. */
  primitive: PrimitiveKind;
  /** Active parameters, in millimetres. */
  parameters: StudioParameters;
  /** Current real mesh bounds (local or server), or null before a preview. */
  bounds: BBox | null;
  /** Shared drawing/section loader; absent in a disconnected shell or a test. */
  designController?: DesignController | null;
}

const TABS: ReadonlyArray<{ id: AuxViewKind; label: string }> = [
  { id: 'section', label: 'Section A-A' },
  { id: 'ortho', label: 'Ortho 3-View' },
  { id: 'tolerance', label: 'Tolerance' },
];

const SECTION_SVG_WIDTH = 300;
const SECTION_SVG_HEIGHT = 168;

export function AuxView(props: AuxViewProps): ReactElement {
  const rawId = useId();
  const baseId = `aux-${rawId.replace(/[^a-zA-Z0-9_-]/g, '')}`;
  const activeDesign = useUiState(selectActiveDesign);
  const designAvailable = useUiState((s) => s.server.designAvailable);
  // The plane is shared with the sheet, the HLR redline, and the clay
  // viewport. There is one state, not a private AuxView tab state: changing
  // it here moves the A–A line and the sheet label in the same render.
  const cut = useUiState(selectSectionCut);
  const sectionPlane = cut.plane;
  // The XZ Section A–A is cut through the upright wall and the base at
  // y = 4 mm (ADR-0009 criterion 4: the measured L-bracket area is
  // 2400 − 25π = 2321.46 mm²). The backend's XZ default is the mid-plane of
  // the whole envelope, which cuts only the base for this part; the shared
  // state names the offset it requests and the returned `offset_mm` is
  // displayed. XY and YZ keep the backend's documented mid-plane default.
  const sectionOffset = cut.offsetMm ?? undefined;

  const designId = activeDesign?.design_id ?? null;
  const drawingKey = designId ? drawingCacheKey(designId, []) : null;
  const drawingResource = useUiState((state) =>
    drawingKey ? selectDrawing(state, drawingKey) : null,
  );
  const sectionKey = designId ? sectionCacheKey(designId, sectionPlane, sectionOffset) : null;
  const sectionResource = useUiState((state) =>
    sectionKey ? selectSection(state, sectionKey) : null,
  );

  // A new design gets the documented default cut (XZ at y = 4 mm). The
  // shared state is reset here, so the sheet and the redline follow too.
  useEffect(() => {
    if (!designId) return;
    setSectionPlane(DEFAULT_DESIGN_SECTION_CUT.plane);
  }, [designId]);

  useEffect(() => {
    if (!designId || !props.designController) return;
    if (props.auxView !== 'ortho') return;
    if (designAvailable !== true) return;
    void props.designController.loadDrawing(designId);
  }, [designId, props.auxView, props.designController, designAvailable]);

  useEffect(() => {
    if (!designId || !props.designController) return;
    if (props.auxView !== 'section') return;
    if (designAvailable !== true) return;
    void props.designController.loadSection(designId, sectionPlane, sectionOffset);
  }, [designId, props.auxView, sectionPlane, sectionOffset, props.designController, designAvailable]);

  // The section profile is cut on the real Z mid-plane of the live mesh
  // bounds, so the label names the plane the viewport actually draws.
  const section = activeDesign ? null : computeSectionView(props.primitive, props.parameters, props.bounds);
  const ortho = activeDesign ? null : computeOrthoViews(props.primitive, props.parameters, props.bounds);
  const active = TABS.find((tab) => tab.id === props.auxView) ?? TABS[0]!;
  const sectionResponse = sectionResource?.data ?? null;

  const onTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>): void => {
    const index = TABS.findIndex((tab) => tab.id === props.auxView);
    let nextIndex: number | null = null;
    if (event.key === 'ArrowRight') nextIndex = (index + 1) % TABS.length;
    else if (event.key === 'ArrowLeft') nextIndex = (index - 1 + TABS.length) % TABS.length;
    else if (event.key === 'Home') nextIndex = 0;
    else if (event.key === 'End') nextIndex = TABS.length - 1;
    if (nextIndex === null) return;
    event.preventDefault();
    props.onAuxViewChange(TABS[nextIndex]!.id);
    const buttons = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
      '[role="tab"]',
    );
    buttons?.[nextIndex]?.focus();
  };

  const cutNote =
    props.auxView !== 'section' && activeDesign
      ? props.auxView === 'ortho'
        ? 'PROJ: 3RD ANGLE'
        : 'SOURCE: DECLARED'
      : 'CUT: 45° ANSI';

  return (
    <section className="aux-view" aria-label="Auxiliary drafting view">
      <div className="aux-view-header">
        <div className="aux-view-tabs" role="tablist" aria-label="Auxiliary view">
          {TABS.map((tab) => {
            const selected = tab.id === props.auxView;
            return (
              <button
                key={tab.id}
                type="button"
                role="tab"
                id={`${baseId}-tab-${tab.id}`}
                aria-selected={selected}
                aria-controls={`${baseId}-panel`}
                tabIndex={selected ? 0 : -1}
                className={`aux-view-tab${selected ? ' is-active' : ''}`}
                onClick={() => props.onAuxViewChange(tab.id)}
                onKeyDown={onTabKeyDown}
              >
                {tab.label}
              </button>
            );
          })}
        </div>
        {/* The 45° ANSI hatch is a symbolic section convention, not a
            material claim: it belongs in the header next to the tabs. */}
        <span className="aux-view-cut-note">{cutNote}</span>
      </div>

      <div
        className="aux-view-body"
        role="tabpanel"
        id={`${baseId}-panel`}
        aria-labelledby={`${baseId}-tab-${active.id}`}
      >
        {props.auxView === 'section' ? (
          activeDesign ? (
            <DesignSectionPanel
              section={sectionResource}
              plane={sectionPlane}
              onPlaneChange={setSectionPlane}
              onRetry={() => {
                if (props.designController) {
                  void props.designController.loadSection(
                    activeDesign.design_id,
                    sectionPlane,
                    sectionOffset,
                  );
                }
              }}
              baseId={baseId}
            />
          ) : (
            <SectionPanel section={section!} baseId={baseId} />
          )
        ) : null}
        {props.auxView === 'ortho' ? (
          activeDesign ? (
            <DesignOrthoPanel
              drawing={drawingResource}
              onRetry={() => {
                if (props.designController) {
                  void props.designController.loadDrawing(activeDesign.design_id);
                }
              }}
            />
          ) : (
            <OrthoPanel views={ortho} bounds={props.bounds} baseId={baseId} />
          )
        ) : null}
        {props.auxView === 'tolerance' ? (
          activeDesign ? (
            <DesignTolerancePanel design={activeDesign} />
          ) : (
            <>
              {designAvailable !== true ? (
                <p className="aux-backend-note" role="status">
                  {designAvailable === false
                    ? 'Design turns need the Python geometry backend; this backend reports design_available: false'
                    : 'Design turns need the Python geometry backend; this backend did not report design_available'}
                </p>
              ) : null}
              <TolerancePanel />
            </>
          )
        ) : null}
      </div>

      <footer className="aux-view-footer">
        {props.auxView === 'section' ? (
          <span>
            {activeDesign
              ? sectionCutNote(cut, activeDesign.view.bounds)
              : section!.kind === 'unavailable'
                ? 'PROFILE: —'
                : `PLANE: ${section!.planeLabel}`}
          </span>
        ) : null}
        {props.auxView === 'section' && activeDesign ? (
          <span>{sectionResponse ? `AREA: ${formatAreaValue(sectionResponse.area_mm2)} mm²` : 'AREA: —'}</span>
        ) : null}
        {props.auxView === 'ortho' ? (
          <>
            <span>PROJ: 3RD ANGLE</span>
            {activeDesign ? (
              <span>
                SOURCE: OCP HLR · VIEW: {activeDesign.provider === 'opencascade' ? 'OPC BUILD' : 'IMPORT'}
              </span>
            ) : (
              <span>{props.bounds ? 'BOUNDS: LIVE' : 'BOUNDS: —'}</span>
            )}
          </>
        ) : null}
        {props.auxView === 'tolerance' ? (
          activeDesign ? (
            <>
              <span>SOURCE: DECLARED</span>
              <span>ANALYSIS: NONE</span>
              <span title={measuredByLabel(activeDesign)}>
                {measuredByLabel(activeDesign)}
              </span>
            </>
          ) : (
            <>
              <span>SOURCE: —</span>
              <span>VALUES: NONE</span>
            </>
          )
        ) : null}
      </footer>
    </section>
  );
}

// --- Design panels ----------------------------------------------------------
//
// Every value in these panels is either a backend measurement (the section
// loops and area, the HLR polylines) or DECLARED metadata (the tolerance
// strings). Nothing is inferred, and every loading/error state names the
// request that produced it.

const SECTION_CELL: DrawingCell = { x: 11, y: 8, width: 278, height: 128 };

function DesignSectionPanel(props: {
  section: DesignResource<SectionResponse> | null;
  plane: SectionPlane;
  onPlaneChange(plane: SectionPlane): void;
  onRetry(): void;
  baseId: string;
}): ReactElement {
  const hatchId = `${props.baseId}-design-hatch`;
  const status = props.section?.status ?? 'idle';
  const data = props.section?.data ?? null;
  const error = props.section?.error ?? null;

  return (
    <div className="aux-design-section">
      <div className="aux-section-plane" role="group" aria-label="Section plane">
        <span className="aux-section-plane-label">PLANE</span>
        {SECTION_PLANES.map((plane) => (
          <button
            key={plane}
            type="button"
            className={`aux-section-plane-button${plane === props.plane ? ' is-active' : ''}`}
            aria-pressed={plane === props.plane}
            onClick={() => props.onPlaneChange(plane)}
          >
            {plane}
          </button>
        ))}
      </div>

      {status === 'error' ? (
        <div className="aux-error" role="alert">
          <p className="aux-error-kicker">Section {props.plane} failed</p>
          <p className="aux-error-body">{error ?? 'The geometry backend did not return a section.'}</p>
          <button type="button" className="aux-retry" onClick={props.onRetry}>
            Retry section
          </button>
        </div>
      ) : status === 'loading' || status === 'idle' ? (
        <p className="aux-loading" role="status">
          {status === 'loading'
            ? `Loading section ${props.plane} from the geometry kernel…`
            : `Section ${props.plane} has not been loaded yet.`}
        </p>
      ) : !data || data.loops.length === 0 ? (
        <div className="aux-empty" role="status">
          <p className="aux-empty-kicker">Section {props.plane} — no loops</p>
          <p className="aux-empty-body">
            The kernel returned a cut with no closed loops on this plane.
          </p>
        </div>
      ) : (
        <DesignSectionSvg data={data} hatchId={hatchId} />
      )}
    </div>
  );
}

function DesignSectionSvg(props: { data: SectionResponse; hatchId: string }): ReactElement {
  const groups = props.data.loops.map((loop) => [
    { points: loop.points, closed: true, point_count: loop.point_count },
  ]);
  const fit = fitPolylineGroups(groups, SECTION_CELL, 4);
  const path = fit ? fit.paths.join(' ') : '';

  if (!path) {
    return (
      <div className="aux-empty" role="status">
        <p className="aux-empty-kicker">Section — no drawable loops</p>
        <p className="aux-empty-body">The loop points could not be fitted to the sheet.</p>
      </div>
    );
  }

  return (
    <div className="aux-design-section-svg">
      <svg
        className="aux-svg"
        viewBox="0 0 300 190"
        role="img"
        aria-label={`Section ${props.data.plane} at offset ${formatMmValue(props.data.offset_mm)} millimetres, area ${formatAreaValue(props.data.area_mm2)} square millimetres`}
      >
        <defs>
          <pattern
            id={props.hatchId}
            width="7"
            height="7"
            patternUnits="userSpaceOnUse"
            patternTransform="rotate(45)"
          >
            <line x1="0" y1="0" x2="0" y2="7" className="aux-hatch-line" />
          </pattern>
        </defs>
        <path
          d={path}
          fillRule="evenodd"
          fill={`url(#${props.hatchId})`}
          className="aux-section-area"
        />
        <path d={path} fill="none" className="aux-section-boundary" />
        <text x="11" y="164" className="aux-caption">
          SECTION {props.data.plane}
        </text>
        <text x="289" y="164" textAnchor="end" className="aux-area-value">
          {props.data.area_exact ? 'AREA' : '≈ AREA'} {formatAreaValue(props.data.area_mm2)} mm²
        </text>
        <text x="11" y="177" className="aux-caption-muted">
          OFFSET {formatMmValue(props.data.offset_mm)} mm · {props.data.outer_loop_count} OUTER ·{' '}
          {props.data.inner_loop_count} INNER LOOP
          {props.data.inner_loop_count === 1 ? '' : 'S'}
          {props.data.truncated ? ' · TRUNCATED' : ''}
        </text>
        <text x="289" y="177" textAnchor="end" className="aux-caption-muted">
          {props.data.area_exact ? 'KERNEL AREA' : 'POLYLINE AREA'}
        </text>
      </svg>
    </div>
  );
}

function DesignOrthoPanel(props: {
  drawing: DesignResource<DrawingResponse> | null;
  onRetry(): void;
}): ReactElement {
  const status = props.drawing?.status ?? 'idle';
  const data = props.drawing?.data ?? null;
  const error = props.drawing?.error ?? null;

  if (status === 'error') {
    return (
      <div className="aux-error" role="alert">
        <p className="aux-error-kicker">Ortho views failed</p>
        <p className="aux-error-body">{error ?? 'The geometry backend did not return a drawing.'}</p>
        <button type="button" className="aux-retry" onClick={props.onRetry}>
          Retry drawing
        </button>
      </div>
    );
  }
  if (status === 'loading' || status === 'idle' || !data) {
    return (
      <p className="aux-loading" role="status">
        {status === 'loading'
          ? 'Loading the HLR ortho views from the geometry kernel…'
          : 'The HLR ortho views have not been loaded yet.'}
      </p>
    );
  }

  const byName = new Map<string, DrawingView>();
  for (const view of data.views) byName.set(view.name, view);
  const truncated = data.views.some((view) => view.truncated);

  return (
    <div className="aux-design-ortho">
      <svg
        className="aux-svg"
        viewBox="0 0 300 180"
        role="img"
        aria-label="Third-angle front, top, and right HLR views of the active design"
      >
        <text x="290" y="9" textAnchor="end" className="aux-caption-muted">
          3RD ANGLE · OCP HLR
        </text>
        <DesignOrthoCell view={byName.get('top') ?? null} cell={{ x: 8, y: 14, width: 136, height: 58 }} name="top" />
        <DesignOrthoCell view={byName.get('front') ?? null} cell={{ x: 8, y: 84, width: 136, height: 82 }} name="front" />
        <DesignOrthoCell view={byName.get('right') ?? null} cell={{ x: 154, y: 84, width: 138, height: 82 }} name="right" />
        <line x1="8" y1="76" x2="292" y2="76" className="aux-sheet-rule" />
        <line x1="148" y1="84" x2="148" y2="168" className="aux-sheet-rule" />
        <text x="290" y="177" textAnchor="end" className="aux-caption-muted">
          {truncated ? 'POLYLINE BUDGET TRUNCATED' : 'TOP ABOVE FRONT · RIGHT OF FRONT'}
        </text>
      </svg>
    </div>
  );
}

function DesignOrthoCell(props: {
  view: DrawingView | null;
  cell: DrawingCell;
  name: 'front' | 'top' | 'right';
}): ReactElement {
  const fit = props.view
    ? fitPolylineGroups([props.view.visible, props.view.hidden], props.cell, 4)
    : null;
  const centreX = props.cell.x + props.cell.width / 2;
  const centreY = props.cell.y + props.cell.height / 2;

  return (
    <g className={`aux-design-ortho-view aux-design-ortho-${props.name}`} data-view={props.name}>
      <line
        x1={props.cell.x + 2}
        y1={centreY}
        x2={props.cell.x + props.cell.width - 2}
        y2={centreY}
        className="aux-centreline aux-centreline-faint"
      />
      <line
        x1={centreX}
        y1={props.cell.y + 2}
        x2={centreX}
        y2={props.cell.y + props.cell.height - 2}
        className="aux-centreline aux-centreline-faint"
      />
      {props.view && fit ? (
        <>
          {fit.paths[0] ? (
            <path className="aux-ortho-visible" data-line="visible" d={fit.paths[0]} />
          ) : null}
          {fit.paths[1] ? (
            <path className="aux-ortho-hidden" data-line="hidden" d={fit.paths[1]} />
          ) : null}
        </>
      ) : (
        <text x={centreX} y={centreY} textAnchor="middle" className="aux-view-dims">
          {props.view ? 'NO POLYLINES' : 'NOT RETURNED'}
        </text>
      )}
      <text
        x={centreX}
        y={props.cell.y + props.cell.height - 2}
        textAnchor="middle"
        className="aux-view-label"
      >
        {props.name.toUpperCase()}
      </text>
    </g>
  );
}

function DesignTolerancePanel(props: { design: DesignResult }): ReactElement {
  return (
    <div className="aux-design-tolerance">
      <p className="aux-declared-note">declared — not analysed</p>
      <p className="aux-empty-note" data-provider={props.design.provider}>
        {measuredByLabel(props.design)}
      </p>
      {props.design.tolerances.length > 0 ? (
        <ul className="aux-tolerance-list">
          {props.design.tolerances.map((tolerance, index) => (
            <li key={`${tolerance}-${index}`}>{tolerance}</li>
          ))}
        </ul>
      ) : (
        <p className="aux-empty-body">No tolerances were declared for this design.</p>
      )}
      {props.design.material ? (
        <p className="aux-empty-note">Material: {props.design.material} (declared)</p>
      ) : null}
      <p className="aux-empty-note">
        The backend reports declared limits only. Kala Studio computes no fits, GD&amp;T, or
        tolerance stacks.
      </p>
    </div>
  );
}

function formatAreaValue(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return value.toLocaleString('en-US', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

function SectionPanel(props: { section: SectionView; baseId: string }): ReactElement {
  const { section, baseId } = props;
  const hatchId = `${baseId}-hatch`;
  const arrowStartId = `${baseId}-arrow-start`;
  const arrowEndId = `${baseId}-arrow-end`;

  if (section.kind === 'unavailable') {
    return (
      <div className="aux-empty" role="status">
        <p className="aux-empty-kicker">Section A–A unavailable</p>
        <p className="aux-empty-body">
          {section.reason ?? 'This primitive has no defined section in the drafting kernel.'}
        </p>
        <p className="aux-empty-note">The main sheet still shows the current solid.</p>
      </div>
    );
  }

  const drawingWidth = 300;
  const drawingHeight = 128;
  const fit = fitToBox(
    drawingWidth,
    drawingHeight,
    section.widthMm,
    section.heightMm,
    30,
  );
  const offsetY = 8;
  const x = fit.x;
  const y = offsetY + fit.y;
  const width = fit.width;
  const height = fit.height;
  const centreX = x + width / 2;
  const centreY = y + height / 2;
  const dimY = y + height + 14;
  const dimX = x - 14;

  return (
    <div className="aux-section">
      <svg
        className="aux-svg"
        viewBox={`0 0 ${SECTION_SVG_WIDTH} ${SECTION_SVG_HEIGHT}`}
        role="img"
        aria-label={`Section A–A, ${section.kind === 'circle' ? `diameter ${formatMmValue(section.widthMm)} millimetres` : `${formatMmValue(section.widthMm)} by ${formatMmValue(section.heightMm)} millimetres`}`}
      >
        <defs>
          <pattern
            id={hatchId}
            width="7"
            height="7"
            patternUnits="userSpaceOnUse"
            patternTransform="rotate(45)"
          >
            <line x1="0" y1="0" x2="0" y2="7" className="aux-hatch-line" />
          </pattern>
          <marker
            id={arrowStartId}
            viewBox="0 0 8 8"
            refX="1"
            refY="4"
            markerWidth="6"
            markerHeight="6"
            orient="auto"
          >
            <path d="M8 0 1 4l7 4z" className="aux-arrow-fill" />
          </marker>
          <marker
            id={arrowEndId}
            viewBox="0 0 8 8"
            refX="7"
            refY="4"
            markerWidth="6"
            markerHeight="6"
            orient="auto"
          >
            <path d="M0 0l7 4-7 4z" className="aux-arrow-fill" />
          </marker>
        </defs>

        <g opacity="0.45" className="aux-grid-lines">
          <line x1="18" y1="32" x2="282" y2="32" />
          <line x1="18" y1="64" x2="282" y2="64" />
          <line x1="18" y1="96" x2="282" y2="96" />
          <line x1="150" y1="8" x2="150" y2="124" />
        </g>

        <line
          x1={x - 16}
          y1={centreY}
          x2={x + width + 16}
          y2={centreY}
          className="aux-centreline"
        />
        <line
          x1={centreX}
          y1={y - 12}
          x2={centreX}
          y2={y + height + 12}
          className="aux-centreline"
        />

        {section.kind === 'circle' ? (
          <circle
            cx={centreX}
            cy={centreY}
            r={Math.min(width, height) / 2}
            fill={`url(#${hatchId})`}
            className="aux-profile"
          />
        ) : (
          <rect x={x} y={y} width={width} height={height} fill={`url(#${hatchId})`} className="aux-profile" />
        )}

        <line
          x1={x}
          y1={dimY}
          x2={x + width}
          y2={dimY}
          className="aux-dim-line"
          markerStart={`url(#${arrowStartId})`}
          markerEnd={`url(#${arrowEndId})`}
        />
        <text x={centreX} y={dimY - 4} textAnchor="middle" className="aux-dim-text">
          {section.kind === 'circle'
            ? `⌀ ${formatMmValue(section.widthMm)}`
            : `W ${formatMmValue(section.widthMm)}`}
        </text>

        <line
          x1={dimX}
          y1={y}
          x2={dimX}
          y2={y + height}
          className="aux-dim-line"
          markerStart={`url(#${arrowStartId})`}
          markerEnd={`url(#${arrowEndId})`}
        />
        <text
          x={dimX - 4}
          y={centreY}
          textAnchor="middle"
          transform={`rotate(-90 ${dimX - 4} ${centreY})`}
          className="aux-dim-text"
        >
          {section.kind === 'circle'
            ? `⌀ ${formatMmValue(section.heightMm)}`
            : `H ${formatMmValue(section.heightMm)}`}
        </text>

        <text x="10" y="154" className="aux-caption">
          SECTION A–A
        </text>
        <text x="290" y="154" textAnchor="end" className="aux-caption-muted">
          {section.planeLabel}
        </text>
        <text x="10" y="165" className="aux-caption-muted">
          ANSI 45° HATCH · REAL PARAMETERS
        </text>
      </svg>
    </div>
  );
}

function OrthoPanel(props: {
  views: OrthoView[] | null;
  bounds: BBox | null;
  baseId: string;
}): ReactElement {
  if (!props.views) {
    return (
      <div className="aux-empty" role="status">
        <p className="aux-empty-kicker">Ortho 3-View unavailable</p>
        <p className="aux-empty-body">
          {props.bounds
            ? 'The current mesh bounds are degenerate, so no projection is drawn.'
            : 'No mesh bounds yet. Run a preview to project the three views.'}
        </p>
        <p className="aux-empty-note">Nothing is inferred before a real preview exists.</p>
      </div>
    );
  }

  const byId = new Map(props.views.map((view) => [view.id, view]));
  const front = byId.get('front')!;
  const top = byId.get('top')!;
  const right = byId.get('right')!;

  return (
    <div className="aux-ortho">
      <svg
        className="aux-svg"
        viewBox="0 0 300 170"
        role="img"
        aria-label="Third-angle orthographic front, top, and right views from the live mesh bounds"
      >
        <text x="290" y="9" textAnchor="end" className="aux-caption-muted">
          3RD ANGLE
        </text>
        <OrthoCell
          view={top}
          cell={{ x: 8, y: 14, width: 136, height: 56 }}
          label={`${top.label} · ${top.direction}`}
        />
        <OrthoCell
          view={front}
          cell={{ x: 8, y: 84, width: 136, height: 78 }}
          label={`${front.label} · ${front.direction}`}
        />
        <OrthoCell
          view={right}
          cell={{ x: 154, y: 84, width: 138, height: 78 }}
          label={`${right.label} · ${right.direction}`}
        />
        <line x1="8" y1="74" x2="292" y2="74" className="aux-sheet-rule" />
        <line x1="148" y1="84" x2="148" y2="162" className="aux-sheet-rule" />
        <text x="290" y="169" textAnchor="end" className="aux-caption-muted">
          THIRD-ANGLE · TOP ABOVE FRONT · RIGHT OF FRONT
        </text>
      </svg>
    </div>
  );
}

function OrthoCell(props: {
  view: OrthoView;
  cell: { x: number; y: number; width: number; height: number };
  label: string;
}): ReactElement {
  const labelHeight = 16;
  const fit = fitToBox(
    props.cell.width,
    props.cell.height - labelHeight,
    props.view.widthMm,
    props.view.heightMm,
    3,
  );
  const x = props.cell.x + fit.x;
  const y = props.cell.y + fit.y;
  const width = fit.width;
  const height = fit.height;
  const centreX = x + width / 2;
  const centreY = y + height / 2;

  return (
    <g className={`aux-ortho-view aux-ortho-${props.view.id}`}>
      <line
        x1={props.cell.x + 2}
        y1={centreY}
        x2={props.cell.x + props.cell.width - 2}
        y2={centreY}
        className="aux-centreline aux-centreline-faint"
      />
      <line
        x1={centreX}
        y1={y - 2}
        x2={centreX}
        y2={y + height + 2}
        className="aux-centreline aux-centreline-faint"
      />
      {props.view.kind === 'circle' ? (
        <circle
          cx={centreX}
          cy={centreY}
          r={Math.min(width, height) / 2}
          className="aux-ortho-profile"
        />
      ) : (
        <rect x={x} y={y} width={width} height={height} className="aux-ortho-profile" />
      )}
      <text x={centreX} y={props.cell.y + props.cell.height - 8} textAnchor="middle" className="aux-view-label">
        {props.label}
      </text>
      <text x={centreX} y={props.cell.y + props.cell.height - 1} textAnchor="middle" className="aux-view-dims">
        {props.view.kind === 'circle'
          ? `⌀ ${formatMmValue(Math.min(props.view.widthMm, props.view.heightMm))} mm`
          : `${formatMmValue(props.view.widthMm)} × ${formatMmValue(props.view.heightMm)} mm`}
      </text>
    </g>
  );
}

function TolerancePanel(): ReactElement {
  return (
    <div className="aux-empty aux-tolerance" role="status">
      <p className="aux-empty-kicker">Tolerance — no model</p>
      <p className="aux-empty-body">
        The backend exposes no tolerance stack, GD&amp;T, or fit class for the current
        primitive. This panel will stay empty rather than invent limits.
      </p>
      <dl className="aux-empty-facts">
        <div>
          <dt>Tolerance values</dt>
          <dd>—</dd>
        </div>
        <div>
          <dt>Fit class</dt>
          <dd>—</dd>
        </div>
        <div>
          <dt>Source</dt>
          <dd>Not provided</dd>
        </div>
      </dl>
    </div>
  );
}
