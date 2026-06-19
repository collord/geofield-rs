import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { MeshData, surfaceKey, pickColor, azimuthDipToPole } from "./model";
import { SurfacePointRow, OrientationRow, SeriesDef, EXTENT } from "./data";
import { relationInfo } from "./relations";

interface Props {
  mesh: MeshData;
  /** Surface keys (see `surfaceKey`) currently hidden via the object tree. */
  hidden: Set<string>;
  surfacePoints: SurfacePointRow[];
  orientations: OrientationRow[];
  series: SeriesDef[];
}

export function Viewer({ mesh, hidden, surfacePoints, orientations, series }: Props) {
  const mountRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<THREE.Scene>();
  const rendererRef = useRef<THREE.WebGLRenderer>();
  const cameraRef = useRef<THREE.PerspectiveCamera>();
  const surfaceGroupRef = useRef<THREE.Group>();
  const dataGroupRef = useRef<THREE.Group>();
  // Latest hidden set, read by the geometry-build effect (which keys off `mesh` only)
  // so a re-solve re-applies the current visibility without depending on `hidden`.
  const hiddenRef = useRef(hidden);
  // Latest series list, read by the data-marker effect for per-series coloring.
  const seriesRef = useRef(series);
  seriesRef.current = series;

  // Color a data marker by the relation family of its series.
  const colorForRow = (row: { series: number }): number =>
    relationInfo(seriesRef.current[row.series]?.relation ?? 0).color;

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

  // Rebuild the surfaces whenever the mesh changes. One mesh per (series, block,
  // interface/unit), colored by its series relation family.
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
      // without the faceting of face-averaged normals). Fall back to computed normals
      // only if the contourer didn't supply them.
      if (surf.normals && surf.normals.length === surf.positions.length) {
        geom.setAttribute("normal", new THREE.BufferAttribute(surf.normals, 3));
      } else {
        geom.computeVertexNormals();
      }

      const color = pickColor(surf);
      // Closed-volume unit solids (unit >= 0) render OPAQUE and front-face-only: they are
      // watertight, so you should not see through one solid into the layers behind it. Open
      // contact sheets stay translucent + double-sided so overlapping sheets remain visible.
      const isSolid = surf.unit >= 0;
      const mat = new THREE.MeshStandardMaterial({
        color,
        metalness: 0.1,
        roughness: 0.7,
        side: isSolid ? THREE.FrontSide : THREE.DoubleSide,
        flatShading: false,
        transparent: !isSolid,
        opacity: isSolid ? 1.0 : 0.85,
        // Push the filled faces slightly back so the edge overlay sits on top
        // without z-fighting.
        polygonOffset: true,
        polygonOffsetFactor: 1,
        polygonOffsetUnits: 1,
      });
      const fill = new THREE.Mesh(geom, mat);
      // Tag with the surface key so the visibility effect can toggle it, and apply the
      // current hidden state immediately (so a re-solve keeps hidden objects hidden).
      const key = surfaceKey(surf);
      fill.userData.surfaceKey = key;
      fill.visible = !hiddenRef.current.has(key);

      // Edge overlay: show every mesh edge (threshold 0 keeps coplanar quad edges too, so
      // the dual-contour cell structure is visible). Darken the surface color for the lines.
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

  // Toggle object visibility from the object tree without rebuilding geometry.
  useEffect(() => {
    hiddenRef.current = hidden;
    const group = surfaceGroupRef.current;
    if (!group) return;
    for (const child of group.children) {
      const key = child.userData?.surfaceKey as string | undefined;
      if (key !== undefined) child.visible = !hidden.has(key);
    }
  }, [hidden]);

  // Rebuild data markers (surface points as spheres, orientations as arrows), colored by
  // each row's series relation family.
  useEffect(() => {
    const group = dataGroupRef.current;
    if (!group) return;
    clearGroup(group);

    const span = Math.max(...EXTENT.filter((_, i) => i % 2 === 1));
    const r = span * 0.012;

    for (const sp of surfacePoints) {
      const s = new THREE.Mesh(
        new THREE.SphereGeometry(r, 16, 12),
        new THREE.MeshStandardMaterial({ color: colorForRow(sp) }),
      );
      s.position.set(sp.x, sp.y, sp.z);
      group.add(s);
    }

    const arrowLen = span * 0.12;
    for (const o of orientations) {
      const [gx, gy, gz] = azimuthDipToPole(o.azimuth, o.dip, o.polarity);
      const dir = new THREE.Vector3(gx, gy, gz).normalize();
      const arrow = new THREE.ArrowHelper(
        dir,
        new THREE.Vector3(o.x, o.y, o.z),
        arrowLen,
        colorForRow(o),
        arrowLen * 0.35,
        arrowLen * 0.22,
      );
      group.add(arrow);
    }
    // `series` is read via seriesRef inside colorForRow; depend on it so markers recolor
    // when a series' relation changes.
  }, [surfacePoints, orientations, series]);

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
