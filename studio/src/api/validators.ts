// Runtime shape validation for the design-turn responses.
//
// The studio backend is a separate process that can be a different version
// than this UI. A response that does not match the frozen contract is
// REFUSED, never coerced: a missing number is a hard error, not a zero.
// Silently defaulting `volume_mm3` to 0 would put a fabricated number on
// the drafting sheet, which is exactly the failure mode ADR-0009 forbids.
//
// Every failure raises `StudioValidationError` with the path of the offending
// field, so the UI can say which part of the contract was broken without
// echoing a payload.

import type {
  CadProviderId,
  CadProviderRecord,
  CadProvidersResponse,
  ChatResponse,
  DesignArtifacts,
  DesignBounds,
  DesignCheck,
  DesignCheckStatus,
  DesignExportFormat,
  DesignMeasured,
  DesignResult,
  DesignView,
  DesignViewName,
  DrawingPolyline,
  DrawingResponse,
  DrawingView,
  JournalEvent,
  JournalPage,
  JournalVerify,
  ProviderId,
  ProviderModel,
  ProviderModelCatalog,
  ProviderModelPricing,
  SectionBounds2D,
  SectionLoop,
  SectionPlane,
  SectionResponse,
} from '../types/studio';
import { CAD_PROVIDER_IDS, DESIGN_VIEW_NAMES, SECTION_PLANES } from '../types/studio';

/** A response did not match the frozen backend contract. */
export class StudioValidationError extends Error {
  /** Dotted path of the offending field, e.g. `measured.volume_mm3`. */
  readonly path: string;

  constructor(path: string, detail: string) {
    super(`${path}: ${detail}`);
    this.name = 'StudioValidationError';
    this.path = path;
  }
}

const DESIGN_ID_PATTERN = /^[0-9a-f]{64}$/;
const HEX64_PATTERN = /^[0-9a-f]{64}$/;
const PROVIDER_IDS: ReadonlyArray<ProviderId> = [
  'local',
  'openai',
  'openrouter',
  'codex',
  'claude_code',
  'codex_cli',
  'opencode_cli',
];

function describe(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'an array';
  return typeof value;
}

