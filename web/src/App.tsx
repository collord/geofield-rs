import { useEffect, useRef, useState } from "react";
import {
  SEED_SURFACE_POINTS,
  SEED_ORIENTATIONS,
  SurfacePointRow,
  OrientationRow,
} from "./data";
import { initWasm, solveAndMesh, MeshData, FaultConfig } from "./model";
import { SurfacePointTable, OrientationTable } from "./Tables";
import { Viewer } from "./Viewer";
import { EXTENT } from "./data";

const DEBOUNCE_MS = 300;

// Domain center, for anchoring the fault plane.
const CENTER: [number, number, number] = [
  (EXTENT[0] + EXTENT[1]) / 2,
  (EXTENT[2] + EXTENT[3]) / 2,
  (EXTENT[4] + EXTENT[5]) / 2,
];
const SPAN = Math.max(
  EXTENT[1] - EXTENT[0],
  EXTENT[3] - EXTENT[2],
  EXTENT[5] - EXTENT[4],
);

/** A vertical fault plane from a map strike azimuth (deg from +Y/North) and a
 *  signed offset of the plane along its horizontal normal from the domain center.
 *  Normal = (cos az, −sin az, 0); the plane point = center + offset · normal. */
function faultFromControls(
  enabled: boolean,
  azimuthDeg: number,
  offset: number,
): FaultConfig {
  const az = (azimuthDeg * Math.PI) / 180;
  const normal: [number, number, number] = [Math.cos(az), -Math.sin(az), 0];
  const point: [number, number, number] = [
    CENTER[0] + offset * normal[0],
    CENTER[1] + offset * normal[1],
    CENTER[2],
  ];
  return { enabled, point, normal };
}

export default function App() {
  const [wasmReady, setWasmReady] = useState(false);
  const [surfacePoints, setSurfacePoints] = useState<SurfacePointRow[]>(
    SEED_SURFACE_POINTS,
  );
  const [orientations, setOrientations] =
    useState<OrientationRow[]>(SEED_ORIENTATIONS);
  const [maxDepth, setMaxDepth] = useState(5);
  // Off → bounded open contact sheet; on → watertight closed solid (sheet +
  // domain-boundary cap).
  const [cap, setCap] = useState(false);
  // Optional single planar fault: the field solves independently on each side, so
  // contacts are laterally offset across the plane (a finite fault).
  const [faultEnabled, setFaultEnabled] = useState(false);
  const [faultAzimuth, setFaultAzimuth] = useState(0); // strike, deg from +Y
  const [faultOffset, setFaultOffset] = useState(0); // along normal from center
  const [mesh, setMesh] = useState<MeshData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
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
      const fault = faultFromControls(faultEnabled, faultAzimuth, faultOffset);
      const { mesh, error, warning } = solveAndMesh(
        surfacePoints,
        orientations,
        maxDepth,
        cap,
        fault,
      );
      setMesh(mesh);
      setError(error);
      setWarning(warning);
      setBusyMs(performance.now() - t0);
    }, DEBOUNCE_MS);
    return () => {
      if (timer.current) window.clearTimeout(timer.current);
    };
  }, [
    wasmReady,
    surfacePoints,
    orientations,
    maxDepth,
    cap,
    faultEnabled,
    faultAzimuth,
    faultOffset,
  ]);

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
              {faultEnabled && (
                <span className="faulted">
                  faulted · {mesh.blockCount} block
                  {mesh.blockCount === 1 ? "" : "s"}
                </span>
              )}
              <span>residual {fmt(mesh.residual)}</span>
              <span>iso-spread {fmt(mesh.isovalueSpread)}</span>
              {busyMs != null && <span>{busyMs.toFixed(0)} ms</span>}
            </>
          )}
          <label className="res">
            octree depth
            <input
              type="range"
              min={2}
              max={7}
              step={1}
              value={maxDepth}
              onChange={(e) => setMaxDepth(Number(e.target.value))}
            />
            {maxDepth}
          </label>
          <label className="res">
            <input
              type="checkbox"
              checked={cap}
              onChange={(e) => setCap(e.target.checked)}
            />
            close domain (cap)
          </label>
          <label className="res">
            <input
              type="checkbox"
              checked={faultEnabled}
              onChange={(e) => setFaultEnabled(e.target.checked)}
            />
            fault
          </label>
          {faultEnabled && (
            <>
              <label className="res">
                strike
                <input
                  type="range"
                  min={0}
                  max={180}
                  step={5}
                  value={faultAzimuth}
                  onChange={(e) => setFaultAzimuth(Number(e.target.value))}
                />
                {faultAzimuth}°
              </label>
              <label className="res">
                offset
                <input
                  type="range"
                  min={-SPAN / 2}
                  max={SPAN / 2}
                  step={SPAN / 40}
                  value={faultOffset}
                  onChange={(e) => setFaultOffset(Number(e.target.value))}
                />
                {faultOffset.toFixed(0)}
              </label>
            </>
          )}
        </div>
      </header>

      {error && <div className="error">solve error: {error}</div>}
      {!error && warning && <div className="warning">note: {warning}</div>}

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
