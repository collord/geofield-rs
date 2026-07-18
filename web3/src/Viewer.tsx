// Minimal three.js viewer for the testbed: renders the streamed surfaces (one mesh per
// SurfacePayload), with user-driven orbit controls and labeled X/Y/Z axes spanning the
// data's bounding box. Adapted/simplified from web/src/Viewer.tsx. Colour is by series index
// in contact mode, or by lithology (from the catalog) in solid mode — see `colorBy` (feature 003).

import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { SurfacePayload } from "./protocol";
import type { AuthoredOverlay } from "./HierarchyPanel/project";
import { type SectionFrame, fromInPlane } from "./section/frame";
import { derivePoles } from "./section/digitizeDerive";

/** Element palette — exported so main.tsx colours section traces identically (feature 005). */
export const SERIES_COLORS = [0x4f83cc, 0xcc7a4f, 0x6aa84f, 0x9b59b6, 0xc0392b];

// ---------------------------------------------------------------------------------------
// Feature 005 (cross-sections): the section-view content contract between main.tsx (which
// SLICES — section/slice.ts over the already-returned payloads) and the Viewer (which only
// DRAWS). All 2D coordinates are in-plane (du, dw); the Viewer lifts them onto the plane.
// ---------------------------------------------------------------------------------------

/** One digitized line as drawn in the section view. `sign` is the EFFECTIVE flip sign
 *  (per-surface flip XOR relation default — contracts/younging-derivation.md §3), so the
 *  younging affordance always shows the side the FIT will see. */
export interface DigLineView {
  lineId: string;
  color: number;
  isTarget: boolean;
  sign: 1 | -1;
  /** The LINE's own stored plane normal (may be anti-parallel to the viewing frame's s if
   *  digitized from a twin baseline drawn the other way) — poles derive from THIS. */
  s: [number, number, number];
  vertices: [number, number, number][]; // world (on the plane)
}

export interface SectionContent {
  /** Contact-mode traces (or solid-mode unfilled fallback chains), per source surface. */
  traces: { color: number; polys: [number, number][][] }[];
  /** Solid-mode filled unit intervals (closed loops, holes classified). */
  fills: { color: number; shapes: { outer: [number, number][]; holes: [number, number][][] }[] }[];
  /** Authored contact points within the slab tolerance (backdrop). */
  backdrop: { p: [number, number]; color: number }[];
  /** Digitized lines lying on this plane. */
  lines: DigLineView[];
}

export interface SectionViewState {
  /** `${setId}:${index}` — camera/content effect key. */
  key: string;
  label: string;
  frame: SectionFrame;
  zRange: [number, number];
  content: SectionContent;
}

/** One 3D translucent plane quad (US-SECT-5), computed by main.tsx per shown set-member. */
export interface SectionQuad {
  frame: SectionFrame;
  zRange: [number, number];
  active: boolean;
}

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

/** Build the authored points/normals overlay for parametric surfaces. `unit` is the size baseline
 *  (data width / 100); the three `sizes` multipliers scale spheres / arrows / labels off it. Points
 *  become labeled spheres; orientations become labeled arrows (tail at the reading). Colored by the
 *  element's palette index (matching the mesh series colors). */
