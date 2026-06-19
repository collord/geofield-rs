---
description: "Task list for feature: Enhanced Authoring for the Stack Testbed"
---

# Tasks: Enhanced Authoring for the Stack Testbed

**Input**: Design documents from `/specs/002-enhanced-authoring/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/](./contracts/), [quickstart.md](./quickstart.md)

**Tests**: Test tasks ARE included and are **non-optional** here. Constitution Principle I makes an automated Playwright functional test the *gate of record* for `web*/` UI behavior, and spec SC-008 binds every new behavioral acceptance criterion to one. The brief §10 enumerates the five specs. `tsc -b` + `vite build` are the compile gate (necessary); Playwright is the behavior gate (sufficient).

**Organization**: Tasks are grouped by user story. The two P1 stories share a foundation (the project document, the compile seam, and the App-level project-centric state) — that foundation is Phase 2 (blocking), per [data-model.md](./data-model.md) and [research.md R1–R2](./research.md).

**Path note**: every path is under `web3/`. New files are tagged `[NEW]`, edits to existing files are not (see plan.md file ledger). No Rust/wasm/worker/ABI/schema file is touched.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1–US6 for user-story phases; no label for Setup/Foundational/Polish

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Confirm the harness builds and the gates run before authoring code lands.

- [x] T001 Verify the `web3/` harness builds and runs from a clean state: `cd web3 && npm install`, confirm `web3/src/wasm-kernel/geofield_kernel.wasm` and `web3/src/wasm/` exist (no Rust rebuild needed — this feature changes no Rust), then `npm run dev` loads a scenario. Record any missing-artifact gap per [quickstart.md](./quickstart.md).
- [x] T002 Confirm both gates run green on the baseline before changes: `cd web3 && npm run build` (compile gate) and `npm run test:e2e` (behavior gate). If Playwright can't run in this environment, log it as an un-closed gap (Constitution IV) — do not proceed as if verified.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: The project document, the single compile seam to the unchanged fit path, the reusable table, the scrim-capable viewer, and the App-level project-centric state. **No user story can function until this phase is complete** — both P1 stories read these.

**⚠️ CRITICAL**: Complete this phase before any Phase 3+ work.

- [x] T003 [P] Create the authoring data model types in `web3/src/HierarchyPanel/project.ts` [NEW]: `SurfaceType`, `Project`, `ProjectElement` (series | vein | intrusion union), `InterfaceData`, `PointRow`, `OrientationRow`, `ProjectValidation` — exactly the shapes in [data-model.md](./data-model.md). Types only in this task; no functions yet. UI-only model — it never crosses the wasm boundary.
- [x] T004 [P] Implement `dipDirToPole(dipDeg, dipDirDeg)` in `web3/src/HierarchyPanel/project.ts`, transcribed sign-for-sign from [contracts/project-model.md](./contracts/project-model.md) (brief §5.3), with the axis convention (X=East, Y=North, Z=Up; dip-dir clockwise from +Y; pole points up-dip) documented in a code comment at the function — this is the one place a sign error silently flips the field (Constitution II).
- [x] T005 Implement `toSeriesJson(project): SeriesJson[]` in `web3/src/HierarchyPanel/project.ts` per [contracts/project-model.md](./contracts/project-model.md): per-element mapping (series/vein/intrusion), relation→code, flattened `sp_xyz`/`sp_if`, concatenated `ori_xyz`/`ori_pole` (poles via `dipDirToPole` from T004), no-fault defaults, youngest-first order preserved. Importing `SeriesJson` from `../protocol` (unchanged). Depends on T003, T004.
- [x] T006 [P] Implement `validate(project): ProjectValidation` in `web3/src/HierarchyPanel/project.ts` per [data-model.md](./data-model.md) validation rules: hard errors (series w/ zero interfaces; interface w/ zero points; vein/intrusion missing a required part) + soft warnings (<3 points; no orientation anywhere) + `canApply`. **Structural preconditions only** — must not re-encode any core geological rule (Constitution III; [research.md R10](./research.md)); add a comment stating this boundary. Depends on T003.
- [x] T007 [P] Implement persistence helpers in `web3/src/HierarchyPanel/project.ts` per [contracts/project-model.md](./contracts/project-model.md): `saveProject` (debounced ~500 ms localStorage write keyed by name), `loadProject` (startup restore, null ⇒ seed), `exportProjectJson` (Blob download `<name>.geoproj.json`), `importProjectJson` (parse File → Project). Platform APIs only. Depends on T003.
- [x] T008 [P] Build the reusable table component in `web3/src/HierarchyPanel/tables.tsx` [NEW] per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md): points (X,Y,Z) and orientations (X,Y,Z,Dip,DipDir) tables with per-cell numeric entry, Tab/Enter navigation (Enter on last row adds a row), per-row delete, row-count/validity badge, and clipboard paste (TSV/CSV → columns in order; **wrong-width rows flagged, not dropped or truncated** — Constitution IV). Pure presentational; takes rows + change callbacks. Depends on T003.
- [x] T009 Convert the three canned scenarios into seed templates in `web3/src/scenarios.ts`: each returns a `Project` (not a `SeriesJson[]` dropdown entry); add a `blank` template. The geology must compile (via `toSeriesJson`) back to the same `SeriesJson[]` the scenario emits today — the round-trip anchor ([quickstart.md](./quickstart.md)). Keep the old `SCENARIOS`/`scenarioById` only if still referenced; otherwise replace. Depends on T003, T005.
- [x] T010 Add the `stale?: boolean` prop to `web3/src/Viewer.tsx` per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md): wrap the canvas mount in a positioned container; when `stale`, render a sibling overlay `<div>` (~40% grey, `pointer-events: none`, a stable `data-testid="stale-scrim"`); camera interaction unaffected; no overlay when false/absent. Add scrim styles to `web3/src/HierarchyPanel/hierarchy.css` (or a co-located style) as needed.
- [x] T011 Rewire `web3/src/main.tsx` to project-centric state per [data-model.md](./data-model.md) state transitions and [contracts/ui-callbacks.md](./contracts/ui-callbacks.md): add `project`/`setProject`, `dirty`/`setDirty`, `applying`/`setApplying`, and a `lastApplied` ref; initialize from `loadProject() ?? blankTemplate()`; pass `stale={dirty}` to `Viewer`; debounced `saveProject` on committed edits. Keep `runStack` and its hard-terminate-on-new-run cancellation **unchanged**; keep `surfaces`/`status`/`extent`/`HierarchyState` state. Wire the panel-level **Apply** (`setApplying(true)` → `runStack(toSeriesJson(project))` → on `done`: snapshot `lastApplied`, clear `dirty`/`applying`) and **Cancel** (revert to `lastApplied`, clear `dirty`). Keep `onSetRelation`/`onSetUnboundedAbove` as cheap immediate edits (dirty stays false). Depends on T003, T005, T006, T007, T009, T010.
- [x] T012 Add the **Apply/Cancel bar** to `web3/src/main.tsx` (shown only when `dirty`): Apply (primary, disabled-with-reason when `!validate(project).canApply`, surfacing the first hard-error reason), Cancel, a dirty dot + "unapplied changes" label. Reads `validate(project)`. Depends on T011.
- [x] T013 Update `web3/src/HierarchyPanel/index.ts` to re-export the new public components/types (`project.ts` types + helpers, `tables.tsx`) so the reusable boundary's surface stays explicit. Depends on T003, T008.

**Checkpoint**: The project compiles to `SeriesJson[]`, Apply/Cancel re-fits and reverts, the scrim is available, templates seed a project, the table renders — but no Add/Edit/menu UI yet. The MVP stories build on this.

---

## Phase 3: User Story 1 — Author a new surface from scratch (Priority: P1) 🎯 MVP

**Goal**: Add a surface by picking a type, enter/paste its data in the dual-table modal, commit, and Apply to see it fit and render.

**Independent Test**: From a blank/seeded project, Add Surface → pick each type → enter valid points → modal Apply → panel Apply → the surface appears in the rendered model and the derived summary; `status: done`.

### Tests for User Story 1 (Constitution I behavior gate)

- [x] T014 [P] [US1] Write `web3/e2e/authoring-add.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V1: Add Surface → pick "Erosional" → modal opens → paste a TSV points block → modal Apply → panel shows the new surface + dirty → panel Apply → `status: done` and the surface appears in the derived summary. (Per Constitution I, the spec is authored to pin the behavior; it should fail until US1 implementation lands.)
- [x] T015 [P] [US1] Write `web3/e2e/modal-validation.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V6: zero-point surface blocks modal Apply; adding points enables it; a series with no orientation shows the amber advisory but still applies.

### Implementation for User Story 1

- [x] T016 [US1] Build the type-picker popover in `web3/src/HierarchyPanel/AddSurfaceMenu.tsx` [NEW] per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md) and brief §4.3: five choices (conformable, erosional, onlap, vein, intrusion) with one-line glosses; on pick, calls back with the chosen `SurfaceType`. Depends on T003.
- [x] T017 [US1] Build the dual-table data modal in `web3/src/HierarchyPanel/SurfaceDataModal.tsx` [NEW] per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md) and brief §5: reads `InterfaceData` into local state; embeds two `tables.tsx` instances (points required, orientations optional); footer validation summary; **modal Apply** (disabled when zero points) commits to the project and closes; **modal Cancel** discards; the modal never runs the worker. Show the dip/dip-dir axis-convention tooltip. Depends on T008, T003.
- [x] T018 [US1] Implement the type→container add rule in `web3/src/HierarchyPanel/project.ts` (e.g. `addSurface(project, type, insertPos): { project, openInterfaceRef }`) per [research.md R4](./research.md)/brief §1: conformable appends to the adjacent conformable series else new series; erosional/onlap → new single-surface series with that relation; vein/intrusion → new container with the required pair/envelope; default name; insert at the oldest end. Depends on T003.
- [x] T019 [US1] Add the **"+ Add Surface"** affordance and singleton flattening to `web3/src/HierarchyPanel.tsx` per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md): a one-interface series renders as a flat surface row (no header); multi-interface series nest under a header; vein/intrusion always show named sub-surfaces; add the `onAddSurfaceClick` prop and surface-row edit entry points. Keep seam/floor/derived-validation/summary rendering unchanged (still read from `CoreUpdate.derived`). Depends on T011.
- [x] T020 [US1] Wire Add in `web3/src/main.tsx`: `onAddSurfaceClick` opens `AddSurfaceMenu`; on type pick, call `addSurface` (T018), `setDirty(true)`, and immediately open `SurfaceDataModal` for the new interface; modal Apply commits the edited `InterfaceData` into `project` and sets dirty; pass `validation` into the panel and the Apply bar. Depends on T012, T016, T017, T018, T019.

**Checkpoint**: US1 is fully functional and independently testable — a user can author any of the five surface types end to end (T014, T015 pass). This is the MVP.

---

## Phase 4: User Story 2 — Edit and re-apply an existing surface's data (Priority: P1)

**Goal**: Edit an existing surface's data via its "…" menu, commit, and pay one re-fit for several edits via the single panel Apply.

**Independent Test**: Open Edit data… on a seeded surface, change its points, modal Apply, panel Apply → the model reflects the new data; modal Cancel discards; two surfaces edited then one Apply → a single re-fit reflects both.

### Tests for User Story 2 (behavior gate)

> Note: the round-trip of "edit → modal Apply → panel Apply → fit reflects it" and "Cancel discards" is exercised by the V2 portion of `authoring-add.spec.ts` (T014) plus the orientation round-trip; add focused assertions there rather than a redundant new spec. If a distinct journey emerges, add `web3/e2e/edit-surface.spec.ts`.

- [x] T021 [US2] Extend `web3/e2e/authoring-add.spec.ts` (from T014) with the V2 assertions per [quickstart.md](./quickstart.md): Edit data… shows current rows; modal Cancel leaves the project unchanged; editing two surfaces then a single panel Apply reflects both; reopening the modal round-trips the typed dip/dipDir values.

### Implementation for User Story 2

- [x] T022 [P] [US2] Build the per-surface "…" menu in `web3/src/HierarchyPanel/SurfaceMenu.tsx` [NEW] per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md) and brief §4.4: Edit data… / Rename… / Duplicate / Delete for a surface; for a multi-interface series header, rename / delete series / add interface. Emits the corresponding callbacks. Depends on T003.
- [x] T023 [US2] Implement the edit/rename/duplicate/delete + add-interface operations on `Project` in `web3/src/HierarchyPanel/project.ts` (pure helpers): edit replaces an `InterfaceData`; duplicate clones with a new id/name; delete removes an interface (and collapses/drops the series if it empties per the singleton rule); add-interface appends an empty interface to a series. Depends on T003.
- [x] T024 [US2] Wire the surface/series menus into `web3/src/HierarchyPanel.tsx` (mount `SurfaceMenu` on each surface row and series header) and the callbacks into `web3/src/main.tsx` (`onEditSurface` opens `SurfaceDataModal`; rename/duplicate/delete/add-interface call the T023 helpers, `setDirty(true)`). Reuses the modal from US1 (T017). Depends on T019, T020, T022, T023.

**Checkpoint**: US1 + US2 work independently — surfaces can be added and freely edited, with one batched re-fit.

---

## Phase 5: User Story 3 — See clearly when the view is stale (Priority: P2)

**Goal**: An unmistakable stale-view scrim while changes are unapplied; cleared on Apply/Cancel; never shown for cheap edits.

**Independent Test**: Make a geometry edit → scrim + Apply/Cancel show; orbit still works; Apply → scrim clears; a seam-relation toggle re-fits immediately with no scrim.

### Tests for User Story 3 (behavior gate)

- [x] T025 [P] [US3] Write `web3/e2e/stale-scrim.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V3: after a geometry edit, the `data-testid="stale-scrim"` overlay is present and the dirty affordances show; after Apply it is gone; a cheap seam-relation toggle re-fits without ever showing the scrim. Assert on the element, not pixels.

