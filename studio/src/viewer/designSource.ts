// The view-provenance labels shared by the 3D viewport, the 2D HLR sheet and
// the AuxView footer.
//
// What the operator actually sees is ALWAYS the OpenCascade import of the
// provider's own STEP bytes (ADR-0011 §3) — including for the opt-in
// OpenCascade build, whose journal event `view.imported` carries the role
// `view-only` as well. The labels therefore name the import and whose STEP
// was imported; they never claim that the OpenCascade kernel built the
// geometry the operator is looking at.

import type { CadProviderId } from '../types/studio';

/** How each CAD provider's STEP file is named in the label. */
const PROVIDER_STEP_NAME: Readonly<Record<CadProviderId, string>> = {
  freecad: 'FreeCAD',
  opencascade: 'OpenCascade (opt-in)',
  solidedge: 'Solid Edge',
};

/**
 * The long label of one design's viewport provenance.
 *
 * `null` (no design) has no provider STEP to name, so the label says only
 * what is true.
 */
export function designViewLabel(provider: CadProviderId | null | undefined): string {
  const name = provider ? PROVIDER_STEP_NAME[provider] : null;
  if (!name) return 'view: OpenCascade import (view-only) of the provider STEP';
  return `view: OpenCascade import (view-only) of the ${name} STEP`;
}

/**
 * The short uppercase label of the same fact, for the drafted footer of the
 * ortho view.
 */
export function designViewStamp(provider: CadProviderId | null | undefined): string {
  const name = provider ? PROVIDER_STEP_NAME[provider] : null;
  if (!name) return 'VIEW: OCC IMPORT OF THE PROVIDER STEP';
  return `VIEW: OCC IMPORT OF ${name.toUpperCase()} STEP`;
}

/**
 * The label of the source of the ortho/HLR drawing: an OpenCascade
 * hidden-line view OF THE IMPORTED MODEL, never of a model the OpenCascade
 * kernel built.
 */
export function drawingSourceStamp(): string {
  return 'SOURCE: OCP HLR OF THE IMPORTED MODEL';
}
