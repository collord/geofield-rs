# Contract: UI ↔ Core API

The configuration UI's contract with the Rust core. **Authoritative shape is
design doc §7** (`StackElementDTO`, `CoreApi`, `CoreUpdate`); this file records
the contract's *rules and invariants* for traceability, not a second copy of the
TypeScript (Constitution Principle III). Types are **generated** from the Rust
schema (research R1, ts-rs) so the two sides cannot drift.

## Direction & ownership

- The UI holds an **editable copy of declared state only**.
- Mutations go to the core; the core returns derived data + meshes.
- The UI treats `CoreUpdate.derived` as a **read-only cache**, refreshed on every
  response. The UI never recomputes geology (FR-017).
- One user gesture → exactly one core mutation (FR-018).

## Mutations (UI → core)

Each returns a fresh `CoreUpdate`. Names/shapes per design doc §7 `CoreApi`:

| Mutation | Effect | Maps to model transition |
|----------|--------|---------------------------|
| `reorderContainers(from, to)` | chronology change | recompute + remesh |
| `reorderWithin(elementId, from, to)` | isovalue sequence of one series | local only (disabled for vein/intrusion) |
| `moveSurface(surfaceId, to)` | cross-container surface move | re-validate both fields |
| `setRelation(elementId, rel)` | Axis-A seam (erode/onlap/conformable) | seam change |
| `setUnboundedAbove(elementId, v)` | Axis-B floor/fill-upward | floor toggle |
| `setEnabled(elementId, enabled, block?)` | visibility / per-block override | enable or `BlockOverride` |
| `reorderFaults(from, to)` | fault timing | blocks rebuilt |
| `addContainer(kind, at)` | new element | insert |
| `selectBlock(block?)` | view state | may stay UI-side (no recompute) |

## Response (core → UI): `CoreUpdate`

- `model`: the full declared-state DTO (echo of canonical state).
- `derived`: `{ cutsBelow, effectiveRelation, validation, blocks, absentInBlock }`
  — **all core-derived**, read-only. `effectiveRelation ∈ {erode, onlap,
  conformable, crossCut, floor}` (the UI shows the derived display, never edits
  it — design doc §6.6).
- `meshes`: mesh update for the active block.

## Invariants (testable)

1. **No UI-side geology.** Every value the UI renders about relationships, cut
   sets, validation, or block presence comes from `derived`. (SC-005; verified by
   the round-trip test — the UI store has no derivation code.)
2. **Single source of truth is Rust.** The grammar (erode/onlap/cross-cut/floor)
   exists once, in the core; the TS types are generated, not authored (FR-020).
3. **Floor has no seam.** A floor element (`unbounded_above`, `relation = null`)
   exposes no `setRelation` affordance and reports `effectiveRelation = floor`.
4. **Cross-cut is read-only.** Vein/intrusion report `effectiveRelation =
   crossCut` and `cutsBelow` derived; no `setRelation`/`reorderWithin` for them.
5. **Idempotent derived cache.** Re-issuing the same mutation yields an identical
   `derived` (pure function of declared state + geometry).

## Transport

Transport-agnostic by design (design doc §7). Concretely: in-browser
`wasm-bindgen` calls dispatched through the solver **worker** (research R5,
mirroring `web/worker.ts` + `kernel-bridge.ts`). The flat low-level wasm calls
(field eval, mesh-one) are unchanged; the hierarchy mutations/derived data are the
new generated-type surface layered over `WasmStack`.
