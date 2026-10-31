// Export dialog.
//
// Export is a focused task, not a dashboard card: the top bar opens this
// modal, which renders the export panel alone. With an active design the
// design export is the primary action and the primitive handoff sits behind
// a disclosure; with no design the primitive export is shown directly.
//
// Accessibility: role="dialog", aria-modal, Escape closes, focus is trapped
// while open, and focus returns to the trigger on close.

import { useEffect, useRef } from 'react';
import type { ReactElement } from 'react';
import { ExportPanel } from './ExportPanel';

export interface ExportDialogProps {
  open: boolean;
  onClose(): void;
}

export function ExportDialog(props: ExportDialogProps): ReactElement | null {
  const dialogRef = useRef<HTMLDivElement | null>(null);
  const closeRef = useRef<HTMLButtonElement | null>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(props.onClose);
  onCloseRef.current = props.onClose;
  const { open } = props;

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement;
    returnFocusRef.current = previous instanceof HTMLElement ? previous : null;
    closeRef.current?.focus();

    function focusables(): HTMLElement[] {
      const dialog = dialogRef.current;
      if (!dialog) return [];
      return Array.from(
        dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((element) => element.getAttribute('aria-hidden') !== 'true');
    }

    function onKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') {
        event.preventDefault();
        onCloseRef.current();
        return;
      }
      if (event.key !== 'Tab') return;
      const items = focusables();
      if (items.length === 0) return;
      const first = items[0]!;
      const last = items[items.length - 1]!;
      const active = document.activeElement;
      if (event.shiftKey && (active === first || active === dialogRef.current)) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('keydown', onKeyDown, true);
      returnFocusRef.current?.focus();
      returnFocusRef.current = null;
    };
  }, [open]);

  if (!open) return null;

  return (
    <div
      className="app-modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) props.onClose();
      }}
    >
      <div
        ref={dialogRef}
        className="export-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-dialog-title"
      >
        <header className="export-dialog-header">
          <h2 id="export-dialog-title" className="export-dialog-title">
            Export
          </h2>
          <button
            ref={closeRef}
            type="button"
            className="export-dialog-close"
            aria-label="Close export"
            onClick={props.onClose}
          >
            Close [Esc]
          </button>
        </header>
        <div className="export-dialog-body">
          <ExportPanel showHeading={false} />
        </div>
      </div>
    </div>
  );
}