function fail(path: string, detail: string): never {
  throw new StudioValidationError(path, detail);
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function requireObject(value: unknown, path: string): Record<string, unknown> {
  if (!isPlainObject(value)) fail(path, `expected an object, got ${describe(value)}`);
  return value;
}

function requireArray(value: unknown, path: string): ReadonlyArray<unknown> {
  if (!Array.isArray(value)) fail(path, `expected an array, got ${describe(value)}`);
  return value;
}

function requireString(value: unknown, path: string): string {
  if (typeof value !== 'string') fail(path, `expected a string, got ${describe(value)}`);
  return value;
}

function requireBoolean(value: unknown, path: string): boolean {
  if (typeof value !== 'boolean') fail(path, `expected a boolean, got ${describe(value)}`);
  return value;
}

/**
 * A number that is present and finite. A missing, `null`, `NaN`, or
 * infinite value is refused — never replaced with 0.
 */
function requireFiniteNumber(value: unknown, path: string): number {
  if (typeof value !== 'number') fail(path, `expected a number, got ${describe(value)}`);
  if (!Number.isFinite(value)) fail(path, 'expected a finite number');
  return value;
}

function requireInteger(value: unknown, path: string): number {
  const parsed = requireFiniteNumber(value, path);
  if (!Number.isInteger(parsed)) fail(path, 'expected an integer');
  return parsed;
}

function requireNonNegativeInteger(value: unknown, path: string): number {
  const parsed = requireInteger(value, path);
  if (parsed < 0) fail(path, 'expected a non-negative integer');
  return parsed;
}

function requirePositiveInteger(value: unknown, path: string): number {
  const parsed = requireInteger(value, path);
  if (parsed < 1) fail(path, 'expected a positive integer');
  return parsed;
}

function requireStringArray(value: unknown, path: string): ReadonlyArray<string> {
  return requireArray(value, path).map((item, index) => requireString(item, `${path}[${index}]`));
}

function requireNumberArray(value: unknown, path: string): ReadonlyArray<number> {
  return requireArray(value, path).map((item, index) => requireFiniteNumber(item, `${path}[${index}]`));
}

function requireNullableString(value: unknown, path: string): string | null {
  if (value === null) return null;
  if (value === undefined) fail(path, 'expected a string or null, got nothing');
  return requireString(value, path);
}

/**
 * A field that the backend always serialises but that may honestly be null
 * (a provider that did not report the property). An absent field is a
 * contract break, not a null: the backend's `DesignMeasured` always carries
 * every key, so a missing one means the payload is not the B2 shape.
 */
function requireNullableNumber(value: unknown, path: string): number | null {
  if (value === null) return null;
  return requireFiniteNumber(value, path);
}

function requireNullableInteger(value: unknown, path: string): number | null {
  if (value === null) return null;
  return requireNonNegativeInteger(value, path);
}

function requireNullableBoolean(value: unknown, path: string): boolean | null {
  if (value === null) return null;
  return requireBoolean(value, path);
}

function requireNullableTuple3(value: unknown, path: string): [number, number, number] | null {
  if (value === null) return null;
  return requireTuple3(value, path);
}

function requireNullableBounds(value: unknown, path: string): DesignBounds | null {
  if (value === null) return null;
  return parseBounds(value, path);
}

function optionalString(source: Record<string, unknown>, key: string, path: string): string | undefined {
  const value = source[key];
  if (value === undefined) return undefined;
  return requireString(value, `${path}.${key}`);
}

function optionalBoolean(source: Record<string, unknown>, key: string, path: string): boolean | undefined {
  const value = source[key];
  if (value === undefined) return undefined;
  return requireBoolean(value, `${path}.${key}`);
}

function optionalNullableString(
  source: Record<string, unknown>,
  key: string,
  path: string,
): string | null | undefined {
  const value = source[key];
  if (value === undefined) return undefined;
  if (value === null) return null;
  return requireString(value, `${path}.${key}`);
}

function requireTuple3(value: unknown, path: string): [number, number, number] {
  const items = requireArray(value, path);
  if (items.length !== 3) fail(path, `expected three numbers, got ${items.length}`);
  return [
    requireFiniteNumber(items[0], `${path}[0]`),
    requireFiniteNumber(items[1], `${path}[1]`),
    requireFiniteNumber(items[2], `${path}[2]`),
  ];
}

function requirePoint2(value: unknown, path: string): [number, number] {
  const items = requireArray(value, path);
  if (items.length !== 2) fail(path, `expected two numbers, got ${items.length}`);
  return [requireFiniteNumber(items[0], `${path}[0]`), requireFiniteNumber(items[1], `${path}[1]`)];
}

function requireDesignId(value: unknown, path: string): string {
  const parsed = requireString(value, path);
  if (!DESIGN_ID_PATTERN.test(parsed)) fail(path, 'expected a 64-character lowercase hex sha256');
  return parsed;
}

function optionalHex64(value: unknown, path: string): string | null {
  // An absent digest is the honest "not exported yet" signal; a present but
  // malformed digest is a contract break and is refused.
  if (value === undefined || value === null) return null;
  const parsed = requireString(value, path);
  if (!HEX64_PATTERN.test(parsed)) fail(path, 'expected a 64-character lowercase hex sha256');
  return parsed;
}

// --- CAD providers ------------------------------------------------------------

/** The closed provider set, refused by name when the value is not one of them. */
export function requireCadProviderId(value: unknown, path = 'provider'): CadProviderId {
  const parsed = requireString(value, path);
  if (!CAD_PROVIDER_IDS.includes(parsed as CadProviderId)) {
    fail(path, `expected one of ${CAD_PROVIDER_IDS.join(', ')}`);
  }
  return parsed as CadProviderId;
}

function parseCadProviderRecord(value: unknown, path: string): CadProviderRecord {
  const source = requireObject(value, path);
  const version = source['version'];
  const reason = source['reason'];
  return {
    id: requireCadProviderId(source['id'], `${path}.id`),
    name: requireString(source['name'], `${path}.name`),
    available: requireBoolean(source['available'], `${path}.available`),
    version:
      version === undefined
        ? fail(`${path}.version`, 'expected a string or null, got nothing')
        : requireNullableString(version, `${path}.version`),
    reason:
      reason === undefined
        ? fail(`${path}.reason`, 'expected a string or null, got nothing')
        : requireNullableString(reason, `${path}.reason`),
    measured_by: requireString(source['measured_by'], `${path}.measured_by`),
    mode: requireString(source['mode'], `${path}.mode`),
  };
}

/**
 * Validate `GET /api/studio/providers/cad`. A duplicate provider id is
 * refused: the selector keys its rows by id, and two rows for one provider
 * would make the availability it shows ambiguous.
 */
export function parseCadProviders(value: unknown, path = 'providers/cad'): CadProvidersResponse {
  const source = requireObject(value, path);
  const records = requireArray(source['providers'], `${path}.providers`).map((item, index) =>
    parseCadProviderRecord(item, `${path}.providers[${index}]`),
  );
  const seen = new Set<string>();
  for (const record of records) {
    if (seen.has(record.id)) fail(`${path}.providers`, `duplicate provider ${record.id}`);
    seen.add(record.id);
  }
  return { providers: records };
}

// --- Design result ------------------------------------------------------------

function parseBounds(value: unknown, path: string): DesignBounds {
  const source = requireObject(value, path);
  return {
    min: requireTuple3(source['min'], `${path}.min`),
    max: requireTuple3(source['max'], `${path}.max`),
  };
}

function parseMeasured(value: unknown, path: string): DesignMeasured {
  const source = requireObject(value, path);
  return {
    volume_mm3: requireNullableNumber(source['volume_mm3'], `${path}.volume_mm3`),
    surface_area_mm2: requireNullableNumber(source['surface_area_mm2'], `${path}.surface_area_mm2`),
    centroid_mm: requireNullableTuple3(source['centroid_mm'], `${path}.centroid_mm`),
    bounds: requireNullableBounds(source['bounds'], `${path}.bounds`),
    size_mm: requireNullableTuple3(source['size_mm'], `${path}.size_mm`),
    valid: requireNullableBoolean(source['valid'], `${path}.valid`),
    solid_count: requireNullableInteger(source['solid_count'], `${path}.solid_count`),
    mass_g: requireNullableNumber(source['mass_g'], `${path}.mass_g`),
  };
}

function parseCheckValue(value: unknown, path: string): DesignCheck['expected'] {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) fail(path, 'expected a finite number');
    return value;
  }
  if (Array.isArray(value)) {
    const items = requireArray(value, path);
    if (items.length !== 2) fail(path, `expected a two-number range, got ${items.length}`);
    return [requireFiniteNumber(items[0], `${path}[0]`), requireFiniteNumber(items[1], `${path}[1]`)];
  }
  return fail(path, `expected a boolean, a number, or a two-number range, got ${describe(value)}`);
}

