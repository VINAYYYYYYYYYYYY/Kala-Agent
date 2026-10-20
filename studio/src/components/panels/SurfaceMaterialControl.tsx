// Surface material control.
//
// `viewport.material` is a real store field with three values, and the
// viewport already implements all three (`SceneCanvas.makeFaceMaterial`).
// This control is the only UI that writes it.
//
// The canvas *background* is deliberately not part of this control: the
// background follows the workspace theme (vellum paper by day, night slate
// at night) so the viewport surface and the drafting sheet behind it can
// never disagree about the paper they are drawn on.

import type { ReactElement } from 'react';
import { dispatch, selectViewport, useUiState } from '../../state/store';
import type { MaterialKind } from '../../state/store';

export interface SurfaceMaterialControlProps {
  /**
   * `compact` renders the abbreviated labels used in the bottom ruler bar;
   * `full` renders the spelled-out labels used in the Options drawer.
   */
  variant?: 'compact' | 'full';
}

const KINDS: ReadonlyArray<{ id: MaterialKind; short: string; full: string }> = [
  { id: 'default', short: 'DEF', full: 'Default' },
  { id: 'matte', short: 'MAT', full: 'Matte' },
  { id: 'wireframe', short: 'WIRE', full: 'Wireframe' },
];

export function SurfaceMaterialControl(props: SurfaceMaterialControlProps): ReactElement {
  const viewport = useUiState(selectViewport);
  const variant = props.variant ?? 'full';
  return (
    <div
      className={`surface-material surface-material-${variant}`}
      role="radiogroup"
      aria-label="Surface material"
    >
      {KINDS.map((kind) => {
        const active = viewport.material === kind.id;
        return (
          <button
            key={kind.id}
            type="button"
            role="radio"
            aria-checked={active}
            aria-label={kind.full}
            title={`${kind.full} surface`}
            data-material={kind.id}
            className={`surface-material-button${active ? ' is-active' : ''}`}
            onClick={() => dispatch({ type: 'viewport/set', patch: { material: kind.id } })}
          >
            {variant === 'compact' ? kind.short : kind.full}
          </button>
        );
      })}
    </div>
  );
}
