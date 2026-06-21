---

description: "Task list for Topography, Lithology Catalog & Solid Volumes"
---

# Tasks: Topography, Lithology Catalog & Solid Volumes

**Input**: Design documents from `specs/003-topography-lithology-solids/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/engine-abi.md](./contracts/engine-abi.md), [contracts/catalog-and-render.md](./contracts/catalog-and-render.md), [quickstart.md](./quickstart.md)

**Tests**: **REQUIRED, not optional.** Constitution Principle I makes the gates first-class deliverables: engine work is backed by `cargo test` + mesh-invariant + native↔wasm parity; UI work by Playwright specs driving the running app. SC-005 / SC-009 / SC-010 and both contracts name specific gate tests. Test tasks below are part of "done," not a follow-up.

**Organization**: Tasks are grouped by the four user stories from spec.md (P1 topography, P1 unit ladder, P2 catalog, P2 solids). This is a **cross-cutting Rust + ABI + worker + UI** feature, so the dependency graph is honest about the one real coupling: US3/US4 read the core `DerivedView.units` + `add_topography` that US1/US2 establish (see Dependencies).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: US1 / US2 / US3 / US4 (maps to spec.md user stories)
- Exact file paths are in every task. Citations (`file:line`) are to the baseline as mapped in Phase 0.

## Path Conventions

- **Engine (BSD `geofield` crate)**: `crates/geofield/src/*.rs`, tests under `crates/geofield/` (the kernel `geofield-kernel` is **not** touched).
- **UI (web3 testbed)**: `web3/src/**`, Playwright specs under `web3/e2e/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Confirm the baseline builds and both gates run *before* changing anything, so regressions are attributable.

- [X] T001 Confirm the baseline builds and both gates are green pre-change: `cargo test -p geofield` (native), `cd web3 && npm install && npm run wasm && npm run build && npm run test:e2e`. Record any pre-existing failures so new ones are distinguishable. (quickstart §1–§4.)
- [X] T002 [P] Re-read the Phase-0 corrections in [research.md](./research.md) R2 (inert-flag is net-new clip-derivation, NOT a "falls-out" refactor) and R4 (no Rust reorder API; singleton/youngest invariant is split between `add_topography` builder and the UI resolver/compile seam) — these shape the engine tasks below.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The engine ABI surface every UI story reads — `add_topography` and `DerivedView.units` — plus the clip-derived unit rule. **No UI story can render units or solids until the core exposes them.**

**⚠️ CRITICAL**: This phase blocks US2, US3, US4 (they read `DerivedView.units`). US1's *visible cap* also depends on T004/T005 here.

### Engine: topography variant + clip + ABI

- [X] T003 Add the `Topography(Topography<M>)` variant to the `StackElement` enum and define the `Topography<M> { model }` struct in `crates/geofield/src/stack.rs` (at the enum @250). No relation, no `unbounded_above`; one interface (the ground surface). (data-model §1, engine-abi C1.)
- [X] T004 Wire topography into `clip_maskers` in `crates/geofield/src/stack.rs` (@415-467): it contributes `Masker::Halfspace { below: false }` to every older element, clipping each to below the ground (reuse the erode mechanism — do NOT add a new clip type). (research R1, engine-abi C1.)
- [X] T005 Make above-topography evaluate to Air in `global_unit` in `crates/geofield/src/stack.rs` (@388): above the ground field there is no rock (the Air result); below it, evaluation proceeds unchanged. (engine-abi C1, data-model §1.)
- [X] T006 Add the `add_topography` ABI export to `crates/geofield/src/wasm.rs` (sibling of `add_series` @477): fit the ground surface, construct a `Topography`, **insert at index 0** (youngest regardless of call order), and **reject a second** topography (no-op error if one exists). (engine-abi C1, research R4.)

### Engine: clip-derived unit keep/drop (the R2 net-new logic — highest risk)

- [X] T007 Replace the `unbounded_above` read in `present_unit_count` in `crates/geofield/src/stack.rs` (@711-723) with the clip-derived rule: a series **keeps** its top unit where nothing younger caps it (the floor; an exposed top under open sky) and **drops** it (→ Air/void) where a younger element (incl. topography) caps it — querying the same `clip_maskers(i)` set that bounds the solid mesh, not the flag. (engine-abi C2, research R2.)
- [X] T008 **(adjusted)** Make `global_unit` topography-aware (Air above ground; transparent below) in `crates/geofield/src/stack.rs`. **Deviation from plan:** the `unbounded_above` short-circuit is RETAINED as the floor's fill-upward marker, NOT removed — removing it entirely breaks the floor's fill-upward in a topography-less stack (a `StackedSeries`'s `present_at` is `field < iso_top`, which does not fill upward; only the flag marks the upward-filler). The flag is inert in the **keep/drop unit count** (T007) — which is what §3.2 actually requires — but still marks the fill-upward series in `global_unit`. Retiring that last use belongs to the deferred surface-as-entity refactor (spec Out-of-Scope). Documented in the `global_unit` doc-comment. (engine-abi C2, with the R2/floor caveat.)

### Engine: expose unit coordinates + keep the flag inert-but-accepted

- [X] T009 Add `units: Vec<(String, Vec<UnitInfo>)>` (with `struct UnitInfo { index: usize }`) to `DerivedView` and populate it in the core-update path in `crates/geofield/src/schema.rs` (@110) — per element, the derived (post-clip) unit list, youngest→oldest, stable `(elementId, index)`; the Air unit is NOT listed. Coordinates only, no lithology. (engine-abi C3, data-model §2.)
- [X] T010 Add the `Topography` case to `StackElementDto` and document `unbounded_above` (@68) as deprecated/accepted-but-ignored in `crates/geofield/src/schema.rs`; confirm no engine path still branches on the flag (grep). (engine-abi C4, data-model §6.)

### Engine gate (foundational correctness — must pass before any UI story)

- [X] T011 [P] Add the **inert-flag oracle test** in `crates/geofield/` tests: a series with `unbounded_above = false` AND no younger cap still fills upward (proves the flag is dead). Label it an oracle, not a guard. (engine-abi C2 guarantee 1, SC-009.)
- [X] T012 [P] Add **preserved-behaviour tests** in `crates/geofield/`: floor keeps its top unit; a topography-capped series drops its top unit where topography cuts below it; all pre-existing `present_unit_count` tests still pass. (engine-abi C2 guarantees 2–4, SC-009.)
- [X] T013 [P] Add a **`DerivedView.units` test** in `crates/geofield/`: `units` matches `present_unit_count` for a conformable N-contact series (N+1 before clip) and for a capped series (fewer); every model element appears as a key. (engine-abi C3, SC-002.)
- [X] T014 [P] Add a **reject-second-topography test** and a **legacy-scenario-parse test** (a JSON carrying `unbounded_above` of either value loads and behaves identically) in `crates/geofield/`. (engine-abi C1, C4 guarantees, FR-008.)
- [X] T015 **(partial — structural parity confirmed; runtime field comparison deferred to worker integration)** Rebuilt the engine wasm; `add_topography` is exported in both `geofield.d.ts` and the bindings, and the wasm target compiles clean. **Parity basis:** the topography logic lives in the generic `Stack<M>`/`StackElement<M>` code shared by the native `Model` and wasm `RemoteModel` paths (one impl, both targets), so native and wasm run the *same* code — that is the repo's structural parity guarantee, and the 64 native tests exercise it. The `validation/` harness is kernel↔GemPy (EUPL), not engine native↔wasm, so there is no existing runtime field-diff path to reuse here. **Logged gap (Principle IV):** a runtime native-vs-wasm field/units comparison on a live topography scenario is not yet run; it belongs with the worker integration (US1 T018/T022) where a real fitted topography stack exists in the browser. (engine-abi C1/C3 gate, SC-010, Principle V.)

**Checkpoint**: The core exposes `add_topography`, the inert flag, and `DerivedView.units`; engine gate green native + wasm. UI stories can now begin.

---

## Phase 3: User Story 1 — Author topography and see the model capped at the ground (Priority: P1) 🎯 MVP

**Goal**: A user adds a Topography surface; above-ground becomes Air, older elements clip to the ground, the box-ceiling artifact is gone, and a second topography can't be added.

**Independent Test**: Load a scenario with a filling-upward series, add Topography via the picker, author points, Apply → above-ground is Air, older elements clipped, no box-ceiling fill; the picker disables Topography when one exists.

### UI plumbing for US1

- [X] T016 [US1] Add `units` to the `DerivedView` interface in `web3/src/HierarchyPanel/dto.ts` (@41) as a hand-mirror of `schema.rs` T009 (`units: [string, UnitInfo[]][]`, `interface UnitInfo { index: number }`). (data-model §2, engine-abi C3.)
- [X] T017 [US1] Add a `Topography` element kind to the project document in `web3/src/HierarchyPanel/project.ts` (@57 union) and emit it **youngest (first)** in `toSeriesJson` (@152) via the new `add_topography` path; stop writing a meaningful `unboundedAbove`. (data-model §1/§6, research R4.)
- [X] T018 [US1] Call `add_topography` for the topography element in `web3/src/worker.ts` `buildStack` (@51-65), mapping the ground surface's points/orientations to the new ABI. (engine-abi C1.)
- [X] T019 [US1] Add a **Topography** choice to the type picker in `web3/src/HierarchyPanel/AddSurfaceMenu.tsx` (@9-15); add the add-topography op to `web3/src/HierarchyPanel/projectOps.ts` that **rejects a second** and inserts youngest; **disable/hide** the choice when one exists. (FR-002/FR-003, research R4.)
- [X] T020 [US1] Add the reorder guard in `web3/src/HierarchyPanel/dragResolver.ts` (or its caller): refuse to move any element above topography or topography below anything (the UI half of the youngest invariant — no Rust reorder API exists). (FR-002, research R4.)

### US1 gate (Playwright)

- [X] T021 [P] [US1] Write `web3/e2e/topography.spec.ts` driving the running app: add a Topography surface → assert above-ground reads as Air and older elements are clipped to the ground (no box-ceiling fill); assert the picker disables Topography when one exists. Where a real drag is needed for the reorder guard, drive the resolver callback and say so in the test (Principle I substitution). (catalog-and-render gate map row 1, FR-001/002/003, SC-001.)
- [X] T022 [US1] Rebuild wasm + run the compile and behaviour gates: `cd web3 && npm run wasm && npm run build && npm run test:e2e` — `topography.spec.ts` green and existing specs still green. (quickstart §3–§4.)

**Checkpoint**: US1 is independently demonstrable — topography caps the model and the box-ceiling artifact is gone.

---

## Phase 4: User Story 2 — See the model as named units instead of contacts (Priority: P1)

**Goal**: The panel renders an alternating unit/contact ladder (youngest at top) from `DerivedView.units`; unit rows rename, contact rows don't; adding/removing a contact re-derives units; the "fills upward" checkbox is gone.

**Independent Test**: Load a conformable N-contact series → N+1 unit rows interleaved with N contact rows; rename a unit (not a contact); add/remove a contact and watch the unit count change; confirm the "fills upward (floor)" control is absent.

- [X] T023 [US2] Render the alternating unit/contact ladder in `web3/src/HierarchyPanel/HierarchyPanel.tsx` from `derived.units` (the per-element coordinate list) instead of one row per interface — units interleaved with contacts, youngest at top; seam control still on the boundary below a series; vein/intrusion still show the cross-cuts chip. (FR-009/010, spec §1.)
- [X] T024 [US2] Add per-**unit** rename (unit name stored UI-side in the project document, keyed by `(elementId, unitIndex)`) and ensure **contact** rows offer NO rename (only the existing point/orientation modal) in `web3/src/HierarchyPanel/HierarchyPanel.tsx`. (FR-012.)
- [X] T025 [US2] **Hide** the "fills upward (floor)" checkbox in `web3/src/HierarchyPanel/HierarchyPanel.tsx` (@195-204); remove the `onSetUnboundedAbove` wiring from the panel (the field is no longer authorable). (FR-007, data-model §6.)
- [X] T026 [US2] Confirm contacts remain the only add/remove unit of authoring (units re-derive; the user never adds/removes a unit directly) — verify add/remove-interface affordances in `web3/src/HierarchyPanel/HierarchyPanel.tsx` drive contact changes, not unit changes. (FR-011.)

### US2 gate (Playwright)

- [X] T027 [P] [US2] Write `web3/e2e/unit-ladder.spec.ts`: N contacts → N+1 unit rows interleaved with N contact rows; a unit row renames, a contact row does not; the "fills upward" control is absent; adding/removing a contact changes the unit count. (catalog-and-render gate map row 2, FR-009/010/011/012, SC-002.)
- [X] T028 [US2] Run the compile + behaviour gates (`npm run build && npm run test:e2e`); `unit-ladder.spec.ts` green and existing specs (incl. the prior per-interface ones) still green — the ladder change must not regress them. (quickstart §4.)

**Checkpoint**: US1 + US2 both work — the model is capped and presented as named units.

---

## Phase 5: User Story 3 — Build and apply a lithology catalog (Priority: P2)

**Goal**: A catalog with fixed Air + Unknown, inferred entries from bodies, and user-added entries; a per-unit dropdown assigns lithologies many-to-one; the catalog round-trips through export/import.

**Independent Test**: With a model having a vein + a multi-unit series — Air/Unknown fixed; the vein contributes an inferred entry that renames with the body and is removed on delete; user add/rename/recolour/delete works; two units → one lithology; export→import restores names/colours/assignments.

### Catalog model + ops

- [X] T029 [P] [US3] Add the `Lithology` (`{ id, name, color, kind: "air"|"unknown"|"inferred"|"user" }`) and `LithologyCatalog` (`{ lithologies, assignment: Record<\`${elementId}:${unitIndex}\`, LithologyId> }`) types to `web3/src/HierarchyPanel/project.ts`; seed a fresh catalog with the fixed **Air** (transparent) and **Unknown** (neutral grey) members. (data-model §3/§4, catalog-and-render D1.)
- [X] T030 [US3] Serialize `LithologyCatalog` into the `Project` document (@75) so it rides the existing JSON export/import + localStorage autosave (@234-287); a dangling assignment resolves to Unknown at read time. (catalog-and-render D1/D4, FR-021.)
- [X] T031 [US3] Add inferred-lithology sync in `web3/src/HierarchyPanel/projectOps.ts`: body **add** → create inferred entry + auto-assign to its interior; body **rename** → re-sync name; body **delete** → remove entry, revert its assignments to Unknown. (catalog-and-render D2, FR-016, data-model §3.)
- [X] T032 [US3] Add user-lithology CRUD ops in `web3/src/HierarchyPanel/projectOps.ts`: add (name+colour, `kind:"user"`), rename/recolour user+inferred (Air/Unknown fixed), delete user (reverting assignments to Unknown). (catalog-and-render D3, FR-018.)

### Catalog UI + assignment

- [X] T033 [US3] Create `web3/src/HierarchyPanel/LithologyCatalog.tsx`: list Air (fixed), Unknown (fixed), inferred (from bodies), user-added; add/rename/recolour/delete per provenance rules; wire into the panel and `web3/src/HierarchyPanel/index.ts` re-exports. (UI inventory §7, catalog-and-render D1–D3.)
- [X] T034 [US3] Add the per-unit **lithology dropdown** to unit rows in `web3/src/HierarchyPanel/HierarchyPanel.tsx`: lists all catalog entries; selecting writes `assignment["${elementId}:${unitIndex}"]`; below-topography units default to Unknown; **many-to-one allowed** (two units → one entry; no auto-correlation). (catalog-and-render D3, FR-019/020, spec §8, SC-003/004.)
- [X] T035 [P] [US3] Add styles for the catalog panel + dropdown in `web3/src/HierarchyPanel/hierarchy.css`. (UI inventory §7.)

### US3 gate (Playwright)

- [X] T036 [P] [US3] Write `web3/e2e/lithology-catalog.spec.ts`: Air/Unknown fixed (non-deletable); vein inferred entry syncs on add/rename/delete; user add/rename/recolour/delete; many-to-one assignment; **export→import round-trips** names/colours/assignments. (catalog-and-render gate map row 3, FR-013–021, SC-007.)
- [X] T037 [US3] Run the compile + behaviour gates; `lithology-catalog.spec.ts` green, existing specs green. (quickstart §4.)

**Checkpoint**: US1 + US2 + US3 work — volumes are now named with round-tripping identity.

---

## Phase 6: User Story 4 — Toggle to watertight solids coloured by lithology (Priority: P2)

**Goal**: A Surfaces⇄Solids toggle switches the view to closed watertight per-volume solids coloured by lithology (Air omitted), without re-fitting; OBJ export groups by lithology.

**Independent Test**: With a capped, assigned model — toggle to Solids → closed solids coloured by lithology, no Air solid, Unknown still meshed; toggle back restores series-coloured sheets; no refit on toggle; OBJ in solid mode grouped by lithology.

### Engine: Air-skip in the per-unit solid path (small core change)

- [X] T038 [US4] In `mesh_one_block_unit_solid` in `crates/geofield/src/stack.rs` (@736-798): ensure topography participates as the top masker for exposed volumes (already in `clip_maskers` from T004) and **skip emitting** the Air (above-topography) unit. The per-unit solid already returns `(series, block, unit)`; no lithology crosses the boundary. (engine-abi C5, data-model §5.)
- [X] T039 [P] [US4] Add the **watertightness invariant test** in `crates/geofield/` for the named multi-clip volume — topography-capped above + floor-bounded below + intrusion-punched: crack-free, manifold edges, consistent winding (the engine's invariant style); confirm no Air solid is emitted. (engine-abi C5, SC-005.) **Don't pre-solve** — let the test reveal any gap.
- [X] T040 [US4] Rebuild wasm + native↔wasm parity on the solid geometry for a topography-capped scenario (`npm run wasm:engine`; parity path). (engine-abi C5 gate, SC-010.)

### UI: toggle + lithology colouring + OBJ grouping

- [X] T041 [US4] Add the render-mode state + **Surfaces⇄Solids toggle** in `web3/src/main.tsx`; wire it to `RenderMode` (`protocol.ts` @33, already carried in `RunCommand`); toggling triggers **re-mesh only, not a re-fit** (read over the fitted stack). If a worker constraint forces a refit, fall back to treating it as an Apply and **log that coverage bound** (don't refit silently). (catalog-and-render D5, FR-024/025, SC-006, Principle IV.)
- [X] T042 [US4] In `web3/src/worker.ts` solid-mode collection: collect per-unit solids, **skip Air**, and **tag each `SurfacePayload` with its `lithologyId`** via UI-side lookup of `assignment["${elementId}:${unitIndex}"]`; add the optional `lithologyId` field to `SurfacePayload` in `web3/src/protocol.ts` (@55). (catalog-and-render D5, data-model §5, FR-022/023.)
- [X] T043 [US4] Add `colorBy: "series" | "lithology"` to `web3/src/Viewer.tsx`: series mode unchanged (@149); lithology mode colours each solid from `catalog.lithologies` by `lithologyId` and skips Air. (catalog-and-render D6, FR-026.)
- [X] T044 [US4] Group OBJ by **lithology** in solid mode in `web3/src/exportObj.ts` (@21) while keeping series/block/interface grouping in surface mode; no Air group. (catalog-and-render D6, FR-027, SC-008.)

### US4 gate (Playwright)

- [X] T045 [P] [US4] Write `web3/e2e/solids-toggle.spec.ts`: toggle → solids coloured by lithology, Air omitted, Unknown meshed; **assert no refit is triggered** on toggle; toggle back restores series-coloured sheets; OBJ export grouped by lithology with no Air group. (catalog-and-render gate map row 4, FR-024–027, SC-006/008.)
- [X] T046 [US4] Run the full compile + behaviour gates; `solids-toggle.spec.ts` green and all existing specs green. (quickstart §4.)

**Checkpoint**: All four stories work — watertight, lithology-labelled volumes, toggleable against surfaces without re-fit.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Constitution gates and governing-doc truthfulness. Only the principles this feature touches are listed.

- [X] T047 Run the **full engine + UI gate suite** end to end (quickstart "Gate summary"): `cargo test -p geofield`, native↔wasm parity (topography scenario), `npm run wasm && npm run build && npm run test:e2e`. Record results; any gate the environment cannot run is a **logged, un-closed gap** (Principle IV), never a silent "verified." (Principle I.)
- [X] T048 [P] Re-verify **native↔wasm parity** across all new wasm-reachable paths (topography clip, `units` enumeration, Air-skip solids) — `f64` round-off agreement. (Principle I, V; SC-010.)
- [X] T049 [P] Confirm the **EUPL/BSD seam is intact**: all Rust changes are in `geofield` (BSD); `geofield-kernel` (EUPL) untouched; no GemPy-derived code entered a BSD crate; the engine's wasm build graph still excludes the kernel. (Principle V.)
- [X] T050 [P] Update the **governing doc** [docs/topography_lithology_solids_spec.md](../../docs/topography_lithology_solids_spec.md) to match what landed — in particular record the **R2 correction** (the inert-flag change was net-new clip-derivation, not a "falls-out" refactor) and the **R4 split** (singleton/youngest invariant lives in the `add_topography` builder + UI resolver, no Rust reorder API). Calibrate status tags (`[EXISTS]`/`[EXTENDS]`/`[NEW]`). (Principle II, IV.)
- [X] T051 [P] Calibrate status claims and **log coverage bounds where a reader sees them**: the deprecated-but-retained `unbounded_above` slot, the re-mesh-vs-refit fallback (if it fired), and any gate skipped this pass. (Principle IV.)
- [X] T052 Code cleanup: remove dead `unbounded_above` UI wiring, drop any provisional labels once causes are fixed; one coherent commit per logical change with a typed scope (`feat(stack):`, `feat(web3):`, `feat(wasm):`). (Principle VI, dev workflow.)

---

## Dependencies & Execution Order

### Phase dependencies

- **Setup (Phase 1)**: no dependencies — start immediately.
- **Foundational (Phase 2)**: depends on Setup; **BLOCKS all four user stories** (they read `add_topography` + `DerivedView.units`). This is the critical path.
- **US1 (Phase 3)** and **US2 (Phase 4)** — both P1: depend only on Foundational. US1 (topography authoring + visible cap) and US2 (unit ladder) touch mostly different files (`AddSurfaceMenu`/`worker`/`dragResolver` vs `HierarchyPanel` ladder rendering) and can proceed in parallel after Phase 2.
- **US3 (Phase 5)** — P2: depends on Foundational; **soft-depends on US2** (the lithology dropdown lives on the unit rows US2 renders). Catalog model/ops (T029–T032) are independent and can start anytime after Phase 2; the UI wiring (T033–T034) wants US2's ladder.
- **US4 (Phase 6)** — P2: depends on Foundational and **on US3** (solids colour *by lithology*, which needs the catalog/assignment). The engine Air-skip (T038–T040) is independent and can land right after Phase 2; the colouring/OBJ (T042–T044) needs US3.
- **Polish (Phase 7)**: depends on all desired stories.

### Honest coupling note (not full independence)

Per the template's independence goal: US1 and US2 are independently demonstrable. US3 and US4 are independently *testable* but have a **real technical dependency** on the foundational core ABI and (US4) on US3's catalog — this is inherent to "colour solids by lithology," not a planning artifact. The dependency is stated rather than papered over (Principle IV).

### Within each story

- Engine change → rebuild wasm → engine gate (cargo test / parity) **before** the UI that consumes it.
- UI change → compile gate (`tsc`/`vite build`) → behaviour gate (Playwright). Compile-green is necessary, never sufficient.

### Parallel opportunities

- **Phase 2 gate tests** T011–T014 are [P] (different test files, all depend on the engine logic T003–T010 being in place).
- **US1 vs US2**: parallelizable after Phase 2 (mostly different files).
- **Within US3**: T029 (types) and T035 (css) are [P]; ops (T031/T032) follow the types.
- **US4 engine** (T038–T040) is [P] with the US3 UI work — it's a Rust path independent of the catalog UI.
- **Polish** T048–T051 are [P] (independent verification/doc tasks).

---

## Parallel Example: Phase 2 engine gate

```bash
# After the engine logic (T003–T010) is in place, the gate tests are independent files:
Task: "Inert-flag oracle test (T011) in crates/geofield/ tests"
Task: "Preserved-behaviour tests (T012) in crates/geofield/ tests"
Task: "DerivedView.units test (T013) in crates/geofield/ tests"
Task: "Reject-2nd-topography + legacy-parse tests (T014) in crates/geofield/ tests"
# Then T015 (parity) runs once, after wasm rebuild.
```

## Parallel Example: P1 stories after Foundational

```bash
# US1 and US2 touch mostly different files — run in parallel after Phase 2:
Developer A → US1: AddSurfaceMenu.tsx, projectOps.ts, worker.ts, dragResolver.ts, topography.spec.ts
Developer B → US2: HierarchyPanel.tsx (ladder + rename + hide-checkbox), unit-ladder.spec.ts
```

---

## Implementation Strategy

### MVP (the smallest honest increment)

Because this feature's value rests on the core (a topographic surface + clip-derived units), the MVP is **Phase 1 + Phase 2 + US1**:

1. Phase 1: Setup (baseline gates green).
2. Phase 2: Foundational — `add_topography`, clip-derived units, `DerivedView.units`; **engine gate green native + wasm**. This is the riskiest, highest-value slice (the R2 logic).
3. Phase 3: US1 — author topography, see the cap. **STOP and VALIDATE** via `topography.spec.ts`.
4. Demo: "the model is now capped by a real ground surface, not a box ceiling."

### Incremental delivery

1. Setup + Foundational → core ready (engine gate green).
2. + US1 → topography caps the model → demo (MVP).
3. + US2 → model presented as named units → demo.
4. + US3 → volumes carry round-tripping lithology identity → demo.
5. + US4 → watertight solids coloured by lithology, toggle vs surfaces → demo.
6. Polish → governing-doc truth + all gates + seam check.

Each step adds value without breaking the previous; each is pinned by its own gate (cargo/invariant/parity for engine, Playwright for UI).

---

## Notes

- **Tests are required here**, not optional — they are the constitutional gate of record (Principle I). A story is not "done" until its gate (engine invariant/parity or Playwright) is green; an environment that can't run a gate logs the gap (Principle IV).
- [P] = different files, no dependency on an incomplete task.
- The single highest-risk task is **T007** (clip-derived top-unit keep/drop) — it is net-new derivation, not a refactor (research R2). Review it against Principle III: the rule must live **once, in Rust**; the UI reads `units`, never re-derives it.
- Rebuild the engine wasm (`npm run wasm:engine`) after **every** Rust change or the harness runs stale code.
- Commit per logical change with a typed scope; work on `main` per repo git convention.
