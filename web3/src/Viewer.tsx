// Minimal three.js viewer for the testbed: renders the streamed surfaces (one mesh per
// SurfacePayload). Adapted/simplified from web/src/Viewer.tsx — enough to show the meshed
// model beside the hierarchy panel (US5). Color is by series index.
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import { useEffect, useRef } from "react";
import * as THREE from "three";
import type { SurfacePayload } from "./protocol";

const SERIES_COLORS = [0x4f83cc, 0xcc7a4f, 0x6aa84f, 0x9b59b6, 0xc0392b];

export function Viewer(props: { surfaces: SurfacePayload[] }) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    group: THREE.Group;
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
    camera.lookAt(500, 500, 500);
    scene.add(new THREE.AmbientLight(0xffffff, 0.7));
    const dir = new THREE.DirectionalLight(0xffffff, 0.6);
    dir.position.set(1, 1, 1);
    scene.add(dir);
    const group = new THREE.Group();
    scene.add(group);

    stateRef.current = { renderer, scene, camera, group };
    let raf = 0;
    const tick = () => {
      group.rotation.z += 0.0015;
      renderer.render(scene, camera);
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => {
      cancelAnimationFrame(raf);
      renderer.dispose();
      mount.removeChild(renderer.domElement);
    };
  }, []);

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

  return <div ref={mountRef} style={{ width: "100%", height: "100%", minHeight: 400 }} />;
}
