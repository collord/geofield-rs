# Feature Specification: Enhanced Authoring for the Stack Testbed

**Feature Branch**: `002-enhanced-authoring`

**Created**: 2026-06-19

**Status**: Draft

**Input**: User description: "read docs/enhanced_authoring_spec.md" — turn the `web3/` stratigraphy testbed from a three-scenario viewer into a generic stack authoring tool: an editable in-memory project document that compiles to the existing fit input and re-fits on an explicit Apply; an Add-Surface→pick-type flow; a per-surface dual-table data editor (points + orientations, hand entry + paste); live container reordering; a stale-state scrim over the 3D view; and export-to-OBJ. Canned scenarios survive as seed templates. Faults and cross-container surface moves are out of scope.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Author a new surface from scratch (Priority: P1)

A user wants to build a stratigraphic model by adding their own surfaces rather than picking a pre-canned scenario. They click "Add Surface", choose a surface *type* (conformable, erosional, onlap, vein, or intrusion) from a short menu of plain-language choices, and a data editor opens for the new surface. They enter or paste the contact points (and optionally orientation readings) that define where the surface sits, confirm the editor, then press a top-level **Apply** to re-fit the model and see the surface appear in the 3D view and the model summary.

**Why this priority**: This is the core of the feature — without it, the testbed is still just a viewer. It is the single capability that converts the three model concepts (conformable series, unconformities, cross-cutting bodies) from canned to authorable. Every other story builds on having an editable surface.

**Independent Test**: Starting from a blank or seeded project, add one surface of each type, give it valid points, Apply, and confirm the fit completes and the surface is present in the rendered model and the derived summary. Delivers a complete author→fit→view loop on its own.

**Acceptance Scenarios**:

1. **Given** a project open in the testbed, **When** the user clicks "Add Surface" and picks "Erosional", **Then** a new single-surface element is created with that relation and its data editor opens immediately.
2. **Given** the data editor is open for a new surface with zero points, **When** the user pastes a block of tab- or comma-separated XYZ rows, **Then** the points table fills with one row per pasted line and the editor reports the point count.
3. **Given** a surface with at least one point, **When** the user confirms the data editor, **Then** the change is committed to the project, the editor closes, and the model is marked as having unapplied changes.
4. **Given** a project with unapplied changes, **When** the user presses Apply, **Then** the model re-fits, the new surface appears in the 3D view and the derived summary, and the unapplied-changes state clears.
5. **Given** the user picks "Conformable" while a conformable series sits at the insert position, **When** the surface is added, **Then** it joins that existing series (sharing its field) rather than starting a separate one; picking "Conformable" with no adjacent conformable series starts a new one.

---

### User Story 2 - Edit and re-apply an existing surface's data (Priority: P1)

A user wants to correct or extend the data behind a surface that already exists (a seeded one or one they authored). They open the surface's "…" menu, choose "Edit data…", and the dual-table editor opens pre-filled with that surface's current points and orientations. They add, edit, paste, or delete rows in either table, confirm the editor, and — possibly after editing several surfaces — press the top-level Apply once to re-fit everything at once.

**Why this priority**: Authoring is iterative; the first points entered are rarely the last. Editing is as essential as adding, and the two-level Apply (per-surface commit, then one model-wide re-fit) is the interaction model that makes editing several surfaces before paying for one re-fit possible. Tied P1 with Story 1.

**Independent Test**: Open the data editor on a seeded surface, change its point set, confirm, Apply, and verify the rendered/derived model reflects the new data. Confirm that Cancel in the editor discards edits without changing the model.

**Acceptance Scenarios**:

1. **Given** an existing surface, **When** the user opens "Edit data…", **Then** the editor shows that surface's current points and orientations.
2. **Given** edits made in the editor, **When** the user presses Cancel, **Then** none of the edits are committed and the project is unchanged.
3. **Given** edits made in the editor, **When** the user confirms, **Then** the edits are committed to the project and the model shows unapplied changes, but no re-fit happens until the top-level Apply.
4. **Given** two surfaces edited in turn, **When** the user presses the top-level Apply once, **Then** both edits are reflected in a single re-fit.
5. **Given** orientation rows entered as dip and dip-direction, **When** the model is re-fit, **Then** the orientations are honored by the fit (the surface tilts as authored) and the raw dip/dip-direction values are preserved if the editor is reopened.

