// A small, honest copy control.
//
// The clipboard is a browser permission, not a guarantee: a denied or
// missing clipboard must never look like a successful copy. The button
// reports the real outcome through `data-state` and its accessible name, so
// a screen reader and a test both see the truth.
//
// The ledger's digest chip and the export receipt's sha256 line share this
// one control; the base styles live in `styles/journal.css` because the
// ledger owns the copy affordance of the workspace.

import { useEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';

/** The outcome of the last copy attempt. */
export type CopyState = 'idle' | 'copied' | 'failed';

export interface CopyButtonProps {
  /** The exact text placed on the clipboard. */
  value: string;
  /** The accessible name of the control, e.g. "Copy digest". */
  label: string;
  /** Extra class names for the button. */
  className?: string;
}

/** How long the copied/failed state stays visible. */
const FEEDBACK_MS = 1400;

/**
 * Put text on the clipboard. Returns false when the browser refused or does
 * not offer a clipboard at all — the caller reports that honestly.
 */
export async function copyTextToClipboard(text: string): Promise<boolean> {
  try {
    const clipboard =
      typeof navigator !== 'undefined'
        ? (navigator as Navigator & { clipboard?: Clipboard }).clipboard
        : undefined;
    if (clipboard && typeof clipboard.writeText === 'function') {
      await clipboard.writeText(text);
      return true;
    }
  } catch {
    // A rejected clipboard permission is a refusal, not a crash.
  }
  return false;
}

export function CopyButton(props: CopyButtonProps): ReactElement {
  const [state, setState] = useState<CopyState>('idle');
  const timerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    },
    [],
  );

  async function onCopy(): Promise<void> {
    const copied = await copyTextToClipboard(props.value);
    setState(copied ? 'copied' : 'failed');
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      setState('idle');
    }, FEEDBACK_MS);
  }

  const className = props.className
    ? `studio-copy-button ${props.className}`
    : 'studio-copy-button';

  return (
    <button
      type="button"
      className={className}
      data-state={state}
      aria-label={describeLabel(props.label, state)}
      title={describeTitle(props.label, state)}
      onClick={() => {
        void onCopy();
      }}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
        <path d="M5.75 5.75h6.5v6.5h-6.5zM3.75 10.25h-1.5v-6.5h6.5v1.5" />
      </svg>
    </button>
  );
}

function describeLabel(label: string, state: CopyState): string {
  if (state === 'copied') return `${label} — copied`;
  if (state === 'failed') return `${label} — the clipboard refused the copy`;
  return label;
}

function describeTitle(label: string, state: CopyState): string {
  if (state === 'copied') return 'Copied to the clipboard';
  if (state === 'failed') return 'The clipboard refused the copy';
  return label;
}
