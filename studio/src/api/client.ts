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
  ExportArtifact,
  ExportFormat,
  ExportRequest,
  HealthResponse,
  HandoffRequest,
  IntegrationsResponse,
  PreviewRequest,
  PreviewResponse,
  ProvidersResponse,
  StudioConnection,
  StudioSettingsResponse,
  UpdateProviderKeyRequest,
} from '../types/studio';
import type { ErrorResponse } from '../types/studio';

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

function authorizationHeader(method: 'GET' | 'POST', token: string): Record<string, string> {
  return method === 'GET' ? {} : { Authorization: `Bearer ${token}` };
}

async function sendRequest<TResponse>(
  connection: StudioConnection,
  method: 'GET' | 'POST',
  path: string,
  body: unknown,
  signal: AbortSignal | null,
): Promise<TResponse> {
  // Reject pre-aborted callers without touching the network. The browser
  // would surface an `AbortError` from `fetch` but the contract here is
  // `StudioAbortError` so callers do not have to special-case
  // DOMException.
  if (signal?.aborted) throw new StudioAbortError();
  const url = new URL(path, connection.baseUrl).toString();
  const baseHeaders: Record<string, string> = {
    Accept: 'application/json',
    ...authorizationHeader(method, connection.token),
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

export function getHealth(connection: StudioConnection, signal: AbortSignal | null = null): Promise<HealthResponse> {
  return sendRequest<HealthResponse>(connection, 'GET', '/api/studio/health', null, signal);
}

export function getIntegrations(connection: StudioConnection, signal: AbortSignal | null = null): Promise<IntegrationsResponse> {
  return sendRequest<IntegrationsResponse>(connection, 'GET', '/api/studio/integrations', null, signal);
}

export function getProviders(connection: StudioConnection, signal: AbortSignal | null = null): Promise<ProvidersResponse> {
  return sendRequest<ProvidersResponse>(connection, 'GET', '/api/studio/providers', null, signal);
}

export function getSettings(connection: StudioConnection, signal: AbortSignal | null = null): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'GET', '/api/studio/settings', null, signal);
}

export function postSettings(
  connection: StudioConnection,
  payload: UpdateProviderKeyRequest,
  signal: AbortSignal | null = null,
): Promise<StudioSettingsResponse> {
  return sendRequest<StudioSettingsResponse>(connection, 'POST', '/api/studio/settings', payload, signal);
}

export function postChat(connection: StudioConnection, payload: ChatRequest, signal: AbortSignal | null = null): Promise<ChatResponse> {
  return sendRequest<ChatResponse>(connection, 'POST', '/api/studio/chat', payload, signal);
}

export function postPreview(connection: StudioConnection, payload: PreviewRequest, signal: AbortSignal | null = null): Promise<PreviewResponse> {
  return sendRequest<PreviewResponse>(connection, 'POST', '/api/studio/preview', payload, signal);
}

// The export and handoff endpoints answer with the artefact bytes
// directly: a single authenticated POST whose response body IS the file,
// named by its `content-disposition: attachment` header (and annotated by
// `x-studio-handoff` for a handoff). We decode and save that one response;
// there is no second "download artifact" GET.
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