---

### User Story 3 - See clearly when the view is stale (Priority: P2)

A user has made a geometry-affecting edit but not yet applied it. They need an unmistakable signal that the 3D view no longer reflects the current project. A translucent grey scrim blends over the 3D view while changes are unapplied; the view can still be orbited but reads as stale. Pressing Apply (on a successful re-fit) clears the scrim; pressing Cancel reverts the edits and also clears it.

**Why this priority**: Without this, a user can stare at a stale render believing it shows their latest edits — a correctness-of-understanding bug, not just a polish issue. It is P2 because authoring works without it, but the authoring loop is misleading without it.

**Independent Test**: Make a geometry edit, observe the scrim and unapplied-changes affordances appear; Apply and observe the scrim clear; in a separate run, make an edit then Cancel and observe the scrim clear with the model reverted.

**Acceptance Scenarios**:

1. **Given** a project with no unapplied changes, **When** nothing has been edited, **Then** no scrim is shown.
2. **Given** a geometry-affecting edit is committed, **When** the change is pending, **Then** the scrim overlays the 3D view and the Apply/Cancel affordances are shown.
3. **Given** the scrim is shown, **When** the user successfully Applies, **Then** the scrim clears and the view reflects the new fit.
4. **Given** the scrim is shown, **When** the user orbits the camera, **Then** the camera still responds (the scrim does not block interaction).
5. **Given** a cheap declared-state edit (changing a boundary's erode/onlap/conformable relation or its floor/unbounded flag), **When** the edit is made, **Then** the model re-fits immediately and no scrim appears (these edits are never stale).

---

### User Story 4 - Reorder the chronology of the stack (Priority: P2)

A user wants to change which surfaces are younger or older — the chronological order that drives cross-cutting and truncation. They drag a container to a new position in the hierarchy; the order updates live in the panel and the model is marked as having unapplied changes; the next Apply re-fits with the new chronology. Within a multi-surface conformable series, the user can likewise reorder the surfaces (their isovalue order within one field).

**Why this priority**: Chronology is a first-class authoring concept — the same surfaces in a different order are a different geology. It is P2 because the affordances and the resolver already exist; this story is wiring them to the project model and the Apply loop.

**Independent Test**: Reorder two containers, observe the panel order change and unapplied-changes appear; Apply and verify the derived order / effective relations change to match, mirroring the existing round-trip test pattern.

**Acceptance Scenarios**:

1. **Given** a stack with two or more containers, **When** the user reorders them, **Then** the panel reflects the new order and the model shows unapplied changes.
2. **Given** a reorder is pending, **When** the user Applies, **Then** the re-fit reflects the new chronology in the derived summary (e.g., effective relations / order change as expected).
3. **Given** a multi-surface conformable series, **When** the user reorders its surfaces, **Then** their within-series order changes and the change applies on the next Apply.
4. **Given** the user attempts to move a surface from one container into another, **When** they do so, **Then** the action is declined with a clear message that this is not supported and the workaround is to delete and re-add (a documented limitation).

---

### User Story 5 - Persist and exchange the authored model (Priority: P2)

A user wants their work to survive a reload and to be shareable. The project autosaves locally as they commit edits and is restored on reload. They can export the whole project to a project file and import one back (with a confirm if there are unapplied changes), and they can seed a fresh project from one of the three former canned scenarios or a blank template.

**Why this priority**: Authoring that loses work on reload is not usable for anything but a demo click-through. Persistence and import/export make the authored model durable and portable. P2 because the core author→fit→view loop works in a single session without it.

**Independent Test**: Author a model, reload the page, and confirm it is restored. Export the project, start fresh, import it, and confirm the model matches. Seed from each template and confirm a valid starting project.

**Acceptance Scenarios**:

1. **Given** committed edits, **When** the page is reloaded, **Then** the project is restored from local storage as it was last committed.
2. **Given** an authored project, **When** the user exports it, **Then** a project file is produced that can later be imported to reproduce the project.
3. **Given** unapplied changes, **When** the user imports a project, **Then** the user is asked to confirm before the current project is replaced.
4. **Given** a fresh start, **When** the user seeds from one of the three template scenarios or the blank template, **Then** a valid project is loaded as the starting point.

---

### User Story 6 - Export the rendered model to OBJ (Priority: P3)

A user wants to take the geometry they have built out of the tool. With a successfully fit model and no unapplied changes, they choose Export → OBJ and the browser downloads a single OBJ file containing the rendered surfaces, grouped per surface, that opens in standard 3D tools.

**Why this priority**: This closes the loop from authored data to a portable artifact, but it is downstream of authoring itself and not needed to validate the authoring model. P3.

**Independent Test**: Fit a model, Export → OBJ, and confirm a download is produced whose content has a non-trivial number of vertices and faces.

**Acceptance Scenarios**:

1. **Given** a successfully fit model with no unapplied changes, **When** the user chooses Export → OBJ, **Then** the browser downloads an OBJ file.
2. **Given** the exported OBJ, **When** it is parsed, **Then** it contains a non-trivial count of vertices and faces and groups the geometry per surface.
3. **Given** a model with unapplied changes (stale view), **When** the user looks at the export action, **Then** the OBJ export is unavailable until the model is applied (you export what is rendered).

---

### Edge Cases

- **Surface with zero points**: a newly added surface starts with no points and is hard-invalid; the model-wide Apply is blocked with a reason until at least one point exists.
- **Surface with 1–2 points**: valid enough to apply but advisory-flagged ("needs ≥3 points to fit well") since a plane is underdetermined.
- **Series with no orientation anywhere**: advisory-flagged ("fit may be ambiguous") but still applicable — the fit can attempt it.
- **Pasted rows with the wrong column count**: extra or short rows are flagged to the user, not silently dropped or truncated.
- **Cross-cutting body missing one of its required parts** (e.g., a vein with only one of its two bounding surfaces): hard-invalid, Apply blocked with a reason.
- **Singleton presentation**: a series with exactly one surface is shown flat as a plain surface; once it has more than one, it is shown as a nested series with a header. A cross-cutting body always shows its named parts.
- **Apply pressed while invalid**: the Apply control is disabled and states why (which surface/container is invalid).
- **Cancel after several edits**: reverts the project to the last applied state, clearing all pending edits and the stale scrim.
- **Wrong-sign orientation**: dip/dip-direction is converted to the fit's internal orientation representation in exactly one place, documented with its axis convention, because a sign error there silently produces an upside-down surface.

## Requirements *(mandatory)*

### Functional Requirements

**Project model & compilation**

- **FR-001**: The testbed MUST maintain a single editable in-memory project as the source of truth for authoring, consisting of named containers (conformable series, veins, intrusions) in youngest-first chronological order, each holding per-surface contact-point and orientation data, plus the model's spatial extent and a project name.
- **FR-002**: The project MUST compile to the existing fit input consumed by the model core, with no change to the core fit path, the worker boundary, or the existing model ABI/schema. The authoring model is the only new layer.
- **FR-003**: The system MUST present a series holding exactly one surface as a single flat surface (hiding the container), and a series holding more than one surface as a nested group under a header. Veins and intrusions MUST always present their named constituent surfaces.

**Adding surfaces**

- **FR-004**: Users MUST be able to add a surface by choosing one of five plain-language types — conformable, erosional, onlap, vein, intrusion — without ever naming or choosing a "container."
- **FR-005**: The chosen type MUST determine container membership automatically: conformable joins the adjacent conformable series if one is at the insert position (else starts a new one); erosional and onlap each start a new single-surface series with the corresponding relation; vein and intrusion each start a new cross-cutting container of that kind.
- **FR-006**: On adding a surface, the system MUST create it with one empty surface (or the required pair/envelope for a cross-cutting body) under a sensible default name, insert it at the oldest end of the stack, mark the model as having unapplied changes, and open its data editor immediately.

**Editing surface data**

- **FR-007**: Users MUST be able to edit a surface's data in a dual-table editor: a required points table (X, Y, Z) and an optional orientations table (X, Y, Z, dip, dip-direction), each supporting per-row add and delete and a live row/validity summary.
- **FR-008**: Users MUST be able to enter table data by hand (per-cell numeric entry with keyboard navigation) and by pasting tab- or comma-separated rows mapped to each table's columns in order; rows with the wrong column count MUST be flagged rather than silently dropped or truncated.
- **FR-009**: The data editor MUST commit its edits to the project only on the editor's own confirm (which closes it and marks the model dirty) and MUST discard them on cancel; the editor itself MUST NOT trigger a re-fit.
- **FR-010**: Orientations MUST be authored as dip and dip-direction and converted to the fit's orientation representation in exactly one place at compile time, with the axis convention documented at that point; the raw dip and dip-direction values MUST be preserved so the editor round-trips what the user typed.

**Apply / Cancel loop**

- **FR-011**: Geometry-affecting edits — committing data-editor changes, adding or deleting a surface, and reordering containers or within-series surfaces — MUST mark the model as having unapplied changes and MUST NOT re-fit until an explicit model-wide Apply.
- **FR-012**: The model-wide Apply MUST re-fit from the compiled project, and on a successful fit MUST clear the unapplied-changes state; Cancel MUST revert the project to the last applied state and clear the unapplied-changes state.
- **FR-013**: Cheap declared-state edits — a boundary's erode/onlap/conformable relation and its floor/unbounded flag — MUST keep their immediate live re-fit and MUST NOT set the unapplied-changes state.

**Stale-state signal**

- **FR-014**: While the model has unapplied changes, the system MUST overlay a translucent grey scrim over the 3D view that signals staleness without blocking camera interaction, and MUST remove it when the unapplied-changes state clears (via Apply or Cancel).

**Reordering**

- **FR-015**: Users MUST be able to reorder containers (changing chronology) and reorder surfaces within a multi-surface conformable series; both MUST mark the model dirty and apply on the next Apply.
- **FR-016**: Moving a surface from one container into another MUST be declined with a clear message stating it is unsupported and that the workaround is delete-and-re-add (a documented limitation, not a silent no-op).

**Validation**

- **FR-017**: The system MUST compute author-time validation on the project before fit and surface it both per surface/container and aggregated on the Apply control. Hard-invalid conditions — a series with zero surfaces, a surface with zero points, or a cross-cutting body missing a required part — MUST block Apply with a stated reason. Soft conditions — fewer than 3 points on a surface, or a series with no orientation anywhere — MUST be shown as advisories without blocking.
- **FR-018**: The existing derived per-container badges and summary (sourced from the fit result) MUST be preserved unchanged.

**Persistence & templates**

- **FR-019**: The system MUST autosave the project to local storage on committed edits and restore it on reload.
- **FR-020**: Users MUST be able to export the whole project to a project file and import one back, with a confirmation prompt if there are unapplied changes when importing.
- **FR-021**: The three former canned scenarios MUST be re-exposed as named seed templates that produce a starting project, alongside a blank template; they MUST NOT be the auto-loaded default flow.

**Export**

- **FR-022**: Users MUST be able to export the rendered model to a single OBJ file, grouped per surface, available only when a fit has produced surfaces and there are no unapplied changes (export reflects what is rendered). Export MUST be purely client-side.

**Reusable boundary**

- **FR-023**: All new authoring UI MUST stay inside the reusable hierarchy-panel boundary so it can be lifted into the product UI later, except the OBJ serialization helper which lives alongside the testbed's other view-level code.

### Key Entities *(include if feature involves data)*

- **Project**: the editable source of truth for authoring. Has a version, a name, a spatial extent, and an ordered (youngest-first) list of containers. Persists to local storage and to an exportable project file.
- **Container (Project Element)**: one of — a *conformable series* (a named, ordered group of one or more surfaces sharing one fitted field, plus its boundary relation and floor/unbounded flag); a *vein* (a named cross-cutting body defined by two bounding surfaces); or an *intrusion* (a named cross-cutting body defined by an inner and outer envelope, equal when a single envelope). A series with one surface is presented flat.
- **Surface (Interface Data)**: one surface's interpolation data — a stable id, a name, a set of contact points, and an optional set of orientation readings. The unit a user adds and edits.
- **Point**: a single XYZ contact location on a surface.
- **Orientation**: a single reading — an XYZ location plus a dip and dip-direction — authored in geological convention and converted to the fit's internal orientation at compile time.
- **Validation result**: the per-surface, per-container, and aggregate author-time assessment (hard-invalid with reason vs. soft advisory) that gates the Apply control.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can author a complete model of each of the five surface types — add, enter or paste data, apply, and see it rendered — without ever selecting a canned scenario and without editing any file by hand.
- **SC-002**: Pasting a block of contact-point rows from a spreadsheet produces one surface point per row, with mismatched rows reported, in a single paste action (no per-row retyping).
- **SC-003**: A user editing several surfaces pays for exactly one re-fit by pressing Apply once after all edits, rather than one re-fit per edit.
- **SC-004**: Whenever the rendered view does not reflect the current project, the staleness is visible to the user 100% of the time (the scrim is present for every pending geometry edit and absent otherwise), with no false "stale" after a successful Apply.
- **SC-005**: Reordering the chronology and applying changes the model's derived order/relations in the way a domain user expects, demonstrated by an automated round-trip.
- **SC-006**: An authored project survives a page reload and a round-trip through export then import, reproducing the same model both times.
- **SC-007**: An exported OBJ opens in a standard 3D viewer and contains the rendered surfaces (non-trivial vertex and face counts, grouped per surface).
- **SC-008**: Every new behavioral acceptance criterion above is backed by an automated functional test that drives the running app (per the project's UI behavior gate), not by a manual walk-through.
- **SC-009**: An invalid project (a surface with no points, or a cross-cutting body missing a part) cannot be applied, and the reason is shown to the user at the point of the blocked Apply.

## Assumptions

- This work targets the in-repo `web3/` testbed (Playwright-gated), not the shipping product UI; authoring components nonetheless stay inside the reusable hierarchy-panel boundary so they lift into the product later.
- This is a delta against the current `web3/` baseline (hierarchy panel, drag resolver, DTOs, store, worker, scenarios, protocol). The fit core, worker boundary, and model ABI/schema are unchanged — no kernel, engine, worker, or schema changes this iteration.
- The drag-reorder affordances and the reorder resolver already exist and are correct; this feature wires their output into the project model and the Apply loop rather than building new drag mechanics.
- The three existing canned scenarios are the basis of the seed templates.
- Local-storage autosave is debounced (~half a second) and keyed by project name; the project file is the demo's "save file."
- The fit's existing hard-terminate-on-new-run cancellation behavior is retained.
- Standard axis convention is X=East, Y=North, Z=Up, with dip-direction measured clockwise from North; this is stated to the user where orientations are entered.
- DTOs remain hand-mirrored (unchanged from baseline); the project model is UI-only and does not cross the wasm boundary, so no schema auto-generation is introduced.

## Out of Scope / Non-Goals

- **Faults / block model** — per-series fault fields keep their no-fault defaults; no authoring UI for faults. The seam is the existing per-series fault fields, left at their defaults; the fault partition is a later feature.
- **Cross-container surface moves** — punted; the seam is the move callback, kept as a declined action with a documented delete-and-re-add workaround (FR-016), not a silent no-op.
- **Insert-at-selection on Add** — v1 appends at the oldest end and relies on reorder; the seam is the insert position, fixed to "append" for now.
- **In-modal 3D preview** — the main 3D view updates on Apply; there is no live preview inside the data editor.
- **CSV/spreadsheet file import** — clipboard/TSV-or-CSV paste only for tabular data; the JSON project file is the only file-import path.
- **Schema auto-generation of DTOs** — DTOs stay hand-mirrored; the authoring model does not cross the wasm boundary.
- **Materials in the OBJ export** — geometry and per-surface groups first; a materials companion file is optional and may be omitted in v1 (a logged coverage bound, not a silent omission).
