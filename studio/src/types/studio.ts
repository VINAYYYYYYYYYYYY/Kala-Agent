// Type definitions matching the backend's frozen Pydantic schemas.
// Source: src/kala_agent/studio/schemas.py and src/kala_agent/studio/geometry/limits.py.
// Field names and shapes are deliberately stable; new fields follow rather than guess.

export type PrimitiveKind = 'box' | 'cylinder' | 'sphere';
export type ExportFormat = 'stl' | 'step';

// --- Workspace view contracts -------------------------------------------------
//
// These are shared between the 4-zone shell, the bottom bar, the Options
// drawer, and the P2 slot components. They are deliberately explicit so P2
// can build against a frozen contract.

/** How the drafting sheet renders the active primitive. */
export type RenderMode = 'pencil2d' | 'clay3d';

/** Secondary display shown in the right-hand AuxView slot. */
export type AuxViewKind = 'section' | 'ortho' | 'tolerance';

/** Workspace theme. Persisted in localStorage (never in server state). */
export type ThemeName = 'vellum' | 'slate';

/** Units of the parametric dimensions shown on the drafting sheet. */
export type StudioUnits = 'mm';

/** The four drafted cards in the Options drawer. */
export type OptionsCard = 'inspector' | 'export' | 'providers' | 'connection';

/** Journal event categories. */
export type JournalKind = 'preview' | 'chat' | 'tool' | 'export' | 'connection' | 'system';

/** One bounded journal row. `digest` is optional and only shown when real. */
export interface JournalEntry {
  id: string;
  ts: number;
  kind: JournalKind;
  message: string;
  digest?: string;
}

/** Hash-chain verification state. `'unknown'` until the backend reports it. */
export type HashChainStatus = 'unknown' | 'verified' | 'broken';

/** Maximum number of journal rows kept by the store (oldest dropped first). */
export const JOURNAL_CAP = 500;
export type IntegrationStatus =
  | 'available_via_neutral_file'
  | 'native_api_unavailable'
  | 'not_configured';
export type IntegrationCategory = 'cad' | 'analysis';
export type ProviderId = 'local' | 'openai' | 'openrouter' | 'codex';
export type CredentialProviderId = 'openai' | 'openrouter';
export type CredentialSource = 'none' | 'environment' | 'stored';

export const MIN_DIMENSION_MM = 0.1;
export const MAX_DIMENSION_MM = 2000;
export const MIN_BOX_SUBDIVISIONS = 1;
export const MAX_BOX_SUBDIVISIONS = 64;
export const MIN_CYLINDER_RADIAL_SEGMENTS = 6;
export const MAX_CYLINDER_RADIAL_SEGMENTS = 256;
export const MIN_CYLINDER_HEIGHT_SEGMENTS = 1;
export const MAX_CYLINDER_HEIGHT_SEGMENTS = 64;
export const MIN_SPHERE_RINGS = 3;
export const MAX_SPHERE_RINGS = 96;
export const MIN_SPHERE_SECTORS = 4;
export const MAX_SPHERE_SECTORS = 128;
export const MAX_MESH_VERTICES = 200_000;

export interface BBox {
  min: [number, number, number];
  max: [number, number, number];
}

// Backend primitives — keys must match `build_primitive` exactly.
// Human labels (Width / Height / Depth) are kept in the inspector; the
// payload always uses the API keys.
export interface BoxParameters {
  size_x: number;
  size_y: number;
  size_z: number;
  subdivisions: number;
}

export interface CylinderParameters {
  radius: number;
  height: number;
  radial_segments: number;
  height_segments: number;
}

export interface SphereParameters {
  radius: number;
  rings: number;
  sectors: number;
}

export type StudioParameters = BoxParameters | CylinderParameters | SphereParameters;

export interface HealthResponse {
  status: string;
  service: string;
  version: string;
  offline: boolean;
  step_available: boolean;
  /**
   * True when the backend can compile and measure a design turn (ADR-0009).
   * Absent on older backends; the UI treats "absent" as "not available"
   * rather than assuming the feature exists.
   */
  design_available?: boolean;
  integrations: ReadonlyArray<string>;
  providers: ReadonlyArray<string>;
}

export interface IntegrationRecord {
  id: string;
  name: string;
  category: IntegrationCategory;
  platforms: ReadonlyArray<string>;
  status: IntegrationStatus;
  capabilities: ReadonlyArray<string>;
  notes: string;
}

