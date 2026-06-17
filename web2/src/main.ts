// Two-wasm scale benchmark. Fits the generated scenario (3 sediment contacts + erosional
// basalt + late fault, sampled by 3000 boreholes) as a WasmStack over the kernel module,
// then times fit and octree meshing at several depths, reporting triangles, vertices,
// wall-clock, and cross-boundary gf_eval counts.

import init, { WasmStack } from "./wasm/geofield.js";
import { initKernel, setEngineMemory, evalCount, evalCountReset } from "../../web/src/kernel-bridge";
import kernelWasmUrl from "../public/geofield_kernel.wasm?url";
import { createViewer, type RenderMesh, type DataPoint, type Viewer } from "./viewer";

interface SeriesJson {
  name: string;
  relation: number; // 0=Erode 1=Onlap 2=Conformable 3=Basement
  faulted: boolean;
  sp_xyz: number[];
  sp_if: number[];
  ori_xyz: number[];
  ori_pole: number[];
}
interface Scenario {
  extent: [number, number, number, number, number, number];
  fault: { point: number[]; normal: number[]; strike_deg: number; dip_deg: number; offset_ft: number };
  interface_names: Record<string, string>;
  series: SeriesJson[];
  stats: Record<string, number>;
}

const NAN = Number.NaN;
const $ = (id: string) => document.getElementById(id)!;
const status = (s: string) => { $("status").textContent = s; };
const now = () => performance.now();

let ready: Promise<void> | null = null;
let viewer: Viewer | null = null;
function initWasm(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      await initKernel(kernelWasmUrl);
      const wasm = await init();
      setEngineMemory(wasm.memory as WebAssembly.Memory);
    })();
  }
  return ready;
}

function f64(a: number[]): Float64Array {
  return new Float64Array(a);
}

/** Build the WasmStack from the scenario, youngest series first. `decimateFrac` > 0
 *  enables the kernel's residual-guided point decimation (re-add threshold = that
 *  fraction of the field's isovalue range); NaN = keep all points (full solve). Returns
 *  the stack and the wall-ms spent in each series' fit (the cross-boundary kernel solve). */
function buildStack(scn: Scenario, decimateFrac: number): { stack: WasmStack; fitMs: number[] } {
  const stack = new WasmStack();
  const fitMs: number[] = [];
  for (const s of scn.series) {
    const fp = s.faulted ? scn.fault.point : [0, 0, 0];
    const fn = s.faulted ? scn.fault.normal : [0, 0, 0];
    const t0 = now();
    stack.add_series(
      f64(s.sp_xyz), f64(s.sp_if), f64(s.ori_xyz), f64(s.ori_pole),
      NAN, NAN, NAN, NAN, 1, NAN,
      s.relation,
      decimateFrac,
      f64(fp), f64(fn),
    );
    fitMs.push(now() - t0);
  }
  return { stack, fitMs };
}

interface DepthRow {
  depth: number;
  meshMs: number;
  tris: number;
  verts: number;
  evals: number;
}

/** Mesh every interface of every series at one octree depth; return aggregate timing,
 *  triangle/vertex totals, and boundary-crossing eval count. */
function meshAtDepth(stack: WasmStack, scn: Scenario, depth: number): DepthRow {
  const [x0, x1, y0, y1, z0, z1] = scn.extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  const QEF = 1e-6;

  evalCountReset();
  let tris = 0, verts = 0;
  const t0 = now();
  for (let si = 0; si < stack.series_count; si++) {
    const nIface = stack.interface_count(si);
    for (let k = 0; k < nIface; k++) {
      const m = stack.mesh_interface_octree(si, k, x0, y0, z0, size, depth, minDepth, QEF, false);
      tris += m.triangle_count;
      verts += m.vertex_count;
      m.free();
    }
  }
  const meshMs = now() - t0;
  const evals = evalCount();
  return { depth, meshMs, tris, verts, evals };
}

/** The distinct interface ids of a series, ASCENDING — matching the order
 *  `mesh_interface_octree(si, k)` meshes them (ascending isovalue ≈ ascending id here,
 *  since deeper sediments have lower ids). Derived from the scenario's surface points. */
function seriesInterfaceIds(s: SeriesJson): number[] {
  return [...new Set(s.sp_if)].sort((a, b) => a - b);
}

/** Mesh every interface of every series at one depth and KEEP the geometry (for the 3D
 *  viewer). Each surface is tagged with its interface id so it gets the right color. */
function collectSurfaces(stack: WasmStack, scn: Scenario, depth: number): RenderMesh[] {
  const [x0, x1, y0, y1, z0, z1] = scn.extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  const out: RenderMesh[] = [];
  for (let si = 0; si < stack.series_count; si++) {
    const ids = seriesInterfaceIds(scn.series[si]);
    const nIface = stack.interface_count(si);
    for (let k = 0; k < nIface; k++) {
      const m = stack.mesh_interface_octree(si, k, x0, y0, z0, size, depth, minDepth, 1e-6, false);
      if (m.triangle_count > 0) {
        out.push({
          interfaceId: ids[k] ?? ids[0] ?? 0,
          positions: m.positions(),
          normals: m.normals(),
          indices: m.indices(),
        });
      }
      m.free();
    }
  }
  return out;
}

