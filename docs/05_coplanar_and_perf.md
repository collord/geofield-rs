# Task 05 — Coplanar intersection handling and the O(|C|·|W|) sweep

**Priority:** Medium. Two related correctness/scaling gaps that imported meshes will hit harder than
generated ones.

## Part A — Coplanar (and near-coplanar) triangle pairs are silently dropped

`tri_tri_segment` (≈ line 310) returns `None` for coplanar pairs:

- `if dot(dir, dir) <= COPLANAR_EPS2 { return None; }` (≈ line 344), and
- the docstring states "Milestone 1 returns `None` for coplanar pairs (logged by the caller as a
  deferred case)" — but I don't see the caller actually logging it; confirm and fix.

For generated contact-vs-wall geometry, exact coplanarity is rare. For **imported** meshes it is
common and load-bearing: a fault sheet and a bedding contact that are locally parallel, or two
imported surfaces that share a flat region, will produce coplanar overlaps whose boundary is the real
seam. Dropping them leaves the contact running THROUGH the wall with no seam — a silent
watertightness hole that no current test catches because all fixtures cross transversally.

What to do:

1. First, make the deferral observable: actually log/count coplanar skips (a debug counter on
   `ClipResult`, or a `tracing` event) so it stops being invisible.
2. Decide the intended semantics for a coplanar overlap in a *keep-one-side* corefiner. The
   principled handling is the 2D polygon-overlap boundary in the shared plane (the coplanar branch of
   Möller / the standard CGAL corefinement coplanar case). Scope this as its own milestone; it's more
   than an epsilon tweak.
3. At minimum, add a test that constructs a coplanar overlap and asserts the CURRENT behavior is
   *detected and reported* (not silently wrong), so the limitation is documented and gated rather than
   latent.

## Part B — the all-pairs sweep won't scale to imports

Step A is the full `for tc in C { for tw in W { ... } }` double loop (≈ line 158), explicitly marked
"Milestone 1 uses the full O(|C|·|W|) sweep; an AABB grid is a perf-only follow-up." Generated
contour meshes near a single seam are small; an imported mesh can be hundreds of thousands of
triangles, making this quadratic loop the dominant cost and possibly wasm-OOM/timeout territory.

What to do:

1. Add an AABB acceleration structure over W's triangles (a uniform grid keyed on the seam quantum,
   or a simple BVH) and query only candidate W triangles per C triangle. Keep the exact tri-tri test
   unchanged — this is purely a candidate filter, so results must be identical.
2. Gate it so small inputs still take the simple path (avoid the structure's overhead when |C|·|W|
   is tiny) — or just always use it if the BVH build is cheap; measure.

## Acceptance

- Part A: a coplanar-overlap test that asserts the case is reported (and, if you implement the
  coplanar boundary, that the seam is watertight). Document the chosen scope in
  `docs/mesh_clip_design.md`.
- Part B: a perf test / benchmark on a large synthetic import (e.g. two 200×200 jittered grids)
  showing the accelerated path produces byte-identical triangles to the brute-force sweep and runs in
  a fraction of the time. Assert identical output, not just "faster."
- Native and wasm green; the perf path must not diverge between targets.

## Watch-out

- `COPLANAR_EPS2 = 1e-18` is an absolute squared threshold on an UNnormalized cross product, so its
  effective angular tolerance scales with the triangles' edge lengths. For large imported triangles
  this is far too loose (it'll call genuinely-crossing pairs coplanar); for tiny ones too tight. Make
  the coplanarity test scale-invariant (normalize, or compare against the normals' magnitudes) as
  part of Part A.
