# Task 01 — Wire `clip` (or a shared seam) into the volume/solid mesh paths

**Priority:** High. This is the "known issues when generating volume meshes instead of just surface
meshes — I don't think the `mesh_clip.rs` routines are being called there" item, made concrete.

## Context

In `stack.rs`, the **surface** paths corefine correctly:

- `mesh_cross_cut_shared` calls `clip(&c, combined, Keep::Predicate(&exterior), None)` (around
  line 1266) and `mesh_one_erode_clipped` calls `clip(&c, &w, Keep::Predicate(&survive), None)`
  (around line 793).

The **volume/solid** paths do not call `clip` at all:

- `mesh_cross_cut_solid` (≈ line 1292) builds one `Scene` and calls `octree_dual_contour_scene`.
- `mesh_one_block_unit_solid` (≈ line 1402) builds one `Scene`, calls `octree_dual_contour_scene`,
  then welds bounding contacts with `ContactHeightField::build(...).weld(&mut mesh)`.

A cross-cutting body's interior solid and the surrounding rock unit solid are therefore **two
independent dual-contourings of the same foot-wall surface, from opposite sides**, with no shared
vertices and no weld between them. The regression test `vein_body_meets_the_cross_cut_rock` (≈ line
3851) documents the resulting see-through vertical channel and asserts the two surfaces agree on the
foot-wall to within `cell * 0.25`. Confirm whether that test currently passes; if it passes it is
likely because the per-mesh `weld` masks the disagreement at coarse depth — verify by tightening the
tolerance and/or raising depth.

## The actual problem to solve

The two solids must share the wall geometry vertex-for-vertex, not merely land near it. There are
two viable strategies — evaluate both and recommend one in a short design note
(`docs/volume_mesh_seam.md`) before implementing:

**Strategy A — shared-tree contour, then `clip` the cap faces.** Mirror what
`mesh_cross_cut_shared` already does for surfaces: contour the body solid and the rock unit solid on
ONE shared octree (`octree_dual_contour_shared`), so the shared wall lands on identical edge
crossings in both. Then, where each solid's cap face meets the wall, run them through `clip` so the
seam vertices are shared by index. This reuses the corefiner and is the most faithful to the
existing surface path.

**Strategy B — contour the wall once, clip both solids against that single wall mesh.** Contour the
shared wall a single time, convert to `ClipMesh`, and `clip` each solid against that one `W` with
complementary `Keep` predicates (body keeps interior, rock keeps exterior). This guarantees a single
seam-id space and is closest to how an *imported* wall would be handled, so it doubles as the
imported-mesh volume path.

In either case the body and the rock must end up referencing the **same** wall vertices, so the
`vein_body_meets_the_cross_cut_rock` disagreement goes to ~0 (float noise), not `< 0.25 cell`.

## Concrete steps

1. Reproduce first. Tighten `vein_body_meets_the_cross_cut_rock`'s `tol` toward float-noise and/or
   raise `depth`; capture the worst disagreement so you have a before/after number.
2. Write `docs/volume_mesh_seam.md` choosing A or B with rationale (watertightness, reuse of the
   corefiner, imported-mesh parity, cost). Note that `mesh_one_block_unit_solid` currently relies on
   `ContactHeightField::weld`, which is a height-field weld and will NOT generalize to a vertical or
   overhanging imported wall — that's a strike against leaning on weld and a point for `clip`.
3. Implement the chosen strategy behind the existing `SeamMode` enum so `Snap` stays byte-identical
   and `Clip` opts into the shared-seam volume path (matching how the surface paths gate it).
   `mesh_cross_cut_solid` and `mesh_one_block_unit_solid` currently take no `SeamMode`; add it
   (defaulting to `Snap`) rather than changing default behavior silently.
4. Make `clip`'s output usable as a closed-solid cap: the corefiner emits an OPEN side (it keeps one
   side of `W` for `C`). For a watertight *solid* you need the kept `C`, the relevant part of `W` as
   the cap face, and the rest of the solid's box/lower-contact caps to all share the seam ring. Spell
   out in the doc how the cap closes (this is the part most likely to leave a hole).

## Acceptance

- `vein_body_meets_the_cross_cut_rock` passes with a tolerance near float noise (propose the exact
  bound in the PR), not `0.25 * cell`.
- A NEW test asserts the merged body∪rock solid is watertight across the shared wall using the same
  edge-incidence criterion as `assert_seam_watertight` in `mesh_clip.rs` (incidence-3 along the seam,
  no incidence-1 off the rim). Put it next to the existing solid tests in `stack.rs`.
- `Snap` mode output is unchanged (add an assertion or snapshot to prove it).
- Native and wasm both green.

## Watch-outs

- `mesh_one_block_unit_solid`'s `weld` step and a new `clip` step must not fight each other — if you
  adopt `clip`, the bounding-contact weld for the *clipped* faces should be removed or made
  seam-aware, or you'll re-introduce a sub-cell pull off the shared seam.
- The closed solid uses `cap = true`; the corefiner was validated on open sheets (`cap = false`).
  The cap faces (box, lower contact) are new neighbors for the seam ring — test that they close.
