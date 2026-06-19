# Implementation Plan: Enhanced Authoring for the Stack Testbed

**Branch**: `002-enhanced-authoring` (working on `main` per repo git convention) | **Date**: 2026-06-19 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/002-enhanced-authoring/spec.md`

**Design source of record**: [docs/enhanced_authoring_spec.md](../../docs/enhanced_authoring_spec.md) — the build brief this plan implements. Where this plan and that brief disagree, the brief is the document that wins (Constitution Principle II) and this plan is the bug.

## Summary

Turn the `web3/` testbed from a three-scenario viewer into a generic stack authoring tool. The entire change is a **TypeScript/React layer over an unchanged two-wasm core**: introduce an editable client-side **project document** (containers + per-surface point/orientation tables) that compiles, via one pure function, to the existing `SeriesJson[]` the worker already consumes. Authoring edits set a `dirty` flag and re-fit on an explicit **Apply** (with **Cancel** to revert); cheap declared-state edits keep their immediate round-trip. Add a type-on-add surface flow, a dual-table data modal with clipboard paste, live container/within-series reordering (wiring the existing resolver), a stale-state scrim over the 3D view, localStorage autosave + JSON import/export, seed templates from the former scenarios, and client-side OBJ export. **No Rust, kernel, worker, ABI, or schema changes.**

## Technical Context

**Language/Version**: TypeScript 5.5, React 18.3, targeting ES modules (Vite 5.4). No Rust changes this feature.

**Primary Dependencies**: React 18 + react-dom, three.js 0.169 (viewer), `@dnd-kit/core` + `@dnd-kit/sortable` 6/8 (drag, already present and wired into `dragResolver.ts`). No new runtime dependencies anticipated (clipboard paste, localStorage, OBJ serialization, and Blob download are all platform APIs).

**Storage**: Browser `localStorage` for project autosave (debounced ~500 ms, keyed by project name); JSON project file (`<name>.geoproj.json`) as the portable "save file" via Blob download / file input. No server, no database.

**Testing**: Playwright (`@playwright/test` 1.61) end-to-end against the running app — the constitution's UI behavior gate. `tsc -b` + `vite build` are the compile gate. New unit-level pure functions (`toSeriesJson`, `validate`, `dipDirToPole`, OBJ serialization, TSV/CSV parse) are exercised through the Playwright scenarios and, where cheap, directly; the brief's §10 names five new specs.

**Target Platform**: Modern evergreen browser (WASM + ES modules + Clipboard API + Blob/`URL.createObjectURL`). Same target as the existing `web3/` harness.

**Project Type**: In-repo browser testbed (`web3/`), Playwright-gated. Not the shipping product UI, but authoring components stay inside the reusable `HierarchyPanel/` boundary so they lift into the product later.

**Performance Goals**: Interactive authoring — table edits and modal commits are instant (no fit); a re-fit on Apply runs at the existing scenario fit cost (the same `runStack` path, unchanged). The dirty/Apply design exists precisely to make the re-fit cost explicit and batched (edit many, fit once). No new perf target beyond "the existing fit, run on demand."

**Constraints**:
- **No core changes.** `dto.ts`, `store.ts`, `protocol.ts`, `worker.ts`, `kernel-bridge.ts`, and all Rust crates are unchanged. The project model is UI-only and never crosses the wasm boundary.
- The authoring model compiles **down** to the existing `SeriesJson` (youngest-first array) via one seam function; the worker's `add_series`/`add_vein`/`add_intrusion` ABI is untouched.
- DTOs (`dto.ts`) stay hand-mirrored against `crates/geofield/src/schema.rs` (single source of truth) — no `ts-rs` introduced.
- The existing worker hard-terminate-on-new-run cancellation is retained.

**Scale/Scope**: Demo/testbed scale — a single project, a handful of containers, tens to low-hundreds of points per surface (hand- or paste-entered). 6 new files, 5 changed files, 5 new Playwright specs (per brief §9–§10).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (below).*

Derived from `.specify/memory/constitution.md` v1.1.0.

- [x] **I — Ground truth (NON-NEGOTIABLE).** This is UI/behavioral work, so its gate is **automated Playwright functional tests that drive the running app**, not a manual walk-through. The spec's SC-008 makes this explicit; the brief's §10 enumerates five specs (authoring-add, stale-scrim, reorder, export-obj, modal-validation) covering each new behavioral acceptance criterion. `tsc -b` + `vite build` are the compile gate; Playwright is the behavior gate; **both** run before "done." No GemPy reference is introduced (none is reachable from `web3/` TS anyway). Where a Playwright assertion would be flaky (real drag), the test drives the wired callback / asserts on the resolver, consistent with the existing `drag-affordance.spec` — and that substitution is documented in the test, not silent. **If the environment cannot run Playwright this pass, that is a logged, un-closed gap (Principle IV), never a silent "verified."**
- [x] **II — The document wins.** The governing document is [docs/enhanced_authoring_spec.md](../../docs/enhanced_authoring_spec.md) (referenced by `CLAUDE.md` once this plan lands). The one load-bearing formula — dip/dip-direction → pole — is transcribed from the brief's §5.3 into a single documented function and matches it sign-for-sign. The DTO contract's source of truth remains `crates/geofield/src/schema.rs`; this feature touches none of it.
- [x] **III — Derive, don't declare.** The project document declares only irreducible authoring state (containers, per-surface points/orientations, names, the erode/onlap/conformable seam bit, the unbounded bit, chronological order). Everything geological — effective relations, cross-cut sets, validation of the *fitted* result — stays **derived by the core** and read back through `CoreUpdate.derived`, never recomputed in the UI. The one new UI-side computation, author-time `validate(project)`, checks **structural** preconditions the core cannot be asked about because they block the fit from running at all (zero points, missing cross-cut part) — it does not re-encode any core geological rule, and the plan states this boundary explicitly so the two validations don't drift. Container membership is **decided once** at add-time from the chosen type (the type→container mapping is the single rule), not stored as a redundant "container" the user picks.
- [x] **IV — Honest seams & documented punts.** The spec's Out-of-Scope section carries every punt with its seam: faults (per-series fault fields left at no-fault defaults), cross-container surface moves (the move callback stays a **declined action with a user-facing message + workaround**, not a silent noop — FR-016), insert-at-selection (append + reorder), in-modal 3D preview, CSV file import (TSV/CSV *paste* only), OBJ materials (geometry + groups first; `.mtl` optional, logged as a coverage bound). Each is written where a reader sees it.
- [x] **V — WASM-safety & license seam (ARCHITECTURAL).** No change touches the Rust build graph, the wasm artifacts, or `kernel-bridge.ts`. The authoring layer is pure TypeScript above the worker boundary; it cannot pull the kernel into the engine graph or cross the EUPL/BSD seam because it adds no Rust and no wasm-reachable dependency. The two existing `.wasm` artifacts ship unchanged.
- [x] **VI — Smallest correct intervention.** The brief is written as a **delta**: reuse the existing `runStack` fit path driven by a compiled project (don't rebuild the fit), wire the **already-correct** `dragResolver.ts` output instead of replacing drag mechanics, re-expose the existing scenarios as seed templates instead of inventing fixtures, attach a positioned scrim `<div>` over the existing canvas mount instead of restructuring the viewer. New abstractions are limited to what the authoring model genuinely needs (the project document and its compile seam). No package added where a platform API suffices (paste/localStorage/Blob).

**Result: all six gates pass; Complexity Tracking is empty.**

## Project Structure

### Documentation (this feature)

```text
specs/002-enhanced-authoring/
├── plan.md              # This file (/speckit-plan output)
├── research.md          # Phase 0 output — decisions + rationale
├── data-model.md        # Phase 1 output — the project document & its compile mapping
├── quickstart.md        # Phase 1 output — how to run the harness + the Playwright gate
├── contracts/           # Phase 1 output — the authoring↔fit seam + UI callback contracts
│   ├── project-model.md
│   └── ui-callbacks.md
├── checklists/
│   └── requirements.md  # /speckit-specify output (already present)
└── tasks.md             # /speckit-tasks output — NOT created by /speckit-plan
```

### Source Code (repository root)

This feature lives entirely under `web3/`. Tagging follows the constitution's `[EXISTS]`/`[CHANGED]`/`[NEW]` convention.

```text
web3/
├── src/
│   ├── HierarchyPanel/                 # the reusable boundary — keep authoring UI inside it
│   │   ├── project.ts            [NEW]   Project/ProjectElement/InterfaceData types;
│   │   │                                 toSeriesJson(); validate(); dipDirToPole();
│   │   │                                 localStorage load/save; JSON import/export.
│   │   ├── AddSurfaceMenu.tsx     [NEW]   five-type picker popover
│   │   ├── SurfaceDataModal.tsx   [NEW]   dual-table edit-and-apply modal
│   │   ├── SurfaceMenu.tsx        [NEW]   per-surface "…" menu
│   │   ├── tables.tsx            [NEW]   reusable points/orientations table + clipboard paste
│   │   ├── HierarchyPanel.tsx   [CHANGED] singleton flattening, "…" menus, add-interface
│   │   │                                 affordance, surface-row edit entry points
│   │   │                                 (seam/floor/validation/derived rendering unchanged)
│   │   ├── dragResolver.ts      [EXISTS]  unchanged — its output now consumed, not ignored
│   │   ├── dto.ts              [EXISTS]  unchanged (hand-mirror of schema.rs)
│   │   ├── store.ts           [EXISTS]  unchanged
│   │   ├── hierarchy.css      [CHANGED]  styles for new menus/modal/scrim as needed
│   │   └── index.ts            [CHANGED]  re-export the new public components
│   ├── exportObj.ts              [NEW]   OBJ serialization from SurfacePayload[]
│   ├── main.tsx               [CHANGED]  project-centric state (project, dirty, applying,
│   │                                     lastApplied); Apply/Cancel bar; project bar; scrim
│   │                                     wiring; wire reorder callbacks; export action
│   ├── scenarios.ts           [CHANGED]  re-expose the 3 scenarios as seed templates
│   │                                     returning a Project; add a "blank" template
│   ├── Viewer.tsx             [CHANGED]  accept stale?: boolean; render scrim overlay
│   ├── protocol.ts            [EXISTS]  unchanged
│   ├── worker.ts              [EXISTS]  unchanged
│   └── kernel-bridge.ts       [EXISTS]  unchanged
└── e2e/
    ├── authoring-add.spec.ts     [NEW]
    ├── stale-scrim.spec.ts       [NEW]
    ├── reorder.spec.ts           [NEW]
    ├── export-obj.spec.ts        [NEW]
    ├── modal-validation.spec.ts  [NEW]
    ├── smoke.spec.ts          [EXISTS]  unchanged (kept)
    ├── round-trip.spec.ts     [EXISTS]  unchanged (kept; reorder.spec mirrors its pattern)
    ├── cross-cut.spec.ts      [EXISTS]  unchanged (kept)
    ├── drag-affordance.spec.ts[EXISTS]  unchanged (kept; reorder.spec mirrors its no-real-drag pattern)
    └── viewer-controls.spec.ts[EXISTS]  unchanged (kept)
