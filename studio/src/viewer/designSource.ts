// The source label shared by the 3D viewport and the 2D HLR sheet.
//
// Both surfaces always render the OpenCascade import of the provider's exact
// STEP bytes (ADR-0011 §3). Only an opt-in OpenCascade build is a design that
// OpenCascade itself built; every other provider's part is view-only here.

import type { CadProviderId } from '../types/studio';

export function designViewLabel(provider: CadProviderId | null | undefined): string {
  return provider === 'opencascade'
    ? 'view: OpenCascade (opt-in) build'
    : 'view: OpenCascade import (view-only)';
}
