// Manifold composition for the unified volume mesher (docs/14, path b).
//
// WHERE THIS RUNS: in a DEDICATED, PERSISTENT worker (manifoldWorker.ts) — NOT the main thread and
// NOT the solver worker. manifold-3d is a SINGLE-THREADED wasm whose boolean op deadlocks on the 2nd
// instantiation in one JS REALM (measured: `subtract` never returns) — it wants ONE persistent
// instance. The solver worker is terminate-per-fit (hard-cancel), so it re-instantiates Manifold
// every run and can't host it; a Web Worker is a SEPARATE realm, so a persistent worker that inits
// Manifold exactly ONCE (and never terminates) sidesteps the deadlock and keeps the heavy
// #bodies×#triangles compose off the UI thread. The solver worker only emits a boolean RECIPE over
// single-level-set primitives (Rust `mesh_volume_recipe`); the main thread relays it here.
// `executeRecipe` composes the rock units and streams per-unit progress via the `onUnit` callback.
//
// (This module holds the Manifold LOGIC — `initManifold` + `executeRecipe`; the worker wrapper that
// owns the message loop and the run-token guarding lives in manifoldWorker.ts / main.tsx.)

import ManifoldModule from "manifold-3d";
import manifoldWasmUrl from "manifold-3d/manifold.wasm?url";
import type { ManifoldToplevel } from "manifold-3d";
import type { RecipeOp, SurfacePayload } from "./protocol";

/** A Manifold solid instance (the return type of the static constructors). */
type Solid = ReturnType<ManifoldToplevel["Manifold"]["cube"]>;

// Lazy, cached one-time init of the Manifold wasm engine. ONE instance for the page lifetime — never
// re-instantiated (that hangs). The `?url` + locateFile resolves the asset deterministically.
let manifoldReady: Promise<ManifoldToplevel> | null = null;
export function initManifold(): Promise<ManifoldToplevel> {
  if (!manifoldReady) {
    manifoldReady = (async () => {
      const wasm = await ManifoldModule({ locateFile: () => manifoldWasmUrl });
      wasm.setup();
      return wasm;
    })();
  }
  return manifoldReady;
}

/** Execute a unified volume-mesh RECIPE on the persistent main-thread Manifold instance. The flat op
 *  list interleaves primitive DEFINITIONS (`op === 255`, mesh present, id = `primId`) and per-unit OPS
 *  (`op < 255`, referencing `primId`). Each primitive is `ofMesh`'d ONCE into a pool and reused across
 *  every unit that references it — ONE Manifold ingestion per surface (no redundant re-contour /
 *  re-ingest). Then each `(series, block, unit)` group is composed: 0 = Base, 1 = Intersect,
 *  2 = Subtract, 3 = BaseBox (the domain cube). Every unit is clipped to the exact domain (the
 *  primitives overhang, so the cut is flap-free); shared contacts coincide by construction (the SAME
 *  pooled primitive bounds both adjacent units) → no gaps. One tagged SurfacePayload per non-empty
 *  unit. A primitive that fails `ofMesh` is null in the pool; any unit referencing it is dropped.
 *
 *  Returns the surfaces AND a count of rock units dropped because a primitive failed to ingest, so
 *  the caller can warn instead of rendering the missing volumes silently as "done" (review 2.3). */
export interface RecipeResult {
  surfaces: SurfacePayload[];
  /** Rock units skipped because a referenced primitive failed Manifold `ofMesh` (NotManifold). */
  droppedUnits: number;
}

