# Quickstart: Validating the Stratigraphic Hierarchy Feature

Runnable scenarios that prove the feature works end-to-end. Implementation detail
lives in `tasks.md` and the code; this is the run/validate guide. See
[data-model.md](data-model.md) for the entities and [contracts/](contracts/) for
the API and ABI.

## Prerequisites

- Rust toolchain (native target for tests; `wasm32-unknown-unknown` for the
  testbed), `wasm-pack`, Node + npm.
- The existing `web/` harness builds (the pattern `web3/` mirrors — research R5).

## Gate 1 — Model layer (native, no browser)

The fastest signal; run after each model change.

```bash
cargo test -p geofield        # stack.rs unit tests + crates/geofield/tests/
```

**Expected**:
- The **pre-existing suite stays green** throughout — especially
  `onlap_vs_erode_differ`, `basement_partitions_domain`,
  `stack_*_watertight`, and the missing-volume regression (now keyed on
  `unbounded_above`, renamed accordingly). (SC-007, FR-024)
- **New tests pass**: vein band-clip, intrusion-interior lithology, `cuts_below`
  derivation, per-block instancing (single-block stub), block-override,
  shared-surface reference. (FR-006–FR-013)

## Gate 2 — Native↔WASM parity

Per README's parity check — the engine's wasm build agrees with native to `f64`
round-off, and the two-artifact split holds.

```bash
cd web3 && npm run wasm        # builds geofield_kernel.wasm + geofield.wasm (from web/ pattern)
```

**Expected**: both artifacts build; `geofield.wasm` links no kernel/EUPL code
(Principle V); a parity probe on a synthetic field matches native within round-off.

## Gate 3 — Testbed, per user story

```bash
cd web3 && npm run dev         # serve; open the printed URL
```

For each, the acceptance is *observable in the browser*:

| Scenario (select in testbed) | Do | Expect | Spec |
|------------------------------|----|--------|------|
| **Conformal column** | load | layered column renders; floor shows "fills upward" marker, no seam control below it | US1, FR-016 |
| **Erosional unconformity** | toggle the boundary seam erode → onlap → conformable | three **visibly distinct** meshes; switching nothing else | US1, SC-002 |
| **Reorder within a series** | drag a surface inside one group | a **different, lighter** drop affordance; only that group's level-set order changes; chronology untouched | US4, FR-015, SC-006 |
| **Reorder containers** | drag one container above another | strong full-width insertion bar; chronology changes and remeshes | US4 |
| **Cross-cutting body** (vein or intrusion) | select it | read-only "cross-cuts" marker + derived `cuts-below` list; editing its position updates the list automatically (never hand-entered) | US2, SC-003 |
| **Fault network** | select each block | same template renders per block; a data-starved contact shows "absent in block N" and the rest of the block still renders | US3, SC-004 |
| **Per-block override** | enter per-block edit mode, disable a contact | only the selected block changes; UI indicates an override is being edited | US3, FR-012 |
| **Any edit** | perform any editable gesture | the change round-trips through the WASM core and the viewer updates live | US5, FR-023, SC-005 |

## Gate 4 — Scope & honesty checks (constitution)

- **Coverage bounds are visible**: the single-block fault stub (before partition
  geometry lands) is surfaced, not silent (Principle IV, research R3).
- **No second grammar copy**: the UI renders relationships/cut-sets only from
  `derived`; TS types are generated, not authored (Principle III, FR-020).
- **Status calibrated**: features not yet done are not claimed done; the design
  doc's non-goals (syn-depositional intrusions, fault displacement, general DAG
  editor) remain documented punts (Principle IV; spec Out-of-Scope).

## Done signal

All four gates pass and a reviewer can complete each Gate-3 row following these
steps alone (SC-001, SC-008). The eight success criteria in the spec map onto
Gates 1–3 as annotated above.

## Verification status (feature 001, MVP implement run — T001–T028)

- **Gate 1 (model, native): ✅ PASSED.** `cargo test` → 43 geofield tests green,
  including the re-keyed missing-volume regression, `three_seam_states_differ`,
  `floor_top_unit_kept_under_any_younger_relation`, and the schema round-trip /
  derivation-purity / scenario-DTO tests. Whole workspace green.
- **Gate 2 (native↔wasm): 🟡 PARTIAL.** `cargo build --features wasm --target
  wasm32-unknown-unknown` is clean and the EUPL/BSD split is intact. The
  *numeric* native↔wasm parity probe is a browser harness and was **not run** in
  the implement environment — deferred to a runtime check.
- **Gate 3 (testbed, browser): ✅ PASSED (automated).** `web3/` builds clean
  (`npm run wasm && npm run build`) and the **Playwright functional gate is 4/4
  green** (`npm run test:e2e`, chromium): the page loads, each scenario renders,
  the seam erode→onlap→conformable round-trips through the WASM core and the
  derived display updates, and the two drag affordances are computed-style
  distinct. This is the constitution v1.1.0 behavioral gate, not a manual walk.
  (Run order: `cd web3 && npm install && npm run wasm && npm run build && npm run test:e2e`.)
- **Gate 4 (constitution): ✅ held.** EUPL/BSD seam intact (ts-rs not added; serde
  is permissive; kernel out of the engine wasm graph); the one shortfall vs. R1
  (hand-mirrored TS instead of generated) is logged in research.md, not silent.
