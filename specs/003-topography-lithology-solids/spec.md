# Feature Specification: Topography, Lithology Catalog & Solid Volumes

**Feature Branch**: `003-topography-lithology-solids`

**Created**: 2026-06-20

**Status**: Draft

**Input**: User description: "take as a new spec docs/topography_lithology_solids_spec.md"

## Overview

This feature turns the geological surface model into a **named-volume model**. Today the authoring tool keys off *interfaces* (contacts/surfaces) and colours output by series index; "open sky vs. rock" is decided by a per-series flag and the model's output is a set of open contact sheets with no notion of rock type. This feature inverts the presentation to key off the **units** the surfaces bound, adds a dedicated **Topography** surface that caps the model against air, makes filling-upward the universal default (retiring the per-series flag behaviour-first), introduces a **lithology catalog** that names and colours rock types, lets the user **assign a lithology to each derived volume**, and produces **watertight per-volume solids** coloured by lithology — all toggleable against the existing surface view, with no change to the underlying field fit.

The honest conceptual core: a conformable series with N contacts genuinely defines N+1 units, and contacts cannot name those units (a contact is *between* two units). So the model never asserts a unit count — it labels whatever volumes the field yields, naming the **units**, never the contacts.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Author topography and see the model capped at the ground (Priority: P1)

A geologist adds a Topography surface to a model that previously filled the entire domain box. The space above the ground surface becomes air (no rock), every older series is clipped to below the ground, and exposed series no longer fill to the artificial ceiling of the domain box.

**Why this priority**: This is the foundational shift the whole feature rests on — it supplies the surface that defines "open sky vs. rock" and makes fill-upward the universal default. Without it, units above the ground are meaningless and solids cannot be bounded against air. Every other story builds on a model that is correctly capped.

**Independent Test**: Load a scenario with at least one filling-upward series, add a Topography surface via the type picker, and confirm: (a) the region above the ground surface evaluates to air / no rock, (b) older series are truncated at the ground, (c) the model no longer fills to the domain-box ceiling, and (d) a second Topography cannot be added.

**Acceptance Scenarios**:

1. **Given** a model with no topography, **When** the user adds a Topography surface and authors its points, **Then** the volume above the ground surface contains no rock (air) and all older elements are clipped to below it.
2. **Given** a model that already has a Topography surface, **When** the user opens the add-surface type picker, **Then** the Topography choice is disabled or hidden so a second cannot be added.
3. **Given** a Topography surface exists, **When** the user attempts to reorder it below another element, **Then** the reorder is refused and topography remains the youngest element.
4. **Given** a series that previously relied on the domain-box ceiling to bound its top, **When** topography is present above it, **Then** the series fills upward to the ground surface rather than to the box ceiling.

---

### User Story 2 - See the model as named units instead of contacts (Priority: P1)

The authoring panel presents an alternating ladder of **units** and **contacts**, youngest at top. Each unit row is nameable; contact rows keep today's point/orientation entry but have no name. The user adds and removes **contacts** (interfaces); the units re-derive automatically from the geometry.

**Why this priority**: This is the presentation inversion that makes the rest legible — assigning a lithology, naming a formation, and reading a solids view all depend on the user seeing and addressing **units**. It is independently demonstrable as soon as the core exposes per-element unit coordinates, before any lithology work.

**Independent Test**: Load a conformable series with N contacts and confirm the panel renders N+1 unit rows interleaved with N contact rows, youngest at top; that a unit row can be renamed while a contact row cannot; that adding a contact increases the unit count and removing one decreases it; and that the per-series "fills upward" checkbox is no longer present.

**Acceptance Scenarios**:

1. **Given** a conformable series with N contacts, **When** the panel renders, **Then** it shows an alternating ladder of N+1 unit rows and N contact rows, youngest at top.
2. **Given** a unit row, **When** the user opens its actions, **Then** rename is available; **Given** a contact row, **When** the user opens its actions, **Then** rename is not offered (only the existing point/orientation modal).
3. **Given** a series, **When** the user adds or removes a contact, **Then** the number of derived unit rows changes accordingly (N contacts → N+1 units, minus any clipping) without the user ever directly adding or removing a unit.
4. **Given** the panel as redesigned, **When** the user looks for the per-series "fills upward (floor)" control, **Then** it is absent.

