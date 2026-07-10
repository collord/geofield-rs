// Minimal three.js viewer for the testbed: renders the streamed surfaces (one mesh per
// SurfacePayload), with user-driven orbit controls and labeled X/Y/Z axes spanning the
// data's bounding box. Adapted/simplified from web/src/Viewer.tsx. Colour is by series index
// in contact mode, or by lithology (from the catalog) in solid mode — see `colorBy` (feature 003).

import { useEffect, useMemo, useRef } from "react";
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

/** Axis-aligned bounds (min,max) over ALL surface positions — the DATA extent, which for imported
 *  world-coordinate (e.g. UTM) meshes is nowhere near the origin/model-box. `null` if no data. */
function dataBounds(surfaces: SurfacePayload[]): { min: [number, number, number]; max: [number, number, number] } | null {
  let has = false;
  const min: [number, number, number] = [Infinity, Infinity, Infinity];
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  for (const s of surfaces) {
    const p = s.positions;
    for (let i = 0; i + 2 < p.length; i += 3) {
      has = true;
      for (let k = 0; k < 3; k++) {
        const v = p[i + k];
        if (v < min[k]) min[k] = v;
        if (v > max[k]) max[k] = v;
      }
    }
  }
  return has ? { min, max } : null;
}

/** Point the camera at the centre of a box from ABOVE-SOUTH-EAST (X=East, Y=North, Z=Up), at a
 *  distance that fits the whole box in view. The industry-standard default geological view; works at
 *  any world coordinate (frames the DATA, not the origin). */
