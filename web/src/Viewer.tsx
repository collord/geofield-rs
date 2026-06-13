import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { MeshData } from "./model";
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

  // Rebuild the isosurface mesh whenever it changes.
  useEffect(() => {
    const group = surfaceGroupRef.current;
    if (!group) return;
    clearGroup(group);
    if (mesh.triangleCount === 0) return;

    const geom = new THREE.BufferGeometry();
    geom.setAttribute(
      "position",
      new THREE.BufferAttribute(mesh.positions, 3),
    );
    geom.setIndex(new THREE.BufferAttribute(mesh.indices, 1));
    geom.computeVertexNormals();

    const mat = new THREE.MeshStandardMaterial({
      color: 0x9aa7b4,
      metalness: 0.1,
      roughness: 0.75,
      side: THREE.DoubleSide,
      flatShading: false,
    });
    group.add(new THREE.Mesh(geom, mat));
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