const CHECK_STATUSES: ReadonlyArray<DesignCheckStatus> = ['passed', 'failed', 'not_reported'];

function parseCheckStatus(value: unknown, path: string): DesignCheckStatus {
  const parsed = requireString(value, path);
  if (!CHECK_STATUSES.includes(parsed as DesignCheckStatus)) {
    fail(path, `expected one of ${CHECK_STATUSES.join(', ')}`);
  }
  return parsed as DesignCheckStatus;
}

function parseCheck(value: unknown, path: string): DesignCheck {
  const source = requireObject(value, path);
  const status = parseCheckStatus(source['status'], `${path}.status`);
  const measured =
    source['measured'] === undefined
      ? fail(`${path}.measured`, 'expected a check value or null, got nothing')
      : source['measured'] === null
        ? null
        : parseCheckValue(source['measured'], `${path}.measured`);
  const passed = requireBoolean(source['passed'], `${path}.passed`);
  // The three facts must agree. A `not_reported` row is neither a pass nor a
  // failure; a `passed` row cannot claim no measured value, and a `failed`
  // row cannot claim true. Refusing the contradiction here means the card
  // can render the status without interpreting it.
  if (status === 'not_reported' && measured !== null) {
    fail(`${path}.measured`, 'a not_reported check must carry no measured value');
  }
  if (status !== 'not_reported' && measured === null) {
    fail(`${path}.measured`, `a ${status} check must carry a measured value`);
  }
  if ((status === 'passed') !== passed) {
    fail(`${path}.passed`, `a ${status} check must report passed: ${status === 'passed'}`);
  }
  return {
    id: requireString(source['id'], `${path}.id`),
    label: requireString(source['label'], `${path}.label`),
    expected: parseCheckValue(source['expected'], `${path}.expected`),
    measured,
    tolerance: requireFiniteNumber(source['tolerance'], `${path}.tolerance`),
    unit: requireString(source['unit'], `${path}.unit`),
    passed,
    status,
  };
}