```

**Structure Decision**: Single in-repo browser package (`web3/`). All new authoring code lands under `web3/src/HierarchyPanel/` except `exportObj.ts` (view-level, sits beside `Viewer.tsx`). The `HierarchyPanel/` boundary is preserved so the authoring UI is liftable into the product later. No new top-level project, crate, or package is created — the feature is a delta on the existing testbed.

## Complexity Tracking

> No constitution gate is violated; this table is intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none)    | —          | —                                    |

## Phase 0 — Research

See [research.md](./research.md). All Technical Context items were resolvable from the spec, the design brief, and the existing `web3/` baseline; no item is left as NEEDS CLARIFICATION.

## Phase 1 — Design & Contracts

- [data-model.md](./data-model.md) — the `Project` document, its elements and surface data, validation rules, and the `toSeriesJson` compile mapping (the single seam to the unchanged fit path).
- [contracts/project-model.md](./contracts/project-model.md) — the authoring↔fit seam: the compile contract and the dip/dip-direction → pole conversion.
- [contracts/ui-callbacks.md](./contracts/ui-callbacks.md) — the `HierarchyPanel` callback surface (which exist, which change, which are newly wired) and the `Viewer` `stale` prop.
- [quickstart.md](./quickstart.md) — how to build the wasm artifacts, run the dev server, and run the Playwright behavior gate; the end-to-end validation scenarios.

Agent context updated: `CLAUDE.md`'s SPECKIT block now points at this plan.
