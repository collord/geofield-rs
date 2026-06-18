# Stratigraphic Hierarchy: Logical Model + UI Specification

**Status:** design spec / build brief
**Architecture:** contact logic, relationship derivation, and meshing in **Rust**; configuration UI in **TypeScript / React**
**Reference implementation:** `stack.rs` (current `Stack` / `StackedSeries` / `Relation`)
**Audience:** Claude Code (implementation), with the *Grammar of Contact Surfaces* doc as background

---

## 0. How to read this spec

The current `stack.rs` already implements most of the *model*. This spec is therefore written as a **delta**, not a greenfield design. Every substantive subsection is tagged:

- **[EXISTS]** — already in `stack.rs`; described so the contract is visible, not to be rebuilt.
- **[EXTENDS]** — a change to something that exists.
- **[NEW]** — not present in `stack.rs` at all.

The honest summary: the core model primitive we care about — *a series is one co-kriged field with N interface isovalues* — **already exists**. So does youngest-present-wins lithology, and so does erode-vs-onlap as a clip/no-clip mechanism. The genuinely new work concentrates in three places: **(1) cross-cutting bodies (veins, intrusions)**, **(2) inverting the fault nesting** so faults are an outer partition that instances the stack per block (today faults live *inside* each series), and **(3) the entire UI/interaction layer.** A fourth, smaller model change falls out of this conversation: **decomposing `Basement`** into its two real axes.

---

## 1. The two-axis model (the central correction) **[EXTENDS]**

The current `Relation` enum has four variants — `Erode`, `Onlap`, `Conformable`, `Basement` — and treats them as one axis ("how this series sits on the older series beneath it"). But `Basement` is a category error: it is not a relationship to the series below, it is an intrinsic property of the series itself, and it is overloaded across three behaviors in the current code:

1. **present-everywhere** — `global_unit` short-circuits the present test for `Basement`;
2. **never-clips** — `erode_masker_fields` collects only `Erode`; but this is vacuously true for the *oldest* series regardless;
3. **keep-topmost-unit** — `present_unit_count` returns `n` for `Basement` vs `n-1` for everything else (the "missing volume" regression test pins this).

All three follow from a single property: **the series has no bounding top — it fills upward until something younger claims the space.** That is orthogonal to how it clips what's beneath it. So the one enum is really two independent fields:

```rust
/// Axis A — how this series clips the (older) series beneath it. The seam below.
/// This is the genuine inter-series relationship; the Lajaunie–Calcagno binary
/// (erode | onlap) plus the emergent-conformity no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Truncates older series wherever this series is present. The eroding field is
    /// added as a clip half-space to older series' Scenes. (Calcagno erode; GemPy
    /// ERODE; LoopStructural ERODE; Leapfrog Erosion.)
    Erode,
    /// Laps onto the older surface and pinches out; present only on its deposited
    /// side, does NOT clip older series. (Calcagno onlap; Leapfrog Deposit.)
    Onlap,
    /// Stacks normally; subdivides within its own region but never clips older series.
    /// Conformity is emergent (shared field); this is the explicit non-clipping series.
    Conformable,
}

/// Axis B — intrinsic to the series: is it bounded by its own top contact, or does
/// it fill upward until a younger series claims the space? Replaces the `Basement`
/// variant. The oldest series is conventionally `unbounded_above = true`.
pub struct StackedSeries<M> {
    pub model: M,
    pub relation: Relation,        // Axis A — None-equivalent for the floor (see note)
    pub unbounded_above: bool,     // Axis B — was the `Basement` variant
}
```

**Why this is the right cut, concretely:** `present_unit_count` currently branches on `relation == Basement`, but what it *actually* needs to know is "is the top unit rock or void," which is exactly `unbounded_above`. The current code uses `Basement` as a proxy because the two coincide at the bottom of the stack. Splitting them makes the branch semantically honest:

