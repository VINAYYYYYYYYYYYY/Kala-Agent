// Test helpers for the composer's provider picker.
//
// The picker is a listbox popup, not a native `<select>`, so the tests
// drive it the way an operator does: open the popup, then pick a row.
//
// In the full ChatPanel the provider picker lives behind the model chip's
// popover (app-feel wave). In the picker's own component test it is mounted
// directly. These helpers handle both: when the trigger is not on screen,
// they open the model chip first.

import { fireEvent } from '@testing-library/react';

/** The model chip of the composer. */
export function modelChip(container: HTMLElement): HTMLButtonElement {
  const chip = container.querySelector<HTMLButtonElement>('[data-chip="model"]');
  if (!chip) throw new Error('the composer has no model chip');
  return chip;
}

/** Open the model popover unless the provider picker is already mounted. */
export function openModelPopover(container: HTMLElement): void {
  if (container.querySelector('.provider-picker-trigger')) return;
  fireEvent.click(modelChip(container));
}

/** The provider picker's trigger button. */
export function providerTrigger(container: HTMLElement): HTMLButtonElement {
  openModelPopover(container);
  const trigger = container.querySelector<HTMLButtonElement>('.provider-picker-trigger');
  if (!trigger) throw new Error('the provider picker trigger is not mounted');
  return trigger;
}

/** Open the provider listbox. */
export function openProviderPicker(container: HTMLElement): void {
  fireEvent.click(providerTrigger(container));
}

/** One row of the open provider listbox. */
export function providerOption(container: HTMLElement, providerId: string): HTMLElement {
  const option = container.querySelector<HTMLElement>(`[data-provider-id="${providerId}"]`);
  if (!option) throw new Error(`the provider picker has no row for "${providerId}"`);
  return option;
}

/** Pick a provider the way the operator does: open the list, click a row. */
export function pickProvider(container: HTMLElement, providerId: string): void {
  openProviderPicker(container);
  fireEvent.click(providerOption(container, providerId));
}
