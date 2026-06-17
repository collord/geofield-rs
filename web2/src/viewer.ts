// A minimal vanilla-three.js viewer for the scale scenario: the borehole data points
// (colored by interface) plus the meshed contact surfaces of the (decimated) model.
//
// This is the visual companion to the timing table — it renders the SAME stack the
// benchmark fits and meshes, so you can see the 3 sediment contacts, the basalt
// unconformity, and the fault offset on the data the harness reports timings for.

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { interfaceColor, relationInfo } from "./relations";

export interface RenderMesh {
  interfaceId: number;
  /** Owning series' relation (0=Erode 1=Onlap 2=Conformable 3=Basement). */
  relation: number;
  positions: Float32Array; // 3 * V
  normals: Float32Array; // 3 * V (may be empty)
  indices: Uint32Array; // 3 * T
}

/** How surfaces are colored: per-contact hue, or one color per stack relation. */
export type ColorMode = "interface" | "relation";

export interface DataPoint {
  x: number;
  y: number;
  z: number;
  interfaceId: number;
}

// Surface/point colors come from the shared relations module so the scene and the legend
// panel agree. A surface is drawn either by its individual contact hue ("interface" mode)
// or by its stack relation ("relation" mode) — see setColorMode.
function colorFor(mesh: RenderMesh, mode: ColorMode): number {
  return mode === "relation" ? relationInfo(mesh.relation).color : interfaceColor(mesh.interfaceId);
}

export interface Viewer {
  setSurfaces(meshes: RenderMesh[]): void;
  /** Remove all surfaces (start of a progressive stream). */
  clearSurfaces(): void;
  /** Add ONE surface (progressive rendering as the worker streams them). */
  addSurface(mesh: RenderMesh): void;
  setPoints(points: DataPoint[]): void;
  /** Toggle the data-point cloud; returns the new visibility. */
  togglePoints(): boolean;
  /** Recolor all current surfaces by contact hue or by stack relation. */
  setColorMode(mode: ColorMode): void;
}

/** Build the scene into `mount`. `extent` is [minX,maxX,minY,maxY,minZ,maxZ]. */
export function createViewer(mount: HTMLElement, extent: number[]): Viewer {
  const [minX, maxX, minY, maxY, minZ, maxZ] = extent;
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const cz = (minZ + maxZ) / 2;
  const span = Math.max(maxX - minX, maxY - minY, maxZ - minZ);

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x1a1d23);

  const camera = new THREE.PerspectiveCamera(
    50,
    mount.clientWidth / Math.max(mount.clientHeight, 1),
    span * 0.01,
    span * 20,
  );
  camera.up.set(0, 0, 1); // Z up (geological convention)
  // The model is wide and shallow (16,500 ft across, ~4,700 ft deep) — frame from a low
  // oblique angle so the stacked contacts + the fault offset read, not a top-down map.
  camera.position.set(cx + span * 0.9, cy - span * 1.1, cz + span * 0.55);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(mount.clientWidth, mount.clientHeight || 480);
  mount.appendChild(renderer.domElement);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.target.set(cx, cy, cz);
  controls.update();

  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const key = new THREE.DirectionalLight(0xffffff, 0.9);
  key.position.set(1, -1, 2);
  scene.add(key);

  scene.add(
    new THREE.Box3Helper(
      new THREE.Box3(
        new THREE.Vector3(minX, minY, minZ),
        new THREE.Vector3(maxX, maxY, maxZ),
      ),
      new THREE.Color(0x444a55),
    ),
  );

  const surfaceGroup = new THREE.Group();
  const dataGroup = new THREE.Group();
  scene.add(surfaceGroup, dataGroup);

  let colorMode: ColorMode = "interface";

  const clear = (g: THREE.Group) => {
    for (const c of [...g.children]) {
      g.remove(c);
      const obj = c as THREE.Mesh | THREE.Points;
      obj.geometry?.dispose();
      const m = (obj as THREE.Mesh).material;
      if (Array.isArray(m)) m.forEach((mm) => mm.dispose());
      else m?.dispose();
    }
  };

  let raf = 0;
  const animate = () => {
    controls.update();
    renderer.render(scene, camera);
    raf = requestAnimationFrame(animate);
  };
  animate();

  const onResize = () => {
    const w = mount.clientWidth;
    const h = mount.clientHeight || 480;
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h);
  };
  window.addEventListener("resize", onResize);
  // keep the listener for the page lifetime (single-page bench); no teardown needed.
  void raf;

  const addOne = (m: RenderMesh) => {
    if (m.indices.length === 0) return;
    const geom = new THREE.BufferGeometry();
    geom.setAttribute("position", new THREE.BufferAttribute(m.positions, 3));
    geom.setIndex(new THREE.BufferAttribute(m.indices, 1));
    if (m.normals.length === m.positions.length) {
      geom.setAttribute("normal", new THREE.BufferAttribute(m.normals, 3));
    } else {
      geom.computeVertexNormals();
    }
    const mat = new THREE.MeshStandardMaterial({
      color: colorFor(m, colorMode),
      metalness: 0.1,
      roughness: 0.7,
      side: THREE.DoubleSide,
      transparent: true,
      opacity: 0.85,
    });
    const mesh = new THREE.Mesh(geom, mat);
    mesh.userData.render = m; // keep the source mesh so setColorMode can recolor
    surfaceGroup.add(mesh);
  };

  return {
    setSurfaces(meshes: RenderMesh[]) {
      clear(surfaceGroup);
      for (const m of meshes) addOne(m);
    },
    clearSurfaces() {
      clear(surfaceGroup);
    },
    addSurface(m: RenderMesh) {
      addOne(m);
    },

    setPoints(points: DataPoint[]) {
      clear(dataGroup);
      if (points.length === 0) return;
      // One Points cloud (3000+ points = one draw call). Per-vertex color by interface.
      const pos = new Float32Array(points.length * 3);
      const col = new Float32Array(points.length * 3);
      const c = new THREE.Color();
      points.forEach((p, i) => {
        pos[3 * i] = p.x;
        pos[3 * i + 1] = p.y;
        pos[3 * i + 2] = p.z;
        c.setHex(interfaceColor(p.interfaceId));
        col[3 * i] = c.r;
        col[3 * i + 1] = c.g;
        col[3 * i + 2] = c.b;
      });
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
      geom.setAttribute("color", new THREE.BufferAttribute(col, 3));
      const mat = new THREE.PointsMaterial({
        size: span * 0.0025, // small so the surfaces stay legible through the cloud
        vertexColors: true,
        sizeAttenuation: true,
        opacity: 0.85,
        transparent: true,
      });
      dataGroup.add(new THREE.Points(geom, mat));
    },

    togglePoints() {
      dataGroup.visible = !dataGroup.visible;
      return dataGroup.visible;
    },

    setColorMode(mode: ColorMode) {
      colorMode = mode;
      for (const c of surfaceGroup.children) {
        const m = (c as THREE.Mesh).userData.render as RenderMesh | undefined;
        const mat = (c as THREE.Mesh).material as THREE.MeshStandardMaterial;
        if (m && mat) mat.color.setHex(colorFor(m, colorMode));
      }
    },
  };
}
