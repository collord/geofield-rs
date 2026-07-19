# Feature Specification: Imported Mesh Surfaces

**Feature Branch**: `004-imported-mesh-surfaces`

**Created**: 2026-07-09

**Status**: Draft

**Input**: User description: "an imported mesh can be any surface in the model where currently a parameterized mesh exists. AND (here's the kicker) we need to be able to project it out to a model boundary if it doesn't extend adequately... exact mesh within the convex hull of the mesh triangles, smooth projection at a sane resolution out from there"

## Context (why this is more than plumbing)

Today every surface in the model is a scalar **field** fitted (by co-kriging) from surface points + orientations, then contoured. The field is defined everywhere, so a fitted surface reaches the domain boundary by raw extrapolation, hard-clipped at the domain box. An **imported mesh** is field-less — just triangles — and only exists where those triangles are. To let an imported mesh stand in for *any* fitted surface, and to make it produce a watertight, domain-spanning model, the imported mesh must acquire the same field-backed nature a fitted surface has: **exact where the user gave geometry, smoothly extended to the domain boundary beyond it.**

The low-level geometry primitives for consuming a field-less triangle sheet already exist and are unit-tested (a mesh boundary trait, per-component winding normalization, a mesh signed-distance proxy, a field-agnostic mesh-clip corefiner). What does **not** exist is: any ingress (no mesh parser, no imported-surface data model), any pipeline call site that treats an imported mesh as a model surface, the exact-inside / smooth-outside projection to the boundary, and the UI. This feature builds those.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Import a mesh and use it as a stratigraphic contact (Priority: P1)

