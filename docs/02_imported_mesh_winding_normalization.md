# Task 02 — Provide a winding-normalization producer for imported `W` meshes

**Priority:** High. This is the single biggest gap between "the corefiner is field-agnostic" and "you
can actually feed it an imported mesh."

## Context

`mesh_clip::clip` has a hard precondition on `W` (see the `KeepSide` doc comment ≈ line 104 and the
`debug_assert!(w_is_consistently_wound(w), ...)` at the top of `clip`, ≈ line 152):

> W is a coherently oriented sheet (all triangles wound consistently). Our octree contourer satisfies
> this; **imported meshes must be winding-normalized BEFORE `clip`** (per connected component — a
> multi-part import may mix normal senses). `clip` asserts this in debug builds; **it does not repair
> winding.**

But there is **no function anywhere in the crate that performs that per-component winding
normalization for an arbitrary imported mesh.** What exists:

- `octree::orient_closed_outward` (≈ line 1047) — flood-fills consistent winding then flips by signed
  volume. It assumes a CLOSED mesh and orients OUTWARD by enclosed volume sign. An imported clipping
  *wall* is typically an OPEN sheet, where "outward by volume" is undefined. So this is close in
  spirit (it already does the flood-fill half) but wrong for the open-sheet case.
- `w_is_consistently_wound` in `mesh_clip.rs` (≈ line 843) — only *detects* inconsistency via the
  "same directed edge seen twice" test; it does not repair, and it is a debug check.

So today, importing a mesh and calling `clip` will trip the debug assert (or silently misbehave in
release) unless the import happened to be authored with globally consistent winding.

## What to build

A reusable, **field-agnostic** winding normalizer that takes a triangle soup / indexed mesh and
returns a per-connected-component consistently-wound mesh. It must NOT live in `mesh_clip.rs` (that
module must stay free of anything beyond the corefinement primitive) — put it in `mesh.rs` or a new
`mesh_repair.rs`, and have `mesh_clip`'s tests/imports use it.

Required behavior:

1. **Connected-component split** by shared edges (an import may contain several disjoint sheets with
   independently arbitrary winding). Document the adjacency definition (shared undirected edge;
   decide how to treat non-manifold edges — probably keep them as component boundaries).
2. **Flood-fill consistent winding within each component** (reuse the logic from
   `orient_closed_outward`'s flood-fill, factored out so both call it).
3. **Per-component sense choice.** For a CLOSED component, orient outward by signed volume (as
   `orient_closed_outward` does). For an OPEN component, "outward" is ambiguous — the corefiner only
   needs *internal consistency*, and the caller picks the keep side via `KeepSide`/`Keep::Predicate`
   anyway, so for open components just make the winding internally consistent and document that the
   absolute sense is the caller's responsibility (this matches the existing
   `m2_staircase_on_real_contoured_meshes` comment that the clip "only needs W consistently wound").
4. Handle the degenerate inputs an import will actually contain: duplicate vertices (weld within an
   epsilon first, or document that the caller must), zero-area triangles (drop), and isolated
   vertices.

## Concrete steps

1. Factor the flood-fill out of `octree::orient_closed_outward` into a shared
   `fn flood_fill_consistent_winding(&mut Mesh)` (or on the soup type) and have `orient_closed_outward`
   call it, then apply the volume flip. Keep `orient_closed_outward`'s tests green.
2. Add `fn normalize_winding_per_component(mesh) -> mesh` implementing 1–4 above.
3. Add `ClipMesh`-level convenience (e.g. `ClipMesh::winding_normalized(self) -> Self`) or document
   the call site so a caller does `let w = normalize_winding_per_component(import); clip(&c, &w, …)`.
4. Update the `mesh_clip.rs` module docs and the `KeepSide` precondition comment to point at the new
   function by name, so the contract is discoverable.

## Acceptance

- A test that takes a known mesh, randomly flips a subset of its triangle windings AND its
  components, normalizes, and asserts `w_is_consistently_wound` is true and (for closed components)
  outward by volume. Use a multi-component fixture (two disjoint sheets) to exercise the per-component
  path.
- A test that feeds a deliberately mis-wound multi-component import through
  `normalize_winding_per_component` and then `clip`, and confirms the debug assert no longer fires
  and the seam is watertight (`assert_seam_watertight`).
- `orient_closed_outward`'s existing octree tests unchanged.

## Watch-outs

- Generalized winding number (`TriMesh::winding`) is robust to imperfect meshes for *inside/outside*,
  but winding *normalization* is a topological flood-fill — don't conflate them.
- Non-manifold and boundary edges: an open sheet has incidence-1 edges everywhere on its rim; the
  flood-fill must treat those as component boundaries, not inconsistencies.