### Implementation for User Story 3

- [x] T026 [US3] Confirm/finish the scrim wiring in `web3/src/main.tsx`: `Viewer stale={dirty}` (set in T011) plus verify the dirty/cheap-edit distinction holds — geometry edits set dirty (scrim), cheap declared-state edits (`onSetRelation`/`onSetUnboundedAbove`) re-fit immediately and never set dirty. Mostly assembled in Phase 2; this task closes the loop and makes T025 pass. Depends on T010, T011.

**Checkpoint**: Staleness is visible 100% of the time it exists and never falsely (SC-004).

---

## Phase 6: User Story 4 — Reorder the chronology of the stack (Priority: P2)

**Goal**: Reorder containers (chronology) and within-series surfaces; both set dirty and re-fit on Apply; cross-container move is declined with a message.

**Independent Test**: Reorder two containers → panel order + dirty change; Apply → derived order/relations change as expected (round-trip); cross-container move shows the declined toast and changes nothing.

### Tests for User Story 4 (behavior gate)

- [x] T027 [P] [US4] Write `web3/e2e/reorder.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V4: reorder two containers (drive the wired `onReorderContainers` via a test hook / assert on `resolveDrop` + a programmatic reorder — **no real `@dnd-kit` drag**, mirroring `drag-affordance.spec`); the panel order changes and dirty shows; after Apply the derived `effectiveRelation`/order changes (mirror `round-trip.spec`); a cross-container move is declined with a toast and no project change.

### Implementation for User Story 4

- [x] T028 [US4] Implement the reorder helpers on `Project` in `web3/src/HierarchyPanel/project.ts`: `reorderContainers(project, fromId, toId)` reorders `elements` (chronology); `reorderWithin(project, elementId, from, to)` reorders a series' `interfaces`. Pure. Depends on T003.
- [x] T029 [US4] Wire the reorder callbacks in `web3/src/main.tsx` consuming `dragResolver.ts` output (unchanged): `onReorderContainers` → `reorderContainers` + `setDirty(true)`; `onReorderWithin` → `reorderWithin` + `setDirty(true)`; `onMoveSurface` → a **declined** toast ("moving a surface between containers isn't supported — delete and re-add"), no project change (FR-016 — not a silent noop). Depends on T011, T028.

**Checkpoint**: Reorder round-trips through the fit (SC-005); the punt is a visible decline, not silence.

---

## Phase 7: User Story 5 — Persist and exchange the authored model (Priority: P2)

**Goal**: Autosave + reload restore; export/import the project file (confirm-if-dirty); seed from templates.

**Independent Test**: Author → reload restores it; export → New → import reproduces it (confirm if dirty); seed from each template + blank loads a valid project.

### Tests for User Story 5 (behavior gate)

- [x] T030 [P] [US5] Write `web3/e2e/persistence.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V7: a committed edit survives reload (localStorage restore); export then import reproduces the model; importing while dirty asks to confirm; seeding from a template loads a valid project. (Brief §10 names four new specs; this story's persistence behavior is real and gets its own spec to keep the gate honest, per Constitution I.)

### Implementation for User Story 5

- [x] T031 [US5] Add the **Project bar** to `web3/src/main.tsx` per [contracts/ui-callbacks.md](./contracts/ui-callbacks.md): editable project name, New / Import / Export buttons, and a "Seed from template ▾" menu listing the three former scenarios + blank. New = blank template (confirm if dirty); Import calls `importProjectJson` (confirm if dirty) then sets project; Export calls `exportProjectJson`; Seed loads a template `Project`. Depends on T007, T009, T011.
- [x] T032 [US5] Confirm autosave/restore is live: debounced `saveProject` fires on each committed edit and `loadProject()` is the initial state seed in `web3/src/main.tsx` (wired in T011); this task verifies the round-trip and makes T030 pass. Depends on T011, T031.

**Checkpoint**: Authored work survives reload and a file round-trip (SC-006).

---

## Phase 8: User Story 6 — Export the rendered model to OBJ (Priority: P3)

**Goal**: One-click client-side OBJ export of the rendered surfaces, grouped per surface, available only when fit and not dirty.

**Independent Test**: With a fit and not dirty, Export → OBJ downloads a file with non-trivial `v`/`f` counts and per-surface groups; unavailable when dirty.

### Tests for User Story 6 (behavior gate)

- [x] T033 [P] [US6] Write `web3/e2e/export-obj.spec.ts` [NEW] per [quickstart.md](./quickstart.md) V5: trigger Export → OBJ; capture the download; parse the blob and assert vertex + face counts > 0 and per-surface `g` groups; the action is unavailable while dirty.

### Implementation for User Story 6

- [x] T034 [P] [US6] Implement OBJ serialization in `web3/src/exportObj.ts` [NEW] per [contracts/project-model.md](./contracts/project-model.md)/brief §8: `SurfacePayload[]` → one OBJ string (`v`/`vn` from positions/normals, `f` with `//` normal refs, vertex-index offset accumulated across surfaces, `g series{si}_block{b}_iface{k}` groups); materials optional/deferred — log the coverage bound in a comment (Constitution IV). Pure; no worker round-trip. Depends only on the `SurfacePayload` type (existing).
- [x] T035 [US6] Add the **Export mesh ▾ → OBJ** action to the project bar in `web3/src/main.tsx`: enabled only when `surfaces.length > 0 && !dirty`; calls `exportObj` (T034) on the held `surfaces` and triggers a `<project-name>.obj` Blob download. Depends on T031, T034.

**Checkpoint**: All six stories are independently functional (SC-007).

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Constitution gates and final calibration. (Only the principles this feature actually touches are kept — it changes no kernel math, no wasm, no license seam.)

- [x] T036 Run the compile gate `cd web3 && npm run build` (`tsc -b && vite build`) and fix any type/build errors across all new/changed files (Constitution I — necessary gate).
- [x] T037 Run the behavior gate `cd web3 && npm run test:e2e` and ensure all new specs (authoring-add, modal-validation, stale-scrim, reorder, persistence, export-obj) plus the retained existing specs pass (Constitution I — gate of record). If the environment cannot run Playwright this pass, log it as an un-closed gap (Constitution IV); do not report verified.
- [x] T038 [P] Confirm the Constitution III boundary holds: `validate(project)` checks only structural preconditions and no geological rule is re-encoded in the UI (all effective-relation/cross-cut/fitted-validation rendering still reads `CoreUpdate.derived`). Add/keep the boundary comment in `project.ts`.
- [x] T039 [P] Confirm the WASM/license seam is untouched (Constitution V): `git diff --stat` shows no change under `crates/`, `web3/src/wasm/`, `web3/src/wasm-kernel/`, `worker.ts`, `protocol.ts`, `dto.ts`, `store.ts`, or `kernel-bridge.ts`. The authoring layer added no Rust and no wasm-reachable dependency.
- [x] T040 [P] Calibrate status claims and log coverage bounds where a reader sees them (Constitution IV): the documented punts (faults, cross-container move, insert-at-selection, in-modal preview, CSV file import, OBJ materials) are noted in code/docs at their seams, not silently present; update the brief or a short status note if anything landed differently than `docs/enhanced_authoring_spec.md` describes (Constitution II — the document wins; amend it deliberately if so).
- [x] T041 Final cleanup of `web3/src/HierarchyPanel/` and `web3/src/main.tsx`: remove dead scenario-dropdown code superseded by templates, dedupe any provisional helpers, ensure `index.ts` exports the intended public surface, and confirm no `noop` reorder callbacks remain.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies.
- **Foundational (Phase 2)**: depends on Setup. **Blocks all user stories** — the project document, `toSeriesJson`, `validate`, persistence, the table, the scrim-capable viewer, and the project-centric App state are read by every story.
- **User Stories (Phase 3–8)**: all depend on Foundational. Then:
  - **US1 (P1)** is the MVP; **US2 (P1)** depends on US1 (reuses `SurfaceDataModal` and the panel surface-row entry points).
  - **US3, US4, US5, US6** depend only on Foundational and can proceed in parallel with each other (and after US1 for any panel-row interaction). US6 needs the project bar from US5 for its button placement (T035 → T031) but `exportObj.ts` itself (T034) is independent.
- **Polish (Phase 9)**: after all desired stories.

### User Story Dependencies

- **US1 (P1)**: after Foundational. The MVP slice.
- **US2 (P1)**: after US1 (shares the modal + panel rows). Independently testable once US1 lands.
- **US3 (P2)**: after Foundational (scrim prop is foundational; US3 closes the dirty/cheap-edit distinction + its spec). Independent of US1/US2.
- **US4 (P2)**: after Foundational. Independent.
- **US5 (P2)**: after Foundational. Independent (project bar + persistence). 
- **US6 (P3)**: `exportObj.ts` after Foundational; the button (T035) after US5's project bar (T031).

### Within Each User Story

- The Playwright spec is authored to pin the behavior (Constitution I); it should fail before the story's implementation lands and pass after.
- Pure `project.ts` helpers before the components that call them; components before their `main.tsx` wiring.

### Parallel Opportunities

- **Phase 2**: T003 first (types), then T004/T006/T007/T008 in parallel (all touch `project.ts` functions or separate files — note T004/T006/T007 share `project.ts`, so if one agent owns that file, run them sequentially; T008 `tables.tsx`, T010 `Viewer.tsx` are independent files and truly parallel). T005 after T003+T004. T009 after T005. T011 integrates everything.
- **Specs are parallel**: T014, T015, T025, T027, T030, T033 are all separate `e2e/*.spec.ts` files — write in parallel.
- **Across stories** (after Foundational): US3, US4, US5 implementation tracks are largely independent files and can be staffed in parallel; US6's `exportObj.ts` too.

---

## Parallel Example: Foundational `project.ts` + sibling files

```bash
# After T003 (types) lands, these touch different files and can run together:
Task: "T008 Build reusable table in web3/src/HierarchyPanel/tables.tsx"
Task: "T010 Add stale scrim prop to web3/src/Viewer.tsx"
# Within project.ts, run sequentially (same file): T004 → T005, plus T006, T007.
```

## Parallel Example: behavior-gate specs

```bash
# All separate e2e files — author in parallel to pin each story's behavior:
Task: "T014 web3/e2e/authoring-add.spec.ts"
Task: "T025 web3/e2e/stale-scrim.spec.ts"
Task: "T027 web3/e2e/reorder.spec.ts"
Task: "T030 web3/e2e/persistence.spec.ts"
Task: "T033 web3/e2e/export-obj.spec.ts"
```

---

## Implementation Strategy

### MVP First (US1 only)

1. Phase 1 Setup → Phase 2 Foundational (the bulk of the structural work) → Phase 3 US1.
2. **STOP and VALIDATE**: T014 + T015 pass; a user can author each surface type end to end.
3. Demo: this alone converts the testbed from canned to authorable.

### Incremental Delivery

1. Setup + Foundational → foundation ready.
2. US1 → test → demo (MVP: add + author surfaces).
3. US2 → test → demo (full editing, batched re-fit).
4. US3 → test (stale signal honest).
5. US4 → test (reorder round-trips; punt is visible).
6. US5 → test (durable + portable).
7. US6 → test (OBJ artifact).
8. Polish → gates green, seams confirmed.

### Notes

- `[P]` = different files, no incomplete-task dependency. Tasks naming the same file (`project.ts`, `main.tsx`) are **not** parallel with each other even when both touch that file.
- The `[Story]` label maps each task to its spec user story for traceability.
- Commit after each task or logical group; scoped/typed messages (`feat(web3): …`).
- No Rust/wasm/worker/ABI/schema file is modified — Phase 9 T039 verifies this.
- If Playwright can't run in this environment, that is a logged, un-closed gap (Constitution IV), never a silent "verified."
