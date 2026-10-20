// The testbed's solver worker: owns BOTH wasm instances (EUPL kernel + BSD engine) + the
// kernel-bridge, fits the stack and meshes off the UI thread, and streams status, surfaces,
// and the hierarchy `CoreUpdate` back. Adapted from web/src/worker.ts to the two-axis model
// (feature 001): add_series carries `unboundedAbove`, and the worker posts the parsed
// `CoreUpdate` (declared state + derived view) from `WasmStack.core_update_json()`.
//
// Cancellation is by hard terminate from the main thread (a single fit is one
// uninterruptible wasm call). A new edit terminates the previous worker and spawns a fresh
// one (an accepted ~1s cold start).
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import init, { WasmStack } from "./wasm/geofield.js";
import { initKernel, setEngineMemory, setKernelErrorSink } from "./kernel-bridge";
import kernelWasmUrl from "./wasm-kernel/geofield_kernel.wasm?url";
import type {
  Command,
  DoneMessage,
  KernelName,
  RecipeOp,
  RunCommand,
  SeriesJson,
  SurfacePayload,
  WorkerMessage,
} from "./protocol";
import type { CoreUpdate } from "./HierarchyPanel/dto";

const NAN = Number.NaN;
const QEF_EPS = 1e-6;

// NOTE: the Manifold boolean engine does NOT run here. This worker is terminate-per-fit, and the
// single-threaded manifold-3d wasm hangs its boolean op when re-instantiated in a fresh worker (the
// 2nd instance in a page deadlocks in `subtract`). So in solid mode the worker only emits a boolean
// RECIPE over single-level-set primitives (`mesh_volume_recipe_octree`); the booleans run on the
// persistent main thread (`manifoldVolumes.ts` `executeRecipe`), one Manifold instance for the page.

function emit(msg: WorkerMessage, transfer: Transferable[] = []) {
  (self as unknown as Worker).postMessage(msg, transfer);
}

// Init the two wasm modules ONCE per worker lifetime, kernel-first (the engine's "kernel"
// imports must be live before it runs), then register engine memory for the bridge.
let ready: Promise<void> | null = null;
function initWasm(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      // Forward ERROR-level kernel diagnostics (fit failure, kernel panic) to the app as a
      // non-fatal warning, so a kernel-side event surfaces in the UI instead of only the console
      // (review 4.1/4.2). Registered before initKernel so the import is live on first fit.
      setKernelErrorSink((message) => emit({ type: "warning", message: `kernel: ${message}` }));
      await initKernel(kernelWasmUrl);
      const wasm = await init();
      setEngineMemory(wasm.memory as WebAssembly.Memory);
    })();
  }
  return ready;
}

const f64 = (a: number[]) => new Float64Array(a);

/** The engine's `kernel` argument: `KernelFunction::code` in geofield-kernel (the source of
 *  truth — this is only its wire encoding, like the relation/role codes below). */
const KERNEL_CODE: Record<KernelName, number> = { cubic: 0, matern_5_2: 1, exponential: 2 };

/** Fit the whole stack youngest-first. NaN kernel options → crate defaults (range/c_o stay
 *  automatic for every kernel, as in gempy); uni_degree = 1; decimate off (testbed inputs are
 *  tiny). The new ABI arg order puts `unboundedAbove` (Axis B) right after `relation`
 *  (Axis A) — see WasmStack::add_series (contract D1). */
function buildStack(series: SeriesJson[], kernel: KernelName): WasmStack {
  const stack = new WasmStack();
  const kernelCode = KERNEL_CODE[kernel];
  for (const s of series) {
    const fit = [f64(s.sp_xyz), f64(s.sp_if), f64(s.ori_xyz), f64(s.ori_pole)] as const;
    // range, c_o, i_res, gi_res, uni_degree, kernel, nugget
    const opts = [NAN, NAN, NAN, NAN, 1, kernelCode, NAN] as const;
    const faults = [f64(s.fault_point), f64(s.fault_normal)] as const;
    if (s.importedMesh) {
      // Imported mesh surface (feature 004): the engine parses the .obj bytes and fits a field
      // through the mesh; point/orientation arrays are unused. role 0=contact 1=erosion 2=veinWall
      // 3=topography. Arg order: bytes, role, relation, ...opts, decimate, ...faults.
      const roleCode = { contact: 0, erosion: 1, veinWall: 2, topography: 3 }[s.importedMesh.role];
      stack.add_imported_surface(
        s.importedMesh.bytes,
        roleCode,
        s.relation,
        s.importedMesh.extend,
        ...opts,
        NAN,
        ...faults,
      );
    } else if (s.topography) {
      // Singleton ground surface (feature 003): pinned youngest by add_topography (insert-0),
      // caps the model to Air above. No relation/unbounded/crossCut. Emitted first by toSeriesJson.
      stack.add_topography(...fit, ...opts, NAN, ...faults);
    } else if (s.crossCut?.kind === "vein") {
      stack.add_vein(...fit, ...opts, s.crossCut.footWall, s.crossCut.hangingWall, NAN, ...faults);
    } else if (s.crossCut?.kind === "intrusion") {
      stack.add_intrusion(...fit, ...opts, s.crossCut.envelopeInner, s.crossCut.envelopeOuter, NAN, ...faults);
    } else {
      stack.add_series(...fit, ...opts, s.relation, s.unboundedAbove, NAN, ...faults);
    }
  }
  return stack;
}