/**
 * The display view. It is the B2 `view` object, never the removed B1 `mesh`
 * field: a payload that still carries `mesh` has no `view` and is refused, so
 * an old backend cannot silently render an unverified tessellation.
 */
function parseView(value: unknown, path: string): DesignView {
  const source = requireObject(value, path);
  const positions = requireNumberArray(source['positions'], `${path}.positions`);
  const normals = requireNumberArray(source['normals'], `${path}.normals`);
  const indices = requireArray(source['indices'], `${path}.indices`).map((item, index) =>
    requireInteger(item, `${path}.indices[${index}]`),
  );
  if (positions.length % 3 !== 0) fail(`${path}.positions`, 'expected a flat list of xyz triples');
  if (normals.length % 3 !== 0) fail(`${path}.normals`, 'expected a flat list of xyz triples');
  if (positions.length !== normals.length) {
    fail(`${path}.normals`, 'expected one normal per position');
  }
  return {
    positions,
    normals,
    indices,
    vertex_count: requireNonNegativeInteger(source['vertex_count'], `${path}.vertex_count`),
    triangle_count: requireNonNegativeInteger(source['triangle_count'], `${path}.triangle_count`),
    deflection_mm: requireFiniteNumber(source['deflection_mm'], `${path}.deflection_mm`),
    engine: requireString(source['engine'], `${path}.engine`),
    bounds: parseBounds(source['bounds'], `${path}.bounds`),
    solids: requireNonNegativeInteger(source['solids'], `${path}.solids`),
    warnings: source['warnings'] === undefined
      ? []
      : requireStringArray(source['warnings'], `${path}.warnings`),
  };
}

function parseArtifacts(value: unknown, path: string): DesignArtifacts {
  const source = requireObject(value, path);
  const nativeKind = source['native_kind'];
  return {
    script_sha256: requireDesignId(source['script_sha256'], `${path}.script_sha256`),
    step_sha256: optionalHex64(source['step_sha256'], `${path}.step_sha256`) ?? null,
    native_sha256: optionalHex64(source['native_sha256'], `${path}.native_sha256`) ?? null,
    native_kind:
      nativeKind === undefined ? null : requireNullableString(nativeKind, `${path}.native_kind`),
  };
}

/** Validate a `DesignResult` payload. Throws `StudioValidationError`. */
export function parseDesignResult(value: unknown, path = 'design'): DesignResult {
  const source = requireObject(value, path);
  const declaredEnvelope = source['declared_envelope'];
  const result: DesignResult = {
    design_id: requireDesignId(source['design_id'], `${path}.design_id`),
    label: requireString(source['label'], `${path}.label`),
    provider: requireCadProviderId(source['provider'], `${path}.provider`),
    provider_version:
      source['provider_version'] === undefined
        ? fail(`${path}.provider_version`, 'expected a string or null, got nothing')
        : requireNullableString(source['provider_version'], `${path}.provider_version`),
    measured_by: requireString(source['measured_by'], `${path}.measured_by`),
    script: requireObject(source['script'], `${path}.script`),
    measured: parseMeasured(source['measured'], `${path}.measured`),
    checks: requireArray(source['checks'], `${path}.checks`).map((item, index) =>
      parseCheck(item, `${path}.checks[${index}]`),
    ),
    verified: requireBoolean(source['verified'], `${path}.verified`),
    view: parseView(source['view'], `${path}.view`),
    artifacts: parseArtifacts(source['artifacts'], `${path}.artifacts`),
    journal_head: optionalHex64(source['journal_head'], `${path}.journal_head`) ?? null,
    material: optionalNullableString(source, 'material', path) ?? null,
    tolerances: source['tolerances'] === undefined
      ? []
      : requireStringArray(source['tolerances'], `${path}.tolerances`),
    declared_envelope:
      declaredEnvelope === undefined || declaredEnvelope === null
        ? null
        : parseBounds(declaredEnvelope, `${path}.declared_envelope`),
    cached: optionalBoolean(source, 'cached', path) ?? false,
    warnings: source['warnings'] === undefined
      ? []
      : requireStringArray(source['warnings'], `${path}.warnings`),
    component_count: source['component_count'] === undefined
      ? 0
      : requireNonNegativeInteger(source['component_count'], `${path}.component_count`),
    fused: optionalBoolean(source, 'fused', path) ?? false,
  };
  return result;
}

