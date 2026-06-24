# Imported-mesh field proxy — design decision

**Status:** decided 2026-06-23. Implements the spike called for by
`docs/07_imported_mesh_field_proxy.md`.

## The question

A generated surface in this engine is the zero set of an implicit `ScalarField` (the cokriging
`Model` / `BlockField`, or `RemoteField` across the wasm boundary). Every production clip caller in
`stack.rs` drives `Keep::Predicate` from that field, not from the wall *mesh*, on purpose: near the
domain rim a wall fit extrapolates past its constraints, and a nearest-triangle plane test there
mis-signs an uncut face the field classifies correctly (`mesh_one_erode_clipped` ≈ stack.rs:786,
`mesh_cross_cut_shared`). The field is the ground truth the pipeline agrees against.

An **imported** mesh has no such field. Tasks 02/03 give the corefiner a purely *geometric* keep
predicate (signed distance / winding), which is enough to cut with. But three downstream behaviours
still want a scalar field *per import*:

- `fill_normals_from_field` (stack.rs:140) reshades corefiner output from a field gradient. The
  corefiner drops normals; an imported clip currently has nothing to reshade from and would fall
  back to face-averaged normals (the field-CSG comment warns this renders black/flat).
- the `clip` `refine` hook (`Option<&dyn ScalarField>`, mesh_clip.rs:175) is the sanctioned way to
  nudge a seam vertex onto a zero set. It is a no-op today; an imported field proxy is exactly the
  argument that would make it usable.
- octree solid-CSG contributors (`ContactSurface`, `ContactHalfSpace`, `SlabComplement`) are all
  field-backed; an import that wants to participate in field-CSG **solid** contouring needs a field.

## The three options

### 1. Signed-distance field (SDF) from the mesh
Sign from the generalized winding number we already have (`TriMesh::winding`), magnitude from the
clamped nearest-triangle distance (`TriMesh::nearest_triangle_clamped`), gradient from the
closest-point direction (analytic, unit-magnitude) with the nearest face normal as the on-surface
fallback.

- **Pros:** exact zero set ON the mesh; the gradient near the surface *is* the surface normal — the
  only thing `fill_normals_from_field` and `refine` actually need; cheap; reuses existing oracles;
  invents no global field structure.
- **Cons:** the distance gradient is the surface gradient only *near* the surface; kinks at the
  medial axis; not a smooth geological field far from the mesh.

### 2. RBF / implicit reconstruction (Hermite-RBF, like the kernel's cokriging)
Fit a smooth implicit to the imported vertices + face normals.

- **Pros:** smooth everywhere; imported and generated surfaces behave *identically* downstream,
  including far-field CSG.
- **Cons:** cost (another solve, per import); and — decisively — it **invents** field values away
  from the surface that may not match the import's intent. That is precisely the extrapolation
  failure the field-CSG comments cite as the reason to trust the field over the mesh in the first
  place; option 2 reintroduces it, just relocated from "the rim of a fitted wall" to "everywhere an
  imported wall is silent." It manufactures the very unreliability the field is supposed to be the
  cure for.

### 3. No proxy; geometric predicates only
Tasks 02/03 predicates for cutting, face-averaged normals for shading, imports excluded from
field-CSG solid contouring (use the mesh-clip volume path exclusively for imports).

- **Pros:** honest — no invented field at all.
- **Cons:** flat/face-averaged shading on imported clips; imports can never use the field-CSG solid
  path or the `refine` hook.

## Decision: option 3's discipline, with an SDF (option 1) where — and only where — a gradient is needed

We adopt **option 1** as a thin, *caller-side* `MeshSdf` proxy, used exactly at the two sites that
need a local gradient (`fill_normals_from_field` and the `refine` hook), and we **decline option 2**.

Rationale:

- The two real needs (reshade output normals; nudge a seam vertex) are both **local to the
  surface**, where an SDF's gradient is exactly the surface normal. They do not need a globally
  smooth field, so the SDF's medial-axis kinks and far-field non-smoothness are irrelevant to them.
- Option 2's only advantage over option 1 is far-field smoothness, and that advantage is bought with
  the one property this codebase has explicitly decided it does not want: an invented field where
  the input said nothing. We will not pay correctness for smoothness we don't use.
- This keeps the corefiner field-agnostic (the watch-out): `mesh_clip.rs` does not depend on the
  SDF; `MeshSdf` is a convenience a caller constructs and hands to `fill_normals_from_field` or to
  `clip`'s optional `refine` argument. Nothing more.

**Far-field solid CSG for imports** (the one thing this forgoes) is left to the mesh-clip *volume*
path (Task 01 Strategy B), not to a fabricated field. If a future need for imports in field-CSG
solid contouring emerges, revisit option 2 then, scoped to that need — do not pre-build it now.

## What the SDF is

`MeshSdf { mesh: TriMesh }` implementing `ScalarField`, matching the contourer's
`b < 0 ⇔ inside` convention:

- `scalar_field(p)` = `±distance`, sign negative inside (`winding > 0.5`), positive outside, zero on
  the surface, magnitude = clamped nearest-triangle distance. Empty mesh → 0.
- `scalar_field_gradient(p)` = the outward unit normal of the signed-distance field: `(p − closest)/d`
  outside, `(closest − p)/d` inside; on the surface (`d ≤ 1e-9`) the nearest face's outward normal
  (the case that matters for corefiner-output vertices, which lie *on* the wall, where the
  closest-point direction degenerates but the face normal is exactly right).

## Wiring

`fill_normals_from_field(&mut Mesh, &dyn ScalarField)` already accepts any `ScalarField`, so `MeshSdf`
slots in at the type level with **no change to that function** — a caller on a future imported-clip
output path passes `&MeshSdf::new(import_wall)` exactly where the field-CSG path passes
`&contact_field`. A stack test exercises that call directly.

> Scope note: there is no live imported-mesh clip *caller* in `stack.rs` yet — both clip callers
> today drive a real generated field, and the imported path is referenced only as "future"
> (stack.rs:685, :776, :1208). Standing up that caller is a separate feature (Tasks 02/03 stack
> wiring), deliberately **not** built here; this task delivers the proxy and proves it composes with
> `fill_normals_from_field`, so the caller is a one-liner when it lands. The `refine` hook is
> likewise still a no-op (mesh_clip.rs:182); `MeshSdf` is its intended argument once that nudge is
> implemented, but implementing the nudge is out of scope (and would couple the corefiner to a
> field, which the watch-out forbids).