/** Mesh + collect the viewer surfaces (open contact sheets, one per interface per block).
 *  Contact mode is the MVP default; the solid-mode toggle is a later task. */
function collectSurfaces(
  stack: WasmStack,
  series: SeriesJson[],
  extent: [number, number, number, number, number, number],
  depth: number,
  weld: boolean,
  seamClip: boolean,
): SurfacePayload[] {
  const [x0, x1, y0, y1, z0, z1] = extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  const out: SurfacePayload[] = [];
  // (series,interface) contacts already emitted by a cross-cut body's SHARED-CREASE meshing — the
  // series loop skips these so they aren't meshed twice (the shared mesh is the watertight one).
  const sharedContacts = new Set<string>();

  for (let si = 0; si < stack.series_count; si++) {
    const n = stack.interface_count(si);
    // A cross-cutting body (vein/intrusion) — with weld ON — co-contours its OWN walls AND the
    // contacts it cuts on one octree, so the crease is shared and watertight (no see-through
    // holes). It returns the walls plus the cut contacts, tagged; we emit all and record the
    // contacts so the series loop below skips re-meshing them independently.
    if (series[si]?.crossCut != null && weld) {
      // Two no-flag bindings (a trailing flag arg is dropped across this wasm-bindgen toolchain —
      // see `wasm.rs`): pick the clip or snap seam closer by the checkbox.
      const tagged = seamClip
        ? stack.mesh_cross_cut_shared_octree_clip(si, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS)
        : stack.mesh_cross_cut_shared_octree(si, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS);
      for (const tm of tagged) {
        if (tm.triangle_count > 0) {
          out.push({
            series: tm.series,
            block: 0,
            interface: tm.interface,
            positions: tm.positions(),
            normals: tm.normals(),
            indices: tm.indices(),
          });
        }
        if (tm.is_contact) sharedContacts.add(`${tm.series}:${tm.interface}`);
        tm.free();
      }
      continue;
    }
    // An IMPORTED surface (feature 004): the contact sheet IS its triangles — return them directly
    // (no contouring; exact + instant at any world coordinate). Series-only interface meshing can't
    // handle it (it's not a fitted Series). Solid mode composes unit solids via the recipe path.
    if (series[si]?.importedMesh != null) {
      const m = stack.imported_surface_contact(si);
      if (m.triangle_count > 0) {
        out.push({ series: si, block: 0, interface: 0, positions: m.positions(), normals: m.normals(), indices: m.indices() });
      }
      continue;
    }
    // Topography, OR a cross-cut body with weld OFF (the escape hatch): mesh own surfaces as open
    // sheets, one per interface, the original independent way.
    const isSheetElement = series[si]?.crossCut != null || series[si]?.topography === true;
    if (isSheetElement) {
      for (let k = 0; k < n; k++) {
        const m = stack.mesh_cross_cut_surface_octree(
          si, k, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS, weld,
        );
        if (m.triangle_count > 0) {
          out.push({ series: si, block: 0, interface: k, positions: m.positions(), normals: m.normals(), indices: m.indices() });
        }
        m.free();
      }
      continue;
    }
    // Does a YOUNGER erode series clip this one? (youngest-first: indices < si). If so, with the
    // "clip seam" toggle ON, close that erosion seam with the mesh–mesh COREFINER instead of the
    // field masker — the A/B that makes the toggle meaningful for erode scenes (it previously only
    // affected cross-cut bodies). The Rust side falls back to the masker for faulted series.
    const youngerErodeAbove = series.some(
      (s, sj) => sj < si && s?.crossCut == null && s?.topography !== true && s?.relation === 0,
    );
    const blocks = Math.max(1, stack.series_block_count(si));
    for (let b = 0; b < blocks; b++) {
      for (let k = 0; k < n; k++) {
        // Skip a contact already produced by a cross-cut body's shared-crease meshing (block 0
        // only — the shared path doesn't fault-block; faulted series fall through to remesh).
        if (b === 0 && sharedContacts.has(`${si}:${k}`)) continue;
        const m =
          seamClip && youngerErodeAbove
            ? stack.mesh_series_block_interface_erode_clip(
                si, b, k, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS,
              )
            : stack.mesh_series_block_interface(
                si, b, k, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS, false,
              );
        if (m.triangle_count > 0) {
          out.push({
            series: si,
            block: b,
            interface: k,
            positions: m.positions(),
            normals: m.normals(),
            indices: m.indices(),
          });
        }
        m.free();
      }
    }
  }
  return out;
}