---

### User Story 3 - Build and apply a lithology catalog (Priority: P2)

The user manages a catalog of lithologies (named, coloured rock types). Some entries are fixed (Air, Unknown), some are inferred automatically from cross-cutting bodies (a vein named "Vein A" contributes a lithology "Vein A"), and some are user-added. Each derived unit gets a lithology from a dropdown; multiple units may point at the same catalog entry. The catalog round-trips through scenario export/import.

**Why this priority**: The catalog is the identity layer that makes volumes *named* rather than just *bounded*. It depends on units being visible (Story 2) and is the input to lithology-coloured solids (Story 4), so it sits between them.

**Independent Test**: With a model that has at least one vein and one multi-unit series, confirm: Air and Unknown appear as fixed, non-deletable entries; the vein contributes an inferred entry that renames with the body and is removed when the body is deleted; the user can add/rename/recolour/delete a user entry; a unit's dropdown lists all entries and assigning two separate units to the same entry is allowed; and exporting then re-importing the scenario preserves names, colours, and assignments.

**Acceptance Scenarios**:

1. **Given** a fresh catalog, **When** the user inspects it, **Then** Air and Unknown are present as fixed entries (non-deletable, non-renameable) and below-topography units default to Unknown.
2. **Given** a vein named "Vein A", **When** the catalog is shown, **Then** an inferred lithology "Vein A" exists and is auto-assigned to that vein's interior; **When** the body is renamed, **Then** the inferred lithology re-syncs; **When** the body is deleted, **Then** the inferred lithology is removed.
3. **Given** the catalog UI, **When** the user adds a lithology with a name and colour, **Then** it becomes available in every unit row's lithology dropdown and can be renamed, recoloured, and deleted.
4. **Given** two separate units (e.g. two sandstone slabs), **When** the user points both dropdowns at the same catalog entry, **Then** both are assigned to it (many-to-one), with no automatic correlation performed.
5. **Given** assignments and a customised catalog, **When** the user exports the scenario and re-imports it, **Then** all lithology names, colours, and per-unit assignments are restored.

---

### User Story 4 - Toggle to watertight solids coloured by lithology (Priority: P2)

The user flips a project-bar toggle from Surfaces to Solids. The view switches from open contact sheets (coloured by series) to closed, watertight per-volume solids (coloured by lithology), with air omitted. Exporting in solid mode groups the geometry by lithology.

**Why this priority**: This is the payoff — visible, watertight, lithology-labelled volumes — but it depends on every prior story (capped model, derived units, catalog/assignment). It is the demonstrable end state, so it is the last priority while still being core to v1.

**Independent Test**: With a capped model that has assigned lithologies, toggle to Solids and confirm: solids are closed/watertight, coloured from the catalog's colours, with no solid emitted for air; toggling back to Surfaces restores the series-coloured contact sheets; and an OBJ export in solid mode groups geometry by lithology. No re-fit of the field occurs on toggle.

**Acceptance Scenarios**:

1. **Given** a capped model with assigned lithologies, **When** the user toggles to Solids, **Then** each below-topography volume renders as a closed solid coloured by its assigned lithology, and no solid is emitted for air.
2. **Given** the Solids view, **When** the user toggles back to Surfaces, **Then** the open contact sheets coloured by series are restored.
3. **Given** either mode, **When** the user toggles, **Then** the underlying field fit is not recomputed (same fitted stack, different mesh extraction).
4. **Given** the Solids view, **When** the user exports to OBJ, **Then** the exported geometry is grouped by lithology.
5. **Given** a unit left at the Unknown default, **When** the user views Solids, **Then** it is meshed as a solid (Unknown is rock of unspecified type, not void), unlike air.

### Edge Cases

