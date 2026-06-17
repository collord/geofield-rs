// UI thread. Spawns the solver worker, sends it the scenario, and renders the streamed
// status + results live: a progress bar, per-series fit timings as they land, the
// contact surfaces appearing one-by-one as the worker meshes them, the depth-sweep table,
// and a running gf_eval ticker. The heavy wasm work runs in the worker, so this thread
// stays responsive (progress animates, Cancel works instantly).

import { createViewer, type RenderMesh, type DataPoint, type Viewer } from "./viewer";
import type { Command, Scenario, WorkerEvent } from "./protocol";

const $ = (id: string) => document.getElementById(id)!;
const setStatus = (s: string) => { $("status").textContent = s; };
const setBar = (p: number) => { ($("bar") as HTMLProgressElement).value = Math.max(0, Math.min(1, p)); };

let viewer: Viewer | null = null;
let worker: Worker | null = null;

const DECIMATE_FRAC = 0.2;
const DEPTHS = [4, 5, 6]; // depth 7 is the long pole; 4–6 already shows the scaling
const VIEW_DEPTH = 5;

interface Accum {
  scn: Scenario;
  fullFit: { name: string; points: number; ms: number }[];
  decFit: { name: string; points: number; ms: number }[];
  depthRows: { depth: number; meshMs: number; tris: number; verts: number; evals: number }[];
  surfaces: number;
  evalLive: number;
}

function renderTables(a: Accum, done?: { fullFitMs: number | null; decimatedFitMs: number }) {
  let html = "";
  html += `<h2>Scenario</h2>`;
  html += `<pre>extent ${a.scn.extent.map((v) => Math.round(v)).join(", ")} ft
fault: strike ${a.scn.fault.strike_deg}° dip ${a.scn.fault.dip_deg}° offset ${a.scn.fault.offset_ft} ft (down-to-east)
${JSON.stringify(a.scn.stats)}</pre>`;

  if (a.decFit.length) {
    html += `<h2>Fit (cross-boundary cokriging solve, per series — streamed as each lands)</h2>`;
    const rows = a.scn.series.map((s, i) => [
      s.name,
      s.sp_if.length,
      a.fullFit[i] ? a.fullFit[i].ms.toFixed(1) : "—",
      a.decFit[i] ? a.decFit[i].ms.toFixed(1) : "…",
    ]);
    html += table(["series", "points", "full fit ms", "decimated fit ms"], rows);
    if (done) {
      const sp = done.fullFitMs ? (done.fullFitMs / done.decimatedFitMs) : 0;
      html += `<p>total fit: ${done.fullFitMs ? `full <span class="num">${done.fullFitMs.toFixed(1)} ms</span> → ` : ""}decimated
        <span class="num">${done.decimatedFitMs.toFixed(1)} ms</span>${
        done.fullFitMs ? ` = <span class="num">${sp.toFixed(1)}× faster</span>` : ""}.</p>`;
    }
  }

  if (a.depthRows.length) {
    html += `<h2>Mesh — octree depth sweep</h2>`;
    html += table(
      ["depth", "mesh ms", "triangles", "vertices", "gf_eval calls", "ns / eval"],
      a.depthRows.map((r) => [
        r.depth, r.meshMs.toFixed(1), r.tris.toLocaleString(), r.verts.toLocaleString(),
        r.evals.toLocaleString(), r.evals ? ((r.meshMs * 1e6) / r.evals).toFixed(0) : "—",
      ]),
    );
  }
  $("out").innerHTML = html;
}

function table(headers: string[], rows: (string | number)[][]): string {
  const th = headers.map((h) => `<th>${h}</th>`).join("");
  const trs = rows
    .map((r) => "<tr>" + r.map((c, i) => `<td${i > 0 ? ' class="num"' : ""}>${c}</td>`).join("") + "</tr>")
    .join("");
  return `<table><thead><tr>${th}</tr></thead><tbody>${trs}</tbody></table>`;
}

function startRun(scn: Scenario) {
  // Fresh worker each run (hard-cancel terminates the previous one).
  worker?.terminate();
  worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });

  if (!viewer) viewer = createViewer($("scene"), scn.extent);
  viewer.clearSurfaces();
  // Show the data points immediately (we have them without the worker).
  viewer.setPoints(collectPoints(scn));
  $("togglePts").removeAttribute("disabled");

  const a: Accum = {
    scn, fullFit: [], decFit: [], depthRows: [], surfaces: 0, evalLive: 0,
  };

  worker.onmessage = (e: MessageEvent<WorkerEvent>) => {
    const ev = e.data;
    switch (ev.type) {
      case "phase":
        setStatus(ev.label);
        setBar(ev.progress);
        break;
      case "fit-done":
        (ev.pass === "full" ? a.fullFit : a.decFit)[ev.seriesIndex] = {
          name: ev.seriesName, points: ev.points, ms: ev.ms,
        };
        renderTables(a);
        break;
      case "surface":
        viewer!.addSurface({
          interfaceId: ev.interfaceId,
          positions: ev.positions,
          normals: ev.normals,
          indices: ev.indices,
        } as RenderMesh);
        a.surfaces++;
        break;
      case "depth-row":
        a.depthRows.push(ev);
        renderTables(a);
        break;
      case "eval-tick":
        if (ev.count > 0) { a.evalLive = ev.count; }
        break;
      case "done":
        renderTables(a, { fullFitMs: ev.fullFitMs, decimatedFitMs: ev.decimatedFitMs });
        setStatus(`done. ${ev.dataPointCount.toLocaleString()} data points, ${ev.surfaceCount} surfaces (depth ${VIEW_DEPTH}). Drag to orbit; "Toggle data points" for surfaces alone.`);
        setBar(1);
        finishRun();
        break;
      case "error":
        setStatus("ERROR: " + ev.message);
        finishRun();
        break;
    }
  };

  worker.onerror = (e) => { setStatus("WORKER ERROR: " + e.message); finishRun(); };

  const cmd: Command = {
    type: "run", scenario: scn, decimateFrac: DECIMATE_FRAC,
    fullBaseline: true, depths: DEPTHS, viewDepth: VIEW_DEPTH,
  };
  worker.postMessage(cmd);

  $("run").setAttribute("disabled", "true");
  $("cancel").removeAttribute("disabled");
}

function finishRun() {
  $("run").removeAttribute("disabled");
  $("cancel").setAttribute("disabled", "true");
}

function cancelRun() {
  worker?.terminate();
  worker = null;
  setStatus("cancelled.");
  setBar(0);
  finishRun();
}

/** All borehole surface points across series, for the data-point cloud. */
function collectPoints(scn: Scenario): DataPoint[] {
  const pts: DataPoint[] = [];
  for (const s of scn.series) {
    for (let i = 0; i < s.sp_if.length; i++) {
      pts.push({
        x: s.sp_xyz[3 * i], y: s.sp_xyz[3 * i + 1], z: s.sp_xyz[3 * i + 2],
        interfaceId: s.sp_if[i],
      });
    }
  }
  return pts;
}

async function onRun() {
  setStatus("loading scenario…");
  setBar(0.01);
  const scn: Scenario = await (await fetch("/scenario.json")).json();
  startRun(scn);
}

$("run").addEventListener("click", () => { onRun().catch((e) => setStatus("ERROR: " + e)); });
$("cancel").addEventListener("click", cancelRun);
$("togglePts").addEventListener("click", () => { viewer?.togglePoints(); });
