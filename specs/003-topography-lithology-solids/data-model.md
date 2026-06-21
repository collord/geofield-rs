# Phase 1 — Data Model: Topography, Lithology Catalog & Solid Volumes

**Feature**: `003-topography-lithology-solids` · **Date**: 2026-06-20

This feature's data lives in **two places that must not blur**: the **core** (Rust, wasm-reachable)
declares irreducible geometry and *derives* unit coordinates; the **UI** (web3 project document) holds
lithology identity metadata that never crosses the wasm boundary. The seam is the whole point
(Principle III). Each entity below is tagged with where it lives.

---

## 1. Topography (core — `StackElement` variant)

**Lives in**: `crates/geofield/src/stack.rs` (variant), `schema.rs` (DTO case), `wasm.rs` (`add_topography`).

A fourth `StackElement` variant, sibling of `Series`/`Vein`/`Intrusion`
([stack.rs:250](../../crates/geofield/src/stack.rs#L250)):

```rust
enum StackElement<M> {
    Series(StackedSeries<M>),
    Vein(Vein<M>),
    Intrusion(Intrusion<M>),
    Topography(Topography<M>),   // NEW
}

struct Topography<M> {
    model: M,    // one co-kriged field, a single interface (the ground surface)
    // upper side is ALWAYS Air; lower side is "everything below, present".
    // Reuses Masker::Halfspace { below: false } — every older element clips to below the ground.
}
```

**Invariants (engine-enforced, not user-asserted)**:

| Invariant | Where enforced |
|-----------|----------------|
| At most one `Topography` per stack | `add_topography` builder rejects a second (wasm.rs) |
| Always index 0 (youngest) | `add_topography` inserts at front; compile seam emits it first |
| Never reorderable below any element | UI resolver guard (no Rust reorder API exists — see R4) |
| Upper unit is Air (never meshed as a solid) | `mesh_one_block_unit_solid` skips the Air unit (stack.rs) |
| Clips all older elements to below it | joins `clip_maskers(i)` as a `Halfspace` (stack.rs:415) |

**Relationships**: It is the youngest erode surface; it contributes a top masker to *every* older
element (series/vein/intrusion). It owns exactly one interface (the ground). It is not cross-cut by
anything (nothing is younger).

**DTO** (`schema.rs` `StackElementDto`, mirrored `dto.ts`): a `Topography { id, name, surface }`
case. No `relation`, no `unbounded_above`, no second interface.

---

## 2. Unit coordinate (core-derived — `DerivedView.units`)

**Lives in**: `crates/geofield/src/schema.rs` `DerivedView` ([schema.rs:110](../../crates/geofield/src/schema.rs#L110)),
hand-mirrored in [dto.ts:41](../../web3/src/HierarchyPanel/dto.ts#L41).

The core already computes unit counts (`present_unit_count`/`block_unit_count`); this exposes them as
ordered coordinates so the UI can render the ladder and key assignments. **The core supplies
coordinates only — no names, no colours.**

```rust
struct DerivedView {
    effective_relation: Vec<(String, EffectiveRelation)>,   // EXISTS
    validation:         Vec<(String, ContainerValidation)>, // EXISTS
    cuts_below:         Vec<(String, Vec<String>)>,         // EXISTS
    units:              Vec<(String, Vec<UnitInfo>)>,       // NEW: (elementId, ordered units)
}

struct UnitInfo {
    index: usize,   // youngest→oldest within the element; stable per (element, index)
}
```

```ts
// dto.ts — hand-mirror
interface UnitInfo { index: number }
interface DerivedView {
  effectiveRelation: [string, EffectiveRelation][];
  validation:        [string, ContainerValidation][];
  cutsBelow:         [string, string[]][];
  units:             [string, UnitInfo[]][];   // NEW
}
```

**Derivation rule (the R2 net-new logic)**: the unit list for an element is its derived units **after
clipping** — a conformable series with N contacts has at most N+1 units, **minus** any dropped because
a younger element (including topography) caps it there. The keep/drop of the *top* unit is the bit
that moves off the `unbounded_above` flag onto "is a younger element present above this series' top
contact." (See the contract for the exact rule.)

**Relationships**: keyed by `elementId` (the stable string id the schema already uses). The
`(elementId, unitIndex)` pair is the **volume coordinate** the catalog assignment points at.

**Validation**: every element appears in `units`; an element with no meshable interface yields an
empty unit list (consistent with today's `validation` error path). The Air unit (above topography) is
**not** a `UnitInfo` in any element's list — Air is the UI's distinguished member, assigned
automatically, not a core-enumerated rock unit.

---

## 3. Lithology (UI — catalog member)

**Lives in**: `web3/src/HierarchyPanel/project.ts`. **Never crosses the wasm boundary.**

```ts
type LithologyId = string;

interface Lithology {
  id: LithologyId;
  name: string;
  color: string;                       // hex; drives solid colour + swatch
  kind: "air" | "unknown" | "inferred" | "user";   // provenance → editability
}
```

**Provenance rules** (the `kind` field controls editability):

| kind | name | color | deletable | source |
|------|------|-------|-----------|--------|
| `air` | "Air" (fixed) | transparent | no | distinguished singleton; assigned to every above-topography / uncapped-void volume; **never meshed** |
| `unknown` | "Unknown" (fixed) | neutral grey | no | distinguished singleton; default for every below-topography series-internal unit; **is meshed** (rock of unspecified type) |
| `inferred` | = body name | body-derived/editable | via the body only | one per vein/intrusion; auto-assigned to that body's interior; renames/resyncs with the body; removed when the body is deleted |
| `user` | user-set | user-set | yes | user-added; available in every unit dropdown |

**State transitions (inferred entries)**:
- body **added** → inferred lithology created, auto-assigned to the body's interior unit.
- body **renamed** → inferred lithology name re-syncs.
- body **deleted** → inferred lithology removed; any assignment pointing at it falls back to `unknown`.

**Invariant**: every below-topography volume resolves to *some* lithology (Unknown at worst); every
above-topography volume resolves to Air — the model is never blocked on assignment (FR-020).

---

## 4. LithologyCatalog + Assignment (UI — project document)

**Lives in**: `web3/src/HierarchyPanel/project.ts`, serialized into the project JSON.

```ts
interface LithologyCatalog {
  lithologies: Lithology[];                       // includes the fixed Air + Unknown members
  assignment: Record<string, LithologyId>;        // key = `${elementId}:${unitIndex}`
}
```

**Assignment key** = the core's volume coordinate `(elementId, unitIndex)` from `DerivedView.units`.
**Many-to-one**: multiple keys may map to the same `LithologyId` (two sandstone slabs → "Sandstone").
This is the *entire* correlation mechanism — no automatic detection (FR-019, spec §8).

**Relationships**: `assignment` values reference `lithologies[].id`. A dangling reference (e.g. the
body behind an inferred entry was deleted) resolves to `unknown` at read time, never an error.

**Persistence**: the `LithologyCatalog` is a field on the `Project` document
([project.ts:75](../../web3/src/HierarchyPanel/project.ts#L75)), so it rides the **existing** JSON
export/import + localStorage autosave ([project.ts:234-287](../../web3/src/HierarchyPanel/project.ts#L234)).
No separate store. Round-trip restores names, colours, and assignments (FR-021, SC-007).

---

## 5. Solid payload tagging (worker — render seam)

**Lives in**: `web3/src/protocol.ts` `SurfacePayload` + `web3/src/worker.ts`.

A solid is one `(element, unit)` cell's watertight mesh. The core already returns one payload per
cell carrying `(series, block, interface/unit)` indices ([protocol.ts:55](../../web3/src/protocol.ts#L55)).
In solid mode the worker maps the payload's `(elementId, unitIndex)` through `catalog.assignment` →
`LithologyId`, and tags the payload (or carries the coordinate for a UI-side lookup):

```ts
interface SurfacePayload {
  series: number; block: number; interface: number;   // EXISTS
  positions: Float32Array; normals: Float32Array; indices: Uint32Array;  // EXISTS
  lithologyId?: LithologyId;   // NEW (solid mode only); Air-tagged payloads are not emitted
}
```

**Rule**: Air volumes are **not emitted** (no payload). Every emitted solid carries a non-Air
`lithologyId`. The viewer colours from `catalog.lithologies` by id; OBJ groups by `lithologyId`.

---

## 6. Deprecated state — `unbounded_above` (inert)

**Lives in**: `schema.rs` `StackElementDto` ([schema.rs:68](../../crates/geofield/src/schema.rs#L68)),
`SeriesJson` ([protocol.ts:17](../../web3/src/protocol.ts#L17)), `add_series` ABI (wasm.rs), and the
project document's `series.unboundedAbove` ([project.ts:57](../../web3/src/HierarchyPanel/project.ts#L57)).

**State**: the slot is **retained and accepted on import** (existing scenarios parse), but:
- the **engine ignores it** — `global_unit`/`present_unit_count` no longer read it (R2);
- the **compile seam stops writing it meaningfully** (or writes a constant; documented deprecated);
- the **UI control is hidden** (the "fills upward (floor)" checkbox at
  [HierarchyPanel.tsx:195-204](../../web3/src/HierarchyPanel/HierarchyPanel.tsx#L195) is removed).

**Seam left** (Principle IV): the slot stays, documented deprecated; removing it from the
serialized form is later cleanup (Out of Scope). A legacy scenario carrying the flag loads unchanged
and behaves identically because the *behaviour* (floor keeps its top unit; capped series drop theirs)
is preserved, now clip-driven.

---

## Entity relationship summary

```
                         ┌─────────────── CORE (Rust / wasm) ───────────────┐
  StackElement::Topography ──(Halfspace clip)──▶ every older StackElement
        │ owns 1 interface (ground)                      │
        │                                                ▼
        │                                   present_unit_count (clip-derived)
        │                                                │
        └──────────────── DerivedView.units: (elementId, [UnitInfo{index}]) ─┘
                                                 │  (coordinate only — no names)
═════════════════════════════════════ wasm boundary ═══════════════════════════════════
                                                 │
                        ┌──────────────── UI (web3 project document) ─────────┐
   LithologyCatalog ─── assignment: `${elementId}:${unitIndex}` ──▶ Lithology │
        │  (Air, Unknown fixed; inferred from bodies; user-added)             │
        ▼                                                                     │
   SurfacePayload.lithologyId (solid mode) ──▶ Viewer colour / OBJ group ─────┘
                       (Air payloads not emitted)
```
