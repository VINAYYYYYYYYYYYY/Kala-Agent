// @vitest-environment happy-dom
// The mesh object handed to the viewport must be referentially stable.
//
// The shell re-renders for many reasons that have nothing to do with the
// geometry: a journal row is appended, the auxiliary tab changes, the
// options drawer opens, the theme switches. If the mesh object changed on
// every one of those renders, the viewport would rebuild the GPU geometry
// and re-frame the camera, and the operator would lose their view every
// time they sent a chat message.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render } from '@testing-library/react';
import type { ReactNode } from 'react';

const mocks = vi.hoisted(() => ({
  renders: [] as Array<{ mesh: unknown; meshSource: string }>,
}));

// The viewport is replaced by a recorder so the test can compare the mesh
// object identity across renders. The real WebGL canvas is not needed here.
vi.mock('../src/viewer/SceneCanvas', async () => {
  const { createElement } = await import('react');
  return {
    SceneCanvas: (props: { mesh: unknown; meshSource: string }) => {
      mocks.renders.push({ mesh: props.mesh, meshSource: props.meshSource });
      return createElement('div', { className: 'viewer-canvas', 'data-mock': 'scene-canvas' });
    },
  };
});

// The boot gate is the connection handshake; it is covered by its own tests.
// Here the shell is rendered directly.
vi.mock('../src/components/shell/BootGate', async () => {
  const { createElement, Fragment } = await import('react');
  return {
    BootGate: ({ children }: { children: ReactNode }) => createElement(Fragment, null, children),
  };
});

import { AppShell } from '../src/components/shell/AppShell';
import { appendJournal, dispatch, resetStore } from '../src/state/store';
import type { PreviewResponse } from '../src/types/studio';

const serverPreview: PreviewResponse = {
  positions: [0, 0, 0, 10, 0, 0, 0, 10, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  indices: [0, 1, 2],
  bounds: { min: [0, 0, -5], max: [10, 10, 5] },
  engine: 'analytic',
  warnings: [],
};

beforeEach(() => {
  resetStore();
  mocks.renders.length = 0;
});

afterEach(() => {
  cleanup();
  resetStore();
});

function lastRender(): { mesh: unknown; meshSource: string } {
  return mocks.renders[mocks.renders.length - 1]!;
}

function renderShellWithServerMesh(): void {
  dispatch({ type: 'preview/request-success', preview: serverPreview });
  // The drawing is a secondary surface: it must be open for the viewport
  // (and its one SceneCanvas) to be mounted at all.
  dispatch({ type: 'chrome/drawing-open' });
  render(<AppShell />);
  expect(lastRender().meshSource).toBe('server');
}

describe('viewport mesh identity', () => {
  it('keeps the same mesh object when a journal row is appended', () => {
    renderShellWithServerMesh();
    const mesh = lastRender().mesh;
    // The ledger lives in Details › Activity now; open it so the row is
    // really rendered somewhere in the frame.
    act(() => {
      dispatch({ type: 'chrome/details-open', tab: 'activity' });
    });

    act(() => {
      appendJournal({ kind: 'system', message: 'unrelated ledger row' });
    });

    // The row landed in the ledger ...
    expect(document.body.textContent).toContain('unrelated ledger row');
    // ... and the viewport still holds the very same mesh object.
    expect(lastRender().mesh).toBe(mesh);
  });

  it('keeps the same mesh object when the auxiliary view tab changes', () => {
    renderShellWithServerMesh();
    const mesh = lastRender().mesh;

    act(() => {
      dispatch({ type: 'auxView/set', view: 'ortho' });
    });
    expect(lastRender().mesh).toBe(mesh);

    act(() => {
      dispatch({ type: 'auxView/set', view: 'tolerance' });
    });
    expect(lastRender().mesh).toBe(mesh);
  });

  it('keeps the same mesh object when the options drawer opens and the theme changes', () => {
    renderShellWithServerMesh();
    const mesh = lastRender().mesh;

    act(() => {
      dispatch({ type: 'options/open', card: 'workspace' });
    });
    expect(lastRender().mesh).toBe(mesh);

    act(() => {
      dispatch({ type: 'theme/set', theme: 'slate' });
    });
    expect(lastRender().mesh).toBe(mesh);
  });

  it('hands the viewport a new mesh object when a new server preview arrives', () => {
    renderShellWithServerMesh();
    const first = lastRender().mesh;

    act(() => {
      dispatch({
        type: 'preview/request-success',
        preview: {
          ...serverPreview,
          positions: [0, 0, 0, 20, 0, 0, 0, 20, 0],
          bounds: { min: [0, 0, -10], max: [20, 20, 10] },
        },
      });
    });

    // The identity guard above must not be vacuous: a genuinely new mesh
    // still produces a new object.
    expect(lastRender().mesh).not.toBe(first);
  });
});
