// MAIN-THREAD Manifold boolean pass for cross-cut volume meshing (docs/13 §5, docs/14).
//
// WHY HERE (not in the worker): the solver worker is terminate-per-fit (its hard-cancel design), so
// every re-render spawns a fresh worker. manifold-3d is a SINGLE-THREADED wasm whose boolean op
// deadlocks on the 2nd instantiation in a page (measured: `subtract` never returns) — it wants ONE
// persistent instance, the exact opposite of the engine wasm. So Manifold lives on the main thread,
// inited once and reused for the page's lifetime; the worker only produces the raw co-contoured host
// slabs (`mesh_cross_cut_host_slabs`) and ships them here. The booleans are ~0.4 s and run at the tail
// of a render the user is already waiting on (status shows "fitting/meshing"), so the brief main-thread
// pass is not a perceptible block.

import ManifoldModule from "manifold-3d";
import manifoldWasmUrl from "manifold-3d/manifold.wasm?url";
import type { ManifoldToplevel } from "manifold-3d";
import type { ManifoldSlab, SurfacePayload } from "./protocol";

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

/** Difference the vein BAND from each wall-free host SLAB and clip to the exact domain, producing
 *  watertight rock-unit solids whose shared wall is corefined by Manifold (no peek-a-boo) and whose
 *  borders are flap-free (the band + slabs overhang the domain; one clean `domainBox` cut, docs/14).
 *  `slabs[0]` is the band; `slabs[1..]` are the cut units. Returns the band solid + each rock unit
 *  (tagged for colour-by-lithology), or `null` if the band can't be made a valid Manifold (caller
 *  falls back to the field-path solids already in `surfaces`). A single bad slab is skipped, not fatal. */
export function meshManifoldVolumes(
  wasm: ManifoldToplevel,
  slabs: ManifoldSlab[],
  extent: [number, number, number, number, number, number],
  veinIndex: number,
  assignment: Record<string, string>,
): SurfacePayload[] | null {
  const { Manifold, Mesh } = wasm;
  const [x0, x1, y0, y1, z0, z1] = extent;
  if (slabs.length === 0) return null;

  const toManifold = (positions: Float32Array, indices: Uint32Array) => {
    const mesh = new Mesh({ numProp: 3, vertProperties: Float32Array.from(positions), triVerts: Uint32Array.from(indices) });
    mesh.merge(); // weld near-coincident verts; host slabs are clean so this suffices
    try {
      const m = Manifold.ofMesh(mesh);
      return m.status() === "NoError" ? m : (m.delete(), null);
    } catch {
      return null; // NotManifold throws — caller treats as failure
    }
  };

  const band = toManifold(slabs[0].positions, slabs[0].indices);
  if (!band) return null; // band not a valid solid → fall back rather than emit garbage

  // The single authoritative domain cube (the EXACT extent). The band and slabs OVERHANG it, so every
  // emitted boundary face is a transversal cut by THIS cube — no coplanar boundary meet → no flaps.
  const domainBox = Manifold.cube([x1 - x0, y1 - y0, z1 - z0]).translate([x0, y0, z0]);

  const out: SurfacePayload[] = [];
  // The vein body (band) as its own solid (series=veinIndex, unit 0) — clipped to the exact domain.
  {
    const bandClipped = band.intersect(domainBox);
    const bm = bandClipped.getMesh();
    out.push({
      series: veinIndex,
      block: 0,
      interface: 0,
      unit: 0,
      lithologyId: assignment[`${veinIndex}:0`],
      positions: Float32Array.from(bm.vertProperties),
      normals: new Float32Array(0), // renderer recomputes face normals when empty
      indices: Uint32Array.from(bm.triVerts),
    });
    bandClipped.delete();
  }

  // --- cut rock units = (hostSlab − band) ∩ domainBox, one per cut (series,block,unit) ---
  for (let i = 1; i < slabs.length; i++) {
    const s = slabs[i];
    const slab = toManifold(s.positions, s.indices);
    if (!slab) continue; // a defective slab → skip (field fallback for this unit kept by the caller)
    const diff = slab.subtract(band);
    const rock = diff.intersect(domainBox);
    if (rock.status() === "NoError" && rock.volume() > 0) {
      const rm = rock.getMesh();
      out.push({
        series: s.series,
        block: s.block,
        interface: s.unit,
        unit: s.unit,
        lithologyId: assignment[`${s.series}:${s.unit}`],
        positions: Float32Array.from(rm.vertProperties),
        normals: new Float32Array(0),
        indices: Uint32Array.from(rm.triVerts),
      });
    }
    slab.delete();
    diff.delete();
    rock.delete();
  }
  band.delete();
  domainBox.delete();
  return out;
}
