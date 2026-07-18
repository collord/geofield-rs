# Tasks: Cross-Sections & Digitized Surfaces

**Input**: Design documents from `/specs/005-cross-section-digitizing/`

**Prerequisites**: plan.md, spec.md, research.md (R1–R10), data-model.md,
contracts/younging-derivation.md, contracts/project-json.md, quickstart.md

**Tests**: INCLUDED — Constitution Principle I makes Playwright functional tests the gate of record
for UI/behavioral acceptance; each story phase ends with its e2e gate task. They are written after
the story's implementation (they need `data-testid`s and hooks to exist) but the story is NOT done
until they pass. `tsc -b && vite build` must stay green after every task (compile gate).

**Organization**: Tasks are grouped by user story (spec.md US1–US6). All paths are under
`web3/` unless noted. No Rust/wasm/worker-ABI change anywhere (plan Constitution Check V/VI).

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: US1–US6 from spec.md

## Phase 1: Setup

**Purpose**: Module skeleton — no project init needed (web3 toolchain exists; no new deps).

- [X] T001 Create the pure-geometry module skeleton `web3/src/section/frame.ts`, `web3/src/section/slice.ts`, `web3/src/section/digitizeDerive.ts` — exported type/function signatures only, each file header citing its contract section (`contracts/younging-derivation.md` §1 / research R1–R2 / §2–4); keep the module free of React/three imports (types only) so it stays viewer-agnostic (plan Structure Decision)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Frame math, data model, and the non-dirty section state path that every story reads.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T002 [P] Implement `SectionFrame` in `web3/src/section/frame.ts`: `frameFromBaseline(a,b)` → `{u,w,s}` (u horizontal-normalized, w=+Z, s=u×w), `planeK(set,k)` → `{origin, s}` with `offset + k·spacing`, world↔in-plane `(du,dw)` mapping, `slabTol(set, extent)` (spacing/2 serial, 2.5% diagonal standalone — research R8), degenerate-baseline guard `|b−a| > εbaseline`; per contracts/younging-derivation.md §1
- [X] T003 [P] Extend the data model in `web3/src/HierarchyPanel/project.ts`: add `SectionSet`, `Project.sections?`, `Project.view.section?` (ActiveSection), `DigitizedLine` (`frame` snapshot + world `vertices` + soft `sectionRef`), `InterfaceData.digitized?` — all additive, version stays 1; extend `importProjectJson` with the guard table from contracts/project-json.md (count/spacing rejects, view clamp/drop, ŝ renormalize, off-plane vertex → hard validation error not load failure); autosave/export pick the fields up automatically via JSON round-trip — verify `slimForExport` leaves them intact
- [X] T004 Add section state to `web3/src/main.tsx`: non-dirty updaters `addSectionSet`/`updateSectionSet`/`deleteSectionSet`/`setActiveSection` cloned from the `setRelation`/`commitLithology` immediate path (`main.tsx:365,390` — save immediately, NEVER `commit`, never re-fit, `dirty` untouched per spec FR-009); expose `window.__sections()` beside the existing hooks (`main.tsx:471-511`)

**Checkpoint**: Foundation ready — types compile, sections can exist in state, no UI yet.

---

## Phase 3: User Story 1 — Create and read a cross-section (Priority: P1) 🎯 MVP

**Goal**: Two plan-view clicks → a section; open it face-on (ortho, u right / Z up) showing traces
of the applied model + backdrop points, sharing render mode and stale-scrim; zero dirty/re-fit.

**Independent Test**: quickstart.md manual walk steps 1–3 + `e2e/section-view.spec.ts` (SC-001, SC-004).

### Implementation for User Story 1

