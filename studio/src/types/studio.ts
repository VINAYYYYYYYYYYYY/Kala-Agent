// Type definitions matching the backend's frozen Pydantic schemas.
// Source: src/kala_agent/studio/schemas.py and src/kala_agent/studio/geometry/limits.py.
// Field names and shapes are deliberately stable; new fields follow rather than guess.

export type PrimitiveKind = 'box' | 'cylinder' | 'sphere';
export type ExportFormat = 'stl' | 'step';
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
}

export interface ChatResponse {
  text: string;
  provider: ProviderId;
  model: string | null;
  offline: boolean;
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