/** Mesh + collect closed watertight UNIT SOLIDS (solid mode, feature 003 §5). One solid per
 *  present `(series, block, unit)` cell; topography participates as the universal top masker
 *  (already in the fitted stack) so each solid daylights against the ground. AIR IS SKIPPED:
 *  topography contributes no rock unit, and the per-series solid path is series-only, so a
 *  topography element simply yields no solids. Each solid is tagged with its lithology id from
 *  the forwarded assignment (keyed `${series}:${unit}`) for colour-by-lithology + OBJ grouping. */
function collectSolids(
  stack: WasmStack,
  series: SeriesJson[],
  extent: [number, number, number, number, number, number],
  depth: number,
  assignment: Record<string, string>,
  seamClip: boolean,
): SurfacePayload[] {
  const [x0, x1, y0, y1, z0, z1] = extent;
  const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
  const minDepth = Math.min(2, depth);
  const out: SurfacePayload[] = [];
  // (series,block,unit) rock solids already emitted by a cross-cut body's SHARED-TREE meshing — the
  // per-series loop skips these so they aren't re-meshed independently (the shared mesh fuses the
  // body's wall; the independent one would staircase + show daylight).
  const sharedUnits = new Set<string>();

  for (let si = 0; si < stack.series_count; si++) {
    // Topography contributes no rock unit (Air above) — skip it (§5.2).
    if (series[si]?.topography) continue;
    // A cross-cutting body (vein/intrusion): co-contour its interior solid AND the rock units it cuts
    // on ONE shared tree so they fuse the wall vertex-for-vertex (no daylight). With "clip seam" ON
    // (A.2, docs/09), the wall∩bed crease ring is projected onto the analytic intersection so the
    // flat-bed-meets-curved-vein junction is SMOOTH (no clip → wasm-deterministic). The two no-flag
    // bindings avoid the trailing-flag ABI desync; the worker picks one by the checkbox. An EMPTY
    // result (wall-less body) falls back to the independent interior solid.
    if (series[si]?.crossCut) {
      const tagged = seamClip
        ? stack.mesh_cross_cut_solids_shared_clip_octree(si, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS)
        : stack.mesh_cross_cut_solids_shared_octree(si, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS);
      if (tagged.length > 0) {
        for (const tm of tagged) {
          if (tm.triangle_count > 0) {
            out.push({
              series: tm.series,
              block: tm.block,
              interface: tm.unit,
              unit: tm.unit,
              lithologyId: assignment[`${tm.series}:${tm.unit}`],
              positions: tm.positions(),
              normals: tm.normals(),
              indices: tm.indices(),
            });
          }
          sharedUnits.add(`${tm.series}:${tm.block}:${tm.unit}`);
          tm.free();
        }
        continue;
      }
      // Fallback (corefined declined): the body's independent interior solid (today's path); the cut
      // rock units fall through to the per-series loop below, meshed independently (welded).
      const m = stack.mesh_cross_cut_solid_octree(si, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS);
      if (m.triangle_count > 0) {
        out.push({
          series: si,
          block: 0,
          interface: 0,
          unit: 0,
          lithologyId: assignment[`${si}:0`],
          positions: m.positions(),
          normals: m.normals(),
          indices: m.indices(),
        });
      }
      m.free();
      continue;
    }
    const blocks = Math.max(1, stack.series_block_count(si));
    for (let b = 0; b < blocks; b++) {
      const units = stack.present_unit_count(si, b);
      for (let u = 0; u < units; u++) {
        // Skip a unit already meshed (watertight, smooth) by a cross-cut body's corefined path.
        if (sharedUnits.has(`${si}:${b}:${u}`)) continue;
        const m = stack.mesh_series_block_unit_solid(
          si, b, u, x0, y0, z0, size, x1, y1, z1, depth, minDepth, QEF_EPS,
        );
        if (m.triangle_count > 0) {
          out.push({
            series: si,
            block: b,
            interface: u,
            unit: u,
            lithologyId: assignment[`${si}:${u}`],
            positions: m.positions(),
            normals: m.normals(),
            indices: m.indices(),
          });
        }
        m.free();
      }
    }
  }
  return out;
}

