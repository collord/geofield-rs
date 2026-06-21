# Contract — Engine / ABI seam

**Feature**: `003-topography-lithology-solids` · **Phase 1** · BSD `geofield` crate only (kernel untouched)

This is the **wasm-reachable** contract: what the core adds, and the parity/invariant obligations that
back each addition (Principle I/V gates). All citations are to the baseline mapped in Phase 0.

---

## C1. `add_topography` — new ABI entry

**Adds**: a `WasmStack::add_topography(...)` export, sibling of `add_series`
([wasm.rs:477](../../../crates/geofield/src/wasm.rs#L477)).

**Shape** (mirrors `add_series` minus relation/unbounded, plus the singleton/youngest rules):
- Inputs: the ground surface's sample points + optional orientations + kernel options (same fit
  parameters `add_series` takes). **No** `relation`, **no** `unbounded_above`.
- Effect: fits the ground field, constructs a `Topography` element, and **inserts it at index 0**
  (youngest). The first `add_*` is youngest today; topography must be youngest **regardless of call
  order**, so it is inserted at the front, not appended.
- **Rejects a second topography**: if a `Topography` already exists in the stack, the call is a
  no-op error (the builder enforces the singleton invariant — it is not left to the UI alone).

**Contract guarantees**:
1. After `add_topography`, the region above the ground field evaluates to Air (no rock) in
   `global_unit`; below it, evaluation proceeds as before.
2. Every older element is clipped to below the ground (topography contributes a
   `Masker::Halfspace { below: false }` to each older element's `clip_maskers`,
   [stack.rs:415-467](../../../crates/geofield/src/stack.rs#L415)).
3. `add_topography` called when one exists does not add a second.

**Gate**: native↔wasm parity on a scenario with topography (the capped evaluation agrees to `f64`
round-off); a unit test that a second `add_topography` is rejected.

---

## C2. Clip-derived top-unit keep/drop (the R2 net-new logic)

**Changes**: `global_unit` ([stack.rs:388](../../../crates/geofield/src/stack.rs#L388)) and
`present_unit_count` ([stack.rs:711](../../../crates/geofield/src/stack.rs#L711)) **stop reading
`unbounded_above`**. The keep-vs-drop decision for a series' top unit is derived from **whether a
younger element caps the series at that point**.

**The rule** (the single source of truth — lives once, in Rust):
- A series **keeps** its top unit where **nothing younger caps it there** — the floor (nothing above
  it) and an exposed top under open sky both keep their top unit.
- A series **drops** its top unit (→ Air/void) where **a younger element caps it** — topography
  cutting below the series' top contact, or a younger erode/unit doing the same.
- Equivalently: the old `unbounded_above => n` / `else => n - 1` branch in `present_unit_count`
  becomes a query on the **same `clip_maskers(i)` set** that already bounds the solid mesh — is a
  younger contributor present above this series' top contact at the queried column?

**Why this is net-new, not a refactor** (the brief's "falls out of the clip result" is optimistic):
today neither `global_unit` nor `present_unit_count` consults the clip set when deciding the top unit;
they read the flag directly. The derivation must be *written*. (research.md R2.)

**Contract guarantees**:
1. A series with `unbounded_above = false` **and no younger cap** still fills upward (the new
   universal default) — proving the flag is inert. *(New oracle test — SC-009.)*
2. The floor (nothing younger above it) keeps its top unit. *(Existing behaviour, now clip-driven.)*
3. A series capped by topography drops its top unit to Air where topography cuts below it. *(New.)*
4. All pre-existing `present_unit_count` tests pass unchanged (behaviour preserved). *(SC-009.)*

**Gate**: `cargo test` (the four guarantees above) + native↔wasm parity on unit enumeration. The
inert-flag test is an **oracle** (it pins the new universal behaviour), not a self-consistency guard.

---

## C3. `DerivedView.units` — per-element unit coordinates

**Adds**: `units: Vec<(String, Vec<UnitInfo>)>` to `DerivedView`
([schema.rs:110](../../../crates/geofield/src/schema.rs#L110)); `UnitInfo { index: usize }`. Mirrored
in [dto.ts:41](../../../web3/src/HierarchyPanel/dto.ts#L41) by hand (no `ts-rs`).

**Contract guarantees**:
1. Every element id present in `model` appears as a key in `units`.
2. Each element's `Vec<UnitInfo>` is its **derived** unit list, ordered youngest→oldest within the
   element, **after clipping** (so the count reflects topography/younger-erode removal, not the raw
   N+1).
3. `(elementId, index)` is **stable** across re-derivations for the same geometry — it is the key the
   UI's lithology assignment points at, so it must not renumber under unrelated edits.
4. The Air unit is **not** listed (Air is the UI's distinguished member, not a core rock unit).
5. The core puts **no lithology name/colour** here — coordinates only (Principle III seam).

**Gate**: `cargo test` that `units` matches `present_unit_count` for representative stacks (conformable
N-contact series → N+1 before clip; capped series → fewer); native↔wasm parity that `units` is
identical across builds; the web3 hand-mirror is exercised by the Playwright ladder spec.

---

## C4. `unbounded_above` — retained, accepted, ignored

**Unchanged slot, changed semantics**: `StackElementDto.unbounded_above`
([schema.rs:68](../../../crates/geofield/src/schema.rs#L68)) and the `add_series` parameter
([wasm.rs:477](../../../crates/geofield/src/wasm.rs#L477)) **remain** and are **accepted** on import,
but the engine no longer reads them (C2). Documented deprecated in the schema.

**Contract guarantees**:
1. An existing serialized scenario carrying `unbounded_above` (either value) parses without error.
2. Its fitted result is identical to before for the floor/capped cases (behaviour preserved via C2).
3. No engine code path branches on the flag after this feature.

**Seam left** (Principle IV): removing the slot from the serialized form is later cleanup, Out of
Scope here. The slot stays, documented deprecated.

---

## C5. Per-unit solid — Air skip + coordinate passthrough

**Reuses** `mesh_one_block_unit_solid` → `octree_dual_contour_scene` with `cfg.cap = true`
([stack.rs:736](../../../crates/geofield/src/stack.rs#L736), [octree.rs:877](../../../crates/geofield/src/octree.rs#L877))
**verbatim**, with two additions:
1. **Topography participates** as a universal top masker for every exposed volume — already the
   mechanism (it's the youngest erode; it's in `clip_maskers`).
2. **The Air unit is skipped** — the solid mesher emits no payload for the above-topography unit.

The returned payload already carries `(series, block, unit)` indices; **no lithology id crosses the
boundary** — the UI tags the payload from the catalog assignment (contract `catalog-and-render.md`).

**Contract guarantees**:
1. Each below-topography `(element, unit)` cell meshes as a closed, watertight solid.
2. The **multi-clip volume** — capped by topography above, bounded by its floor contact below, and
   punched by an intrusion — is watertight: no holes, manifold edges, consistent winding. *(The named
   test case — SC-005.)*
3. No solid is emitted for the Air unit.

**Gate**: **mesh invariant tests** (crack-free / manifold / winding, the engine's existing invariant
style) on the multi-clip volume; native↔wasm parity on the solid geometry. This is geometry, validated
by invariant — **never** by screenshot, **never** against a GemPy reference (Principle I).

---

## Summary — what the engine adds, and its gate

| Addition | File:line | Backing gate |
|----------|-----------|--------------|
| `add_topography` (singleton, youngest, erode-clip) | wasm.rs:477 (sibling) | parity + reject-2nd unit test |
| Clip-derived top-unit keep/drop (flag inert) | stack.rs:388, :711 | inert-flag **oracle** test + preserved-behaviour tests + parity |
| `DerivedView.units` coordinate | schema.rs:110 / dto.ts:41 | matches `present_unit_count`; parity; ladder Playwright |
| `unbounded_above` accepted-but-ignored | schema.rs:68 | legacy-scenario parse test |
| Per-unit solid: topo masker + Air skip | stack.rs:736 / octree.rs:877 (reused) | watertightness invariant on the multi-clip volume + parity |
