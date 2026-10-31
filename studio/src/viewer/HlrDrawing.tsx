// The 2D pencil HLR drawing.
//
// When a design is active and the sheet is in 2D Pencil mode this SVG *is*
// the drawing. It renders the backend's hidden-line-removal polylines on the
// vellum sheet: the isometric view large, and the front/top/right views in
// third-angle arrangement as small views. Visible edges are 2B ink, hidden
// edges are dashed 2H construction, and the redline callouts carry the
// kernel-measured envelope — never a number computed here.
//
// The drawing supports wheel zoom, drag pan, and a reset. It is deliberately
// static: there is no WebGL canvas mounted while it is shown, so the studio
// keeps exactly one GPU context at most (the clay viewport owns it in 3D
// mode).

import { useEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent, ReactElement, WheelEvent as ReactWheelEvent } from 'react';
import type {
  DesignResult,
  DesignViewName,
  DrawingPolyline,
  DrawingResponse,
  DrawingView,
  SectionPlane,
  SectionResponse,
} from '../types/studio';
import { formatMmValue } from './draftingGeometry';
import {
  IDENTITY_TRANSFORM,
  VIEW_MEASURED_AXES,
  designViewCaption,
  drawingTransformAttribute,
  fitPoint,
  fitPolylineGroups,
  measuredAxisValue,
  measuredEnvelope,
  panDrawing,
  polylinePath,
  polylinesExtent,
  sectionLoopsInView,
  zoomDrawingAt,
  type DrawingCell,
  type DrawingTransform,
} from './hlr';
import { designViewLabel } from './designSource';
import { DEFAULT_DESIGN_SECTION_CUT, sectionCutNote, type SectionCut } from '../state/sectionCut';
import '../styles/drafting.css';

export interface HlrDrawingProps {
  /** The measured design whose drawing is on the sheet. */
  design: DesignResult;
  /** The backend's drawing response, or null before it arrives. */
  drawing: DrawingResponse | null;
  /** The load lifecycle of `drawing`. */
  status: 'idle' | 'loading' | 'ready' | 'error';
  /** The load error, when `status` is `'error'`. */
  error: string | null;
  /** Show the redline measured-envelope callouts. */
  showDimensions: boolean;
  /** Increments when the operator asks to re-frame the drawing. */
  viewResetToken: number;
  /** Re-request the drawing after a failure. */
  onRetry?(): void;
  /** The measured section at the shared cut; null until it is loaded. */
  section?: SectionResponse | null;
  /** The shared cutting plane the sheet and the AuxView both name. */
  cut?: SectionCut;
}

const SVG_WIDTH = 940;
const SVG_HEIGHT = 1000;

// The sheet layout: iso large on the left, third-angle small views on the
// right. In third angle the top view sits above the front view and the right
// view sits to the right of the front view, so the top-right cell is empty
// and carries the projection note.
const ISO_CELL: DrawingCell = { x: 76, y: 96, width: 520, height: 470 };
const TOP_CELL: DrawingCell = { x: 620, y: 90, width: 140, height: 200 };
const FRONT_CELL: DrawingCell = { x: 620, y: 330, width: 140, height: 200 };
const RIGHT_CELL: DrawingCell = { x: 790, y: 330, width: 140, height: 200 };

