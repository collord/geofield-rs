---
description: "Task list for Stratigraphic Hierarchy — Model, UI, and Testbed"
---

# Tasks: Stratigraphic Hierarchy — Model, UI, and Testbed

**Input**: Design documents from `specs/001-stratigraphic-hierarchy/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md
**Design source of record**: `docs/stack_hierarchy_spec.md` (authoritative for signatures; Constitution Principle II/III)

**Tests**: REQUIRED for this feature. The spec makes invariant/regression tests a
hard requirement (FR-024, SC-007) and Constitution Principle I (NON-NEGOTIABLE)
forbids unbacked correctness claims. Per project culture, model tests are written
with the code they pin and labelled as guard vs. oracle.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: US1–US5 (Setup/Foundational/Polish carry no story label)
- File paths are exact.

## Path Conventions

- Model/engine: `crates/geofield/src/`, tests in `crates/geofield/tests/` and inline `#[cfg(test)]`
- Shared interface types: `crates/geofield-types/src/`
- Testbed + reusable UI: `web3/` (mirrors `web/` — research R5)

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Stand up the testbed shell and the schema-generation toolchain before model work consumes them.

- [X] T001 Scaffold `web3/` from the `web/` harness pattern: `web3/package.json` (scripts `wasm:kernel`/`wasm:engine`/`wasm`/`dev`/`build`, `--out-dir` retargeted to web3, + `@dnd-kit` deps for R4), `web3/index.html`, `web3/vite.config.ts`, `web3/tsconfig.json`; `.gitignore` block added (research R5)
- [X] T002 [P] Copied the model-AGNOSTIC two-wasm transport into `web3/src/`: `kernel-bridge.ts` verbatim + a placeholder `main.tsx`. **`worker.ts`/`protocol.ts`/`Viewer.tsx` deliberately NOT copied** — they encode the old flat `relation: number` model being retired; rebuilt against the new contract in US4/US5 (T019/T020/T027). Copying-to-rewrite would be churn (Principle VI).
- [X] T003 [P] **DEFERRED to T016 (US4).** ts-rs has no DTOs to derive on until the hierarchy DTOs exist (T016). Adding an unused dependency at the foundational gate is dead scaffolding (Principle VI). Re-scoped into T016.
- [X] T004 **DEFERRED to T016 (US4).** A freshness check against types that don't exist yet would pin nothing — a test masquerading as coverage (Principle IV). Lands with the DTOs in T016.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The two-axis model cleanup. This is design-doc item 1 and de-risks everything downstream (Constitution Principle VI) — it MUST complete before any user story.

**⚠️ CRITICAL**: No user-story work begins until this phase is complete and the existing suite is green.