export interface IntegrationsResponse {
  integrations: ReadonlyArray<IntegrationRecord>;
}

export interface ProviderRecord {
  id: string;
  name: string;
  configured: boolean;
  notes: string;
}

export interface ProvidersResponse {
  providers: ReadonlyArray<ProviderRecord>;
}

export interface ProviderCredentialStatus {
  provider: CredentialProviderId;
  env_var: string;
  configured: boolean;
  source: CredentialSource;
}

export interface StudioSettingsResponse {
  credentials: ReadonlyArray<ProviderCredentialStatus>;
  providers: ReadonlyArray<ProviderRecord>;
  offline: boolean;
}

export interface UpdateProviderKeyRequest {
  provider: CredentialProviderId;
  api_key: string | null;
}

export interface PreviewRequest {
  kind: PrimitiveKind;
  parameters: StudioParameters;
}

export interface PreviewResponse {
  positions: ReadonlyArray<number>;
  normals: ReadonlyArray<number>;
  indices: ReadonlyArray<number>;
  bounds: BBox;
  engine: string;
  warnings: ReadonlyArray<string>;
}

export interface ExportRequest {
  kind: PrimitiveKind;
  parameters: StudioParameters;
  format: ExportFormat;
}

export interface HandoffRequest {
  integration_id: string;
  kind: PrimitiveKind;
  parameters: StudioParameters;
  format: ExportFormat;
}

// ``POST /api/studio/export`` and ``POST /api/studio/handoff`` do not
// return JSON. They stream the artefact bytes as the response body with a
// ``content-disposition: attachment`` header (plus ``x-studio-handoff`` for
// handoff). The client unwraps that single response into this descriptor
// so the panel can save the blob directly and report the real byte count.
// There is no separate artefact download endpoint.
export interface ExportArtifact {
  blob: Blob;
  filename: string;
  bytes: number;
  format: ExportFormat;
  // The registry row the artefact was rendered for, or null for a direct
  // export that targets no integration.
  integrationId: string | null;
}

export interface ChatRequest {
  provider: ProviderId;
  model: string | null;
  prompt: string;
  allow_remote: boolean;
  /**
   * The CAD target that should build the design of this turn (ADR-0011).
   * `null` means the operator has not chosen one yet; the backend then
   * replies with the provider-choice prompt and builds nothing.
   */
  cad_provider?: CadProviderId | null;
}

export interface ChatResponse {
  text: string;
  provider: ProviderId;
  model: string | null;
  offline: boolean;
  /**
   * The compiled, measured design of this turn, when the turn produced one.
   * `null`/absent is the honest "no design was built" signal (ADR-0009 §6).
   */
  design?: DesignResult | null;
}

export interface ErrorResponse {
  code: string;
  message: string;
  details: ReadonlyArray<string>;
}

export interface StudioConnection {
  baseUrl: string;
  token: string;
  mode: 'desktop' | 'browser-manual';
}

// Local preview descriptor. Identical shape to PreviewResponse, plus a
// `source` flag and the parameters used to generate it so the inspector can
// show a server-vs-local diff.
export interface StudioLocalPreview {
  kind: PrimitiveKind;
  parameters: StudioParameters;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  bounds: BBox;
  source: 'local' | 'server';
  generatedAt: number;
  engine: string;
  warnings: ReadonlyArray<string>;
}

// --- Design turns (ADR-0009) --------------------------------------------------
//
// These mirror the frozen backend schemas in
// `src/kala_agent/studio/schemas.py` and the design-intent models in
// `src/kala_agent/studio/design/intent.py`. Every number here is either
// measured by the selected CAD provider or declared by the operator; the UI
// never computes a design number of its own.

/** The four HLR views the backend can draw. */
export type DesignViewName = 'front' | 'top' | 'right' | 'iso';

/** The allow-listed view names, in the backend's order. */
export const DESIGN_VIEW_NAMES: ReadonlyArray<DesignViewName> = ['front', 'top', 'right', 'iso'];

/** The three section planes the backend can cut. */
export type SectionPlane = 'XY' | 'XZ' | 'YZ';

/** The allow-listed section planes. */
export const SECTION_PLANES: ReadonlyArray<SectionPlane> = ['XY', 'XZ', 'YZ'];

/** Export formats a compiled design can be written in. */
export type DesignExportFormat = 'step' | 'stl';

