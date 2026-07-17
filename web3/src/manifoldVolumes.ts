// MAIN-THREAD Manifold pass for the unified volume mesher (docs/14, path b).
//
// WHY HERE (not in the worker): the solver worker is terminate-per-fit (its hard-cancel design), so
// every re-render spawns a fresh worker. manifold-3d is a SINGLE-THREADED wasm whose boolean op
// deadlocks on the 2nd instantiation in a page (measured: `subtract` never returns) — it wants ONE
// persistent instance, the exact opposite of the engine wasm. So Manifold lives on the main thread,
// inited once and reused for the page's lifetime; the worker only emits a boolean RECIPE over
// single-level-set primitives (Rust `mesh_volume_recipe`) and ships it here. `executeRecipe` composes
// the rock units at the tail of a render the user is already waiting on, so it is not a perceptible block.

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
  // are reused, deleted only at the end).
  const out: SurfacePayload[] = [];
  let droppedUnits = 0;
  for (const [key, list] of groups) {
    let result: Solid | null = null;
    let ownsResult = false; // whether `result` is ours to delete (NOT a pooled prim / domainBox)
    let failed = false;
    for (const o of list) {
      if (o.op === 3) { result = domainBox; ownsResult = false; continue; } // BaseBox
      const m = pool.get(o.primId);
      if (!m) { failed = true; break; } // a primitive failed → drop this unit
      if (o.op === 0) {
        result = m;
        ownsResult = false; // pooled — don't delete
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
      continue;
    }
    const clipped = result.intersect(domainBox);
    if (ownsResult) result.delete();
    if (clipped.status() === "NoError" && clipped.volume() > 0) {
      const [series, block, unit] = key.split(":").map(Number);
      const mesh = clipped.getMesh();
      out.push({
        series,
        block,
        interface: unit,
        unit,
        lithologyId: assignment[`${series}:${unit}`],
        positions: Float32Array.from(mesh.vertProperties),
        normals: new Float32Array(0),
        indices: Uint32Array.from(mesh.triVerts),
      });
    }
    clipped.delete();
  }
  for (const m of pool.values()) m?.delete();
  domainBox.delete();
  return { surfaces: out, droppedUnits };
}
