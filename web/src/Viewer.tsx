import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { MeshData, SurfaceMesh } from "./model";
import {
  SurfacePointRow,
  OrientationRow,
  EXTENT,
  SERIES_ORDER,
} from "./data";
import { azimuthDipToPole } from "./model";

interface Props {
  mesh: MeshData;
  surfacePoints: SurfacePointRow[];
  orientations: OrientationRow[];
}

const FORMATION_COLORS: Record<string, number> = {
  rock1: 0xe07b39,
  rock2: 0x3d9be9,
};

/** Color for one surface. Contacts use the formation palette (rock1/rock2). Closed
 *  lithology-unit solids use an evenly-spaced HSL hue per unit (so adjacent layers are
 *  maximally distinct, unlike a string hash where "unit 1"/"unit 2" collide), shifted
 *  in lightness per fault block so block A's and block B's layers also differ. */
function pickColor(surf: SurfaceMesh): number {
  if (surf.unit === undefined) {
    return FORMATION_COLORS[surf.formation] ?? colorFor(surf.formation);
  }
  // Golden-angle hue spacing → well-separated colors for any unit count.
  const hue = (surf.unit * 0.61803398875) % 1;
  // Per-block lightness shift: block 0 darker, block 1 lighter (and so on).
  const block = surf.block ?? 0;
  const light = 0.45 + 0.12 * (block % 3);
  return hslToHex(hue, 0.6, light);
}

function hslToHex(h: number, s: number, l: number): number {
  const f = (n: number) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255);
  };
  return (f(0) << 16) | (f(8) << 8) | f(4);
}

