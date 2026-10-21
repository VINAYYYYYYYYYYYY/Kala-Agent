// HTTP client for the studio API.
//
// All requests go to the same loopback origin the backend serves. We never
// follow a redirect that could change method or host, never log the bearer
// token, and never persist it. Errors carry their backend `code` and
// `message` so the UI can render them verbatim.
//
// Every request takes an `AbortSignal`; the preview controller, the boot
// gate, and the chat panel each manage their own controllers so a
// cancellation propagates without leaking effects.

import {
  ChatRequest,
  ChatResponse,
  DesignArtifactDownload,
  DesignArtifactKind,
  DesignExportFormat,
  DesignExportArtifact,
  DesignRequest,
  DesignResult,
  DesignViewName,
  DrawingResponse,
  ExportArtifact,
  ExportFormat,
  ExportRequest,
  HealthResponse,
  HandoffRequest,
  IntegrationsResponse,
  CadProvidersResponse,
  JournalPage,
  JournalVerify,
  PreviewRequest,
  PreviewResponse,
  ProviderModelCatalog,
  ProvidersResponse,
  SectionPlane,
  SectionResponse,
  StudioConnection,
  StudioSettingsResponse,
  UpdateProviderKeyRequest,
} from '../types/studio';
import type { ErrorResponse } from '../types/studio';
import {
  parseCadProviders,
  parseChatResponse,
  parseDesignResult,
  parseDrawingResponse,
  parseJournalPage,
  parseJournalVerify,
  parseProviderModelCatalog,
  parseSectionResponse,
  requireCadProviderId,
  requireDesignExportFormat,
  StudioValidationError,
} from './validators';
import { dispatch, getState } from '../state/store';

const REDIRECT_MODE: RequestRedirect = 'error';

export class StudioApiError extends Error {
  readonly code: string;
  readonly status: number;
  readonly details: ReadonlyArray<string>;

  constructor(status: number, body: ErrorResponse) {
    super(body.message);
    this.name = 'StudioApiError';
    this.code = body.code;
    this.status = status;
    this.details = body.details;
  }
}

export class StudioNetworkError extends Error {
  readonly cause: unknown;

  constructor(message: string, cause: unknown) {
    super(message);
    this.name = 'StudioNetworkError';
    this.cause = cause;
  }
}

export class StudioAuthError extends StudioApiError {
  constructor(status: number, body: ErrorResponse) {
    super(status, body);
    this.name = 'StudioAuthError';
  }
}

export class StudioAbortError extends Error {
  constructor() {
    super('request aborted');
    this.name = 'StudioAbortError';
  }
}

/**
 * The request exceeded its own budget and was aborted by the client.
 * Distinct from a server error: the backend may still be working.
 */
export class StudioTimeoutError extends Error {
  readonly path: string;

  constructor(path: string) {
    super(`request to ${path} timed out`);
    this.name = 'StudioTimeoutError';
    this.path = path;
  }
}

// Bounds for the design-turn routes. A compile, a mesh, and an export are
// slow; a journal page and a verification are not. Every request is
// abandoned when its budget runs out so a wedged backend can never pin the
// UI on a spinner forever.
export const DESIGN_COMPILE_TIMEOUT_MS = 60_000;
export const DESIGN_READ_TIMEOUT_MS = 20_000;
export const DESIGN_EXPORT_TIMEOUT_MS = 60_000;
export const JOURNAL_READ_TIMEOUT_MS = 10_000;
/**
 * The model catalog is a small JSON document, but a cold provider can be
 * slow. The budget is generous enough for a cold OpenRouter list and short
 * enough that a wedged backend cannot pin the picker.
 */
export const MODEL_CATALOG_TIMEOUT_MS = 20_000;

/**
 * The chat turn's own client deadline.
 *
 * It is NOT a limit on how long a turn may take. The backend publishes its
 * own worst-case turn budget in `/health` (`design_turn_budget_seconds`,
 * computed from the real model and CAD-build deadlines); the client adds a
 * five-minute margin. When an older backend reports no budget, the documented
 * 70-minute fallback applies.
 *
 * It exists only to unstick a wedged backend: without it a half-dead
 * backend pins the UI on "Awaiting response…" forever, and the operator has
 * no reason to know they should press Cancel.
 */
