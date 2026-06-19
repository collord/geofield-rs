# Spec: imported topographic surface as the model's upper bound (exact-tessellation)

Status: **proposed** (not yet implemented). Scoped against commit `5ec73a5`.

## Goal

Import a static topographic mesh (think **~2M vertices**). When present, the topo is the **top
of the whole volumetric model**: every geologic unit's closed solid terminates at the topo,
and the unit's **top cap reuses the topo's ACTUAL triangles** — vertex-for-vertex faithful to
the imported tessellation, not a resampled approximation. Where the topo dips below a contact,
that contact daylights at the ground surface; where a unit reaches the surface, its lid is a
piece of the real topo.

## The key architectural decision (and why the obvious approach is wrong)

Dual contouring **resamples every surface it touches** to the octree's cell resolution
(~`domain/2^depth`; ~515 ft at depth 6 in the scale demo). If the topo were just another
`Boundary` contributor (the approach a naive read suggests, and what an earlier draft of this
spec proposed), the topo cap would be **smoothed to octree resolution** — a 2M-vertex topo
would render as a few hundred averaged samples. That is NOT vertex-faithful. **Rejected.**

Instead, the topo cap is **never contoured**. The pipeline splits the unit's boundary:

- **Sides + bottom** (geologic contacts, faults, box walls) → produced by the existing dual
  contourer, at octree resolution. Unchanged math.
- **Top cap** (the ground surface) → a **clipped copy of the real topo mesh**, keeping the
  topo's native vertices and triangles, stitched onto the contoured sides.

So "vertex-faithful" is achieved by *carrying the topo triangles through unmodified*, not by
sampling them. The contourer never sees the topo as a field to cross.

## Pipeline (per unit solid)

For each `(series, block, unit)` closed solid that the topo intersects:

1. **Contour the sides+bottom only.** Mesh the unit as today (`mesh_one_block_unit_solid`) but
   with its **top open** — i.e. do NOT cap at the box top; cap at a temporary plane above the
   topo max so the contour closes, then discard the temp-top faces. Result: a watertight
   "tube" open where the topo will lid it. (Alternative: contour fully then delete faces
   above the topo — but open-top contouring is cleaner; decide in build.)

2. **Clip the topo to the unit's footprint.** Keep only the topo triangles that lie over the
   unit's XY footprint AND below/above the relevant contacts. The topo INTERIOR triangles are
   carried through **unmodified** (native vertices preserved). Only triangles straddling the
   unit's lateral boundary are split along it.

