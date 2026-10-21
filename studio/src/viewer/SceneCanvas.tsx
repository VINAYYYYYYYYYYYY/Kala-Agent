// Three.js viewport for the Vellum drafting sheet.
//
// Rendering contract:
//   * `pencil2d`  — flat vellum faces, 2B graphite visible edges, dashed 2H
//                   hidden edges, narrow FOV for an orthographic feel;
//   * `clay3d`    — matte warm clay (roughness 1) with soft hemisphere light
//                   and the same graphite edge overlay;
//   * grid/axes   — pencil colours read from the theme tokens;
//   * dimensions  — real mesh bounds only, redline callouts projected from
//                   the 3D bounds corners;
//   * section     — a redline cutting plane at the real Z mid-plane when
//                   `showSection` is on.
//
// The scene background is transparent for the paper/dark viewport settings
// so the vellum sheet (and its grain/grid) shows through. Only the explicit
// `studio` background paints an opaque colour behind the model.
//
// Render-on-demand: nothing animates, so `prefers-reduced-motion` needs no
// special loop. Every camera change, mesh change, or preference change runs
// a single render; OrbitControls damping is intentionally disabled.
//
// Camera ownership: the operator owns the camera. A mesh re-frame happens
// only when the mesh bounds actually change (or on the first mesh), so an
// unrelated workspace re-render never steals the view back. `resetView`
// re-frames on demand.
//
// Disposal walks every geometry and material that was ever attached,
// including Lines, GridHelper, and AxesHelper, so changing the mesh,
// render mode, theme, or unmounting releases GPU resources.

import { useEffect, useRef } from 'react';
import type { KeyboardEvent, ReactElement } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import type {
  BBox,
  CadProviderId,
  PreviewResponse,
  RenderMode,
  StudioLocalPreview,
} from '../types/studio';
import { boundsDimensions, formatMillimetres } from './localPrimitives';
import { designViewLabel } from './designSource';
import { chainDashSegments, computeCentreLineAxes, shouldReframeBounds } from './draftingGeometry';
import { PRIMITIVE_SECTION_CUT, sectionCutCorners, type SectionCut } from '../state/sectionCut';
import { formatMeasureDistance, measureDistanceMm } from './measure';
import type { MeasurePoint, MeasureState } from './measure';
import { pointerToNdc, raycastFacePoint } from './raycast';
import { VIEWPORT_KEY_HELP, viewportCommand, VIEWPORT_MAX_POLAR_RAD, VIEWPORT_MIN_POLAR_RAD } from './viewportKeys';
import type { ViewportCommand } from './viewportKeys';
import type { MaterialKind, BackgroundKind } from '../state/store';

export interface SceneCanvasProps {
  mesh: StudioLocalPreview | PreviewResponse | null;
  meshSource: 'local' | 'server' | 'design' | 'none';
  /** True when `mesh` is the OpenCascade import of a provider's STEP. */
  designMesh?: boolean;
  /** The provider that built the active design, when there is one. */
  designProvider?: CadProviderId | null;
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
  /** 2D pencil drafting or 3D clay solid. */
  renderMode: RenderMode;
  /** Draw the redline A–A cutting plane. */
  showSection: boolean;
  /** The shared cutting plane; defaults to the primitive Z mid-plane. */
  sectionCut?: SectionCut;
  /** Increments when the operator asks to re-frame the camera. */
  viewResetToken: number;
  /** The measure tool: armed or not, plus the picked points (model mm). */
  measure: MeasureState;
  /** Called with the real raycast hit in model mm, or null when the pick missed. */
  onMeasurePoint(point: MeasurePoint | null): void;
}

interface DraftingPalette {
  sheet: string;
  ink: string;
  construction: string;
  gridMajor: string;
  gridMinor: string;
  redline: string;
  pencilFace: string;
  pencilShade: string;
  clayFace: string;
  clayShade: string;
}

/** One HTML label projected from a world point onto the canvas. */
interface OverlayLabel {
  element: HTMLDivElement;
  world: THREE.Vector3;
}

interface SceneState {
  applyMesh(
    mesh: StudioLocalPreview | PreviewResponse | null,
    source: 'local' | 'server' | 'design' | 'none',
    designMesh: boolean,
  ): void;
  applyViewport(next: ViewportSnapshot): void;
  applyRenderMode(mode: RenderMode): void;
  applySection(show: boolean): void;
  applySectionCut(cut: SectionCut): void;
  applyMeasure(state: MeasureState): void;
  /** Apply one keyboard command (orbit, zoom, reset) to the live camera. */
  applyCommand(command: ViewportCommand): void;
  resetView(): void;
  renderNow(): void;
  handleResize(): void;
  dispose(): void;
}

interface ViewportSnapshot {
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
}

const GRID_SIZE_MM = 1000;
const GRID_DIVISIONS = 50;
const BACKGROUND_COLORS: Record<BackgroundKind, number> = {
  studio: 0x14181f,
  paper: 0xeef2f6,
  dark: 0x05070a,
};

// Vellum fallbacks in case the CSS custom properties are unavailable (a
// test DOM or the very first paint before tokens.css is applied).
const FALLBACK_PALETTE: DraftingPalette = {
  sheet: '#f6f3eb',
  ink: '#23211e',
  construction: '#7a756c',
  gridMajor: '#dfdbd0',
  gridMinor: '#eae6dc',
  redline: '#c84b31',
  pencilFace: '#f1ead9',
  pencilShade: '#e4dcc7',
  clayFace: '#c98f6b',
  clayShade: '#a96b4b',
};

