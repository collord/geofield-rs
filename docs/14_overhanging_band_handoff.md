# Handoff — overhanging-band source to kill the Manifold boundary flaps

**Branch:** `manifold-volumes` (the Manifold-boolean volume testbed; UNCOMMITTED working-tree changes —
Rust binding + worker + curved template + e2e). **Read `docs/13_corefinement_volume_mesher_scope.md`
(esp. §5 SPIKE RESULT) and the memory note `dipping-wall-fixed-peekaboo-needs-exact-corefiner` first.**

## What works — DO NOT redo
The closed-primitive + Manifold path produces watertight cross-cut rock-unit volumes, **including curved
beds**, with no peek-a-boo. Verified: native 123/123; node harness on the curved scene — 4 units
watertight (0 open / 0 non-manifold), pairwise overlap 0.02% of box, partition residual 0.006% (at the
beds, sub-cell — a documented pre-existing slack). The architecture is settled:
- **band** = `mesh_cross_cut_solids_shared_clip_octree(vein)[0]` (the body) — Manifold-clean.
- **slabs** = the NEW `mesh_cross_cut_host_slabs_octree` binding (`mesh_cross_cut_host_slabs` in
  `stack.rs` — the wall-free, shared-tree co-contoured host-slab half of `mesh_cross_cut_solids_corefined`).
  Clean by construction (the per-unit `mesh_series_block_unit_solid` is NOT — 279–588 non-manifold edges
  from adaptive-octree T-junctions; merge() and a JS edge-split both failed; don't go back to it).
- **rock unit_k** = `slab_k.subtract(band)` in `web3/src/worker.ts` `meshManifoldVolumes`. Manifold
  corefines the shared wall internally → no peek-a-boo (the wall lives only in the band).
The "clean closed primitives" come from the SHARED-TREE co-contour, wall-free. That question is answered.

## The remaining bug — boundary FLAPS (cosmetic, but real)
Thin triangular sliver flaps hang off the solids **along the domain border** (the data-free y=0/y=1000
walls, and the floor/bed borders). Root cause: the band and slabs are **clamped to the domain box**, so
their boundary faces are **coplanar** with the slab/box at the domain edge, and Manifold sheds attached
slivers along that shared plane (a classic boolean coplanar/edge-on degeneracy).

**A prior fix was WRONG and must not be repeated:** `decompose()` → drop components below a one-cell
volume → recombine. That only removes *disconnected* degenerate shells. **These flaps are CONNECTED
slivers** — attached to the main solid — so decompose can't isolate them. The e2e assertion "zero tiny
*components*" PASSED while the flaps remained — the metric measured the wrong thing. **The user's eyes
are the gate; a green component-count check is not.** (This bit twice this session — also the W\* peek-a-boo.)

## The fix — overhang every primitive, clip once with a clean domain box
The proven trick (the corefined path already uses it: `mesh_cross_cut_solids_corefined`, `stack.rs` ~1788,
`margin = cell*3`, `wmin = min - margin`, `wsize = size + 2*margin` — "dodging the edge-on-face
degeneracy at the box∩wall∩bed corners"): make NOTHING coplanar at the domain boundary.

1. **Overhanging band.** Produce the vein-band closed solid contoured on an OVERSIZED box (domain ± margin,
   margin ≈ 3 cells) so its y/z box-caps sit OUTSIDE the domain. Likely a small new wasm binding
   `mesh_cross_cut_band_overhang_octree` — contour the vein body solid (`mesh_cross_cut_solid` /
   `mesh_cross_cut_solids_shared` body) on `[min - m, ext + m]`, capped/closed, NO `clamp_mesh_to_box`.
   Mirror the host-slabs binding the agent just added (same shape, just oversized + closed band).
2. **Overhanging slabs.** The host slabs are also clamped at the domain → their y/x (and floor/ceiling z)
   faces are coplanar too. Contour `mesh_cross_cut_host_slabs` on the SAME oversized box so they overhang
   as well. (Beds stay interior — only the domain-box-capped faces need to overhang.)
3. **One authoritative clip.** In `meshManifoldVolumes`: `rock = slab.subtract(band).intersect(domainBox)`
   where `domainBox = Manifold.cube([x1-x0, y1-y0, z1-z0]).translate([x0,y0,z0])` — the single clean cube.
   All boundary faces now come from `domainBox` (transversal cuts), nothing coplanar between primitives →
   no flaps. Drop the decompose/volume-floor hack.

## Verification discipline (the part that was skipped)
- **RENDER AND LOOK is the gate.** `cd web3 && npm run wasm:engine && npm run dev` → "Cross-cutting vein
  (dipping)" (curved bed 3) → Solids → check **manifold** → zoom the y=0/floor borders. No claim of "fixed"
  before the border is visually clean.
- **Build a connected-sliver detector** (the e2e currently can't see these). After clip, count triangles
  that are thin slivers touching the domain boundary (e.g. triangles with ≥2 verts within ε of a box face
  AND aspect ratio / area below a floor), or assert the boundary faces are planar patches of the box.
  Disconnected-component count is NOT sufficient.
- Node harness (`web3/scratch-verify/`, throwaway): keep the watertight + pairwise-overlap + partition
  checks green on the curved scene; the clip must not reintroduce overlap or holes.
- Native `cargo test -p geofield` stays green (the host-slab contour is shared with the corefined tests).

## Honest bounds / watch-outs
- Veins + curved/dipping beds work; **faults, topography, intrusions UNTESTED** in this path.
- Bed-contact sub-cell residual (0.006% curved / ~3% flat) is documented, pre-exists in the field path —
  NOT this task.
- The `intersect(domainBox)` must use the EXACT domain extent (`extent` in the worker), and the band/slab
  margins must be strictly larger than any contour vertex jitter so nothing lands back on the box face.
- If overhanging the band turns out hard to expose cleanly, the corefined path already contours
  overhanging WALL sheets (`foot_wall`/`hang_wall`); a band solid can be closed from those — but the
  wall-free shared-tree slab co-contour is the clean-primitive source, so keep the band consistent with it.

## RESULT (2026-06-28 — executed, uncommitted on `manifold-volumes`)
**FLAPS FIXED.** Implemented exactly as scoped, with ONE simplification: no new band binding was needed —
the band IS `mesh_cross_cut_host_slabs[0]` (the worker gets band + slabs from that ONE call), so overhanging
that single fn oversizes both. Changes: `stack.rs::mesh_cross_cut_host_slabs` contours on the oversized box
(`margin = cell·3`, `wmin/wext/wsize`, `box_b` + `octree_dual_contour_shared` on `wmin,wsize`, sentinel iso
sampled on the oversized box); `worker.ts::meshManifoldVolumes` builds `domainBox = Manifold.cube(extent)`
and emits `band ∩ domainBox` + `(slab − band) ∩ domainBox`, decompose/flap-floor hack DELETED.

Verification (the connected-sliver detector this doc asked for — built in `scratch-verify/verify-manifold.mjs`
AND the e2e; min-altitude `h = 2·area/longest-edge → 0` needle with ≥2 verts on ONE box face; component
count abandoned as insufficient): node harness curved scene `cross-cutting-vein-dipping` went **890 flaps /
72 collapsed slivers (baseline) → 0 collapsed slivers**, watertight, 0 pairwise overlap, partition 0.012%.
e2e green. Native `cargo test -p geofield` 123/123. Render-and-look: 4 Playwright canvas screenshots
(default + broadside y-walls + floor) — borders visually clean, no protruding slivers, no see-through.

Watch-outs confirmed: (1) **playwright serves `dist/` via `vite preview`, NOT `src/wasm`** — you MUST
`npm run build` after `npm run wasm:engine` or the e2e tests STALE code (a stale dist reported a phantom 18
slivers; the fresh build = 0). (2) Residual ~5 FLOAT-scale micro-triangles (area ~8e-5) survive at
vein∩wall∩bed TRIPLE corners — the boolean engine's documented triple-point limit (docs/13 §3); sub-visual,
watertight-preserving, NOT see-through. The gate is collapsed slivers (`h<1e-3`), which is 0. Faults / topo
/ intrusions still UNTESTED in this path.

## State to commit (once the border is visually clean)
Rust binding (`mesh_cross_cut_host_slabs` + the new overhang band) + `wasm.rs` + `web3/src/worker.ts` +
`web3/src/scenarios.ts` (curved template) + `web3/e2e/manifold-volumes.spec.ts` (with the new sliver
check). Leave `scratch-verify/` out. Branch `manifold-volumes`; commit only when asked.