- **Multi-clip volume**: a unit capped by topography above, bounded by its floor contact below, and punched through by an intrusion — must remain watertight with no holes. This is the explicit watertightness test case.
- **Exposed vs. buried tops**: where topography cuts below a series' top, that top unit becomes air/void; where topography is above it, the top unit is kept. The keep/drop decision is driven by clipping, not by any flag.
- **Series with the legacy flag set to "not filling upward" and no younger cap**: must still fill upward (proving the deprecated flag is inert and the universal default holds).
- **Uncapped void**: any volume with nothing younger capping it and no topography above resolves to air.
- **Loading a legacy scenario** that still carries the deprecated per-series flag: parses without error; the flag is accepted but ignored.
- **Deleting a body whose inferred lithology is assigned to a unit**: the inferred entry is removed; affected assignments fall back to the Unknown default.
- **Empty catalog assignment**: every below-topography volume always has *a* lithology (Unknown at worst); the model is never blocked on assignment.

## Requirements *(mandatory)*

### Functional Requirements

#### Topography & fill-upward

- **FR-001**: The system MUST support a dedicated Topography surface whose upper side is always air and whose lower side caps every older element to below it.
- **FR-002**: The system MUST enforce, structurally, that at most one Topography surface exists in a model and that it is always the youngest element; reorder operations MUST refuse to place any element above it or move it below another.
- **FR-003**: Users MUST be able to add a Topography surface via the add-surface type picker and author its points and optional orientations using the existing surface-authoring flow; the picker MUST disable or hide the Topography choice when one already exists.
- **FR-004**: The system MUST evaluate the region above the topography surface as air (no rock) and evaluate below it using the existing field evaluation.
- **FR-005**: Every series MUST fill upward by default — present below its top contact and continuing upward until a younger element (including topography) clips it — independent of any per-series flag.
- **FR-006**: The decision to keep or drop a series' top unit MUST be derived from whether a younger element actually caps the series at that point (kept where nothing younger caps it, e.g. the floor; dropped to air/void where topography cuts below it), not from a per-series flag.
- **FR-007**: The legacy per-series "fills upward" flag MUST be retained in the serialized data and accepted on import but ignored in evaluation (behaviour-first deprecation); its authoring control MUST be removed from the UI.
- **FR-008**: Legacy scenarios carrying the deprecated flag MUST continue to load and parse without error.

#### Units as the unit of presentation

- **FR-009**: The core MUST expose, per element, its ordered list of derived units with stable unit indices (youngest→oldest within the element), so units can be addressed as `(element, unit)` coordinates.
- **FR-010**: The authoring panel MUST render an alternating ladder of unit and contact rows, youngest at top, deriving the units from the core's per-element unit list rather than rendering one row per interface.
- **FR-011**: Units MUST be derivable only — the user MUST NOT be able to add or remove a unit directly; adding/removing contacts re-derives the units (N contacts → N+1 units, minus clipping).
- **FR-012**: Users MUST be able to rename a unit; contact rows MUST NOT offer a rename (a contact has no user-facing name) and MUST keep their existing point/orientation modal.

#### Lithology catalog & assignment

- **FR-013**: The system MUST maintain a lithology catalog of entries, each with a name and a colour, plus a provenance distinguishing fixed (Air), inferred (from bodies), and user-added entries.
- **FR-014**: The catalog MUST include a distinguished, non-deletable, non-renameable **Air** entry rendered transparent; Air MUST be the lithology of every volume above topography and of any uncapped void, and MUST never be meshed as a solid.
- **FR-015**: The catalog MUST include a distinguished, non-deletable **Unknown** entry (neutral) that is the default lithology of every below-topography series-internal unit until assigned; Unknown MUST be meshed as a solid (it is rock of unspecified type, not void).
- **FR-016**: The system MUST infer a catalog entry from each cross-cutting body (vein/intrusion) named after the body, auto-assigned to that body's interior; renaming the body MUST re-sync the inferred entry and deleting the body MUST remove it.
- **FR-017**: The system MUST NOT infer names for series-internal units; those MUST start at the Unknown default.
- **FR-018**: Users MUST be able to add user lithologies (name + colour), rename and recolour user and inferred entries, and delete user entries; inferred entries are managed via their body and Air/Unknown are fixed.
- **FR-019**: Each derived unit MUST offer a lithology dropdown listing all catalog entries; the system MUST allow multiple `(element, unit)` cells to be assigned to the same catalog entry (many-to-one), with no automatic correlation detection.
- **FR-020**: Every below-topography volume MUST always resolve to some lithology (Unknown at worst) and every above-topography volume MUST resolve to Air, so the model is never blocked on assignment.
- **FR-021**: The lithology catalog (entries, colours, and assignments) MUST be serialized into the exported scenario and restored on import, round-tripping names, colours, and assignments. No separate persistence (no local storage / database) is required.

