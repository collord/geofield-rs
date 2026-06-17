// The solver worker: owns BOTH wasm instances (the EUPL kernel + the BSD engine) and the
// kernel-bridge, runs the fit + mesh off the UI thread, and STREAMS status + results back
// so the page never freezes. Cancellation is by hard terminate from the main thread (a
// single fit is one uninterruptible wasm call), so this worker has no cancel logic.
//
// Meshed surfaces are posted as they are produced and TRANSFERRED (zero-copy) — the
// typed-array buffers move to the main thread, not copied.

import init, { WasmStack } from "./wasm/geofield.js";
import { initKernel, setEngineMemory, evalCount, evalCountReset } from "../../web/src/kernel-bridge";
import kernelWasmUrl from "../public/geofield_kernel.wasm?url";
import type {
  Command,
  Scenario,
  SeriesJson,
  WorkerEvent,
  SurfaceEvent,
} from "./protocol";

const NAN = Number.NaN;
const now = () => performance.now();

/** Post a typed event. `transfer` lists ArrayBuffers to move zero-copy. */
function emit(ev: WorkerEvent, transfer: Transferable[] = []) {
  (self as unknown as Worker).postMessage(ev, transfer);
}

// Init the two wasm modules ONCE per worker lifetime. (A hard cancel terminates the
// worker, so a fresh worker re-runs this — an accepted ~1s cold start.)
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

const f64 = (a: number[]) => new Float64Array(a);

/** Distinct interface ids of a series, ascending — the order mesh_interface_octree meshes. */
function seriesInterfaceIds(s: SeriesJson): number[] {
  return [...new Set(s.sp_if)].sort((a, b) => a - b);
}

/** Fit the whole stack youngest-first; emit a fit-done event per series. */
function buildStack(scn: Scenario, decimateFrac: number, pass: "full" | "decimated"): {
  stack: WasmStack;
  totalMs: number;
} {
  const stack = new WasmStack();
  const t0 = now();
  scn.series.forEach((s, i) => {
    const fp = s.faulted ? scn.fault.point : [0, 0, 0];
    const fn = s.faulted ? scn.fault.normal : [0, 0, 0];
    const ts = now();
    stack.add_series(
      f64(s.sp_xyz), f64(s.sp_if), f64(s.ori_xyz), f64(s.ori_pole),
      NAN, NAN, NAN, NAN, 1, NAN,
      s.relation, decimateFrac,
      f64(fp), f64(fn),
    );
    emit({
      type: "fit-done", pass, seriesIndex: i, seriesName: s.name,
      points: s.sp_if.length, ms: now() - ts,
    });
  });
  return { stack, totalMs: now() - t0 };
}

/** Mesh every interface at one depth for timing (geometry discarded). */
function meshAtDepth(stack: WasmStack, scn: Scenario, depth: number) {
  const [x0, x1, y0, y1, z0, z1] = scn.extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  evalCountReset();
  let tris = 0, verts = 0;
  const t0 = now();
  for (let si = 0; si < stack.series_count; si++) {
    const n = stack.interface_count(si);
    for (let k = 0; k < n; k++) {
      const m = stack.mesh_interface_octree(si, k, x0, y0, z0, size, depth, minDepth, 1e-6, false);
      tris += m.triangle_count;
      verts += m.vertex_count;
      m.free();
    }
  }
  return { meshMs: now() - t0, tris, verts, evals: evalCount() };
}

/** Mesh every interface at `depth` and STREAM each surface back (transferred). */
function streamSurfaces(stack: WasmStack, scn: Scenario, depth: number): number {
  const [x0, x1, y0, y1, z0, z1] = scn.extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  let count = 0;
  for (let si = 0; si < stack.series_count; si++) {
    const ids = seriesInterfaceIds(scn.series[si]);
    const n = stack.interface_count(si);
    for (let k = 0; k < n; k++) {
      const m = stack.mesh_interface_octree(si, k, x0, y0, z0, size, depth, minDepth, 1e-6, false);
      if (m.triangle_count > 0) {
        const positions = m.positions();
        const normals = m.normals();
        const indices = m.indices();
        const ev: SurfaceEvent = {
          type: "surface", seriesIndex: si, interfaceIndex: k,
          interfaceId: ids[k] ?? ids[0] ?? 0,
          positions, normals, indices,
        };
        emit(ev, [positions.buffer, normals.buffer, indices.buffer]);
        count++;
      }
      m.free();
    }
  }
  return count;
}

async function run(cmd: Extract<Command, { type: "run" }>) {
  const { scenario: scn, decimateFrac, fullBaseline, depths, viewDepth } = cmd;

  emit({ type: "phase", phase: "loading-wasm", label: "loading kernel + engine wasm…", progress: 0.02 });
  await initWasm();

  // Optional full baseline (the slow path — this is the one that proves the worker keeps
  // the UI alive). Each series fit posts a fit-done as it lands.
  let fullFitMs: number | null = null;
  if (fullBaseline) {
    emit({ type: "phase", phase: "fitting", label: "fitting FULL baseline (no decimation)…", progress: 0.05 });
    fullFitMs = buildStack(scn, NAN, "full").totalMs;
  }

  emit({ type: "phase", phase: "fitting", label: `fitting decimated (frac ${decimateFrac})…`, progress: 0.4 });
  const { stack, totalMs: decimatedFitMs } = buildStack(scn, decimateFrac, "decimated");

  // Stream the renderable surfaces FIRST, so the model APPEARS as soon as it is fit — the
  // user shouldn't wait through the (slower) timing sweep just to see the geology.
  emit({ type: "phase", phase: "rendering", label: `meshing surfaces for the viewer (depth ${viewDepth})…`, progress: 0.55 });
  const surfaceCount = streamSurfaces(stack, scn, viewDepth);

  // Then the depth sweep (timing). Warm pass discarded, second pass reported.
  for (let i = 0; i < depths.length; i++) {
    const d = depths[i];
    emit({
      type: "phase", phase: "meshing",
      label: `timing mesh at octree depth ${d}…`,
      progress: 0.7 + 0.3 * (i / depths.length),
    });
    meshAtDepth(stack, scn, d); // warm
    const r = meshAtDepth(stack, scn, d);
    emit({ type: "depth-row", depth: d, ...r });
    emit({ type: "eval-tick", count: r.evals });
  }

  const dataPointCount = scn.series.reduce((a, s) => a + s.sp_if.length, 0);
  emit({ type: "phase", phase: "done", label: "done.", progress: 1 });
  emit({ type: "done", fullFitMs, decimatedFitMs, surfaceCount, dataPointCount });
}

self.onmessage = (e: MessageEvent<Command>) => {
  const cmd = e.data;
  if (cmd.type === "run") {
    run(cmd).catch((err) => {
      emit({ type: "error", message: err instanceof Error ? err.message : String(err) });
    });
  }
};