3. **Conform the clip boundary to the contoured side (CHOSEN: "conform topo edge to
   contour").** Where the clipped topo's new boundary edge meets the contoured side's open top
   edge, **snap/insert the topo clip-edge vertices onto the contour's boundary loop**. The
   topo interior is untouched; only the seam ring is resampled to match the contour vertices.
   This guarantees a **watertight, T-junction-free** weld: every seam vertex is shared by
   exactly one lid triangle fan and one side triangle.

4. **Weld** the conformed topo lid onto the contoured tube along the shared seam loop (merge
   coincident seam vertices into single indices). Output one watertight `Mesh`: contoured
   sides/bottom + native-tessellation topo lid.

Where a **contact daylights** (rises above the topo), step 2's clip removes the contact
geometry above the topo and the topo becomes the surface there — same machinery, the seam is
just the daylight line instead of a box edge.

## New building blocks (none exist today)

`Mesh` is a bare positions/normals/indices soup — **no boundary extraction, no welding**. All
of the following are new:

- **`TopoMesh`** (engine, BSD — pure geometry, not in the EUPL kernel): the imported topo as a
  height field with an **XY grid spatial index** (≈1 triangle/cell). Supplies: `height(xy)`,
  `triangles_over(xy_rect)`, `clip_to_footprint(polygon) -> Vec<triangle>` (Sutherland–Hodgman
  per straddling triangle, in XY). The grid index is what makes 2M triangles tractable
  (O(footprint area), not O(triangles)). **This is the perf core** — without it nothing at 2M
  scale is feasible.
- **Boundary-loop extraction** on `Mesh`: the open-boundary edge ring = edges used by exactly
  ONE triangle (the same incidence logic the watertightness tests already compute). Needed to
  get the contour's open-top loop to stitch against.
- **Seam conform + weld**: snap topo clip-edge vertices onto the contour boundary loop, insert
  vertices where a contour vertex falls mid-topo-edge (kills T-junctions), merge coincident
  vertices. The fiddly part; the bulk of correctness risk lives here.
- **Open-top contouring**: a `cap`/lid option so `octree_dual_contour_scene` can leave the top
  uncapped (or a post-pass that strips the temp-top faces and returns the boundary loop).

## What is REUSED

- The whole dual contourer for sides+bottom (no algorithm change).
- The XY grid index *idea* (an earlier draft proposed this for a `Boundary` topo; here it lives
  in `TopoMesh` for clipping, not field-crossing).
- `clip_mesh_to_half_spaces` (Sutherland–Hodgman) as the model for the XY topo clip.
- The edge-incidence logic from the watertightness tests, promoted to a real boundary-loop
  extractor.

## ABI + host plumbing

- `geofield` (engine) owns `TopoMesh` and the stitch. `WasmStack` gains `set_topo(verts: &[f64],
  indices: &[u32])` (upload once, build the index), `clear_topo()`. The unit-solid mesher
  consults the stored topo automatically.
- **protocol.ts**: a `SetTopoCommand` carrying the topo (transferable `Float64Array` verts +
  `Uint32Array` indices — 2M verts ≈ 48 MB, fine as a one-time transfer). Topo on/off is a
  **remesh** (no refit — the fit never sees the topo).
- **worker.ts**: on set-topo, `stack.set_topo(...)` then re-stream.
- **main.ts / index.html**: file input → mesh parse → typed arrays → worker; a "Topo: on/off"
  toggle (disabled until loaded); optionally render the topo semi-transparent for context.

## Performance reality check (2M vertices)

- **Upload**: ~48 MB typed-array transfer to the worker, once. Fine.
- **Index build**: one pass over 2M triangles into the XY grid. Seconds, once.
- **Per unit-solid clip**: O(triangles over the unit's footprint), not O(2M). A unit covering
  10% of the map touches ~200k topo triangles → its lid alone is ~200k tris. **This is heavy**:
  a few units each carrying a chunk of a 2M-tri topo means multi-million-triangle output
  meshes. Rendering + transfer cost is real.
- **Mitigation to decide**: (a) ship the full-res lid and let the GPU handle it; (b) decimate
  the lid per unit (defeats "exact" — only for a fast preview LOD); (c) clip the topo ONCE to
  the whole model's top and share it, rather than per-unit (most units share the same ground).
  **Proposal: (c)** — compute the ground lid once, then split it per unit by the units' lateral
  boundaries (cheap 2D partition), so each topo triangle is emitted once total, not once per
  unit. This keeps total lid triangles ≈ the topo's own count, not a multiple.

## Test plan (native, pin the contract)

1. **Exact tessellation.** Topo = a known mesh with distinctive vertices; assert the unit's lid
   triangles' interior vertices are **bit-identical** to the corresponding input topo vertices
   (the faithful claim — distinct from "on-surface").
2. **Watertight weld.** Unit solid + topo lid is edge-incidence-2 everywhere (no seam crack, no
   T-junction). Extend the existing watertight checks.
3. **Seam conformance.** Every seam-loop vertex is shared by exactly one side face and one lid
   face; no contour vertex lies mid-topo-edge unsplit (T-junction probe).
4. **Daylighting.** A contact rising above the topo is truncated at the ground; the topo is the
   surface there.
5. **Footprint clip correctness.** `TopoMesh::clip_to_footprint` (indexed) agrees with a
   brute-force all-triangles clip on fuzzed footprints (fixed seed).
6. **Perf smoke.** ~2M-triangle topo: index build + one model-wide lid clip completes in a sane
   time and emits ≈topo-count triangles total (asserts the share-once strategy, not per-unit
   multiplication).

## Open questions for you

1. **Topo file format**: OBJ / PLY / glTF? (OBJ is the trivial demo parser; 2M verts in ASCII
   OBJ is ~100 MB+ and slow to parse — PLY-binary or glTF is far better at this scale.
   **Recommend binary PLY or glTF.**)
2. **Lid sharing**: OK to clip the topo once model-wide and partition per unit (proposal §perf
   (c)), keeping total lid triangles ≈ topo count? Or must each unit independently carry its own
   topo copy?
3. **Output size**: are multi-million-triangle result meshes acceptable to render/transfer, or
   do we need an LOD/decimated preview alongside the exact mesh?
4. **Outside the topo footprint** (XY beyond the topo): cap with the box top? (proposed default)
5. **Does topo bound the fit too**, or meshing only? (proposed: meshing only — fit never sees
   topo, so topo on/off is a pure remesh.)

## Effort estimate (revised for exact-tessellation)

- `TopoMesh` + XY grid index + `clip_to_footprint`: ~1 day.
- Boundary-loop extraction + open-top contouring: ~½ day.
- **Seam conform + watertight weld** (the risk): ~1.5–2 days.
- Lid-share/partition strategy: ~½ day.
- ABI + protocol + worker + file-input UI (+ a real mesh parser): ~1 day.
- Tests (§): ~½ day.
- **Total: ~4.5–5.5 days.** No kernel/EUPL changes. The seam weld is the correctness risk;
  everything else is mechanical.