export const CHAT_TURN_DEADLINE_MARGIN_MS = 5 * 60_000;
export const FALLBACK_CHAT_TURN_TIMEOUT_MS = 70 * 60_000;
/** Compatibility alias: the fallback deadline of an older backend. */
export const CHAT_TURN_TIMEOUT_MS = FALLBACK_CHAT_TURN_TIMEOUT_MS;

/**
 * The client deadline for one chat turn, from the backend's reported budget.
 *
 * `budgetSeconds` is `/health`'s `design_turn_budget_seconds`; a missing,
 * non-finite or non-positive value falls back to
 * [`FALLBACK_CHAT_TURN_TIMEOUT_MS`].
 */
export function chatTurnTimeoutMs(budgetSeconds: number | null | undefined): number {
  if (
    typeof budgetSeconds !== 'number' ||
    !Number.isFinite(budgetSeconds) ||
    budgetSeconds <= 0
  ) {
    return FALLBACK_CHAT_TURN_TIMEOUT_MS;
  }
  return budgetSeconds * 1_000 + CHAT_TURN_DEADLINE_MARGIN_MS;
}

/** The backend's own page limit for `/journal`. */
export const JOURNAL_LIMIT_MAX = 500;
/** The page size the poller asks for. */
export const JOURNAL_LIMIT_DEFAULT = 200;

interface BoundedRequest {
  signal: AbortSignal;
  timedOut(): boolean;
  dispose(): void;
}

/**
 * Compose the caller's abort signal with a timeout. The returned signal
 * aborts when either the caller cancels or the budget expires; `timedOut()`
 * tells the two apart so an expired budget is reported as a timeout rather
 * than as a cancellation.
 */
function boundedSignal(external: AbortSignal | null, timeoutMs: number): BoundedRequest {
  const controller = new AbortController();
  let expired = false;
  const timer = setTimeout(() => {
    expired = true;
    controller.abort();
  }, timeoutMs);
  const onExternalAbort = (): void => controller.abort();
  if (external) {
    if (external.aborted) controller.abort();
    else external.addEventListener('abort', onExternalAbort);
  }
  return {
    signal: controller.signal,
    timedOut: () => expired,
    dispose: () => {
      clearTimeout(timer);
      if (external) external.removeEventListener('abort', onExternalAbort);
    },
  };
}

async function readJsonError(response: Response): Promise<ErrorResponse> {
  const text = await response.text();
  if (!text) {
    return { code: 'empty_body', message: response.statusText || 'unknown error', details: [] };
  }
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed && typeof parsed === 'object') {
      const obj = parsed as { code?: unknown; message?: unknown; details?: unknown };
      if (typeof obj.code === 'string' && typeof obj.message === 'string') {
        const details = Array.isArray(obj.details)
          ? (obj.details.filter((x): x is string => typeof x === 'string'))
          : [];
        return { code: obj.code, message: obj.message, details };
      }
    }
  } catch {
    // fall through
  }
  return { code: 'malformed_error', message: text.slice(0, 256), details: [] };
}