/** Maximum number of designs the store keeps (least-recently-used evicted). */
export const DESIGN_LRU_CAP = 16;

/** Maximum number of journal events one page may carry (backend limit 500). */
export const JOURNAL_PAGE_LIMIT = 200;

/** Maximum number of journal events the store keeps (oldest dropped first). */
export const SESSION_JOURNAL_CAP = 500;

// --- CAD providers (ADR-0011) --------------------------------------------------

/**
 * The closed set of CAD providers. The backend refuses anything else; the UI
 * never invents a provider and never falls back to one silently.
 */
export type CadProviderId = 'opencascade' | 'freecad' | 'solidedge';

/** The closed set in the backend's fixed order. */
export const CAD_PROVIDER_IDS: ReadonlyArray<CadProviderId> = [
  'opencascade',
  'freecad',
  'solidedge',
];

/**
 * One live provider probe result from `GET /api/studio/providers/cad`.
 * `available` is evidence from the host, not a constant; `reason` explains an
 * unavailable provider and is shown in the selector's tooltip.
 */
export interface CadProviderRecord {
  id: CadProviderId;
  name: string;
  available: boolean;
  version: string | null;
  reason: string | null;
  measured_by: string;
  mode: string;
}

export interface CadProvidersResponse {
  providers: ReadonlyArray<CadProviderRecord>;
}

// --- Design intent (request) --------------------------------------------------

export interface LbracketTemplateCall {
  template: 'l_bracket';
  width: number;
  depth: number;
  height: number;
  wall: number;
  bore_d?: number;
  hole_d?: number;
  hole_count?: 0 | 2 | 4;
  edge?: number;
}

export interface SteppedShaftTemplateCall {
  template: 'stepped_shaft';
  segments: ReadonlyArray<{ diameter: number; length: number }>;
}

export interface HousingTemplateCall {
  template: 'housing_envelope';
  width: number;
  depth: number;
  height: number;
  wall: number;
}

export type DesignTemplateCall =
  | LbracketTemplateCall
  | SteppedShaftTemplateCall
  | HousingTemplateCall;

/**
 * A raw construction script. The frontend never builds one; it only carries
 * an operator-supplied script to the backend, so the steps stay opaque.
 */
export interface ConstructionScriptRequest {
  summary?: string;
  components: ReadonlyArray<{
    name: string;
    steps: ReadonlyArray<Record<string, unknown>>;
  }>;
  assembly?: ReadonlyArray<Record<string, unknown>>;
}

/**
 * The body of `POST /api/studio/design` (and the worker
 * `POST /internal/v1/design`). Exactly one of `template`/`script` must be
 * present. `provider` is REQUIRED: ADR-0011 has no default provider, so a
 * request without one is refused before any geometry work. `material` and
 * `tolerances` are DECLARED metadata and are labelled as such in the UI; they
 * are never analysed.
 */
export interface DesignIntent {
  template?: DesignTemplateCall;
  script?: ConstructionScriptRequest;
  material?: string | null;
  tolerances?: ReadonlyArray<string>;
  label?: string | null;
}

/** A design intent plus the CAD provider that must build it. */
export interface DesignRequest extends DesignIntent {
  provider: CadProviderId;
}

// --- Design result (response) -------------------------------------------------

export interface DesignBounds {
  min: [number, number, number];
  max: [number, number, number];
}

/** A check row's declared-vs-measured value. */
export type CheckValue = boolean | number | ReadonlyArray<number>;

/**
 * `passed`/`failed` are real verdicts; `not_reported` means the provider did
 * not report the value, so the UI prints an em dash. A not-reported row is
 * neither a pass nor a failure.
 */
export type DesignCheckStatus = 'passed' | 'failed' | 'not_reported';

/**
 * Every provider-measured property of one design. A provider that does not
 * report a property leaves it `null`; the UI prints "—" and never substitutes
 * a value of its own (ADR-0011 §4).
 */
export interface DesignMeasured {
  volume_mm3: number | null;
  surface_area_mm2: number | null;
  centroid_mm: [number, number, number] | null;
  bounds: DesignBounds | null;
  size_mm: [number, number, number] | null;
  valid: boolean | null;
  solid_count: number | null;
  mass_g: number | null;
}

