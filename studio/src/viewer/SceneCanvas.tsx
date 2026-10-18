// Three.js viewport.
import type { ReactElement } from 'react';
//
// Renders the latest preview mesh (server-verified when present, local
// otherwise) with orbit controls, a millimetre grid, axes, optional
// dimension annotations, and the chosen material. The component owns its
// own renderer and scene; React feeds it new mesh data and viewport
// preferences.
//
// Render-on-demand: the scene re-renders only when something changes
// (new mesh, viewport preference, camera move). Orbit damping still
// works because `OrbitControls.change` fires while damping is active.
// Disposal walks every geometry and material that was ever attached and
// releases its GPU resources; this matters whenever the viewport
// preferences swap the material kind or the mesh is replaced.

import { useEffect, useRef } from 'react';
import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import {
  BBox,
  StudioLocalPreview,
  PreviewResponse,
} from '../types/studio';
import { boundsDimensions, formatMillimetres } from './localPrimitives';
import type { MaterialKind, BackgroundKind } from '../state/store';

export interface SceneCanvasProps {
  mesh: StudioLocalPreview | PreviewResponse | null;
  meshSource: 'local' | 'server' | 'none';
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
}

interface DimensionOverlay {
  element: HTMLDivElement;
}

const GRID_SIZE_MM = 1000;
const GRID_DIVISIONS = 50;

const BACKGROUND_COLORS: Record<BackgroundKind, number> = {
  studio: 0x14181f,
  paper: 0xeef2f6,
  dark: 0x05070a,
};

export function SceneCanvas(props: SceneCanvasProps): ReactElement {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const overlayHostRef = useRef<HTMLDivElement | null>(null);
  const sceneStateRef = useRef<SceneState | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    const overlayHost = overlayHostRef.current;
    if (!container || !overlayHost) return;
    let state: SceneState | null = null;
    try {
      state = createScene(container, overlayHost);
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
    state.applyMesh(props.mesh, props.meshSource);
  }, [props.mesh, props.meshSource]);

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

  return (
    <div className="viewer-canvas" ref={containerRef} role="img" aria-label="3D viewport of the current primitive">
      <div className="viewer-canvas-overlay" ref={overlayHostRef} aria-hidden="false" />
      <div className="viewer-canvas-source" aria-live="polite">
        {props.meshSource === 'server'
          ? 'Server verified mesh'
          : props.meshSource === 'local'
            ? 'Local preview — pending server verification'
            : 'No mesh'}
      </div>
    </div>
  );
}

interface ViewportSnapshot {
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;
  background: BackgroundKind;
}

interface SceneState {
  applyMesh(mesh: StudioLocalPreview | PreviewResponse | null, source: 'local' | 'server' | 'none'): void;
  applyViewport(next: ViewportSnapshot): void;
  renderNow(): void;
  handleResize(): void;
  dispose(): void;
}

