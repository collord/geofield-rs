// web3 — stratigraphic-hierarchy testbed (US5). Scenario selector wiring the hierarchy
// panel + the 3D viewer + the solver worker (the two-wasm core). Selecting a scenario fits
// it and renders; editing a seam/axis in the panel re-posts a `run` (the
// mutation→derive→mesh round-trip) and refreshes both panel-derived state and the viewer.
//
// Build status: type-checked design; not yet built/served here (web3 deps uninstalled).
// The MVP completion notes record what remains to verify in a browser.

import { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { HierarchyPanel, applyCoreUpdate, emptyState, type HierarchyState } from "./HierarchyPanel";
import type { RelationDto } from "./HierarchyPanel/dto";
import "./HierarchyPanel/hierarchy.css";
import { Viewer } from "./Viewer";
import { SCENARIOS, scenarioById, type Scenario } from "./scenarios";
import type { RunCommand, SurfacePayload, WorkerMessage } from "./protocol";

function App() {
  const [scenarioId, setScenarioId] = useState<string>(SCENARIOS[0].id);
  const [state, setState] = useState<HierarchyState>(emptyState());
  const [surfaces, setSurfaces] = useState<SurfacePayload[]>([]);
  const [status, setStatus] = useState<string>("idle");
  // Extent is state (drives the viewer's axes) as well as a ref (read in runStack).
  const [extent, setExtent] = useState<Scenario["extent"]>(SCENARIOS[0].extent);
  // The live series array (editable) — initialized from the scenario, mutated by the panel.
  const seriesRef = useRef<RunCommand["series"]>([]);
  const extentRef = useRef<Scenario["extent"]>(SCENARIOS[0].extent);
  const workerRef = useRef<Worker | null>(null);

  // (Re)spawn the worker and post a run. Hard-terminate supersedes any in-flight fit.
  function runStack() {
    workerRef.current?.terminate();
    const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (e: MessageEvent<WorkerMessage>) => {
      const msg = e.data;
      if (msg.type === "status") setStatus(msg.phase);
      else if (msg.type === "error") setStatus("error: " + msg.message);
      else if (msg.type === "done") {
        setSurfaces(msg.surfaces);
        setState(applyCoreUpdate(msg.update)); // derived = read-only cache (FR-018)
        setStatus("done");
      }
    };
    const cmd: RunCommand = {
      type: "run",
      series: seriesRef.current,
      extent: extentRef.current,
      maxDepth: 6,
      renderMode: "contact",
    };
    worker.postMessage(cmd);
  }

  // Load a scenario: seed the editable series + extent, then fit.
  function loadScenario(id: string) {
    const sc = scenarioById(id);
    if (!sc) return;
    setScenarioId(id);
    seriesRef.current = sc.series.map((s) => ({ ...s }));
    extentRef.current = sc.extent;
    setExtent(sc.extent);
    runStack();
  }

  useEffect(() => {
    loadScenario(SCENARIOS[0].id);
    return () => workerRef.current?.terminate();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --- panel mutation callbacks: edit declared state, then re-run (round-trip) ---------
  // (MVP wires the two US1-relevant axes; reorder/move land with their model support.)
  function setRelation(elementId: string, rel: RelationDto) {
    const idx = Number(elementId);
    const code = rel === "erode" ? 0 : rel === "onlap" ? 1 : 2;
    const s = seriesRef.current[idx];
    if (s) {
      s.relation = code;
      runStack();
    }
  }
  function setUnboundedAbove(elementId: string, v: boolean) {
    const idx = Number(elementId);
    const s = seriesRef.current[idx];
    if (s) {
      s.unboundedAbove = v;
      runStack();
    }
  }
  const noop = () => setStatus("reorder/move land with US2/US3 model support");

  const scenario = useMemo(() => scenarioById(scenarioId), [scenarioId]);

  return (
    <div style={{ display: "grid", gridTemplateColumns: "360px 1fr", height: "100vh" }}>
      <aside style={{ padding: "1rem", overflow: "auto", borderRight: "1px solid #ccd" }}>
        <h2 style={{ margin: "0 0 0.5rem" }}>Stratigraphic hierarchy</h2>
        <select value={scenarioId} onChange={(e) => loadScenario(e.target.value)}>
          {SCENARIOS.map((s) => (
            <option key={s.id} value={s.id}>{s.name}</option>
          ))}
        </select>
        <p style={{ fontSize: "0.8em", color: "#678" }}>{scenario?.description}</p>
        <div style={{ fontSize: "0.75em", color: "#789" }}>status: {status}</div>
        <hr />
        <HierarchyPanel
          update={{ model: state.model, derived: state.derived }}
          onReorderContainers={noop}
          onReorderWithin={noop}
          onMoveSurface={noop}
          onSetRelation={setRelation}
          onSetUnboundedAbove={setUnboundedAbove}
        />
      </aside>
      <main>
        <Viewer surfaces={surfaces} extent={extent} />
      </main>
    </div>
  );
}

const root = document.getElementById("root");
if (root) createRoot(root).render(<App />);
