// Two-wasm scale benchmark. Fits the generated scenario (3 sediment contacts + erosional
// basalt + late fault, sampled by 3000 boreholes) as a WasmStack over the kernel module,
// then times fit and octree meshing at several depths, reporting triangles, vertices,
// wall-clock, and cross-boundary gf_eval counts.

import init, { WasmStack } from "./wasm/geofield.js";
import { initKernel, setEngineMemory, evalCount, evalCountReset } from "../../web/src/kernel-bridge";
import kernelWasmUrl from "../public/geofield_kernel.wasm?url";

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

/** Build the WasmStack from the scenario, youngest series first. Returns the stack and
 *  the wall-ms spent in each series' fit (the cross-boundary kernel solve). */
function buildStack(scn: Scenario): { stack: WasmStack; fitMs: number[] } {
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
  status(`fitting ${scn.series.length} series (${totalSp} surface points)…`);
  const t0 = now();
  const { stack, fitMs } = buildStack(scn);
  const totalFit = now() - t0;

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
    ["series", "points", "fit ms"],
    scn.series.map((s, i) => [s.name, s.sp_if.length, fitMs[i].toFixed(1)]),
  );
  html += `<p>total fit: <span class="num">${totalFit.toFixed(1)} ms</span></p>`;

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
  status("done.");
  $("run").removeAttribute("disabled");

  // Also dump to console for copy/paste.
  console.table(rows);
  console.log("fit ms", fitMs, "total", totalFit);
}

$("run").addEventListener("click", () => { run().catch((e) => { status("ERROR: " + e); console.error(e); }); });
