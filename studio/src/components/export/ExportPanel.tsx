import type { ReactElement } from 'react';
// Export & handoff panel.
//
// Compact form: pick an integration target, pick a format (capability-
// filtered), download. The available formats come from the integration's
// `capabilities` list so we never offer a button the backend cannot
// satisfy. A single authenticated POST streams the artefact bytes back;
// the panel saves that response directly — there is no second download
// call.
//
// A registry row's `native_api_unavailable` status means only that the
// studio cannot drive that CAD tool through a native automation API. It
// says nothing about neutral-file handoff: the backend renders a neutral
// file for any listed capability, so those rows stay exportable (with an
// honest warning). `not_configured` rows stay disabled.

import { useEffect, useMemo, useState } from 'react';
import {
  postExport,
  postHandoff,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { useUiState } from '../../state/store';
import type { ExportFormat } from '../../api/client';
import type { IntegrationRecord, IntegrationStatus } from '../../types/studio';

const SUPPORTED_FORMATS: ReadonlyArray<ExportFormat> = ['stl', 'step'];

interface ExportResult {
  filename: string;
  bytes: number;
}

export function ExportPanel(): ReactElement {
  const connection = useUiState((s) => s.connection);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const integrations = useUiState((s) => s.server.integrations);
  const cadIntegrations = useMemo(
    () => integrations.filter((i) => i.category === 'cad'),
    [integrations],
  );

  const [targetId, setTargetId] = useState<string>('native');
  const [format, setFormat] = useState<ExportFormat>('stl');
  const [busy, setBusy] = useState(false);
  const [lastResult, setLastResult] = useState<ExportResult | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  const target: TargetDescriptor = useMemo(() => {
    if (targetId === 'native') {
      return {
        id: 'native',
        name: 'Direct export (no handoff)',
        status: 'native',
        capabilities: [...SUPPORTED_FORMATS],
      };
    }
    const integration = cadIntegrations.find((i) => i.id === targetId);
    if (!integration) {
      return {
        id: targetId,
        name: targetId,
        status: 'native_api_unavailable',
        capabilities: [],
      };
    }
    const descriptor: TargetDescriptor = {
      id: integration.id,
      name: integration.name,
      status: integration.status,
      capabilities: filterFormats(integration.capabilities),
    };
    if (integration.notes) descriptor.notes = integration.notes;
    return descriptor;
  }, [targetId, cadIntegrations]);

  // Reset format if the target stops supporting it.
  useEffect(() => {
    if (target.capabilities.length === 0) {
      setFormat('stl');
      return;
    }
    if (!target.capabilities.includes(format)) {
      setFormat(target.capabilities[0]!);
    }
  }, [target, format]);

  const canDownload =
    connection.kind === 'ready' &&
    !busy &&
    target.capabilities.includes(format) &&
    (target.status === 'native' || isHandoffStatus(target.status));

  async function onDownload(): Promise<void> {
    if (!canDownload || connection.kind !== 'ready') return;
    setBusy(true);
    setLastError(null);
    try {
      // Export and handoff both answer with the artefact bytes in the body
      // of this one POST. Save that response and report its real size.
      const artifact = target.id === 'native'
        ? await postExport(connection.connection, { kind: primitive, parameters, format })
        : await postHandoff(connection.connection, {
            integration_id: target.id,
            kind: primitive,
            parameters,
            format,
          });
      saveBlob(artifact.blob, artifact.filename);
      setLastResult({ filename: artifact.filename, bytes: artifact.bytes });
    } catch (cause) {
      setLastError(describeError(cause).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="export-panel" aria-label="Export and handoff">
      <header className="export-panel-header">
        <h2>Export & handoff</h2>
      </header>
      <p className="export-panel-help">
        Renders the active primitive as a neutral artefact (STL, STEP) for the picked target —
        either to finish the build here or to hand the geometry to an external CAD/CAE package
        such as SOLIDWORKS, FreeCAD, or Fusion. Handoff never launches a native tool.
      </p>
      <label className="export-panel-field">
        <span>Target</span>
        <select value={targetId} onChange={(event) => setTargetId(event.target.value)}>
          <option value="native">Direct export (no handoff)</option>
          {cadIntegrations.map((integration) => (
            <option key={integration.id} value={integration.id}>
              {integration.name} ({statusOptionLabel(integration.status)})
            </option>
          ))}
        </select>
      </label>
      <label className="export-panel-field">
        <span>Format</span>
        <select
          value={format}
          onChange={(event) => setFormat(event.target.value as ExportFormat)}
          disabled={target.capabilities.length === 0}
        >
          {SUPPORTED_FORMATS.map((entry) => (
            <option key={entry} value={entry} disabled={!target.capabilities.includes(entry)}>
              {entry.toUpperCase()}{!target.capabilities.includes(entry) ? ' (not supported by target)' : ''}
            </option>
          ))}
        </select>
      </label>
      {target.notes ? <p className="export-panel-notes">{target.notes}</p> : null}
      {target.status === 'native_api_unavailable' ? (
        <p className="export-panel-warning">
          Native automation is not available for {target.name} from Kala Studio. Exporting
          writes a neutral {format.toUpperCase()} file for the operator to import in their
          local {target.name} session; it never launches or drives {target.name}.
        </p>
      ) : null}
      {target.status === 'not_configured' ? (
        <p className="export-panel-warning">
          This integration is not configured on the backend, so no artefact is offered for it.
        </p>
      ) : null}
      <button
        type="button"
        className="export-panel-button"
        onClick={() => void onDownload()}
        disabled={!canDownload}
      >
        {busy ? 'Working…' : `Download ${format.toUpperCase()}`}
      </button>
      {lastResult ? (
        <p className="export-panel-result" role="status">
          Saved <code>{lastResult.filename}</code> · {lastResult.bytes.toLocaleString()} bytes
        </p>
      ) : null}
      {lastError ? <p className="export-panel-error" role="alert">{lastError}</p> : null}
    </section>
  );
}

interface TargetDescriptor {
  id: string;
  name: string;
  status: IntegrationRecord['status'] | 'native';
  capabilities: ReadonlyArray<ExportFormat>;
  notes?: string;
}

function isHandoffStatus(status: TargetDescriptor['status']): boolean {
  // The studio can render a neutral file for any registry row that lists
  // the capability — including `native_api_unavailable`, which only means
  // the studio cannot drive that tool's native automation API.
  return status === 'available_via_neutral_file' || status === 'native_api_unavailable';
}

function statusOptionLabel(status: IntegrationStatus): string {
  if (status === 'available_via_neutral_file') return 'neutral file';
  if (status === 'native_api_unavailable') return 'neutral file (no native automation)';
  return 'not configured';
}

function filterFormats(values: ReadonlyArray<string>): ExportFormat[] {
  const set = new Set<string>(SUPPORTED_FORMATS);
  return values.filter((value): value is ExportFormat => set.has(value));
}

function saveBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = 'noopener';
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}

function describeError(cause: unknown): { message: string; kind: 'cancelled' | 'timeout' | 'network' | 'server' } {
  if (cause instanceof StudioAbortError) return { message: 'cancelled', kind: 'cancelled' };
  if (cause instanceof StudioAuthError) return { message: `auth: ${cause.message} (${cause.status})`, kind: 'server' };
  if (cause instanceof StudioApiError) return { message: `${cause.code}: ${cause.message}`, kind: 'server' };
  if (cause instanceof StudioNetworkError) {
    if (cause.cause instanceof DOMException && cause.cause.name === 'AbortError') {
      return { message: 'cancelled', kind: 'cancelled' };
    }
    return { message: cause.message, kind: 'network' };
  }
  if (cause instanceof Error) return { message: cause.message, kind: 'network' };
  return { message: 'unknown error', kind: 'network' };
}