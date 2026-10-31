// Active geometry summary — the workbench's honest record of what the sheet
// is holding right now.
//
// Moved out of the chat column (D4) into Details › Properties. With a
// compiled design the rows quote the provider's measured values and name the
// provider; without one they quote the closed-form nominal estimate of the
// primitive. A value that was not reported is "—", never a substitute.
//
// The markup and classes are local to the shell so the chat and the details
// column can never share a stylesheet rule by accident.

import { useMemo } from 'react';
import type { ReactElement } from 'react';
import { selectActiveDesign, useUiState } from '../../state/store';
import {
  computeGeometryEstimate,
  formatAreaEstimate,
  formatEnvelopeEstimate,
  formatVolumeEstimate,
} from '../../viewer/localPrimitives';
import { measuredByLabel } from '../chat/designCard';
import type {
  BoxParameters,
  CylinderParameters,
  PrimitiveKind,
  SphereParameters,
  StudioParameters,
} from '../../types/studio';

export function ActiveGeometrySummary(): ReactElement {
  const activeDesign = useUiState(selectActiveDesign);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);

  const geometry = useMemo(() => {
    if (activeDesign) {
      const measured = activeDesign.measured;
      const bounds = measured.bounds;
      return {
        kind: 'design',
        dimensions: activeDesign.label,
        envelope: bounds
          ? `${(bounds.max[0] - bounds.min[0]).toFixed(2)} × ${(bounds.max[1] - bounds.min[1]).toFixed(2)} × ${(bounds.max[2] - bounds.min[2]).toFixed(2)} mm`
          : '—',
        volume: measured.volume_mm3 === null
          ? '—'
          : `${measured.volume_mm3.toFixed(2)} mm³ (${(measured.volume_mm3 / 1000).toFixed(3)} cm³)`,
        area: measured.surface_area_mm2 === null
          ? '—'
          : `${measured.surface_area_mm2.toFixed(2)} mm² (${(measured.surface_area_mm2 / 1000).toFixed(3)} cm²)`,
        source: measuredByLabel(activeDesign),
      };
    }
    const estimate = computeGeometryEstimate(primitive, parameters);
    return {
      kind: primitive,
      dimensions: describeParameters(primitive, parameters),
      envelope: formatEnvelopeEstimate(estimate),
      volume: formatVolumeEstimate(estimate),
      area: formatAreaEstimate(estimate),
      source: 'nominal estimate (closed-form)',
    };
  }, [activeDesign, primitive, parameters]);

  return (
    <section className="details-geometry" aria-label="Active OpenCascade geometry context">
      <div className="details-geometry-head">
        <span className="details-geometry-label">Active geometry</span>
        <code className="details-geometry-kind">{geometry.kind}</code>
      </div>
      <dl className="details-geometry-table">
        <div className="details-geometry-row">
          <dt>DIMS</dt>
          <dd className="details-geometry-dims">{geometry.dimensions}</dd>
        </div>
        <div className="details-geometry-row">
          <dt>ENVELOPE</dt>
          <dd className="details-geometry-envelope">{geometry.envelope}</dd>
        </div>
        <div className="details-geometry-row">
          <dt>VOLUME</dt>
          <dd className="details-geometry-volume">{geometry.volume}</dd>
        </div>
        <div className="details-geometry-row">
          <dt>SURFACE AREA</dt>
          <dd className="details-geometry-area">{geometry.area}</dd>
        </div>
        <div className="details-geometry-row">
          <dt>SOURCE</dt>
          <dd className="details-geometry-source">{geometry.source}</dd>
        </div>
      </dl>
    </section>
  );
}

function describeParameters(
  primitive: PrimitiveKind,
  parameters: StudioParameters,
): string {
  if (primitive === 'box') {
    const p = parameters as BoxParameters;
    return `${mm(p.size_x)} × ${mm(p.size_y)} × ${mm(p.size_z)}`;
  }
  if (primitive === 'cylinder') {
    const p = parameters as CylinderParameters;
    return `⌀${mm(p.radius * 2)} × ${mm(p.height)}`;
  }
  const p = parameters as SphereParameters;
  return `⌀${mm(p.radius * 2)}`;
}

function mm(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return `${value.toFixed(2)} mm`;
}
