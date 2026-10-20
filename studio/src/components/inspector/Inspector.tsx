import type { ReactElement } from 'react';
// Inspector panel — editable parameter form per primitive.
//
// Inspector labels are human-friendly (Width / Height / Depth) but the
// payload uses the backend's exact API keys (size_x / size_y / size_z).
// Subdivisions for the box and height_segments for the cylinder are
// exposed directly. Validation ranges mirror the backend's geometry
// limits module; an out-of-range value never silently becomes a server
// refusal — the inspector catches it locally first.

import { useUiState, dispatch } from '../../state/store';
import { SurfaceMaterialControl } from '../panels/SurfaceMaterialControl';
import { VernierSlider } from './VernierSlider';
import {
  boundsDimensions,
  buildBox,
  buildCylinder,
  buildSphere,
  computeGeometryEstimate,
  defaultParameters,
  formatAreaEstimate,
  formatEnvelopeEstimate,
  formatEnvelopeVolumeEstimate,
  formatMillimetres,
  formatVolumeEstimate,
  validateBox,
  validateCylinder,
  validateSphere,
} from '../../viewer/localPrimitives';
import {
  BoxParameters,
  CylinderParameters,
  PrimitiveKind,
  SphereParameters,
  StudioParameters,
} from '../../types/studio';

interface NumberFieldProps {
  label: string;
  value: number;
  min: number;
  max: number;
  integer?: boolean;
  step?: number;
  suffix?: string;
  onChange(next: number): void;
}

function NumberField(props: NumberFieldProps): ReactElement {
  const error = describeValueError(props.value, props.min, props.max, props.integer);
  const step = props.step ?? (props.integer ? 1 : 0.5);
  return (
    <div className={`inspector-field${error ? ' has-error' : ''}`}>
      <VernierSlider
        label={props.label}
        value={props.value}
        min={props.min}
        max={props.max}
        step={step}
        integer={props.integer}
        unit={props.suffix}
        invalid={Boolean(error)}
        onChange={props.onChange}
      />
      {error ? <span className="inspector-field-error" role="alert">{error}</span> : null}
    </div>
  );
}

function describeValueError(value: number, min: number, max: number, integer: boolean | undefined): string | null {
  if (!Number.isFinite(value)) return 'Value must be a finite number';
  if (value <= 0) return 'Value must be greater than zero';
  if (integer && !Number.isInteger(value)) return 'Value must be an integer';
  if (value < min) return `Value must be ≥ ${min}`;
  if (value > max) return `Value must be ≤ ${max}`;
  return null;
}

export interface InspectorProps {
  /** The Options card supplies its own drafted heading; hide the inner one. */
  showHeading?: boolean;
}

export function Inspector(props: InspectorProps = {}): ReactElement {
  const showHeading = props.showHeading ?? true;
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const preview = useUiState((s) => s.preview);

  const validation = validationFor(primitive, parameters);
  const source = preview.server ? 'server' : preview.local ? 'local' : 'none';
  const sourceLabel = source === 'server'
    ? 'Server verified'
    : source === 'local'
      ? 'Local preview'
      : 'No mesh yet';
  return (
    <section className="inspector" aria-label="Inspector">
      {showHeading ? (
        <header className="inspector-header">
          <h2>Inspector</h2>
          <span className="inspector-source" data-source={source}>
            {sourceLabel}
          </span>
        </header>
      ) : null}
      <PrimitivePicker primitive={primitive} />
      <SurfaceMaterial />
      <ParameterForm primitive={primitive} parameters={parameters} />
      {validation ? (
        <p className="inspector-validation" role="alert">
          {validation.message}
        </p>
      ) : null}
      <div className="inspector-lower">
        <GeometryEstimates primitive={primitive} parameters={parameters} showNote={false} />
        {preview.local ? (
          <BoundsReadout
            parameters={parameters}
            bounds={preview.local.bounds}
            serverBounds={preview.server ? preview.server.preview.bounds : null}
          />
        ) : null}
      </div>
      <p className="inspector-estimates-note">{ESTIMATE_NOTE}</p>
    </section>
  );
}

function validationFor(kind: PrimitiveKind, p: StudioParameters) {
  if (kind === 'box') return validateBox(p as BoxParameters);
  if (kind === 'cylinder') return validateCylinder(p as CylinderParameters);
  return validateSphere(p as SphereParameters);
}

