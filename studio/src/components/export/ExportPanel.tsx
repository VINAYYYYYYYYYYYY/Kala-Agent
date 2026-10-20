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
  exportDesign,
  postExport,
  postHandoff,
  StudioAbortError,
  StudioApiError,
  StudioAuthError,
  StudioNetworkError,
} from '../../api/client';
import { useUiState, appendJournal, selectActiveDesign } from '../../state/store';
import type { ExportFormat } from '../../api/client';
import type {
  DesignExportArtifact,
  DesignExportFormat,
  IntegrationRecord,
  IntegrationStatus,
} from '../../types/studio';
import { CopyButton } from '../journal/CopyButton';
import { measuredByLabel, nativeKindLabel } from '../chat/designCard';

const SUPPORTED_FORMATS: ReadonlyArray<ExportFormat> = ['stl', 'step'];

/** The formats a compiled design can be written in. */
const DESIGN_FORMATS: ReadonlyArray<DesignExportFormat> = ['step', 'stl'];

interface ExportResult {
  filename: string;
  bytes: number;
}

interface ExportPanelProps {
  /** The Options card supplies its own drafted heading; hide the inner one. */
  showHeading?: boolean;
}

export function ExportPanel(props: ExportPanelProps = {}): ReactElement {
  const showHeading = props.showHeading ?? true;
  const connection = useUiState((s) => s.connection);
  const primitive = useUiState((s) => s.primitive);
  const parameters = useUiState((s) => s.parameters);
  const integrations = useUiState((s) => s.server.integrations);
  // The design-turn capability flag of the backend, and the design the
  // operator is looking at. A backend that reports no `design_available`
  // has no design routes at all, so it never offers a design export.
  const designAvailable = useUiState((s) => s.server.designAvailable);
  const activeDesign = useUiState(selectActiveDesign);
  const cadIntegrations = useMemo(
    () => integrations.filter((i) => i.category === 'cad'),
    [integrations],
  );

  const [targetId, setTargetId] = useState<string>('native');
  const [format, setFormat] = useState<ExportFormat>('stl');
  const [busy, setBusy] = useState(false);
  const [lastResult, setLastResult] = useState<ExportResult | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);

  const [designFormat, setDesignFormat] = useState<DesignExportFormat>('step');
  const [designBusy, setDesignBusy] = useState(false);
  const [designArtifact, setDesignArtifact] = useState<DesignExportArtifact | null>(null);
  const [designError, setDesignError] = useState<string | null>(null);

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
      appendJournal({
        kind: 'tool',
        message: `POST /api/studio/${target.id === 'native' ? 'export' : 'handoff'} · ${format.toUpperCase()} · ${primitive}`,
      });
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
      appendJournal({
        kind: 'export',
        message: `Saved ${artifact.filename}`,
        digest: `${artifact.bytes.toLocaleString()} bytes · ${format.toUpperCase()}`,
      });
    } catch (cause) {
      const message = describeError(cause).message;
      setLastError(message);
      appendJournal({ kind: 'export', message: `Export failed · ${message}` });
    } finally {
      setBusy(false);
    }
  }

  // A receipt belongs to the design it was written from: when the operator
  // activates another design, the old receipt is dropped instead of being
  // shown next to the wrong part.
  const activeDesignId = activeDesign?.design_id ?? null;
  useEffect(() => {
    setDesignArtifact(null);
    setDesignError(null);
  }, [activeDesignId]);

  const canExportDesign =
    connection.kind === 'ready' && !designBusy && activeDesign !== null;

  /**
   * Export the active design. The backend answers with the artefact bytes
   * and reports the sha256 of those bytes in `x-studio-digest`; the receipt
   * shows the real filename, the real byte count, and the digest the
   * backend reported (or "—" when it reported none).
   */
  async function onExportDesign(): Promise<void> {
    if (!canExportDesign || connection.kind !== 'ready' || !activeDesign) return;
    setDesignBusy(true);
    setDesignError(null);
    try {
      appendJournal({
        kind: 'export',
        message: `POST /api/studio/export · ${designFormat.toUpperCase()} · design ${activeDesign.design_id.slice(0, 8)} · ${activeDesign.provider}`,
      });
      const artifact = await exportDesign(
        connection.connection,
        activeDesign.design_id,
        designFormat,
      );
      saveBlob(artifact.blob, artifact.filename);
      setDesignArtifact(artifact);
      appendJournal({
        kind: 'export',
        message: `Saved ${artifact.filename}`,
        digest: artifact.digest
          ? `${artifact.bytes.toLocaleString()} bytes · sha256 ${artifact.digest.slice(0, 8)}`
          : `${artifact.bytes.toLocaleString()} bytes · sha256 —`,
      });
    } catch (cause) {
      const message = describeError(cause).message;
      setDesignError(message);
      appendJournal({ kind: 'export', message: `Design export failed · ${message}` });
    } finally {
      setDesignBusy(false);
    }
  }

  return (
    <section className="export-panel" aria-label="Export and handoff">
      {showHeading ? (
        <header className="export-panel-header">
          <h2>Export & handoff</h2>
        </header>
      ) : null}
      {activeDesign ? (
        <section className="export-panel-design" aria-label="Active design export">
          <p className="export-panel-design-head">
            <span>ACTIVE DESIGN</span>
            <span className="export-panel-design-id">{activeDesign.design_id.slice(0, 8)}</span>
          </p>
          <p className="export-panel-design-label">{activeDesign.label}</p>
          <p className="export-panel-design-provider" data-provider={activeDesign.provider}>
            {measuredByLabel(activeDesign)}
          </p>
          <p className="export-panel-design-artifacts">
            <span className="export-panel-design-artifact" data-artifact="script">
              <span className="export-panel-design-artifact-label">script</span>
              <code title={activeDesign.artifacts.script_sha256}>
                {activeDesign.artifacts.script_sha256.slice(0, 8)}
              </code>
            </span>
            {activeDesign.artifacts.step_sha256 ? (
              <span className="export-panel-design-artifact" data-artifact="step">
                <span className="export-panel-design-artifact-label">step</span>
                <code title={activeDesign.artifacts.step_sha256}>
                  {activeDesign.artifacts.step_sha256.slice(0, 8)}
                </code>
              </span>
            ) : null}
            {activeDesign.artifacts.native_sha256 ? (
              <span className="export-panel-design-artifact" data-artifact="native">
                <span className="export-panel-design-artifact-label">
                  {nativeKindLabel(activeDesign.artifacts.native_kind)}
                </span>
                <code title={activeDesign.artifacts.native_sha256}>
                  {activeDesign.artifacts.native_sha256.slice(0, 8)}
                </code>
              </span>
            ) : null}
          </p>
          <div className="export-panel-design-actions">
            <label className="export-panel-field">
              <span>Design format</span>
              <select
                value={designFormat}
                onChange={(event) => setDesignFormat(event.target.value as DesignExportFormat)}
                disabled={designBusy}
              >
                {DESIGN_FORMATS.map((entry) => (
                  <option key={entry} value={entry}>
                    {entry.toUpperCase()}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              className="export-panel-design-button"
              onClick={() => void onExportDesign()}
              disabled={!canExportDesign}
            >
              {designBusy ? 'Working…' : `Export design ${designFormat.toUpperCase()}`}
            </button>
          </div>
          {designArtifact ? (
            <div className="export-panel-receipt" role="status">
              <p className="export-panel-receipt-line">
                Saved <code>{designArtifact.filename}</code> ·{' '}
                {designArtifact.bytes.toLocaleString()} bytes
              </p>
              <p className="export-panel-receipt-line">
                <span className="export-panel-receipt-label">sha256</span>
                <code
                  className="export-panel-receipt-digest"
                  {...(designArtifact.digest ? { title: designArtifact.digest } : {})}
                >
                  {designArtifact.digest ?? '—'}
                </code>
                {designArtifact.digest ? (
                  <CopyButton
                    value={designArtifact.digest}
                    label="Copy sha256"
                    className="export-receipt-copy"
                  />
                ) : null}
              </p>
            </div>
          ) : null}
          {designError ? (
            <p className="export-panel-error" role="alert">
              {designError}
            </p>
          ) : null}
        </section>
      ) : designAvailable === false ? (
        <p className="export-panel-note">
          Design export needs the Python geometry backend; this backend reports design_available:
          false.
        </p>
      ) : null}
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
  if (cause instanceof StudioApiError) {
    const details = cause.details.length > 0 ? ` — ${cause.details.join(' · ')}` : '';
    return { message: `${cause.code}: ${cause.message}${details}`, kind: 'server' };
  }
  if (cause instanceof StudioNetworkError) {
    if (cause.cause instanceof DOMException && cause.cause.name === 'AbortError') {
      return { message: 'cancelled', kind: 'cancelled' };
    }
    return { message: cause.message, kind: 'network' };
  }
  if (cause instanceof Error) return { message: cause.message, kind: 'network' };
  return { message: 'unknown error', kind: 'network' };
}