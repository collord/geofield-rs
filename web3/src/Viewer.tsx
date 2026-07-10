// Minimal three.js viewer for the testbed: renders the streamed surfaces (one mesh per
// SurfacePayload), with user-driven orbit controls and labeled X/Y/Z axes spanning the
// data's bounding box. Adapted/simplified from web/src/Viewer.tsx. Colour is by series index
// in contact mode, or by lithology (from the catalog) in solid mode — see `colorBy` (feature 003).

import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { SurfacePayload } from "./protocol";

const SERIES_COLORS = [0x4f83cc, 0xcc7a4f, 0x6aa84f, 0x9b59b6, 0xc0392b];

/** Data bounding box `[x0, x1, y0, y1, z0, z1]`. */
export type Extent = [number, number, number, number, number, number];

/** Serializable camera/orbit state — exported with the project so a view round-trips
 *  (and so an exact viewpoint can be shared for repro). */
export interface ViewState {
  camera: [number, number, number]; // camera world position
  target: [number, number, number]; // orbit pivot
  up: [number, number, number]; // camera up
  fov: number; // vertical FOV (deg)
}

const AXIS_COLOR = { x: 0xd1453b, y: 0x3b8c3b, z: 0x3b5bd1 } as const;

/** A billboard text label (canvas texture on a Sprite) placed at `pos`. */
function makeLabel(text: string, color: number, pos: [number, number, number]): THREE.Sprite {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#" + color.toString(16).padStart(6, "0");
  ctx.font = "bold 48px system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText(text, 32, 34);
  const sprite = new THREE.Sprite(
    new THREE.SpriteMaterial({ map: new THREE.CanvasTexture(canvas), depthTest: false }),
  );
  sprite.position.set(...pos);
  return sprite;
}

/** Build the labeled X/Y/Z axes spanning the data bounding box (lines from the box origin
 *  corner along each edge + a billboard letter just past each axis end). */
function buildAxes(extent: Extent): THREE.Group {
  const [x0, x1, y0, y1, z0, z1] = extent;
  const g = new THREE.Group();
  const pad = 0.08 * Math.max(x1 - x0, y1 - y0, z1 - z0); // label offset past the box

  const axis = (from: [number, number, number], to: [number, number, number], color: number) => {
    const geom = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(...from),
      new THREE.Vector3(...to),
    ]);
    g.add(new THREE.Line(geom, new THREE.LineBasicMaterial({ color })));
  };

  // Axes run along the box edges meeting at the (x0,y0,z0) corner.
  axis([x0, y0, z0], [x1, y0, z0], AXIS_COLOR.x);
  axis([x0, y0, z0], [x0, y1, z0], AXIS_COLOR.y);
  axis([x0, y0, z0], [x0, y0, z1], AXIS_COLOR.z);

  g.add(makeLabel("X", AXIS_COLOR.x, [x1 + pad, y0, z0]));
  g.add(makeLabel("Y", AXIS_COLOR.y, [x0, y1 + pad, z0]));
  g.add(makeLabel("Z", AXIS_COLOR.z, [x0, y0, z1 + pad]));

  // Scale the billboard letters to a readable fraction of the domain.
  const labelSize = 0.06 * Math.max(x1 - x0, y1 - y0, z1 - z0);
  g.children.forEach((c) => {
    if (c instanceof THREE.Sprite) c.scale.setScalar(labelSize);
  });
  return g;
}

/** Build a 12-edge wireframe box spanning the model bounds (feature: draw model bounds). Distinct
 *  from `buildAxes` (which only draws the 3 axis edges from one corner) — this is the full prism. */
function buildBoundsBox(extent: Extent): THREE.LineSegments {
  const [x0, x1, y0, y1, z0, z1] = extent;
  const c: [number, number, number][] = [
    [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0], // bottom
    [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], // top
  ];
  const edges: [number, number][] = [
    [0, 1], [1, 2], [2, 3], [3, 0], // bottom loop
    [4, 5], [5, 6], [6, 7], [7, 4], // top loop
    [0, 4], [1, 5], [2, 6], [3, 7], // verticals
  ];
  const pts: THREE.Vector3[] = [];
  for (const [a, b] of edges) {
    pts.push(new THREE.Vector3(...c[a]), new THREE.Vector3(...c[b]));
  }
  const geom = new THREE.BufferGeometry().setFromPoints(pts);
  return new THREE.LineSegments(geom, new THREE.LineBasicMaterial({ color: 0x22d3ee })); // cyan
}