function createScene(container: HTMLDivElement, overlayHost: HTMLDivElement): SceneState {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(BACKGROUND_COLORS.studio);

  const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100_000);
  camera.position.set(220, 180, 260);
  camera.up.set(0, 1, 0);

  const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false, powerPreference: 'high-performance' });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.0;
  container.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = false;
  controls.minDistance = 1;
  controls.maxDistance = 10_000;

  const ambient = new THREE.AmbientLight(0xffffff, 0.4);
  scene.add(ambient);
  const key = new THREE.DirectionalLight(0xffffff, 0.85);
  key.position.set(400, 600, 300);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0x99bbff, 0.3);
  fill.position.set(-300, 200, -200);
  scene.add(fill);

  const grid = new THREE.GridHelper(GRID_SIZE_MM, GRID_DIVISIONS, 0x4a5566, 0x2a323d);
  (grid.material as THREE.Material).transparent = true;
  (grid.material as THREE.Material).opacity = 0.85;
  scene.add(grid);

  const axes = new THREE.AxesHelper(120);
  scene.add(axes);

  const meshGroup: { current: THREE.Group | null } = { current: null };
  const dimensionOverlay: { current: DimensionOverlay | null } = { current: null };
  let currentBounds: BBox | null = null;
  let viewport: ViewportSnapshot = {
    showGrid: true,
    showAxes: true,
    showDimensions: true,
    material: 'default',
    background: 'studio',
  };
  let disposed = false;

  controls.addEventListener('change', renderNow);
  function renderNow(): void {
    if (disposed) return;
    renderer.render(scene, camera);
  }

  function disposeObject(object: THREE.Object3D): void {
    object.traverse((child) => {
      if (child instanceof THREE.Mesh) {
        child.geometry.dispose();
        const mat = child.material;
        if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
        else mat.dispose();
      }
      if (child instanceof THREE.GridHelper) {
        child.geometry.dispose();
        const mat = child.material;
        if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
        else mat.dispose();
      }
      if (child instanceof THREE.AxesHelper) {
        child.geometry.dispose();
        const mat = child.material as THREE.LineBasicMaterial | undefined;
        if (mat) mat.dispose();
      }
    });
  }

  function disposeMeshGroup(): void {
    if (meshGroup.current) {
      scene.remove(meshGroup.current);
      disposeObject(meshGroup.current);
      meshGroup.current = null;
    }
  }

  function disposeHelpers(): void {
    grid.geometry.dispose();
    const gridMat = grid.material as THREE.Material | THREE.Material[];
    if (Array.isArray(gridMat)) gridMat.forEach((m) => m.dispose());
    else gridMat.dispose();
    axes.geometry.dispose();
    const axesMat = axes.material as THREE.LineBasicMaterial;
    axesMat.dispose();
  }

  function applyMesh(
    mesh: StudioLocalPreview | PreviewResponse | null,
    source: 'local' | 'server' | 'none',
  ): void {
    if (!mesh) {
      disposeMeshGroup();
      clearDimensions();
      currentBounds = null;
      renderNow();
      return;
    }
    const geometry = meshToGeometry(mesh);
    if (!geometry) {
      disposeMeshGroup();
      clearDimensions();
      currentBounds = null;
      renderNow();
      return;
    }
    disposeMeshGroup();
    const group = new THREE.Group();
    const threeMesh = new THREE.Mesh(geometry, makeMaterial(viewport.material));
    group.add(threeMesh);
    scene.add(group);
    meshGroup.current = group;
    currentBounds = mesh.bounds;
    frameOnBounds(mesh.bounds, source === 'none' ? 'server' : source);
    renderDimensions(mesh.bounds);
    renderNow();
  }

  function frameOnBounds(bounds: BBox, source: 'local' | 'server' | 'none'): void {
    const min = new THREE.Vector3(bounds.min[0], bounds.min[1], bounds.min[2]);
    const max = new THREE.Vector3(bounds.max[0], bounds.max[1], bounds.max[2]);
    const centre = min.clone().add(max).multiplyScalar(0.5);
    const size = max.clone().sub(min);
    const radius = Math.max(size.x, size.y, size.z, 1);
    const distance = radius * 3.2;
    if (source === 'none') return;
    controls.target.copy(centre);
    camera.position.copy(centre.clone().add(new THREE.Vector3(distance, distance * 0.75, distance)));
    controls.update();
  }

  function applyViewport(next: ViewportSnapshot): void {
    const materialChanged = next.material !== viewport.material;
    viewport = next;
    grid.visible = next.showGrid;
    axes.visible = next.showAxes;
    scene.background = new THREE.Color(BACKGROUND_COLORS[next.background]);
    if (dimensionOverlay.current) {
      dimensionOverlay.current.element.style.display = next.showDimensions ? '' : 'none';
    }
    if (meshGroup.current && materialChanged) {
      const child = meshGroup.current.children[0];
      if (child instanceof THREE.Mesh) {
        // Dispose the previous material so we don't leak GPU resources on
        // every material-kind toggle.
        const old = child.material;
        child.material = makeMaterial(next.material);
        if (Array.isArray(old)) old.forEach((m) => m.dispose());
        else old.dispose();
      }
    }
    renderNow();
  }

  function renderDimensions(bounds: BBox): void {
    clearDimensions();
    if (!viewport.showDimensions) return;
    const dims = boundsDimensions(bounds);
    const element = document.createElement('div');
    element.className = 'viewer-canvas-dimensions';
    element.setAttribute('role', 'status');
    element.textContent = [
      `Width  ${formatMillimetres(dims.width)}`,
      `Height ${formatMillimetres(dims.height)}`,
      `Depth  ${formatMillimetres(dims.depth)}`,
    ].join('   ');
    overlayHost.appendChild(element);
    dimensionOverlay.current = { element };
  }

  function clearDimensions(): void {
    if (dimensionOverlay.current) {
      dimensionOverlay.current.element.remove();
      dimensionOverlay.current = null;
    }
  }

  function makeMaterial(kind: MaterialKind): THREE.Material {
    if (kind === 'wireframe') return new THREE.MeshBasicMaterial({ color: 0x66ccff, wireframe: true });
    if (kind === 'matte') {
      return new THREE.MeshStandardMaterial({ color: 0xb4c4d6, metalness: 0.0, roughness: 0.95 });
    }
    return new THREE.MeshStandardMaterial({ color: 0x7aa6ff, metalness: 0.15, roughness: 0.45 });
  }

  function handleResize(): void {
    const rect = container.getBoundingClientRect();
    const w = Math.max(1, rect.width);
    const h = Math.max(1, rect.height);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h, false);
  }

  return {
    applyMesh,
    applyViewport,
    renderNow,
    handleResize,
    dispose(): void {
      disposed = true;
      controls.removeEventListener('change', renderNow);
      clearDimensions();
      disposeMeshGroup();
      disposeHelpers();
      renderer.dispose();
      controls.dispose();
      if (renderer.domElement.parentNode === container) {
        container.removeChild(renderer.domElement);
      }
      scene.traverse((object) => {
        if (object instanceof THREE.Mesh) {
          object.geometry.dispose();
          const m = object.material;
          if (Array.isArray(m)) m.forEach((mm) => mm.dispose());
          else m.dispose();
        }
      });
      currentBounds = null;
    },
  };
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