function buildAuthoredOverlay(
  data: AuthoredOverlay,
  unit: number,
  sizes: { point: number; arrow: number; label: number },
): THREE.Group {
  const g = new THREE.Group();
  const color = (i: number) => SERIES_COLORS[i % SERIES_COLORS.length];
  const sphereR = unit * sizes.point;
  const arrowLen = unit * sizes.arrow * 8; // ×8 so 1.0× reads sensibly vs a 1.0× sphere
  const labelScale = unit * sizes.label * 4; // ×4 tuned so 1.0× is legible at the sphere's scale
  // One shared sphere geometry (all points same radius); per-point material only for color reuse.
  const sphereGeom = new THREE.SphereGeometry(sphereR, 16, 12);
  const mats = new Map<number, THREE.MeshLambertMaterial>();
  const matFor = (c: number) => {
    let m = mats.get(c);
    if (!m) {
      m = new THREE.MeshLambertMaterial({ color: c });
      mats.set(c, m);
    }
    return m;
  };

  for (const p of data.points) {
    const c = color(p.colorIndex);
    const mesh = new THREE.Mesh(sphereGeom, matFor(c));
    mesh.position.set(...p.pos);
    g.add(mesh);
    // Label offset up by a sphere-radius so it doesn't sit inside the sphere.
    const label = makeLabel(String(p.row), c, [p.pos[0], p.pos[1], p.pos[2] + sphereR * 1.5]);
    label.scale.setScalar(labelScale);
    g.add(label);
  }

  for (const n of data.normals) {
    // `n.dir` is the true geometric up-normal (⊥ the surface), converted from the table's geologic
    // dip/dip-direction upstream (buildAuthoredOverlay → dipDirToNormal). It is NOT the down-dip
    // vector nor the fit pole: the arrow points UP-and-up-dip, out of the surface. See OverlayNormal.
    const c = color(n.colorIndex);
    const d = new THREE.Vector3(...n.dir);
    if (d.lengthSq() === 0) continue; // degenerate pole — skip
    d.normalize();
    const tail = new THREE.Vector3(...n.pos);
    const arrow = new THREE.ArrowHelper(d, tail, arrowLen, c, arrowLen * 0.25, arrowLen * 0.12);
    g.add(arrow);
    const tip = tail.clone().addScaledVector(d, arrowLen * 1.1);
    const label = makeLabel(String(n.row), c, [tip.x, tip.y, tip.z]);
    label.scale.setScalar(labelScale);
    g.add(label);
  }
  return g;
}

/** Dispose everything an overlay group owns (geometries, materials, label textures) before dropping
 *  it — sphere geometry is shared so dispose it once; ArrowHelper owns its own line/cone. */
