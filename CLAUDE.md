<!-- SPECKIT START -->
For technologies, project structure, shell commands, and other context, read the
current plan: `specs/004-imported-mesh-surfaces/plan.md` (feature: imported mesh
surfaces — an imported `.obj` triangle mesh can be ANY surface a fitted one can be
(contact/erosion/vein wall/topography), honoring the mesh EXACTLY inside its
axis-aligned bounds and extending it SMOOTHLY to the domain box beyond them via a
watertight seam. Resolved approach: reuse the co-kriging `Model::fit` (mesh vertices
as points, normals as orientations, single interface) across the existing EUPL-kernel
wasm boundary with the existing `DecimateConfig` point-budget; use the built `MeshSdf`
as the exact-inside field; and PATCH-STITCH the exact patch (clipped to the mesh AABB)
to the smooth kriged patch (outside AABB → box) with the `mesh_clip::clip` corefiner —
NOT a scalar-field blend (avoids spurious phantom sheets). New: a ~50-line byte OBJ
parser (`geofield/src/obj.rs`), a `StackElement::ImportedSurface` variant along the
usual element-plumbing chain, and a distinct `.obj` import UI. Deliberately amends
`docs/07_imported_mesh_field_proxy.md` (full-parity imports adopt a smooth far-field,
confined to beyond the user's geometry). The prior feature is
`specs/003-topography-lithology-solids/plan.md` (source of record
`docs/topography_lithology_solids_spec.md`); before it,
`specs/002-enhanced-authoring/plan.md` and `specs/001-stratigraphic-hierarchy/plan.md`.
The project constitution is `.specify/memory/constitution.md`.
<!-- SPECKIT END -->