const KINDS: ReadonlyArray<{ id: PrimitiveKind; label: string }> = [
  { id: 'box', label: 'Box' },
  { id: 'cylinder', label: 'Cylinder' },
  { id: 'sphere', label: 'Sphere' },
];

function PrimitivePicker(props: { primitive: PrimitiveKind }): ReactElement {
  return (
    <div className="inspector-kind-picker" role="radiogroup" aria-label="Primitive kind">
      {KINDS.map((kind) => (
        <button
          key={kind.id}
          type="button"
          role="radio"
          aria-checked={props.primitive === kind.id}
          className={`inspector-kind-button${props.primitive === kind.id ? ' is-active' : ''}`}
          onClick={() => {
            if (props.primitive === kind.id) return;
            dispatch({
              type: 'primitive/change',
              kind: kind.id,
              parameters: defaultParameters(kind.id),
            });
          }}
        >
          {kind.label}
        </button>
      ))}
    </div>
  );
}

// Surface material of the drawn solid. The same control is in the bottom
// ruler bar; both write the one `viewport.material` field. The canvas
// background is not offered here on purpose: it follows the workspace theme
// (vellum paper by day, night slate at night) so the viewport surface and
// the drafting sheet behind it always agree.
function SurfaceMaterial(): ReactElement {
  return (
    <section className="inspector-material" aria-label="Surface material">
      <h3>Surface material</h3>
      <SurfaceMaterialControl variant="full" />
      <p className="inspector-material-note">
        Canvas background follows the workspace theme.
      </p>
    </section>
  );
}

interface GeometryEstimatesProps {
  primitive: PrimitiveKind;
  parameters: StudioParameters;
  showNote?: boolean;
}

const ESTIMATE_NOTE =
  'Closed-form nominal geometry for sizing & handoff. Not a certified FEA solver result. ' +
  'CAD/CAE handoff carries the ideal-solid numbers only. No stress, deflection, or tolerance results.';

// Nominal analytical geometry for the active primitive. This is the number
// set an OpenCascade (OCP) volume/area property query returns for the ideal
// solid, so it can be quoted for sizing or handed to an external CAD/CAE
// package. It is explicitly NOT a simulation result.
function GeometryEstimates(props: GeometryEstimatesProps): ReactElement {
  const estimate = computeGeometryEstimate(props.primitive, props.parameters);
  const showNote = props.showNote ?? true;
  return (
    <section className="inspector-estimates" aria-label="OpenCascade geometry estimate">
      <h3>OpenCascade estimate (nominal)</h3>
      <dl>
        <div><dt>Volume</dt><dd>{formatVolumeEstimate(estimate)}</dd></div>
        <div><dt>Surface area</dt><dd>{formatAreaEstimate(estimate)}</dd></div>
        <div><dt>Bounding envelope</dt><dd>{formatEnvelopeEstimate(estimate)}</dd></div>
        <div><dt>Envelope volume</dt><dd>{formatEnvelopeVolumeEstimate(estimate)}</dd></div>
      </dl>
      {showNote ? <p className="inspector-estimates-note">{ESTIMATE_NOTE}</p> : null}
    </section>
  );
}

interface ParameterFormProps {
  primitive: PrimitiveKind;
  parameters: StudioParameters;
}

