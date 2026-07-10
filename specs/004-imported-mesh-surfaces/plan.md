# Implementation Plan: Imported Mesh Surfaces

**Branch**: `004-imported-mesh-surfaces` | **Date**: 2026-07-09 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/004-imported-mesh-surfaces/spec.md`

## Summary

Let a modeler import a triangle mesh (`.obj`) and use it as **any** surface a fitted surface can be — stratigraphic contact, erosion, vein/intrusion wall, topography — with the imported geometry honored **exactly inside the mesh's axis-aligned bounds** and a **smooth field-fitted extension** carried out to the domain box beyond them, joined by a **watertight seam**.

Technical approach (resolved in [research.md](./research.md)): the smooth far-field is a co-kriging fit through the mesh's vertices (as surface points) + normals (as orientations), reusing the existing `Model::fit` across the existing EUPL-kernel wasm boundary with the existing point-`decimate` coreset to keep the N³ solve bounded. The exact-inside surface is the already-built `MeshSdf` (a `ScalarField` whose zero set is the mesh). The two are **stitched geometrically** — exact patch clipped to the AABB, smooth patch clipped to [outside AABB → box], welded at the shared AABB seam by the already-built `mesh_clip::clip` corefiner (seam shared by index → watertight, **no field-blend, so no spurious phantom sheet**). A ~50-line byte-oriented OBJ parser and a new `StackElement::ImportedSurface` variant thread through the conventional element-plumbing chain.

**Refinement of the spec's decided approach (flag for review):** the spec's Assumptions recorded "fit a field, exact-**blend** near the mesh." Research (below) shows the robust realization of that same intent is a geometric **stitch** (corefiner weld), not a scalar-field blend — the codebase already declined a fabricated blended far-field for imports (`docs/07`), and a field-blend reintroduces the spurious-lobe risk. Patch-stitch delivers the identical user-facing outcome (exact inside, smooth kriged outside, continuous seam) with strictly less risk. This is an implementation-level decision within the "fit a field" family the user chose; it is called out here and in research.md, and it obliges a **deliberate amendment to `docs/07`** (Principle II — see Constitution Check II).

## Technical Context

**Language/Version**: Rust (edition per workspace) for the `geofield` engine + `geofield-kernel`; TypeScript (React) for `web3` UI.

**Primary Dependencies**: existing `geofield` engine (`octree` dual-contour, `mesh_clip` corefiner, `MeshSdf`/`TriMesh`, `below_solid`/`band_solid`), `geofield-kernel` co-kriging fit (`Model::fit`, `DecimateConfig`) reached across the wasm module boundary (`remote::fit`), `manifold-3d` (main-thread volume booleans), `three.js` viewer. **New:** a hand-rolled `v/vn/f` OBJ parser (~50 lines, BSD, in the `geofield` engine — no new crate).

**Storage**: project JSON (`.geoproj`) — extended with an imported-surface element (mesh geometry embedded or referenced; round-trips per FR-012).

**Testing**: `cargo test -p geofield` (native + geometric-invariant tests: watertight, consistent winding, manifold edges, flush-to-extent, seam continuity); native↔wasm parity assertions (design-invariant, per the no-wasm-cargo-test-harness constraint); Playwright e2e in `web3/e2e` (import affordance, element appears, role assignment, render); the scratch permute-harness pattern for partition/watertightness sweeps.

**Target Platform**: `wasm32-unknown-unknown` (browser worker + main thread) and native (x86/arm) for tests.

**Project Type**: geological meshing engine (Rust, dual-target) + browser authoring UI (existing `web3` harness).

**Performance Goals**: import + fit + mesh a ~100k-triangle mesh within an interactive budget (SC-010); the co-kriging solve is bounded by `DecimateConfig.point_budget` (hard cap), not by raw vertex count.

**Constraints**: native↔wasm `f64` round-off parity (Principle I/V); geometric invariants as the correctness oracle (no GemPy reference — Principle I); no EUPL/GemPy code in the BSD `geofield`/`geofield-types` crates and no native-only dep on a wasm-reachable path (Principle V); the OBJ parser consumes **bytes** (browser reads the file), never a filesystem path.

**Scale/Scope**: one imported surface per element; multiple imported surfaces per project; single-block domain (matches the current mesher).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Derived from `.specify/memory/constitution.md` v1.1.0.

- [x] **I — Ground truth (NON-NEGOTIABLE).** The imported-surface engine paths are `geofield` (mesher) work → validated by **geometric invariants** (watertight/zero-holes SC-002/003, consistent winding, manifold edges, flush-to-extent, seam continuity SC-005) + **native↔wasm parity** (SC-007), introducing **no GemPy reference** (the co-kriging fit it reuses already answers to the kernel's GemPy gate inside the kernel crate; the mesh geometry answers only to geometric invariants). **UI acceptance** (import → element → role → render) is gated by a **Playwright** functional test (SC-009). New invariant tests (seam-watertight, exact-inside-tolerance) are in scope, not follow-ups. → **PASS.**
- [x] **II — The document wins.** Governing docs: `docs/06_surface_distance_and_boundary_trait.md` and `docs/07_imported_mesh_field_proxy.md` (the imported-mesh design of record). **This feature deliberately amends `docs/07`**: that doc *declined* a fabricated smooth far-field for imports (scoped to clip-only boundaries with no units, where an invented field is pure downside). This feature is **full-parity** (units/lithology), where a smooth far-field beyond the mesh bounds is the *desired* behavior — and confining it to *beyond the user's own geometry* answers the original objection (there the user supplied nothing, so a smooth extrapolation is the honest choice, identical to how fitted surfaces already extrapolate-then-clip). The amendment is made deliberately, with this rationale, as a task — not as a side effect. → **PASS (with a recorded, deliberate doc amendment).**
- [x] **III — Derive, don't declare.** The `ImportedSurface` element declares only the irreducible state: the mesh geometry, the role, and (where the role is stratigraphic) the one erode/onlap/conformable relation bit — reusing the existing relation model, not a new flag. The coverage bounds (mesh AABB), the co-kriged field, the exact/smooth patches, and the stitched surface are **all derived** by the core and never stored or re-encoded in the UI. → **PASS.**
- [x] **IV — Honest seams & documented punts.** Non-goals stated in the spec (formats beyond `.obj`, mesh repair beyond winding-normalization, coordinate transform on import, concave-boundary coverage, imported faults). The coarse far-field (FR-006) and the AABB-vs-tight coverage bound are logged, not silent. Any decimation/point-budget cap is logged where a reader sees it. → **PASS.**
- [x] **V — WASM-safety & license seam (ARCHITECTURAL).** CONFIRMED by research: reusing `Model::fit` threads the **existing** `add_series → fit_model → remote::fit → gf_fit` wasm-import boundary (`remote.rs:30-57`); the EUPL kernel stays out of `geofield.wasm` exactly as today. The OBJ parser is BSD, hand-rolled **inside the `geofield` engine**, consumes bytes (no `std::fs`, no native-only dep) → wasm-safe by construction. `MeshSdf`/`mesh_clip` are already BSD engine code. **No new seam crossing.** → **PASS.**
- [x] **VI — Smallest correct intervention.** Reuses `Model::fit` + `DecimateConfig` (fit + decimation — no rebuild), `MeshSdf` (exact field), `mesh_clip::clip` (seam weld — the corefiner was built for exactly this, and its no-op `refine` hook is the sanctioned seam-continuity knob), `below_solid`/`octree_dual_contour_scene` (patch contouring), and the existing element-plumbing pattern. **New code is minimal:** a ~50-line parser, the patch-stitch caller, one `StackElement` variant, and the UI affordance. The rejected field-blend would have been *more* code and *more* risk. → **PASS.**

**Result: all gates pass.** One deliberate doc amendment (II) is recorded and scheduled as a task; no unjustified violations → Complexity Tracking is empty.

## Project Structure

### Documentation (this feature)

```text
specs/004-imported-mesh-surfaces/
├── plan.md              # This file
├── research.md          # Phase 0 — decisions (fit reuse, decimation, patch-stitch, parser)
├── data-model.md        # Phase 1 — ImportedSurface entity + derived representation
├── quickstart.md        # Phase 1 — runnable validation scenarios
├── contracts/           # Phase 1 — ABI + DTO + project-element contracts
│   ├── abi.md           #   WasmStack::add_imported_surface signature
│   ├── protocol.md      #   SeriesJson `importedMesh` discriminator + project element
│   └── invariants.md    #   geometric acceptance contracts (watertight/exact/seam)
└── tasks.md             # Phase 2 (/speckit-tasks — NOT created here)
```

### Source Code (repository root) — touch-points, not new trees

```text
crates/geofield/src/
├── obj.rs               # NEW — ~50-line byte OBJ parser (v/vn/f) → (verts, faces, normals?)
├── stack.rs             # EXTEND — StackElement::ImportedSurface variant + patch-stitch mesher
│                        #   (reuses below_solid / octree_dual_contour_scene / mesh_clip::clip)
├── octree.rs            # REUSE — MeshSdf, TriMesh (surface_distance, edge_crossing_count)
├── mesh_clip.rs         # REUSE — clip() weld; possibly wire the refine hook (currently no-op)
├── schema.rs            # EXTEND — StackElementDto::ImportedSurface (DTO round-trip)
└── wasm.rs              # EXTEND — WasmStack::add_imported_surface (bytes → parse → fit → element)

