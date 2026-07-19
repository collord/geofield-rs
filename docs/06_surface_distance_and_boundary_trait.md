# Task 06 — `surface_distance` gaps and `Boundary`-trait import-readiness

**Priority:** Medium. These are correctness papercuts in the octree side that specifically bite the
imported-mesh (`TriMesh`) and planar-fault (`PlaneHalfSpace`) boundaries.

## Issue A — `surface_distance` is unimplemented on `TriMesh` and `PlaneHalfSpace`

`Boundary::surface_distance` (≈ octree.rs line 112) defaults to `None`. It is used by
`Scene::seam_boundary_normal` (≈ line 454) to pick the **nearest** contributor's surface to constrain
a seam vertex's projection — the comment is explicit that first-responder is wrong because the domain
box answers `surface_normal` for every point, so a contact↔wall seam vertex gets slid along a box
face instead of the real wall ("the see-through dashes").

`ContactSurface`, `ContactHalfSpace`, and `SlabComplement` all implement `surface_distance`. But:

- `TriMesh` does **not** implement `surface_distance` (it has `inside`, `edge_crossing`,
  `surface_normal` only — ≈ line 714). So an **imported fault mesh** used as a contributor always
  returns `None` for distance, which means `seam_boundary_normal` can never pick it as the nearest
  surface — it falls back to first-responder and slides the seam onto the box. This is the exact bug
  the nearest-surface logic was written to fix, reintroduced for imports.
- `PlaneHalfSpace` also does **not** implement `surface_distance` (≈ line 354). A planar fault
  contributor has the same problem.

Fix: implement `surface_distance` on both.

- `PlaneHalfSpace`: trivial — `|signed(p)|` (it already has `signed`).
- `TriMesh`: unsigned distance to the nearest triangle. You already have
  `weld::closest_point_on_triangle_pub` (used in `mesh_clip::nearest_w_signed`); reuse it, or the
  centroid-distance `surface_normal` already does as a cheaper approximation (the trait says
  "approximate is fine"). Prefer the clamped closest-point for correctness on large facets.

## Issue B — `TriMesh::edge_crossing` returns the FIRST/nearest hit only; multi-crossing edges

`TriMesh::edge_crossing` (≈ line 722) returns the single nearest ray-triangle hit, and
`edge_crossing_count` is the default `is_some() as usize` (0 or 1). A non-convex imported mesh (a
folded fault, a mesh the cell edge enters and exits) can cross the SAME imported boundary twice on
one cell edge. The `SlabComplement` boundary overrides `edge_crossing_count` precisely because a thin
band can be double-crossed and must force subdivision. `TriMesh` has the same exposure but no
override, so a double-crossed cell against an imported mesh won't subdivide and the contour can crack.

Fix: implement `TriMesh::edge_crossing_count` to count *all* segment/triangle intersections along the
edge (you already iterate all triangles in `edge_crossing` — return the count there too, or factor a
shared "all hits" helper). This wires the imported boundary into the existing double-crossed-edge
refinement (`edge_double_crossed`, ≈ line 516) for free.

## Issue C — `surface_normal` by nearest CENTROID picks the wrong facet on large/oblique triangles

`TriMesh::surface_normal` (≈ line 741) picks the facet whose CENTROID is nearest `p`. For an imported
mesh with non-uniform triangle sizes, the centroid-nearest triangle is not necessarily the one `p`
actually sits on (a big neighbor's centroid can be closer than the small facet under `p`). Use the
clamped closest-point distance instead, consistent with the fix you're already making for
`surface_distance` — they should share one "nearest triangle by clamped distance" helper.

## Concrete steps

1. Add a shared private helper on `TriMesh`: `nearest_triangle_clamped(p) -> (usize, f64, [f64;3])`
   returning (triangle, distance, closest point) using `closest_point_on_triangle`.
2. Implement `TriMesh::surface_distance` and rewrite `surface_normal` on top of that helper.
3. Implement `TriMesh::edge_crossing_count` (all hits).
4. Implement `PlaneHalfSpace::surface_distance` as `|signed(p)|`.

## Acceptance

- A test placing an imported `TriMesh` fault as contributor 1 (with the box as contributor 0) and
  asserting `seam_boundary_normal` at a seam vertex returns the FAULT's normal, not the box's — i.e.
  the nearest-surface choice now works for imports. (Today it returns the box normal.)
- A double-crossed-edge test against a non-convex `TriMesh` showing the cell now subdivides and the
  contour is watertight where it previously cracked.
- Existing octree/`TriMesh` tests (`octahedron`, `curved_surface_is_crack_free_*`) unchanged.

## Watch-out

- Don't change the `Boundary` trait's default `surface_distance` to non-`None` — other impls rely on
  the `None` fallback meaning "not a nearest-surface candidate." Only add concrete impls.