function frameAboveSE(
  camera: THREE.PerspectiveCamera,
  controls: OrbitControls,
  min: [number, number, number],
  max: [number, number, number],
) {
  const c = new THREE.Vector3((min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2);
  const r = 0.5 * Math.hypot(max[0] - min[0], max[1] - min[1], max[2] - min[2]) || 1;
  const fovV = (camera.fov * Math.PI) / 180;
  const fovH = 2 * Math.atan(Math.tan(fovV / 2) * camera.aspect);
  const fov = Math.min(fovV, fovH);
  const dist = (r / Math.sin(fov / 2)) * 1.15; // 15% margin so nothing clips the frame
  // above-south-east direction: +East(+X), −North(−Y = south), +Up(+Z), normalized.
  const dir = new THREE.Vector3(1, -1, 1).normalize();
  const damp = controls.enableDamping;
  controls.enableDamping = false;
  controls.update();
  camera.up.set(0, 0, 1); // Z is up (geological convention), so orbit + framing stay upright
  camera.position.copy(c).addScaledVector(dir, dist);
  // Keep near/far sensible for the data scale so world-coordinate meshes don't clip.
  camera.near = Math.max(dist * 1e-3, 0.01);
  camera.far = dist * 100;
  camera.updateProjectionMatrix();
  controls.target.copy(c);
  controls.update();
  controls.enableDamping = damp;
}

/** A small direction-only axes gnomon (X/Y/Z), placed at a reasonable OFFSET below the data box's
 *  min corner — for orientation, not scale (the bounds box conveys scale). Sized to a fraction of the
 *  data extent so it reads at any world scale. */
function buildGnomon(min: [number, number, number], max: [number, number, number]): THREE.Group {
  const span = Math.max(max[0] - min[0], max[1] - min[1], max[2] - min[2]) || 1;
  const len = span * 0.15;
  const off = span * 0.08;
  const o: [number, number, number] = [min[0] - off, min[1] - off, min[2] - off]; // just outside the box
  const g = new THREE.Group();
  const axis = (d: [number, number, number], color: number, letter: string) => {
    const end: [number, number, number] = [o[0] + d[0] * len, o[1] + d[1] * len, o[2] + d[2] * len];
    g.add(new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(...o), new THREE.Vector3(...end)]),
      new THREE.LineBasicMaterial({ color }),
    ));
    const label = makeLabel(letter, color, [o[0] + d[0] * len * 1.2, o[1] + d[1] * len * 1.2, o[2] + d[2] * len * 1.2]);
    label.scale.setScalar(len * 0.4);
    g.add(label);
  };
  axis([1, 0, 0], AXIS_COLOR.x, "X"); // East
  axis([0, 1, 0], AXIS_COLOR.y, "Y"); // North
  axis([0, 0, 1], AXIS_COLOR.z, "Z"); // Up
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
  /** Bump to reframe the camera on the current data (above-SE, fit) — a "Fit view" button. */
  fitNonce?: number;
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
    framedKey: string | null; // the data-bounds key we last auto-framed (frame only on NEW data)
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
    camera.up.set(0, 0, 1); // Z is up (X=East, Y=North, Z=Up) — geological convention
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

    stateRef.current = { renderer, scene, camera, controls, group, axes: null, boundsBox: null, rubber: null, framedKey: null };
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

  // The DATA extent (bounding box of the rendered surfaces) — the thing to frame + orient to. For
  // imported world-coordinate meshes this is nowhere near the origin/model-box.
  const dataBox = useMemo(() => dataBounds(props.surfaces), [props.surfaces]);

  // The direction-only axes gnomon, placed just outside the data box (fallback: the model extent for
  // a blank project). Rebuilt when the box changes.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.axes) {
      st.scene.remove(st.axes);
      st.axes = null;
    }
    const [ex0, ex1, ey0, ey1, ez0, ez1] = props.extent;
    const min = dataBox ? dataBox.min : ([ex0, ey0, ez0] as [number, number, number]);
    const max = dataBox ? dataBox.max : ([ex1, ey1, ez1] as [number, number, number]);
    const gnomon = buildGnomon(min, max);
    st.scene.add(gnomon);
    st.axes = gnomon;
  }, [dataBox, props.extent]);

  // Auto-frame the camera on NEW data (above-south-east, fit-to-extent). Keyed on the data box, so a
  // re-mesh of the SAME data (same box) does NOT reframe — it never fights the user's orbit. Runs
  // BEFORE the importedView effect, so a saved-view project-import overrides it. Also drives `fitNonce`.
  useEffect(() => {
    const st = stateRef.current;
    if (!st || !dataBox) return;
    const key = dataBox.min.join(",") + "|" + dataBox.max.join(",");
    if (st.framedKey === key) return; // same data already framed — leave the user's orbit alone
    st.framedKey = key;
    frameAboveSE(st.camera, st.controls, dataBox.min, dataBox.max);
  }, [dataBox]);

  // "Fit view" on demand: reframe above-SE on the current data (or the model extent if no data).
  useEffect(() => {
    const st = stateRef.current;
    if (!st || !props.fitNonce) return;
    const [ex0, ex1, ey0, ey1, ez0, ez1] = props.extent;
    const min = dataBox ? dataBox.min : ([ex0, ey0, ez0] as [number, number, number]);
    const max = dataBox ? dataBox.max : ([ex1, ey1, ez1] as [number, number, number]);
    frameAboveSE(st.camera, st.controls, min, max);
  }, [props.fitNonce]);

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
    // Frame the top-down view on the DATA (so the meshes stay visible while you draw the bounds
    // around them) — NOT the model extent, which for world-coordinate data sits far off at the origin.
    // Fall back to the extent only when there's no data.
    const [ex0, ex1, ey0, ey1, ez0, ez1] = props.extent;
    const bmin = dataBox ? dataBox.min : ([ex0, ey0, ez0] as [number, number, number]);
    const bmax = dataBox ? dataBox.max : ([ex1, ey1, ez1] as [number, number, number]);
    const cx = (bmin[0] + bmax[0]) / 2;
    const cy = (bmin[1] + bmax[1]) / 2;
    const zmid = (bmin[2] + bmax[2]) / 2;
    const span = Math.max(bmax[0] - bmin[0], bmax[1] - bmin[1]) || 1;

    // Snap top-down: look straight down -Z with +Y (North) up, from well above the data. Scale
    // near/far to the data so world-coordinate meshes stay inside the frustum (don't clip away).
    const damp = controls.enableDamping;
    controls.enableDamping = false;
    controls.update();
    camera.up.set(0, 1, 0);
    const dist = span * 2.0 || 1000;
    camera.position.set(cx, cy, zmid + dist);
    camera.near = Math.max(dist * 1e-3, 0.01);
    camera.far = dist * 100;
    camera.updateProjectionMatrix();
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