function disposeOverlay(g: THREE.Group): void {
  const seenGeom = new Set<THREE.BufferGeometry>();
  g.traverse((o) => {
    if (o instanceof THREE.Mesh) {
      if (!seenGeom.has(o.geometry)) {
        seenGeom.add(o.geometry);
        o.geometry.dispose();
      }
      (o.material as THREE.Material).dispose();
    } else if (o instanceof THREE.Sprite) {
      const m = o.material as THREE.SpriteMaterial;
      m.map?.dispose();
      m.dispose();
    } else if (o instanceof THREE.ArrowHelper) {
      o.dispose();
    }
  });
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
   *  camera on Export, and `debugCounts()` for tests to assert overlay geometry got built. */
  viewApiRef?: { current: { getView: () => ViewState; debugCounts: () => { spheres: number; arrows: number } } | null };
  /** The authored points/normals overlay for parametric surfaces (viewer-agnostic, world-space).
   *  Absent/empty ⇒ nothing to draw. */
  authoredData?: AuthoredOverlay;
  /** Show the authored points/normals overlay. */
  showAuthored?: boolean;
  /** Size multipliers for the overlay (each defaults to 1.0). All scale a width/100 baseline. */
  overlaySizes?: { point: number; arrow: number; label: number };
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
  // --- feature 005: cross-sections ---
  /** When set (a NEW object), enter 2-point top-down SECTION-BASELINE digitization (clone of
   *  the bounds gesture, US-SECT-1 ↔ US-BOUNDS-1). Second click → `onSectionBaseline(a, b)`. */
  drawSection?: { nonce: number };
  onSectionBaseline?: (a: [number, number], b: [number, number]) => void;
  /** The open section view (undefined ⇒ ordinary 3D). Camera locks orthographic face-on
   *  (u right, Z up); 3D content hides; the section content groups render instead. */
  sectionView?: SectionViewState;
  /** Step to the adjacent parallel section (PageUp/PageDown while in section view). */
  onSectionStep?: (dir: -1 | 1) => void;
  /** Translucent plane quads shown in the 3D view (US-SECT-5). */
  sectionQuads?: SectionQuad[];
  /** Digitizing tool state (armed = plane-locked pointer edits are live). */
  digitize?: { armed: boolean };
  onDigitizeClick?: (world: [number, number, number]) => void;
  onDigitizeMove?: (lineId: string, vertexIndex: number, world: [number, number, number]) => void;
  onDigitizeDelete?: (lineId: string, vertexIndex: number) => void;
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
    rubber: THREE.Line | null; // live rectangle/baseline during digitization
    framedKey: string | null; // the data-bounds key we last auto-framed (frame only on NEW data)
    overlay: THREE.Group | null; // authored points/normals overlay
    // --- feature 005: cross-sections ---
    sectionActive: boolean; // true while the section view owns the camera (3D content hidden)
    sectionCam: THREE.OrthographicCamera | null;
    sectionControls: OrbitControls | null;
    sectionGroup: THREE.Group | null; // traces/fills/backdrop/digitized lines
    quadGroup: THREE.Group | null; // the 3D translucent plane quads
  } | null>(null);
  // Live digitize-callback ref so pointer handlers never capture stale closures.
  const digitizeCbRef = useRef<{
    click?: (w: [number, number, number]) => void;
    move?: (lineId: string, vi: number, w: [number, number, number]) => void;
    del?: (lineId: string, vi: number) => void;
    lines: DigLineView[];
    armed: boolean;
  }>({ lines: [], armed: false });
  digitizeCbRef.current = {
    click: props.onDigitizeClick,
    move: props.onDigitizeMove,
    del: props.onDigitizeDelete,
    lines: props.sectionView?.content.lines ?? [],
    armed: !!props.digitize?.armed,
  };

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

    stateRef.current = {
      renderer, scene, camera, controls, group,
      axes: null, boundsBox: null, rubber: null, framedKey: null, overlay: null,
      sectionActive: false, sectionCam: null, sectionControls: null, sectionGroup: null, quadGroup: null,
    };
    if (props.viewApiRef) {
      props.viewApiRef.current = {
        getView: () => ({
          camera: camera.position.toArray() as [number, number, number],
          target: controls.target.toArray() as [number, number, number],
          up: camera.up.toArray() as [number, number, number],
          fov: camera.fov,
        }),
        // Overlay geometry counts (spheres = points, arrows = normals) for test assertions.
        debugCounts: () => {
          const ov = stateRef.current?.overlay;
          let spheres = 0;
          let arrows = 0;
          ov?.traverse((o) => {
            if (o instanceof THREE.Mesh && (o.geometry as THREE.BufferGeometry).type === "SphereGeometry") spheres++;
            if (o instanceof THREE.ArrowHelper) arrows++;
          });
          return { spheres, arrows };
        },
      };
    }
    let raf = 0;
    const tick = () => {
      const st = stateRef.current;
      // Section view swaps in the orthographic camera + its pan/zoom-only controls (005).
      if (st?.sectionActive && st.sectionCam) {
        st.sectionControls?.update();
        renderer.render(scene, st.sectionCam);
      } else {
        controls.update(); // damping only; no scene rotation
        renderer.render(scene, camera);
      }
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
    gnomon.visible = !st.sectionActive; // 005: 3D content stays hidden while a section is open
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
      box.visible = !st.sectionActive; // 005: hidden while a section is open
      st.scene.add(box);
      st.boundsBox = box;
    }
  }, [props.extent, props.showBounds]);

  // Authored points/normals overlay (parametric surfaces). Rebuilt when the data, visibility, sizes,
  // or the size baseline change; disposed + removed on hide. Mirrors the showBounds idiom.
  const sizes = props.overlaySizes;
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.overlay) {
      st.scene.remove(st.overlay);
      disposeOverlay(st.overlay);
      st.overlay = null;
    }
    if (!props.showAuthored || !props.authoredData) return;
    if (props.authoredData.points.length === 0 && props.authoredData.normals.length === 0) return;
    // Size baseline = data width / 100 (scale-invariant). Fall back to the model extent when there
    // are no meshes yet (e.g. authored-but-not-yet-fitted), so the overlay is still sized sanely.
    const span = dataBox
      ? Math.max(dataBox.max[0] - dataBox.min[0], dataBox.max[1] - dataBox.min[1], dataBox.max[2] - dataBox.min[2])
      : Math.max(props.extent[1] - props.extent[0], props.extent[3] - props.extent[2], props.extent[5] - props.extent[4]);
    const unit = (span || 1) / 100;
    const overlay = buildAuthoredOverlay(props.authoredData, unit, {
      point: sizes?.point ?? 1,
      arrow: sizes?.arrow ?? 1,
      label: sizes?.label ?? 1,
    });
    overlay.visible = !st.sectionActive; // 005: hidden while a section is open
    st.scene.add(overlay);
    st.overlay = overlay;
  }, [props.authoredData, props.showAuthored, sizes?.point, sizes?.arrow, sizes?.label, dataBox, props.extent]);

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

  // =====================================================================================
  // Feature 005 — cross-sections.
  // =====================================================================================

  // SECTION-BASELINE digitization (US-SECT-1): a clone of the bounds gesture above — snap
  // top-down, two ground-plane clicks A→B, amber rubber LINE, Escape cancels. A degenerate
  // baseline (shorter than 0.1% of the extent diagonal) ignores the second click.
  useEffect(() => {
    const st = stateRef.current;
    if (!st || !props.drawSection) return;
    const { renderer, camera, controls, scene } = st;
    const dom = renderer.domElement;
    const [ex0, ex1, ey0, ey1, ez0, ez1] = props.extent;
    const bmin = dataBox ? dataBox.min : ([ex0, ey0, ez0] as [number, number, number]);
    const bmax = dataBox ? dataBox.max : ([ex1, ey1, ez1] as [number, number, number]);
    const cx = (bmin[0] + bmax[0]) / 2;
    const cy = (bmin[1] + bmax[1]) / 2;
    const zmid = (bmin[2] + bmax[2]) / 2;
    const span = Math.max(bmax[0] - bmin[0], bmax[1] - bmin[1]) || 1;
    const minLen = 1e-3 * Math.hypot(ex1 - ex0, ey1 - ey0, ez1 - ez0); // BASELINE_MIN_FRACTION

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
    controls.enabled = false;

    const raycaster = new THREE.Raycaster();
    const plane = new THREE.Plane(new THREE.Vector3(0, 0, 1), -zmid);
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

    let pointA: THREE.Vector3 | null = null;
    const clearRubber = () => {
      if (st.rubber) {
        scene.remove(st.rubber);
        st.rubber.geometry.dispose();
        st.rubber = null;
      }
    };
    const onMove = (e: PointerEvent) => {
      if (!pointA) return;
      const b = groundXY(e);
      if (!b) return;
      clearRubber();
      const line = new THREE.Line(
        new THREE.BufferGeometry().setFromPoints([
          new THREE.Vector3(pointA.x, pointA.y, zmid),
          new THREE.Vector3(b.x, b.y, zmid),
        ]),
        new THREE.LineBasicMaterial({ color: 0xfacc15 }),
      );
      scene.add(line);
      st.rubber = line;
    };
    const onDown = (e: PointerEvent) => {
      const p = groundXY(e);
      if (!p) return;
      if (!pointA) {
        pointA = p.clone();
        return;
      }
      if (Math.hypot(p.x - pointA.x, p.y - pointA.y) < minLen) return; // degenerate — ignore
      const a: [number, number] = [pointA.x, pointA.y];
      const b: [number, number] = [p.x, p.y];
      cleanup();
      props.onSectionBaseline?.(a, b);
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
  }, [props.drawSection]);

  // SECTION VIEW camera (US-SECT-3): an OrthographicCamera on the side ŝ points toward,
  // looking −s at the plane, up = +Z — u to screen-right, Z up (contract §1). Controls are
  // pan/zoom only (enableRotate = false). Entering hides the 3D content; exiting restores
  // it and returns to the untouched perspective camera. PageUp/PageDown step slices.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    const sv = props.sectionView;
    if (!sv) {
      // exit (or never entered)
      if (st.sectionActive) {
        st.sectionControls?.dispose();
        st.sectionControls = null;
        st.sectionCam = null;
        st.sectionActive = false;
        st.controls.enabled = true;
        st.group.visible = true;
        if (st.axes) st.axes.visible = true;
        if (st.overlay) st.overlay.visible = true;
        if (st.boundsBox) st.boundsBox.visible = true;
        if (st.quadGroup) st.quadGroup.visible = true;
      }
      return;
    }
    const { renderer } = st;
    const dom = renderer.domElement;
    const aspect = dom.clientWidth / Math.max(dom.clientHeight, 1);
    const { frame, zRange } = sv;
    const [z0, z1] = zRange;
    const zmid = (z0 + z1) / 2;
    const center = fromInPlane(frame, frame.uLen / 2, zmid);
    const dist = Math.max(frame.uLen, z1 - z0, 1) * 2;
    let halfW = frame.uLen * 0.55;
    let halfH = Math.max(z1 - z0, 1) * 0.55;
    if (halfW / halfH > aspect) halfH = halfW / aspect;
    else halfW = halfH * aspect;

    if (!st.sectionCam) {
      st.sectionCam = new THREE.OrthographicCamera(-halfW, halfW, halfH, -halfH, dist * 1e-3, dist * 10);
      st.sectionControls = new OrbitControls(st.sectionCam, dom);
      st.sectionControls.enableRotate = false; // pan + zoom only — the view stays face-on
      st.sectionControls.enableDamping = false;
    }
    const cam = st.sectionCam;
    cam.left = -halfW;
    cam.right = halfW;
    cam.top = halfH;
    cam.bottom = -halfH;
    cam.near = dist * 1e-3;
    cam.far = dist * 10;
    cam.zoom = 1;
    cam.up.set(0, 0, 1);
    cam.position.set(center[0] + frame.s[0] * dist, center[1] + frame.s[1] * dist, center[2] + frame.s[2] * dist);
    cam.lookAt(center[0], center[1], center[2]);
    cam.updateProjectionMatrix();
    st.sectionControls!.target.set(center[0], center[1], center[2]);
    st.sectionControls!.update();

    st.sectionActive = true;
    st.controls.enabled = false; // the perspective controls sleep while the section owns input
    st.group.visible = false; // 3D meshes hidden — the section reads as a 2D drawing
    if (st.axes) st.axes.visible = false;
    if (st.overlay) st.overlay.visible = false;
    if (st.boundsBox) st.boundsBox.visible = false;
    if (st.quadGroup) st.quadGroup.visible = false;

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "PageUp") props.onSectionStep?.(-1);
      else if (e.key === "PageDown") props.onSectionStep?.(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [props.sectionView?.key]);

  // SECTION CONTENT (US-SECT-3/6, US-DIG-2): traces, filled unit intervals, backdrop points
  // and digitized lines with the younging affordance. All content is computed in-plane by
  // main.tsx (section/slice.ts); this effect only lifts it onto the plane and draws. Small
  // +s offsets layer lines over fills without z-fighting.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.sectionGroup) {
      st.sectionGroup.traverse((o) => {
        if (o instanceof THREE.Mesh || o instanceof THREE.Line || o instanceof THREE.Points) {
          (o as THREE.Mesh).geometry?.dispose();
          const m = (o as THREE.Mesh).material as THREE.Material | undefined;
          m?.dispose();
        }
      });
      st.scene.remove(st.sectionGroup);
      st.sectionGroup = null;
    }
    const sv = props.sectionView;
    if (!sv) return;
    const { frame, content } = sv;
    const lift = (l: number): [number, number, number] => [frame.s[0] * l, frame.s[1] * l, frame.s[2] * l];
    const L = Math.max(frame.uLen, 1) * 1e-3;
    const g = new THREE.Group();

    // world position of an in-plane point, lifted by `k` layers toward the camera
    const at = (p: [number, number], k: number): THREE.Vector3 => {
      const w = fromInPlane(frame, p[0], p[1]);
      const o = lift(k * L);
      return new THREE.Vector3(w[0] + o[0], w[1] + o[1], w[2] + o[2]);
    };

    // Filled unit intervals (solid mode). ShapeGeometry triangulates in local XY; the basis
    // matrix maps local x→u, y→w(=+Z), z→s onto the plane.
    const basis = new THREE.Matrix4().makeBasis(
      new THREE.Vector3(...frame.u),
      new THREE.Vector3(0, 0, 1),
      new THREE.Vector3(...frame.s),
    );
    basis.setPosition(new THREE.Vector3(...frame.origin));
    for (const f of content.fills) {
      for (const sh of f.shapes) {
        const shape = new THREE.Shape(sh.outer.map(([x, y]) => new THREE.Vector2(x, y)));
        for (const hole of sh.holes) {
          shape.holes.push(new THREE.Path(hole.map(([x, y]) => new THREE.Vector2(x, y))));
        }
        const geom = new THREE.ShapeGeometry(shape);
        geom.applyMatrix4(basis);
        g.add(new THREE.Mesh(geom, new THREE.MeshBasicMaterial({ color: f.color, side: THREE.DoubleSide })));
      }
    }

    // Traces (contact mode; also the solid-mode unfilled fallback chains).
    for (const t of content.traces) {
      for (const poly of t.polys) {
        if (poly.length < 2) continue;
        g.add(new THREE.Line(
          new THREE.BufferGeometry().setFromPoints(poly.map((p) => at(p, 1))),
          new THREE.LineBasicMaterial({ color: t.color }),
        ));
      }
    }

    // Backdrop authored points within the slab tolerance (US-SECT-3).
    const dotR = Math.max(frame.uLen, 1) * 0.004;
    const dotGeom = new THREE.SphereGeometry(dotR, 10, 8);
    for (const bp of content.backdrop) {
      const mesh = new THREE.Mesh(dotGeom.clone(), new THREE.MeshBasicMaterial({ color: bp.color }));
      mesh.position.copy(at(bp.p, 2));
      g.add(mesh);
    }

    // Digitized lines + the YOUNGING AFFORDANCE (US-DIG-2): the polyline, its vertices, a
    // direction arrow at the end, and hachure ticks on the EFFECTIVE younger side
    // (sign · (ŝ × t̂) — what the fit will see; contracts §2–3).
    const vR = Math.max(frame.uLen, 1) * (0.006);
    for (const line of content.lines) {
      const pts = line.vertices.map((v) => {
        const o = lift(3 * L);
        return new THREE.Vector3(v[0] + o[0], v[1] + o[1], v[2] + o[2]);
      });
      if (pts.length >= 2) {
        g.add(new THREE.Line(
          new THREE.BufferGeometry().setFromPoints(pts),
          new THREE.LineBasicMaterial({ color: line.color, linewidth: line.isTarget ? 2 : 1 }),
        ));
      }
      const sGeom = new THREE.SphereGeometry(line.isTarget ? vR * 1.4 : vR, 10, 8);
      for (const p of pts) {
        const m = new THREE.Mesh(sGeom.clone(), new THREE.MeshBasicMaterial({ color: line.color }));
        m.position.copy(p);
        g.add(m);
      }
      // per-segment poles → hachures; last segment → direction arrow. Derived from the
      // LINE's own plane normal (not the viewing frame's), so a twin-baseline section
      // reads the same younging the fit will use.
      const poles = derivePoles(line.vertices, line.s);
      const hachLen = Math.max(frame.uLen, 1) * 0.015;
      for (const d of poles) {
        const mid = new THREE.Vector3(...d.pos).add(new THREE.Vector3(...lift(3 * L)));
        const younger = new THREE.Vector3(...d.pole).multiplyScalar(line.sign * hachLen);
        g.add(new THREE.Line(
          new THREE.BufferGeometry().setFromPoints([mid, mid.clone().add(younger)]),
          new THREE.LineBasicMaterial({ color: line.color, transparent: true, opacity: 0.8 }),
        ));
      }
      if (line.vertices.length >= 2) {
        const a = line.vertices[line.vertices.length - 2];
        const b = line.vertices[line.vertices.length - 1];
        const dir = new THREE.Vector3(b[0] - a[0], b[1] - a[1], b[2] - a[2]);
        if (dir.lengthSq() > 0) {
          dir.normalize();
          const tip = pts[pts.length - 1];
          g.add(new THREE.ArrowHelper(dir, tip, hachLen * 2, line.color, hachLen, hachLen * 0.5));
        }
      }
    }

    st.scene.add(g);
    st.sectionGroup = g;
  }, [props.sectionView]);

  // 3D PLANE QUADS (US-SECT-5): one translucent quad per shown plane, with its baseline
  // edge and viewing-normal arrow. Hidden while a section view is open (it owns the scene).
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    if (st.quadGroup) {
      st.quadGroup.traverse((o) => {
        if (o instanceof THREE.Mesh || o instanceof THREE.Line) {
          (o as THREE.Mesh).geometry?.dispose();
          ((o as THREE.Mesh).material as THREE.Material | undefined)?.dispose();
        }
      });
      st.scene.remove(st.quadGroup);
      st.quadGroup = null;
    }
    const quads = props.sectionQuads;
    if (!quads || quads.length === 0) return;
    const g = new THREE.Group();
    for (const q of quads) {
      const { frame, zRange, active } = q;
      const [z0, z1] = zRange;
      const zmid = (z0 + z1) / 2;
      const center = fromInPlane(frame, frame.uLen / 2, zmid);
      const geom = new THREE.PlaneGeometry(frame.uLen, z1 - z0);
      const basis = new THREE.Matrix4().makeBasis(
        new THREE.Vector3(...frame.u),
        new THREE.Vector3(0, 0, 1),
        new THREE.Vector3(...frame.s),
      );
      basis.setPosition(new THREE.Vector3(...center));
      geom.applyMatrix4(basis);
      g.add(new THREE.Mesh(
        geom,
        new THREE.MeshBasicMaterial({
          color: active ? 0xf59e0b : 0x22d3ee,
          transparent: true,
          opacity: active ? 0.3 : 0.15,
          side: THREE.DoubleSide,
          depthWrite: false,
        }),
      ));
      // baseline edge (at z0) + the out-of-plane viewing normal at the centre
      const b0 = fromInPlane(frame, 0, z0);
      const b1 = fromInPlane(frame, frame.uLen, z0);
      g.add(new THREE.Line(
        new THREE.BufferGeometry().setFromPoints([new THREE.Vector3(...b0), new THREE.Vector3(...b1)]),
        new THREE.LineBasicMaterial({ color: 0xf59e0b }),
      ));
      const arrowLen = Math.max(frame.uLen, z1 - z0) * 0.15;
      g.add(new THREE.ArrowHelper(
        new THREE.Vector3(...frame.s),
        new THREE.Vector3(...center),
        arrowLen,
        active ? 0xf59e0b : 0x22d3ee,
      ));
    }
    g.visible = !st.sectionActive;
    st.scene.add(g);
    st.quadGroup = g;
  }, [props.sectionQuads]);

  // DIGITIZING pointer tool (US-DIG-1): plane-locked click-to-append; drag a vertex to move
  // it (committed on pointerup); alt-click a vertex to delete it. Reuses the same
  // ray∩plane picking as the gestures above, against the ACTIVE section plane. The left
  // button is free because rotate is disabled in section view.
  useEffect(() => {
    const st = stateRef.current;
    const sv = props.sectionView;
    if (!st || !sv || !props.digitize?.armed) return;
    const { renderer } = st;
    const dom = renderer.domElement;
    const frame = sv.frame;
    const plane = new THREE.Plane(
      new THREE.Vector3(...frame.s),
      -(frame.s[0] * frame.origin[0] + frame.s[1] * frame.origin[1] + frame.s[2] * frame.origin[2]),
    );
    const raycaster = new THREE.Raycaster();
    const hitPlane = (e: PointerEvent): [number, number, number] | null => {
      const cam = st.sectionCam;
      if (!cam) return null;
      const r = dom.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - r.left) / r.width) * 2 - 1,
        -((e.clientY - r.top) / r.height) * 2 + 1,
      );
      raycaster.setFromCamera(ndc, cam);
      const hit = new THREE.Vector3();
      return raycaster.ray.intersectPlane(plane, hit) ? [hit.x, hit.y, hit.z] : null;
    };
    const pickRadius = Math.max(frame.uLen, 1) * 0.02;
    const nearestVertex = (w: [number, number, number]): { lineId: string; vi: number } | null => {
      let best: { lineId: string; vi: number; d: number } | null = null;
      for (const line of digitizeCbRef.current.lines) {
        if (!line.isTarget) continue; // only the target surface's lines are editable
        line.vertices.forEach((v, vi) => {
          const d = Math.hypot(v[0] - w[0], v[1] - w[1], v[2] - w[2]);
          if (d < pickRadius && (!best || d < best.d)) best = { lineId: line.lineId, vi, d };
        });
      }
      return best;
    };

    let drag: { lineId: string; vi: number } | null = null;
    let downAt: [number, number] | null = null;
    const onDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      const w = hitPlane(e);
      if (!w) return;
      downAt = [e.clientX, e.clientY];
      const hit = nearestVertex(w);
      if (hit && e.altKey) {
        digitizeCbRef.current.del?.(hit.lineId, hit.vi);
        downAt = null;
        return;
      }
      if (hit) {
        drag = hit;
        if (st.sectionControls) st.sectionControls.enabled = false; // don't pan while dragging
      }
    };
    const onUp = (e: PointerEvent) => {
      if (e.button !== 0 || !downAt) return;
      const moved = Math.hypot(e.clientX - downAt[0], e.clientY - downAt[1]) > 3;
      const w = hitPlane(e);
      if (drag) {
        if (w && moved) digitizeCbRef.current.move?.(drag.lineId, drag.vi, w);
        drag = null;
        if (st.sectionControls) st.sectionControls.enabled = true;
      } else if (w && !moved) {
        digitizeCbRef.current.click?.(w);
      }
      downAt = null;
    };
    dom.addEventListener("pointerdown", onDown);
    dom.addEventListener("pointerup", onUp);
    dom.style.cursor = "crosshair";
    return () => {
      dom.removeEventListener("pointerdown", onDown);
      dom.removeEventListener("pointerup", onUp);
      dom.style.cursor = "";
      if (st.sectionControls) st.sectionControls.enabled = true;
    };
  }, [props.sectionView?.key, props.digitize?.armed]);

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