crates/geofield-kernel/  # UNTOUCHED — Model::fit + DecimateConfig reused across the wasm boundary

web3/src/
├── HierarchyPanel/
│   ├── project.ts       # EXTEND — ProjectElement { kind: "importedSurface" } variant
│   └── project.ts:toSeriesJson  # EXTEND — compile importedSurface → SeriesJson discriminator
├── protocol.ts          # EXTEND — SeriesJson.importedMesh?: {...} discriminator + DTO
├── worker.ts            # EXTEND — buildStack branch → stack.add_imported_surface(...)
└── main.tsx / HierarchyPanel  # EXTEND — a distinct ".obj import" affordance (not the JSON Import)

docs/07_imported_mesh_field_proxy.md  # AMEND — deliberate: full-parity adopts a smooth far-field
```

**Structure Decision**: no new top-level projects or crates. The feature is a delta on the existing dual-target `geofield` engine + `web3` UI (Principle VI). The only new file is `crates/geofield/src/obj.rs` (the parser); everything else extends existing modules along the mapped element-plumbing chain.

## Implementation Status (2026-07-09/10)

Landed on branch `manifold-volumes` (see tasks.md for per-task marks). Both P1 stories + US3 role parity are built and verified; the two geometry risks were spiked green before any plumbing.

- **Foundational**: OBJ parser (`obj.rs`, byte/BSD/wasm-safe, 9 tests), ingress hygiene (winding-normalize + health report), `StackElement::ImportedSurface` + DTO, `add_imported_surface` ABI. Both spikes green (`T003` seam_gap=0; mesh-direct `max_dz=0.73`).
- **US1** (exact contact): `MeshSignedField` exact patch; native invariant test (watertight `open=0`, on-the-mesh `max|dz|<2`); Playwright import→render; wasm ABI + full TS plumbing + import UI. ✅
- **US2** (projection to boundary): `StitchedField` (exact inside footprint, smooth kriged outside, one continuous contour — `refine` hook proved unnecessary); native test: watertight, reaches box faces, exact inside. ✅
- **US3** (role parity): erosion + topography clip older elements via `clip_maskers` (native per-role tests); role selector UI + Playwright. ✅
- **Polish**: Constitution V seam verified (kernel cfg-gated out of `geofield.wasm`; parser BSD, no `std::fs`; fit reuses `remote::fit`); imported paths are deterministic f64 (parity pinned natively + Playwright cross-target); tolerances pinned in the tests (`max|dz|<2`, seam/volume checks); `docs/07` amended.

### Coverage bounds (Principle IV — logged, not silent)

- **`.obj` only**; STL/PLY/glTF are a parser seam (not built).
- **VeinWall role is a documented seam** — a single imported sheet is not a band; only contact/erosion/topography roles are wired. The UI role selector omits veinWall.
- **Coverage = the mesh's xy footprint (axis-aligned)** — a tighter (convex-hull / concave) exact region is a refinement seam.
- **Mesh repair not done** — holes/non-manifold/self-intersection are detected + reported (`MeshHealth`), not healed.
- **No coordinate transform on import** — meshes land as authored.
- **Panel rendering of an imported element is minimal** — it has no editable `InterfaceData` rows (its geometry is the mesh); it shows name + role selector + derived units.
- **A dedicated non-convex-import invariant test** (T030) is a minor follow-up; watertightness/exactness are covered by the existing US1/US2 edge-incidence assertions.

## Complexity Tracking

> No Constitution gate is violated; this table is intentionally empty. The one
> deliberate doc amendment (`docs/07`, Principle II) is a sanctioned amendment
> with a recorded rationale, not a violation, and is scheduled as a task.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