export function SceneCanvas(props: SceneCanvasProps): ReactElement {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const overlayHostRef = useRef<HTMLDivElement | null>(null);
  const sceneStateRef = useRef<SceneState | null>(null);
  // The pick callback is read through a ref so the scene never captures a
  // stale closure over the sheet's props.
  const measurePointRef = useRef(props.onMeasurePoint);

  useEffect(() => {
    measurePointRef.current = props.onMeasurePoint;
  }, [props.onMeasurePoint]);

  useEffect(() => {
    const container = containerRef.current;
    const overlayHost = overlayHostRef.current;
    if (!container || !overlayHost) return;
    let state: SceneState | null = null;
    try {
      state = createScene(container, overlayHost, (point) => measurePointRef.current(point));
    } catch (cause) {
      // WebGL may be unavailable in headless environments or restricted
      // contexts. Render a graceful fallback rather than letting the
      // error bubble through React.
      renderWebGLFallback(container, cause);
      return;
    }
    sceneStateRef.current = state;
    const observer = new ResizeObserver(() => {
      state!.handleResize();
      state!.renderNow();
    });
    observer.observe(container);
    return () => {
      observer.disconnect();
      state!.dispose();
      sceneStateRef.current = null;
    };
  }, []);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applyMesh(props.mesh, props.meshSource, props.designMesh === true);
  }, [props.mesh, props.meshSource, props.designMesh]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applyViewport({
      showGrid: props.showGrid,
      showAxes: props.showAxes,
      showDimensions: props.showDimensions,
      material: props.material,
      background: props.background,
    });
  }, [props.showGrid, props.showAxes, props.showDimensions, props.material, props.background]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applyRenderMode(props.renderMode);
  }, [props.renderMode]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applySection(props.showSection);
  }, [props.showSection]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applySectionCut(props.sectionCut ?? PRIMITIVE_SECTION_CUT);
  }, [props.sectionCut]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state) return;
    state.applyMeasure(props.measure);
  }, [props.measure]);

  useEffect(() => {
    const state = sceneStateRef.current;
    if (!state || props.viewResetToken === 0) return;
    state.resetView();
  }, [props.viewResetToken]);

  // The keyboard contract of the viewport. The keys are handled here, on the
  // focused container, and are applied to the same camera the pointer drives.
  // A key the viewport does not own is left to the browser.
  function onKeyDown(event: KeyboardEvent<HTMLDivElement>): void {
    const command = viewportCommand(event.key);
    if (!command) return;
    // The viewport owns the arrow keys while it is focused: the page must
    // not scroll behind it.
    event.preventDefault();
    const state = sceneStateRef.current;
    if (!state) return;
    state.applyCommand(command);
  }

  const viewportName = props.renderMode === 'pencil2d' ? '2D pencil' : '3D clay';
  const viewportSubject = props.designMesh === true ? 'design' : 'primitive';

  return (
    <div
      className="viewer-canvas"
      ref={containerRef}
      // The viewport is an interactive widget with its own key handling, not
      // a static picture: `application` says so, and the label names every
      // key it owns.
      role="application"
      tabIndex={0}
      onKeyDown={onKeyDown}
      data-design-mesh={props.designMesh === true ? 'true' : 'false'}
      aria-label={`${viewportName} viewport of the current ${viewportSubject}. ${VIEWPORT_KEY_HELP}`}
    >
      <div className="viewer-canvas-overlay" ref={overlayHostRef} aria-hidden="true" />
      <div className="viewer-canvas-source" aria-live="polite">
        {props.meshSource === 'design'
          ? designViewLabel(props.designProvider)
          : props.meshSource === 'server'
            ? 'Server verified mesh'
            : props.meshSource === 'local'
              ? 'Local preview — pending server verification'
              : 'No mesh'}
      </div>
    </div>
  );
}

