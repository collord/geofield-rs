# Volume / solid mesh seam — design note (Task 01)

Decision record for how a cross-cutting body's interior solid and the surrounding rock unit solids
come to **share the wall geometry vertex-for-vertex** instead of contouring it independently. Source
task: `docs/01_volume_mesh_clip_wiring.md`.

## Reproduction (Step 1) — the premise as written does not reproduce at HEAD

The task assumes the body solid (`Stack::mesh_cross_cut_solid`) and the rock unit solid
(`Stack::mesh_one_block_unit_solid`) are two independent dual-contourings of the same wall with "no
shared vertices and no weld," leaving a see-through vertical channel. Measured on the
`vertical_vein_over_column` fixture (an *oblique*, deliberately y-wavy 2-point wall fit), at octree
depths 5/6/7:

| metric | result |
| --- | --- |
| `vein_body_meets_the_cross_cut_rock` worst foot-wall disagreement | **0.00** |
| body solid open edges off the box rim | **0** (individually watertight) |
| rock solid open edges off the box rim | **0** (individually watertight) |
| body∪rock merged + position-deduped — see-through wall edges | **0** at every depth |

Both solids are closed (`cap = true`, so the octree closes each one's own wall cap face), and their
wall vertices coincide because **both trees force max-depth refinement at the same `vein_field = lo`
crossing** — the body via its lower-contact half-space (`ContactHalfSpace::new(field, lo, false)`),
the rock via the vein `SlabComplement` masker — and a planar crossing's QEF is exact.

The task's stated mechanism ("the per-mesh weld masks the disagreement at coarse depth") is also
incorrect: `mesh_one_block_unit_solid`'s weld runs only over `bounding_contacts(unit, n)` — the
**horizontal bed contacts** — via a `ContactHeightField` (a height map `z = f(x,y)`) that cannot
represent a near-vertical or overhanging wall. It never touches the wall, so it cannot mask any wall
disagreement; raising depth reveals none.

The recent surface-mode seam corefiner (`35e5005`, `b2022ac`, `285ad10`) closed the analogous
*surface*-mode see-through dashes; the solid path was never the broken one. **This work is therefore
forward-looking robustness, not a bug fix** — chosen deliberately to make wall-sharing hold by
construction for curved walls (where the two independent trees *could* refine to different cells) and
to establish the primitive an imported wall path will reuse.

## Strategies considered

**Strategy A — shared-tree contour.** Contour the body-band solid and the adjacent rock-unit solids
on **one** octree (`octree::octree_dual_contour_shared`, which already contours each scene on the
same leaves via `contour_on_tree`, honoring `cfg.cap` — so it contours *closed solids* directly).
The wall lands on identical edge crossings → identical QEF vertices in both, robust to curvature.
Mirrors the surface path's evolution (`Stack::mesh_cross_cut_shared`).

**Strategy B — clip both solids against one wall mesh W.** Contour the wall once, convert to
`ClipMesh`, `clip` each solid against that single W with complementary keep predicates.

## Decision — Strategy A

1. **B hits the coplanar degeneracy `mesh_clip` deliberately defers.** A field-based body solid's
   wall cap face is *already coplanar with W* (the wall *is* the body's own band-boundary
   isosurface). `clip` logs-and-skips coplanar C/W triangle pairs (see the deferred-work note), so it
   would skip exactly the pairs that matter here. `clip` keeps-one-side of a mesh that *crosses* W; it
   is not built to corefine a closed solid whose surface already *is* W. B is the correct tool for an
   **imported, field-less** wall (no field to share a tree) — that is the
   `docs/07_imported_mesh_field_proxy.md` path, which has no solid call site today.

2. **A reuses the proven corefiner-free closure.** Each solid is closed by the octree's own
   `cap = true` solid closure + `orient_closed_outward`. There is **no hand-grafted cap face** — which
   is exactly the part Step 4 warns "is most likely to leave a hole." The wall cap is produced by the
   dual contour itself, the same way it is today; sharing the tree only makes the two solids' caps
   coincide. So A inherits today's watertight closure and adds sharing on top.

3. **A generalizes what already works.** The independent trees already coincide on a planar wall (the
   reproduction above). The shared tree extends that to the curved case, where the two trees might
   otherwise refine to different leaves near a strongly bent wall.

### Note on the weld

A's rock-unit scenes drop the per-unit `ContactHeightField::weld` step. On the shared tree the rock
unit's bed contacts already land on the same crossings as the neighbouring unit (same leaves, same
isovalue), so the weld — a height-field approximation that would also *fight* the shared seam by
pulling verts sub-cell — is unnecessary and is omitted in the shared path. The independent
(`mesh_one_block_unit_solid`) path keeps its weld unchanged.

## Cap closure (Step 4) — how the solid stays watertight

For a body band `lo < field < hi`, the closed solid's boundary is: the foot wall (`field = lo`), the
hanging wall (`field = hi`), and the box / lower-contact / younger-masker cap faces where the band
exits the domain. All of these are emitted by the single `cap = true` dual contour of the band
scene — the foot/hanging walls as *contact* faces (the band field crosses the cell's minimal edge)
and the rest as *cap* faces (a contributor surface crosses it), tagged at emission. The seam ring
where a wall meets a box/contact cap is shared **within** that one contour by construction (it is one
mesh). The rock unit's matching wall cap is a *separate* mesh, but on the shared tree it is the same
crossing in the same leaves, so the two abut with coincident vertices. Nothing is hand-stitched, so
there is no new hole to close — the watertightness test asserts this: **no incidence-1 edges off the
box rim** (nothing see-through), and **incidence-≥3 edges along the shared wall** as positive
evidence the two solids fused. Note this differs from `mesh_clip::assert_seam_watertight`'s exact
incidence-3: that criterion is for a *half-sheet* C meeting a *through-sheet* W (1 + 2 layers). Here
both body and rock are *closed* solids, so a shared wall edge carries each solid's two cap triangles
— incidence-4 where the tessellations coincide, ≥3 in general.

## API shape

A requires a **combined** meshser, because the body and its rock units must be contoured on one
tree — they cannot be produced by independent calls (this departs from Step 3's "add `SeamMode` to
each existing function"):

```
Stack::mesh_cross_cut_solids_shared(i, min, size, ext_max, cfg)
    -> Option<(Mesh /* body solid */, Vec<(usize, usize, usize, Mesh)> /* (series, block, unit) rock solids */)>
```

It builds one refinement tree (refined for the body band + its walls + every cut rock unit's bed
contacts and fault planes, mirroring `mesh_cross_cut_shared`'s build-scene assembly) and contours the
body-band scene plus each rock-unit scene on it. The existing `mesh_cross_cut_solid` and
`mesh_one_block_unit_solid` are unchanged — they remain the independent (`Snap`-equivalent) path, kept
byte-identical and asserted so.

## Scope

Native-first. The web3 solid view is already watertight, so wiring the combined path through `wasm.rs`
and the worker is a visual no-op today and is deferred (a follow-up, picked up when `docs/07` lands an
imported field-less wall, or if a curved-wall divergence is ever observed). "wasm green" here means
the crate still builds for `wasm32-unknown-unknown`; the worker need not adopt the new entry point.
