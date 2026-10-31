// One chip of the composer's setup row, and the popover it opens.
//
// The composer shows one row with two chips (provider/model and CAD target):
// everything else about a turn is one click away instead of filling the
// column. The popover is a local, non-modal dialog: it is anchored above the
// chip, closes on Escape and on a click outside, and hands the focus back to
// the chip when it closes. It uses no dependency and no portal — the
// composer's own stacking context is enough for a popover that opens upward.

import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent, ReactElement, ReactNode } from 'react';

export interface ComposerPopoverProps {
  /** Stable hook: `model` or `cad`. */
  name: string;
  /**
   * The accessible name of both the chip and the dialog. The chip shows the
   * live value in its children; this label says what the value means.
   */
  label: string;
  /** The chip's visible content. */
  children: ReactNode;
  /** The dialog's content. */
  panel: ReactNode;
  /** Full text for the chip's native tooltip (the visible value may truncate). */
  title?: string;
  /** Draw the chip in the vermilion attention style (a choice is required). */
  attention?: boolean;
}

export function ComposerPopover(props: ComposerPopoverProps): ReactElement {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const chipRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);

  // A click outside the chip and its popover closes the popover. The
  // listener exists only while the popover is open.
  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: MouseEvent): void {
      const root = rootRef.current;
      if (!root) return;
      if (event.target instanceof Node && root.contains(event.target)) return;
      setOpen(false);
      chipRef.current?.focus();
    }
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  // Escape closes the popover and returns the focus to the chip. A nested
  // listbox (the provider picker) handles its own Escape first and marks the
  // event handled; the popover must not swallow that first press.
  useEffect(() => {
    if (!open) return;
    function onKeyDown(event: globalThis.KeyboardEvent): void {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      event.preventDefault();
      setOpen(false);
      chipRef.current?.focus();
    }
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [open]);

  // Opening moves the focus into the dialog, so Tab walks its controls and
  // Escape is heard from inside it.
  useEffect(() => {
    if (open) panelRef.current?.focus();
  }, [open]);

  function close(returnFocus: boolean): void {
    setOpen(false);
    if (returnFocus) chipRef.current?.focus();
  }

  function onPanelKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    if (event.key !== 'Escape' || event.defaultPrevented) return;
    event.preventDefault();
    close(true);
  }

  return (
    <div className="composer-chip" ref={rootRef} data-composer-chip={props.name}>
      <button
        type="button"
        ref={chipRef}
        className="composer-chip-button"
        data-chip={props.name}
        data-attention={props.attention === true ? 'yes' : 'no'}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        aria-label={props.label}
        title={props.title ?? props.label}
        onClick={() => (open ? close(false) : setOpen(true))}
      >
        {props.children}
      </button>
      {open ? (
        <div
          ref={panelRef}
          id={panelId}
          className="composer-popover"
          role="dialog"
          aria-label={props.label}
          data-popover={props.name}
          tabIndex={-1}
          onKeyDown={onPanelKeyDown}
        >
          {props.panel}
        </div>
      ) : null}
    </div>
  );
}
