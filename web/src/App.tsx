import { useEffect, useRef, useState } from "react";
import {
  SEED_SERIES,
  SEED_SURFACE_POINTS,
  SEED_ORIENTATIONS,
  SurfacePointRow,
  OrientationRow,
  SeriesDef,
  EXTENT,
} from "./data";
import {
  solveAndMesh,
  remesh,
  MeshData,
  RenderMode,
} from "./model";
import { SurfacePointTable, OrientationTable } from "./Tables";
import { SeriesPanel, newSeries } from "./SeriesPanel";
import { Viewer } from "./Viewer";
import { ObjectTree } from "./ObjectTree";

const DEBOUNCE_MS = 300;

export default function App() {
  const [series, setSeries] = useState<SeriesDef[]>(SEED_SERIES);
  const [activeSeries, setActiveSeries] = useState(0);
  const [surfacePoints, setSurfacePoints] = useState<SurfacePointRow[]>(
    SEED_SURFACE_POINTS,
  );
  const [orientations, setOrientations] =
    useState<OrientationRow[]>(SEED_ORIENTATIONS);
  const [maxDepth, setMaxDepth] = useState(5);
  // "contact" → open contact sheets; "solid" → watertight closed unit slabs.
  const [renderMode, setRenderMode] = useState<RenderMode>("contact");
  const [mesh, setMesh] = useState<MeshData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [busyMs, setBusyMs] = useState<number | null>(null);
  const [solving, setSolving] = useState(false);
  // Surface keys hidden from the 3-D view (toggled in the object tree). Kept across
  // re-solves so hiding a block stays hidden as you drag sliders.
  const [hidden, setHidden] = useState<Set<string>>(new Set());

  const setHide = (keys: string[], hide: boolean) =>
    setHidden((prev) => {
      const next = new Set(prev);
      for (const k of keys) {
        if (hide) next.add(k);
        else next.delete(k);
      }
      return next;
    });

  // --- series add/remove: keep the row tables' `series` indices in sync. -------------
  const addSeries = () => {
    // New series is the youngest (index 0): shift every existing row up by one.
    const next = [newSeries(`series ${series.length + 1}`), ...series];
    setSeries(next);
    setSurfacePoints((rows) => rows.map((r) => ({ ...r, series: r.series + 1 })));
    setOrientations((rows) => rows.map((r) => ({ ...r, series: r.series + 1 })));
    setActiveSeries(0);
  };

  const removeSeries = (idx: number) => {
    if (series.length <= 1) return;
    setSeries(series.filter((_, i) => i !== idx));
    // Drop the removed series' rows; renumber the rest (anything above idx shifts down).
    const fix = <T extends { series: number }>(rows: T[]) =>
      rows
        .filter((r) => r.series !== idx)
        .map((r) => (r.series > idx ? { ...r, series: r.series - 1 } : r));
    setSurfacePoints(fix);
    setOrientations(fix);
    setActiveSeries((a) => (a >= series.length - 1 ? series.length - 2 : a));
  };

  // --- the solve: debounced async fit+mesh in the worker. ----------------------------
  // A change to inputs/series/depth REFITS; a change to renderMode alone REMESHES (instant,
  // no refit). We diff on a ref to know which path to take.
  const lastSolveKey = useRef<string>("");
  const timer = useRef<number | null>(null);

  useEffect(() => {
    if (timer.current) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      // Key the "needs refit" inputs; renderMode is intentionally excluded.
      const fitKey = JSON.stringify({ series, surfacePoints, orientations, maxDepth });
      const renderOnly = fitKey === lastSolveKey.current;
      lastSolveKey.current = fitKey;

      const t0 = performance.now();
      setSolving(true);
      const driver = renderOnly ? remesh : solveAndMesh;
      driver(
        series,
        surfacePoints,
        orientations,
        EXTENT,
        maxDepth,
        renderMode,
      ).then(({ mesh, error, warning, fitMs }) => {
        setMesh(mesh);
        setError(error);
        setWarning(warning);
        setBusyMs(renderOnly ? performance.now() - t0 : fitMs);
        setSolving(false);
      });
    }, DEBOUNCE_MS);
    return () => {
      if (timer.current) window.clearTimeout(timer.current);
    };
  }, [series, surfacePoints, orientations, maxDepth, renderMode]);

  const anyFaulted = series.some((s) => s.fault != null);

  return (
    <div className="app">
      <header>
        <h1>geofield-rs · interactive geomodel</h1>
        <div className="status">
          {solving && <span>solving…</span>}
          {mesh && (
            <>
              <span>
                {mesh.surfaces.length} surface
                {mesh.surfaces.length === 1 ? "" : "s"}
              </span>
              <span>{mesh.triangleCount.toLocaleString()} tris</span>
              <span>
                {series.length} series
              </span>
              {anyFaulted && (
                <span className="faulted">
                  faulted · {mesh.blockCount} block
                  {mesh.blockCount === 1 ? "" : "s"}
                </span>
              )}
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
          <button
            className="mode-toggle"
            onClick={() =>
              setRenderMode((m) => (m === "contact" ? "solid" : "contact"))
            }
          >
            {renderMode === "solid"
              ? "Surfaces: closed solids"
              : "Surfaces: contacts"}
          </button>
        </div>
      </header>

      {error && <div className="error">solve error: {error}</div>}
      {!error && warning && <div className="warning">note: {warning}</div>}

      <section className="tables">
        <SeriesPanel
          series={series}
          activeSeries={activeSeries}
          onSelect={setActiveSeries}
          onChange={setSeries}
          onAdd={addSeries}
          onRemove={removeSeries}
        />
        <SurfacePointTable
          rows={surfacePoints}
          series={series}
          activeSeries={activeSeries}
          onChange={setSurfacePoints}
        />
        <OrientationTable
          rows={orientations}
          series={series}
          activeSeries={activeSeries}
          onChange={setOrientations}
        />
      </section>

      <section className="viewer">
        {mesh && (
          <ObjectTree
            surfaces={mesh.surfaces}
            series={series}
            hidden={hidden}
            onToggle={(k, hide) => setHide([k], hide)}
            onToggleMany={(keys, hide) => setHide(keys, hide)}
          />
        )}
        <div className="viewer-canvas">
          {mesh && (
            <Viewer
              mesh={mesh}
              hidden={hidden}
              surfacePoints={surfacePoints}
              orientations={orientations}
              series={series}
            />
          )}
        </div>
      </section>
    </div>
  );
}
