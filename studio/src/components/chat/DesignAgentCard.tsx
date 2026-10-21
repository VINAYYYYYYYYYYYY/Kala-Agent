// The agent card of a design turn: what the CAD provider measured.
//
// The card is the honest record of one provider-built design. It shows:
//   - the provider identity and version beside every measurement;
//   - the measured table (volume, area, envelope X/Y/Z, and the mass only
//     when the provider reported one or the backend DECLARED a density);
//   - the backend's checks, each with a passed/failed/not-reported mark and
//     the expected value beside the measured one;
//   - the digest chips of the artefacts, the first eight characters of the
//     SHA-256 plus a copy button for the full digest;
//   - the Prussian "B-Rep Verified" stamp, and only when `verified` is true;
//   - a "Section A–A ↗" pill that makes this design the active one and
//     switches the auxiliary view to the section cut.
//
// Nothing here is invented. A value the provider did not report prints "—",
// and every stamp is absent unless the data behind it exists.

import { useCallback, useEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import { activateDesign, appendJournal, dispatch, useUiState } from '../../state/store';
import { getDesignArtifact, StudioApiError } from '../../api/client';
import type { DesignArtifactKind, DesignCheck, DesignResult } from '../../types/studio';
import {
  checkSummary,
  digestChips,
  formatCheckUnit,
  formatCheckValue,
  measuredByLabel,
  measuredRows,
} from './designCard';
import { saveBlob } from '../../saveBlob';

export interface DesignAgentCardProps {
  /** The design the turn produced. The card reads it from the store. */
  designId: string;
}

/** How long the "copied" mark stays on a digest chip. */
const COPY_FEEDBACK_MS = 1500;

export function DesignAgentCard(props: DesignAgentCardProps): ReactElement {
  // The store owns the design: the chat turn only remembers which design
  // it produced, so a card can never show a stale copy of a measurement.
  const design = useUiState((s) => s.designs.designs.get(props.designId) ?? null);
  const connection = useUiState((s) => s.connection);
  const [copied, setCopied] = useState<string | null>(null);
  const [copyFailed, setCopyFailed] = useState<boolean>(false);
  const [downloading, setDownloading] = useState<DesignArtifactKind | null>(null);
  const [downloadError, setDownloadError] = useState<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  /**
   * Download one stored artifact next to its digest chip. The bytes are the
   * backend's exact stored file, named by its `content-disposition`; the
   * receipt in the journal shows the digest the backend verified.
   */
  const onDownload = useCallback(
    async (kind: DesignArtifactKind, digest: string) => {
      if (connection.kind !== 'ready' || downloading !== null) return;
      setDownloading(kind);
      setDownloadError(null);
      try {
        appendJournal({
          kind: 'export',
          message: `GET /api/studio/design/${props.designId.slice(0, 8)}/artifact/${kind}`,
        });
        const artifact = await getDesignArtifact(connection.connection, props.designId, kind);
        saveBlob(artifact.blob, artifact.filename);
        appendJournal({
          kind: 'export',
          message: `Saved ${artifact.filename}`,
          digest: artifact.digest
            ? `${artifact.bytes.toLocaleString()} bytes · sha256 ${digest.slice(0, 8)}`
            : `${artifact.bytes.toLocaleString()} bytes · sha256 —`,
        });
      } catch (cause) {
        const message =
          cause instanceof StudioApiError
            ? `${cause.code}: ${cause.message}`
            : cause instanceof Error
              ? cause.message
              : 'unknown error';
        setDownloadError(message);
        appendJournal({ kind: 'export', message: `Artifact download failed · ${message}` });
      } finally {
        setDownloading(null);
      }
    },
    [connection, downloading, props.designId],
  );

  const onCopy = useCallback(async (digest: string) => {
    const written = await writeClipboard(digest);
    if (timerRef.current) clearTimeout(timerRef.current);
    if (written) {
      setCopyFailed(false);
      setCopied(digest);
    } else {
      setCopyFailed(true);
      setCopied(null);
    }
    timerRef.current = setTimeout(() => {
      setCopied(null);
      setCopyFailed(false);
    }, COPY_FEEDBACK_MS);
  }, []);

  if (!design) {
    return (
      <section
        className="chat-panel-design-card chat-panel-design-card-missing"
        aria-label="Compiled design"
      >
        <p className="chat-panel-design-missing">
          This design is no longer held in the session store; the design cache evicted it.
        </p>
      </section>
    );
  }

  const summary = checkSummary(design);
  const chips = digestChips(design);
  const measuredBy = measuredByLabel(design);
  const warnings = [...design.warnings, ...design.view.warnings];

  return (
    <section className="chat-panel-design-card" aria-label="Compiled design">
      <header className="chat-panel-design-card-head">
        <span className="chat-panel-design-card-label">{design.label}</span>
        <span
          className="chat-panel-design-card-id"
          title={`design_id ${design.design_id}`}
        >
          {design.design_id.slice(0, 8)}
        </span>
        {design.verified ? (
          <span className="chat-panel-design-stamp">B-Rep Verified</span>
        ) : summary.total > 0 ? (
          <span
            className="chat-panel-design-stamp is-failed"
            data-stamp={summary.failed > 0 ? 'failed' : 'unverified'}
          >
            {stampText(summary)}
          </span>
        ) : null}
      </header>

      <p className="chat-panel-design-provider" data-provider={design.provider}>
        {measuredBy}
      </p>

      <table className="chat-panel-design-table">
        <caption className="chat-panel-design-caption">
          {measuredBy}
        </caption>
        <tbody>
          {measuredRows(design).map((row) => (
            <tr key={row.key} data-measure={row.key}>
              <th scope="row">{row.label}</th>
              <td title={row.title}>{row.value}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {summary.total > 0 ? (
        <ul className="chat-panel-design-checks" aria-label="Backend checks">
          {design.checks.map((check) => (
            <CheckRow key={check.id} check={check} />
          ))}
        </ul>
      ) : null}

      {chips.length > 0 ? (
        <div className="chat-panel-design-digests" aria-label="Artefact digests">
          {chips.map((chip) => (
            <span
              key={chip.kind}
              className="chat-panel-design-digest"
              data-digest-kind={chip.kind}
            >
              <span className="chat-panel-design-digest-label">{chip.label}</span>
              <code className="chat-panel-design-digest-value" title={chip.digest}>
                {chip.short}
              </code>
              <button
                type="button"
                className="chat-panel-design-digest-copy"
                data-copied={copied === chip.digest ? 'yes' : 'no'}
                data-copy-failed={copyFailed ? 'yes' : 'no'}
                aria-label={`Copy ${chip.label} sha256`}
                title={`Copy the full SHA-256 of this ${chip.label} artefact`}
                onClick={() => {
                  void onCopy(chip.digest);
                }}
              >
                {copied === chip.digest ? 'copied' : 'copy'}
              </button>
              {chip.downloadKind ? (
                <button
                  type="button"
                  className="chat-panel-design-digest-download"
                  data-artifact={chip.downloadKind}
                  aria-label={`Download ${chip.label}`}
                  title={`Download the stored ${chip.label} artefact`}
                  disabled={downloading !== null}
                  onClick={() => {
                    void onDownload(chip.downloadKind!, chip.digest);
                  }}
                >
                  {downloading === chip.downloadKind ? '…' : 'download'}
                </button>
              ) : null}
            </span>
          ))}
        </div>
      ) : null}

      {downloadError ? (
        <p className="chat-panel-design-error" role="alert">
          {downloadError}
        </p>
      ) : null}

      {warnings.length > 0 ? (
        <ul className="chat-panel-design-warnings" aria-label="Design warnings">
          {warnings.map((warning, index) => (
            <li key={`${warning}-${index}`}>{warning}</li>
          ))}
        </ul>
      ) : null}

      <button
        type="button"
        className="chat-panel-design-section-pill"
        onClick={() => {
          activateDesign(design.design_id);
          dispatch({ type: 'auxView/set', view: 'section' });
        }}
      >
        Section A–A ↗
      </button>
    </section>
  );
}

function stampText(summary: ReturnType<typeof checkSummary>): string {
  if (summary.failed > 0 && summary.notReported > 0) {
    return `${summary.failed} failed · ${summary.notReported} not reported`;
  }
  if (summary.failed > 0) return `${summary.failed} of ${summary.total} checks failed`;
  if (summary.notReported > 0) {
    return `${summary.notReported} of ${summary.total} checks not reported`;
  }
  return `${summary.passed}/${summary.total} checks pass`;
}

function CheckRow(props: { check: DesignCheck }): ReactElement {
  const { check } = props;
  const unit = formatCheckUnit(check.unit);
  const passed = check.status === 'passed';
  const notReported = check.status === 'not_reported';
  return (
    <li
      className="chat-panel-design-check"
      data-check={check.id}
      data-passed={passed ? 'yes' : 'no'}
      data-status={check.status}
    >
      <span
        className="chat-panel-design-check-mark"
        role="img"
        aria-label={notReported ? 'not reported' : passed ? 'pass' : 'fail'}
      >
        {notReported ? '—' : passed ? '✓' : '✗'}
      </span>
      <span className="chat-panel-design-check-label">{check.label}</span>
      <span className="chat-panel-design-check-values">
        <span className="chat-panel-design-check-expected">
          expected {formatCheckValue(check.expected)}
        </span>
        <span className="chat-panel-design-check-measured">
          measured {formatCheckValue(check.measured)}
          {check.measured !== null && unit ? ` ${unit}` : ''}
        </span>
      </span>
    </li>
  );
}

/**
 * Put the full digest on the clipboard. A browser without the async
 * clipboard API (or a denied permission) reports the failure instead of
 * pretending the digest was copied.
 */
async function writeClipboard(text: string): Promise<boolean> {
  const clipboard = typeof navigator === 'undefined' ? undefined : navigator.clipboard;
  if (!clipboard || typeof clipboard.writeText !== 'function') return false;
  try {
    await clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
