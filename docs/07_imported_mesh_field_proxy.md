# Task 07 — The "equivalent field after import" question (design + spike)

**Priority:** Low / Design-first. This addresses your parenthetical: "imported meshes where an
implicit field is not available (though perhaps an equivalent could be created after import?)."

## Why this is worth a real design pass

Every production clip caller in `stack.rs` uses `Keep::Predicate` driven by a **field**
(`body_field.scalar_field(p)`, `efield.scalar_field(p) > eiso`). The comments repeatedly justify
this over a mesh nearest-plane test: the field is robust where a wall fit extrapolates past its
constraints near the domain rim, where "the wall MESH and the field disagree" and a nearest-triangle
plane test mis-signs an uncut face (`mesh_one_erode_clipped` ≈ line 786, `mesh_cross_cut_shared`
≈ line 1223). So the field isn't just convenient — it's the ground truth the whole pipeline agrees
against.

An imported mesh has no such field. Tasks 02/03 give it a *geometric* keep predicate
(signed distance to the mesh), which is enough for the corefiner. But several downstream behaviors
still want a scalar field per import:

- `octree` contributors (`ContactSurface`, `ContactHalfSpace`, `SlabComplement`) are all
  field-backed; an imported surface that needs to participate in CSG **solid** contouring (Task 01,
  Strategy A) or be a refinement driver wants a `ScalarField`.
- `fill_normals_from_field` (stack.rs ≈ line 140) reshades corefiner output from a field gradient. An
  imported clip currently has nothing to reshade from (you'd fall back to face-averaged normals).
- The `refine` hook on `clip` (`Option<&dyn ScalarField>`) is the sanctioned way to nudge seam
  vertices onto a zero set — it's a no-op today, but an imported-mesh field proxy is exactly what
  would make it usable.

## The design question to answer

Should an imported mesh get a derived `ScalarField` proxy, and if so which construction? Options to
evaluate in a short `docs/imported_field_proxy.md`:

1. **Signed distance field (SDF) from the mesh** — generalized-winding sign (you already have
   `TriMesh::winding`) × unsigned distance (clamped closest point). Pros: exact on the mesh, gives a
   gradient (the closest-point direction) for normals and `refine`. Cons: distance gradient is not the
   *surface* gradient except near the surface; kinks at the medial axis.
2. **RBF / implicit reconstruction** (Hermite-RBF, à la the cokriging the kernel already does) fit to
   the imported vertices + face normals. Pros: smooth, matches the generated-field character, so
   imported and generated surfaces behave identically downstream. Cons: cost, and it *invents* field
   values away from the surface that may not match the import's intent — the same extrapolation
   problem the comments warn about, just moved.
3. **No proxy; geometric predicates only** (Tasks 02/03), plus face-averaged normals for shading, and
   leave imports out of field-CSG solid contouring (use the mesh-clip volume path from Task 01
   Strategy B exclusively for imports). Pros: honest, no invented field. Cons: imports can't use the
   field-CSG solid path at all.

My recommendation to evaluate first is **3 with an SDF (1) added only where a gradient is actually
needed** (normals, `refine`) — it avoids inventing a global field while still giving local surface
normals. But make the call in the doc with the tradeoffs explicit; this is a real architecture fork.

## Concrete steps (spike only)

1. Write `docs/imported_field_proxy.md` comparing the three, with a recommendation.
2. Implement the *minimal* version your recommendation needs:
   - If SDF: a `MeshSdf { TriMesh }` implementing `ScalarField` (sign from winding, value from
     clamped distance, gradient from the closest-point direction / finite differences). This slots
     into `fill_normals_from_field` and the `refine` hook immediately and is independently testable.
3. Add a test: build a `MeshSdf` from the `octahedron` fixture, assert sign flips across the surface,
   `|value|` ≈ distance, and gradient ≈ outward normal near a face.

## Acceptance

- The design doc exists and is decisive.
- If SDF chosen: `MeshSdf` implements `ScalarField`, has the test above, and is wired into
  `fill_normals_from_field` for the imported-clip output path (so an imported clipped contact shades,
  rather than rendering black as the field-backed comment warns).

## Watch-out

- Do NOT make the corefiner itself depend on any of this — `mesh_clip.rs` stays field-agnostic. A
  field proxy is a *caller-side* convenience and an optional `refine` argument, nothing more.

---

## Amendment — 2026-07-09 (feature 004, imported-mesh surfaces): a smooth far-field IS adopted, deliberately

**Context this doc was written in:** imports as **clip-only boundaries** (a fault/wall that cuts
older elements) with *no units of their own*. In that scope a fabricated global field is pure
downside — it "invents field values away from the surface that may not match the import's intent" —
so Option 2 (RBF/cokriging fit through the mesh) was **declined**, and far-field CSG was left to the
mesh-clip *volume* path, not a fabricated field.

**Feature 004 changes the scope** (see `specs/004-imported-mesh-surfaces/`): an imported mesh can be
**any** surface a fitted one can be (contact / erosion / vein wall / topography), participating in
units and lithology. Full parity **requires** a field that behaves like a fitted surface's *out to
the domain boundary* — a smooth extension beyond where the mesh has triangles. So this feature
**adopts a smooth far-field**, and this amendment records that decision and why it does NOT contradict
the original:

- The smooth field is **confined to BEYOND the mesh's axis-aligned bounds** — precisely where the
  user supplied no geometry. There, a smooth extrapolation is the *honest* choice, and it is
  **identical to how every fitted surface already extrapolates-then-clips** to the box. The original
  objection ("inventing geometry the user didn't intend") does not apply where the user intended
  nothing.
- **Inside** the mesh bounds the exact imported geometry still wins — the fabricated field never
  overrides the user's own triangles.
- The join is a **geometric patch-stitch, NOT a scalar-field blend**: contour the exact `MeshSdf`
  patch (clipped to the mesh AABB) and the smooth cokriged patch (outside AABB → box) **separately**,
  then weld them at the AABB seam with `mesh_clip::clip` (shared by index → watertight). This keeps
  the corefiner field-agnostic (the watch-out above stands) and avoids a composite field that could
  grow a spurious phantom sheet in a transition band.
- The smooth field reuses the existing cokriging `Model::fit` across the existing EUPL-kernel wasm
  boundary (mesh vertices as points, normals as orientations, single interface, `DecimateConfig`
  point-budget) — no new license/wasm seam.

Net: the field proxy is no longer *only* a caller-side convenience for normals/`refine`; for a
full-parity import it also supplies the smooth far-field. The corefiner stays field-agnostic; the
exact-inside rule keeps the user's geometry authoritative. Design of record for the mechanism:
`specs/004-imported-mesh-surfaces/research.md` (D3) and `plan.md`.