export function HlrDrawing(props: HlrDrawingProps): ReactElement {
  const dragRef = useRef<{
    startX: number;
    startY: number;
    panX: number;
    panY: number;
    ratioX: number;
    ratioY: number;
  } | null>(null);
  const [transform, setTransform] = useState<DrawingTransform>(IDENTITY_TRANSFORM);
  const [dragging, setDragging] = useState(false);

  // The drafting tool palette's Reset View and a new design both reset the
  // drawing. The reset is local: it never touches the 3D camera.
  useEffect(() => {
    setTransform(IDENTITY_TRANSFORM);
  }, [props.viewResetToken, props.design.design_id]);

  const viewByName = new Map<DesignViewName, DrawingView>();
  for (const view of props.drawing?.views ?? []) viewByName.set(view.name, view);
  const isoView = viewByName.get('iso') ?? null;
  const topView = viewByName.get('top') ?? null;
  const frontView = viewByName.get('front') ?? null;
  const rightView = viewByName.get('right') ?? null;
  const envelope = measuredEnvelope(props.design.measured);
  const truncated = (props.drawing?.views ?? []).some((view) => view.truncated);
  const cut = props.cut ?? DEFAULT_DESIGN_SECTION_CUT;
  // The section loops are measured on the shared plane. The coplanar view
  // shows them in true shape; the iso view shows their projection. A plane
  // with no matching section draws no redline.
  const section = props.section ?? null;
  const sectionForView = (name: DesignViewName): DrawingPolyline[] =>
    section && section.plane === cut.plane ? sectionLoopsInView(section, name) : [];

  function resetTransform(): void {
    setTransform(IDENTITY_TRANSFORM);
  }

  function zoomBy(factor: number): void {
    setTransform((current) => zoomDrawingAt(current, { x: SVG_WIDTH / 2, y: SVG_HEIGHT / 2 }, factor));
  }

  function onWheel(event: ReactWheelEvent<SVGSVGElement>): void {
    if (!props.drawing) return;
    if (event.deltaY === 0) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const point =
      rect.width > 0 && rect.height > 0
        ? {
            x: ((event.clientX - rect.left) / rect.width) * SVG_WIDTH,
            y: ((event.clientY - rect.top) / rect.height) * SVG_HEIGHT,
          }
        : { x: SVG_WIDTH / 2, y: SVG_HEIGHT / 2 };
    const factor = event.deltaY < 0 ? 1.2 : 1 / 1.2;
    setTransform((current) => zoomDrawingAt(current, point, factor));
  }

  function onPointerDown(event: ReactPointerEvent<SVGSVGElement>): void {
    if (!props.drawing) return;
    if (event.button !== 0) return;
    const rect = event.currentTarget.getBoundingClientRect();
    dragRef.current = {
      startX: event.clientX,
      startY: event.clientY,
      panX: transform.panX,
      panY: transform.panY,
      ratioX: rect.width > 0 ? SVG_WIDTH / rect.width : 1,
      ratioY: rect.height > 0 ? SVG_HEIGHT / rect.height : 1,
    };
    setDragging(true);
    // happy-dom and older engines may not implement pointer capture.
    if (typeof event.currentTarget.setPointerCapture === 'function') {
      try {
        event.currentTarget.setPointerCapture(event.pointerId);
      } catch {
        // The pointer is not capturable in this environment; drag still works.
      }
    }
  }

  function onPointerMove(event: ReactPointerEvent<SVGSVGElement>): void {
    const drag = dragRef.current;
    if (!drag) return;
    setTransform((current) =>
      panDrawing(
        { ...current, panX: drag.panX, panY: drag.panY },
        (event.clientX - drag.startX) * drag.ratioX,
        (event.clientY - drag.startY) * drag.ratioY,
      ),
    );
  }

  function endDrag(event: ReactPointerEvent<SVGSVGElement>): void {
    if (!dragRef.current) return;
    dragRef.current = null;
    setDragging(false);
    if (typeof event.currentTarget.releasePointerCapture === 'function') {
      try {
        event.currentTarget.releasePointerCapture(event.pointerId);
      } catch {
        // The pointer may already be released by the browser.
      }
    }
  }

  return (
    <section className="hlr-drawing" aria-label="2D pencil HLR drawing" data-design-id={props.design.design_id}>
      <div className="hlr-heading">
        <span className="hlr-heading-title">HLR · {props.design.label}</span>
        <span className="hlr-heading-meta">
          {props.design.design_id.slice(0, 8)}
          {props.drawing ? ` · ${props.drawing.point_count} PTS` : ''}
          {' · 2B VISIBLE / 2H HIDDEN'}
          {truncated ? ' · TRUNCATED' : ''}
        </span>
        <p className="hlr-heading-view" data-view-provider={props.design.provider}>
          {designViewLabel(props.design.provider)}
        </p>
        <span
          className="hlr-heading-cut"
          data-cut-plane={cut.plane}
          data-cut-offset={cut.offsetMm === null ? 'default' : String(cut.offsetMm)}
        >
          {sectionCutNote(cut, props.design.view.bounds)}
        </span>
      </div>

      {props.drawing ? (
        <svg
          className={`hlr-svg${dragging ? ' is-panning' : ''}`}
          viewBox={`0 0 ${SVG_WIDTH} ${SVG_HEIGHT}`}
          role="img"
          aria-label={`Hidden-line drawing of ${props.design.label}: isometric plus third-angle front, top, and right views`}
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
        >
          <g transform={drawingTransformAttribute(transform)}>
            <HlrView
              view={isoView}
              cell={ISO_CELL}
              name="iso"
              padding={18}
              truncated={isoView?.truncated === true}
              sectionLoops={sectionForView('iso')}
              sectionPlane={cut.plane}
            />
            <HlrView
              view={topView}
              cell={TOP_CELL}
              name="top"
              padding={10}
              truncated={topView?.truncated === true}
              sectionLoops={sectionForView('top')}
              sectionPlane={cut.plane}
            />
            <HlrView
              view={frontView}
              cell={FRONT_CELL}
              name="front"
              padding={10}
              truncated={frontView?.truncated === true}
              sectionLoops={sectionForView('front')}
              sectionPlane={cut.plane}
            />
            <HlrView
              view={rightView}
              cell={RIGHT_CELL}
              name="right"
              padding={10}
              truncated={rightView?.truncated === true}
              sectionLoops={sectionForView('right')}
              sectionPlane={cut.plane}
            />

            {/* The third-angle top-right cell is intentionally empty: the
                arrangement is top above front, right of front. */}
            <g className="hlr-projection-note" aria-hidden="true">
              <text x={790} y={190} className="hlr-projection-title">
                THIRD ANGLE
              </text>
              <text x={790} y={208} className="hlr-projection-line">
                TOP ABOVE FRONT
              </text>
              <text x={790} y={224} className="hlr-projection-line">
                RIGHT OF FRONT
              </text>
              <text x={790} y={246} className="hlr-projection-line">
                HLR · OCP
              </text>
            </g>

            {envelope && props.showDimensions ? (
              <MeasuredCallouts envelope={envelope} />
            ) : null}

            {envelope && props.showDimensions ? (
              <text x={ISO_CELL.x} y={ISO_CELL.y + ISO_CELL.height + 44} className="hlr-envelope-note">
                MEASURED ENVELOPE · X {formatMmValue(envelope.x)} · Y {formatMmValue(envelope.y)} · Z{' '}
                {formatMmValue(envelope.z)} mm
              </text>
            ) : null}

            {/* The sheet footnote lives at the bottom right, clear of the
                title block and just above the view controls. */}
            <text
              x={SVG_WIDTH - 24}
              y={SVG_HEIGHT - 60}
              textAnchor="end"
              className="hlr-legend"
            >
              HLR VECTOR · MEASURED ENVELOPE IN MM · NO SCALE CLAIM
            </text>
          </g>
        </svg>
      ) : (
        <div className="hlr-empty" role={props.status === 'error' ? 'alert' : 'status'}>
          <p className="hlr-empty-kicker">
            {props.status === 'error' ? 'HLR drawing failed' : 'HLR drawing'}
          </p>
          <p className="hlr-empty-body">
            {props.status === 'error'
              ? props.error ?? 'The geometry backend did not return a drawing.'
              : props.status === 'loading'
                ? 'Loading the hidden-line drawing from the geometry kernel…'
                : 'The hidden-line drawing has not been loaded yet.'}
          </p>
          {props.status === 'error' && props.onRetry ? (
            <button type="button" className="hlr-retry" onClick={props.onRetry}>
              Retry drawing
            </button>
          ) : null}
        </div>
      )}

      {props.drawing ? (
        <div className="hlr-controls" role="group" aria-label="Drawing view controls">
          <button type="button" className="hlr-control" aria-label="Zoom out" onClick={() => zoomBy(1 / 1.25)}>
            −
          </button>
          <span className="hlr-zoom-value" aria-live="polite">
            {Math.round(transform.zoom * 100)}%
          </span>
          <button type="button" className="hlr-control" aria-label="Zoom in" onClick={() => zoomBy(1.25)}>
            +
          </button>
          <button type="button" className="hlr-control hlr-control-reset" aria-label="Reset drawing view" onClick={resetTransform}>
            RESET
          </button>
        </div>
      ) : null}
    </section>
  );
}

