// @vitest-environment happy-dom
// The server's verdict on the primitive preview.
//
// A refused server preview used to be invisible: the inspector kept saying
// "Local preview", the bottom bar kept saying "local est.", and the only
// trace was a journal row nobody was reading. These tests pin that the real
// message is shown, that a request in flight says so, and that both clear
// themselves when the preview recovers.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, render } from '@testing-library/react';

import { Inspector } from '../src/components/inspector/Inspector';
import { DraftingSheet } from '../src/components/drafting/DraftingSheet';
import type { DraftingSheetProps, TitleBlockData } from '../src/components/drafting/DraftingSheet';
import { BottomBar } from '../src/components/bottombar/BottomBar';
import { dispatch, resetStore } from '../src/state/store';

const titleBlock: TitleBlockData = {
  sheet: 'SHEET 01',
  partId: 'STUDIO-BOX',
  title: 'box · 40.00 × 20.00 × 30.00 mm',
  scale: 'FIT',
  units: 'mm',
  projection: '3RD ANGLE',
  tolerance: '—',
  revision: 'DRAFT',
  author: 'Kala Studio',
};

function sheetProps(overrides: Partial<DraftingSheetProps> = {}): DraftingSheetProps {
  return {
    mesh: null,
    meshSource: 'none',
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default',
    background: 'paper',
    renderMode: 'pencil2d',
    showSection: true,
    titleBlock,
    units: 'mm',
    viewResetToken: 0,
    ...overrides,
  };
}

beforeEach(() => {
  resetStore();
});

afterEach(() => {
  cleanup();
  resetStore();
});

describe('a refused server preview', () => {
  it('shows the real message in the inspector', () => {
    act(() => {
      dispatch({
        type: 'preview/request-error',
        message: 'provider_timeout: the mesh server exceeded its budget',
        kind: 'timeout',
      });
    });

    const view = render(<Inspector />);
    const error = view.container.querySelector('.inspector-preview-error');
    expect(error?.getAttribute('role')).toBe('alert');
    expect(error?.textContent).toContain('provider_timeout: the mesh server exceeded its budget');
  });

  it('shows the real message on the drafting sheet', () => {
    act(() => {
      dispatch({
        type: 'preview/request-error',
        message: 'mesh_too_large: 400k triangles exceed the server budget',
        kind: 'server',
      });
    });

    const view = render(<DraftingSheet {...sheetProps()} />);
    const note = view.container.querySelector('.drafting-preview-note');
    expect(note?.getAttribute('role')).toBe('alert');
    expect(note?.textContent).toContain('mesh_too_large: 400k triangles exceed the server budget');
    expect(note?.classList.contains('is-error')).toBe(true);
  });

  it('says in the bottom bar that the server refused the preview', () => {
    act(() => {
      dispatch({
        type: 'preview/request-error',
        message: 'provider_timeout: the mesh server exceeded its budget',
        kind: 'timeout',
      });
    });

    const view = render(<BottomBar onOpenOptions={() => {}} />);
    const chip = view.container.querySelector('.bottom-bar-source');
    expect(chip?.textContent).toBe('server refused');
    expect(chip?.getAttribute('data-preview-state')).toBe('refused');
    expect(chip?.getAttribute('title')).toContain('provider_timeout');
  });

  it('clears the message when the preview recovers', () => {
    act(() => {
      dispatch({
        type: 'preview/request-error',
        message: 'provider_timeout: the mesh server exceeded its budget',
        kind: 'timeout',
      });
    });

    const view = render(<Inspector />);
    expect(view.container.querySelector('.inspector-preview-error')).toBeTruthy();

    act(() => {
      dispatch({
        type: 'preview/request-success',
        preview: {
          positions: [],
          normals: [],
          indices: [],
          bounds: { min: [0, 0, 0], max: [1, 1, 1] },
          engine: 'ocp',
          warnings: [],
        },
      });
    });

    expect(view.container.querySelector('.inspector-preview-error')).toBeNull();
  });
});

describe('a preview request in flight', () => {
  it('says that the server is verifying the mesh', () => {
    act(() => {
      dispatch({ type: 'preview/request-start' });
    });

    const inspector = render(<Inspector />);
    const state = inspector.container.querySelector('.inspector-preview-state');
    expect(state?.getAttribute('role')).toBe('status');
    expect(state?.textContent).toContain('Verifying the mesh on the server');

    const sheet = render(<DraftingSheet {...sheetProps()} />);
    const note = sheet.container.querySelector('.drafting-preview-note');
    expect(note?.getAttribute('role')).toBe('status');
    expect(note?.textContent).toContain('Verifying the mesh on the server');

    const bar = render(<BottomBar onOpenOptions={() => {}} />);
    const chip = bar.container.querySelector('.bottom-bar-source');
    expect(chip?.textContent).toBe('verifying…');
    expect(chip?.getAttribute('data-preview-state')).toBe('verifying');
  });

  it('says nothing when no preview was asked for', () => {
    const inspector = render(<Inspector />);
    expect(inspector.container.querySelector('.inspector-preview-state')).toBeNull();
    expect(inspector.container.querySelector('.inspector-preview-error')).toBeNull();
  });
});