// --- Drawing ------------------------------------------------------------------

function parsePolyline(value: unknown, path: string): DrawingPolyline {
  const source = requireObject(value, path);
  return {
    points: requireArray(source['points'], `${path}.points`).map((item, index) =>
      requirePoint2(item, `${path}.points[${index}]`),
    ),
    closed: requireBoolean(source['closed'], `${path}.closed`),
    point_count: requireNonNegativeInteger(source['point_count'], `${path}.point_count`),
  };
}

function parseDrawingView(value: unknown, path: string): DrawingView {
  const source = requireObject(value, path);
  const name = requireString(source['name'], `${path}.name`);
  if (!DESIGN_VIEW_NAMES.includes(name as DesignViewName)) {
    fail(`${path}.name`, `expected one of ${DESIGN_VIEW_NAMES.join(', ')}`);
  }
  return {
    name: name as DesignViewName,
    visible: requireArray(source['visible'], `${path}.visible`).map((item, index) =>
      parsePolyline(item, `${path}.visible[${index}]`),
    ),
    hidden: requireArray(source['hidden'], `${path}.hidden`).map((item, index) =>
      parsePolyline(item, `${path}.hidden[${index}]`),
    ),
    truncated: requireBoolean(source['truncated'], `${path}.truncated`),
    point_count: requireNonNegativeInteger(source['point_count'], `${path}.point_count`),
    polyline_count: requireNonNegativeInteger(source['polyline_count'], `${path}.polyline_count`),
  };
}

/** Validate a `DrawingResponse` payload. Throws `StudioValidationError`. */
export function parseDrawingResponse(value: unknown, path = 'drawing'): DrawingResponse {
  const source = requireObject(value, path);
  return {
    design_id: requireDesignId(source['design_id'], `${path}.design_id`),
    views: requireArray(source['views'], `${path}.views`).map((item, index) =>
      parseDrawingView(item, `${path}.views[${index}]`),
    ),
    chord_tolerance_mm: requireFiniteNumber(source['chord_tolerance_mm'], `${path}.chord_tolerance_mm`),
    max_points: requirePositiveInteger(source['max_points'], `${path}.max_points`),
    point_count: requireNonNegativeInteger(source['point_count'], `${path}.point_count`),
  };
}

// --- Section ------------------------------------------------------------------

function parseSectionBounds(value: unknown, path: string): SectionBounds2D {
  const source = requireObject(value, path);
  return {
    min_u: requireFiniteNumber(source['min_u'], `${path}.min_u`),
    min_v: requireFiniteNumber(source['min_v'], `${path}.min_v`),
    max_u: requireFiniteNumber(source['max_u'], `${path}.max_u`),
    max_v: requireFiniteNumber(source['max_v'], `${path}.max_v`),
    width: requireFiniteNumber(source['width'], `${path}.width`),
    height: requireFiniteNumber(source['height'], `${path}.height`),
    center_u: requireFiniteNumber(source['center_u'], `${path}.center_u`),
    center_v: requireFiniteNumber(source['center_v'], `${path}.center_v`),
  };
}

function parseSectionLoop(value: unknown, path: string): SectionLoop {
  const source = requireObject(value, path);
  return {
    points: requireArray(source['points'], `${path}.points`).map((item, index) =>
      requirePoint2(item, `${path}.points[${index}]`),
    ),
    area_mm2: requireFiniteNumber(source['area_mm2'], `${path}.area_mm2`),
    area_exact: requireBoolean(source['area_exact'], `${path}.area_exact`),
    outer: requireBoolean(source['outer'], `${path}.outer`),
    bounds: parseSectionBounds(source['bounds'], `${path}.bounds`),
    centroid: requirePoint2(source['centroid'], `${path}.centroid`),
    point_count: requireNonNegativeInteger(source['point_count'], `${path}.point_count`),
  };
}