function HlrView(props: {
  view: DrawingView | null;
  cell: DrawingCell;
  name: DesignViewName;
  padding: number;
  truncated: boolean;
  sectionLoops: DrawingPolyline[];
  sectionPlane: SectionPlane;
}): ReactElement {
  const { view, cell, name } = props;
  const sectionLoops = props.sectionLoops;
  // Fit the visible, hidden, and section groups with one transform: the
  // redline trace cannot drift away from the linework it belongs to.
  const groups: Array<ReadonlyArray<DrawingPolyline>> = [];
  if (view) groups.push(view.visible, view.hidden);
  if (sectionLoops.length > 0) groups.push(sectionLoops);
  const fit = view && groups.length > 0 ? fitPolylineGroups(groups, cell, props.padding) : null;
  const visiblePath = view && fit ? fit.paths[0] ?? '' : '';
  const hiddenPath = view && fit ? fit.paths[1] ?? '' : '';
  const sectionPath = fit
    ? sectionLoops
        .map((polyline) => polylinePath(polyline, (u, v) => fitPoint(fit, u, v)))
        .filter((path) => path.length > 0)
        .join(' ')
    : '';
  const captionX = cell.x + cell.width / 2;
  const captionY = name === 'top' ? cell.y - 10 : cell.y + cell.height + 34;
  const sectionExtent = sectionLoops.length > 0 ? polylinesExtent([sectionLoops]) : null;
  const sectionMidV = sectionExtent ? (sectionExtent.minV + sectionExtent.maxV) / 2 : null;
  const sectionMidY =
    sectionExtent && sectionMidV !== null ? fitPoint(fit!, sectionExtent.minU, sectionMidV)[1] : null;
  const sectionLeft =
    sectionExtent && sectionMidV !== null && fit
      ? fitPoint(fit, sectionExtent.minU, sectionMidV)[0]
      : null;
  const sectionRight =
    sectionExtent && sectionMidV !== null && fit
      ? fitPoint(fit, sectionExtent.maxU, sectionMidV)[0]
      : null;

  return (
    <g className={`hlr-view hlr-view-${name}`} data-view={name}>
      <rect
        className="hlr-view-frame"
        x={cell.x}
        y={cell.y}
        width={cell.width}
        height={cell.height}
      />
      {!view ? (
        <text x={captionX} y={cell.y + cell.height / 2} textAnchor="middle" className="hlr-view-missing">
          {name.toUpperCase()} VIEW NOT RETURNED
        </text>
      ) : null}
      {view && !fit ? (
        <text x={captionX} y={cell.y + cell.height / 2} textAnchor="middle" className="hlr-view-missing">
          NO POLYLINES
        </text>
      ) : null}
      {fit ? (
        <>
          {visiblePath ? (
            <path className="hlr-visible-line" data-line="visible" d={visiblePath} />
          ) : null}
          {hiddenPath ? (
            <path className="hlr-hidden-line" data-line="hidden" d={hiddenPath} />
          ) : null}
          {sectionPath ? (
            <g
              className="hlr-section-trace"
              data-line="section"
              data-section-plane={props.sectionPlane}
            >
              <path className="hlr-section-line" d={sectionPath} />
              {sectionLeft !== null && sectionRight !== null && sectionMidY !== null ? (
                <>
                  <text
                    x={Math.max(cell.x + 7, sectionLeft - 11)}
                    y={sectionMidY + 4}
                    textAnchor="middle"
                    className="hlr-section-flag"
                  >
                    A
                  </text>
                  <text
                    x={Math.min(cell.x + cell.width - 7, sectionRight + 11)}
                    y={sectionMidY + 4}
                    textAnchor="middle"
                    className="hlr-section-flag"
                  >
                    A
                  </text>
                </>
              ) : null}
            </g>
          ) : null}
        </>
      ) : null}
      <text x={captionX} y={captionY} textAnchor="middle" className="hlr-view-caption">
        {designViewCaption(name)}
      </text>
      {props.truncated ? (
        <text x={captionX} y={captionY + 12} textAnchor="middle" className="hlr-view-truncated">
          POLYLINE BUDGET TRUNCATED
        </text>
      ) : null}
    </g>
  );
}

