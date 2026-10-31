// Status bar — the app's read-only footer (28 px).
//
// It reports, left to right: the loopback connection (a button that opens
// Settings › Connection), the measured readout (ENV · VOL · AREA · SCALE ·
// source) while the drawing is open, the backend's real FreeCAD integration
// row, and the backend version. There are no view toggles and no
// Export/Settings buttons here: those live on the top bar and in the sheet's
// tool rail (D3/D5).
//
// The readout describes the drawing, so it is shown only while the drawing
// is open (chat-first direction, 2026-10-31): at boot it used to describe an
// unasked-for box. The connection, FreeCAD and version facts always show.

import type { ReactElement } from 'react';
import {
  dispatch,
  useUiState,
  selectActiveDesign,
  selectConnection,
  selectDrawingOpen,
  selectPreview,
  selectServer,
} from '../../state/store';
import type { ConnectionStatus } from '../../state/store';
import type { DesignResult } from '../../types/studio';
import { measurerLabel, providerShortLabel } from '../chat/designCard';
import {
  boundsDimensions,
  computeGeometryEstimate,
} from '../../viewer/localPrimitives';
import { DRAFTING_SCALE } from '../../viewer/draftingGeometry';
import '../../styles/bottombar.css';

export function BottomBar(): ReactElement {
  const preview = useUiState(selectPreview);
  const server = useUiState(selectServer);
  const connection = useUiState(selectConnection);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const activeDesign = useUiState(selectActiveDesign);
  const drawingOpen = useUiState(selectDrawingOpen);

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
  const connectionView = describeConnectionStatus(connection);

  return (
    <footer className="bottom-bar" aria-label="Status bar">
      <div className="bottom-bar-cluster bottom-bar-left">
        <button
          type="button"
          className="bottom-bar-connection"
          data-state={connectionView.state}
          aria-haspopup="dialog"
          aria-label={`Connection: ${connectionView.label} — open Settings`}
          title={`${connectionView.title} — open Settings › Connection`}
          onClick={() => dispatch({ type: 'options/open', card: 'connection' })}
        >
          <span className="bottom-bar-connection-dot" aria-hidden="true" />
          <span className="bottom-bar-connection-label">{connectionView.label}</span>
        </button>
      </div>

      {drawingOpen ? (
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
      ) : null}

      <div className="bottom-bar-cluster bottom-bar-right">
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
        <span className="bottom-bar-version" title="Backend version reported by /health">
          {server.version ? `backend v${server.version}` : 'backend v—'}
        </span>
      </div>
    </footer>
  );
}

/**
 * The real connection state, spelled out for the status bar. A ready
 * loopback names its mode; every other state is reported plainly instead of
 * being dressed up as connected.
 */
export function describeConnectionStatus(connection: ConnectionStatus): {
  label: string;
  state: 'desktop' | 'browser' | 'connecting' | 'error' | 'idle';
  title: string;
} {
  if (connection.kind === 'ready') {
    const desktop = connection.connection.mode === 'desktop';
    return {
      label: `Connected · ${desktop ? 'Desktop' : 'Browser fallback'}`,
      state: desktop ? 'desktop' : 'browser',
      title: `Loopback ${connection.connection.baseUrl} (${desktop ? 'desktop bridge' : 'browser fallback'})`,
    };
  }
  if (connection.kind === 'connecting') {
    return { label: 'Connecting', state: 'connecting', title: 'Waiting for the local backend' };
  }
  if (connection.kind === 'error') {
    return { label: 'Connection failed', state: 'error', title: connection.message };
  }
  return { label: 'Not connected', state: 'idle', title: 'No loopback connection yet' };
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
 * The bar is a single row, so the numbers are compact:
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
 * A compact quantity for the single status row: whole units with thin
 * grouping spaces (`35 571`), or one decimal below 1000 so a small part
 * does not round to a meaningless integer. The exact value stays in the
 * tooltip.
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
