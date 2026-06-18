# Contract: WASM ABI Delta

The additive changes to `crates/geofield/src/wasm.rs` (the `#[cfg(feature="wasm")]`
ABI) needed for the hierarchy feature. **Additive and `cfg`-gated** — no change to
the EUPL/BSD two-artifact split; the engine still reaches the kernel oracle across
the module boundary (Constitution Principle V). Existing exports stay
backward-compatible where the old testbeds (`web/`, `web2/`) still use them.

## Current surface (unchanged, for reference)

`WasmStack`: `add_series(relation:number, flat f64 arrays, fault)`,
`series_count`, `mesh_interface_octree`, `series_block_count`,
`mesh_series_block_interface`, `block_unit_count`, `present_unit_count`,
`mesh_series_block_unit_solid`, `lithology_label`, `series_at`. `relation` is the
integer `0=Erode 1=Onlap 2=Conformable 3=Basement`.

## Delta

### D1 — Two-axis relation (retire `Basement` integer)

- `add_series` (or a new `add_element`) gains an **`unbounded_above: bool`** and
  the `relation` integer narrows to `0=Erode 1=Onlap 2=Conformable` (the floor is
  `relation = none` + `unbounded_above = true`). The old `3=Basement` is mapped
  during migration so existing callers don't break in the transition, then
  retired.
- **Why additive, not overloaded:** keeping `unbounded_above` a separate flag
  rather than a 4th integer code is the same Principle III correction as the Rust
  side — the bit is honest, not smuggled into the enum.

### D2 — Cross-cutting elements

- New constructors: `add_vein(field data, hanging_wall_id, foot_wall_id,
  reference?)` and `add_intrusion(field data, envelope, geometry_model)`.
- New mesh path reuses `mesh_one_block*` with the body's masker geometry (two
  ANDed half-spaces for a vein; envelope interior for an intrusion) — **the same
  contour path**, different maskers (design doc §3.4; Principle VI).

### D3 — Derived-data getters

- Read-only getters mirroring `CoreUpdate.derived`: `cuts_below(element) ->
  indices`, `effective_relation(element) -> code`, `validation(element)`,
  `blocks()`, `absent_in_block(block)`. Pure functions of declared state.

### D4 — Fault network (outer partition)

- A `WasmFaultedModel` (or extension of `WasmStack`) exposing `blocks()` and
  per-block instanced meshing. **`blocks()` returns one whole-domain block first**
  (research R3) — the interface is real, the partition geometry is a logged
  follow-up.

### D5 — Generated DTO bridge

- The hierarchy mutation/derive round-trip crosses as **ts-rs-generated** JSON
  DTOs (research R1), not new flat-array signatures. The wasm functions take/return
  serialized DTOs for the high-level hierarchy operations; the low-level
  field-eval/mesh calls keep their flat `f64[]`/`u32[]` typed-array signatures
  (fast path, unchanged).

## Constraints

- All additions are `#[cfg(feature="wasm")]`; none pull the kernel into the
  engine's wasm graph (Principle V).
- The generated-DTO export tooling (ts-rs) is a dev/test dependency, never in the
  shipped wasm graph.
- Existing `web/`/`web2/` ABI calls remain callable through the migration window;
  removal of the `Basement` integer is the final, deliberate step once both
  testbeds are off it.
