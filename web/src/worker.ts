// The editor's solver worker: owns BOTH wasm instances (the EUPL kernel + the BSD engine)
// and the kernel-bridge, fits the stack + meshes off the UI thread, and STREAMS status +
// surfaces back so the page never freezes. Adapted from web2/src/worker.ts, scoped to the
// interactive editor: it fits whatever series the user has built, then streams either open
// contact sheets or closed unit solids.
//
// Cancellation is by hard terminate from the main thread (a single fit is one
// uninterruptible wasm call), so this worker has no cancel logic. A new edit terminates the
// previous worker and spawns a fresh one — which also re-runs initWasm (an accepted ~1s
// cold start; editor fits are otherwise milliseconds).
//
// Meshed surfaces are posted as they are produced and TRANSFERRED (zero-copy) — the
// typed-array buffers move to the main thread, not copied.

import init, { WasmStack } from "./wasm/geofield.js";
import { initKernel, setEngineMemory } from "./kernel-bridge";
import kernelWasmUrl from "./wasm-kernel/geofield_kernel.wasm?url";
import type {
  Command,
  RenderMode,
  RunCommand,
  SeriesJson,
  SurfaceEvent,
  WorkerEvent,
} from "./protocol";

const NAN = Number.NaN;
const now = () => performance.now();
const QEF_EPS = 1e-6;

/** The most recently fitted stack + the run that produced it, kept so a "remesh" command can
 *  re-mesh in a new render mode WITHOUT re-fitting. */
let fitted: { stack: WasmStack; run: RunCommand } | null = null;

/** Post a typed event. `transfer` lists ArrayBuffers to move zero-copy. */
function emit(ev: WorkerEvent, transfer: Transferable[] = []) {
  (self as unknown as Worker).postMessage(ev, transfer);
}

// Init the two wasm modules ONCE per worker lifetime, kernel-first (the engine's "kernel"
// imports must be live before it runs), then register engine memory for the bridge.
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

/** Fit the whole stack youngest-first. Editor inputs are tiny, so no decimation
 *  (decimate_frac = NaN → keep all points, gempy-exact). NaN kernel options → crate
 *  defaults; uni_degree = 1. Throws (caught by the caller) on a degenerate fit. */
function buildStack(series: SeriesJson[]): { stack: WasmStack; ms: number } {
  const stack = new WasmStack();
  const t0 = now();
  for (const s of series) {
    stack.add_series(
      f64(s.sp_xyz),
      f64(s.sp_if),
      f64(s.ori_xyz),
      f64(s.ori_pole),
      NAN, NAN, NAN, NAN, 1, NAN, // range, c_o, i_res, gi_res, uni_degree, nugget
      s.relation,
      NAN, // decimate_frac: off
      f64(s.fault_point),
      f64(s.fault_normal),
    );
  }
  return { stack, ms: now() - t0 };
}

/** Mesh + STREAM the viewer surfaces (transferred), per FAULT BLOCK — each block's surface
 *  is a separate piece cut flush at the fault, not one surface stretched across the offset.
 *  A non-faulted series is a single block (the whole domain). Returns the surface count and
 *  the total block count summed across series.
 *
 *  `mode` picks the representation:
 *   - "contact": one OPEN contact sheet per interface (cap=false). An eroded contact ends
 *     short of the younger erosion surface (the cut edge is a dropped cap face).
 *   - "solid": one watertight CLOSED slab per present lithology unit, capped by box + fault
 *     + younger erosions — so an eroded unit DAYLIGHTS against the erosion surface. */
