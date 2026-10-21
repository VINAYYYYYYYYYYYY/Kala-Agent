// The agent card's data: the provider-measured values, the backend's checks,
// and the digest chips of a provider-built design.
//
// Every number the card renders comes from the backend's `DesignResult`.
// The card formats what the provider measured and what the operator declared;
// it never measures anything itself. A property the provider did not report
// is null and prints "—". The single derived number is the mass, and it
// exists only when the provider reports it or the backend DECLARED a density:
// a density is never inferred from a material name, because an inferred
// density is a fabricated number on a drafting sheet.

import type { CheckValue, DesignArtifactKind, DesignResult } from '../../types/studio';
import type { ServerState } from '../../state/store';

/** One row of the measured table. */
export interface MeasuredRow {
  /** Stable key, also used as the row's test hook. */
  key: string;
  /** The short label printed in the table's first column. */
  label: string;
  /** The measured value, already formatted with its unit. */
  value: string;
  /** The full explanation, shown as the row's tooltip. */
  title: string;
}

/** One digest chip: the first eight characters plus the full digest. */
export interface DigestChipData {
  /** Which artefact the digest belongs to. */
  kind: 'script' | 'step' | 'native';
  /** The chip's printed kind: `script`, `step`, or the native format (`fcstd`). */
  label: string;
  /** The full SHA-256, the value the copy button puts on the clipboard. */
  digest: string;
  /** The first eight characters, the value printed on the chip. */
  short: string;
  /**
   * The stored-artifact kind the download route accepts, or null when this
   * digest names no downloadable kind (an unknown native format). The chip
   * only offers a download when the backend can actually serve the bytes.
   */
  downloadKind: DesignArtifactKind | null;
}

/** The artifact kinds the download route serves. */
export const DOWNLOADABLE_ARTIFACT_KINDS: ReadonlyArray<DesignArtifactKind> = [
  'script',
  'step',
  'fcstd',
  'stl',
];

/** Whether a native format name is a stored-artifact kind. */
export function downloadableKind(kind: string | null | undefined): DesignArtifactKind | null {
  const normalized = kind?.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === 'stp' || normalized === 'step') return 'step';
  return (DOWNLOADABLE_ARTIFACT_KINDS as ReadonlyArray<string>).includes(normalized)
    ? (normalized as DesignArtifactKind)
    : null;
}

/**
 * The key the backend would use to declare a material density, in g/cm^3.
 *
 * B2 reports a provider-measured `mass_g` directly; this declared-density
 * path remains the fallback for a backend that declares a density but does
 * not report mass. The mass row is omitted when neither exists, because a
 * density inferred from a material name would be a fabricated number.
 */
interface DeclaredDensityCarrier {
  declared_density_g_cm3?: number | null;
}

/**
 * The material density the backend declared, in g/cm^3, or null when it
 * declared none. A missing, non-numeric, or non-positive density is "not
 * declared": a mass row built on it would be a guess.
 */
export function readDeclaredDensity(design: DesignResult): number | null {
  const value = (design as DesignResult & DeclaredDensityCarrier).declared_density_g_cm3;
  if (typeof value !== 'number') return null;
  if (!Number.isFinite(value) || value <= 0) return null;
  return value;
}

/** The mass of a volume at a declared density, in grams. */
export function massGrams(volumeMm3: number, densityGPerCm3: number): number {
  return (volumeMm3 / 1000) * densityGPerCm3;
}

/**
 * The provider identity of a design, e.g. `measured by FreeCAD 1.1.4`.
 *
 * `measured_by` is the backend's name for the measuring provider (ADR-0011
 * §4); the version is the provider's own reported version. The OpenCascade
 * viewer is never named as the measurer of a FreeCAD or Solid Edge part.
 */
export function measuredByLabel(design: DesignResult): string {
  return `measured by ${measurerLabel(design)}`;
}

/** The provider identity without the `measured by` prefix, e.g. `FreeCAD 1.1.4`. */
export function measurerLabel(design: DesignResult): string {
  const version = design.provider_version?.trim();
  return version ? `${design.measured_by} ${version}` : design.measured_by;
}

/**
 * The shortest honest provider label for the 48 px bar. The full identity
 * (`measured by FreeCAD 1.1.4`) stays in the tooltip and on the agent card;
 * the bar only has room for the provider's name.
 */
