// Bottom drafting ruler bar (center column only, 48px).
//
// Everything here is wired to real store state: render mode, grid/axes/dims,
// the Section A-A flag, camera reset, the nominal geometry readout, and the
// backend's actual FreeCAD integration row. There is no decorative telemetry.
//
// The bar is one row at 1440: the view toggles are icon buttons with
// aria-labels and title tooltips, the readout is one mono line in the fixed
// order ENV · VOL · AREA · SCALE · source, and the surface-material control
// is delegated to Options card 01 below 1600px. At 1280 a long value may
// ellipsize, with the full value in its title tooltip.

import type { ReactElement } from 'react';
import {
  dispatch,
  useUiState,
  selectActiveDesign,
  selectPreview,
  selectRenderMode,
  selectServer,
  selectShowSection,
  selectViewport,
} from '../../state/store';
import type { OptionsCard } from '../../types/studio';
import type { DesignResult } from '../../types/studio';
import { measurerLabel, providerShortLabel } from '../chat/designCard';
import {
  boundsDimensions,
  computeGeometryEstimate,
} from '../../viewer/localPrimitives';
import { DRAFTING_SCALE } from '../../viewer/draftingGeometry';
import { SurfaceMaterialControl } from '../panels/SurfaceMaterialControl';
import '../../styles/bottombar.css';

export interface BottomBarProps {
  /** Open the Options drawer on a specific drafted card. */
  onOpenOptions(card: OptionsCard): void;
}

type BarIconName = 'grid' | 'axes' | 'dims' | 'section' | 'reset';

const BAR_ICONS: Record<BarIconName, ReactElement> = {
  grid: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M2.5 2.5h11v11h-11zM2.5 6h11M2.5 10h11M6 2.5v11M10 2.5v11" />
    </svg>
  ),
  axes: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M8 2v12M2 8h12M8 2 6.5 3.7M8 2l1.5 1.7M8 14l-1.5-1.7M8 14l1.5-1.7M2 8l1.7-1.5M2 8l1.7 1.5M14 8l-1.7-1.5M14 8l-1.7 1.5" />
      <circle cx="8" cy="8" r="1.4" />
    </svg>
  ),
  dims: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M3 4v8M13 4v8M3 8h10M3 8l2-1.6M3 8l2 1.6M13 8l-2-1.6M13 8l-2 1.6" />
    </svg>
  ),
  section: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M2 8h12" strokeDasharray="3 2" />
      <path d="M3 3v3l-1-1.5L3 6M13 13v-3l1 1.5L13 10" />
      <path d="M2.6 3.5 3 6M13.4 12.5 13 10" />
    </svg>
  ),
  reset: (
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M4 5a5 5 0 1 1-1 5.5M4 5V2M4 5h3" />
    </svg>
  ),
};