function streamSurfaces(
  stack: WasmStack,
  series: SeriesJson[],
  extent: [number, number, number, number, number, number],
  depth: number,
  mode: RenderMode,
): { count: number; totalBlocks: number } {
  const [x0, x1, y0, y1, z0, z1] = extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  let count = 0;
  let totalBlocks = 0;

  for (let si = 0; si < stack.series_count; si++) {
    const ids = seriesInterfaceIds(series[si]); // ascending interface ids in this series
    const formationOf = (interfaceIdx: number) =>
      series[si].name + (ids.length > 1 ? ` · i${interfaceIdx}` : "");
    const relation = series[si].relation;
    const blocks = Math.max(1, stack.series_block_count(si));
    totalBlocks += blocks;

    for (let b = 0; b < blocks; b++) {
      if (mode === "solid") {
        // One closed solid per PRESENT lithology unit (the rock-bearing units below this
        // series' bounding contact — the topmost void unit is owned by a younger series and
        // not rendered). Unit u sits above interface u-1.
        const units = stack.present_unit_count(si, b);
        for (let u = 0; u < units; u++) {
          const m = stack.mesh_series_block_unit_solid(
            si, b, u, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS,
          );
          if (m.triangle_count > 0) {
            postSurface(m, {
              kind: "solid", seriesIndex: si, relation, block: b,
              interfaceIndex: u > 0 ? u - 1 : -1,
              formation: u > 0 ? formationOf(u - 1) : `${series[si].name} · base`,
              unit: u,
            });
            count++;
          }
          m.free();
        }
      } else {
        // One open contact sheet per interface.
        const n = stack.interface_count(si);
        for (let k = 0; k < n; k++) {
          const m = stack.mesh_series_block_interface(
            si, b, k, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS, false,
          );
          if (m.triangle_count > 0) {
            postSurface(m, {
              kind: "contact", seriesIndex: si, relation, block: b,
              interfaceIndex: k, formation: formationOf(k), unit: -1,
            });
            count++;
          }
          m.free();
        }
      }
    }
  }
  return { count, totalBlocks };
}

/** Read a meshed surface's buffers and post a transferred SurfaceEvent. */
function postSurface(
  m: { positions(): Float32Array; normals(): Float32Array; indices(): Uint32Array; triangle_count: number },
  meta: Omit<SurfaceEvent, "type" | "positions" | "normals" | "indices" | "triangleCount">,
) {
  const positions = m.positions();
  const normals = m.normals();
  const indices = m.indices();
  const ev: SurfaceEvent = {
    type: "surface", ...meta,
    positions, normals, indices, triangleCount: m.triangle_count,
  };
  emit(ev, [positions.buffer, normals.buffer, indices.buffer]);
}

/** Distinct interface ids of a series, ascending — the order the meshers iterate. */
function seriesInterfaceIds(s: SeriesJson): number[] {
  return [...new Set(s.sp_if)].sort((a, b) => a - b);
}

async function run(cmd: RunCommand) {
  emit({ type: "phase", phase: "loading-wasm", label: "loading kernel + engine wasm…" });
  await initWasm();

  emit({ type: "phase", phase: "fitting", label: "fitting stack…" });
  const { stack, ms } = buildStack(cmd.series);
  fitted = { stack, run: cmd };

  emit({ type: "phase", phase: "meshing", label: "meshing surfaces…" });
  emit({ type: "surfaces-cleared" });
  const { count, totalBlocks } = streamSurfaces(
    stack, cmd.series, cmd.extent, cmd.maxDepth, cmd.renderMode,
  );

  // A faulted fit where the plane stranded a block (no orientation on one side) can mesh
  // nothing — flag it so an empty result doesn't look like a silent break.
  const anyFault = cmd.series.some((s) => s.fault_normal.some((v) => v !== 0));
  const warning =
    count === 0 && anyFault
      ? "no meshable surface — a fault plane may strand all of a series' data on one side " +
        "(each side needs at least one orientation). Adjust strike/offset."
      : null;

  emit({ type: "phase", phase: "done", label: "done." });
  emit({ type: "done", fitMs: ms, surfaceCount: count, totalBlocks, warning });
}

/** Re-mesh the already-fitted stack in a new render mode, without re-fitting. */
function remesh(cmd: Extract<Command, { type: "remesh" }>) {
  if (!fitted) return; // nothing fitted yet — the UI will have triggered a full run instead
  const { stack, run: prev } = fitted;
  emit({ type: "phase", phase: "meshing", label: `re-meshing ${cmd.renderMode}…` });
  emit({ type: "surfaces-cleared" });
  const { count, totalBlocks } = streamSurfaces(
    stack, prev.series, prev.extent, cmd.maxDepth, cmd.renderMode,
  );
  emit({ type: "phase", phase: "done", label: "done." });
  emit({ type: "done", fitMs: 0, surfaceCount: count, totalBlocks, warning: null });
}

self.onmessage = (e: MessageEvent<Command>) => {
  const cmd = e.data;
  try {
    if (cmd.type === "run") {
      run(cmd).catch((err) => {
        emit({ type: "error", message: err instanceof Error ? err.message : String(err) });
      });
    } else if (cmd.type === "remesh") {
      remesh(cmd);
    }
  } catch (err) {
    emit({ type: "error", message: err instanceof Error ? err.message : String(err) });
  }
};