function createScene(
  container: HTMLDivElement,
  overlayHost: HTMLDivElement,
  onMeasurePoint: (point: MeasurePoint | null) => void,
): SceneState {
  const scene = new THREE.Scene();
  scene.background = null;

  const camera = new THREE.PerspectiveCamera(24, 1, 0.1, 100_000);
  camera.position.set(220, 180, 260);
  camera.up.set(0, 1, 0);

  const renderer = new THREE.WebGLRenderer({
    antialias: true,
    alpha: true,
    powerPreference: 'high-performance',
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0x000000, 0);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.NoToneMapping;
  container.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = false;
  controls.minDistance = 1;
  controls.maxDistance = 10_000;
  controls.target.set(0, 0, 0);

  const ambient = new THREE.AmbientLight(0xffffff, 0.85);
  scene.add(ambient);
  const hemisphere = new THREE.HemisphereLight(0xfff4e4, 0x4a4038, 0.25);
  scene.add(hemisphere);
  const key = new THREE.DirectionalLight(0xffffff, 0.35);
  key.position.set(400, 600, 300);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xd8d2c6, 0.18);
  fill.position.set(-300, 200, -200);
  scene.add(fill);

  let palette = readPalette();
  let paletteKey = JSON.stringify(palette);
  let renderMode: RenderMode = 'pencil2d';
  let materialKind: MaterialKind = 'default';
  let currentBounds: BBox | null = null;
  // The bounds the camera is currently framed on. A new mesh with the same
  // bounds does not re-frame: the operator keeps the camera they set.
  let framedBounds: BBox | null = null;
  let sectionVisible = true;
  let sectionCut: SectionCut = PRIMITIVE_SECTION_CUT;
  let grid: THREE.GridHelper | null = null;
  let axes: THREE.Group | null = null;
  const meshGroup: { current: THREE.Group | null } = { current: null };
  // The face mesh is the raycast target for the measure tool.
  let faceMesh: THREE.Mesh | null = null;
  let dimensionGroup: { current: THREE.Group | null; labels: OverlayLabel[] } = {
    current: null,
    labels: [],
  };
  let dimensionHost: HTMLDivElement | null = null;
  let sectionGroup: { current: THREE.Group | null } = { current: null };
  const measureGroup: { current: THREE.Group | null } = { current: null };
  const centreGroup: { current: THREE.Group | null } = { current: null };
  let measureLabels: OverlayLabel[] = [];
  let centreLabels: OverlayLabel[] = [];
  let measureHost: HTMLDivElement | null = null;
  let centreHost: HTMLDivElement | null = null;
  let measureState: MeasureState = { active: false, points: [] };
  let pointerStart: { x: number; y: number } | null = null;
  let viewport: ViewportSnapshot = {
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default',
    background: 'paper',
  };
  let disposed = false;

  grid = buildGrid(palette);
  scene.add(grid);
  axes = buildAxes(palette);
  scene.add(axes);

  const themeObserver = new MutationObserver(() => {
    refreshPalette();
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme'],
  });

  controls.addEventListener('change', renderNow);

  // --- Measure picking ----------------------------------------------------
  //
  // A pick is a real raycast against the face mesh, so the point is in
  // model millimetres. A drag is an orbit, not a pick, so a pointer that
  // moved more than a few pixels between press and release is ignored.

  function onPointerDown(event: PointerEvent): void {
    pointerStart = { x: event.clientX, y: event.clientY };
  }

  function onPointerUp(event: PointerEvent): void {
    const start = pointerStart;
    pointerStart = null;
    if (!start) return;
    if (!measureState.active) return;
    const moved = Math.hypot(event.clientX - start.x, event.clientY - start.y);
    if (moved > 4) return;
    const rect = renderer.domElement.getBoundingClientRect();
    const ndc = pointerToNdc(event.clientX, event.clientY, rect);
    const point = raycastFacePoint(camera, faceMesh, ndc);
    onMeasurePoint(point);
  }

  renderer.domElement.addEventListener('pointerdown', onPointerDown);
  renderer.domElement.addEventListener('pointerup', onPointerUp);

  function renderNow(): void {
    if (disposed) return;
    camera.updateMatrixWorld();
    renderer.render(scene, camera);
    updateOverlayLabelPositions();
  }

  function readPalette(): DraftingPalette {
    if (typeof window === 'undefined' || typeof document === 'undefined') {
      return { ...FALLBACK_PALETTE };
    }
    const style = getComputedStyle(document.documentElement);
    const read = (name: string, fallback: string): string => {
      const value = style.getPropertyValue(name).trim();
      return value || fallback;
    };
    return {
      sheet: read('--sheet', FALLBACK_PALETTE.sheet),
      ink: read('--ink', FALLBACK_PALETTE.ink),
      construction: read('--construction', FALLBACK_PALETTE.construction),
      gridMajor: read('--grid-major', FALLBACK_PALETTE.gridMajor),
      gridMinor: read('--grid-minor', FALLBACK_PALETTE.gridMinor),
      redline: read('--redline', FALLBACK_PALETTE.redline),
      pencilFace: read('--drafting-pencil-face', FALLBACK_PALETTE.pencilFace),
      pencilShade: read('--drafting-pencil-shade', FALLBACK_PALETTE.pencilShade),
      clayFace: read('--drafting-clay-face', FALLBACK_PALETTE.clayFace),
      clayShade: read('--drafting-clay-shade', FALLBACK_PALETTE.clayShade),
    };
  }

  function refreshPalette(): void {
    const next = readPalette();
    const nextKey = JSON.stringify(next);
    if (nextKey === paletteKey) return;
    palette = next;
    paletteKey = nextKey;
    rebuildGrid();
    rebuildAxes();
    refreshFaceMaterial();
    recolorEdgeMaterials();
    recolorSection();
    recolorDimensions();
    recolorCentreLines();
    renderNow();
  }

  function rebuildGrid(): void {
    if (grid) {
      scene.remove(grid);
      disposeObject(grid);
    }
    grid = buildGrid(palette);
    grid.visible = viewport.showGrid;
    if (currentBounds) grid.position.y = currentBounds.min[1] - Math.max(0.5, gridOffset());
    scene.add(grid);
  }

  function rebuildAxes(): void {
    if (axes) {
      scene.remove(axes);
      disposeObject(axes);
    }
    axes = buildAxes(palette);
    axes.visible = viewport.showAxes;
    scene.add(axes);
  }

  function buildGrid(next: DraftingPalette): THREE.GridHelper {
    const helper = new THREE.GridHelper(
      GRID_SIZE_MM,
      GRID_DIVISIONS,
      new THREE.Color(next.gridMajor),
      new THREE.Color(next.gridMinor),
    );
    const material = helper.material as THREE.Material | THREE.Material[];
    const mats = Array.isArray(material) ? material : [material];
    for (const mat of mats) {
      mat.transparent = true;
      mat.opacity = 0.75;
      mat.depthWrite = false;
    }
    helper.renderOrder = -1;
    return helper;
  }

  function buildAxes(next: DraftingPalette): THREE.Group {
    const group = new THREE.Group();
    const length = axisLength();
    const solid = new THREE.LineBasicMaterial({ color: new THREE.Color(next.ink), transparent: true, opacity: 0.85 });
    const dashed = new THREE.LineDashedMaterial({
      color: new THREE.Color(next.construction),
      transparent: true,
      opacity: 0.9,
      dashSize: Math.max(length * 0.06, 1),
      gapSize: Math.max(length * 0.03, 0.5),
    });
    const xLine = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(-length, 0, 0), new THREE.Vector3(length, 0, 0)]),
      solid,
    );
    const yLine = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(0, -length, 0), new THREE.Vector3(0, length, 0)]),
      dashed.clone(),
    );
    yLine.computeLineDistances();
    const zLine = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(0, 0, -length), new THREE.Vector3(0, 0, length)]),
      dashed,
    );
    zLine.computeLineDistances();
    group.add(xLine, yLine, zLine);
    return group;
  }

  function axisLength(): number {
    if (!currentBounds) return 120;
    const dims = boundsDimensions(currentBounds);
    return Math.max(dims.width, dims.height, dims.depth, 1) * 0.85;
  }

  function gridOffset(): number {
    if (!currentBounds) return 0;
    return Math.max(boundsDimensions(currentBounds).width, 1) * 0.02;
  }

  function refreshFaceMaterial(): void {
    if (!meshGroup.current) return;
    const face = meshGroup.current.children.find(
      (child): child is THREE.Mesh => child instanceof THREE.Mesh,
    );
    if (!face) return;
    const previous = face.material;
    face.material = makeFaceMaterial();
    disposeMaterial(previous);
  }

  function makeFaceMaterial(): THREE.Material {
    const polygonOffset = { polygonOffset: true, polygonOffsetFactor: 1, polygonOffsetUnits: 1 };
    if (materialKind === 'wireframe') {
      return new THREE.MeshBasicMaterial({
        color: new THREE.Color(palette.ink),
        wireframe: true,
        transparent: true,
        opacity: 0.8,
      });
    }
    if (renderMode === 'pencil2d') {
      const color = materialKind === 'matte' ? palette.pencilShade : palette.pencilFace;
      return new THREE.MeshLambertMaterial({
        color: new THREE.Color(color),
        side: THREE.FrontSide,
        ...polygonOffset,
      });
    }
    const color = materialKind === 'matte' ? palette.clayShade : palette.clayFace;
    return new THREE.MeshStandardMaterial({
      color: new THREE.Color(color),
      roughness: 1,
      metalness: 0,
      side: THREE.FrontSide,
      ...polygonOffset,
    });
  }

  function makeEdgeMaterial(): THREE.LineBasicMaterial {
    return new THREE.LineBasicMaterial({
      color: new THREE.Color(palette.ink),
      transparent: true,
      opacity: 0.95,
    });
  }

  function makeHiddenEdgeMaterial(radius: number): THREE.LineDashedMaterial {
    const dash = Math.max(radius * 0.055, 0.4);
    return new THREE.LineDashedMaterial({
      color: new THREE.Color(palette.construction),
      transparent: true,
      opacity: 0.6,
      depthFunc: THREE.GreaterDepth,
      depthWrite: false,
      dashSize: dash,
      gapSize: dash * 0.65,
    });
  }

  function makeConstructionMaterial(): THREE.LineBasicMaterial {
    return new THREE.LineBasicMaterial({
      color: new THREE.Color(palette.construction),
      transparent: true,
      opacity: 0.16,
    });
  }

  function recolorEdgeMaterials(): void {
    if (!meshGroup.current) return;
    for (const child of meshGroup.current.children) {
      if (child instanceof THREE.LineSegments) {
        const material = child.material;
        if (material instanceof THREE.LineDashedMaterial) {
          material.color.set(palette.construction);
          material.needsUpdate = true;
        } else if (material instanceof THREE.LineBasicMaterial) {
          material.color.set(
            material.opacity < 0.2 ? palette.construction : palette.ink,
          );
          material.needsUpdate = true;
        }
      }
    }
  }

  function applyLighting(): void {
    if (renderMode === 'pencil2d') {
      // Three's Lambert BRDF divides irradiance by π, so the ambient term
      // needs to be around π to read as an almost-flat vellum face rather
      // than a grey solid.
      ambient.intensity = 2.5;
      hemisphere.intensity = 0.35;
      key.intensity = 0.6;
      fill.intensity = 0.2;
    } else {
      ambient.intensity = 0.4;
      hemisphere.intensity = 1.9;
      key.intensity = 1.0;
      fill.intensity = 0.2;
    }
    renderer.toneMapping =
      renderMode === 'clay3d' ? THREE.ACESFilmicToneMapping : THREE.NoToneMapping;
    renderer.toneMappingExposure = 1.05;
  }

  function applyMesh(
    mesh: StudioLocalPreview | PreviewResponse | null,
    source: 'local' | 'server' | 'design' | 'none',
    designMesh: boolean,
  ): void {
    if (!mesh) {
      disposeMeshGroup();
      clearDimensions();
      disposeSection();
      disposeCentreLines();
      currentBounds = null;
      framedBounds = null;
      renderNow();
      return;
    }
    const geometry = meshToGeometry(mesh);
    if (!geometry) {
      disposeMeshGroup();
      clearDimensions();
      disposeSection();
      disposeCentreLines();
      currentBounds = null;
      framedBounds = null;
      renderNow();
      return;
    }
    disposeMeshGroup();
    const bounds = mesh.bounds;
    const dims = boundsDimensions(bounds);
    const radius = Math.max(dims.width, dims.height, dims.depth, 1) / 2;
    const kind = 'kind' in mesh ? mesh.kind : null;

    const group = new THREE.Group();
    const face = new THREE.Mesh(geometry, makeFaceMaterial());
    group.add(face);
    faceMesh = face;

    const edgeGeometry = new THREE.EdgesGeometry(geometry, 18);
    const visibleEdges = new THREE.LineSegments(edgeGeometry, makeEdgeMaterial());
    visibleEdges.renderOrder = 2;
    group.add(visibleEdges);

    const hiddenGeometry = new THREE.EdgesGeometry(geometry, 18);
    const hiddenEdges = new THREE.LineSegments(
      hiddenGeometry,
      makeHiddenEdgeMaterial(radius),
    );
    hiddenEdges.computeLineDistances();
    hiddenEdges.renderOrder = 1;
    group.add(hiddenEdges);

    // Curved primitives have no hard silhouette in EdgesGeometry. A very
    // faint wireframe pass supplies the missing construction lines without
    // turning the clay surface into a wireframe render. A compiled design
    // mesh is a triangle soup with a real HLR drawing of its own, so it
    // never gets the primitive construction pass.
    if (!designMesh && (kind === 'sphere' || kind === 'cylinder')) {
      const construction = new THREE.LineSegments(
        new THREE.WireframeGeometry(geometry),
        makeConstructionMaterial(),
      );
      construction.renderOrder = 0;
      group.add(construction);
    }

    scene.add(group);
    meshGroup.current = group;
    currentBounds = bounds;
    if (grid) grid.position.y = bounds.min[1] - Math.max(0.5, gridOffset());
    if (axes) {
      scene.remove(axes);
      disposeObject(axes);
      axes = buildAxes(palette);
      axes.visible = viewport.showAxes;
      scene.add(axes);
    }
    // Re-frame only when the bounds really changed (or on the first mesh).
    // A rebuild with the same bounds keeps the operator's camera, so a
    // journal entry, a tab switch, or a drawer opening never snaps the
    // view back to the default framing.
    if (shouldReframeBounds(framedBounds, bounds)) {
      frameOnBounds(bounds);
      framedBounds = bounds;
    }
    rebuildDimensions();
    rebuildSection();
    rebuildCentreLines();
    renderNow();
    void source;
  }

  function frameOnBounds(bounds: BBox): void {
    const min = new THREE.Vector3(bounds.min[0], bounds.min[1], bounds.min[2]);
    const max = new THREE.Vector3(bounds.max[0], bounds.max[1], bounds.max[2]);
    const centre = min.clone().add(max).multiplyScalar(0.5);
    const size = max.clone().sub(min);
    const radius = Math.max(size.x, size.y, size.z, 1) / 2;
    const fov = renderMode === 'pencil2d' ? 24 : 38;
    const margin = renderMode === 'pencil2d' ? 1.7 : 1.55;
    const distance = (radius / Math.tan((fov * Math.PI) / 360)) * margin;
    controls.target.copy(centre);
    camera.position.copy(
      centre.clone().add(new THREE.Vector3(distance * 0.82, distance * 0.62, distance * 0.9)),
    );
    controls.update();
  }

  function applyViewport(next: ViewportSnapshot): void {
    const materialChanged = next.material !== viewport.material;
    const axesChanged = next.showAxes !== viewport.showAxes;
    viewport = next;
    materialKind = next.material;
    if (grid) grid.visible = next.showGrid;
    if (axes) axes.visible = next.showAxes;
    // Paper and dark remain transparent so the drafting sheet behind the
    // canvas supplies the real theme surface. `studio` keeps the old opaque
    // technical backdrop.
    scene.background =
      next.background === 'studio' ? new THREE.Color(BACKGROUND_COLORS.studio) : null;
    renderer.setClearAlpha(next.background === 'studio' ? 1 : 0);
    if (dimensionHost) {
      dimensionHost.style.display = next.showDimensions ? '' : 'none';
      if (dimensionGroup.current) {
        dimensionGroup.current.visible = next.showDimensions;
      }
    }
    // The drafted centre lines follow the Axes toggle.
    if (axesChanged) rebuildCentreLines();
    if (materialChanged) refreshFaceMaterial();
    renderNow();
  }

  function applyRenderMode(mode: RenderMode): void {
    if (mode === renderMode) return;
    renderMode = mode;
    camera.fov = mode === 'pencil2d' ? 24 : 38;
    camera.updateProjectionMatrix();
    applyLighting();
    refreshFaceMaterial();
    if (currentBounds) frameOnBounds(currentBounds);
    renderNow();
  }

  function applySection(show: boolean): void {
    sectionVisible = show;
    if (sectionGroup.current) sectionGroup.current.visible = show;
    renderNow();
  }

  function applySectionCut(cut: SectionCut): void {
    sectionCut = cut;
    rebuildSection();
    renderNow();
  }

  function applyMeasure(next: MeasureState): void {
    measureState = next;
    container.classList.toggle('is-measuring', next.active);
    rebuildMeasure();
    renderNow();
  }

  function resetView(): void {
    if (currentBounds) {
      frameOnBounds(currentBounds);
      // The camera is now framed on these bounds, so a later mesh with the
      // same bounds must not re-frame it again.
      framedBounds = currentBounds;
    }
    renderNow();
  }

  // --- Keyboard camera ----------------------------------------------------
  //
  // The pointer orbits through OrbitControls; the keyboard acts on the same
  // camera directly, in its own spherical coordinates, so both routes agree.

  function applyCommand(command: ViewportCommand): void {
    if (command.kind === 'reset') {
      resetView();
      return;
    }
    const offset = camera.position.clone().sub(controls.target);
    if (offset.lengthSq() === 0) return;
    const spherical = new THREE.Spherical().setFromVector3(offset);
    if (command.kind === 'orbit') {
      spherical.theta += command.azimuth;
      spherical.phi = THREE.MathUtils.clamp(
        spherical.phi + command.polar,
        VIEWPORT_MIN_POLAR_RAD,
        VIEWPORT_MAX_POLAR_RAD,
      );
    } else {
      spherical.radius = THREE.MathUtils.clamp(
        spherical.radius * command.factor,
        controls.minDistance,
        controls.maxDistance,
      );
    }
    camera.position.copy(controls.target).add(new THREE.Vector3().setFromSpherical(spherical));
    controls.update();
    renderNow();
  }

  // --- Dimensions ---------------------------------------------------------

  function rebuildDimensions(): void {
    clearDimensions();
    if (!currentBounds) return;
    const dims = boundsDimensions(currentBounds);
    const pad = Math.max(Math.max(dims.width, dims.height, dims.depth) * 0.12, 1);
    const [x0, y0, z0] = currentBounds.min;
    const [x1, y1, z1] = currentBounds.max;
    const points: THREE.Vector3[] = [];
    const labelPoints: Array<{ text: string; world: THREE.Vector3 }> = [];

    // Width: below the front-bottom edge, along X.
    points.push(
      new THREE.Vector3(x0, y0 - pad, z1),
      new THREE.Vector3(x1, y0 - pad, z1),
      new THREE.Vector3(x0, y0, z1),
      new THREE.Vector3(x0, y0 - pad * 1.15, z1),
      new THREE.Vector3(x1, y0, z1),
      new THREE.Vector3(x1, y0 - pad * 1.15, z1),
    );
    labelPoints.push({
      text: `W ${formatMillimetres(dims.width)}`,
      world: new THREE.Vector3((x0 + x1) / 2, y0 - pad, z1),
    });

    // Height: to the right of the front-bottom edge, along Y.
    points.push(
      new THREE.Vector3(x1 + pad, y0, z1),
      new THREE.Vector3(x1 + pad, y1, z1),
      new THREE.Vector3(x1, y0, z1),
      new THREE.Vector3(x1 + pad * 1.15, y0, z1),
      new THREE.Vector3(x1, y1, z1),
      new THREE.Vector3(x1 + pad * 1.15, y1, z1),
    );
    labelPoints.push({
      text: `H ${formatMillimetres(dims.height)}`,
      world: new THREE.Vector3(x1 + pad, (y0 + y1) / 2, z1),
    });

    // Depth: bottom-right, along Z.
    points.push(
      new THREE.Vector3(x1 + pad, y0 - pad, z0),
      new THREE.Vector3(x1 + pad, y0 - pad, z1),
      new THREE.Vector3(x1, y0, z0),
      new THREE.Vector3(x1 + pad * 1.15, y0 - pad * 1.15, z0),
      new THREE.Vector3(x1, y0, z1),
      new THREE.Vector3(x1 + pad * 1.15, y0 - pad * 1.15, z1),
    );
    labelPoints.push({
      text: `D ${formatMillimetres(dims.depth)}`,
      world: new THREE.Vector3(x1 + pad, y0 - pad, (z0 + z1) / 2),
    });

    const group = new THREE.Group();
    const geometry = new THREE.BufferGeometry().setFromPoints(points);
    const lines = new THREE.LineSegments(geometry, makeDimensionMaterial());
    group.add(lines);
    scene.add(group);
    dimensionGroup.current = group;
    group.visible = viewport.showDimensions;

    dimensionHost = document.createElement('div');
    dimensionHost.className = 'viewer-dims';
    dimensionHost.style.display = viewport.showDimensions ? '' : 'none';
    overlayHost.appendChild(dimensionHost);
    dimensionGroup.labels = labelPoints.map(({ text, world }) => {
      const element = document.createElement('div');
      element.className = 'viewer-dim-label';
      element.textContent = text;
      dimensionHost!.appendChild(element);
      return { element, world };
    });
  }

  function makeDimensionMaterial(): THREE.LineBasicMaterial {
    return new THREE.LineBasicMaterial({
      color: new THREE.Color(palette.redline),
      transparent: true,
      opacity: 0.9,
      depthTest: false,
    });
  }

  function recolorDimensions(): void {
    if (!dimensionGroup.current) return;
    for (const child of dimensionGroup.current.children) {
      if (child instanceof THREE.LineSegments) {
        const material = child.material;
        if (material instanceof THREE.LineBasicMaterial) material.color.set(palette.redline);
      }
    }
  }

  function updateOverlayLabelPositions(): void {
    const width = Math.max(container.clientWidth, 1);
    const height = Math.max(container.clientHeight, 1);
    const labels = [
      ...dimensionGroup.labels,
      ...measureLabels,
      ...centreLabels,
    ];
    for (const label of labels) {
      const projected = label.world.clone().project(camera);
      const visible = projected.z >= -1 && projected.z <= 1;
      label.element.style.display = visible ? '' : 'none';
      if (!visible) continue;
      const x = (projected.x * 0.5 + 0.5) * width;
      const y = (-projected.y * 0.5 + 0.5) * height;
      label.element.style.left = `${x}px`;
      label.element.style.top = `${y}px`;
    }
  }

  function clearDimensions(): void {
    if (dimensionGroup.current) {
      scene.remove(dimensionGroup.current);
      disposeObject(dimensionGroup.current);
      dimensionGroup.current = null;
    }
    dimensionGroup.labels = [];
    if (dimensionHost) {
      dimensionHost.remove();
      dimensionHost = null;
    }
  }

  // --- Centre lines --------------------------------------------------------
  //
  // Two chain-dash (— · —) lines through the part centre, with the drafted
  // `+X (WIDTH)` / `+Y (HEIGHT)` labels. They follow the Axes toggle and
  // are drawn from the real mesh bounds, so they always run through the
  // middle of the part that is on screen.

  function rebuildCentreLines(): void {
    disposeCentreLines();
    if (!viewport.showAxes || !currentBounds) return;
    const dims = boundsDimensions(currentBounds);
    const span = Math.max(dims.width, dims.height, dims.depth, 1);
    const pad = span * 0.18;
    const long = Math.max(span * 0.12, 1);
    const short = Math.max(long * 0.22, 0.4);
    const axesToDraw = computeCentreLineAxes(currentBounds, pad);
    const points: THREE.Vector3[] = [];
    for (const axis of axesToDraw) {
      const dashes = chainDashSegments(axis.from, axis.to, [long, short, short, short]);
      for (const [from, to] of dashes) {
        points.push(
          new THREE.Vector3(from.x, from.y, from.z),
          new THREE.Vector3(to.x, to.y, to.z),
        );
      }
    }
    if (points.length === 0) return;
    const group = new THREE.Group();
    const material = new THREE.LineBasicMaterial({
      color: new THREE.Color(palette.redline),
      transparent: true,
      opacity: 0.85,
      depthTest: false,
    });
    const line = new THREE.LineSegments(
      new THREE.BufferGeometry().setFromPoints(points),
      material,
    );
    line.renderOrder = 2;
    group.add(line);
    scene.add(group);
    centreGroup.current = group;

    centreHost = document.createElement('div');
    centreHost.className = 'viewer-centre';
    overlayHost.appendChild(centreHost);
    for (const axis of axesToDraw) {
      const element = document.createElement('div');
      element.className = 'viewer-centre-label';
      element.textContent = axis.label;
      centreHost.appendChild(element);
      centreLabels.push({
        element,
        world: new THREE.Vector3(axis.to.x, axis.to.y, axis.to.z),
      });
    }
  }

  function recolorCentreLines(): void {
    if (!centreGroup.current) return;
    for (const child of centreGroup.current.children) {
      if (child instanceof THREE.LineSegments) {
        const material = child.material;
        if (material instanceof THREE.LineBasicMaterial) material.color.set(palette.redline);
      }
    }
  }

  function disposeCentreLines(): void {
    if (centreGroup.current) {
      scene.remove(centreGroup.current);
      disposeObject(centreGroup.current);
      centreGroup.current = null;
    }
    for (const label of centreLabels) label.element.remove();
    centreLabels = [];
    if (centreHost) {
      centreHost.remove();
      centreHost = null;
    }
  }

  // --- Measure tool --------------------------------------------------------
  //
  // The leader is drawn between the two picked points and the label carries
  // the real distance in millimetres. Both points come from a raycast
  // against the face mesh, so the number is a model length.

  function rebuildMeasure(): void {
    disposeMeasure();
    const points = measureState.points;
    if (points.length < 2) return;
    const first = points[0]!;
    const second = points[1]!;
    const a = new THREE.Vector3(first.x, first.y, first.z);
    const b = new THREE.Vector3(second.x, second.y, second.z);
    const group = new THREE.Group();
    const material = new THREE.LineBasicMaterial({
      color: new THREE.Color(palette.redline),
      transparent: true,
      opacity: 0.95,
      depthTest: false,
    });
    const leader = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([a, b]),
      material,
    );
    leader.renderOrder = 4;
    group.add(leader);

    // Short jaws at both ends, so the two picked points are readable on the
    // drawing.
    const direction = b.clone().sub(a);
    const length = direction.length();
    if (length > 0) {
      const unit = direction.clone().normalize();
      const jaw = unit.clone().cross(new THREE.Vector3(0, 1, 0));
      if (jaw.length() < 1e-6) jaw.set(1, 0, 0);
      jaw.normalize().multiplyScalar(length * 0.02);
      const jawPoints = [
        a.clone().sub(jaw), a.clone().add(jaw),
        b.clone().sub(jaw), b.clone().add(jaw),
      ];
      const jaws = new THREE.LineSegments(
        new THREE.BufferGeometry().setFromPoints(jawPoints),
        material,
      );
      jaws.renderOrder = 4;
      group.add(jaws);
    }
    scene.add(group);
    measureGroup.current = group;

    measureHost = document.createElement('div');
    measureHost.className = 'viewer-measure';
    overlayHost.appendChild(measureHost);
    const element = document.createElement('div');
    element.className = 'viewer-measure-label';
    const distance = measureDistanceMm(measureState);
    element.textContent = formatMeasureDistance(distance ?? Number.NaN);
    measureHost.appendChild(element);
    measureLabels.push({
      element,
      world: a.clone().add(b).multiplyScalar(0.5),
    });
  }

  function disposeMeasure(): void {
    if (measureGroup.current) {
      scene.remove(measureGroup.current);
      disposeObject(measureGroup.current);
      measureGroup.current = null;
    }
    for (const label of measureLabels) label.element.remove();
    measureLabels = [];
    if (measureHost) {
      measureHost.remove();
      measureHost = null;
    }
  }

  // --- Section plane ------------------------------------------------------

  function rebuildSection(): void {
    disposeSection();
    if (!currentBounds) return;
    const dims = boundsDimensions(currentBounds);
    const pad = Math.max(Math.max(dims.width, dims.height, dims.depth) * 0.08, 1);
    // The cutting plane is the shared Section A–A: XZ at y = 4 mm for the
    // ADR design default, or the Z mid-plane of the primitive bounds. A part
    // that is not centred is cut at its own mid-plane.
    const corners = sectionCutCorners(currentBounds, sectionCut, pad).map(
      ([x, y, z]) => new THREE.Vector3(x, y, z),
    );
    const points = [
      corners[0]!, corners[1]!,
      corners[1]!, corners[2]!,
      corners[2]!, corners[3]!,
      corners[3]!, corners[0]!,
    ];
    const span = Math.max(dims.width, dims.height, 1);
    const material = new THREE.LineDashedMaterial({
      color: new THREE.Color(palette.redline),
      transparent: true,
      opacity: 0.9,
      depthTest: false,
      dashSize: Math.max(span * 0.06, 1),
      gapSize: Math.max(span * 0.03, 0.5),
    });
    const line = new THREE.LineSegments(new THREE.BufferGeometry().setFromPoints(points), material);
    line.computeLineDistances();
    line.renderOrder = 3;
    const group = new THREE.Group();
    group.add(line);
    group.visible = sectionVisible;
    scene.add(group);
    sectionGroup.current = group;
  }

  function recolorSection(): void {
    if (!sectionGroup.current) return;
    for (const child of sectionGroup.current.children) {
      if (child instanceof THREE.LineSegments) {
        const material = child.material;
        if (material instanceof THREE.LineDashedMaterial) material.color.set(palette.redline);
      }
    }
  }

  function disposeSection(): void {
    if (sectionGroup.current) {
      scene.remove(sectionGroup.current);
      disposeObject(sectionGroup.current);
      sectionGroup.current = null;
    }
  }

  // --- Lifecycle ----------------------------------------------------------

  function disposeMeshGroup(): void {
    if (meshGroup.current) {
      scene.remove(meshGroup.current);
      disposeObject(meshGroup.current);
      meshGroup.current = null;
    }
    faceMesh = null;
  }

  function handleResize(): void {
    const rect = container.getBoundingClientRect();
    const width = Math.max(1, rect.width);
    const height = Math.max(1, rect.height);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    renderer.setSize(width, height, false);
    updateOverlayLabelPositions();
  }

  applyLighting();

  return {
    applyMesh,
    applyViewport,
    applyRenderMode,
    applySection,
    applySectionCut,
    applyMeasure,
    applyCommand,
    resetView,
    renderNow,
    handleResize,
    dispose(): void {
      disposed = true;
      themeObserver.disconnect();
      controls.removeEventListener('change', renderNow);
      renderer.domElement.removeEventListener('pointerdown', onPointerDown);
      renderer.domElement.removeEventListener('pointerup', onPointerUp);
      clearDimensions();
      disposeSection();
      disposeCentreLines();
      disposeMeasure();
      disposeMeshGroup();
      if (grid) {
        scene.remove(grid);
        disposeObject(grid);
        grid = null;
      }
      if (axes) {
        scene.remove(axes);
        disposeObject(axes);
        axes = null;
      }
      renderer.dispose();
      controls.dispose();
      if (renderer.domElement.parentNode === container) {
        container.removeChild(renderer.domElement);
      }
      currentBounds = null;
    },
  };
}

