---
description: "Task list for Imported Mesh Surfaces"
---

# Tasks: Imported Mesh Surfaces

**Input**: Design documents from `/specs/004-imported-mesh-surfaces/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: REQUIRED. Per Constitution Principle I, correctness for `geofield` (mesher) work is established by **geometric invariants + native↔wasm parity**, and UI acceptance by a **Playwright** functional test — these are the oracle, not optional. Invariant IDs (INV-1..7) are defined in [contracts/invariants.md](./contracts/invariants.md).

**Organization**: by user story (US1/US2/US3 from spec.md), each an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: parallelizable (different files, no incomplete-task dependency)
- **[Story]**: US1/US2/US3 for story-phase tasks only

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: fixtures + the deliberate governing-doc amendment, so the design of record is correct before building against it.

- [ ] T001 [P] Add `.obj` test fixtures under `web3/e2e/fixtures/` and a native copy under `crates/geofield/tests/fixtures/`: (a) `mesh-span-domain.obj` (bed covering the full domain), (b) `mesh-under-covering.obj` (bed strictly inside the domain — projection case), (c) `mesh-bad-winding.obj` (inconsistent per-component winding), (d) `mesh-degenerate.obj` (empty / zero-area triangles)
- [ ] T002 [P] Amend `docs/07_imported_mesh_field_proxy.md` (Principle II deliberate amendment): full-parity imports ADOPT a smooth far-field, **confined to beyond the mesh's axis-aligned bounds** (where the user supplied no geometry, so a smooth extrapolation is the honest choice — the original clip-only objection does not apply); record **patch-stitch** (not field-blend) as the mechanism and cite this plan/research.md

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: prove the one real geometry risk (patch-stitch) and stand up the shared parser + element scaffold every story builds on.

**⚠️ CRITICAL**: T003 is the ARCHITECTURE GATE (plan.md / quickstart.md). If it cannot hit INV-2/3/4 on fixture (b), revisit research.md D3 **before** building the element plumbing. T004–T008 block all user stories.

- [ ] T003 SPIKE (architecture gate): native harness `crates/geofield/tests/imported_mesh_spike.rs` (scratch-verify style) — build `MeshSdf` for fixture (b), contour it clipped to its AABB (exact patch); fit the co-kriged field through its verts+normals (decimated) and contour clipped to [outside AABB → domain box] (smooth patch); weld with `mesh_clip::clip`. Assert INV-2 (watertight + reaches box faces), INV-3 (exact-inside within tol), INV-4 (seam shared by index, crease ≤ tol); DECIDE whether the no-op `refine` hook must be implemented and pick the decimation `point_budget`. Record findings in a comment + update research.md D2/D3 open notes
- [ ] T004 [P] OBJ parser `crates/geofield/src/obj.rs`: parse `v` / `vn` / `f` over `&[u8]` → `(Vec<[f64;3]> verts, Vec<[usize;3]> faces, Option<Vec<[f64;3]>> normals)`; weld duplicate vertices; skip zero-area triangles; unit tests over the fixtures (native, BSD, no `std::fs`)
- [ ] T005 [P] Ingress hygiene in `crates/geofield/src/obj.rs` (+ reuse `mesh::normalize_winding_per_component` / `mesh_clip::ClipMesh::winding_normalized`): normalize winding per connected component; DETECT-and-report (not repair) empty/holes/non-manifold/self-intersection with a clear error status (FR-011); tests over fixtures (c) and (d)
- [ ] T006 Add `StackElement::ImportedSurface { mesh, role, relation }` to `crates/geofield/src/stack.rs` and extend EVERY match over `StackElement` (`present_at`, `model`, meshing dispatch) so the variant compiles and is inert until meshed
- [ ] T007 [P] Add `StackElementDto::ImportedSurface` to `crates/geofield/src/schema.rs` (`to_dtos` + `from` round-trip); the DERIVED field/patches are NOT in the DTO (Principle III) — only mesh + role + relation
- [ ] T008 `WasmStack::add_imported_surface(mesh_bytes, role, relation, decimate_frac, fault…)` skeleton in `crates/geofield/src/wasm.rs` (flat args per the wasm-bindgen trailing-arg constraint): parse (T004) → hygiene (T005) → build `MeshSdf` → push `StackElement::ImportedSurface`; the kriged fit is added in US2. Returns a clear error status on a rejected mesh

**Checkpoint**: patch-stitch proven; parser + element type + ABI skeleton exist; user stories can begin.

---

## Phase 3: User Story 1 - Import a mesh and use it as a stratigraphic contact (Priority: P1) 🎯 MVP

**Goal**: import an `.obj` that spans the domain, see it as a contact element, and have it separate the units above/below — honoring the imported triangles exactly.

**Independent Test**: import fixture (a); the element appears and renders; it divides two units; the rendered surface coincides with the input triangles within tolerance; unit solids are watertight.

### Tests for User Story 1

- [ ] T009 [P] [US1] Native invariant test `crates/geofield/tests/imported_mesh_exact.rs`: import fixture (a) → watertight closed unit solids (INV-1) + contour coincides with input triangles within tolerance (INV-3, sample points on input tris, measure distance to output surface)
- [ ] T010 [P] [US1] Playwright `web3/e2e/imported-mesh.spec.ts`: import `.obj` via the distinct mesh-import control (NOT the JSON Import) → an imported-surface element appears in the hierarchy and its geometry renders in the viewer (SC-009)

### Implementation for User Story 1

- [ ] T011 [US1] Exact-patch mesher in `crates/geofield/src/stack.rs`: contour `MeshSdf` clipped to the mesh AABB into watertight unit solids (reuse `below_solid` / `octree_dual_contour_scene`); wire into the `StackElement::ImportedSurface` meshing dispatch (depends T006, T008)
- [ ] T012 [P] [US1] Add `SeriesJson.importedMesh?` discriminator (sibling to `crossCut?`/`topography?`) in `web3/src/protocol.ts` per [contracts/protocol.md](./contracts/protocol.md)
- [ ] T013 [P] [US1] Add `ProjectElement { kind: "importedSurface" }` to `web3/src/HierarchyPanel/project.ts` (+ `toSeriesJson` compile to `importedMesh`) and `.geoproj` save/load round-trip (FR-012); mesh embedded, derived NOT stored
- [ ] T014 [US1] `buildStack` branch in `web3/src/worker.ts` → `stack.add_imported_surface(...)` (depends T008, T012)
- [ ] T015 [US1] Distinct ".obj import" affordance in `web3/src/main.tsx` (+ HierarchyPanel): read file bytes → add an `importedSurface` element → re-fit/re-mesh; element shows role/relation selection, rename, delete — kept separate from the `.geoproj` "Import" button (FR-013)
- [ ] T016 [US1] Wire the imported surface as a stratigraphic contact in `crates/geofield/src/stack.rs`: its `MeshSdf` (iso 0) drives units above/below via the existing unit-derivation + `clip_maskers` machinery; lithology assignment flows through its derived units identically to a fitted surface (Principle III — reuse, not re-encode)

**Checkpoint**: importing a domain-spanning `.obj` yields a working, watertight, exactly-honored contact surface. MVP.

---

## Phase 4: User Story 2 - Project an under-covering mesh out to the domain boundary (Priority: P1)

**Goal**: an imported mesh that stops short of the domain edges still produces a complete, watertight model — exact inside the bounds, smoothly extended to the box beyond them, continuous at the seam.

**Independent Test**: import fixture (b); the derived surface reaches the box faces; unit solids are watertight; inside the AABB it matches the input within tolerance; no crease/gap at the seam beyond tolerance.

### Tests for User Story 2

- [ ] T017 [P] [US2] Native invariant test `crates/geofield/tests/imported_mesh_projection.rs`: import fixture (b) → watertight AND reaches every domain box face (INV-2); seam continuous — seam edges incidence 2, seam vertices shared by index, dihedral ≤ tol (INV-4); assert exactly one surface component per unit (no spurious phantom sheet)

### Implementation for User Story 2

- [ ] T018 [US2] Kriged smooth fit in `crates/geofield/src/wasm.rs` `add_imported_surface`: fit `Model::fit` through mesh vertices (as `SurfacePoint`s, single `interface_id`) + normals (as `Orientation` poles) via `remote::fit` across the existing kernel wasm boundary, bounding the solve with `DecimateConfig.point_budget` (reuse the `decimate_frac` ABI) — Constitution V: no new seam (research.md D1/D2)
- [ ] T019 [US2] Smooth-patch mesher in `crates/geofield/src/stack.rs`: contour the kriged field clipped to [outside the mesh AABB → domain box] (reuse the `AxisBox` extent-clip; extrapolate-then-clip like a fitted surface)
- [ ] T020 [US2] Patch-stitch weld in `crates/geofield/src/stack.rs`: `mesh_clip::clip` the exact patch (T011) and the smooth patch (T019) at the shared AABB seam → single watertight surface; SHORT-CIRCUIT (FR-007) when the mesh AABB already contains the domain box (exact patch only, no projection)
- [ ] T021 [US2] Conditional on the T003 finding: implement the `mesh_clip` `refine` hook (`crates/geofield/src/mesh_clip.rs:182`) to nudge AABB-seam vertices onto the kriged field's zero set IF the index-shared weld alone misses INV-4 seam continuity; otherwise mark this task no-op-needed with the spike evidence (Principle VI — don't build it unless the spike says so)

**Checkpoint**: under-covering imports close watertight to the boundary with a continuous seam. The kicker works.

---

## Phase 5: User Story 3 - Role parity: erosion, vein wall, and topography (Priority: P2)

**Goal**: the imported surface works in every role a fitted surface can — erosion (truncates older units), vein/intrusion wall (bounds a tabular body), topography (caps to Air) — with units/lithology/export identical to the fitted equivalent.

**Independent Test**: per role, build a scene with the imported surface in that role and assert the role's characteristic result; confirm unit ladder + lithology + OBJ-by-lithology behave as for a fitted surface.

### Tests for User Story 3

- [ ] T022 [P] [US3] Native per-role invariant tests `crates/geofield/tests/imported_mesh_roles.rs`: erosion truncates older units (INV-6); veinWall bounds a body; topography caps to Air above / rock below — each mirroring the fitted-equivalent invariant
- [ ] T023 [P] [US3] Playwright `web3/e2e/imported-mesh.spec.ts` (extend): assign the imported surface as erosion → older units truncate; as topography → Air cap; save then reload → the surface + role persist (FR-012)

### Implementation for User Story 3

- [ ] T024 [US3] Erosion role in `crates/geofield/src/stack.rs`: the imported surface participates in `clip_maskers` as a younger erode exactly as a fitted erosion does (reuse; no new masker) — respects the `erode ⇒ deposit-above` convention
- [ ] T025 [US3] Vein/intrusion wall role in `crates/geofield/src/stack.rs`: the imported surface serves as a band wall via the existing `band_solid` / vein path
- [ ] T026 [US3] Topography role in `crates/geofield/src/stack.rs`: the imported surface serves as the ground cap (Air above / rock below) via the existing topography machinery
- [ ] T027 [US3] Units / lithology / OBJ-export parity: confirm the imported surface's derived units flow through `DerivedView.units` (with the elevation-ordering already in place), lithology assignment, and solid-mode OBJ-by-lithology grouping identically to a fitted surface — extend the UI role/relation controls in `web3/src/HierarchyPanel/` to cover the imported element

**Checkpoint**: an imported mesh is a drop-in for any fitted surface. Full parity.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: the Constitution gates this feature touches + calibration.

- [ ] T028 [P] Re-verify native↔wasm `f64` parity for the imported-surface paths (parser, patch-stitch, fit-marshalling) — pinned natively + asserted as a design invariant + the Playwright cross-target run (INV-7, SC-007; Principle I/V)
- [ ] T029 [P] Confirm the EUPL/BSD + wasm seam is intact: the OBJ parser is BSD in `geofield` with no `std::fs`; the kernel is still absent from `geofield.wasm`; no native-only dep entered a wasm-reachable path (Principle V)
- [ ] T030 [P] Extend invariant tests to pin failure modes of the new path: absence of a spurious phantom sheet in the transition band, and seam watertightness under a non-convex import (Principle I)
- [ ] T031 [P] Update status claims to what landed: `docs/07_imported_mesh_field_proxy.md`, this spec/plan, and any README Scope — log the coverage bounds (coarse far-field FR-006, AABB-not-tight coverage, `.obj`-only, no repair, no coordinate transform) where a reader sees them (Principle IV)
- [ ] T032 Calibrate tolerances as PINNED NUMBERS (not vague) from the T003 spike: INV-3 in-coverage deviation, INV-4 seam crease angle, the decimation `point_budget`, and the working octree depth — recorded in the tests (Principle I/IV)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: no dependencies — start immediately. T002 (doc amendment) should land before implementation so the design of record is correct.
- **Foundational (Phase 2)**: T003 is the architecture gate (do first). T004–T008 block all stories. T006→T007/T008 (variant before DTO/ABI).
- **US1 (Phase 3)**: after Foundational — the MVP.
- **US2 (Phase 4)**: after Foundational; builds on US1's exact-patch mesher (T011) and element plumbing.
- **US3 (Phase 5)**: after Foundational; builds on US1 (and US2 for under-covering role scenes).
- **Polish (Phase 6)**: after the desired stories.

### User Story Dependencies

- **US1 (P1)**: needs only the exact patch + plumbing — independently testable with a domain-spanning mesh, no projection.
- **US2 (P1)**: extends US1's element with the kriged fit + smooth patch + weld. Independently testable with an under-covering mesh.
- **US3 (P2)**: extends the element to the other roles. Each role independently testable.

### Within Each User Story

- Tests written and FAILING before implementation (constitution oracle).
- Engine mesher before UI plumbing; core before integration.

### Parallel Opportunities

- Setup: T001 ∥ T002.
- Foundational: T004 ∥ T005 ∥ T007 can run in parallel; T003 (spike) runs alongside but GATES the architecture; T006 before T007/T008.
- US1: T009 ∥ T010 (tests); T012 ∥ T013 (protocol ∥ project model). T011 (engine) and T012/T013 (TS) are different files → parallel.
- US3: T022 ∥ T023; T024/T025/T026 touch the same `stack.rs` dispatch → sequential.
- Polish: T028 ∥ T029 ∥ T030 ∥ T031.

---

## Parallel Example: Foundational

```bash
# After T006 (the StackElement variant) compiles, run in parallel:
Task: "T004 OBJ parser in crates/geofield/src/obj.rs"
Task: "T005 winding-normalize + malformed-report in crates/geofield/src/obj.rs"  # if split from T004
Task: "T007 StackElementDto::ImportedSurface in crates/geofield/src/schema.rs"
# T003 (spike) proceeds independently as the architecture gate.
```

## Parallel Example: User Story 1

```bash
# Tests first (must fail):
Task: "T009 native exact/watertight invariant test"
Task: "T010 Playwright import-and-render spec"
# Then the TS plumbing in parallel with the engine mesher:
Task: "T012 protocol.ts importedMesh discriminator"
Task: "T013 project.ts importedSurface element + round-trip"
```

---

## Implementation Strategy

### MVP First (US1)

1. Setup (T001–T002) → Foundational (T003 gate, T004–T008).
2. US1 (T009–T016). **STOP and VALIDATE**: import a domain-spanning `.obj`, see it, use as a contact, watertight + exact.
3. Demo the MVP.

### Incremental Delivery

1. Setup + Foundational → foundation ready (patch-stitch proven).
2. US1 → import + exact contact (MVP).
3. US2 → projection to boundary (the kicker) → the feature's headline value.
4. US3 → full role parity.
5. Polish → parity/seam gates + calibrated tolerances + honest-scope doc updates.

### Risk-First Note

The single real geometry risk is the patch-stitch seam (D3). It is front-loaded as the **Foundational spike (T003)**, gating the architecture before any plumbing is built — if it fails, revisit the plan, not the whole codebase. Everything downstream is conventional reuse (parser, element plumbing, existing role machinery).

---

## Notes

- [P] = different files, no incomplete-task dependency. [Story] label maps to spec.md user stories.
- Each user story is an independently completable, independently testable increment.
- Tests (invariants + Playwright) are the correctness oracle (Principle I) — write them failing first.
- Rebuild `dist/` (`npm run wasm:engine && npm run build`) before any Playwright run (preview serves `dist/`).
- Commit per task or logical group; scoped/typed messages (`feat(volume):`, `feat(authoring):`, `docs(...)`).
- The `docs/07` amendment (T002) is a deliberate Principle-II amendment with a recorded rationale, not a silent doc drift.