- [X] T005 [US1] Baseline two-click gesture in `web3/src/Viewer.tsx`: clone the `drawBounds` effect (`Viewer.tsx:432-539`) as `drawSectionBaseline` — same top-down snap, ray∩`THREE.Plane` at z-mid, amber rubber line A→pointer, Escape cancels, nonce-object trigger prop; on second click call `props.onSectionBaseline(a, b)`; reject `|b−a| ≤ εbaseline` (ignore the second click)
- [X] T006 [US1] Create `web3/src/HierarchyPanel/SectionsPanel.tsx`: section-set list (name, count), **New section** button (fires the baseline nonce; creates a `count:1` set via `addSectionSet`), open-in-section-view, delete; `data-testid`s `section-new`, `section-open-<id>`, `section-delete-<id>`; mount in the left panel in `web3/src/main.tsx`
- [X] T007 [P] [US1] Implement contact traces in `web3/src/section/slice.ts`: per-triangle plane-crossing walk over `SurfacePayload.positions/indices` (`protocol.ts:96-98`) → world segments → chained polylines keyed by surface (research R1); O(#tris), no allocation churn on step
- [X] T008 [P] [US1] Implement solid-interval fill input in `web3/src/section/slice.ts`: segment→loop assembly by endpoint hashing in `(du,dw)`, even-odd hole classification, output outer/hole loops for `THREE.ShapeGeometry`; open loops → return as unfilled polylines (the logged Manifold-off fallback, research R2 — leave the bound comment here)
- [X] T009 [US1] Section-view mode in `web3/src/Viewer.tsx`: `THREE.OrthographicCamera` positioned on the side `ŝ` points toward, looking −s, `up=+Z`, frustum = baseline-length × extent Z-range (contracts/younging-derivation.md §1); OrbitControls `enableRotate=false` (pan/zoom only); enter/exit on an `activeSection` prop; handle resize; restore the 3D perspective camera state on exit
- [X] T010 [US1] Render the section scene in `web3/src/Viewer.tsx`: dedicated `stateRef` groups for traces (contact mode, element colors matching 3D), filled intervals (solid mode, lithology colors), and backdrop authored points within `slabTol` (spheres, reusing overlay conventions); rebuild on `surfaces`/active-section change
- [X] T011 [US1] Plumb shared signals into section view in `web3/src/main.tsx`: stale-scrim (`stale={dirty}` applies in section mode too), render-mode toggle usable in section view via existing `setRenderModeAndRerun` with the explicit-override convention (`main.tsx:316-320`)
- [X] T012 [US1] Add `window.__sectionTraces(setId, index)` hook in `web3/src/main.tsx` returning `{surfaceId, polylines}[]` from the slicer (contracts/project-json.md hook table)
- [X] T013 [US1] Write `web3/e2e/section-view.spec.ts` pinning: two clicks create a set (`__sections`); open → ortho face-on; traces present for an applied template (`__sectionTraces` non-empty); backdrop points; scrim appears with pending edits; render-mode toggle changes section content; **create/step/delete N sections keeps `dirty` false and runs zero fits** (assert status line never re-enters "fitting")

**Checkpoint**: US1 fully functional — sections as pure inspection views. MVP.

---

## Phase 4: User Story 2 — Digitize a contact line with directional younging (Priority: P2 ordering — spec P1)

**Goal**: Plane-locked polyline tool on the open section; vertices are contact points (table/overlay
union); drawn direction → live younging affordance; flip never rewrites vertices.

**Independent Test**: quickstart walk step 4 (single slice) + `e2e/section-digitize.spec.ts` (SC-003).

**Depends on**: US1 (needs the section view to draw in).

### Implementation for User Story 2

- [X] T014 [P] [US2] Implement `web3/src/section/digitizeDerive.ts`: per-segment `t̂ᵢ` and pole `mᵢ = ŝ × t̂ᵢ`, orientation midpoints, zero-length-segment skip, per-line mean pole, in-plane self-cross/back-track detection — exactly per contracts/younging-derivation.md §2/§4 (the worked example is the reference case)
- [X] T015 [US2] Digitized-line ops in `web3/src/HierarchyPanel/projectOps.ts` (all through `commit` — these ARE geometry): `addDigitizedLine(surfaceId, frame, sectionRef)`, `appendVertex`, `moveVertex` (project onto the line's plane), `deleteVertex`, `deleteLine`; plus target-surface selection helper that can create a new single-interface `series` element in place (spec US-DIG-1)
- [X] T016 [US2] Digitizing tool in `web3/src/Viewer.tsx`: active in section view with a chosen target — click appends, click near first/last extends, drag moves a vertex, Delete/alt-click removes; all picks via `raycaster.ray.intersectPlane` against the active plane (reuse the T005 picking path); tool state survives camera pan/zoom
- [X] T017 [US2] Target-surface picker + tool toggle UI in `web3/src/HierarchyPanel/SectionsPanel.tsx` (dropdown of parametric surfaces + "new surface…"), `data-testid` `digitize-target`, `digitize-toggle`
- [X] T018 [US2] Younging affordance in `web3/src/Viewer.tsx`: direction cones along the line + hachure ticks offset toward `sign·mᵢ` (the EFFECTIVE pole after the flip XOR — contracts §3), live during draw/edit and on applied lines
- [X] T019 [US2] Points table & overlay union: `web3/src/HierarchyPanel/SurfaceDataModal.tsx` shows digitized vertices as provenance-tagged rows (edits write through `moveVertex`, i.e. plane-projected; delete removes the vertex); `buildAuthoredOverlay` (`project.ts:324`) includes digitized vertices + per-segment effective poles as normal arrows — derived at render, no copy (FR-012, Constitution III)
- [X] T020 [US2] Add `window.__digitizeLine(surfaceId, setId, index, verts)` hook in `web3/src/main.tsx` (programmatic commit path for e2e; contracts/project-json.md), then write `web3/e2e/section-digitize.spec.ts`: canvas-click digitizing on the plane (gesture path, pattern of `e2e/model-bounds.spec.ts`); vertices in table + `__overlayCounts`; affordance on the correct side for a known baseline/draw direction (the contract's worked example, rendered); **flip toggles the younger side with zero vertex deltas, double-flip restores** (compare `__loadProject`-visible vertex arrays)

**Checkpoint**: US1+US2 — drawing on one slice produces real, visible, flippable contact data.

---

## Phase 5: User Story 3 — Serial sections, step, digitize across, loft on Apply (Priority: P1 end-to-end)

**Goal**: Spacing×count serial sets; step with tool active; one Apply lofts all lines into one fitted
surface whose traces land back on the drawn lines; §11.4 validation gates the Apply.

**Independent Test**: quickstart walk steps 2–4 across ≥3 slices + `e2e/section-loft.spec.ts` and
`e2e/section-validation.spec.ts` (SC-002, SC-007).

**Depends on**: US1 + US2.

### Implementation for User Story 3

- [X] T021 [P] [US3] Serial-set creation dialog in `web3/src/HierarchyPanel/SectionsPanel.tsx`: after the baseline gesture, spacing / count / start-offset inputs (defaults sane vs extent), producing one `SectionSet`; show member index (`k+1/count`) on open sets; `data-testid` `section-serial-spacing|count|offset`
- [X] T022 [US3] Step next/prev in `web3/src/main.tsx` + `web3/src/Viewer.tsx`: mutate `view.section.index` (non-dirty, clamped at ends per spec assumption), camera re-targets to plane k, traces/fills/backdrop recompute, **digitizing tool + target survive the step** (spec US-SECT-4); `data-testid` `section-step-next|prev`; keyboard PageUp/PageDown
- [X] T023 [US3] Compile path in `web3/src/HierarchyPanel/project.ts`: extend `flattenInterfaces` (`project.ts:235-261`) — after typed rows, per digitized line emit each vertex → `sp_xyz`/`sp_if` and each segment → midpoint `ori_xyz` + `ori_pole = sign·(ŝ×t̂)` using `digitizeDerive` and the EXISTING flip XOR at the same single site (contracts §3); `SeriesJson`/ABI untouched; cite the contract at the site
- [X] T024 [US3] Validation rules in `web3/src/HierarchyPanel/project.ts` `validate()` (`project.ts:438`, stays structural — keep the Constitution III note honest): hard — line `<2` vertices; entire surface data on one plane and nothing else; (import-guarded) off-plane vertex. Soft — under-constrained (one section / no across-section variation); inconsistent younging via `dot(meanPole_i, meanPole_j) < 0`; self-cross/back-track (data-model.md rules 1–6); reasons name the surface and line
- [X] T025 [US3] "Flip this line to match" affordance: warning row in the apply bar / SectionsPanel offering the per-line fix; action reverses that line's `vertices` order via a `commit` edit in `web3/src/HierarchyPanel/projectOps.ts` (contracts §3 — negates that line's poles only); `data-testid` `flip-line-to-match`
- [X] T026 [US3] Write `web3/e2e/section-loft.spec.ts`: seed template → serial set (≥3 slices) → `__digitizeLine` a consistent contact on each → assert scrim until Apply → Apply once → new surface in stack; **for every drawn vertex, the re-fit surface's trace on that slice passes within ε = 0.1·spacing** (in-plane distance via `__sectionTraces`)
- [X] T027 [US3] Write `web3/e2e/section-validation.spec.ts`: the §11.4 matrix — 1-vertex line blocks with reason (`apply-reason`); single-plane-only-data blocks; one-section-only warns but Applies; opposed-direction sibling line warns and `flip-line-to-match` clears it; self-crossing line warns (SC-007 — every hard case blocks, every soft case doesn't)

**Checkpoint**: The end-to-end payoff works — parallel traces become one fitted surface, gated.

---

## Phase 6: User Story 4 — Digitized lines are ordinary, editable data (Priority: P2)

**Goal**: Reopen, move/delete vertices on any slice, re-digitize a slice, add a slice — all deferred
edits; table and section are two views of one dataset.

**Independent Test**: spec US4 acceptance 1–4 as e2e scenarios (most ops exist from T015/T019).

**Depends on**: US2 (US3 for the multi-slice scenarios).

### Implementation for User Story 4

- [X] T028 [US4] Re-digitize-slice action: "redraw this slice" for the target surface on the active section (deletes that line, arms the tool) in `web3/src/HierarchyPanel/SectionsPanel.tsx` + `projectOps.ts`; plus per-line delete in the section view; `data-testid` `redigitize-slice`
- [X] T029 [US4] Extend `web3/e2e/section-digitize.spec.ts` with the US4 scenarios: move a vertex on one slice → Apply → surface updates (trace moves); delete a vertex; re-digitize a slice replaces its contribution; edit a digitized vertex via the points table and see the section line move (two-way, plane-projected); each edit shows the scrim until Apply

**Checkpoint**: Digitizing is a durable authoring route, not a one-shot.

---

## Phase 7: User Story 5 — Role & relation parity (Priority: P2)

**Goal**: A digitized surface takes any role/relation and composes under the normal cut rules,
indistinguishable from a typed surface.

**Independent Test**: erode-truncation on a lofted surface vs the identical typed surface (SC-006).

**Depends on**: US3.

### Implementation for User Story 5

- [X] T030 [US5] Confirm/wire parity in `web3/src/HierarchyPanel/projectOps.ts` + `HierarchyPanel.tsx`: the digitize-created `series` gets the standard relation control (conformable/onlap/erode — immediate, non-dirty per US-DATA-3), participates in reorder/rename/delete/duplicate; a digitized line on a vein-wall or topography interface flows through the same `flattenInterfaces` path (verify `defaultFlip` composition for erode — contracts §3); fix anything that special-cases `digitized`
- [X] T031 [US5] Extend `web3/e2e/section-loft.spec.ts`: set the lofted surface's relation to erode → solids mode → assert older units truncate (reuse existing solid-mode assertions); toggle to onlap → pinch-out behavior changes; assert downstream export (`__slimForExport`/OBJ enablement) treats it like any surface

**Checkpoint**: Full parity demonstrated behaviorally.

---

## Phase 8: User Story 6 — Orientation in space & persistence (Priority: P3)

**Goal**: Translucent plane quad in 3D; sections + active slice survive export/import.

**Independent Test**: `e2e/section-persist.spec.ts` (SC-005) + quad visible in 3D.

**Depends on**: Foundational (T003) + US1 panel; independent of US2–US5.

### Implementation for User Story 6

- [X] T032 [P] [US6] 3D plane quads in `web3/src/Viewer.tsx`: per `showInScene` set, translucent quads for each plane (active slice highlighted), baseline segment + normal `ArrowHelper`; toggle in `web3/src/HierarchyPanel/SectionsPanel.tsx` (`data-testid` `section-show-<id>`), non-dirty
- [X] T033 [P] [US6] Persistence wiring in `web3/src/main.tsx`: `onExport` (`main.tsx:562-575`) captures `view.section`; `loadFreshProject` (`main.tsx:518-547`) restores sections, clamps/drops a stale `view.section` (contracts/project-json.md guards), and reopens the active section view; localStorage autosave round-trips via T003
- [X] T034 [US6] Write `web3/e2e/section-persist.spec.ts`: export with a serial set open on slice k → New project → import → sections, serial params, and active slice restored (`__sections` + `__getView`); a pre-005 `.geoproj.json` fixture (no new fields) loads cleanly; a 005 file with digitized lines round-trips vertices bit-exactly

**Checkpoint**: All six stories functional and independently gated.

---

## Phase 9: Polish & Cross-Cutting Concerns

- [X] T035 [P] Merge `web3/SPEC_cross_sections.md` into `web3/SPEC.md` as §10/§11 with the §11.2 sign fix (`m = ŝ × t̂`, amendment note citing contracts/younging-derivation.md) and renumbered acceptance; reduce the fragment file to a pointer (Principle II, research R10)
- [X] T036 [P] Place the coverage-bound notes where a reader sees them (Principle IV): Manifold-off unfilled-fallback comment at the fill site (`web3/src/section/slice.ts`, done in T008 — verify), e2e-only pinning note in `web3/src/section/` module headers (research R9), per-member serial-set ops out-of-scope note in `SectionsPanel.tsx`
- [X] T037 Run the full gates: `cd web3 && npm run build && npx playwright test` — all five section specs plus the whole existing suite green (no regressions); confirm `git diff` touches no Rust crate, `web3/src/worker.ts`, or `web3/src/protocol.ts` ABI types (Principles I, V — the seam check is "no crossing occurred")
- [X] T038 Cleanup & calibration: remove dead scaffolding, ensure new symbols cite their contract sections, update `specs/005-cross-section-digitizing/spec.md` status Draft → Implemented only for what the e2e gates actually pin (Principle IV)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (P1) → Foundational (P2)** → everything else.
- **US1 (Phase 3)**: after Foundational. The MVP.
- **US2 (Phase 4)**: after US1 (draws inside the section view).
- **US3 (Phase 5)**: after US1 + US2.
- **US4 (Phase 6)**: after US2 (multi-slice scenarios also want US3).
- **US5 (Phase 7)**: after US3.
- **US6 (Phase 8)**: after Foundational + T006 (panel); independent of US2–US5 — can run in parallel with Phases 4–7.
- **Polish (Phase 9)**: after all desired stories; T035/T036 can start once US3 lands.

### Within stories

- Pure geometry before UI (T007/T008 before T010; T014 before T016–T018).
- Ops/model before tool (T015 before T016), tool before affordance test.
- Each story's e2e task is last and gates the checkpoint.

### Parallel Opportunities

- T002 ∥ T003 (different files); T004 after both.
- Within US1: T007 ∥ T008 ∥ (T005, T006); T009/T010 after T007/T008.
- US6 (T032 ∥ T033) can proceed alongside Phases 4–7 once T006 exists.
- T035 ∥ T036 in Polish.

## Parallel Example: after Foundational

```bash
# Developer A (US1 core):    T005 → T006 → T009 → T010 → T011 → T012 → T013
# Developer B (slicer, [P]): T007 ∥ T008, then joins A at T010
# Developer C (US6, [P]):    T032 ∥ T033 → T034   (after T006 exists)
```

## Implementation Strategy

**MVP first**: Phases 1–3 (T001–T013) deliver sections as a pure inspection aid — independently
valuable, zero geology risk (nothing touches the fit). **Stop and validate** with
`e2e/section-view.spec.ts`.

**Increment 2** (the headline): Phases 4–5 (T014–T027) — digitizing + loft, gated by the
land-on-line and validation specs. **Increment 3**: Phases 6–8 in any order (US6 parallelizable).
**Close**: Phase 9 merges the governing doc and locks the gates.

Commit per task or coherent group (`feat(web3): …` / `test(web3): …`), on `main` per user convention.

---

## Implementation deviations (logged, Principle IV)

- **T009 "handle resize"**: not implemented — the Viewer has never handled window resize for the
  perspective camera either (renderer sized once on mount); the section camera matches that
  pre-existing behavior rather than adding a new capability. Seam: a future resize handler should
  update both cameras.
- **T012/hook shapes**: `__sectionTraces` identifies payloads by worker `(series, block, iface)`
  indices, not a `surfaceId`; `__digitizeLine` takes explicit `(elementId, interfaceId)` (null ⇒
  create). contracts/project-json.md was updated to the as-built shapes.
- **T004 naming**: the non-dirty path landed as `commitSections(next)` + per-action handlers in
  `main.tsx` (equivalent semantics to the named updater list in the task text).
- **US5 truncation gate**: asserted geometrically (max older-bed height above the eroding surface
  flips from +660 to −9.5) — triangle-count deltas are NOT monotone under erode clipping (seam
  refinement increases counts) and were abandoned as an oracle.
- **Stale-closure commit race (FIXED in-app, not test-side)**: back-to-back digitize commits
  (pointer events, `__digitizeLine` after `__addSectionSet`) outran React renders; a commit built
  on the stale `project` closure silently dropped the prior edit (last-write-wins). All digitizing
  paths now commit through functional `setProject(prev => …)` updaters (`commitFn` in main.tsx),
  with the section-set lookup resolved inside the updater. The five gates pass 3× consecutively.
- **Pre-existing failures, NOT feature 005** (each verified against the stashed pre-005 baseline
  on this machine): `e2e/view-roundtrip.spec.ts` (two identical solid-mode fits return meshes with
  different bounding boxes, min-z 29.09 vs 0, so the data-box-keyed auto-frame overrides the
  restored camera), `e2e/smoke.spec.ts` (all-templates fit loop is marginal against its 30s test
  budget — the imported-mesh template alone fits in ~20s here), and
  `e2e/imported-mesh-erosion-stack.spec.ts` (solid-mode "meshing solids (Manifold)…" exceeds the
  120s budget). Final gate of record: full serial suite = 62 passed + these 3 baseline failures;
  the five 005 section specs pass 15/15 across 3 consecutive runs and in every full-suite run.
  Engine/solid-path investigation is out of this feature's scope — left as named, reproducible gaps.