```rust
pub fn present_unit_count(&self, i: usize, b: usize) -> usize {
    let n = self.block_unit_count(i, b);
    if n == 0 { return 0; }
    if self.series[i].unbounded_above { n } else { n - 1 }   // was: matches!(relation, Basement)
}
```

…and it removes a latent bug: today, if the oldest series is mislabelled `Conformable` instead of `Basement`, `present_unit_count` silently drops its top unit and the missing-volume regression returns. With `unbounded_above` explicit, "is this the floor" is no longer smuggled into an enum variant the constructor must remember to pick.

**`relation` for the floor.** The oldest series has no seam beneath it, so its Axis-A value is moot. Either keep `Conformable` as the conventional inert value, or model `relation: Option<Relation>` (`None` for the floor). Recommended: `Option<Relation>`, so the type makes "no series below" unrepresentable-as-a-clip rather than relying on convention.

**Migration:** `Relation::Basement` → `relation: None` (or `Some(Conformable)`) + `unbounded_above: true`. Mechanical; the `global_unit`, `erode_masker_fields`, and `present_unit_count` sites are the only readers.

---

## 2. What already exists (contract, do not rebuild) **[EXISTS]**

These are in `stack.rs` and are the substrate the new work builds on. Stated so the boundaries are visible.

- **Series = one co-kriged field, N interfaces.** `StackedSeries { model, relation }`, `model.interface_isovalues()`, `iso_top()` = the top (bounding/unconformity) isovalue. *This is the primitive our "container = scalar field" idea names; it already exists.*
- **Ordered stack, youngest-first.** `Stack { series: Vec<StackedSeries<M>> }`, index 0 youngest.
- **Youngest-present-wins lithology.** `global_unit(p) -> (series_index, unit)`; a series is `present_at(p)` where `field < iso_top`; the floor is present everywhere. This single rule is correct for all relations — erode and onlap do **not** differ in the lithology lookup.
- **Erode vs. onlap = clip vs. no-clip.** `erode_masker_fields(i)` collects younger `Erode` fields; they become `ContactHalfSpace` maskers in the older series' `Scene` (present on the `field >= iso_top` / outside-the-younger-body side). Onlap/Conformable contribute no masker. *This is our "one declared bit," already implemented more precisely than our prose.* The `onlap_vs_erode_differ` test pins that the two relations produce different older-series meshes.
- **Meshing primitives.** `mesh_one` / `mesh_one_clipped` (one interface, clipped to visible region), `mesh_surfaces` (all interfaces, all series), each a single watertight `Scene` via `octree_dual_contour_scene`. Extent-box clip bounds the contour to true extent, not the octree cube.
- **Per-series fault blocks** (the `BlockModel` trait): `series_block_count`, `mesh_one_block`, `block_unit_count`, `present_unit_count`, `mesh_one_block_unit_solid`. **NB: today faults are nested *inside* each series.** §4 inverts this; the primitives survive, driven differently.
- **Generic over `M: SeriesModel (+ BlockModel)`** so the same logic drives the native kernel `Model` and the wasm `RemoteModel`.

---

## 3. Cross-cutting bodies: veins and intrusions **[NEW]**

This is the largest net-new piece. The current model has **no notion of cross-cutting** — every series is a stratigraphic top that clips downward along its own contact. A vein and an intrusion are not stratigraphic tops, and they do not "relate to the series below"; they relate to **everything they pass through**, derived from chronological position.

### 3.1 The third Axis-A case, or a separate element kind

A cross-cutting body needs a relationship that is neither erode, onlap, nor conformable. Two ways to model it; pick per taste, but the behavior is fixed:

- **Option A (extend `StackElement`):** keep `StackedSeries` as the stratigraphic case and add sibling variants:

```rust
enum StackElement<M> {
    Series(StackedSeries<M>),       // stratigraphic top: Relation + unbounded_above (§1,§2)
    Vein(Vein<M>),                  // cross-cutting tabular body
    Intrusion(Intrusion<M>),        // cross-cutting volumetric body
}
```