A modeler has a surface they produced elsewhere (a bed contact exported from another tool as `.obj`). They import it into a project, and it becomes a contact surface in the hierarchy — the same kind of object as a fitted bed — participating in the model (rendered as a surface; bounding the units above and below it). Within the region the mesh covers, the model honors the imported triangles **exactly** (no re-fitting, no smoothing of the user's own geometry).

**Why this priority**: This is the ingress and the core promise — "an imported mesh can be any surface where a parameterized one exists." Without it there is no feature. A mesh that already spans the domain is fully usable from this slice alone.

**Independent Test**: Import an `.obj` bed that spans the domain; confirm it appears as a contact element, renders where its triangles are, separates the unit above from the unit below, and the rendered surface coincides with the imported triangles within tolerance.

**Acceptance Scenarios**:

1. **Given** an empty project, **When** the modeler imports an `.obj` mesh, **Then** a new imported-surface element appears in the hierarchy and its triangles render in the viewer.
2. **Given** an imported mesh assigned as a stratigraphic contact between two units, **When** the model is meshed, **Then** the contact separates the two units and, within the mesh's coverage, the derived contact coincides with the imported triangles (deviation below tolerance).
3. **Given** an imported mesh whose triangles have inconsistent winding across components, **When** it is imported, **Then** the model still produces a consistently-wound, closed result (winding is normalized on ingress).

---

### User Story 2 - Project an under-covering mesh out to the domain boundary (Priority: P1)

The imported mesh covers only part of the domain (its triangles stop short of the domain edges). The modeler still expects a complete, watertight model: within the mesh's coverage the surface is the exact imported geometry; beyond the coverage it is **smoothly extended** — following the surface's own trend — out to the domain boundary at a sane (coarse) resolution, with no crack or hard crease at the transition.

**Why this priority**: This is the "kicker" and the reason the feature is non-trivial. Real imported surfaces rarely span the whole domain; without projection the resulting solids have holes and are unusable. Exact-inside / smooth-outside is the explicit requirement.

**Independent Test**: Import a mesh whose triangle extent is strictly inside the domain box; confirm the derived surface (a) reaches and daylights on the domain box faces, (b) is watertight when closed into unit solids, (c) matches the imported triangles within tolerance inside the coverage region, and (d) has no discontinuity/crease at the coverage seam beyond tolerance.

**Acceptance Scenarios**:

1. **Given** an imported mesh strictly inside the domain, **When** it is meshed into a surface/solid, **Then** the surface extends to the domain box faces and the resulting unit solids are watertight (no holes).
2. **Given** the coverage seam between the exact region and the projected region, **When** the surface is inspected across that seam, **Then** it is continuous (no gap, no visible crease beyond tolerance).
3. **Given** an imported mesh that already spans the whole domain, **When** it is meshed, **Then** no projection is applied (the exact mesh is used throughout) and the result is unchanged from User Story 1.
4. **Given** the projected (beyond-coverage) region, **When** its triangle density is inspected, **Then** it is coarse ("sane resolution"), not sub-feature-sized replication of the domain.

---

### User Story 3 - Role parity: erosion, vein wall, and topography (Priority: P2)

The modeler uses an imported mesh not only as a conformable contact but in any role a fitted surface can play: as an erosion surface that truncates older units, as a cross-cutting vein/intrusion wall, or as the topography that caps the model against Air. In each role the imported surface behaves identically to a fitted one — it clips, it derives units, it takes lithology assignments, and it exports grouped by lithology.

**Why this priority**: Full parity is the stated goal, but the conformable-contact path (US1+US2) is the load-bearing slice; the other roles reuse the same field-backed surface and the existing per-role machinery. They are separable increments.

**Independent Test**: For each role (erosion, vein wall, topography), build a scene with an imported mesh in that role and confirm the role's characteristic behavior holds (erosion truncates older units; a vein wall bounds a tabular body; topography caps to Air), and that units/lithology/solid export behave as they do for the fitted equivalent.

**Acceptance Scenarios**:

1. **Given** an imported mesh assigned as an erosion surface, **When** the model is meshed, **Then** older units are truncated at the imported surface exactly as with a fitted erosion.
2. **Given** an imported mesh assigned as topography, **When** the model is meshed, **Then** the model is capped to Air above the imported ground and rock below it.
3. **Given** an imported mesh in any supported role, **When** units are derived, **Then** the unit ladder and lithology assignment behave identically to a fitted surface in that role.

---

### Edge Cases

- **Mesh already spans the domain**: no projection; exact mesh throughout (US2 scenario 3).
- **Mesh entirely or partially outside the domain box**: the portion outside the box is clipped away (same as a fitted surface's box clip); the in-domain portion is honored.
- **Inconsistent winding / multiple connected components**: winding is normalized per component on ingress so the result is consistently wound and closeable.
- **Holes, non-manifold edges, or self-intersections in the imported mesh**: the system must produce a defined, non-crashing result and surface a clear warning rather than emitting silent garbage; the exact behavior (repair vs reject vs best-effort) is bounded in Assumptions/Non-Goals.
- **Degenerate triangles / duplicate vertices**: tolerated (welded/skipped) without corrupting the surface.
- **Coverage bounds = the mesh's bounding box**: the "exact region" is the mesh's axis-aligned bounds; where that box includes area the mesh's triangles do not actually cover (a non-rectangular plan extent), that area is filled by the smooth projection/blend (documented behavior, not a hole).
- **Very large meshes** (hundreds of thousands to millions of triangles): import completes within a stated budget or is coarsened to a working resolution with the coverage still honored; any coarsening bound is logged (Principle IV).
- **Coordinate scale/units mismatch** between the imported file and the model domain: the import lands in model coordinates as-authored; rescaling/repositioning is out of scope for v1 (Non-Goals) unless trivially derivable.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST let a modeler import a triangle mesh from a file into a project as a new model surface element.
- **FR-002**: An imported-surface element MUST be usable in any role a fitted surface can occupy — stratigraphic contact, erosion surface, cross-cutting wall (vein/intrusion), and topography — and MUST participate in clipping, unit derivation, lithology assignment, and solid/OBJ export identically to a fitted surface in that role.
- **FR-003**: Within the imported mesh's axis-aligned bounds, the derived surface MUST coincide with the imported geometry to within a stated tolerance — the user's own geometry is honored exactly, not re-fitted or smoothed.
- **FR-004**: Where the imported mesh does not reach the domain boundary, the system MUST extend the surface smoothly (following the surface's own trend) out to the domain box, so the surface daylights on the box faces and unit solids close watertight.
- **FR-005**: The transition between the exact (in-coverage) region and the projected (beyond-coverage) region MUST be continuous — no gap and no crease beyond a stated tolerance (a blended seam).
- **FR-006**: The projected (beyond-coverage) region MUST be generated at a coarse, bounded resolution ("sane resolution") — it MUST NOT replicate the domain at the imported mesh's native triangle density.
- **FR-007**: When the imported mesh already spans the domain, the system MUST use the exact mesh throughout and apply no projection.
- **FR-008**: The system MUST normalize the imported mesh so it is consistently wound and closeable regardless of the source file's per-component winding.
- **FR-009**: The system MUST clip the imported surface to the domain box exactly as fitted surfaces are clipped (nothing rendered beyond the true extent).
- **FR-010**: The system MUST produce meshing output that satisfies the engine's geometric invariants — crack-free, consistent winding, manifold edges, gradient-coherent normals, flush to the true extent — for imported surfaces just as for fitted ones.
- **FR-011**: For a malformed or unusable imported mesh (holes, non-manifold, self-intersections, empty), the system MUST fail safe with a clear, actionable message rather than emitting silent or corrupt geometry.
- **FR-012**: An imported-surface element MUST round-trip through the project save/load format so a project containing imports reopens intact.
- **FR-013**: The imported-surface ingress MUST be distinct from the existing project-JSON "Import" affordance (which loads a `.geoproj` project), so the two are not confused.
- **FR-014**: The engine paths that construct and mesh an imported surface MUST build for `wasm32` and agree with the native build to `f64` round-off (Principle V), and MUST NOT pull EUPL/GemPy-lineage code into the BSD engine crate.

### Key Entities

- **Imported Surface**: a model element whose geometry comes from an external triangle mesh rather than fitted points/orientations. Attributes: the triangle geometry (vertices + faces), its role (contact / erosion / vein wall / topography), its coverage region (the mesh's axis-aligned bounds), and — derived — a field-backed representation that is exact inside the coverage and smoothly extended outside it. Occupies the same place in the hierarchy as a fitted surface and carries the same one declared relation bit where applicable (erode/onlap/conformable).
- **Coverage Region**: the mesh's axis-aligned bounds (bounding box) — the boundary between "exact imported geometry" and "smooth projection." Drives the exact-vs-projected decision and the seam.
- **Projected Extension**: the coarse, smooth surface generated from the coverage boundary out to the domain box, blended to the exact region at the seam. Not authored by the user; derived.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A modeler can import a mesh file and see it as a model surface without hand-editing any project file.
- **SC-002**: For an imported mesh that spans the domain, the resulting unit solids are watertight (zero holes / zero non-manifold edges) — verified by the engine's geometric invariant checks.
- **SC-003**: For an imported mesh that covers only part of the domain, the resulting unit solids are still watertight and the surface reaches every domain box face it should — no holes introduced by under-coverage.
- **SC-004**: Within the coverage region, the derived surface deviates from the imported triangles by no more than a stated small tolerance (i.e. the user's geometry is preserved, not re-fitted).
- **SC-005**: Across the coverage seam, the surface is continuous — no measurable gap and no crease exceeding a stated angular tolerance.
- **SC-006**: An imported surface used as an erosion / vein wall / topography produces the same characteristic result (truncation / tabular body / Air cap) as the fitted equivalent, verified against the existing per-role invariants.
- **SC-007**: The engine paths for imported surfaces produce native↔wasm bit-parity to `f64` round-off (Principle I/V gate).
- **SC-008**: A project containing an imported surface saves and reopens with the surface and its role intact.
- **SC-009**: UI acceptance (import affordance, element appears, role assignment, render) is covered by an automated functional (Playwright) test that drives the running app (Principle I gate).
- **SC-010**: Importing a moderately large mesh (target: on the order of 100k triangles) completes within an interactive budget, or is coarsened to a working resolution with the coverage still honored and the coarsening bound logged.

## Assumptions

- **Format**: `.obj` is the first supported format (the user's stated example). The parser and data model leave a clean seam for STL/PLY/glTF; those are Non-Goals for v1.
- **Normals**: if the imported file carries vertex normals they are used to orient the surface; otherwise orientation is derived from face winding (after normalization). No separate orientation authoring is required for imports.
- **Coverage = mesh bounds**: the exact region is the mesh's axis-aligned bounds (its bounding box) — simpler than a convex hull and aligned with the engine's existing axis-box extent clip. Where the bounding box includes un-covered area (a non-rectangular plan extent), the smooth projection fills it (documented, not a hole). Tighter coverage (convex hull, or the actual boundary loops) is a possible later refinement, noted as a seam.
- **Projection method (decided)**: the smooth far-field extension is produced by fitting a smooth scalar field through the imported mesh (so beyond-coverage behavior matches how fitted surfaces already extrapolate-then-clip), while the exact imported geometry is used inside the mesh bounds and blended to the fitted field at the seam. This is the field-backed nature that gives full role parity; the exact-inside rule answers the standing objection that a fitted field "invents geometry the user didn't intend" — inside coverage the user's geometry wins.
- **Coordinates**: the imported mesh is interpreted in model coordinates as authored; no auto-rescale/reposition in v1.
- **Existing primitives reused**: the mesh boundary trait, per-component winding normalization, the mesh signed-distance proxy, and the mesh-clip corefiner are reused rather than rebuilt (Principle VI); this feature adds ingress, the field-backed exact/smooth surface, the pipeline call site, and UI.
- **Domain box / extent**: the same true-extent box clip that bounds fitted surfaces bounds imported ones; the projection targets that box.

## Out of Scope / Non-Goals

- **Formats beyond `.obj`** (STL, PLY, glTF): seam left at the parser + data model; not built in v1.
- **Mesh repair** beyond winding normalization and degenerate-triangle tolerance: holes/non-manifold/self-intersecting inputs are *detected and reported* (FR-011), not auto-healed. The repair seam is documented; solving it is a later feature.
- **Coordinate transform on import** (rescale, reproject, reposition, unit conversion): out of scope; imports land as-authored. Seam left at ingress.
- **Tighter coverage boundary**: v1 uses the mesh's axis-aligned bounds; a tighter exact region (convex hull, or the actual concave boundary loops) is a documented refinement seam.
- **Editing imported geometry** in-app (vertex/face editing): imports are used as-is; editing is out of scope.
- **Imported meshes as faults or fault blocks**: only surface roles (contact/erosion/vein wall/topography) in v1; fault-block partition from an imported surface is a seam, not built.
- **Sub-feature-resolution far-field**: the projected region is deliberately coarse; matching the imported mesh's density out to the boundary is a non-goal (FR-006).