async function run(cmd: RunCommand) {
  emit({ type: "status", phase: "loading kernel + engine wasm…" });
  await initWasm();

  emit({ type: "status", phase: "fitting stack…" });
  const stack = buildStack(cmd.series, cmd.kernel ?? "cubic");

  const solid = cmd.renderMode === "solid";
  // UNIFIED volume mesher (docs/14, path b): in SOLID mode (the default), emit a boolean RECIPE over
  // single-level-set PRIMITIVES that the main-thread Manifold instance composes into watertight,
  // gap-free rock units (the geology lives in Rust's `clip_maskers`; main is a dumb executor). The
  // booleans run on the persistent MAIN thread (manifold-3d hangs when re-instantiated in this
  // terminate-per-fit worker). The "manifold" toggle OFF selects the legacy field-domain solids as an
  // A/B. Contact mode is unchanged.
  const useRecipe = solid && (cmd.manifold ?? true);
  let surfaces: SurfacePayload[] = [];
  let recipeOps: RecipeOp[] | undefined;
  let recipeMeta: DoneMessage["recipeMeta"];
  if (useRecipe) {
    emit({ type: "status", phase: "meshing solids (Manifold)…" });
    const [x0, x1, y0, y1, z0, z1] = cmd.extent;
    const size = Math.max(x1 - x0, y1 - y0, z1 - z0);
    const minDepth = Math.min(2, cmd.maxDepth);
    const raw = stack.mesh_volume_recipe_octree(x0, y0, z0, size, x1, y1, z1, cmd.maxDepth, minDepth, QEF_EPS);
    recipeOps = raw.map((o) => ({ series: o.series, block: o.block, unit: o.unit, op: o.op, primId: o.primId, positions: o.positions(), indices: o.indices() }));
    for (const o of raw) o.free?.();
    recipeMeta = { extent: cmd.extent, assignment: cmd.assignment ?? {} };
  } else {
    emit({ type: "status", phase: solid ? "meshing solids…" : "meshing surfaces…" });
    surfaces = solid
      ? collectSolids(stack, cmd.series, cmd.extent, cmd.maxDepth, cmd.assignment ?? {}, cmd.seamClip ?? true)
      : collectSurfaces(stack, cmd.series, cmd.extent, cmd.maxDepth, cmd.weldTJunctions ?? true, cmd.seamClip ?? true);
  }

  // The hierarchy round-trip payload (declared state + derived view) — parsed from the
  // core's single JSON getter (contract D3+D5). The UI treats `derived` as a read-only cache.
  const update = JSON.parse(stack.core_update_json(...cmd.extent)) as CoreUpdate;
  stack.free?.();

  const transfer: Transferable[] = [];
  for (const s of surfaces) {
    transfer.push(s.positions.buffer, s.normals.buffer, s.indices.buffer);
  }
  if (recipeOps) {
    for (const o of recipeOps) transfer.push(o.positions.buffer, o.indices.buffer);
  }
  emit({ type: "done", surfaces, update, recipeOps, recipeMeta }, transfer);
}

self.onmessage = (e: MessageEvent<Command>) => {
  const cmd = e.data;
  if (cmd.type === "run") {
    run(cmd).catch((err) => {
      emit({ type: "error", message: err instanceof Error ? err.message : String(err) });
    });
  }
};

// Safety net for failures that escape the per-run `catch` above: a promise rejection thrown OUTSIDE
// `run` fires `unhandledrejection` (not the main thread's `worker.onerror`), and a top-level sync
// throw fires the worker's own `error` event — either would otherwise leave the UI blank/stale with
// no status (review 4.7). Forward both to the same readable `error` message the host already handles.
self.addEventListener("unhandledrejection", (e: PromiseRejectionEvent) => {
  const r = e.reason;
  emit({ type: "error", message: "worker: " + (r instanceof Error ? r.message : String(r)) });
});
self.addEventListener("error", (e: ErrorEvent) => {
  emit({ type: "error", message: "worker: " + (e.message || "unknown error") });
});