- **Option B (a `CrossCut` marker):** add `Relation::CrossCut` and a geometry-kind tag on the series. Lighter, but it strains `Relation` (which is otherwise purely the seam-below axis). **Recommend Option A** — cross-cutting bodies carry different geometry payloads (a HW/FW pair; a body envelope), so a distinct variant is cleaner than a flag.

### 3.2 Vein

A co-kriged hanging-wall/foot-wall pair occupying the tabular region between them. Structurally it is a 2-interface series with a different *interpretation*: the body is present *between* the two contacts (not below a single top), and it **cross-cuts** — it clips everything older it intersects, like an erode, but bounded to its tabular region rather than a half-space.

```rust
struct Vein<M> {
    model: M,                       // co-kriged field; HW and FW are two interfaces
    hanging_wall: InterfaceId,
    foot_wall: InterfaceId,
    reference: Option<InterfaceId>, // optional reference surface (Leapfrog-style)
    // present_at(p): foot_wall_iso < field < hanging_wall_iso  (between the pair)
    // masker contribution to older Scenes: the BETWEEN-region half-space pair
}
```

`present_at` becomes a band test, not a `field < iso_top` test. Its masker contribution to older series' Scenes is the intersection of two half-spaces (above FW, below HW) — reusing `ContactHalfSpace`, just two of them ANDed, instead of one.

### 3.3 Intrusion

A closed body envelope; present *inside* the envelope; cross-cuts everything older it intersects. Geometry comes from a body model (distance-to-contact or conceptual extent), not from thresholding a stratigraphic top.

```rust
struct Intrusion<M> {
    model: M,
    envelope: Envelope,                 // Single(InterfaceId) | Shell { inner, outer }
    geometry_model: IntrusionGeometry,  // DistanceToContact | ConceptualExtent
    // present_at(p): inside the envelope
    // masker contribution: the envelope interior half-space(s)
}
```

### 3.4 Derived cross-cut set (not declared) **[NEW]**

For lithology, cross-cutting bodies fit the existing `global_unit` rule **unchanged**: youngest-present-wins already gives "the intrusion claims every point inside it that no younger series claims first." Nothing to add for lithology.

For *meshing and reporting*, the set of older elements a body cuts is **derived** from chronological position + spatial intersection, never enumerated by the user:

```rust
impl Stack<M> {
    /// Older elements (index > i) whose extent this cross-cutting body intersects.
    /// Pure function of order + geometry; never stored. UI shows it read-only.
    fn cuts_below(&self, i: usize) -> Vec<usize>;
}
```

Mechanically this is the mirror of `erode_masker_fields`: a cross-cutting body at index `i` contributes its interior as a masker to the Scenes of all older series it reaches — exactly the existing erode wiring, with the body's "interior" half-space(s) in place of an erode top's single half-space. So the meshing path is *the same path*, with a different masker geometry.

### 3.5 The one punt (document, don't solve)

**Syn-depositional intrusions** — a body younger than some local units and older than others, not a clean single chronological position — are out of scope. Require modelling as multiple bodies, as Leapfrog does. State the limitation in a UI tooltip; do not attempt per-region body ordering. "Place at one chronology level, derive the rest from what's under it" is the v1 contract.

---

## 4. Faults: outer partition, stack instanced per block **[EXTENDS — inverts current nesting]**

**Decision (kept from our design):** faults are an **orthogonal outer partition**. The fault network decomposes the domain into blocks; the *entire stack* is instanced into each block. This is the inverse of the current code, where faults live *inside* each series (`BlockModel` per series).

### 4.1 Why invert

- A fault is a property of *space*, not of one stratigraphic series. Today every series carries its own block decomposition, which means fault geometry is replicated per series and must be kept consistent across them by construction. Lifting faults out makes the block partition single and authoritative.
- It matches the geological mental model and the Leapfrog architecture: define fault blocks, then the whole chronology is evaluated independently inside each block. A contact with no data in a block simply produces no surface there; evaluation proceeds.
- It removes the per-series fault-offset bookkeeping: which faults offset a unit falls out of which blocks the unit exists in, rather than being tracked per series.

