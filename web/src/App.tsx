import { useEffect, useRef, useState } from "react";
import {
  SEED_SURFACE_POINTS,
  SEED_ORIENTATIONS,
  SurfacePointRow,
  OrientationRow,
} from "./data";
import { initWasm, solveAndMesh, MeshData } from "./model";
import { SurfacePointTable, OrientationTable } from "./Tables";
import { Viewer } from "./Viewer";

const DEBOUNCE_MS = 300;

export default function App() {
  const [wasmReady, setWasmReady] = useState(false);
  const [surfacePoints, setSurfacePoints] = useState<SurfacePointRow[]>(
    SEED_SURFACE_POINTS,
  );
  const [orientations, setOrientations] =
    useState<OrientationRow[]>(SEED_ORIENTATIONS);
  const [res, setRes] = useState(32);
  const [mesh, setMesh] = useState<MeshData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyMs, setBusyMs] = useState<number | null>(null);

  useEffect(() => {
    initWasm().then(() => setWasmReady(true));
  }, []);

  // Debounced auto re-solve + re-mesh on any input change.
  const timer = useRef<number | null>(null);
  useEffect(() => {
    if (!wasmReady) return;
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const t0 = performance.now();
      const { mesh, error } = solveAndMesh(surfacePoints, orientations, res);
      setMesh(mesh);
      setError(error);
      setBusyMs(performance.now() - t0);
    }, DEBOUNCE_MS);
    return () => {
      if (timer.current) window.clearTimeout(timer.current);
    };
  }, [wasmReady, surfacePoints, orientations, res]);

  return (
    <div className="app">
      <header>
        <h1>geofield-rs · interactive geomodel</h1>
        <div className="status">
          {!wasmReady && <span>loading WASM…</span>}
          {wasmReady && mesh && (
            <>
              <span>
                {mesh.surfaces.length} surface
                {mesh.surfaces.length === 1 ? "" : "s"}
              </span>
              <span>{mesh.triangleCount.toLocaleString()} tris</span>
              <span>residual {fmt(mesh.residual)}</span>
              <span>iso-spread {fmt(mesh.isovalueSpread)}</span>
              {busyMs != null && <span>{busyMs.toFixed(0)} ms</span>}
            </>
          )}
          <label className="res">
            grid
            <input
              type="range"
              min={8}
              max={96}
              step={8}
              value={res}
              onChange={(e) => setRes(Number(e.target.value))}
            />
            {res}³
          </label>
        </div>
      </header>

      {error && <div className="error">solve error: {error}</div>}

      <section className="tables">
        <SurfacePointTable rows={surfacePoints} onChange={setSurfacePoints} />
        <OrientationTable rows={orientations} onChange={setOrientations} />
      </section>

      <section className="viewer">
        {mesh && (
          <Viewer
            mesh={mesh}
            surfacePoints={surfacePoints}
            orientations={orientations}
          />
        )}
      </section>
    </div>
  );
}

function fmt(v: number): string {
  if (!Number.isFinite(v)) return "—";
  return v.toExponential(2);
}
