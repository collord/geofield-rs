// Minimal three.js viewer for the testbed: renders the streamed surfaces (one mesh per
// SurfacePayload), with user-driven orbit controls and labeled X/Y/Z axes spanning the
// data's bounding box. Adapted/simplified from web/src/Viewer.tsx. Color is by series index.

import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { SurfacePayload } from "./protocol";

const SERIES_COLORS = [0x4f83cc, 0xcc7a4f, 0x6aa84f, 0x9b59b6, 0xc0392b];

/** Data bounding box `[x0, x1, y0, y1, z0, z1]`. */
export type Extent = [number, number, number, number, number, number];

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

export function Viewer(props: { surfaces: SurfacePayload[]; extent: Extent; stale?: boolean }) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    controls: OrbitControls;
    group: THREE.Group;
    axes: THREE.Group | null;
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

    stateRef.current = { renderer, scene, camera, controls, group, axes: null };
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

  // Rebuild meshes whenever the surfaces change.
  useEffect(() => {
    const st = stateRef.current;
    if (!st) return;
    st.group.clear();
    for (const s of props.surfaces) {
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.BufferAttribute(s.positions, 3));
      geom.setAttribute("normal", new THREE.BufferAttribute(s.normals, 3));
      geom.setIndex(new THREE.BufferAttribute(s.indices, 1));
      const mat = new THREE.MeshStandardMaterial({
        color: SERIES_COLORS[s.series % SERIES_COLORS.length],
        side: THREE.DoubleSide,
        flatShading: true,
      });
      st.group.add(new THREE.Mesh(geom, mat));
    }
  }, [props.surfaces]);

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