function buildInit(method: 'GET' | 'POST', body: unknown, signal: AbortSignal | null): RequestInit {
  const init: RequestInit = {
    method,
    headers: {
      Accept: 'application/json',
    },
    redirect: REDIRECT_MODE,
    cache: 'no-store',
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  };
  if (body !== null) {
    (init.headers as Record<string, string>)['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  if (signal) init.signal = signal;
  return init;
}

function authorizationHeader(
  method: 'GET' | 'POST',
  token: string,
  authenticated: boolean,
): Record<string, string> {
  // POST routes always carry the bearer. The design-turn GET routes
  // (`/design/{id}/drawing`, `/design/{id}/section`, `/journal`,
  // `/journal/verify`) are bearer-guarded too, so they ask for it
  // explicitly; the older public GET routes keep their old behaviour.
  if (method === 'GET' && !authenticated) return {};
  return { Authorization: `Bearer ${token}` };
}

interface RequestOptions {
  /** Send the bearer on a GET route (the design-turn routes require it). */
  authenticated?: boolean;
}

async function sendRequest<TResponse>(
  connection: StudioConnection,
  method: 'GET' | 'POST',
  path: string,
  body: unknown,
  signal: AbortSignal | null,
  options: RequestOptions = {},
): Promise<TResponse> {
  // Reject pre-aborted callers without touching the network. The browser
  // would surface an `AbortError` from `fetch` but the contract here is
  // `StudioAbortError` so callers do not have to special-case
  // DOMException.
  if (signal?.aborted) throw new StudioAbortError();
  const url = new URL(path, connection.baseUrl).toString();
  const baseHeaders: Record<string, string> = {
    Accept: 'application/json',
    ...authorizationHeader(method, connection.token, options.authenticated === true),
  };
  if (body !== null) baseHeaders['Content-Type'] = 'application/json';
  const init = buildInit(method, body, signal);
  (init.headers as Record<string, string>) = { ...(init.headers as Record<string, string>), ...baseHeaders };
  let response: Response;
  try {
    response = await fetch(url, init);
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') {
      throw new StudioAbortError();
    }
    throw new StudioNetworkError(`network request to ${path} failed`, cause);
  }
  if (!response.ok) {
    const parsed = await readJsonError(response);
    if (response.status === 401 || response.status === 403) {
      throw new StudioAuthError(response.status, parsed);
    }
    throw new StudioApiError(response.status, parsed);
  }
  if (response.status === 204) {
    return undefined as TResponse;
  }
  try {
    return (await response.json()) as TResponse;
  } catch (cause) {
    throw new StudioNetworkError(`failed to parse response from ${path}`, cause);
  }
}

/**
 * The health probe. The backend's `design_available` flag is the only
 * signal the UI has that this backend carries the design-turn routes
 * (`/design`, `/journal`, `/journal/verify`). The flag is recorded in the
 * store so the journal poller only runs against a backend that has them;
 * an older backend answers the probe without the flag and is left alone.
 */
export async function getHealth(
  connection: StudioConnection,
  signal: AbortSignal | null = null,
): Promise<HealthResponse> {
  const raw = (await sendRequest<unknown>(
    connection,
    'GET',
    '/api/studio/health',
    null,
    signal,
  )) as HealthResponse;
  const designAvailable = readDesignAvailable(raw);
  const designTurnBudgetSeconds = readDesignTurnBudgetSeconds(raw);
  dispatch({ type: 'server/capabilities', designAvailable, designTurnBudgetSeconds });
  return raw;
}

function readDesignTurnBudgetSeconds(raw: unknown): number | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const value = (raw as { design_turn_budget_seconds?: unknown })
    .design_turn_budget_seconds;
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return null;
  return value;
}

function readDesignAvailable(raw: unknown): boolean | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const value = (raw as { design_available?: unknown }).design_available;
  return typeof value === 'boolean' ? value : null;
}

export function getIntegrations(connection: StudioConnection, signal: AbortSignal | null = null): Promise<IntegrationsResponse> {
  return sendRequest<IntegrationsResponse>(connection, 'GET', '/api/studio/integrations', null, signal);
}

export function getProviders(connection: StudioConnection, signal: AbortSignal | null = null): Promise<ProvidersResponse> {
  return sendRequest<ProvidersResponse>(connection, 'GET', '/api/studio/providers', null, signal);
}

/**
 * The CAD provider probe (ADR-0011). This is a design-turn route, so it
 * carries the studio bearer and its payload is validated like every other
 * response: an unavailable provider must arrive with its real reason, never
 * be inferred from a missing row.
 */
export async function getCadProviders(
  connection: StudioConnection,
  signal: AbortSignal | null = null,
): Promise<CadProvidersResponse> {
  const path = '/api/studio/providers/cad';
  const bounded = boundedSignal(signal, DESIGN_READ_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', path, null, bounded.signal, {
      authenticated: true,
    });
    return parseCadProviders(raw, 'providers/cad');
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

export function getSettings(connection: StudioConnection, signal: AbortSignal | null = null): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'GET', '/api/studio/settings', null, signal);
}

/**
 * The model catalog of one provider (`GET /api/studio/providers/{id}/models`).
 *
 * Listing the models of a remote provider is remote egress like a chat turn,
 * so the call carries the operator's consent stamp in `allow_remote`. The
 * UI NEVER sets the flag itself: it passes the stamp the operator ticked,
 * and the backend refuses the request (403) when the flag is absent. A
 * refusal therefore arrives as a `StudioAuthError` and the picker shows the
 * consent-required state.
 *
 * The response is validated before it is returned, so a malformed catalog is
 * refused rather than rendered as a half-populated list.
 */