### 4.2 Shape

```rust
struct FaultedModel<M> {
    fault_network: FaultNetwork,          // faults + fault-fault timing → block decomposition
    stack: Stack<M>,                      // the template, instanced per block
    block_overrides: HashMap<BlockId, BlockOverride>,
}

struct FaultNetwork {
    faults: Vec<Fault>,
    interactions: Vec<FaultInteraction>,  // younger truncates older — fault chronology only
    // fn blocks(&self) -> Vec<FaultBlock>   (derived partition)
}
struct FaultInteraction { younger: FaultId, older: FaultId }

struct BlockOverride {
    disabled_elements: Vec<ElementId>,    // turn a contact off in this block
    // (per-block data overrides optional, post-v1)
}
```

Two **independent** ordered structures: the fault chronology (`interactions`, for building blocks) and the stratigraphic chronology (`stack.series` order). They do not merge.

### 4.3 Reusing the existing per-block meshing

The inversion does **not** throw away the tested block meshing. The primitives `mesh_one_block`, `mesh_one_block_unit_solid`, `block_interface_isovalues`, `block_field`, `block_half_spaces` already contour a block's own field clipped flush to its fault planes. After the inversion they are driven by an **outer** block index over the whole stack rather than a per-series one:

```
for block in fault_network.blocks():
    for (i, element) in stack within this block:   // template instanced here
        mesh element in `block` using the block's field + fault-plane clip
        (younger-erosion / cross-cut maskers apply per-block, as today)
```

Concretely, the `BlockModel` trait moves up: instead of `series.model.block_field(b)`, the block partition supplies the field-per-block to each instanced element. The half-space clipping (`clip_mesh_to_half_spaces`, `block_half_spaces`) is unchanged. The erode/cross-cut maskers still use whole-model fields (erosion is geological and dispatches per-block correctly — this is already how `mesh_one_block` does it).

### 4.4 Per-block deviation

Default (no overrides) = the same template everywhere (the Leapfrog default). A `BlockOverride` lets a block disable a contact that has no expression there. Surfaced in the UI as a per-block editing mode (§6.7), not as edits to the template.

---

## 5. Surfaces as shared entities (DAG) **[NEW]**

The current code has no surface-entity store; interfaces belong to their series. To represent **correlative conformity** — one eroded surface that is the erode top of the stack below *and* the onlap floor of the stack above — without duplication, lift surfaces to referenced entities:

```rust
type SurfaceId = Uuid;
// Stack elements reference SurfaceIds; a surface may be referenced by two boundaries:
//   lower stack: relation = Erode, consumes it as a truncation top
//   upper stack: relation = Onlap, consumes the SAME SurfaceId as its lap-against floor
```

The data layer is a DAG; the UI presents a tree (§6). v1 may expose sharing only through the seam control, not a general graph editor.

---

## 6. UI specification (TypeScript / React) **[NEW — all of it]**

The model is a flat `Vec`; the hierarchy, the two drag semantics, the seam control, and validation surfacing are entirely presentation/interaction. The UI edits **declared state only** and renders what the Rust core derives and meshes (see §7 for the contract). The UI never derives geology.

### 6.1 Layout

```
┌───────────────────────────┬───────────────────────────┐
│  STRATIGRAPHIC HIERARCHY   │   3D / SECTION VIEW        │
│  (draggable tree)          │   (meshes from core)      │
│  ── FAULT NETWORK ──       │   block selector ▾        │
│  (separate ordered list)   │                           │
└───────────────────────────┴───────────────────────────┘
```

The fault network is a **visually separate** sub-panel — reinforcing §4: orthogonal partition, not a node in the stack.

### 6.2 The hierarchy tree

Vertical, draggable, **youngest at top**, two node levels:

- **Container nodes** (expandable): a series (conformal stack), a vein, or an intrusion. Shows kind icon, name, enable toggle, validation badge, derived-relation annotation, and the `unbounded_above` marker (a "floor" chip) where set.
- **Surface nodes** (children): one co-kriged interface each. Conformal stack → N children in isovalue order; vein → exactly two (HW/FW) as a fixed pair; intrusion → envelope surface(s).

| Type | Maps to | Accent | Carries erode/onlap? |
|------|---------|--------|----------------------|
| Conformal stack | `StackElement::Series` | blue | yes (its `relation`) |
| Vein | `StackElement::Vein` | purple | no — cross-cut (derived) |
| Intrusion | `StackElement::Intrusion` | red/orange | no — cross-cut (derived) |

(There is no separate "single surface" type — that is just a conformal stack with one interface.)

### 6.3 Two drag behaviours (must be visually distinct)

- **Reorder containers (between-container):** changes **chronology** → recompute + remesh. Strong full-width insertion bar; during drag, a transient annotation on adjacent boundaries ("D will now truncate C"). The consequential drag.
- **Reorder surfaces within a conformal stack:** changes the **isovalue sequence** of that one shared field only — local stacking, not chronology. *Visually different* drop affordance (indented, lighter bar) + tooltip: "reorders level sets within shared field — not chronology." Disabled for vein (fixed HW/FW pair) and intrusion.
- **Cross-container surface move:** allowed; triggers immediate re-validation of both fields. If a field becomes under-constrained, warn but **don't block** (user may be mid-edit).

One DnD library; branch drop semantics on `(dragNodeType, dropZoneType)` in a single explicit resolver.

### 6.4 The erode/onlap/conformable control (the declared Axis-A bit)

Rendered on the **boundary below** each conformal-stack container — a control in the gap *between* this container and the next-older one, not in a property panel:

```
   ┌── Conformal Stack: "Cretaceous seq." ──┐
   │   • Top Chalk / • Top Gault             │
   └──────────────────────────────────────────┘
        ⇩  [ erode | onlap | conformable ]   ← seam control (Axis A)
   ┌── Conformal Stack: "Jurassic seq." ─────┐
```

- Three-state for conformal stacks; sets `relation`.
- For vein/intrusion the seam shows a read-only `⨯ cross-cuts` chip (derived, not editable).
- The **floor** container (oldest, `unbounded_above`) shows no seam control below it (no series beneath) and a "fills upward / floor" marker instead.

### 6.5 Validation surfacing

Each container shows a badge: green ✓ / amber ! (under-constrained), hover detail listing which surfaces lack points or the field lacks orientation. A container that produces no surface in the selected block (data absent there) renders **dimmed** with an "absent in block N" note — mirroring §4.

### 6.6 Derived-relationship display

For the selected container, a read-only summary built entirely from core-derived data:

```
Granite intrusion (intrusion)
  chronology: position 1 of 6 (2nd youngest)
  cross-cuts: Cretaceous seq., Jurassic seq., Basement floor   (derived)
  geometry: conceptual extent
```

Never editable — it is the core's derivation. This is where the user *sees* that "what it cuts" came from "what's under it."

### 6.7 Fault network sub-panel

A separate ordered list (its own chronology):

- faults with up/down reorder = **fault–fault timing** → core rebuilds blocks;
- a "Blocks" readout (derived count) and a **block selector** driving the 3D view and per-block dimming/override state;
- per-block override mode: with a block selected, toggling a container's enable writes a `BlockOverride` rather than editing the template — clearly indicated ("editing block N override").

---

## 7. UI ↔ core contract **[NEW]**

TS types **generated** from the Rust schema (serde + ts-rs or equivalent) so they cannot drift. The UI holds an editable copy of declared state; mutations go to the core; the core returns derived data + meshes. Transport is an implementation choice (wasm-bindgen in-browser, or IPC/HTTP); the contract is transport-agnostic.