export function Viewer({ mesh, surfacePoints, orientations }: Props) {
  const mountRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<THREE.Scene>();
  const rendererRef = useRef<THREE.WebGLRenderer>();
  const cameraRef = useRef<THREE.PerspectiveCamera>();
  const surfaceGroupRef = useRef<THREE.Group>();
  const dataGroupRef = useRef<THREE.Group>();

  // One-time scene setup.
  useEffect(() => {
    const mount = mountRef.current!;
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x1a1d23);

    const [minX, maxX, minY, maxY, minZ, maxZ] = EXTENT;
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    const cz = (minZ + maxZ) / 2;
    const span = Math.max(maxX - minX, maxY - minY, maxZ - minZ);

    const camera = new THREE.PerspectiveCamera(
      50,
      mount.clientWidth / mount.clientHeight,
      span * 0.01,
      span * 20,
    );
    camera.up.set(0, 0, 1); // Z is up (geological convention)
    camera.position.set(cx + span * 1.4, cy - span * 1.6, cz + span * 1.2);

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(mount.clientWidth, mount.clientHeight);
    mount.appendChild(renderer.domElement);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(cx, cy, cz);
    controls.update();

    scene.add(new THREE.AmbientLight(0xffffff, 0.6));
    const key = new THREE.DirectionalLight(0xffffff, 0.9);
    key.position.set(1, -1, 2);
    scene.add(key);

    // Bounding box of the model extent.
    const box = new THREE.Box3(
      new THREE.Vector3(minX, minY, minZ),
      new THREE.Vector3(maxX, maxY, maxZ),
    );
    scene.add(new THREE.Box3Helper(box, new THREE.Color(0x444a55)));

    const surfaceGroup = new THREE.Group();
    const dataGroup = new THREE.Group();
    scene.add(surfaceGroup, dataGroup);

    sceneRef.current = scene;
    rendererRef.current = renderer;
    cameraRef.current = camera;
    surfaceGroupRef.current = surfaceGroup;
    dataGroupRef.current = dataGroup;

    let raf = 0;
    const animate = () => {
      controls.update();
      renderer.render(scene, camera);
      raf = requestAnimationFrame(animate);
    };
    animate();

    const onResize = () => {
      camera.aspect = mount.clientWidth / mount.clientHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(mount.clientWidth, mount.clientHeight);
    };
    window.addEventListener("resize", onResize);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", onResize);
      controls.dispose();
      renderer.dispose();
      mount.removeChild(renderer.domElement);
    };
  }, []);

  // Rebuild the contact surfaces whenever the mesh changes. One mesh per
  // interface, colored by formation, at its recovered isovalue.
  useEffect(() => {
    const group = surfaceGroupRef.current;
    if (!group) return;
    clearGroup(group);

    for (const surf of mesh.surfaces) {
      if (surf.triangleCount === 0) continue;
      const geom = new THREE.BufferGeometry();
      geom.setAttribute(
        "position",
        new THREE.BufferAttribute(surf.positions, 3),
      );
      geom.setIndex(new THREE.BufferAttribute(surf.indices, 1));
      // Exact per-vertex normals from the analytic field gradient (smooth shading
      // without the faceting of face-averaged normals). Fall back to computed
      // normals only if the contourer didn't supply them.
      if (surf.normals && surf.normals.length === surf.positions.length) {
        geom.setAttribute("normal", new THREE.BufferAttribute(surf.normals, 3));
      } else {
        geom.computeVertexNormals();
      }

      const color = pickColor(surf);
      const mat = new THREE.MeshStandardMaterial({
        color,
        metalness: 0.1,
        roughness: 0.7,
        side: THREE.DoubleSide,
        flatShading: false,
        transparent: true,
        opacity: 0.85,
        // Push the filled faces slightly back so the edge overlay sits on top
        // without z-fighting.
        polygonOffset: true,
        polygonOffsetFactor: 1,
        polygonOffsetUnits: 1,
      });
      const fill = new THREE.Mesh(geom, mat);

      // Edge overlay: show every mesh edge (threshold 0 keeps coplanar quad
      // edges too, so the dual-contour cell structure is visible). Darken the
      // formation color for the lines.
      const edges = new THREE.EdgesGeometry(geom, 0);
      const lineColor = new THREE.Color(color).multiplyScalar(0.45);
      const lines = new THREE.LineSegments(
        edges,
        new THREE.LineBasicMaterial({ color: lineColor }),
      );
      fill.add(lines); // child so it shares the surface's transform

      group.add(fill);
    }
  }, [mesh]);

  // Rebuild data markers (surface points as spheres, orientations as arrows).
  useEffect(() => {
    const group = dataGroupRef.current;
    if (!group) return;
    clearGroup(group);

    const span = Math.max(...EXTENT.filter((_, i) => i % 2 === 1));
    const r = span * 0.012;

    for (const sp of surfacePoints) {
      const color =
        FORMATION_COLORS[sp.formation] ?? colorFor(sp.formation);
      const s = new THREE.Mesh(
        new THREE.SphereGeometry(r, 16, 12),
        new THREE.MeshStandardMaterial({ color }),
      );
      s.position.set(sp.x, sp.y, sp.z);
      group.add(s);
    }

    const arrowLen = span * 0.12;
    for (const o of orientations) {
      const [gx, gy, gz] = azimuthDipToPole(o.azimuth, o.dip, o.polarity);
      const dir = new THREE.Vector3(gx, gy, gz).normalize();
      const color =
        FORMATION_COLORS[o.formation] ?? colorFor(o.formation);
      const arrow = new THREE.ArrowHelper(
        dir,
        new THREE.Vector3(o.x, o.y, o.z),
        arrowLen,
        color,
        arrowLen * 0.35,
        arrowLen * 0.22,
      );
      group.add(arrow);
    }
  }, [surfacePoints, orientations]);

  return <div ref={mountRef} style={{ width: "100%", height: "100%" }} />;
}

function clearGroup(group: THREE.Group) {
  for (let i = group.children.length - 1; i >= 0; i--) {
    const child = group.children[i];
    group.remove(child);
    child.traverse((o) => {
      const m = o as THREE.Mesh;
      if (m.geometry) m.geometry.dispose();
      if (m.material) {
        const mats = Array.isArray(m.material) ? m.material : [m.material];
        mats.forEach((mat) => mat.dispose());
      }
    });
  }
}

// Stable fallback color for any formation not in FORMATION_COLORS.
function colorFor(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return h & 0xffffff;
}

// Keep SERIES_ORDER referenced so unused-import lint stays quiet if needed later.
void SERIES_ORDER;
