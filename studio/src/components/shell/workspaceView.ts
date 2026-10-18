// Workspace view mode.
//
// Kala Studio is chat-first: the Mechanical Engineering Agent owns the main
// viewport and the OpenCascade workbench (3D preview, parametric sizing,
// nominal geometry estimates, STEP/STL handoff) lives in the right rail.
//
// `canvas` is the optional inspection mode — the 3D canvas expands into the
// main viewport for close mesh work, and the rail drops its docked preview
// so only one WebGL context is ever live.
//
// This is deliberately UI-local state rather than store state: it is a view
// preference with no bearing on geometry, and keeping it out of the store
// means the preview controller can never be woken by a view change.

export type WorkspaceViewMode = 'agent' | 'canvas';

export const DEFAULT_VIEW_MODE: WorkspaceViewMode = 'agent';

export interface ViewModeOption {
  id: WorkspaceViewMode;
  label: string;
  description: string;
}

export const VIEW_MODE_OPTIONS: ReadonlyArray<ViewModeOption> = [
  {
    id: 'agent',
    label: 'Agent + Workbench',
    description:
      'Mechanical engineering agent in the viewport, OpenCascade 3D preview and sizing in the rail.',
  },
  {
    id: 'canvas',
    label: '3D Canvas Focus',
    description: 'Expand the 3D OpenCascade canvas into the viewport for close mesh inspection.',
  },
];