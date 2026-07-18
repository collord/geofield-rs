# Feature Specification: Cross-Sections & Digitized Surfaces

**Feature Branch**: `005-cross-section-digitizing`

**Created**: 2026-07-18

**Status**: Implemented 2026-07-18 — all six stories landed and gated by the five `web3/e2e/section-*.spec.ts` Playwright specs (see tasks.md); `web3/SPEC.md` §10–§11 is the merged source of truth (with the §11.2 pole-formula amendment recorded in contracts/younging-derivation.md).

**Input**: User description: "web3/SPEC_cross_sections.md — proposed §10/§11 extension to web3/SPEC.md: on-demand vertical cross-sections (roadmap B7) and digitizing surfaces as directed polylines on parallel sections (roadmap B8), adapting Leapfrog Geo's serial-sections + polyline-editor + younging-direction workflow."

## Context (why this is a workflow feature, not an engine feature)

Today every surface in the model is authored either by typing contact points and orientations into a table or by importing a mesh. Geologists, however, think and draw in **section**: a vertical slice through the model read face-on, the way a paper section is read. This feature adds that workflow in two layers. First, **cross-sections as views**: a vertical plane defined by two plan-view clicks, opened face-on to show the *traces* where the applied model's surfaces cut the plane — pure inspection, never touching the geology. Second, **sections as drawing surfaces**: the user digitizes an ordered line along a contact in the slice, repeats on several parallel slices, and the app lofts those lines into one ordinary fitted surface — the drawn *direction* of each line supplying the surface's younging (which side is younger) without the user typing any dip numbers.

The deliberate constraint is that digitizing produces **the same kind of data the table already holds**: each drawn vertex is a contact point; each drawn direction yields orientations. The existing fit, stacking, roles, flip semantics, deferred edit-and-apply loop, and validation panel are reused as-is; no new modeling capability is required. A digitized surface is indistinguishable downstream from a typed one.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Create and read a cross-section (Priority: P1)

A user switches to top-down view, clicks two points to set a baseline, and opens the resulting vertical section face-on. The view locks orthographic (baseline direction to the right, up vertical) and shows the traces of the currently-applied model surfaces where they cross the plane, plus any authored contact points lying near the plane, as a backdrop — a readable 2D section drawing of the live model.

**Why this priority**: It is the substrate for everything in this feature and delivers standalone value as an inspection aid: seeing the layering along a chosen line is useful even before any digitizing exists.

**Independent Test**: In a project with an applied model, create one section by two clicks, open it, and confirm the traces match the model along that slice; confirm creating/deleting sections never re-fits the model.

**Acceptance Scenarios**:

1. **Given** a project with an applied model, **When** the user picks two points A→B in top-down view, **Then** a vertical section plane through that baseline, spanning the full model height, is created and appears in the project's section list.
2. **Given** a section, **When** it is opened in section view, **Then** the camera locks orthographic and face-on (baseline direction right, up vertical) and the traces of all currently-applied surfaces are drawn where they intersect the plane.
3. **Given** authored contact points lying within a slab tolerance of the plane, **When** the section is viewed, **Then** those points are shown as a backdrop at their in-plane positions.
4. **Given** pending (un-applied) edits, **When** a section is viewed, **Then** the same stale indicator appears as in the 3D view, and clears on Apply.
5. **Given** the shared render-mode toggle, **When** switched, **Then** the section shows surface traces (contact mode) or filled unit intervals (solid mode), reading the same geology as the 3D view.
6. **Given** any number of sections created, moved, or deleted, **Then** the project is never marked dirty and no re-fit occurs — sections are view constructs, not geology.

---

### User Story 2 - Digitize a contact line with directional younging (Priority: P1)

In a section view, the user picks a target surface (an existing element, or a new one created for this purpose) and traces a line along where they see a contact in the slice: click to add ordered vertices, click to extend, drag to move, delete a vertex. The direction they draw sets which side of the surface is younger, shown live as an arrow along the line with hachures/shading on the younger side; a flip control reverses it without touching the drawn vertices.