export function providerShortLabel(design: DesignResult): string {
  if (design.provider === 'freecad') return 'FreeCAD';
  if (design.provider === 'opencascade') return 'OpenCascade';
  if (design.provider === 'solidedge') return 'Solid Edge';
  return design.measured_by;
}

/** The size of the measured envelope along one axis, in millimetres, or null. */
export function envelopeSize(design: DesignResult, axis: 0 | 1 | 2): number | null {
  const bounds = design.measured.bounds;
  if (!bounds) return null;
  return bounds.max[axis] - bounds.min[axis];
}

/**
 * The measured table of the card: volume, area, and the envelope along the
 * three measured axes. A property the provider did not report prints "—" —
 * never a substituted number. The mass row appears when the provider reports
 * a mass, or when the backend DECLARED a density and reported a volume.
 */
export function measuredRows(design: DesignResult): MeasuredRow[] {
  const measured = design.measured;
  const measurer = design.measured_by;
  const rows: MeasuredRow[] = [
    {
      key: 'volume',
      label: 'volume',
      value: formatMeasuredValue(measured.volume_mm3, ' mm³'),
      title: measured.volume_mm3 === null
        ? `Volume not reported by ${measurer}`
        : `Volume measured by ${measurer}`,
    },
    {
      key: 'area',
      label: 'area',
      value: formatMeasuredValue(measured.surface_area_mm2, ' mm²'),
      title: measured.surface_area_mm2 === null
        ? `Surface area not reported by ${measurer}`
        : `Surface area measured by ${measurer}`,
    },
    {
      key: 'envelope_x',
      label: 'envelope X',
      value: formatMm(envelopeSize(design, 0), ' mm'),
      title: measured.bounds
        ? `Measured bounds X: ${formatMm(measured.bounds.min[0])} → ${formatMm(measured.bounds.max[0])} mm`
        : `Envelope X not reported by ${measurer}`,
    },
    {
      key: 'envelope_y',
      label: 'envelope Y',
      value: formatMm(envelopeSize(design, 1), ' mm'),
      title: measured.bounds
        ? `Measured bounds Y: ${formatMm(measured.bounds.min[1])} → ${formatMm(measured.bounds.max[1])} mm`
        : `Envelope Y not reported by ${measurer}`,
    },
    {
      key: 'envelope_z',
      label: 'envelope Z',
      value: formatMm(envelopeSize(design, 2), ' mm'),
      title: measured.bounds
        ? `Measured bounds Z: ${formatMm(measured.bounds.min[2])} → ${formatMm(measured.bounds.max[2])} mm`
        : `Envelope Z not reported by ${measurer}`,
    },
  ];

  if (measured.mass_g !== null) {
    rows.push({
      key: 'mass',
      label: 'mass',
      value: `${formatMm(measured.mass_g)} g`,
      title: `Mass measured by ${measurer}`,
    });
    return rows;
  }

  const density = readDeclaredDensity(design);
  if (density !== null && measured.volume_mm3 !== null) {
    rows.push({
      key: 'mass',
      label: 'mass',
      value: `${formatMm(massGrams(measured.volume_mm3, density))} g`,
      title: `Mass from the declared density ${density} g/cm³ and the measured volume`,
    });
  }
  return rows;
}

/**
 * The digest chips of the card. The script digest is always present; the
 * STEP digest appears once the provider's STEP artefact exists, and a native
 * chip (e.g. FCStd) appears when the provider delivered one. A digest the
 * backend did not report is not shown at all.
 */
export function digestChips(design: DesignResult): DigestChipData[] {
  const chips: DigestChipData[] = [];
  const script = design.artifacts.script_sha256;
  if (typeof script === 'string' && script.length > 0) {
    chips.push({
      kind: 'script',
      label: 'script',
      digest: script,
      short: script.slice(0, 8),
      downloadKind: 'script',
    });
  }
  const step = design.artifacts.step_sha256;
  if (typeof step === 'string' && step.length > 0) {
    chips.push({
      kind: 'step',
      label: 'step',
      digest: step,
      short: step.slice(0, 8),
      downloadKind: 'step',
    });
  }
  const native = design.artifacts.native_sha256;
  if (typeof native === 'string' && native.length > 0) {
    chips.push({
      kind: 'native',
      label: nativeKindLabel(design.artifacts.native_kind),
      digest: native,
      short: native.slice(0, 8),
      downloadKind: downloadableKind(design.artifacts.native_kind),
    });
  }
  return chips;
}