/** Validate a `SectionResponse` payload. Throws `StudioValidationError`. */
export function parseSectionResponse(value: unknown, path = 'section'): SectionResponse {
  const source = requireObject(value, path);
  const plane = requireString(source['plane'], `${path}.plane`);
  if (!SECTION_PLANES.includes(plane as SectionPlane)) {
    fail(`${path}.plane`, `expected one of ${SECTION_PLANES.join(', ')}`);
  }
  return {
    design_id: requireDesignId(source['design_id'], `${path}.design_id`),
    plane: plane as SectionPlane,
    offset_mm: requireFiniteNumber(source['offset_mm'], `${path}.offset_mm`),
    loops: requireArray(source['loops'], `${path}.loops`).map((item, index) =>
      parseSectionLoop(item, `${path}.loops[${index}]`),
    ),
    area_mm2: requireFiniteNumber(source['area_mm2'], `${path}.area_mm2`),
    area_exact: requireBoolean(source['area_exact'], `${path}.area_exact`),
    chord_tolerance_mm: requireFiniteNumber(source['chord_tolerance_mm'], `${path}.chord_tolerance_mm`),
    truncated: requireBoolean(source['truncated'], `${path}.truncated`),
    outer_loop_count: requireNonNegativeInteger(source['outer_loop_count'], `${path}.outer_loop_count`),
    inner_loop_count: requireNonNegativeInteger(source['inner_loop_count'], `${path}.inner_loop_count`),
    point_count: requireNonNegativeInteger(source['point_count'], `${path}.point_count`),
  };
}

// --- Journal ------------------------------------------------------------------

function parseJournalEvent(value: unknown, path: string): JournalEvent {
  const source = requireObject(value, path);
  const occurredAt = requireString(source['occurred_at'], `${path}.occurred_at`);
  if (Number.isNaN(Date.parse(occurredAt))) {
    fail(`${path}.occurred_at`, 'expected an ISO-8601 timestamp');
  }
  const payload = source['payload'];
  return {
    sequence: requirePositiveInteger(source['sequence'], `${path}.sequence`),
    event_type: requireString(source['event_type'], `${path}.event_type`),
    aggregate_id: requireString(source['aggregate_id'], `${path}.aggregate_id`),
    payload: payload === undefined || payload === null ? null : requireObject(payload, `${path}.payload`),
    digest: requireString(source['digest'], `${path}.digest`),
    occurred_at: occurredAt,
    previous_event_digest: requireString(source['previous_event_digest'], `${path}.previous_event_digest`),
    event_digest: requireString(source['event_digest'], `${path}.event_digest`),
  };
}

/** Validate a `JournalPage` payload. Throws `StudioValidationError`. */
export function parseJournalPage(value: unknown, path = 'journal'): JournalPage {
  const source = requireObject(value, path);
  return {
    events: requireArray(source['events'], `${path}.events`).map((item, index) =>
      parseJournalEvent(item, `${path}.events[${index}]`),
    ),
    next_after: requireNonNegativeInteger(source['next_after'], `${path}.next_after`),
    has_more: requireBoolean(source['has_more'], `${path}.has_more`),
    limit: requirePositiveInteger(source['limit'], `${path}.limit`),
  };
}

/** Validate a `JournalVerify` payload. Throws `StudioValidationError`. */
export function parseJournalVerify(value: unknown, path = 'journal/verify'): JournalVerify {
  const source = requireObject(value, path);
  const firstBad = source['first_bad_sequence'];
  return {
    ok: requireBoolean(source['ok'], `${path}.ok`),
    event_count: requireNonNegativeInteger(source['event_count'], `${path}.event_count`),
    head_digest: optionalHex64(source['head_digest'], `${path}.head_digest`) ?? null,
    first_bad_sequence:
      firstBad === undefined || firstBad === null
        ? null
        : requirePositiveInteger(firstBad, `${path}.first_bad_sequence`),
  };
}

// --- Model catalog ------------------------------------------------------------