export function BottomBar(props: BottomBarProps): ReactElement {
  const renderMode = useUiState(selectRenderMode);
  const viewport = useUiState(selectViewport);
  const showSection = useUiState(selectShowSection);
  const preview = useUiState(selectPreview);
  const server = useUiState(selectServer);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const activeDesign = useUiState(selectActiveDesign);

  const estimate = computeGeometryEstimate(primitive, parameters);
  const hasGeometry = preview.server !== null || preview.local !== null;
  const serverBounds = preview.server?.preview.bounds ?? null;
  // A provider-built design replaces the analytic readout: the bar then
  // quotes the numbers the selected CAD provider measured and names it.
  // Without a design the bar keeps its primitive behaviour.
  const designReadout = activeDesign ? describeDesignReadout(activeDesign) : null;
  const envelope = designReadout?.envelope
    ?? (serverBounds ? describeBounds(serverBounds) : describeEnvelope(estimate.envelopeMm));
  const volume = designReadout?.volume
    ?? (hasGeometry ? `${estimate.volumeCm3.toFixed(1)} cm³` : '—');
  const area = designReadout?.area
    ?? (hasGeometry ? `${estimate.surfaceAreaCm2.toFixed(1)} cm²` : '—');
  const source = designReadout?.source
    ?? (preview.inFlight
      ? 'verifying…'
      : preview.lastError
        ? 'server refused'
        : preview.server
          ? 'server'
          : preview.local
            ? 'local est.'
            : '—');

  const freecad = server.integrations.find(
    (entry) => entry.id.toLowerCase() === 'freecad' || entry.name.toLowerCase().includes('freecad'),
  );
  const freecadStatus = freecad?.status ?? 'none';

  return (
    <footer className="bottom-bar" aria-label="Drafting controls and readout">
      <div className="bottom-bar-cluster bottom-bar-left" role="toolbar" aria-label="View controls">
        <div className="bottom-bar-mode" role="group" aria-label="Render mode">
          {(['pencil2d', 'clay3d'] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              className={`bottom-bar-button${renderMode === mode ? ' is-active' : ''}`}
              aria-pressed={renderMode === mode}
              aria-label={mode === 'pencil2d' ? '2D Pencil' : '3D Clay'}
              title={mode === 'pencil2d' ? '2D Pencil render mode' : '3D Clay render mode'}
              data-render-mode={mode}
              onClick={() => dispatch({ type: 'renderMode/set', mode })}
            >
              {mode === 'pencil2d' ? '2D' : '3D'}
            </button>
          ))}
        </div>
        <IconToggle icon="grid" label="Grid" active={viewport.showGrid} onClick={() => dispatch({ type: 'viewport/set', patch: { showGrid: !viewport.showGrid } })} />
        <IconToggle icon="axes" label="Axes" active={viewport.showAxes} onClick={() => dispatch({ type: 'viewport/set', patch: { showAxes: !viewport.showAxes } })} />
        <IconToggle icon="dims" label="Dims" active={viewport.showDimensions} onClick={() => dispatch({ type: 'viewport/set', patch: { showDimensions: !viewport.showDimensions } })} />
        <IconToggle icon="section" label="Section A-A" active={showSection} onClick={() => dispatch({ type: 'section/toggle' })} />
        <SurfaceMaterialControl variant="compact" />
        <button
          type="button"
          className="bottom-bar-icon-button"
          aria-label="Reset View"
          title="Re-frame the drafting sheet camera"
          onClick={() => dispatch({ type: 'view/reset' })}
        >
          {BAR_ICONS.reset}
        </button>
      </div>

      <div className="bottom-bar-readout" aria-live="polite">
        <Readout
          label="ENV"
          value={designReadout || hasGeometry ? envelope : '—'}
          title={designReadout
            ? designReadout.envelopeTitle
            : 'Bounding envelope'}
        />
        <span className="bottom-bar-separator" aria-hidden="true">·</span>
        <Readout
          label="VOL"
          value={volume}
          title={designReadout
            ? designReadout.volumeTitle
            : 'Closed-form nominal volume'}
        />
        <span className="bottom-bar-separator" aria-hidden="true">·</span>
        <Readout
          label="AREA"
          value={area}
          title={designReadout
            ? designReadout.areaTitle
            : 'Closed-form nominal surface area'}
        />
        <span className="bottom-bar-separator" aria-hidden="true">·</span>
        <Readout
          label="SCALE"
          value={DRAFTING_SCALE}
          className="bottom-bar-readout-scale"
          title="The viewport frames the part to fit the sheet, so the sheet carries no fixed model-to-paper ratio. FIT states that honestly."
        />
        <span className="bottom-bar-separator" aria-hidden="true">·</span>
        <span
          className="bottom-bar-source"
          data-source={source}
          data-source-kind={designReadout ? 'provider' : preview.server ? 'server' : preview.local ? 'local' : 'none'}
          data-provider={designReadout?.provider ?? 'none'}
          data-preview-state={preview.inFlight ? 'verifying' : preview.lastError ? 'refused' : 'idle'}
          title={designReadout
            ? `These values are ${designReadout.measuredBy}; the viewport shows the OpenCascade import of the provider STEP.`
            : preview.lastError
              ? `The server refused the preview: ${preview.lastError}`
              : preview.inFlight
                ? 'The server is verifying the mesh.'
                : 'Envelope follows the server mesh when one exists; volume and area are closed-form nominal values.'}
        >
          {source}
        </span>
      </div>

      <div className="bottom-bar-cluster bottom-bar-right">
        <button
          type="button"
          className="bottom-bar-button bottom-bar-export"
          aria-label="Export STEP"
          onClick={() => props.onOpenOptions('export')}
          title="Open the export and handoff card"
        >
          Export
        </button>
        <span
          className="bottom-bar-freecad"
          data-status={freecadStatus}
          role="status"
          aria-label={describeFreecad(freecadStatus)}
          title={`${describeFreecad(freecadStatus)}${freecad?.notes ? ` — ${freecad.notes}` : ''}`}
        >
          <span className="bottom-bar-freecad-dot" aria-hidden="true" />
          <span className="bottom-bar-freecad-label">FreeCAD</span>
        </span>
        <button
          type="button"
          className="bottom-bar-options"
          aria-haspopup="dialog"
          onClick={() => props.onOpenOptions('inspector')}
        >
          ⚙ Options
        </button>
      </div>
    </footer>
  );
}

