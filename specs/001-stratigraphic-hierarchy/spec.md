# Feature Specification: Stratigraphic Hierarchy — Model, UI, and Testbed

**Feature Branch**: `001-stratigraphic-hierarchy`

**Created**: 2026-06-18

**Status**: Draft

**Input**: User description: "implement docs/stack_hierarchy_spec.md in its entirety — changes to rust and new reusable react/ts UI elements as described, and a testbed in say web3/ that makes use of the rust crates in WASM form (basic pattern like web/) that shows off the UI and underlying geo modeling with some synthetic data."

**Source of record**: The technical design lives in [`docs/stack_hierarchy_spec.md`](../../docs/stack_hierarchy_spec.md)
and is authoritative for the model semantics, Rust contracts, and UI ↔ core
protocol. This spec states the WHAT/WHY and the acceptance criteria; it does not
restate the design doc's signatures (Constitution Principle III — no second,
drift-prone copy). Where this spec and the design doc disagree on intended
behavior, the design doc wins and this spec is corrected.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Configure a layered stratigraphy and see it mesh (Priority: P1)

A geologist organizes a set of co-kriged interfaces into ordered stratigraphic
groups (youngest at top), sets how each group meets the one beneath it (the group
either truncates what's below, laps onto it, or stacks conformably), marks the
floor group as filling upward, and watches the 3D model re-mesh to match. This is
the core "stratigraphic column" the whole feature exists to express.

**Why this priority**: Without it there is no model to configure and nothing to
show. It is also the smallest coherent slice that delivers value, and it exercises
the foundational model correction (retiring the overloaded "basement" notion into
two independent properties — how a group clips below, and whether it is bounded on
top). Everything else builds on this.

**Independent Test**: Load synthetic data for two or more stratigraphic groups,
toggle the seam between two adjacent groups through its three states, and confirm
the meshed result changes exactly as the geology dictates (truncation removes the
older surface where the younger group is present; lapping does not), with the floor
group present everywhere.

**Acceptance Scenarios**:

1. **Given** two stacked groups where the younger truncates the older, **When** the
   model meshes, **Then** the older group's surface is absent wherever the younger
   group is present, and present elsewhere.
2. **Given** the same two groups switched to "laps onto," **When** the model
   meshes, **Then** the older surface is no longer truncated and the younger group
   pinches out against it — a visibly different result from the truncation case.
3. **Given** the oldest group marked as the floor, **When** lithology is queried at
   any point not claimed by a younger group, **Then** the floor's top unit is
   reported (it is present everywhere and is not silently dropped).
4. **Given** a group whose top contact has interfaces in a given order, **When** the
   user reorders interfaces *within* that one group, **Then** only that group's
   internal level-set sequence changes — chronology (group order) is untouched.

---

### User Story 2 - Place cross-cutting bodies (veins and intrusions) (Priority: P2)

A geologist adds a vein (a tabular body between a hanging-wall/foot-wall pair) or
an intrusion (a closed body) at a point in the chronology. The body claims the
space inside it and cuts through everything older it intersects. The geologist
never enumerates "what it cuts" — the system derives that from where the body sits
in the chronology and what it spatially reaches, and shows it read-only.

**Why this priority**: This is the largest genuinely new modelling capability — the
existing truncate/lap/conform vocabulary has no slot for a body that relates to
everything it passes through rather than to the one group beneath it. It depends on
the P1 foundation but is independently demonstrable.

**Independent Test**: Place a vein spanning a band across two older groups; confirm
it claims only its band and clips those groups only within the band. Place an
intrusion; confirm it claims only its interior and the "cuts" set the system
reports matches the older groups it actually overlaps — with nothing declared by
hand.

**Acceptance Scenarios**:

1. **Given** a vein placed above several older groups, **When** the model meshes,
   **Then** the vein is present only between its two bounding contacts and clips
   older groups only inside that band, not as an unbounded half-space.
2. **Given** an intrusion placed at a chronology position, **When** lithology is
   queried, **Then** the intrusion claims every interior point not already claimed
   by a younger element, with no change required to the lithology rule.
3. **Given** any cross-cutting body, **When** the user views its summary, **Then**
   the set of older elements it cuts is shown read-only and updates automatically
   when the body is reordered — it is never hand-entered.

---

### User Story 3 - Faults as an outer partition; per-block deviation (Priority: P2)

A geologist defines a fault network that partitions the domain into blocks; the
entire stratigraphy is evaluated independently inside each block. A block where a
contact has no expression simply shows no surface there. The geologist can select a
block to view it and, in a clearly-marked per-block editing mode, disable a contact
in just that block without altering the shared template.

**Why this priority**: Inverting faults from per-group bookkeeping to a single
authoritative partition matches the geological model and removes a class of
consistency bugs. It is independently valuable and testable, and the existing
per-block meshing primitives are reused rather than rebuilt (Constitution Principle
VI). It can ship after P1 with a single-block stub partition.

**Independent Test**: With a fault network producing two blocks, confirm the same
stratigraphic template renders independently in each block, a starved contact
produces no surface (and is shown as absent for that block) without aborting
evaluation, and a per-block disable affects only the selected block.

**Acceptance Scenarios**:

1. **Given** a fault network of N blocks, **When** the model meshes, **Then** the
   stratigraphy is evaluated per block and surfaces terminate flush at fault planes
   with no stitching across the fault.
2. **Given** a block with no data for a contact, **When** that block is viewed,
   **Then** the contact renders as absent-in-block (dimmed, annotated) and the rest
   of the block still evaluates.
3. **Given** per-block editing mode on a selected block, **When** the user disables
   a contact, **Then** only that block changes and the shared template is untouched,
   with the UI clearly indicating an override is being edited.

---

### User Story 4 - Reusable hierarchy UI editing declared state (Priority: P1)

An application developer embeds a draggable hierarchy panel that presents the model
as a tree (containers with surface children, youngest at top) and a visually
separate fault sub-panel. Reordering containers changes chronology; reordering
surfaces within a container changes only that group's level-set order — the two
drag behaviors look and feel distinct so the consequential one is never triggered
by accident. The seam (truncate/lap/conform) is set on the boundary between
containers; cross-cutting bodies show a read-only "cross-cuts" marker; validation
and derived relationships are surfaced as the core reports them. The UI edits
declared state only and never derives geology itself.

**Why this priority**: The panel is the visible product and the reusable artifact
the prompt asks for. It is co-equal with P1 model work because the model correction
and the editing surface together form the first demonstrable slice.

**Independent Test**: Drive every editable gesture (reorder containers, reorder
within a container, move a surface across containers, set a seam state, toggle the
floor marker, enable/disable, reorder faults, select a block) and confirm each maps
to exactly one core mutation, the view reflects the core's returned derived data and
meshes, and the two drag behaviors are visually unmistakable.

**Acceptance Scenarios**:

1. **Given** the hierarchy tree, **When** a user drags one container above another,
   **Then** chronology changes and the model re-meshes, with a strong full-width
   insertion affordance and a transient note on affected boundaries.
2. **Given** a conformal group, **When** a user drags one of its surfaces, **Then** a
   visibly *different*, lighter/indented affordance is shown and only the group's
   level-set order changes; this gesture is disabled for veins and intrusions.
3. **Given** any container, **When** the core reports it under-constrained or absent
   in the selected block, **Then** the panel shows the matching badge/dimming and
   hover detail, sourced entirely from the core's derived data.
4. **Given** the floor container, **When** it is shown, **Then** no seam control
   appears below it and a "fills upward / floor" marker is shown instead.

---

### User Story 5 - web3/ testbed demonstrating the whole feature on synthetic data (Priority: P1)

A developer or reviewer opens the `web3/` testbed in a browser, picks from a small
set of synthetic scenarios (a layered column; a column with an unconformity; one
with a vein or intrusion; one with a fault), and sees the hierarchy UI driving the
Rust modeling core compiled to WebAssembly — editing the configuration re-meshes
and re-renders live, exactly as a host application would wire it.

**Why this priority**: It is the acceptance vehicle the prompt explicitly asks for —
the thing that "shows off" the feature end-to-end and proves the reusable UI and the
WASM core actually compose. Without it, the feature is parts without a demonstrated
whole.

**Independent Test**: Build and serve `web3/`, select each synthetic scenario, and
confirm the hierarchy panel renders it, edits round-trip through the WASM core, and
the 3D/section view updates — following the same basic harness pattern as the
existing `web/`.

**Acceptance Scenarios**:

1. **Given** the testbed served in a browser, **When** a synthetic scenario is
   selected, **Then** its stratigraphy loads into the hierarchy panel and renders in
   the viewer.
2. **Given** a loaded scenario, **When** the user performs an editable gesture in the
   panel, **Then** the change is applied by the WASM core and the view updates,
   demonstrating the full mutation → derive → mesh round-trip.
3. **Given** the testbed, **When** the synthetic set is enumerated, **Then** it
   covers at least: a conformal column, an erosional unconformity, one cross-cutting
   body, and one fault network.

---

### Edge Cases

- A group whose field is under-constrained (lacks points or orientation) — the
  system warns but does not block editing, and the UI surfaces which surfaces are
  short of data.
- Moving a surface across containers mid-edit such that a field becomes
  under-constrained — warn, do not block.
- The oldest group mislabeled as a normal conformal group instead of the floor — must
  not silently drop its top unit (the historical "missing volume" failure mode).
- A cross-cutting body that reaches no older element — its derived "cuts" set is
  empty, shown as such, not an error.
- A fault block with no data for a contact — renders absent-in-block; evaluation of
  the rest of the block proceeds.
- A correlative-conformity surface that is one stack's truncation top and the next
  stack's lap-against floor — represented once and referenced, not duplicated.

## Requirements *(mandatory)*

### Functional Requirements

**Model — stratigraphic core**

- **FR-001**: The system MUST represent the relationship of a group to the group
  beneath it as one of three states — truncates / laps-onto / stacks-conformably —
  set independently per boundary.
- **FR-002**: The system MUST represent "is this group bounded by its own top, or
  does it fill upward until a younger group claims the space" as a property of the
  group itself, independent of FR-001, replacing the overloaded "basement" notion.
- **FR-003**: The system MUST assign lithology by youngest-present-wins, with the
  floor group present everywhere, and MUST NOT drop the floor group's top unit.
- **FR-004**: The floor group MUST have no relationship-below to set (the boundary
  beneath it is not representable as a clip).
- **FR-005**: Existing tested behaviors (one co-kriged field with N interfaces per
  group; truncate-vs-lap as clip-vs-no-clip; watertight meshing clipped to true
  extent) MUST be preserved, not rebuilt.

**Model — cross-cutting bodies**

- **FR-006**: The system MUST support a vein: a tabular body present between a
  hanging-wall/foot-wall pair, clipping older groups only within that band.
- **FR-007**: The system MUST support an intrusion: a closed body present inside its
  envelope, claiming its interior.
- **FR-008**: For any cross-cutting body, the set of older elements it cuts MUST be
  derived from chronological position plus spatial intersection and MUST NOT be
  user-enumerated; it is presented read-only.
- **FR-009**: Lithology assignment MUST require no special case for cross-cutting
  bodies (youngest-present-wins already covers them).

**Model — faults**

- **FR-010**: The system MUST treat faults as an outer partition of space into
  blocks, with the entire stratigraphy evaluated independently inside each block.
- **FR-011**: A block lacking data for a contact MUST produce no surface there
  without aborting evaluation of the rest of the block.
- **FR-012**: The default (no overrides) MUST be the same template in every block;
  the system MUST allow a per-block override that disables a contact in one block
  without editing the template.

**Model — shared surfaces**

- **FR-013**: A single surface MUST be representable as referenced by two boundaries
  (e.g., a truncation top below and a lap-against floor above) without duplication.

**UI — reusable hierarchy panel**

- **FR-014**: The UI MUST present the model as a draggable tree, youngest at top,
  with container nodes (group / vein / intrusion) and surface children, plus a
  visually separate fault sub-panel.
- **FR-015**: The UI MUST provide two visually distinct drag behaviors: reordering
  containers (changes chronology) and reordering surfaces within a conformal group
  (changes only that group's level-set order); within-group reorder MUST be disabled
  for veins and intrusions.
- **FR-016**: The UI MUST expose the three-state seam control on the boundary between
  containers, show a read-only cross-cuts marker for bodies, and show a floor marker
  (no seam control) for the floor.
- **FR-017**: The UI MUST surface validation (under-constrained, absent-in-block) and
  the derived relationship summary using only data the core reports — it MUST NOT
  recompute geology.
- **FR-018**: Each user gesture MUST map to exactly one core mutation; the UI MUST
  treat all derived data as a read-only cache refreshed on each core response.
- **FR-019**: The UI components MUST be reusable — packaged so a host application
  other than the testbed can embed them.

**UI ↔ core contract**

- **FR-020**: The shared data types crossing the UI ↔ core boundary MUST be generated
  from the core's schema so the two cannot drift, with the core as the single source
  of truth for the grammar.

**Testbed**

- **FR-021**: A `web3/` testbed MUST drive the modeling core compiled to WebAssembly,
  following the existing `web/` harness pattern.
- **FR-022**: The testbed MUST ship synthetic scenarios covering at least: a conformal
  column, an erosional unconformity, one cross-cutting body, and one fault network.
- **FR-023**: In the testbed, an edit in the hierarchy panel MUST round-trip through
  the WASM core and update the viewer live.

**Cross-cutting (project constitution)**

- **FR-024**: Model behavior MUST be validated by invariant/regression tests, not by
  appearance; the historical "missing volume" regression MUST remain pinned (now
  keyed on the bounded-on-top property, not the retired basement variant).
- **FR-025**: The WASM artifact split MUST be preserved: the testbed reaches the
  interpolation kernel across the module boundary; no kernel/GemPy-derived code is
  embedded in the engine artifact.

### Key Entities *(include if feature involves data)*

- **Stratigraphic group (container)**: one co-kriged field exposing N ordered
  interfaces; carries a relationship-below (truncate/lap/conform) and a
  bounded-on-top property.
- **Vein**: a cross-cutting container present between a hanging-wall/foot-wall
  interface pair.
- **Intrusion**: a cross-cutting container present inside a closed envelope.
- **Surface**: one co-kriged interface; a referenced entity that may be shared by two
  boundaries.
- **Fault network**: faults plus their relative timing; derives the block partition.
- **Fault block**: a region of the partition in which the whole stratigraphy is
  evaluated; may carry per-block overrides.
- **Derived view**: read-only data the core returns each round-trip — cut-sets,
  effective relationships, validation, blocks, absent-in-block — never edited by the
  UI.
- **Synthetic scenario**: a named sample model bundled with the testbed.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All four primary configurations — conformal column, erosional
  unconformity, a cross-cutting body, and a faulted model — can be expressed and
  meshed from the hierarchy UI in the testbed.
- **SC-002**: Switching a single boundary between its three relationship states
  produces three visibly distinct meshed results, with no other declared input
  changed.
- **SC-003**: For every cross-cutting body in the synthetic set, the "what it cuts"
  set shown to the user matches the older elements it actually overlaps, with zero
  hand-entered cut declarations.
- **SC-004**: In a multi-block faulted scenario, the same template renders
  independently per block and a data-starved contact is reported absent without
  aborting that block — observable in the testbed.
- **SC-005**: 100% of editable UI gestures map to exactly one core mutation and the
  view reflects only core-derived data (no UI-side geology computation), verifiable
  by the round-trip test.
- **SC-006**: The two drag behaviors are distinguishable by users without a tooltip in
  a quick review (chronology vs. within-group reorder are never confused).
- **SC-007**: The pre-existing model regression suite (including the missing-volume
  guard) stays green throughout, and new model behaviors each add a test that pins
  their failure mode.
- **SC-008**: A reviewer can build and serve `web3/`, select each synthetic scenario,
  and complete a configuration edit that re-meshes live, following the same steps as
  the existing `web/` harness.

## Assumptions

- The interpolation/meshing engine internals are not in scope here; this feature is
  the configuration + model layer and UI in front of the existing Rust meshing, per
  the design doc.
- "Reusable UI elements" means components packaged for embedding by a host other than
  the testbed; the testbed is one consumer, not the only intended one.
- `web3/` follows the existing `web/` harness conventions (browser, WASM-loaded core,
  synthetic fixtures) rather than introducing a new app architecture.
- Synthetic data is authored for demonstration and testing; no real dataset import is
  required for this feature.
- The shared UI ↔ core types are generated from the Rust schema; the transport
  (in-browser WASM bindings) follows the existing pattern.

## Out of Scope / Non-Goals

Carried from the design doc's stated non-goals plus this feature's boundaries
(Constitution Principle IV — scope is stated, not implied):

- **Syn-depositional intrusions** (a body younger than some local units and older than
  others). Modeled as multiple bodies; a seam is left at the body variant and the
  limitation is surfaced in a UI tooltip — not solved here.
- **Fault displacement / offset modeling** beyond fault timing used to derive blocks.
- **A general DAG editing UI.** v1 exposes surface sharing only through the seam
  control, not a free-form graph editor.
- **Interpolation and meshing engine internals** — reused as-is, not modified.
- **Per-block data overrides** beyond disabling a contact in a block (post-v1).
- **Degree-2 universal drift and other kernel-scope items** unrelated to this feature.
- **Real topographic/dataset import** (tracked separately under the topo-surface spec).