/** All borehole surface points across series, for the data-point cloud. */
function collectPoints(scn: Scenario): DataPoint[] {
  const pts: DataPoint[] = [];
  for (const s of scn.series) {
    for (let i = 0; i < s.sp_if.length; i++) {
      pts.push({
        x: s.sp_xyz[3 * i],
        y: s.sp_xyz[3 * i + 1],
        z: s.sp_xyz[3 * i + 2],
        interfaceId: s.sp_if[i],
      });
    }
  }
  return pts;
}

function table(headers: string[], rows: (string | number)[][]): string {
  const th = headers.map((h) => `<th>${h}</th>`).join("");
  const trs = rows
    .map((r) => "<tr>" + r.map((c, i) => `<td${i > 0 ? ' class="num"' : ""}>${c}</td>`).join("") + "</tr>")
    .join("");
  return `<table><thead><tr>${th}</tr></thead><tbody>${trs}</tbody></table>`;
}

async function run() {
  $("run").setAttribute("disabled", "true");
  status("loading wasm…");
  await initWasm();

  status("loading scenario…");
  const scn: Scenario = await (await fetch("/scenario.json")).json();

  const totalSp = scn.series.reduce((a, s) => a + s.sp_if.length, 0);

  // Fit FULL (no decimation) for the baseline, then DECIMATED for the speedup. The
  // decimated stack is the one we mesh (it's what an app would ship).
  status(`fitting FULL (${scn.series.length} series, ${totalSp} surface points)…`);
  const tF = now();
  const full = buildStack(scn, NAN);
  const fullFit = now() - tF;
  await new Promise((r) => setTimeout(r, 0));

  const DECIMATE_FRAC = 0.2; // re-add threshold = 20% of the field's isovalue range
  status(`fitting DECIMATED (frac ${DECIMATE_FRAC})…`);
  const tD = now();
  const { stack, fitMs } = buildStack(scn, DECIMATE_FRAC);
  const decFit = now() - tD;
  const totalFit = decFit;

  // Depth sweep.
  const depths = [4, 5, 6, 7];
  const rows: DepthRow[] = [];
  for (const d of depths) {
    status(`meshing at octree depth ${d}…`);
    // Two passes: discard the first (warm caches), keep the second for steady-state timing.
    meshAtDepth(stack, scn, d);
    rows.push(meshAtDepth(stack, scn, d));
    // Yield to the event loop so the status text repaints.
    await new Promise((r) => setTimeout(r, 0));
  }

  // Render.
  let html = "";
  html += `<h2>Scenario</h2>`;
  html += `<pre>extent ${scn.extent.map((v) => Math.round(v)).join(" × ").replace(/ × /g, ", ")} ft
fault: strike ${scn.fault.strike_deg}° dip ${scn.fault.dip_deg}° offset ${scn.fault.offset_ft} ft (down-to-east)
${JSON.stringify(scn.stats)}</pre>`;

  html += `<h2>Fit (cross-boundary cokriging solve, per series)</h2>`;
  html += table(
    ["series", "points", "full fit ms", "decimated fit ms"],
    scn.series.map((s, i) => [
      s.name, s.sp_if.length, full.fitMs[i].toFixed(1), fitMs[i].toFixed(1),
    ]),
  );
  const speedup = decFit > 0 ? (fullFit / decFit) : 0;
  html += `<p>total fit: full <span class="num">${fullFit.toFixed(1)} ms</span>
    → decimated (frac ${DECIMATE_FRAC}) <span class="num">${decFit.toFixed(1)} ms</span>
    = <span class="num">${speedup.toFixed(1)}× faster</span>.
    Decimation re-adds only points the coarse field mis-predicts by &gt; ${DECIMATE_FRAC}·(isovalue range),
    so the geology is honored to that tolerance.</p>`;

  html += `<h2>Mesh — octree depth sweep (all series × interfaces)</h2>`;
  html += table(
    ["depth", "mesh ms", "triangles", "vertices", "gf_eval calls", "ns / eval"],
    rows.map((r) => [
      r.depth,
      r.meshMs.toFixed(1),
      r.tris.toLocaleString(),
      r.verts.toLocaleString(),
      r.evals.toLocaleString(),
      r.evals ? ((r.meshMs * 1e6) / r.evals).toFixed(0) : "—",
    ]),
  );
  html += `<p>gf_eval = per-sample calls crossing the wasm module boundary (engine→kernel).
    "ns / eval" is the amortized boundary-crossing + field-evaluation cost.</p>`;

  $("out").innerHTML = html;

  // Render the DECIMATED model: data points + contact surfaces (meshed once at a
  // readable depth). This is the same stack the table reports timings for.
  status("rendering scene…");
  await new Promise((r) => setTimeout(r, 0));
  const VIEW_DEPTH = 5;
  const surfaces: RenderMesh[] = collectSurfaces(stack, scn, VIEW_DEPTH);
  const points: DataPoint[] = collectPoints(scn);
  if (!viewer) viewer = createViewer($("scene"), scn.extent);
  viewer.setSurfaces(surfaces);
  viewer.setPoints(points);
  $("togglePts").removeAttribute("disabled");

  status(`done. ${points.length} data points, ${surfaces.length} surfaces (octree depth ${VIEW_DEPTH}). Drag to orbit; "Toggle data points" to see surfaces alone.`);
  $("run").removeAttribute("disabled");

  // Also dump to console for copy/paste.
  console.table(rows);
  console.log("fit ms", fitMs, "total", totalFit);
}

$("run").addEventListener("click", () => { run().catch((e) => { status("ERROR: " + e); console.error(e); }); });
$("togglePts").addEventListener("click", () => { viewer?.togglePoints(); });
