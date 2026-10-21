// The view-provenance labels.
//
// What the operator sees in the viewport, on the HLR sheet and in the AuxView
// footer is ALWAYS the OpenCascade import of the provider's own STEP bytes —
// including for the opt-in OpenCascade build, whose `view.imported` journal
// event carries the role `view-only` as well. The labels must therefore say
// that, and must never claim that the OpenCascade kernel built the geometry.

import { describe, expect, it } from 'vitest';

import { designViewLabel, designViewStamp, drawingSourceStamp } from '../src/viewer/designSource';

describe('the view-provenance label', () => {
  it('names the provider whose STEP was imported', () => {
    expect(designViewLabel('freecad')).toBe(
      'view: OpenCascade import (view-only) of the FreeCAD STEP',
    );
    expect(designViewLabel('solidedge')).toBe(
      'view: OpenCascade import (view-only) of the Solid Edge STEP',
    );
  });

  it('calls the opt-in OpenCascade build an import as well', () => {
    expect(designViewLabel('opencascade')).toBe(
      'view: OpenCascade import (view-only) of the OpenCascade (opt-in) STEP',
    );
    // It never claims that the OpenCascade kernel built the part.
    expect(designViewLabel('opencascade')).not.toContain('build');
  });

  it('never claims that the kernel built anything', () => {
    for (const provider of ['freecad', 'solidedge', 'opencascade'] as const) {
      expect(designViewLabel(provider)).toContain('import (view-only)');
      expect(designViewLabel(provider)).not.toContain('OPC BUILD');
    }
  });

  it('says only what is true when there is no provider', () => {
    expect(designViewLabel(null)).toBe(
      'view: OpenCascade import (view-only) of the provider STEP',
    );
    expect(designViewLabel(undefined)).toBe(
      'view: OpenCascade import (view-only) of the provider STEP',
    );
  });
});

describe('the short view stamp', () => {
  it('names the imported STEP in upper case', () => {
    expect(designViewStamp('freecad')).toBe('VIEW: OCC IMPORT OF FREECAD STEP');
    expect(designViewStamp('opencascade')).toBe(
      'VIEW: OCC IMPORT OF OPENCASCADE (OPT-IN) STEP',
    );
    expect(designViewStamp(null)).toBe('VIEW: OCC IMPORT OF THE PROVIDER STEP');
  });
});

describe('the drawing source stamp', () => {
  it('says the drawing is a hidden-line view of the imported model', () => {
    expect(drawingSourceStamp()).toBe('SOURCE: OCP HLR OF THE IMPORTED MODEL');
    expect(drawingSourceStamp()).toContain('IMPORTED MODEL');
  });
});