/**
 * The redline callouts. Each small view carries the two measured envelope
 * dimensions it actually spans (A2's view bases), and the iso view carries
 * the full envelope note. The numbers come from `measured.bounds`.
 */
function MeasuredCallouts(props: {
  envelope: { x: number; y: number; z: number };
}): ReactElement {
  const axes = VIEW_MEASURED_AXES;
  return (
    <g className="hlr-callouts">
      <HorizontalCallout
        cell={TOP_CELL}
        y={TOP_CELL.y + TOP_CELL.height + 14}
        label={`${axes.top.horizontal} ${formatMmValue(measuredAxisValue(props.envelope, axes.top.horizontal))}`}
      />
      <VerticalCallout
        cell={TOP_CELL}
        x={TOP_CELL.x - 14}
        label={`${axes.top.vertical} ${formatMmValue(measuredAxisValue(props.envelope, axes.top.vertical))}`}
      />
      <HorizontalCallout
        cell={FRONT_CELL}
        y={FRONT_CELL.y + FRONT_CELL.height + 14}
        label={`${axes.front.horizontal} ${formatMmValue(measuredAxisValue(props.envelope, axes.front.horizontal))}`}
      />
      <VerticalCallout
        cell={FRONT_CELL}
        x={FRONT_CELL.x - 14}
        label={`${axes.front.vertical} ${formatMmValue(measuredAxisValue(props.envelope, axes.front.vertical))}`}
      />
      <HorizontalCallout
        cell={RIGHT_CELL}
        y={RIGHT_CELL.y + RIGHT_CELL.height + 14}
        label={`${axes.right.horizontal} ${formatMmValue(measuredAxisValue(props.envelope, axes.right.horizontal))}`}
      />
      <VerticalCallout
        cell={RIGHT_CELL}
        x={RIGHT_CELL.x - 14}
        label={`${axes.right.vertical} ${formatMmValue(measuredAxisValue(props.envelope, axes.right.vertical))}`}
      />
    </g>
  );
}

