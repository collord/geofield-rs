# Implementation Plan: Cross-Sections & Digitized Surfaces

**Branch**: `main` (no feature branch — per user convention) | **Date**: 2026-07-18 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/005-cross-section-digitizing/spec.md`; source fragment `web3/SPEC_cross_sections.md` (§10 sections, §11 digitizing)

## Summary

Add vertical **cross-section views** (a plane from a two-click plan-view baseline; camera locked orthographic face-on; traces of applied surfaces as backdrop; serial parallel sets with stepping) and **polyline digitizing on sections** (ordered vertices become contact points; the drawn direction yields orientation poles via the section frame; per-surface flip reuses the existing XOR) to the web3 harness. **Everything is TypeScript/UI-side**: traces and solid-interval fills are computed on the main thread from the mesh payloads the worker already returns; digitized lines compile through the existing `flattenInterfaces → SeriesJson → add_series` path with one additive extension (a direct-pole source alongside dip/dipDir). No engine, worker-ABI, or kernel change. Sections are view-state (never dirty, never re-fit); digitized lines are geometry (dirty, deferred to Apply). The behavior gate is a set of new Playwright e2e specs; `web3/SPEC.md` is amended to absorb the fragment (with one sign fix, see research.md R3).

## Technical Context

**Language/Version**: TypeScript ~5.5 (React 18, three ^0.169), running against the existing Rust engine via prebuilt wasm — **no Rust changes planned**

**Primary Dependencies**: `three` (adds `OrthographicCamera` usage; `Raycaster`+`Plane` picking and `ShapeGeometry` fill — all already shipped), `react`, existing solver worker (`src/worker.ts`, terminate-per-fit) and persistent Manifold worker (untouched)

**Storage**: localStorage autosave + `.geoproj.json` export, version 1 with **additive optional fields** (`Project.sections`, `Project.view.section`, `InterfaceData.digitized`) — old exports load unchanged

**Testing**: Playwright e2e against the production build (`npm run build && npm run test:e2e`) is the behavior gate; `tsc -b && vite build` is the compile gate. No unit-test framework exists in web3 and none is added; pure-geometry helpers are exercised through `window.__*` e2e hooks (documented bound, see Constitution I note)

**Target Platform**: Browser (Chromium in CI/e2e), Vite production build

**Project Type**: Web app (the `web3/` authoring harness of a Rust+wasm workspace)

**Performance Goals**: Section open/step recomputes traces UI-side in interactive time (target <50 ms for typical payloads, ~1e5 triangles across surfaces); digitizing interactions at frame rate; section create/move/delete/step trigger **zero** engine calls

**Constraints**: No wasm/ABI change; section ops must not set `dirty` (clone the `setRelation`/`commitLithology` immediate-path, `main.tsx:365,390`); no second Manifold instantiation (single-realm constraint, `manifoldVolumes.ts:6-10`) — solid-interval fill therefore does NOT use Manifold; state setters are async → explicit-override convention for anything passed to `runStack` (`main.tsx:316-320`)

**Scale/Scope**: Models with ~10s of elements, mesh payloads ~1e4–1e5 triangles; serial sets of ~5–50 sections; digitized lines of ~5–50 vertices each

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Derived from `.specify/memory/constitution.md` v1.1.0.

- [x] **I — Ground truth (NON-NEGOTIABLE).** All acceptance is UI/behavioral → the gate is **new Playwright e2e specs** (enumerated in [quickstart.md](quickstart.md)): section create/read, younging affordance + flip invariance, serial digitize→Apply→loft round-trip (trace-lands-on-line assertion via a `window.__sectionTraces` hook), section-persistence round-trip, no-dirty/no-refit guard, and the §11.4 validation matrix. `tsc -b && vite build` is the compile gate. The pure slicer/derivation helpers have no native unit harness in web3 — they are pinned through e2e hooks; that is a **logged coverage bound** (research.md R9), not a silent one. No kernel/field claims are made, so no `validation/` harness is touched; no GemPy reference is introduced.
- [x] **II — The document wins.** Governing doc: `web3/SPEC.md`. This feature **merges the proposed fragment `web3/SPEC_cross_sections.md` into `SPEC.md` as §10/§11** as part of implementation, amending deliberately: the fragment's §11.2 pole formula `m = t × s` is corrected to `m = ŝ × t̂` to match its own normative "youngs to the left" prose (worked derivation in [contracts/younging-derivation.md](contracts/younging-derivation.md)). The derivation contract is cited from the code at the derivation site.
- [x] **III — Derive, don't declare.** Declared state is irreducible: a section **set** (baseline, spacing, count, offsets — planes are derived, never stored per-plane); a digitized **line** (ordered world vertices + its plane frame snapshot). Everything else is derived at compile/render time: section frames (u, w, s), traces, filled intervals, tangents, poles (via the same single flip-XOR site `flattenInterfaces`, `project.ts:253` — extended, not duplicated). Digitized vertices are NOT copied into `points[]`; the table view unions them at render time (one dataset, two views — FR-012).
- [x] **IV — Honest seams & documented punts.** Non-goals restated from spec (no tilted/fence sections, no free 3D drawing, no auto-trace, no mesh editing, no engine-side loft). Logged bounds: solid-mode filled intervals require watertight unit solids (Manifold path, default-on) — with the legacy manifold-off diagnostic toggle the section falls back to unfilled boundary segments, noted in-UI comment + spec note (research.md R2); e2e hooks stand in for unit tests on pure helpers (R9); per-member ops on serial sets (delete one plane of a set) are out — you edit or delete the set.
- [x] **V — WASM-safety & license seam (ARCHITECTURAL).** No change to any Rust crate, wasm build graph, or worker ABI. All new code is BSD-side TypeScript in `web3/src`. The kernel/engine seam is not touched; nothing new crosses it.
- [x] **VI — Smallest correct intervention.** The loft **is** the existing co-kriging fit consuming digitized points+poles — no loft code is written. Traces/fills are computed from meshes the worker already returns — no new worker round-trip. The only data-model change is additive (`digitized?` on `InterfaceData`, `sections?` on `Project`). The one place we extend rather than reuse: a direct-pole path in `flattenInterfaces`, because dip∈[0,90]+per-surface flip cannot represent per-vertex poles in the lower hemisphere (overturned/steep digitized segments) — back-conversion to dip/dipDir was evaluated and rejected (research.md R4).

**Post-design re-check (after Phase 1)**: PASS — no violations introduced by the design; Complexity Tracking left empty.

## Project Structure

### Documentation (this feature)

```text
specs/005-cross-section-digitizing/
├── plan.md              # This file
├── research.md          # Phase 0 — decisions R1–R10
├── data-model.md        # Phase 1 — SectionSet, DigitizedLine, derivations, validation
├── quickstart.md        # Phase 1 — build/run/e2e gates + manual walk
├── contracts/
│   ├── project-json.md         # Additive .geoproj.json schema (sections, digitized, view)
│   └── younging-derivation.md  # Section frame + pole math, worked example, flip XOR
└── tasks.md             # Phase 2 (/speckit-tasks — not created here)
```

### Source Code (repository root)

```text
web3/
├── SPEC.md                          # [EXTENDS] absorb fragment as §10/§11 (+sign fix)
├── SPEC_cross_sections.md           # [ABSORBED] becomes a pointer or is deleted after merge
├── src/
│   ├── section/                     # [NEW] pure, viewer-agnostic geometry (no React/three deps
│   │   ├── frame.ts                 #   beyond types): SectionFrame from baseline (u, w=+Z, s=u×w),
│   │   │                            #   serial-plane enumeration, world↔in-plane mapping
│   │   ├── slice.ts                 #   plane∩mesh → polyline segments (contact traces);
│   │   │                            #   segment→loop assembly + even-odd fill triangulation input
│   │   │                            #   (solid intervals); slab-tolerance point picking
│   │   └── digitizeDerive.ts        #   tangents per segment, pole m = ŝ×t̂, younging-consistency
│   │                                #   check (shared by overlay, compile, and validation)
│   ├── HierarchyPanel/
│   │   ├── project.ts               # [EXTENDS] Project.sections, view.section, InterfaceData.digitized;
│   │   │                            #   flattenInterfaces direct-pole path (same flip XOR);
│   │   │                            #   validate() digitizing rules; save/load/export additive fields
│   │   ├── projectOps.ts            # [EXTENDS] section-set CRUD (non-dirty path) + digitized-line ops
│   │   │                            #   (commit path): add/move/delete vertex, re-digitize, flip
│   │   ├── SurfaceDataModal.tsx     # [EXTENDS] points table unions digitized vertices (read/write,
│   │   │                            #   edits projected back onto the line's plane)
│   │   └── SectionsPanel.tsx        # [NEW] section list, create (baseline nonce → Viewer), serial
│   │                                #   dialog (spacing/count/offsets), open/step/delete, 3D-plane toggle
│   ├── Viewer.tsx                   # [EXTENDS] section-view mode: OrthographicCamera face-on (u right,
│   │                                #   Z up), pan/zoom-only controls; baseline two-click picker (clone
│   │                                #   of drawBounds effect, Viewer.tsx:432); trace/fill/backdrop/
│   │                                #   affordance render groups; translucent plane quad in 3D;
│   │                                #   digitizing pointer tool (click/extend/drag/delete on plane)
│   └── main.tsx                     # [EXTENDS] section state + non-dirty updaters; step-with-tool;
│                                    #   window.__ hooks (__sections, __sectionTraces, __digitizeLine);
│                                    #   stale-scrim + render-mode plumbed into section view
└── e2e/
    ├── section-view.spec.ts         # [NEW] create/open/read + no-dirty/no-refit guard
    ├── section-digitize.spec.ts     # [NEW] younging affordance, flip invariance, table union
    ├── section-loft.spec.ts         # [NEW] serial digitize → Apply → trace-lands-on-line
    ├── section-persist.spec.ts      # [NEW] export/import round-trip incl. active section
    └── section-validation.spec.ts   # [NEW] §11.4 hard/soft matrix
```

**Structure Decision**: All work lands in `web3/` (the BSD-side browser harness). New pure geometry is isolated in `web3/src/section/` so the slicer/derivation math has a single home the Viewer, compile path, and validation all import — no rule lives twice (Principle III). No Rust crate is touched.

## Complexity Tracking

> No constitution violations to justify — table intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| — | — | — |