export interface DesignCheck {
  id: string;
  label: string;
  expected: CheckValue;
  measured: CheckValue | null;
  tolerance: number;
  unit: string;
  passed: boolean;
  status: DesignCheckStatus;
}

/**
 * The display view imported from the provider's exact STEP bytes. The
 * OpenCascade import is always the viewer's source, including for the opt-in
 * OpenCascade builder; this mesh is never a re-compile of the design.
 */
export interface DesignView {
  positions: ReadonlyArray<number>;
  normals: ReadonlyArray<number>;
  indices: ReadonlyArray<number>;
  vertex_count: number;
  triangle_count: number;
  deflection_mm: number;
  engine: string;
  /** The imported B-Rep's optimal box, not the tessellation's extents. */
  bounds: DesignBounds;
  solids: number;
  warnings: ReadonlyArray<string>;
}

export interface DesignArtifacts {
  /** The canonical construction-script digest (not the design id). */
  script_sha256: string;
  /** The provider's STEP export digest. */
  step_sha256: string | null;
  /** The provider's native file digest (e.g. FCStd), when it has one. */
  native_sha256: string | null;
  /** The native file format, e.g. `fcstd`; null when there is no native file. */
  native_kind: string | null;
}

export interface DesignResult {
  design_id: string;
  label: string;
  /** The provider that built and measured the part. */
  provider: CadProviderId;
  /** The provider's reported version, when it reports one. */
  provider_version: string | null;
  /** The provider identity printed beside its measurements, e.g. `FreeCAD`. */
  measured_by: string;
  script: Record<string, unknown>;
  measured: DesignMeasured;
  checks: ReadonlyArray<DesignCheck>;
  verified: boolean;
  view: DesignView;
  artifacts: DesignArtifacts;
  journal_head: string | null;
  material: string | null;
  tolerances: ReadonlyArray<string>;
  declared_envelope: DesignBounds | null;
  cached: boolean;
  warnings: ReadonlyArray<string>;
  component_count: number;
  fused: boolean;
}

// --- Drawing (HLR polylines) --------------------------------------------------

export interface DrawingPolyline {
  points: ReadonlyArray<[number, number]>;
  closed: boolean;
  point_count: number;
}

export interface DrawingView {
  name: DesignViewName;
  visible: ReadonlyArray<DrawingPolyline>;
  hidden: ReadonlyArray<DrawingPolyline>;
  truncated: boolean;
  point_count: number;
  polyline_count: number;
}

export interface DrawingResponse {
  design_id: string;
  views: ReadonlyArray<DrawingView>;
  chord_tolerance_mm: number;
  max_points: number;
  point_count: number;
}

// --- Section (closed loops) ----------------------------------------------------

export interface SectionBounds2D {
  min_u: number;
  min_v: number;
  max_u: number;
  max_v: number;
  width: number;
  height: number;
  center_u: number;
  center_v: number;
}

export interface SectionLoop {
  points: ReadonlyArray<[number, number]>;
  area_mm2: number;
  area_exact: boolean;
  outer: boolean;
  bounds: SectionBounds2D;
  centroid: [number, number];
  point_count: number;
}

export interface SectionResponse {
  design_id: string;
  plane: SectionPlane;
  offset_mm: number;
  loops: ReadonlyArray<SectionLoop>;
  area_mm2: number;
  area_exact: boolean;
  chord_tolerance_mm: number;
  truncated: boolean;
  outer_loop_count: number;
  inner_loop_count: number;
  point_count: number;
}

// --- Studio session journal ----------------------------------------------------

export interface JournalEvent {
  sequence: number;
  event_type: string;
  aggregate_id: string;
  payload: Record<string, unknown> | null;
  digest: string;
  occurred_at: string;
  previous_event_digest: string;
  event_digest: string;
}

export interface JournalPage {
  events: ReadonlyArray<JournalEvent>;
  next_after: number;
  has_more: boolean;
  limit: number;
}

export interface JournalVerify {
  ok: boolean;
  event_count: number;
  head_digest: string | null;
  first_bad_sequence: number | null;
}

// --- Design export receipt ----------------------------------------------------

/**
 * The bytes of a compiled design's export plus the digests the backend
 * reported. `digest` is the SHA-256 of the returned bytes; it is null only
 * when the backend sent no `x-studio-digest` header.
 */
export interface DesignExportArtifact extends ExportArtifact {
  designId: string;
  digest: string | null;
  artifactKind: DesignExportFormat;
}