```ts
// declared state (editable in UI, owned by core)
type StackElementDTO =
  | { kind: "series"; meta: ElementMeta; surfaces: SurfaceId[];
      relation: "erode" | "onlap" | "conformable" | null;   // null = floor (no seam below)
      unboundedAbove: boolean }
  | { kind: "vein"; meta: ElementMeta; hangingWall: SurfaceId; footWall: SurfaceId; reference: SurfaceId | null }
  | { kind: "intrusion"; meta: ElementMeta; envelope: EnvelopeDTO; geometryModel: "distanceToContact" | "conceptualExtent" };

interface CoreApi {              // UI → core; one user gesture, one call
  reorderContainers(from: number, to: number): Promise<CoreUpdate>;   // chronology
  reorderWithin(elementId: ElementId, from: number, to: number): Promise<CoreUpdate>; // isovalues
  moveSurface(surfaceId: SurfaceId, to: ElementId | "newSeries"): Promise<CoreUpdate>;
  setRelation(elementId: ElementId, rel: "erode" | "onlap" | "conformable"): Promise<CoreUpdate>;
  setUnboundedAbove(elementId: ElementId, v: boolean): Promise<CoreUpdate>;
  setEnabled(elementId: ElementId, enabled: boolean, block?: BlockId): Promise<CoreUpdate>;
  reorderFaults(from: number, to: number): Promise<CoreUpdate>;
  addContainer(kind: StackElementDTO["kind"], at: number): Promise<CoreUpdate>;
  selectBlock(block: BlockId | null): void;   // view state, may stay UI-side
}

interface CoreUpdate {           // core → UI; UI never recomputes any of `derived`
  model: GeologicalModelDTO;
  derived: {
    cutsBelow: Record<ElementId, ElementId[]>;
    effectiveRelation: Record<ElementId, "erode" | "onlap" | "conformable" | "crossCut" | "floor">;
    validation: Record<ElementId, ContainerValidationDTO>;
    blocks: FaultBlockDTO[];
    absentInBlock: Record<BlockId, ElementId[]>;
  };
  meshes: MeshUpdate;            // for the active block
}
```

The UI store persists only editable declared state and treats `derived` as a read-only cache refreshed on every `CoreUpdate`. **The single source of truth for the grammar is Rust** — the UI cannot encode a second, drifting copy of erode/onlap/cross-cut logic.

---

## 8. Work ledger (what to actually build)

In rough dependency order:

