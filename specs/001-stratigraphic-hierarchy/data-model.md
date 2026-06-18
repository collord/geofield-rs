# Phase 1 Data Model: Stratigraphic Hierarchy

Entity-level model derived from the spec's Key Entities and
`docs/stack_hierarchy_spec.md` (§1–§5). Field-level Rust signatures live in the
design doc and are **not** restated here (Constitution Principle III). This file
records entities, their fields-of-meaning, relationships, validation rules, and
the tests that pin each behavior.

## Entities

### StackElement (container) — the draggable unit

The ordered chronology is `Vec<StackElement<M>>`, youngest at index 0. Three
kinds (design doc §3.1 Option A; research R2):

| Kind | Payload of meaning | Present-where | Clips older via |
|------|--------------------|---------------|------------------|
| **Series** (stratigraphic group) | one co-kriged field, N ordered interfaces; `relation: Option<Relation>` (Axis A); `unbounded_above: bool` (Axis B) | `field < iso_top` (bounded) or everywhere-until-claimed (`unbounded_above`) | erode masker half-space when `relation = Erode` |
| **Vein** | co-kriged field; hanging-wall + foot-wall interface ids; optional reference | between the HW/FW isovalues (a **band**) | two ANDed `ContactHalfSpace`s (above FW, below HW) |
| **Intrusion** | co-kriged field; envelope (single or shell); geometry model (distance-to-contact \| conceptual-extent) | inside the envelope interior | the envelope interior half-space(s) |

### Relation (Axis A) — the seam below

`Erode` | `Onlap` | `Conformable`. **`Basement` is removed** (the design doc's
central correction). The floor element carries `relation: None` (no seam beneath
it is representable as a clip). Veins/intrusions carry no `Relation` — their
relationship is derived cross-cutting, not a seam.

### unbounded_above (Axis B) — bounded vs. fills-upward

A `bool` on a Series, independent of Axis A. `true` = the series fills upward
until a younger element claims the space (the old `Basement` behavior, now
honest). The floor is conventionally `unbounded_above = true`. This is the single
bit `present_unit_count` branches on (was: `matches!(relation, Basement)`).

### Surface — referenced interface entity

A co-kriged interface lifted to a referenced entity (`SurfaceId`). May be
referenced by **two** boundaries (a truncation top below + a lap-against floor
above) without duplication — the correlative-conformity case. The data layer is a
DAG; the UI presents a tree.

### FaultNetwork / FaultBlock — the outer partition

`FaultNetwork` = faults + fault–fault timing (`interactions`, younger truncates
older) → derives a block partition. The **entire stack** is instanced into each
`FaultBlock`. A `BlockOverride` may disable an element in one block without editing
the template. (Inverts the current per-series nesting — design doc §4.)

### Derived view — core-owned, read-only

Computed by the core each round-trip, never stored or edited by the UI:
`cutsBelow`, `effectiveRelation` (erode/onlap/conformable/crossCut/floor),
`validation` (per element), `blocks`, `absentInBlock`.

### Synthetic scenario — testbed fixture

A named declared-state bundle (`StackElementDTO[]` + extent + optional fault
network) loaded into the core. The set covers ≥4 cases (research R-scenarios).

## Relationships

- `Stack` **has-ordered** `StackElement`s (chronology; index 0 youngest).
- `StackElement::Series` **has-ordered** `Surface` children (isovalue sequence).
- `Surface` **may-be-referenced-by** two element boundaries (DAG edge).
- `FaultNetwork` **partitions-into** `FaultBlock`s; each block **instances** the
  whole `Stack`.
- `FaultBlock` **may-have** a `BlockOverride` (disabled elements).
- A cross-cutting element **cuts** the derived set `cuts_below(i)` (order ∩
  geometry; never stored).

## State transitions (declared-state mutations)

Each maps to exactly one core mutation (FR-018); the core returns a fresh
`CoreUpdate`. See [contracts/core-api.md](contracts/core-api.md).

- reorder containers → chronology change → recompute + remesh
- reorder surfaces within a series → isovalue-sequence change (local only)
- move a surface across containers → both fields re-validated
- set relation (Axis A) → seam change on one boundary
- set unbounded_above (Axis B) → floor/fill-upward toggle
- set enabled (optionally per-block) → visibility / `BlockOverride`
- reorder faults → fault timing → blocks rebuilt
- add container (kind, at index) → new element

## Validation rules

| Rule | Source | Pinned by |
|------|--------|-----------|
| Lithology is youngest-present-wins; floor present everywhere; floor top unit never dropped | FR-003 | re-keyed `basement_keeps_topmost_unit_which_daylights` (now on `unbounded_above`); `basement_partitions_domain` |
| Erode clips older surfaces; onlap/conformable do not | FR-001, FR-005 | `onlap_vs_erode_differ` (must stay green) |
| Per-element meshes stay watertight | FR-005 | `stack_*_surfaces_are_watertight`, `stack_four_series_watertight` |
| A vein is present only between HW/FW and clips older only within that band | FR-006 | **new**: vein-band-clip test (older bed cut only inside band) |
| An intrusion claims only its interior; lithology needs no special case | FR-007, FR-009 | **new**: intrusion-interior lithology test |
| `cuts_below` matches actual older overlap; never user-entered | FR-008 | **new**: cuts_below derivation test |
| A starved block produces no surface without aborting the block | FR-011 | **new**: per-block instancing test (single-block stub first, R3) |
| A per-block disable affects only that block | FR-012 | **new**: block-override test |
| A shared surface is represented once across two boundaries | FR-013 | **new**: shared-surface reference test |
| Existing suite stays green throughout | FR-024, SC-007 | the full `crates/geofield/tests/` + `stack.rs` unit tests |

## Migration note (the de-risking first step)

`Relation::Basement` → `relation: None` (or `Some(Conformable)`) +
`unbounded_above: true`. The only readers to update: `global_unit`,
`erode_masker_fields`, `present_unit_count`. Mechanical; do it first so the model
axes are clean before cross-cutting bodies and the fault inversion build on them
(design doc §8 item 1; Principle VI). The `relation: number` wasm ABI (currently
`3 = Basement`) gains a parallel `unbounded_above` flag rather than overloading the
integer — the ABI delta is in [contracts/wasm-abi-delta.md](contracts/wasm-abi-delta.md).