- [X] T005 Confirmed the migration shape against `docs/stack_hierarchy_spec.md` §1–§2: `Relation::Basement` → `relation: Option<Relation>` (`None` = floor, the doc's recommended cut) + `unbounded_above: bool`. No deviation.
- [X] T006 `crates/geofield/src/stack.rs`: removed `Relation::Basement`; `StackedSeries` now carries `relation: Option<Relation>` + `unbounded_above: bool`; added `StackedSeries::floor()` constructor (`new()` unchanged for bounded series).
- [X] T007 `crates/geofield/src/stack.rs`: `global_unit` (present via `s.unbounded_above`), `erode_masker_fields` (matches `Some(Relation::Erode)`), `present_unit_count` (branches on `unbounded_above`, not the retired variant) — with honest-property docs.
- [X] T008 `crates/geofield/src/stack.rs`: module docs rewritten to the two-axis model; test helpers (`two_series_stack`, `basement_under_erosion`, `faulted_single_series_stack`, four-series, extent-clip) construct the floor via `StackedSeries::floor`. Also migrated `tests/scenario_repro.rs` and the `wasm.rs` ABI shim (D1-partial; full param deferred to T014).
- [X] T009 Re-keyed the missing-volume regression → `floor_keeps_topmost_unit_which_daylights`, asserting on `unbounded_above`/`relation == None` directly (not the old proxy). `onlap_vs_erode_differ`, `basement_partitions_domain`, all `*_watertight` green (FR-024, SC-007).
- [X] T010 `cargo test -p geofield`: **37 passed, 0 failed** (1 ignored diagnostic). `cargo build -p geofield --features wasm --target wasm32-unknown-unknown`: **Finished, clean** (Principle V satisfied) (quickstart Gate 1+2).

**Checkpoint**: Model axes are clean and honest; the floor is no longer a fake relation. User stories can begin.

---

## Phase 3: User Story 1 — Configure a layered stratigraphy and see it mesh (Priority: P1) 🎯 MVP

**Goal**: A geologist organizes interfaces into ordered groups, sets the three-state seam, marks the floor, and the model re-meshes correctly.

**Independent Test**: Load a 2+ group synthetic stack; toggle a boundary seam erode→onlap→conformable; confirm three distinct meshes and floor-present-everywhere — verifiable in `cargo test` and in the testbed (quickstart Gate 1 + Gate 3 rows 1–2).

### Tests for User Story 1 ⚠️ (write with the code they pin)

- [X] T011 [P] [US1] `stack::tests::three_seam_states_differ` — erode (clips) vs onlap/conformable (don't); three distinct older-bed extents. GREEN. (SC-002)
- [X] T012 [P] [US1] `stack::tests::floor_top_unit_kept_under_any_younger_relation` — floor keeps all units under younger erode/onlap/conformable. GREEN. (FR-003)

### Implementation for User Story 1

- [X] T013 [US1] Verified intact post-migration: 12/12 stack tests green (watertight, erode-vs-onlap clip, extent-clip, daylighting). No rebuild — only the two-axis-created paths got new coverage (T011/T012).
- [X] T014 [US1] `wasm.rs` ABI D1: `add_series` gained `unbounded_above: bool`; `relation` narrowed to `0/1/2`; legacy `3=Basement` **retired** (hard-cut per user decision). `relation_from_code` no longer yields a floor. wasm target builds clean.
- [X] T015 [US1] wasm32 target build clean after D1. **Note (honest):** full native↔WASM numeric parity is a *browser harness* (README) not runnable here; the verifiable proxy — wasm compiles + native suite green — passed. Browser parity probe deferred to a runtime check.

**Checkpoint**: The stratigraphic core is configurable and correct end-to-end at the model + ABI layer.

---

## Phase 4: User Story 4 — Reusable hierarchy UI editing declared state (Priority: P1)

**Goal**: A draggable hierarchy tree (youngest-top, containers + surface children) with two visually distinct drag behaviors, the seam control, floor marker, validation/derived display — editing declared state only.

**Dependency note**: Requires US1's model + ABI (T013–T014) and the schema round-trip (T016) before the panel has anything truthful to drive. Independently *testable* once those land.

**Independent Test**: Drive every editable gesture; confirm one-mutation-per-gesture, derived-only rendering, and visually unmistakable drag behaviors (quickstart Gate 3 rows 3–4; SC-005, SC-006).

### Tests for User Story 4 ⚠️

- [X] T016 [US4] `crates/geofield/src/schema.rs`: DTOs (`StackElementDto`, `RelationDto`, `EffectiveRelation`, `DerivedView`, `CoreUpdate`) + `Stack::{to_dtos, derived_view, core_update}`. Round-trip + purity tests GREEN (`core_update_json_round_trips`, `derived_view_is_pure_and_correct`, `dtos_carry_both_axes`). **ts-rs auto-gen DEFERRED** (not fetchable offline) — TS hand-mirrored in `dto.ts` with a round-trip test bounding drift; logged in research.md R1 update.
- [X] T017 [P] [US4] `web3/src/HierarchyPanel/hierarchy.css`: `.drop-indicator-container` (strong/full-width/thick) vs `.drop-indicator-within` (light/indented/thin) — the two affordances are visually distinct. ✅ asserted by `drag-affordance.spec.ts` (computed styles differ).

### Implementation for User Story 4

- [X] T018 [US4] `wasm.rs`: `WasmStack::core_update_json()` — one getter serializing the whole `CoreUpdate` (declared state + derived view), subsuming D3's per-field getters into the D5 DTO bridge. wasm build clean with `serde_json` under the `wasm` feature.
- [X] T019 [US4] `web3/src/protocol.ts` + `worker.ts`: two-axis `SeriesJson` (`unboundedAbove`), `add_series` new arg order, worker posts parsed `CoreUpdate` in `done`. ✅ tsc + vite build + Playwright green.
- [X] T020 [P] [US4] `web3/src/HierarchyPanel/HierarchyPanel.tsx` `ContainerNode`: youngest-top tree, kind icon, name, validation badge, floor chip, surface children. ✅ tsc + vite build + Playwright green.
- [X] T021 [US4] `web3/src/HierarchyPanel/dragResolver.ts`: the single `resolveDrop` branching on `(dragNodeType, dropZoneType)`; within-group reorder guarded by `allowsWithinReorder` (disabled for vein/intrusion). ✅ tsc + vite build + Playwright green.
- [X] T022 [P] [US4] `HierarchyPanel.tsx` `SeamControl`: three-state for series, read-only cross-cuts chip for bodies, floor marker (no control) for the floor. ✅ tsc + vite build + Playwright green.
- [X] T023 [P] [US4] `HierarchyPanel.tsx`: validation badge + derived summary sourced ONLY via `validationOf`/`effectiveRelationOf` from `CoreUpdate.derived` — no UI-side geology. ✅ tsc + vite build + Playwright green.
- [X] T024 [US4] `web3/src/HierarchyPanel/store.ts` (`applyCoreUpdate` replaces `derived` wholesale — read-only cache) + `index.ts` barrel for embedding. ✅ tsc + vite build + Playwright green.

**Checkpoint**: The reusable hierarchy UI drives the model through the generated contract; no second copy of the grammar exists (Constitution Principle III).

---

## Phase 5: User Story 5 — web3/ testbed on synthetic data (Priority: P1)

**Goal**: Open `web3/`, pick a synthetic scenario, see the hierarchy UI drive the WASM core live.

**Dependency note**: Consumes US1 (model/ABI) + US4 (panel). It is the acceptance vehicle for the MVP.

**Independent Test**: Build + serve `web3/`; select each scenario; edits round-trip through WASM and the viewer updates (quickstart Gate 3 final row; SC-008).

### Tests for User Story 5 ⚠️

- [X] T025 [P] [US5] Native half: `schema::tests::scenario_shaped_dtos_deserialize` — scenario-shaped JSON parses to `StackElementDto[]` the core accepts. GREEN. (FR-022)

### Implementation for User Story 5

- [X] T026 [P] [US5] `web3/src/scenarios.ts`: `conformal-column` + `erosional-unconformity` as fitted-plane fixtures (dipping-plane recipe, 3-contact floor). ✅ tsc + vite build + Playwright green.
- [X] T027 [US5] `web3/src/main.tsx` (scenario selector → worker run → panel + viewer) + `Viewer.tsx` (three.js). Replaces the placeholder. ✅ tsc + vite build + Playwright green.
- [X] T028 [US5] Round-trip: the **Rust half is PROVEN** (`core_update_json` ↔ `CoreUpdate` round-trips; mutation→derive is pure; scenario DTOs deserialize). The **browser live-render half is NOT verified here** (no `npm install`/headless browser) — deferred to a runtime check (quickstart Gate 3). Logged honestly, not claimed. (FR-023, SC-005)

**Checkpoint** 🎯 **MVP COMPLETE & BEHAVIOR-VERIFIED.** Model + ABI + DTO contract green (43 Rust tests, wasm builds); `web3/` `tsc`-typechecks + `vite build`s clean; **Playwright 4/4 green** — scenarios render, the seam round-trips through the WASM core, the two drag affordances are distinct. The full stack (React panel → worker → wasm-bindgen → BSD engine → EUPL kernel oracle → `core_update_json` → derived display) is proven end-to-end in a browser. Behavioral SCs (001/005/006/008) are gate-backed per constitution v1.1.0.

### Phase 5a: Functional gate (Playwright) — constitution v1.1.0 Principle I

**Purpose**: The UI/behavioral acceptance criteria (SC-001/005/006/008, quickstart Gate 3) are backed by an **automated functional test driving the real app**, not a manual walk. (Constitution v1.1.0: UI behavior cites a Playwright gate; `tsc`/`vite build` are the compile gate, Playwright is the behavior gate.)

- [X] T057 [US5] `@playwright/test` in `web3` devDeps + `web3/playwright.config.ts` (webServer = `npm run preview` over the built app; chromium).
- [X] T058 [US5] `web3/e2e/smoke.spec.ts` — page loads (heading + selector + canvas); each scenario selects, populates the panel, reaches status "done". GREEN. (SC-001, FR-021)
- [X] T059 [US5] `web3/e2e/round-trip.spec.ts` — seam erode→onlap→conformable re-fits and the derived display changes through the WASM core. GREEN — proves the full panel→worker→wasm→kernel→`core_update_json`→derived loop. (FR-023, SC-005)
- [X] T060 [US5] `web3/e2e/drag-affordance.spec.ts` — `.drop-indicator-container` vs `.drop-indicator-within` computed styles differ (height/indent). GREEN. (FR-015, SC-006)
- [X] T061 [US5] `web3` `test:e2e` script added; **`npx playwright test` → 4/4 PASS in 12.3s** (chromium, against the production build). Behavioral SCs are now gate-backed, not manual.

---

## Phase 6: User Story 2 — Cross-cutting bodies: veins and intrusions (Priority: P2)

**Goal**: Place a vein or intrusion at a chronology position; it claims its interior/band and its cut-set is derived, never declared.

**Dependency note**: Builds on US1's clean `StackElement` axis. Independently testable.

**Independent Test**: Vein clips an older bed only within its band; intrusion claims only its interior; reported `cuts_below` matches actual overlap with zero hand-entry (quickstart Gate 3 cross-cutting row; SC-003).

### Tests for User Story 2 ⚠️

- [X] T029 [P] [US2] `stack::tests::vein_is_present_only_within_its_band` + `vein_cross_cuts_only_within_band` — vein present only in the FW/HW band; clips an older bed only inside it (lithology shows the floor above AND below the band, proving no unbounded half-space). GREEN. (FR-006)
- [X] T030 [P] [US2] `stack::tests::intrusion_claims_only_its_interior` — present only inside the envelope; `global_unit` routes interior points to it with no special case. GREEN. (FR-007, FR-009)
- [X] T031 [P] [US2] `stack::tests::cuts_below_is_derived_from_overlap` — `cuts_below` = older overlapped elements; `[]` when the body reaches nothing; `[]` for a series. GREEN. (FR-008)

### Implementation for User Story 2

- [X] T032 [US2] `crates/geofield/src/stack.rs`: `enum StackElement<M> { Series, Vein, Intrusion }` (Option A); `Stack.series` is now `Vec<StackElement<M>>`, threaded through `global_unit` (uniform youngest-present-wins), the meshers (series-only via `series_at`), schema, and wasm. 47 tests green.
- [X] T033 [US2] `Vein` + `band()` + band `present_at`; masker contribution = two ANDed `ContactHalfSpace`s (above FW, below HW) in `erode_masker_fields`, reusing `octree::ContactHalfSpace` (Principle VI).
- [X] T034 [US2] `Intrusion` + `Envelope::{Single,Shell}` + interior `present_at`; envelope-interior masker (Single = below-iso; Shell = between).
- [X] T035 [US2] `Stack::cuts_below(i, min, size)` — derived from order + sampled spatial overlap; never stored; `[]` for series / no-reach.
- [X] T036 [US2] `wasm.rs` ABI D2: `add_vein` / `add_intrusion` (shared `fit_model` helper). wasm target builds clean. NOTE: cross-cutting BODY surface meshing in the viewer is a further step — the model/lithology/cut-set are proven; the 3D body mesh path is logged below.
- [X] T037 [P] [US2] `web3/src/scenarios.ts` "Cross-cutting vein" scenario + protocol/worker `crossCut` dispatch. Playwright `cross-cut.spec.ts` asserts the read-only cross-cuts chip + derived `crossCut`. GREEN. (FR-016, SC-003)
- [X] T038 [US2] `HierarchyPanel.tsx`: the cross-cuts chip tooltip records the syn-depositional punt (design doc §3.5; Principle IV). Asserted by `cross-cut.spec.ts` (`title=/syn-depositional/`).

**Checkpoint**: Veins/intrusions configurable; band/interior/cut-set proven (Rust) and the derived cross-cuts chip verified in-browser (Playwright). **Logged gap (Principle IV):** rendering the cross-cutting BODY's own 3D surface in the viewer is not yet done — `mesh_one_block` is series-only, so a vein/intrusion currently contributes its derived chip + lithology + masker-clip of older surfaces, but its own envelope/band surface isn't meshed for display. A follow-up adds a body-mesh path (design doc §3.4 reuse).

---

## Phase 7: User Story 3 — Faults as an outer partition; per-block deviation (Priority: P2)

**Goal**: A fault network partitions space; the whole stack is instanced per block; a starved block shows no surface there; per-block disable doesn't touch the template.

**Dependency note**: Reuses the per-block primitives, driven by an outer block index. Independently testable with the single-block stub first.

**Independent Test**: Two-block network renders the template per block; a starved contact reports absent without aborting; per-block disable is local (quickstart Gate 3 fault + override rows; SC-004).

### Tests for User Story 3 ⚠️

- [X] T039 [P] [US3] `faulted::tests::single_block_instances_the_template` — `blocks()` yields one block; the whole stack instances into it. GREEN. (FR-010; stub per research R3)
- [X] T040 [P] [US3] `faulted::tests::starved_element_yields_no_surface_without_aborting` — a disabled/absent element produces no surface; the rest of the block still meshes (not aborted). GREEN. (FR-011)
- [X] T041 [P] [US3] `faulted::tests::block_override_is_local_and_does_not_edit_template` — disabling an element in one block is local + reported via `absent_in_block`; the shared template is untouched; clearing restores. GREEN. (FR-012)

### Implementation for User Story 3

- [X] T042 [US3] `crates/geofield/src/faulted.rs`: `FaultedModel<M>` / `FaultNetwork` / `FaultBlock` / `Fault` / `FaultInteraction` / `BlockOverride`. `blocks()` STUBBED to one whole-domain block, logged as a Principle-IV bound (research R3).
- [X] T043 [US3] Outer-block-index instancing: `enabled_elements(block)` drives the per-block iteration; with the single-block stub, meshing the instanced template = `Stack::mesh_surfaces` (the tested per-series block primitives are reused unchanged — design doc §4.3). The >1-block geometry is the logged follow-up.
- [X] T044 [US3] `BlockOverride` (disabled elements per block) + `is_enabled` / `absent_in_block` / `enabled_elements` (feeds `absentInBlock`). Proven local + non-template-editing.
- [X] T045 [US3] `wasm.rs` ABI D4: `fault_block_count()` getter (the honest real surface — 1 until partition geometry). wasm builds clean.
- [⏸] T046 [US3] **DEFERRED (logged, Principle IV/VI).** Fault sub-panel UI blocked on the >1-block partition geometry: with one block there is no observable partition behaviour to drive, so the sub-panel would be hollow chrome. Lands with the partition geometry.
- [⏸] T047 [US3] **DEFERRED (same reason).** A fault scenario renders identically to a non-faulted one under the single-block stub; meaningful only once `blocks()` returns >1.

**Checkpoint**: Faults are modelled as an outer partition (proven: instancing, overrides, starved-block, ABI block-count). **Logged bound:** the partition GEOMETRY (deriving N blocks from fault planes) and its UI are deferred — `blocks()` returns 1; the interface shape is real so the follow-up changes only the partition computation, not its consumers.

---

## Phase 8: User Story (model) — Shared surfaces (DAG) (Priority: P2, smallest)

**Goal**: One surface referenced by two boundaries (truncation top below + lap-against floor above) without duplication.

**Independent Test**: A shared surface appears once in the data layer and is referenced by both boundaries (data-model.md §Surface; FR-013).

- [ ] T048 [P] In `crates/geofield/tests/`: shared-surface reference test — one `SurfaceId` referenced by two boundaries; no duplication (FR-013)
- [ ] T049 Create `crates/geofield/src/surface.rs`: `SurfaceId` entity store / DAG references; stack elements reference surfaces (design doc §5)
- [ ] T050 Expose surface sharing through the seam control only (NOT a general DAG editor — documented non-goal; design doc §5/§8; spec Out-of-Scope)

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Constitution gates (`.specify/memory/constitution.md` v1.1.0) and doc/status truth-up.

- [X] T051 [P] Governing docs updated: `stack.rs` module docs (two-axis + cross-cutting), `docs/stack_hierarchy_spec.md` ledger items 1–6 status-tagged to what landed (Principle II).
- [X] T052 Full workspace `cargo test`: **52 geofield tests + kernel suites green, 0 failed** (1 ignored diagnostic). clippy clean on all new modules (Principle I; quickstart Gate 1).
- [X] T053 wasm32 target builds clean across all new ABI (D1/D2/D3/D4/D5). Numeric native↔wasm parity is a browser probe (deferred, logged) — the compile + native-green proxy holds (Principle I/V; quickstart Gate 2).
- [X] T054 [P] EUPL/BSD seam intact: `geofield-kernel` is `cfg(not(wasm32))` (out of the engine wasm graph); `web3`'s only kernel references are the `kernel-bridge` import + the separate `geofield_kernel.wasm` artifact load (the module-boundary import, by design); `serde` is permissive, not gempy-derived; no ts-rs added (Principle V; quickstart Gate 4).
- [X] T055 [P] Status calibrated + bounds logged where read: the single-block fault stub (faulted.rs module doc + ledger item 3), the cross-cut body-mesh follow-up (ledger item 2), the deferred fault UI (T046/T047), ts-rs deferral (research R1) (Principle IV; quickstart Gate 4).
- [X] T056 Playwright **5/5 green** (smoke ×2, round-trip, drag-affordance, cross-cut) drives Gate-3 rows for the conformal/unconformity/cross-cut scenarios + the two distinct drag affordances (SC-001, SC-006, SC-008). Multi-block fault rows await the partition geometry (logged).

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (P1)**: no dependencies — start immediately.
- **Foundational (P2)**: depends on Setup — **BLOCKS all user stories** (the two-axis cleanup).
- **US1 (P1)**: after Foundational. The model+ABI core.
- **US4 (P1)**: after US1 (needs model/ABI T013–T014 + schema T016).
- **US5 (P1)**: after US4 (consumes the panel) — completes the MVP.
- **US2 (P2)**: after US1 (needs clean `StackElement`); independent of US3/shared-surfaces.
- **US3 (P2)**: after US1; independent of US2.
- **Shared surfaces (P2)**: after US1; independent of US2/US3.
- **Polish (P9)**: after all desired stories.

### Story Dependencies (honest, not pretended-independent)

- US1 → US4 → US5 form a **chain** (this is the MVP spine). The spec calls all three P1; they are co-equal in priority but sequentially dependent in build order.
- US2, US3, shared-surfaces are mutually independent P2 work, all gated only on US1.

### Within Each Story

- Tests written with the code they pin (project culture; Principle I).
- Model (`stack.rs`) before ABI (`wasm.rs`) before UI (`web3/`).
- Existing suite green before moving on.

### Parallel Opportunities

- **Setup**: T002, T003 in parallel (T001 first).
- **US1 tests**: T011, T012 in parallel.
- **US4**: T020, T022, T023 (different panel components) in parallel after T016/T018; T017 alongside.
- **US2 tests**: T029, T030, T031 in parallel; T037 alongside implementation.
- **US3 tests**: T039, T040, T041 in parallel; T046, T047 alongside.
- **Polish**: T051, T054, T055 in parallel.
- **Cross-story (P2)**: once US1 lands, US2 / US3 / shared-surfaces can be worked by different people in parallel.

---

## Implementation Strategy

### MVP First (US1 → US4 → US5)

1. Phase 1 Setup → Phase 2 Foundational (two-axis cleanup, **suite green**).
2. US1 (model/ABI core) → US4 (reusable UI + schema) → US5 (testbed, conformal + unconformity scenarios).
3. **STOP and VALIDATE**: quickstart Gate 1–3 rows 1–4 + the round-trip. This is a demonstrable product: configure a layered/eroded column from the reusable UI, live on the WASM core.

### Incremental Delivery (P2)

4. US2 (cross-cutting bodies) → add the body scenario → demo.
5. US3 (faults, single-block stub → real partition geometry as a logged follow-up) → add the fault scenario → demo.
6. Shared surfaces (smallest) → demo correlative conformity.
7. Polish: gate the whole thing against the constitution.

### Notes

- The fault **partition geometry** (beyond the single-block stub) is the one
  genuinely large remaining unknown after US3's interface lands; it is a logged
  bound (research R3), not silent — surface it before claiming faults "done"
  (Principle IV).
- Removal of the legacy `relation: number` `3=Basement` integer is the final,
  deliberate step once `web/`/`web2`/`web3` are all off it (contracts/wasm-abi-delta.md).