function disposeObject(root: THREE.Object3D): void {
  root.traverse((object) => {
    const withGeometry = object as THREE.Object3D & { geometry?: THREE.BufferGeometry };
    if (withGeometry.geometry) withGeometry.geometry.dispose();
    const withMaterial = object as THREE.Object3D & {
      material?: THREE.Material | THREE.Material[];
    };
    if (withMaterial.material) disposeMaterial(withMaterial.material);
  });
}

function disposeMaterial(material: THREE.Material | THREE.Material[]): void {
  if (Array.isArray(material)) {
    for (const item of material) item.dispose();
    return;
  }
  material.dispose();
}

function meshToGeometry(mesh: StudioLocalPreview | PreviewResponse): THREE.BufferGeometry | null {
  const positions = mesh.positions instanceof Float32Array
    ? mesh.positions
    : new Float32Array(mesh.positions);
  const normals = mesh.normals instanceof Float32Array
    ? mesh.normals
    : new Float32Array(mesh.normals);
  const indices = mesh.indices instanceof Uint32Array
    ? mesh.indices
    : new Uint32Array(mesh.indices);
  if (positions.length < 9 || indices.length < 3) return null;
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
  if (normals.length === positions.length) {
    geometry.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
  } else {
    geometry.computeVertexNormals();
  }
  geometry.setIndex(new THREE.BufferAttribute(indices, 1));
  geometry.computeBoundingSphere();
  return geometry;
}

function renderWebGLFallback(container: HTMLDivElement, cause: unknown): void {
  const message = document.createElement('div');
  message.className = 'viewer-canvas-fallback';
  message.setAttribute('role', 'status');
  const reason = cause instanceof Error ? cause.message : 'WebGL is not available in this context.';
  message.textContent = `3D preview unavailable: ${reason}. The inspector and panels still work.`;
  container.appendChild(message);
}