1. **`Basement` decomposition [EXTENDS]. ✅ DONE (feature 001, T005–T010).** Split `Relation` → `relation: Option<Relation>` (erode/onlap/conformable; `None` = floor) + `unbounded_above: bool`; added `StackedSeries::floor()`. Updated `global_unit`, `erode_masker_fields`, `present_unit_count` to key on `unbounded_above`. All 37 `geofield` tests green; the missing-volume regression is now `floor_keeps_topmost_unit_which_daylights` (asserts on `unbounded_above` directly). The wasm ABI carries a transitional shim (code `3` → floor); the full `unbounded_above` ABI parameter is item 5 / task T014.
2. **Cross-cutting bodies [NEW]. ✅ DONE (feature 001, T029–T038).** `enum StackElement<M> { Series, Vein, Intrusion }` threaded through `Stack`; `Vein` band `present_at` + two-ANDed-half-space masker; `Intrusion` envelope (`Single`/`Shell`) interior `present_at` + masker; `cuts_below` derived from order + sampled overlap. `global_unit` unchanged (youngest-present-wins, uniform). Discriminating tests green (vein band-clip, intrusion interior, cuts_below). ABI D2 (`add_vein`/`add_intrusion`); web3 cross-cut scenario + derived chip verified by Playwright. **Logged bound:** the cross-cutting body's OWN 3D surface mesh (for the viewer) is a follow-up — `mesh_one_block` is series-only; the body's lithology/cut-set/masker-clip are done.
3. **Fault inversion [EXTENDS]. 🟡 MODEL DONE, GEOMETRY STUBBED (feature 001, T039–T045).** `FaultedModel` / `FaultNetwork` / `BlockOverride` in `crates/geofield/src/faulted.rs`; outer-block-index instancing (`enabled_elements`), local per-block overrides (proven non-template-editing), starved-element-doesn't-abort, `absent_in_block`. ABI D4 `fault_block_count()`. **`blocks()` STUBBED to one block** (research R3) — the partition GEOMETRY (deriving N blocks from fault planes) and the fault sub-panel UI (T046/T047) are the logged follow-up; the interface shape is real so consumers don't change.
4. **Surface entity store / DAG [NEW]. ✅ DONE (feature 001, T048–T050).** `crates/geofield/src/surface.rs`: `SurfaceId` newtype + `SurfaceStore` holding each surface once; one surface referenced by two boundaries (`TruncationTop` below + `LapAgainstFloor` above) with no duplication — proven by test. Data layer only; sharing is exposed through the seam control, NOT a graph editor (documented non-goal). `u64` id, not `Uuid` (no dep needed — Principle VI).
5. **ts-rs schema export + `CoreApi`/`CoreUpdate` [NEW]. 🟡 PARTIAL (feature 001, T016/T018).** `CoreUpdate` (declared `StackElementDto` + read-only `DerivedView`) defined in `crates/geofield/src/schema.rs` (serde) and exposed over the ABI as `WasmStack::core_update_json()`. Native round-trip + derivation-purity tests green. **ts-rs auto-generation is DEFERRED** (the crate is not fetchable in the current offline build env); the TS is hand-mirrored in `web3/src/HierarchyPanel/dto.ts` and the Rust round-trip test bounds drift. Scope: `series` kind + the derived view only (vein/intrusion/fault DTOs land with their model code, items 2–3). Full `CoreApi` mutation surface (reorder/move) beyond set-relation/set-floor is also pending those items.
6. **React hierarchy panel [NEW]. ✅ DONE & BEHAVIOR-VERIFIED (feature 001, T017/T020–T024, T057–T061).** `web3/src/HierarchyPanel/` — tree (youngest-top, containers + surfaces), the single dnd-kit drop resolver (two distinct semantics), three-state seam control + floor marker + read-only cross-cuts chip, validation badges + derived summary sourced only from `CoreUpdate.derived`, a read-only-cache store, and an embeddable barrel. Plus the `web3/` testbed (scenarios, worker, viewer, app). `tsc` + `vite build` clean and **Playwright 4/4 green** (constitution v1.1.0 functional gate): scenarios render, the seam round-trips through the WASM core, the two affordances are distinct. The fault sub-panel lands with item 3.

**Non-goals (document, don't solve):** syn-depositional intrusions (§3.5); fault displacement beyond timing-for-blocks; a general DAG editing UI; the interpolation/meshing engine internals (this is the configuration + model layer in front of the existing Rust meshing).

---

## 9. Why this is worth building (for reviewers)

The model primitive that matters — *a series is one co-kriged field* — already exists in `stack.rs`; so do youngest-present-wins and erode-vs-onlap-as-clip. What is genuinely new is: **cross-cutting bodies** (veins, intrusions) for which the current erode/onlap/conformable model has no slot; **inverting the fault nesting** so faults are an authoritative outer partition rather than per-series bookkeeping; the **two-axis cleanup** that retires `Basement` as a fake relation; and a **UI that surfaces the co-kriging group as the editable organizing primitive** — "the group you cokrige together is the folder" — with cut-sets derived rather than declared and exactly one free Axis-A bit per stratigraphic boundary. SKUA's stratigraphic column and LoopStructural's `StratigraphicColumn` nest units and unconformities internally, so the semantics are not unprecedented; presenting the co-kriging group as the visible, draggable container, and unifying conformal stacks with veins as the same kind of container holding different payloads, is the part that is.