function IconToggle(props: { icon: BarIconName; label: string; active: boolean; onClick(): void }): ReactElement {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.active}
      aria-label={props.label}
      title={`${props.label} ${props.active ? 'on' : 'off'}`}
      className={`bottom-bar-icon-button${props.active ? ' is-active' : ''}`}
      onClick={props.onClick}
    >
      {BAR_ICONS[props.icon]}
    </button>
  );
}

function Readout(props: { label: string; value: string; title: string; className?: string }): ReactElement {
  return (
    <span
      className={`bottom-bar-readout-item${props.className ? ` ${props.className}` : ''}`}
      title={`${props.label} ${props.value} — ${props.title}`}
    >
      <span className="bottom-bar-readout-label">{props.label}</span>
      <span className="bottom-bar-readout-value">{props.value}</span>
    </span>
  );
}

function describeBounds(bounds: { min: [number, number, number]; max: [number, number, number] }): string {
  const dims = boundsDimensions(bounds);
  return `${shortNumber(dims.width)}×${shortNumber(dims.height)}×${shortNumber(dims.depth)} mm`;
}

/**
 * The readout of an active compiled design. Every value is the one the
 * provider reported: the envelope from its measured bounds, the volume and
 * surface area from its measured properties. A value the provider did not
 * report prints "—". The source names the provider and its version, so
 * nobody reads these as the closed-form nominal values of the primitive
 * preview or as an OpenCascade measurement of another provider's part.
 *
 * The bar is a single 48 px row, so the numbers are compact:
 * `60×45×40 mm · 35 571 mm³ · 11 889 mm²`. The exact measured values live
 * in the item titles and in the agent card.
 */
export interface DesignBarReadout {
  envelope: string;
  volume: string;
  area: string;
  source: string;
  measuredBy: string;
  provider: string;
  envelopeTitle: string;
  volumeTitle: string;
  areaTitle: string;
}

export function describeDesignReadout(design: DesignResult): DesignBarReadout {
  const measured = design.measured;
  const bounds = measured.bounds;
  const measuredBy = measurerLabel(design);
  const size = (axis: 0 | 1 | 2): string => {
    if (!bounds) return '—';
    return shortNumber(bounds.max[axis] - bounds.min[axis]);
  };
  const exactSize = (axis: 0 | 1 | 2): string => {
    if (!bounds) return '—';
    return fullNumber(bounds.max[axis] - bounds.min[axis]);
  };
  return {
    envelope: bounds ? `${size(0)}×${size(1)}×${size(2)} mm` : '—',
    volume: measured.volume_mm3 === null ? '—' : `${compactQuantity(measured.volume_mm3)} mm³`,
    area:
      measured.surface_area_mm2 === null
        ? '—'
        : `${compactQuantity(measured.surface_area_mm2)} mm²`,
    source: providerShortLabel(design),
    measuredBy: `measured by ${measuredBy}`,
    provider: design.provider,
    envelopeTitle: bounds
      ? `Envelope measured by ${measuredBy}: ${exactSize(0)} × ${exactSize(1)} × ${exactSize(2)} mm`
      : `Envelope not reported by ${measuredBy}`,
    volumeTitle:
      measured.volume_mm3 === null
        ? `Volume not reported by ${measuredBy}`
        : `Volume measured by ${measuredBy}: ${fullNumber(measured.volume_mm3)} mm³`,
    areaTitle:
      measured.surface_area_mm2 === null
        ? `Surface area not reported by ${measuredBy}`
        : `Surface area measured by ${measuredBy}: ${fullNumber(measured.surface_area_mm2)} mm²`,
  };
}

function describeEnvelope(envelopeMm: [number, number, number]): string {
  return `${shortNumber(envelopeMm[0])}×${shortNumber(envelopeMm[1])}×${shortNumber(envelopeMm[2])} mm`;
}

function shortNumber(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return Number(value.toFixed(2)).toString();
}

/** The exact measured number, six decimals at most, without float noise. */
function fullNumber(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return value.toFixed(6).replace(/\.?0+$/, '');
}

/**
 * A compact quantity for the 48 px bar: whole units with thin grouping
 * spaces (`35 571`), or one decimal below 1000 so a small part does not
 * round to a meaningless integer. The exact value stays in the tooltip.
 */
function compactQuantity(value: number): string {
  if (!Number.isFinite(value)) return '—';
  if (Math.abs(value) < 1000) return String(Number(value.toFixed(1)));
  return Math.round(value)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, '\u202F');
}

function describeFreecad(status: string): string {
  if (status === 'available_via_neutral_file') return 'FreeCAD: neutral';
  if (status === 'native_api_unavailable') return 'FreeCAD: no API';
  if (status === 'not_configured') return 'FreeCAD: unset';
  return 'FreeCAD: —';
}