export async function getProviderModels(
  connection: StudioConnection,
  providerId: string,
  allowRemote: boolean,
  signal: AbortSignal | null = null,
): Promise<ProviderModelCatalog> {
  const provider = encodeURIComponent(providerId);
  const path = `/api/studio/providers/${provider}/models?allow_remote=${allowRemote ? 'true' : 'false'}`;
  const bounded = boundedSignal(signal, MODEL_CATALOG_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', path, null, bounded.signal, {
      authenticated: true,
    });
    return parseProviderModelCatalog(raw, `providers/${providerId}/models`);
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

export function postSettings(
  connection: StudioConnection,
  payload: UpdateProviderKeyRequest,
  signal: AbortSignal | null = null,
): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'POST', '/api/studio/settings', payload, signal);
}

export function postChat(connection: StudioConnection, payload: ChatRequest, signal: AbortSignal | null = null): Promise<ChatResponse> {
  return sendChat(connection, payload, signal);
}

/**
 * The chat turn is the only path by which a design can arrive without an
 * explicit `/design` call, so the client is the single place that knows a
 * reply carried one. The validated design is handed to the store; the chat
 * thread itself is untouched (the panel still renders the reply text).
 */
async function sendChat(
  connection: StudioConnection,
  payload: ChatRequest,
  signal: AbortSignal | null,
): Promise<ChatResponse> {
  const path = '/api/studio/chat';
  // The turn's own budget comes from the backend's reported worst case (see
  // `chatTurnTimeoutMs`): an expired budget is reported as a timeout, never
  // as a user cancellation.
  const bounded = boundedSignal(
    signal,
    chatTurnTimeoutMs(getState().server.designTurnBudgetSeconds),
  );
  try {
    const raw = await sendRequest<unknown>(connection, 'POST', path, payload, bounded.signal);
    const response = parseChatResponse(raw);
    if (response.design) {
      dispatch({ type: 'design/received', design: response.design });
    }
    return response;
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

export function postPreview(connection: StudioConnection, payload: PreviewRequest, signal: AbortSignal | null = null): Promise<PreviewResponse> {
  return sendRequest<PreviewResponse>(connection, 'POST', '/api/studio/preview', payload, signal);
}

// --- Design turns (ADR-0009) --------------------------------------------------

const DESIGN_ID_PATTERN = /^[0-9a-f]{64}$/;
const SECTION_PLANE_VALUES: ReadonlyArray<SectionPlane> = ['XY', 'XZ', 'YZ'];
/** The backend's own bound on a section offset, in millimetres. */
export const MAX_SECTION_OFFSET_MM = 10_000;

/**
 * A design id is a sha256 digest, so it is safe to put in a path. Anything
 * else is refused before it can become a path segment.
 */
function requireDesignId(value: string, path: string): string {
  if (!DESIGN_ID_PATTERN.test(value)) {
    throw new StudioValidationError(path, 'expected a 64-character lowercase hex sha256 design id');
  }
  return value;
}

function translateFailure(cause: unknown, path: string, bounded: BoundedRequest): unknown {
  if (bounded.timedOut() && cause instanceof StudioAbortError) {
    return new StudioTimeoutError(path);
  }
  return cause;
}

/**
 * Compile, measure, and check a design intent with a named CAD provider.
 * ADR-0011 has no default provider: the `provider` field is part of the
 * request type and is checked before the network call, so an older caller
 * cannot silently ask the backend to pick one.
 *
 * The response is validated against the frozen `DesignResult` contract
 * before it is returned; a malformed payload is refused rather than
 * rendered. A compiled design is handed to the store the same way a chat
 * turn's design is: it becomes the active one.
 */
export async function postDesign(
  connection: StudioConnection,
  request: DesignRequest,
  signal: AbortSignal | null = null,
): Promise<DesignResult> {
  const path = '/api/studio/design';
  if (typeof request !== 'object' || request === null) {
    throw new StudioValidationError('design', 'expected a design request');
  }
  requireCadProviderId(request.provider, 'provider');
  const bounded = boundedSignal(signal, DESIGN_COMPILE_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'POST', path, request, bounded.signal);
    const design = parseDesignResult(raw, 'design');
    dispatch({ type: 'design/received', design });
    return design;
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/**
 * The HLR drawing of one stored design. `views` is the allow-listed view
 * set; an empty list asks the backend for all four views.
 */
export async function getDesignDrawing(
  connection: StudioConnection,
  designId: string,
  views: ReadonlyArray<DesignViewName> = [],
  signal: AbortSignal | null = null,
): Promise<DrawingResponse> {
  requireDesignId(designId, 'design_id');
  const path = `/api/studio/design/${designId}/drawing`;
  const query = views.length > 0 ? `?views=${encodeURIComponent(views.join(','))}` : '';
  const bounded = boundedSignal(signal, DESIGN_READ_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', `${path}${query}`, null, bounded.signal, {
      authenticated: true,
    });
    return parseDrawingResponse(raw, 'drawing');
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/**
 * The section cut of one stored design. `offsetMm` is the signed distance
 * of the cutting plane from the origin, in millimetres.
 */
export async function getDesignSection(
  connection: StudioConnection,
  designId: string,
  plane: SectionPlane,
  offsetMm?: number,
  signal: AbortSignal | null = null,
): Promise<SectionResponse> {
  requireDesignId(designId, 'design_id');
  if (!SECTION_PLANE_VALUES.includes(plane)) {
    throw new StudioValidationError('plane', `expected one of ${SECTION_PLANE_VALUES.join(', ')}`);
  }
  if (offsetMm !== undefined && !Number.isFinite(offsetMm)) {
    throw new StudioValidationError('offset', 'expected a finite offset in millimetres');
  }
  if (offsetMm !== undefined && Math.abs(offsetMm) > MAX_SECTION_OFFSET_MM) {
    throw new StudioValidationError(
      'offset',
      `expected an offset within +/-${MAX_SECTION_OFFSET_MM} mm`,
    );
  }
  const path = `/api/studio/design/${designId}/section`;
  const query = offsetMm === undefined ? `?plane=${plane}` : `?plane=${plane}&offset=${offsetMm}`;
  const bounded = boundedSignal(signal, DESIGN_READ_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', `${path}${query}`, null, bounded.signal, {
      authenticated: true,
    });
    return parseSectionResponse(raw, 'section');
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/** A bounded page of the Studio session journal, newest sequence last. */
export async function getJournal(
  connection: StudioConnection,
  after = 0,
  limit = JOURNAL_LIMIT_DEFAULT,
  signal: AbortSignal | null = null,
): Promise<JournalPage> {
  if (!Number.isInteger(after) || after < 0) {
    throw new StudioValidationError('after', 'expected a non-negative integer sequence');
  }
  const boundedLimit = Math.min(Math.max(1, Math.floor(limit)), JOURNAL_LIMIT_MAX);
  const path = `/api/studio/journal?after=${after}&limit=${boundedLimit}`;
  const bounded = boundedSignal(signal, JOURNAL_READ_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', path, null, bounded.signal, {
      authenticated: true,
    });
    return parseJournalPage(raw, 'journal');
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/**
 * The hash-chain verdict. `ok: true` is the only thing that may light the
 * HASH-CHAIN OK stamp; the UI never infers it.
 */
export async function getJournalVerify(
  connection: StudioConnection,
  signal: AbortSignal | null = null,
): Promise<JournalVerify> {
  const path = '/api/studio/journal/verify';
  const bounded = boundedSignal(signal, JOURNAL_READ_TIMEOUT_MS);
  try {
    const raw = await sendRequest<unknown>(connection, 'GET', path, null, bounded.signal, {
      authenticated: true,
    });
    return parseJournalVerify(raw, 'journal/verify');
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/**
 * Export a compiled design. The response body IS the artefact; the backend
 * reports the sha256 of those bytes in `x-studio-digest`.
 */
export async function exportDesign(
  connection: StudioConnection,
  designId: string,
  format: DesignExportFormat,
  signal: AbortSignal | null = null,
): Promise<DesignExportArtifact> {
  requireDesignId(designId, 'design_id');
  const exportFormat = requireDesignExportFormat(format, 'format');
  const path = '/api/studio/export';
  const bounded = boundedSignal(signal, DESIGN_EXPORT_TIMEOUT_MS);
  try {
    const url = new URL(path, connection.baseUrl).toString();
    const init: RequestInit = {
      method: 'POST',
      headers: {
        Accept: 'model/stl, application/step, application/octet-stream',
        'Content-Type': 'application/json',
        Authorization: `Bearer ${connection.token}`,
      },
      body: JSON.stringify({ design_id: designId, format: exportFormat }),
      redirect: REDIRECT_MODE,
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      signal: bounded.signal,
    };
    let response: Response;
    try {
      response = await fetch(url, init);
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === 'AbortError') {
        throw new StudioAbortError();
      }
      throw new StudioNetworkError(`network request to ${path} failed`, cause);
    }
    if (!response.ok) {
      const parsed = await readJsonError(response);
      if (response.status === 401 || response.status === 403) {
        throw new StudioAuthError(response.status, parsed);
      }
      throw new StudioApiError(response.status, parsed);
    }
    let blob: Blob;
    try {
      blob = await response.blob();
    } catch (cause) {
      throw new StudioNetworkError(`failed to read response bytes from ${path}`, cause);
    }
    const reported = response.headers.get('x-studio-design-id');
    return {
      blob,
      filename: sanitizeDispositionFilename(
        response.headers.get('content-disposition'),
        `kala-studio-${designId.slice(0, 8)}.${exportFormat}`,
      ),
      bytes: blob.size,
      format: exportFormat,
      integrationId: null,
      designId: reported && DESIGN_ID_PATTERN.test(reported) ? reported : designId,
      digest: response.headers.get('x-studio-digest'),
      artifactKind: exportFormat,
    };
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/**
 * Download one stored artifact of a design.
 *
 * The response body IS the stored file; the backend verified its SHA-256
 * against the journalled digest before sending, and reports that digest in
 * `x-studio-digest`. The filename comes from `content-disposition` and is
 * sanitized like the export receipt.
 */
export async function getDesignArtifact(
  connection: StudioConnection,
  designId: string,
  kind: DesignArtifactKind,
  signal: AbortSignal | null = null,
): Promise<DesignArtifactDownload> {
  requireDesignId(designId, 'design_id');
  const artifactKind = requireDesignArtifactKind(kind, 'kind');
  const path = `/api/studio/design/${designId}/artifact/${artifactKind}`;
  const bounded = boundedSignal(signal, DESIGN_EXPORT_TIMEOUT_MS);
  try {
    const url = new URL(path, connection.baseUrl).toString();
    const init: RequestInit = {
      method: 'GET',
      headers: {
        Accept: 'application/json, application/step, model/stl, application/octet-stream',
        Authorization: `Bearer ${connection.token}`,
      },
      redirect: REDIRECT_MODE,
      cache: 'no-store',
      credentials: 'omit',
      referrerPolicy: 'no-referrer',
      signal: bounded.signal,
    };
    let response: Response;
    try {
      response = await fetch(url, init);
    } catch (cause) {
      if (cause instanceof DOMException && cause.name === 'AbortError') {
        throw new StudioAbortError();
      }
      throw new StudioNetworkError(`network request to ${path} failed`, cause);
    }
    if (!response.ok) {
      const parsed = await readJsonError(response);
      if (response.status === 401 || response.status === 403) {
        throw new StudioAuthError(response.status, parsed);
      }
      throw new StudioApiError(response.status, parsed);
    }
    let blob: Blob;
    try {
      blob = await response.blob();
    } catch (cause) {
      throw new StudioNetworkError(`failed to read response bytes from ${path}`, cause);
    }
    const reported = response.headers.get('x-studio-artifact-kind');
    return {
      blob,
      filename: sanitizeDispositionFilename(
        response.headers.get('content-disposition'),
        `kala-studio-${designId.slice(0, 8)}.${artifactExtension(artifactKind)}`,
      ),
      bytes: blob.size,
      kind: reported && isArtifactKind(reported) ? reported : artifactKind,
      designId,
      digest: response.headers.get('x-studio-digest'),
    };
  } catch (cause) {
    throw translateFailure(cause, path, bounded);
  } finally {
    bounded.dispose();
  }
}

/** The artifact kinds the download route serves, in the backend's order. */
export const DESIGN_ARTIFACT_KINDS: ReadonlyArray<DesignArtifactKind> = [
  'script',
  'step',
  'fcstd',
  'stl',
];

function isArtifactKind(value: string): value is DesignArtifactKind {
  return (DESIGN_ARTIFACT_KINDS as ReadonlyArray<string>).includes(value);
}

function requireDesignArtifactKind(value: string, field: string): DesignArtifactKind {
  if (!isArtifactKind(value)) {
    throw new StudioValidationError(
      field,
      `expected one of ${DESIGN_ARTIFACT_KINDS.join(', ')}`,
    );
  }
  return value;
}

/** The default filename extension of one artifact kind. */
export function artifactExtension(kind: DesignArtifactKind): string {
  if (kind === 'script') return 'json';
  if (kind === 'step') return 'step';
  if (kind === 'stl') return 'stl';
  return 'FCStd';
}

// The export and handoff endpoints answer with the artefact bytes directly: a
// single authenticated POST whose response body IS the file, named by its
// `content-disposition: attachment` header (and annotated by
// `x-studio-handoff` for a handoff).
function extensionFor(format: ExportFormat): string {
  return format === 'stl' ? 'stl' : 'step';
}

// Extract a safe filename from a `Content-Disposition` header. We accept
// the RFC 6266 `filename*=UTF-8''…` form first, then the plain quoted form,
// and always reduce the result to a basename with control characters and
// quotes stripped, so a hostile header cannot traverse paths or smuggle a
// header/extension the operator did not expect.
export function sanitizeDispositionFilename(header: string | null, fallback: string): string {
  if (!header) return fallback;
  let candidate = '';
  const extended = /filename\*\s*=\s*([^;]+)/i.exec(header);
  if (extended) {
    let value = extended[1]!.trim().replace(/^["']|["']$/g, '');
    const encoded = value.split("''");
    if (encoded.length === 2) value = encoded[1]!;
    try {
      candidate = decodeURIComponent(value);
    } catch {
      candidate = value;
    }
  }
  if (!candidate) {
    const plain = /filename\s*=\s*(?:"([^"]*)"|([^;]+))/i.exec(header);
    if (plain) candidate = (plain[1] ?? plain[2] ?? '').trim();
  }
  const base = candidate.split(/[\\/]/).pop() ?? '';
  const cleaned = base
    .replace(/[\u0000-\u001f\u007f]/g, '')
    .replace(/["']/g, '')
    .trim();
  return cleaned || fallback;
}

function handoffIntegrationId(response: Response, fallback: string | null): string | null {
  const header = response.headers.get('x-studio-handoff');
  if (!header) return fallback;
  const id = header.split(';')[0]!.trim();
  return id || fallback;
}

async function sendBinaryRequest(
  connection: StudioConnection,
  path: string,
  body: unknown,
  signal: AbortSignal | null,
  format: ExportFormat,
  fallbackFilename: string,
  integrationId: string | null,
): Promise<ExportArtifact> {
  if (signal?.aborted) throw new StudioAbortError();
  const url = new URL(path, connection.baseUrl).toString();
  const init: RequestInit = {
    method: 'POST',
    headers: {
      Accept: 'model/stl, application/step, application/octet-stream, application/json',
      'Content-Type': 'application/json',
      Authorization: `Bearer ${connection.token}`,
    },
    body: JSON.stringify(body),
    redirect: REDIRECT_MODE,
    cache: 'no-store',
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  };
  if (signal) init.signal = signal;
  let response: Response;
  try {
    response = await fetch(url, init);
  } catch (cause) {
    if (cause instanceof DOMException && cause.name === 'AbortError') {
      throw new StudioAbortError();
    }
    throw new StudioNetworkError(`network request to ${path} failed`, cause);
  }
  if (!response.ok) {
    const parsed = await readJsonError(response);
    if (response.status === 401 || response.status === 403) {
      throw new StudioAuthError(response.status, parsed);
    }
    throw new StudioApiError(response.status, parsed);
  }
  let blob: Blob;
  try {
    blob = await response.blob();
  } catch (cause) {
    throw new StudioNetworkError(`failed to read response bytes from ${path}`, cause);
  }
  return {
    blob,
    filename: sanitizeDispositionFilename(response.headers.get('content-disposition'), fallbackFilename),
    bytes: blob.size,
    format,
    integrationId: handoffIntegrationId(response, integrationId),
  };
}

export function postExport(connection: StudioConnection, payload: ExportRequest, signal: AbortSignal | null = null): Promise<ExportArtifact> {
  return sendBinaryRequest(
    connection,
    '/api/studio/export',
    payload,
    signal,
    payload.format,
    `kala-studio-${payload.kind}.${extensionFor(payload.format)}`,
    null,
  );
}

export function postHandoff(connection: StudioConnection, payload: HandoffRequest, signal: AbortSignal | null = null): Promise<ExportArtifact> {
  return sendBinaryRequest(
    connection,
    '/api/studio/handoff',
    payload,
    signal,
    payload.format,
    `${payload.integration_id}.${extensionFor(payload.format)}`,
    payload.integration_id,
  );
}

export type { ExportFormat };
export { StudioValidationError };
