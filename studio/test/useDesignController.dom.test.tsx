// @vitest-environment happy-dom
// The shared design controller hook: it exists only while the connection is
// ready, requests through the cached controller, and puts the response in the
// store under the same key the sheet and the AuxView read.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  drawing: vi.fn(),
  section: vi.fn(),
}));

vi.mock('../src/api/client', () => {
  class StudioAbortError extends Error {}
  class StudioTimeoutError extends Error {
    path = '';
  }
  class StudioAuthError extends Error {
    status = 0;
  }
  class StudioApiError extends Error {
    code = '';
    details: string[] = [];
  }
  class StudioNetworkError extends Error {}
  return {
    getDesignDrawing: mocks.drawing,
    getDesignSection: mocks.section,
    StudioAbortError,
    StudioTimeoutError,
    StudioAuthError,
    StudioApiError,
    StudioNetworkError,
  };
});

import { useEffect } from 'react';
import type { ReactElement } from 'react';
import { cleanup, render, waitFor } from '@testing-library/react';
import { useDesignController } from '../src/components/drafting/useDesignController';
import {
  dispatch,
  drawingCacheKey,
  getState,
  resetStore,
} from '../src/state/store';
import type { StudioConnection } from '../src/types/studio';
import { BRACKET_DESIGN_ID, bracketDrawing } from './designFixtures';

const connection: StudioConnection = {
  baseUrl: 'http://127.0.0.1:8765',
  token: 'test-token',
  mode: 'browser-manual',
};

function Harness(props: { designId: string }): ReactElement {
  const controller = useDesignController();
  const { designId } = props;
  useEffect(() => {
    if (controller) void controller.loadDrawing(designId);
  }, [controller, designId]);
  return <div>{controller ? 'ready' : 'waiting'}</div>;
}

beforeEach(() => {
  resetStore();
  mocks.drawing.mockReset();
  mocks.section.mockReset();
});

afterEach(cleanup);

describe('useDesignController', () => {
  it('requests the drawing only after a ready connection exists', async () => {
    mocks.drawing.mockResolvedValue(bracketDrawing);
    const { rerender } = render(<div>no connection</div>);
    expect(mocks.drawing).not.toHaveBeenCalled();

    dispatch({ type: 'connection/ready', connection });
    rerender(<Harness designId={BRACKET_DESIGN_ID} />);

    await waitFor(() => {
      expect(getState().designs.drawings.get(drawingCacheKey(BRACKET_DESIGN_ID, []))?.status).toBe(
        'ready',
      );
    });
    expect(mocks.drawing).toHaveBeenCalledTimes(1);
    expect(mocks.drawing.mock.calls[0]![0]).toBe(connection);
    expect(mocks.drawing.mock.calls[0]![1]).toBe(BRACKET_DESIGN_ID);
    expect(mocks.drawing.mock.calls[0]![3]).toBeInstanceOf(AbortSignal);
  });

  it('returns no controller while the connection is not ready', () => {
    const { container } = render(<Harness designId={BRACKET_DESIGN_ID} />);
    expect(container.textContent).toBe('waiting');
    expect(mocks.drawing).not.toHaveBeenCalled();
  });
});