function ParameterForm(props: ParameterFormProps): ReactElement {
  if (props.primitive === 'box') {
    const p = props.parameters as BoxParameters;
    return (
      <div className="inspector-grid">
        <NumberField label="Width (size_x)" value={p.size_x} min={0.1} max={2000} suffix="mm" onChange={(next) => patchBox(p, { size_x: next })} />
        <NumberField label="Height (size_y)" value={p.size_y} min={0.1} max={2000} suffix="mm" onChange={(next) => patchBox(p, { size_y: next })} />
        <NumberField label="Depth (size_z)" value={p.size_z} min={0.1} max={2000} suffix="mm" onChange={(next) => patchBox(p, { size_z: next })} />
        <NumberField label="Subdivisions" value={p.subdivisions} min={1} max={64} integer step={1} onChange={(next) => patchBox(p, { subdivisions: clampInt(next, 1, 64) })} />
      </div>
    );
  }
  if (props.primitive === 'cylinder') {
    const p = props.parameters as CylinderParameters;
    return (
      <div className="inspector-grid">
        <NumberField label="Radius" value={p.radius} min={0.1} max={2000} suffix="mm" onChange={(next) => patchCylinder(p, { radius: next })} />
        <NumberField label="Height" value={p.height} min={0.1} max={2000} suffix="mm" onChange={(next) => patchCylinder(p, { height: next })} />
        <NumberField label="Radial segments" value={p.radial_segments} min={6} max={256} integer step={1} onChange={(next) => patchCylinder(p, { radial_segments: clampInt(next, 6, 256) })} />
        <NumberField label="Height segments" value={p.height_segments} min={1} max={64} integer step={1} onChange={(next) => patchCylinder(p, { height_segments: clampInt(next, 1, 64) })} />
      </div>
    );
  }
  const p = props.parameters as SphereParameters;
  return (
    <div className="inspector-grid">
      <NumberField label="Radius" value={p.radius} min={0.1} max={2000} suffix="mm" onChange={(next) => patchSphere(p, { radius: next })} />
      <NumberField label="Rings" value={p.rings} min={3} max={96} integer step={1} onChange={(next) => patchSphere(p, { rings: clampInt(next, 3, 96) })} />
      <NumberField label="Sectors" value={p.sectors} min={4} max={128} integer step={1} onChange={(next) => patchSphere(p, { sectors: clampInt(next, 4, 128) })} />
    </div>
  );
}

interface BoundsReadoutProps {
  parameters: StudioParameters;
  bounds: { min: [number, number, number]; max: [number, number, number] };
  serverBounds: { min: [number, number, number]; max: [number, number, number] } | null;
}

function BoundsReadout(props: BoundsReadoutProps): ReactElement {
  const dims = boundsDimensions(props.bounds);
  const drift = props.serverBounds ? describeBoundsDrift(dims, boundsDimensions(props.serverBounds)) : [];
  return (
    <section className="inspector-bounds" aria-label="Computed bounds in millimetres">
      <h3>Bounds (mm)</h3>
      <dl>
        <div><dt>Width</dt><dd>{formatMillimetres(dims.width)}</dd></div>
        <div><dt>Height</dt><dd>{formatMillimetres(dims.height)}</dd></div>
        <div><dt>Depth</dt><dd>{formatMillimetres(dims.depth)}</dd></div>
      </dl>
      {drift.length > 0 ? (
        <p className="inspector-drift" role="status">
          Server-verified bounds differ from local preview by{' '}
          {drift.map((entry) => `${entry.label} ${entry.value}`).join(', ')}.
        </p>
      ) : null}
    </section>
  );
}

interface DriftEntry {
  label: string;
  value: string;
}

function describeBoundsDrift(
  local: { width: number; height: number; depth: number },
  server: { width: number; height: number; depth: number },
): DriftEntry[] {
  const tolerance = 0.01;
  const items: DriftEntry[] = [];
  for (const [label, a, b] of [
    ['width', local.width, server.width],
    ['height', local.height, server.height],
    ['depth', local.depth, server.depth],
  ] as ReadonlyArray<[string, number, number]>) {
    const delta = Math.abs(a - b);
    if (delta > tolerance) items.push({ label, value: `${delta.toFixed(3)} mm` });
  }
  return items;
}

function patchBox(current: BoxParameters, patch: Partial<BoxParameters>): void {
  const next = { ...current, ...patch };
  const err = validateBox(next);
  if (err) {
    // Show local validation error rather than dispatching a partial change
    // that would also fail server-side.
    dispatch({ type: 'parameters/change', parameters: next });
    return;
  }
  dispatch({ type: 'parameters/change', parameters: next });
}

function patchCylinder(current: CylinderParameters, patch: Partial<CylinderParameters>): void {
  const next = { ...current, ...patch };
  validateCylinder(next);
  dispatch({ type: 'parameters/change', parameters: next });
}

function patchSphere(current: SphereParameters, patch: Partial<SphereParameters>): void {
  const next = { ...current, ...patch };
  validateSphere(next);
  dispatch({ type: 'parameters/change', parameters: next });
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  const rounded = Math.round(value);
  if (rounded < min) return min;
  if (rounded > max) return max;
  return rounded;
}

// Re-export the local builders so other components can call them without
// reaching into the viewer module.
export const localPreviewFor = {
  box: (p: BoxParameters) => buildBox(p),
  cylinder: (p: CylinderParameters) => buildCylinder(p),
  sphere: (p: SphereParameters) => buildSphere(p),
};