/**
 * One model row of `GET /api/studio/providers/{id}/models`.
 *
 * The rules are the backend's own: `free` is true when both prices are zero
 * or the id ends with `:free`. A payload that carries a non-numeric price or
 * a non-boolean `free` is a contract break and is refused; a field the
 * backend may honestly omit (context length, pricing, tool support) is
 * carried as "not reported" rather than filled in.
 */
function parseProviderModel(value: unknown, path: string): ProviderModel {
  const source = requireObject(value, path);
  const id = requireString(source['id'], `${path}.id`);
  const name = source['name'] === undefined ? id : requireString(source['name'], `${path}.name`);
  const contextLength = source['context_length'];
  const pricing = source['pricing'];
  const free = source['free'];
  const supportsTools = source['supports_tools'];
  return {
    id,
    name,
    context_length:
      contextLength === undefined || contextLength === null
        ? null
        : requireFiniteNumber(contextLength, `${path}.context_length`),
    pricing:
      pricing === undefined || pricing === null
        ? null
        : parseProviderModelPricing(pricing, `${path}.pricing`),
    free: free === undefined ? id.endsWith(':free') : requireBoolean(free, `${path}.free`),
    supports_tools:
      supportsTools === undefined || supportsTools === null
        ? null
        : requireBoolean(supportsTools, `${path}.supports_tools`),
  };
}

function parseProviderModelPricing(value: unknown, path: string): ProviderModelPricing {
  const source = requireObject(value, path);
  return {
    prompt: requireFiniteNumber(source['prompt'], `${path}.prompt`),
    completion: requireFiniteNumber(source['completion'], `${path}.completion`),
  };
}

/**
 * Validate the catalog envelope.
 *
 * `provider`, `source` and `models` are required; `fetched_at` and `error`
 * are optional (a timestamp format the UI does not need, and the honest
 * "the list could not be fetched" signal).
 */
export function parseProviderModelCatalog(
  value: unknown,
  path = 'providers/{id}/models',
): ProviderModelCatalog {
  const source = requireObject(value, path);
  const fetchedAt = source['fetched_at'];
  const error = source['error'];
  return {
    provider: requireString(source['provider'], `${path}.provider`),
    source: source['source'] === undefined ? '' : requireString(source['source'], `${path}.source`),
    fetched_at:
      fetchedAt === undefined || fetchedAt === null
        ? null
        : typeof fetchedAt === 'number'
          ? requireFiniteNumber(fetchedAt, `${path}.fetched_at`)
          : requireString(fetchedAt, `${path}.fetched_at`),
    models: requireArray(source['models'], `${path}.models`).map((item, index) =>
      parseProviderModel(item, `${path}.models[${index}]`),
    ),
    error: error === undefined || error === null ? null : requireString(error, `${path}.error`),
  };
}

// --- Chat ---------------------------------------------------------------------

/**
 * Validate a `ChatResponse` payload. The optional `design` is validated with
 * the full design contract, so a malformed design is refused instead of
 * being rendered as a half-populated agent card.
 */
export function parseChatResponse(value: unknown, path = 'chat'): ChatResponse {
  const source = requireObject(value, path);
  const provider = requireString(source['provider'], `${path}.provider`);
  if (!PROVIDER_IDS.includes(provider as ProviderId)) {
    fail(`${path}.provider`, `expected one of ${PROVIDER_IDS.join(', ')}`);
  }
  const design = source['design'];
  const response: ChatResponse = {
    text: requireString(source['text'], `${path}.text`),
    provider: provider as ProviderId,
    model: requireNullableString(source['model'], `${path}.model`),
    offline: requireBoolean(source['offline'], `${path}.offline`),
    design: design === undefined || design === null ? null : parseDesignResult(design, `${path}.design`),
  };
  return response;
}

// --- Export -------------------------------------------------------------------

const EXPORT_FORMATS: ReadonlyArray<DesignExportFormat> = ['step', 'stl'];

/** Validate a `format` query/body value for a design export. */
export function requireDesignExportFormat(value: unknown, path = 'format'): DesignExportFormat {
  const parsed = requireString(value, path);
  if (!EXPORT_FORMATS.includes(parsed as DesignExportFormat)) {
    fail(path, `expected one of ${EXPORT_FORMATS.join(', ')}`);
  }
  return parsed as DesignExportFormat;
}