function HorizontalCallout(props: { cell: DrawingCell; y: number; label: string }): ReactElement {
  const x1 = props.cell.x + 4;
  const x2 = props.cell.x + props.cell.width - 4;
  const y = props.y;
  return (
    <g className="hlr-dim" data-dim={props.label}>
      <line x1={x1} y1={y - 5} x2={x1} y2={y + 5} className="hlr-dim-tick" />
      <line x1={x2} y1={y - 5} x2={x2} y2={y + 5} className="hlr-dim-tick" />
      <line x1={x1} y1={y} x2={x2} y2={y} className="hlr-dim-line" />
      <text x={(x1 + x2) / 2} y={y - 5} textAnchor="middle" className="hlr-dim-text">
        {props.label}
      </text>
    </g>
  );
}

function VerticalCallout(props: { cell: DrawingCell; x: number; label: string }): ReactElement {
  const y1 = props.cell.y + 4;
  const y2 = props.cell.y + props.cell.height - 4;
  const x = props.x;
  return (
    <g className="hlr-dim" data-dim={props.label}>
      <line x1={x - 5} y1={y1} x2={x + 5} y2={y1} className="hlr-dim-tick" />
      <line x1={x - 5} y1={y2} x2={x + 5} y2={y2} className="hlr-dim-tick" />
      <line x1={x} y1={y1} x2={x} y2={y2} className="hlr-dim-line" />
      <text
        x={x - 4}
        y={(y1 + y2) / 2}
        textAnchor="middle"
        transform={`rotate(-90 ${x - 4} ${(y1 + y2) / 2})`}
        className="hlr-dim-text"
      >
        {props.label}
      </text>
    </g>
  );
}