/** The printed name of a provider's native artefact, e.g. `FCStd`. */
export function nativeKindLabel(kind: string | null | undefined): string {
  const trimmed = kind?.trim();
  if (!trimmed) return 'native';
  return trimmed.toLowerCase() === 'fcstd' ? 'FCStd' : trimmed;
}

/** How many of the backend's checks passed, failed, or were not reported. */
export function checkSummary(design: DesignResult): {
  passed: number;
  failed: number;
  notReported: number;
  total: number;
} {
  let passed = 0;
  let failed = 0;
  let notReported = 0;
  for (const check of design.checks) {
    if (check.status === 'not_reported') notReported += 1;
    else if (check.status === 'failed' || !check.passed) failed += 1;
    else passed += 1;
  }
  return { passed, failed, notReported, total: design.checks.length };
}

/** The expected or measured value of a check row, formatted for the card. */
export function formatCheckValue(value: CheckValue | null): string {
  if (value === null) return '—';
  if (typeof value === 'boolean') return value ? 'true' : 'false';
  if (typeof value === 'number') return formatCheckNumber(value);
  return `[${value.map((entry) => formatCheckNumber(entry)).join(', ')}]`;
}

/** The unit of a check row, or null when the unit adds nothing. */
export function formatCheckUnit(unit: string): string | null {
  const trimmed = unit.trim();
  if (trimmed.length === 0) return null;
  if (trimmed === 'bool') return null;
  return trimmed;
}

/**
 * A measured property at the provider's precision, without float noise.
 *
 * The bar rounds to whole units; the card and the tooltips keep the value
 * the provider reported. Six decimals is enough to cover the FreeCAD and
 * OpenCascade reports and still trims their binary noise
 * (`45.00000000000001` → `45`).
 */
export function formatMeasuredNumber(value: number | null): string {
  if (value === null || !Number.isFinite(value)) return '—';
  return String(Number(value.toFixed(6)));
}

/** The exact measured number with its unit, for the card's value column. */
export function formatMeasuredValue(value: number | null, unit: string): string {
  if (value === null || !Number.isFinite(value)) return '—';
  return `${formatMeasuredNumber(value)}${unit}`;
}

/** A millimetre-scale measurement, with two decimals; null prints "—". */
export function formatMm(value: number | null, unit = ''): string {
  if (value === null || !Number.isFinite(value)) return '—';
  return `${value.toFixed(2)}${unit}`;
}

/** A check number, without trailing zeros. */
export function formatCheckNumber(value: number): string {
  if (!Number.isFinite(value)) return '—';
  return String(Number(value.toFixed(3)));
}

// --- The backend without design support ---------------------------------------

/**
 * What the agent card says about design-turn support.
 *
 * `design_available` is the only signal the frontend has: `false` means the
 * backend says it cannot compile a design, `null` means the backend did not
 * report the flag at all (an older build), and `true` means the routes are
 * there. The poller in the data layer uses the same flag, so the panel and
 * the ledger never disagree about whether design turns exist.
 */
export interface DesignSupport {
  /** True when this backend cannot compile a design turn. */
  unavailable: boolean;
  /** The one honest line to print, or null when design turns are available. */
  note: string | null;
}

/**
 * The honest one-line note for a backend that cannot compile designs.
 *
 * The note is shown once the health probe has answered. Before that the
 * panel stays quiet: a boot-time flash of "no design support" would be a
 * claim the frontend cannot make yet.
 */
export function designSupport(server: ServerState): DesignSupport {
  if (server.designAvailable === false) {
    return {
      unavailable: true,
      note:
        'Design turns need the Python geometry backend; this backend reports design_available: false.',
    };
  }
  if (server.designAvailable === null && server.health === 'ok') {
    return {
      unavailable: true,
      note:
        'Design turns need the Python geometry backend; this backend did not report design_available.',
    };
  }
  return { unavailable: false, note: null };
}