export function executeRecipe(
  wasm: ManifoldToplevel,
  ops: RecipeOp[],
  extent: [number, number, number, number, number, number],
  assignment: Record<string, string>,
  onUnit?: (k: number, n: number) => void,
): RecipeResult {
  const { Manifold, Mesh } = wasm;
  const [x0, x1, y0, y1, z0, z1] = extent;
  const domainBox = Manifold.cube([x1 - x0, y1 - y0, z1 - z0]).translate([x0, y0, z0]);

  const toManifold = (positions: Float32Array, indices: Uint32Array): Solid | null => {
    const mesh = new Mesh({ numProp: 3, vertProperties: Float32Array.from(positions), triVerts: Uint32Array.from(indices) });
    mesh.merge();
    try {
      const m = Manifold.ofMesh(mesh);
      return m.status() === "NoError" ? m : (m.delete(), null);
    } catch {
      return null; // NotManifold throws
    }
  };

  // Pass 1: ingest each primitive ONCE into the pool (id → Manifold | null).
  const pool = new Map<number, Solid | null>();
  const groups = new Map<string, RecipeOp[]>();
  for (const o of ops) {
    if (o.op === 255) {
      pool.set(o.primId, toManifold(o.positions, o.indices));
      continue;
    }
    const key = `${o.series}:${o.block}:${o.unit}`;
    let g = groups.get(key);
    if (!g) groups.set(key, (g = []));
    g.push(o);
  }

  // Pass 2: compose each unit from the pooled primitives (booleans produce NEW solids; pooled inputs
  // are reused, deleted only at the end). `onUnit(k, n)` fires after each unit group so the host
  // worker can stream per-unit progress to the UI (n = total unit groups, known now).
  const out: SurfacePayload[] = [];
  let droppedUnits = 0;
  const n = groups.size;
  let k = 0;
  for (const [key, list] of groups) {
    let result: Solid | null = null;
    let ownsResult = false; // whether `result` is ours to delete (NOT a pooled prim / domainBox)
    let failed = false;
    for (const o of list) {
      if (o.op === 3) { result = domainBox; ownsResult = false; continue; } // BaseBox
      const m = pool.get(o.primId);
      if (!m) { failed = true; break; } // a primitive failed → drop this unit
      if (o.op === 0) {
        // Clip the Base to the domain BEFORE the boolean ops — set-identical to clipping after
        // (`(A−B)∩box ≡ (A∩box)−B`), but it kills the primitive's margin-overhang ring up front.
        // Left until the end, that ring survives any Subtract whose prim spans exactly the domain
        // footprint (an imported `close_sheet_below` prism, unlike a `below_solid` contour, has no
        // lateral margin), and the final clip then crushes it into zero-thickness coincident faces
        // on the domain wall — flap triangles in `getMesh()` that break downstream nearest-facet
        // sign tests (the partition harness measured a phantom 10% overlap from exactly this).
        result = m.intersect(domainBox);
        ownsResult = true;
      } else if (result) {
        const next: Solid = o.op === 1 ? result.intersect(m) : result.subtract(m);
        if (ownsResult) result.delete();
        result = next;
        ownsResult = true;
      }
    }
    if (failed || !result) {
      if (ownsResult && result) result.delete();
      // A primitive failed to ingest (NotManifold) → this rock unit is missing from the scene.
      // Count it so the caller can surface a warning rather than finishing silently (review 2.3).
      if (failed) droppedUnits++;
      onUnit?.(++k, n);
      continue;
    }
    const clipped = result.intersect(domainBox);
    if (ownsResult) result.delete();
    const volume = clipped.status() === "NoError" ? clipped.volume() : 0;
    if (volume > 0) {
      const [series, block, unit] = key.split(":").map(Number);
      const mesh = clipped.getMesh();
      out.push({
        series,
        block,
        interface: unit,
        unit,
        volume,
        lithologyId: assignment[`${series}:${unit}`],
        positions: Float32Array.from(mesh.vertProperties),
        normals: new Float32Array(0),
        indices: Uint32Array.from(mesh.triVerts),
      });
    }
    clipped.delete();
    onUnit?.(++k, n);
  }
  for (const m of pool.values()) m?.delete();
  domainBox.delete();
  return { surfaces: out, droppedUnits };
}