#### Solid meshing & rendering

- **FR-022**: The system MUST produce a closed, watertight solid for each below-topography `(element, unit)` volume, reusing the existing capped-Scene meshing path with topography participating as a universal top masker, and MUST emit nothing for air volumes.
- **FR-023**: Each emitted solid MUST carry the unit coordinate / lithology identity needed to colour and group it by lithology in the viewer.
- **FR-024**: The harness MUST provide a Surfaces ⇄ Solids toggle: Surfaces shows open contact sheets coloured by series (today's default); Solids shows closed watertight unit solids coloured by lithology with air omitted.
- **FR-025**: Toggling render mode MUST NOT recompute the field fit — it re-runs only the mesh extraction on the already-fitted stack.
- **FR-026**: The viewer MUST support colouring by series (surfaces) or by lithology (solids), using the catalog's colours in solid mode and skipping air.
- **FR-027**: OBJ export MUST group geometry by lithology when in solid mode.

### Key Entities *(include if feature involves data)*

- **Topography**: A dedicated, singleton, always-youngest surface whose upper side is air and whose clip caps every older element to below it. Authored like any other surface (points + optional orientations).
- **Unit**: A volumetric slab bounded by contacts, derived by the core (N contacts → N+1 units within a conformable series). Addressed by an `(element, unit-index)` coordinate; the named, lithology-assignable thing. Not directly authored.
- **Contact (interface)**: A boundary between two units. Authored (points/orientations) and addable/removable by the user; carries no user-facing name.
- **Lithology**: A named, coloured rock type with a provenance: Air (fixed, transparent, never solid), Unknown (fixed, neutral, solid), inferred (named after a body), or user-added. The catalog's member.
- **Lithology Catalog**: The collection of lithologies plus the assignment mapping each derived `(element, unit)` volume to a catalog entry. Lives with the project document, serialized into the exported scenario.
- **Assignment**: A many-to-one mapping from `(element, unit)` volume coordinates onto catalog entries — the explicit, user-driven correlation mechanism.
- **Solid volume**: The watertight closed mesh of one below-topography `(element, unit)` volume, tagged with its lithology for colouring/grouping. Air volumes produce no solid.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can add a topography surface to a previously box-filled model and see the above-ground region become air and all older elements clipped to the ground, in a single authoring action (no flag toggling, no per-series adjustment).
- **SC-002**: For a conformable series with N contacts, the panel displays exactly N+1 nameable unit rows interleaved with N contact rows; adding or removing a contact changes the unit count correctly 100% of the time, and the user never adds or removes a unit directly.
- **SC-003**: A user can assign two physically separate volumes to the same lithology via dropdowns alone, with no manual coordinate entry and no automatic correlation step.
- **SC-004**: Every below-topography volume always displays a lithology (Unknown if unassigned) and every above-topography volume displays as air — there is no state in which a volume has no lithology.
- **SC-005**: In solid mode, every emitted volume mesh is watertight (no holes, manifold edges, consistent winding) — including the multi-clip case (topography-capped, floor-bounded, intrusion-punched) — verified by an automated invariant test, and no solid is emitted for air.
- **SC-006**: Toggling between Surfaces and Solids changes the view without re-fitting the field (the toggle does not trigger a kernel solve).
- **SC-007**: Exporting a scenario with a customised catalog and assignments, then re-importing it, restores 100% of lithology names, colours, and per-unit assignments.
- **SC-008**: An OBJ exported in solid mode contains geometry grouped by lithology, with no air group present.
- **SC-009**: All pre-existing Rust tests that pin unit presence/count continue to pass with the per-series flag made inert, and a new test confirms a series with the flag unset and no younger cap still fills upward.
- **SC-010**: Native and wasm builds agree to `f64` round-off on the new topography-capped evaluation and unit enumeration (engine parity preserved).

## Assumptions

- **Reuse over rebuild**: The existing capped-Scene solid meshing path (`octree_dual_contour_scene` with `cap = true`) and the non-convex complement path that keeps vein/shell volumes watertight are reused verbatim; this feature adds topography as a universal top masker and an air-skip, not a new mesher. (Constitution Principle VI.)
- **Structural topography variant**: Topography is modelled as its own stack-element variant (not a flagged youngest series) so its invariants — singleton, youngest, erode-clip, air above — are structural rather than conventional. It reuses the existing erode half-space clip mechanism.
- **Derive, don't declare**: Unit count is never asserted by the model; the UI labels whatever volumes the field yields. Lithology *names* are a UI concern compiled back in for colouring; the core provides only unit *coordinates*. (Constitution Principle III.)
- **Staged deprecation**: The per-series "fills upward" flag is neutralised in stages (engine ignores it → ABI/protocol keep the slot accepted-but-ignored → UI hides the control → tests pin the preserved behaviour) rather than ripped out, so existing serialized scenarios and the Rust test suite stay green. Removing the slot from the data contract entirely is later cleanup, out of scope here.
- **Live + scenario-JSON persistence only**: The catalog and assignments live in memory and are serialized into the downloadable scenario JSON; there is no separate persistence layer (no local storage, no database).
- **Air is data, not absence**: Air is a distinguished catalog member (transparent, never solid), not a null — so "above topography" and "uncapped void" both resolve to a concrete lithology.
- **Unknown mirrors Leapfrog**: Series-internal units default to a distinguished "Unknown" lithology that *is* solid-meshed, keeping the model never-blocked while leaving naming explicit.
- **Existing protocol carries render mode**: The protocol already carries a contact/solid render-mode distinction; this feature wires it to a UI toggle rather than introducing the concept.
- **Faults are not present**: This feature assumes a fault-free model; topography clips and unit derivation are defined only over series/vein/intrusion elements plus topography.
- **Functional + invariant gates apply**: Per the constitution, the UI behaviours here are backed by automated functional (Playwright) tests against the running app, and the watertightness claims by invariant tests — not by manual walk-throughs or screenshots. (Constitution Principles I and V.)

## Out of Scope / Non-Goals

- **Faults / block model** — out of scope; no fault element, no block partition. The model is fault-free here.
- **Hard removal of the legacy fill-upward flag from the data contract** — the flag is made inert and invisible now; removing its slot from the serialized form / ABI is deferred to a later cleanup once nothing reads it. The seam left: the slot remains, documented as deprecated.
- **Drillhole-label inheritance** — units start at Unknown and are named by the user; deriving lithology names from drillhole labels is a future inference source that can layer onto the same catalog. Seam left: the catalog's "inferred" provenance is the attachment point.
- **Surface-as-entity / correlative conformity refactor** — surfaces are not lifted to shared referenced entities in this step. Seam left: retiring the fill-upward flag removes one of the two bits that future refactor must reconcile.
- **Automatic correlation detection** — the geometry cannot know two slabs are the same formation; correlation is the explicit, user-driven many-to-one dropdown assignment and nothing more. This is intentionally manual.
- **Persistent storage** — live-only plus scenario-JSON round-trip; no local storage or database.
- **Re-architecting the mesher** — the capped-Scene path is reused as-is; no new meshing architecture.