**Why this priority**: This is the headline capability — drawing geology instead of typing it — and it is independently valuable on a single section: the drawn vertices immediately become ordinary contact points on the target surface.

**Independent Test**: Digitize a line on one section into a new surface; confirm each vertex appears in that surface's points table at its world position, the younging affordance shows the younger side per the drawn direction, and flipping changes only the younger side, never a vertex.

**Acceptance Scenarios**:

1. **Given** an open section view and a chosen target surface, **When** the user clicks a sequence of points on the plane, **Then** an ordered line is drawn and each vertex becomes a contact point of that surface at its world position on the plane.
2. **Given** an existing digitized line, **When** the user clicks to extend, drags a vertex, or deletes a vertex, **Then** the line and the surface's contact data update accordingly, as deferred geometry edits.
3. **Given** a drawn line, **When** viewed face-on (up vertical, looking along the section's normal), **Then** the younger side is the left of the drawn direction, and the affordance (direction arrow plus hachures/shading on the younger side) shows this live.
4. **Given** the younging affordance shows the wrong side, **When** the user flips the surface, **Then** the younger side reverses everywhere, no drawn vertex changes, and flipping again restores the original — identical semantics to the existing per-surface flip.
5. **Given** digitized vertices, **When** the surface's points table or the 3D points/normals overlay is opened, **Then** the same vertices appear there — the table and the section are two views of one dataset.

---

### User Story 3 - Serial sections, step, digitize across, and loft on Apply (Priority: P1)

From one baseline the user creates a serial set of parallel sections by entering a spacing and a count (optionally front/back offsets). With the digitizing tool active they step next/previous through the slices, drawing the same contact on each. One Apply lofts the collected lines into a single fitted surface that meshes into the stack; back in each section view, the new surface's trace lands on the drawn lines.

**Why this priority**: This is the end-to-end payoff (the fragment's acceptance walk): parallel traces becoming one 3D surface. Without it, digitizing is only a point-entry convenience.

**Independent Test**: Create a serial set of ≥3 sections, digitize the same new surface on each, Apply once, and confirm one new surface appears in the stack whose trace on every digitized section passes through the drawn vertices; confirm stepping preserved the active tool throughout.

**Acceptance Scenarios**:

1. **Given** a baseline, **When** the user enters a spacing and a count (and optional front/back offsets), **Then** that many parallel sections are created at even offsets along the baseline's horizontal normal.
2. **Given** a serial set and an active digitizing tool, **When** the user steps next/previous, **Then** the view moves to the adjacent parallel section without leaving section view and the tool stays active on the same target surface.
3. **Given** lines digitized for one surface on several parallel sections, **When** the user Applies, **Then** the surface is fitted and meshed with the rest of the stack in the same single deferred pass as any geometry edit — no separate loft step is exposed.
4. **Given** the Apply has completed, **When** each digitized section is reopened, **Then** the new surface's trace lies on the drawn line within a tolerance small relative to the section spacing.
5. **Given** un-applied digitized lines, **When** any view is shown, **Then** the stale indicator is present until Apply.

---

### User Story 4 - Digitized lines are ordinary, editable surface data (Priority: P2)

The user reopens a previously digitized surface: moves or deletes vertices on any section, re-digitizes one slice entirely, or adds a line on a newly created section. Every change is a deferred geometry edit that re-fits only on the next Apply. Because the vertices are ordinary contact points, they can equally be inspected or edited from the surface's points table.

**Why this priority**: Editability is what makes digitizing a durable authoring route rather than a one-shot import; it builds directly on Stories 2–3.

**Independent Test**: After Story 3, move a vertex on one slice, delete another, re-digitize a third slice, Apply, and confirm the surface updates accordingly; edit one of the same points from the table and confirm the section view reflects it.

**Acceptance Scenarios**:

1. **Given** an applied digitized surface, **When** the user moves or deletes vertices on any of its sections and Applies, **Then** the surface re-fits with the changed data.
2. **Given** a slice whose line was drawn wrong, **When** the user re-digitizes that slice, **Then** the new line replaces that slice's contribution to the surface.
3. **Given** a new parallel section added to the set, **When** the user digitizes the surface on it and Applies, **Then** the added line participates in the fit.
4. **Given** a digitized vertex edited via the points table, **When** the section is viewed, **Then** the drawn line reflects the edit — one dataset, two authoring routes.

---

### User Story 5 - Digitized surfaces take any role and compose in the stack (Priority: P2)

The user assigns the digitized surface a role (stratigraphic contact, erosion, onlap, vein wall, topography) and a relation (conformable/onlap/erode), and it composes with beds, bodies, and imported meshes under the normal cut rules by its position in the element list — exactly like a typed or imported surface.

**Why this priority**: Full parity is the stated goal, but it reuses the existing per-role machinery wholesale; the load-bearing new work is Stories 1–3.

**Independent Test**: Give a digitized surface the erosion role above older units and confirm it truncates them; repeat with a contact role and confirm normal stacking — matching the behavior of an identically-placed typed surface.

**Acceptance Scenarios**:

1. **Given** a digitized surface, **When** a role and relation are set, **Then** the same role/relation choices available to a typed surface are available, and the surface stacks by its list position.
2. **Given** a digitized erosion surface above older elements, **When** the model is meshed, **Then** it truncates them under the same cut rules as a fitted erosion surface.
3. **Given** any downstream behavior (units, lithology assignment, export), **When** exercised on a digitized surface, **Then** it is indistinguishable from a typed surface with the same points and orientations.

---

### User Story 6 - Orientation in space and persistence (Priority: P3)

The user toggles a section's plane on in the 3D view — a translucent quad with its baseline and viewing normal shown — so the slice's position in space is always legible. Saved views/exported projects persist the project's sections (baselines, spacing, active section) so a reopened project comes back on the same slices.

**Why this priority**: Quality-of-life and continuity; the core workflows function without it.

**Independent Test**: Toggle a plane on and confirm the quad, baseline, and normal render in 3D; export and reopen the project and confirm sections and the active slice are restored.

**Acceptance Scenarios**:

1. **Given** a section, **When** its plane toggle is on, **Then** the 3D view shows a translucent quad spanning the plane with its baseline and normal indicated.
2. **Given** a project with sections (including a serial set) and an active section, **When** the project is exported and reopened, **Then** all sections, their serial parameters, and the active section are restored.

---

### Edge Cases

- **Degenerate baseline**: the two picked points coincide or are nearly coincident — the gesture is rejected (or the second click ignored) rather than creating a zero-width plane.
- **Section that cuts nothing**: a baseline outside the modeled volume, or serial offsets extending past it, still create valid sections; their views simply show an empty backdrop.
- **Deleting a section that carries digitized lines**: the section (a view construct) disappears, but the digitized vertices remain the surface's contact data, still visible and editable via the points table and overlay.
- **Fewer than 2 vertices in a line**: blocks Apply (a single point is not a line).
- **Everything on one plane**: a surface whose entire digitized set lies on a single section plane and has no other data blocks Apply — a single coplanar trace cannot define a bounded 3D surface.
- **Under-constrained but not blocked**: all lines on one section (surface has other data), or on parallel sections with no across-section variation — Apply proceeds with a warning that the surface is effectively extruded/extrapolated off-section and that ≥2 (ideally 3) sections determine it.
- **Inconsistent younging** (the classic digitizing slip): one line's drawn direction implies the opposite younger side from its siblings on the same surface — warn and offer a one-action "flip this line to match"; a mixed set describes a self-contradictory surface.
- **Back-tracking or self-crossing line**: the drawn order is ambiguous for deriving a direction — warn, don't block.
- **Off-plane vertex**: impossible via the plane-locked tool; guarded against on project import so imported data cannot violate the invariant silently.
- **Stepping past the ends of a serial set**: stepping clamps at the first/last section (no wraparound).
- **Same surface digitized on non-parallel sections**: not blocked — the vertices are ordinary contact data — but the guided serial workflow and the loft-fidelity expectation apply to parallel sets only.

## Requirements *(mandatory)*

### Functional Requirements

**Sections (views)**

- **FR-001**: Users MUST be able to create a cross-section by picking two points in top-down view; the section is the vertical plane through that baseline, spanning the full model height.
- **FR-002**: Users MUST be able to create a serial set of parallel sections from one baseline by entering a spacing and a count, with optional front/back offsets bounding the set; sections are evenly offset along the baseline's horizontal normal.
- **FR-003**: Opening a section MUST lock the view orthographic and face-on to the plane, baseline direction to the right and up vertical.
- **FR-004**: A section view MUST show the traces of all currently-applied model surfaces where they intersect the plane, and the authored contact points lying within a slab tolerance of the plane, as a backdrop.
- **FR-005**: A section view MUST carry the same pending-edits stale indicator as the 3D view, appearing when edits are pending and clearing on Apply.
- **FR-006**: A section view MUST share the model render-mode toggle, showing surface traces (contact mode) or filled unit intervals (solid mode).
- **FR-007**: Users MUST be able to step to the next/previous section of a serial set without leaving section view, with any active tool (including digitizing, with its target surface) preserved across the step.
- **FR-008**: Users MUST be able to toggle a section's plane on/off in the 3D view as a translucent quad showing its baseline and viewing normal.
- **FR-009**: Creating, moving, or deleting sections MUST never mark the project dirty and MUST never trigger a re-fit; sections affect only what is drawn. (Digitized content is geometry and does set dirty.)
- **FR-010**: Exported projects MUST persist their sections (baselines, serial spacing/count/offsets, active section) and restore them on reopen.

**Digitizing (authoring)**

- **FR-011**: In a section view, users MUST be able to select a target surface — an existing element or a new one created in place — and digitize an ordered line on the plane with: click to append a vertex, click to extend, drag to move a vertex, and delete a vertex. Drawing is locked to the active section plane.
- **FR-012**: Each digitized vertex MUST become a contact point of the target surface at its world position on the plane, and MUST appear in the surface's points table and in the points/normals overlay — the table and the drawn line are two views of one dataset, editable from either.
- **FR-013**: The drawn direction MUST determine the surface's younging: viewed face-on (up vertical, section normal toward the viewer), the surface youngs to the **left** of the drawn direction; the orientations derived from the line point to the younger side. The convention is fixed, not configurable.
- **FR-014**: While drawing and editing, the section view MUST show a live younging affordance: a direction arrow along the line and hachures/shading on the younger side.
- **FR-015**: The per-surface flip control MUST reverse the younger side of a digitized surface by negating its derived orientations only — never rewriting drawn vertices — reversibly, with the same semantics as the existing flip on typed orientations.
- **FR-016**: Users MUST be able to digitize the same surface on multiple parallel sections; every slice's line contributes to that one surface's data.
- **FR-017**: On Apply, digitized data MUST be fitted and meshed with the rest of the stack in the same single deferred pass as any other geometry edit; the user sees no separate loft step. All digitizing edits (add/move/delete vertex, re-digitize a slice, add a slice) are deferred geometry edits taking effect on the next Apply.
- **FR-018**: A digitized surface MUST accept any role (stratigraphic contact, erosion, onlap, vein wall, topography) and relation (conformable/onlap/erode) available to a typed surface, stack by its list position under the normal cut rules, and be indistinguishable downstream from a typed surface with the same data.

**Validation (extends the existing pre-Apply validation)**

- **FR-019**: The validation panel MUST block Apply, stating the first blocking reason, when: a digitized line has fewer than 2 vertices; a surface's entire digitized set lies on a single section plane and the surface has no other data; or (guarded at project import) a digitized vertex does not lie on its section plane.
- **FR-020**: The validation panel MUST warn without blocking when: a surface's digitized lines are under-constrained (all on one section, or on parallel sections with no across-section variation — advising that ≥2, ideally 3, sections determine the surface); a line's drawn direction implies the opposite younger side from its sibling lines on the same surface (offering a one-action "flip this line to match"); or a line back-tracks or self-crosses on its plane (ambiguous direction).

### Key Entities

- **Section**: a vertical plane through the model defined by a plan-view baseline A→B; carries a derived in-plane horizontal axis (along the baseline), an in-plane vertical axis (up), and a horizontal viewing normal. A view/authoring construct, never geology.
- **Serial section set**: one baseline plus spacing, count, and optional front/back offsets, yielding parallel sections ordered along the normal; has a notion of the active (currently open) section.
- **Digitized line**: an ordered chain of vertices drawn on one section for one target surface, with a drawn direction; the unit of digitizing edits (extend, move, delete, re-digitize) and of the younging affordance and per-line consistency checks.
- **Contact point / orientation (existing)**: the data a digitized line decomposes into — one contact point per vertex, orientations pointing to the younger side derived from the drawn direction and the section's normal. Identical in kind to table-entered data.
- **Target surface element (existing)**: the surface the lines belong to; holds role, relation, list position, and the flip state; may mix digitized and table-entered data.
- **Saved view / project export (existing, extended)**: persists sections and serial parameters alongside what it saves today.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user goes from top-down view to reading a face-on section of the applied model in at most four interactions (two baseline picks, at most two more to open it).
- **SC-002**: End to end, a user digitizes one contact on three parallel sections and, after a single Apply, one new surface appears in the stack whose trace on every digitized section passes through the drawn vertices within a tolerance small relative to the section spacing.
- **SC-003**: In 100% of digitized lines, the younger side matches the fixed drawn-direction convention, and flipping a surface changes only the younger side — zero drawn vertices altered by any number of flips.
- **SC-004**: Creating, moving, stepping through, or deleting any number of sections causes zero re-fits and leaves the project clean (not dirty).
- **SC-005**: An exported project reopens with 100% of its sections, serial parameters, and active slice restored.
- **SC-006**: For every role a typed surface can hold, a digitized surface with equivalent data exhibits the same stacking/cutting behavior — no test distinguishes them downstream of the fit.
- **SC-007**: Every enumerated hard validation case blocks Apply with a stated reason, and every soft case produces its warning without blocking, in 100% of the enumerated cases.

## Assumptions

- The existing edit-and-apply loop (deferred geometry edits, single fit pass on Apply, stale indicator), per-surface flip semantics, points table, points/normals overlay, render modes, and role/relation stacking are reused as-is; the feature adds no new modeling capability and assumes no engine change is required.
- Whether orientations are derived per vertex or per line segment is a design detail deferred to planning; either satisfies FR-013/FR-015.
- Digitized vertices are surface data independent of their section's continued existence: deleting a section never deletes geometry (consistent with FR-009's "sections are not geology").
- Sections are vertical planes in the model's East/North/Up frame; "up" in a section view is world up.
- The younging convention ("youngs to the left of the drawn direction, facing the section") is fixed; users correct mistakes via the visual affordance and flip, not by configuring the convention.
- Stepping through a serial set clamps at its ends.
- A single section constrains only the in-plane component of a surface's attitude; recovering the full 3D attitude requires multiple parallel sections — the basis for the under-constrained warning (FR-020).

## Out of Scope / Non-Goals

- **Tilted, folded, or fence-polyline sections** — sections are vertical planes from a straight baseline only; the seam for richer section geometry is the section definition itself.
- **Free 3D drawing** on surfaces or in space — digitizing is plane-locked to the active section.
- **Auto-tracing** from images, gridded data, or drillhole intercepts — there is no drillhole data model yet (roadmap B11); every digitized line is authored by hand.
- **Mesh/triangle editing** — digitizing authors *input data* for the fit; it never edits the fitted surface's triangles.
- **A dedicated engine-side loft** — lofting is simply the existing fit consuming digitized points and orientations; an engine-side derivation helper is an optional later refactor, not part of this feature.
- **Guided workflows for non-parallel sections** — digitizing one surface from non-parallel sections is not blocked (the data is ordinary contact data), but no consistency tooling or loft-fidelity claim covers it.