/** Convert a "#rrggbb" hex string to a three.js numeric colour. */
function hexToNum(hex: string): number {
  return parseInt(hex.replace("#", ""), 16) || 0x888888;
}

/** A stable key for one rendered payload, for per-volume visibility (debug). Solid mode keys
 *  by `(series, unit)`; contact mode by `(series, block, interface)`. */
export function surfaceKey(s: SurfacePayload): string {
  return s.unit !== undefined ? `${s.series}:${s.unit}` : `${s.series}:${s.block}:${s.interface}`;
}

export function Viewer(props: {
  surfaces: SurfacePayload[];
  extent: Extent;
  stale?: boolean;
  /** "series" (contact mode, default) or "lithology" (solid mode). */
  colorBy?: "series" | "lithology";
  /** lithology id → hex colour (for colorBy="lithology"). */
  lithoColors?: Record<string, string>;
  /** Debug: draw triangle edges over each mesh. */
  wireframe?: boolean;
  /** Debug: surface keys (see `surfaceKey`) to HIDE. Absent/empty ⇒ show all. */
  hiddenKeys?: Set<string>;
  /** Filled by the Viewer with a `getView()` so the parent can capture the current
   *  camera on Export. */
  viewApiRef?: { current: { getView: () => ViewState } | null };
  /** When set (a NEW object), the Viewer applies this camera state — used on Import to
   *  restore a saved view. Applied after the extent re-center so it wins. */
  importedView?: ViewState;
  /** Draw the model-bounds wireframe box (feature: draw model bounds). Display-only. */
  showBounds?: boolean;
  /** When set (a NEW object), enter 2-point top-down bounds digitization. On the second click the
   *  Viewer calls `onBoundsXY(x0,x1,y0,y1)` with the drawn rectangle (model coords) and exits. */
  drawBounds?: { nonce: number };
  onBoundsXY?: (x0: number, x1: number, y0: number, y1: number) => void;
}) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: OrbitControls;
    group: THREE.Group;
    axes: THREE.Group | null;
    boundsBox: THREE.LineSegments | null;
    rubber: THREE.LineLoop | null; // live rectangle during digitization
  } | null>(null);

  // One-time three.js setup.
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setSize(mount.clientWidth, mount.clientHeight);
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0xf0f2f5);
    const camera = new THREE.PerspectiveCamera(50, mount.clientWidth / mount.clientHeight, 1, 1e5);
    camera.position.set(1600, 1600, 1600);
    scene.add(new THREE.AmbientLight(0xffffff, 0.7));
    const dir = new THREE.DirectionalLight(0xffffff, 0.6);
    dir.position.set(1, 1, 1);
    scene.add(dir);
    const group = new THREE.Group();
    scene.add(group);

    // Standard orbit controls (drag to rotate, wheel to zoom, right-drag to pan). The view
    // is user-driven — NO automatic rotation.
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.target.set(500, 500, 500);
    controls.update();

    stateRef.current = { renderer, scene, camera, controls, group, axes: null, boundsBox: null, rubber: null };
    if (props.viewApiRef) {
      props.viewApiRef.current = {
        getView: () => ({
          camera: camera.position.toArray() as [number, number, number],
          target: controls.target.toArray() as [number, number, number],
          up: camera.up.toArray() as [number, number, number],
          fov: camera.fov,
        }),
      };
    }
    let raf = 0;
    const tick = () => {
      controls.update(); // damping only; no scene rotation
      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => {
      cancelAnimationFrame(raf);
      controls.dispose();
      renderer.dispose();
      mount.removeChild(renderer.domElement);
    };
  }, []);

  // Rebuild the labeled axes whenever the data extent changes, and center the orbit target
  // on the box so rotation pivots around the data.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.axes) {
      st.scene.remove(st.axes);
      st.axes = null;
    }
    const axes = buildAxes(props.extent);
    st.scene.add(axes);
    st.axes = axes;

    const [x0, x1, y0, y1, z0, z1] = props.extent;
    st.controls.target.set((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2);
    st.controls.update();
  }, [props.extent]);

  // The model-bounds wireframe box — rebuilt on extent change, shown/hidden by `showBounds`.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.boundsBox) {
      st.scene.remove(st.boundsBox);
      st.boundsBox.geometry.dispose();
      st.boundsBox = null;
    }
    if (props.showBounds) {
      const box = buildBoundsBox(props.extent);
      st.scene.add(box);
      st.boundsBox = box;
    }
  }, [props.extent, props.showBounds]);

  // 2-point BOUNDS DIGITIZATION (feature: draw model bounds). Fires when `drawBounds` is a new
  // object: snap top-down, disable orbit, and capture two ground-plane clicks into a rectangle.
  useEffect(() => {
    const st = stateRef.current;
    if (!st || !props.drawBounds) return;
    const { renderer, camera, controls, scene } = st;
    const dom = renderer.domElement;
    const [x0, x1, y0, y1, z0, z1] = props.extent;
    const cx = (x0 + x1) / 2;
    const cy = (y0 + y1) / 2;
    const zmid = (z0 + z1) / 2;
    const span = Math.max(x1 - x0, y1 - y0);

    // Snap top-down: look straight down -Z with +Y up, from well above the box.
    const damp = controls.enableDamping;
    controls.enableDamping = false;
    controls.update();
    camera.up.set(0, 1, 0);
    camera.position.set(cx, cy, zmid + span * 2.0);
    controls.target.set(cx, cy, zmid);
    controls.update();
    controls.enableDamping = damp;
    controls.enabled = false; // no orbit while digitizing

    const raycaster = new THREE.Raycaster();
    const plane = new THREE.Plane(new THREE.Vector3(0, 0, 1), -zmid); // XY plane at z = zmid
    const groundXY = (e: PointerEvent): THREE.Vector3 | null => {
      const r = dom.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - r.left) / r.width) * 2 - 1,
        -((e.clientY - r.top) / r.height) * 2 + 1,
      );
      raycaster.setFromCamera(ndc, camera);
      const hit = new THREE.Vector3();
      return raycaster.ray.intersectPlane(plane, hit) ? hit : null;
    };

    let cornerA: THREE.Vector3 | null = null;
    const rect = (a: THREE.Vector3, b: THREE.Vector3): THREE.Vector3[] => [
      new THREE.Vector3(a.x, a.y, zmid),
      new THREE.Vector3(b.x, a.y, zmid),
      new THREE.Vector3(b.x, b.y, zmid),
      new THREE.Vector3(a.x, b.y, zmid),
    ];
    const clearRubber = () => {
      if (st.rubber) {
        scene.remove(st.rubber);
        st.rubber.geometry.dispose();
        st.rubber = null;
      }
    };

    const onMove = (e: PointerEvent) => {
      if (!cornerA) return;
      const b = groundXY(e);
      if (!b) return;
      clearRubber();
      const loop = new THREE.LineLoop(
        new THREE.BufferGeometry().setFromPoints(rect(cornerA, b)),
        new THREE.LineBasicMaterial({ color: 0xfacc15 }), // amber while drawing
      );
      scene.add(loop);
      st.rubber = loop;
    };
    const onDown = (e: PointerEvent) => {
      const p = groundXY(e);
      if (!p) return;
      if (!cornerA) {
        cornerA = p.clone();
        return;
      }
      // Second click → finish.
      const rx0 = Math.min(cornerA.x, p.x);
      const rx1 = Math.max(cornerA.x, p.x);
      const ry0 = Math.min(cornerA.y, p.y);
      const ry1 = Math.max(cornerA.y, p.y);
      cleanup();
      props.onBoundsXY?.(rx0, rx1, ry0, ry1);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") cleanup();
    };

    function cleanup() {
      dom.removeEventListener("pointerdown", onDown);
      dom.removeEventListener("pointermove", onMove);
      window.removeEventListener("keydown", onKey);
      clearRubber();
      controls.enabled = true;
    }

    dom.addEventListener("pointerdown", onDown);
    dom.addEventListener("pointermove", onMove);
    window.addEventListener("keydown", onKey);
    dom.style.cursor = "crosshair";
    return () => {
      cleanup();
      dom.style.cursor = "";
    };
  }, [props.drawBounds]);

  // Apply an imported view (declared AFTER the extent re-center so it overrides the
  // auto-centered target). Fires whenever `importedView` is a new object (each Import).
  useEffect(() => {
    const st = stateRef.current;
    const v = props.importedView;
    if (!st || !v) return;
    // Toggle damping OFF around the set so OrbitControls FLUSHES any residual orbit delta
    // (an un-settled drag would otherwise keep rotating the camera off the imported angle —
    // visible headless, where rAF/damping barely advances). First update() zeros the delta;
    // then we set the pose; the second update() syncs the internal spherical to it.
    const damp = st.controls.enableDamping;
    st.controls.enableDamping = false;
    st.controls.update();
    st.camera.position.set(...v.camera);
    st.camera.up.set(...v.up);
    if (v.fov) {
      st.camera.fov = v.fov;
      st.camera.updateProjectionMatrix();
    }
    st.controls.target.set(...v.target);
    st.controls.update();
    st.controls.enableDamping = damp;
  }, [props.importedView]);

  // Rebuild meshes whenever the surfaces change.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    st.group.clear();
    const byLithology = props.colorBy === "lithology";
    const hidden = props.hiddenKeys;
    for (const s of props.surfaces) {
      if (hidden && hidden.has(surfaceKey(s))) continue; // per-volume visibility (debug)
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.BufferAttribute(s.positions, 3));
      geom.setIndex(new THREE.BufferAttribute(s.indices, 1));
      // The field path supplies analytic per-vertex normals; the Manifold boolean path emits none
      // (an empty array) — compute them from the geometry so those solids shade instead of going
      // black. (An empty normal attribute would mismatch the position count and render unlit.)
      if (s.normals.length === s.positions.length && s.normals.length > 0) {
        geom.setAttribute("normal", new THREE.BufferAttribute(s.normals, 3));
      } else {
        geom.computeVertexNormals();
      }
      // Solid mode: colour each unit solid by its assigned lithology (Air is never emitted,
      // so there is nothing transparent to draw). Contact mode: colour by series index.
      const color =
        byLithology && s.lithologyId && props.lithoColors?.[s.lithologyId] !== undefined
          ? hexToNum(props.lithoColors[s.lithologyId])
          : byLithology
            ? 0xc8c8cd // Unknown / unassigned solid (neutral grey)
            : SERIES_COLORS[s.series % SERIES_COLORS.length];
      const mat = new THREE.MeshStandardMaterial({ color, side: THREE.DoubleSide, flatShading: true });
      st.group.add(new THREE.Mesh(geom, mat));
      // Debug: overlay triangle edges so the mesh tessellation is visible.
      if (props.wireframe) {
        const edges = new THREE.LineSegments(
          new THREE.WireframeGeometry(geom),
          new THREE.LineBasicMaterial({ color: 0x111111, transparent: true, opacity: 0.4 }),
        );
        st.group.add(edges);
      }
    }
  }, [props.surfaces, props.colorBy, props.lithoColors, props.wireframe, props.hiddenKeys]);

  // Positioned container so the stale scrim can sit as a sibling overlay over the canvas.
  return (
    <div style={{ position: "relative", width: "100%", height: "100%", minHeight: 400 }}>
      <div ref={mountRef} style={{ width: "100%", height: "100%", minHeight: 400 }} />
      {props.stale && (
        // ~40% grey, pointer-events:none so orbiting still works — the view reads as STALE
        // until the next successful Apply. Tests assert on the element, not pixels.
        <div
          className="stale-scrim"
          data-testid="stale-scrim"
          style={{
            position: "absolute",
            inset: 0,
            background: "rgba(120,120,120,0.4)",
            pointerEvents: "none",
          }}
        />
      )}
    </div>
  );
}
