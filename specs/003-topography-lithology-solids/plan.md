# Implementation Plan: Topography, Lithology Catalog & Solid Volumes

**Branch**: `003-topography-lithology-solids` (working on `main` per repo git convention) | **Date**: 2026-06-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `specs/003-topography-lithology-solids/spec.md`

**Design source of record**: [docs/topography_lithology_solids_spec.md](../../docs/topography_lithology_solids_spec.md) — the build brief this plan implements. Where this plan and that brief disagree, the brief is the document that wins (Constitution Principle II) and this plan is the bug — **except** where this plan records a *correction* to the brief's model of the existing code (R2, R4 in [research.md](./research.md)); those corrections are findings from reading the baseline and supersede the brief's incidental description of how the code works, not its design intent.

## Summary

Promote the surface model to a **named-volume model**. Unlike the prior feature (002, a pure TS layer over an unchanged core), this feature is **genuinely cross-cutting: Rust + ABI + worker + UI**. Four moves:

1. **Topography** — a new `StackElement::Topography` variant (singleton, pinned youngest, reusing the erode `Masker::Halfspace` clip), with a matching `add_topography` ABI entry. It caps every older element to the ground; above it is Air.
2. **Universal fill-upward** — make `unbounded_above` inert by **deriving keep-vs-drop of a series' top unit from whether a younger element caps it** (net-new core logic — see the R2 correction), keeping the schema/ABI slot accepted-but-ignored and hiding its UI control.
3. **Units as the unit of presentation** — the core exposes a per-element ordered unit list on `DerivedView`; the UI renders an alternating unit/contact ladder and attaches a **lithology catalog** (Air + Unknown fixed, inferred from bodies, user-added) with a many-to-one `(element, unit) → lithology` assignment, serialized into the project JSON.
4. **Solids by lithology** — reuse the capped-Scene mesher with topography as a universal top masker, skip Air, tag each solid by its unit coordinate, and wire the existing `RenderMode` to a Surfaces⇄Solids toggle that re-meshes without re-fitting; the viewer colours solids by lithology and OBJ export groups by lithology.

The fit is untouched; everything new is either core *derivation* (not new declared state) or UI *identity metadata*.

## Technical Context

**Language/Version**: Rust (engine `crates/geofield/`, edition per `Cargo.toml`) building for both native and `wasm32-unknown-unknown`; TypeScript 5.5 / React 18.3 / Vite 5.4 in `web3/`. The EUPL kernel (`geofield-kernel`) is **not touched** — this is all `geofield` (BSD engine) + `web3` UI.

**Primary Dependencies**: `geofield` engine (stack/octree/mesh/schema/wasm), the two existing wasm artifacts (`geofield_bg.wasm` engine, `geofield_kernel.wasm`), React + three.js 0.169 viewer, `@dnd-kit` (already wired in `dragResolver.ts`). **No new runtime dependencies** anticipated (catalog is plain TS; colours are hex strings; serialization is JSON; toggle is a platform-level state change).

**Storage**: Browser project JSON (`<name>.geoproj.json`) via Blob download / file input, plus the existing debounced localStorage autosave — both extended to carry the `LithologyCatalog` and assignments. No server, no database, no separate catalog store.

**Testing**: Two gates, per Constitution Principle I.
- **Engine**: `cargo test` (native) for unit derivation + the inert-flag test; **mesh invariant tests** (no holes / manifold / consistent winding) for the multi-clip watertightness case; **native↔wasm parity** on the new topography-capped evaluation and unit enumeration.
- **UI**: **Playwright** (`@playwright/test`, [web3/e2e/](../../web3/e2e) already present) driving the running app for every behavioural acceptance criterion; `tsc -b` + `vite build` as the compile gate.

**Target Platform**: Native (engine tests/parity) + `wasm32-unknown-unknown` (shipped engine) + modern evergreen browser (web3 harness). Same dual target the engine already meets and verifies.

**Project Type**: In-repo geological engine (`crates/geofield/`) + browser testbed (`web3/`). Cross-cutting Rust + TS feature.

**Performance Goals**: The fit is unchanged (no kernel change). Mesh extraction in solid mode is the existing capped-Scene cost per `(element, unit)` cell; the Surfaces⇄Solids toggle re-runs mesh collection only (no re-fit) — interactive switch on the already-fitted stack. No new perf target beyond "existing fit + existing per-unit solid mesh, re-collected on toggle."

**Constraints**:
- **No kernel change, no EUPL/BSD seam crossing, no GemPy reference.** All Rust work is in the BSD `geofield` crate; the two wasm artifacts stay separate (Principle V).
- **Native↔wasm parity preserved** on the new evaluation/enumeration (Principle V gate).
- **`unbounded_above` is made inert, not removed** — schema/ABI slot stays accepted-but-ignored; existing serialized scenarios and the Rust test suite stay green (staged deprecation).
- **No mesher re-architecture** — reuse `mesh_one_block_unit_solid` / `octree_dual_contour_scene` with `cap = true` verbatim; topography is one more masker.
- **`dto.ts` stays a hand-mirror** of `crates/geofield/src/schema.rs` (single source of truth) — the new `units` field is added on both sides in lockstep, no `ts-rs`.
- Lithology stays **entirely UI-side**; the core exposes only unit *coordinates*.

**Scale/Scope**: Demo/testbed scale — one project, a handful of elements, tens–hundreds of points per surface, a small catalog (single-digit to low-tens of lithologies). Touches ~5 Rust files (`stack.rs`, `schema.rs`, `wasm.rs`, and the mesh/octree masker assembly) and ~8 web3 files (`project.ts`, `projectOps.ts`, `dto.ts`, `protocol.ts`, `worker.ts`, `HierarchyPanel.tsx`, `AddSurfaceMenu.tsx`, `Viewer.tsx`, `exportObj.ts`, plus a new `LithologyCatalog.tsx`), plus new Rust tests and new Playwright specs.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design (below).*

Derived from `.specify/memory/constitution.md` v1.1.0.

- [x] **I — Ground truth (NON-NEGOTIABLE).** Both halves have a named gate. **Engine**: topography clip, the clip-derived unit keep/drop, the inert-flag behaviour, and watertight solids are backed by `cargo test` + **mesh invariant tests** (crack-free / manifold / winding) + **native↔wasm parity** — *not* "it looks right," and with **no GemPy reference** (none is reachable from the mesher and none is added). The self-consistency risk (two mutually-compensating bugs) is guarded by the parity gate + the invariant test, and the inert-flag test is an *oracle* (it pins the new universal behaviour), labelled as such. **UI**: every behavioural criterion (topography add+cap, unit ladder, catalog CRUD + JSON round-trip, solids toggle + lithology colour, OBJ-by-lithology) is backed by a **Playwright** spec driving the running app; `tsc -b` + `vite build` are the compile gate. If the environment cannot run a gate this pass, that is a **logged, un-closed gap** (Principle IV), never a silent "verified."
- [x] **II — The document wins.** Governing doc is [docs/topography_lithology_solids_spec.md](../../docs/topography_lithology_solids_spec.md) (CLAUDE.md's SPECKIT block repoints to this plan). The one place this plan *corrects* the brief — the brief's claim that the inert-flag change "falls out of the clip result" — is a finding about the **existing code** (`present_unit_count` reads the flag with no clip-awareness; see R2), not a disagreement with the brief's *design intent*; it is recorded in research.md and surfaced to the user, and the brief's design (derive from the cap) stands. No kernel-math doc (`KERNEL_MATH.md`) is touched; the kernel is untouched. The engine's stack/mesh contracts are documented in the spec/as-built, which this work keeps truthful.
- [x] **III — Derive, don't declare.** This is the feature's spine. The **unit count is never declared** — the core derives it (incl. clipping) and exposes it via `DerivedView.units`; the UI must *not* recompute N+1 (the conformable count before clipping would drift from the clipped truth). The **keep/drop of a top unit** moves from a *declared* flag (`unbounded_above`) to a *derived* fact (is a younger element capping here) — removing a declared bit, the right direction. **Lithology is UI-side identity metadata**, not core state: the core knows only `(element, unit)` coordinates; names/colours/assignment live once in the project document and never cross the wasm boundary. The one risk to watch (and the reviewer's focus): the new clip-derived keep/drop logic must live **once, in Rust**, and the UI must read the unit list, never re-derive it. Topography being a *variant* (not a flag) is the Principle-III-correct modelling of "exactly one, youngest, air-above."
- [x] **IV — Honest seams & documented punts.** The spec's Out-of-Scope section carries every punt with its seam: faults (no fault element), **hard removal of the `unbounded_above` slot** (inert now, slot removed later — the seam is the documented-deprecated slot), drillhole-label inheritance (units start Unknown; seam is the catalog's "inferred" provenance), surface-as-entity refactor (seam: retiring the flag removes one of two bits it must reconcile), automatic correlation (intentionally manual many-to-one), persistent storage (project-JSON only). The re-mesh-without-refit fallback ("treat as Apply if a worker constraint forces a refit") is logged as a bounded coverage note, not a silent behaviour.
- [x] **V — WASM-safety & license seam (ARCHITECTURAL).** All Rust work is in the **BSD `geofield` crate**; the **EUPL `geofield-kernel` is untouched**. No change pulls the kernel into the engine's wasm graph or embeds EUPL/GemPy code in a BSD crate. The new evaluation (topography clip) and enumeration (`DerivedView.units`) are in wasm-reachable engine code and so carry the **native↔wasm parity** obligation explicitly (a gate, not an assumption). No native-only dependency is added to a wasm-reachable path. The two `.wasm` artifacts stay separate.
- [x] **VI — Smallest correct intervention.** Reuse is the through-line: topography is a **fourth variant reusing the erode masker**, not a new clip mechanism; solids reuse `mesh_one_block_unit_solid` / `octree_dual_contour_scene` **verbatim** with one extra masker; the toggle wires the **already-present** `RenderMode`; the catalog is **plain TS in the existing project document**, serialized via the existing JSON round-trip; lithology tagging reuses the **existing per-payload index** plumbing. The one genuinely net-new core logic (clip-derived unit keep/drop, R2) is *unavoidable* — it's the smallest change that makes the flag inert correctly, and it *removes* declared state rather than adding it. No new crate, package, or abstraction beyond the `LithologyCatalog` the feature genuinely needs.

**Result: all six gates pass; Complexity Tracking is empty.** The cross-cutting Rust scope is larger than 002, but it is the *minimum* the spec's named-volume goal requires (a topography surface and a clip-derived unit count cannot be done UI-side), and every piece reuses an existing mechanism.

## Project Structure

### Documentation (this feature)

```text
specs/003-topography-lithology-solids/
├── plan.md              # This file (/speckit-plan output)
├── research.md          # Phase 0 output — decisions, rationale, the R2/R4 corrections
├── data-model.md        # Phase 1 output — variant, DerivedView.units, catalog & assignment
├── quickstart.md        # Phase 1 output — build wasm, run harness, run both gates
├── contracts/           # Phase 1 output
│   ├── engine-abi.md         # add_topography, DerivedView.units, inert flag, solid tagging
│   └── catalog-and-render.md # LithologyCatalog model, assignment key, render-mode toggle, colorBy
├── checklists/
│   └── requirements.md  # /speckit-specify output (already present)
└── tasks.md             # /speckit-tasks output — NOT created by /speckit-plan
```

### Source Code (repository root)

Tagging follows the constitution's `[EXISTS]` / `[CHANGED]` / `[NEW]` convention. Citations are to the baseline as mapped in Phase 0.

```text
crates/geofield/src/                     # BSD engine — the only Rust touched (kernel untouched)
├── stack.rs            [CHANGED]  StackElement::Topography variant (add to enum @250);
│                                  global_unit (@388) & present_unit_count (@711) derive
│                                  top-unit keep/drop from younger-element cap, not the flag
│                                  (the R2 net-new logic); topography joins clip_maskers (@415)
│                                  as a universal top masker; mesh_one_block_unit_solid (@736)
│                                  skips the Air unit. Singleton/index-0 placement enforced here.
├── schema.rs           [CHANGED]  DerivedView (@110) gains `units: Vec<(String, Vec<UnitInfo>)>`;
│                                  StackElementDto (@56) gains a Topography case; unbounded_above
│                                  (@68) retained, documented deprecated/ignored.
├── wasm.rs             [CHANGED]  add_topography ABI entry (sibling of add_series @477, inserts
│                                  youngest + rejects a second); core_update_json carries units;
│                                  per-unit solid path unchanged (already returns (series,block,unit)).
├── octree.rs           [EXISTS]   masker/Scene/cap machinery reused verbatim (no change expected;
│                                  topography is a Halfspace contributor like erode).
├── mesh.rs             [EXISTS]   Mesh struct unchanged (no lithology in geometry).
└── tests / *.rs        [NEW]      inert-flag test (series w/ flag=false, no cap, still fills up);
                                   clip-derived keep/drop tests; watertightness invariant test for
                                   the topography-capped + floor-bounded + intrusion-punched volume;
                                   native↔wasm parity on the new eval/enumeration.

web3/src/                                # browser testbed
├── HierarchyPanel/
│   ├── project.ts          [CHANGED] LithologyCatalog type (Lithology{id,name,color,kind} +
│   │                                 assignment Record); serialize into project JSON; inferred-
│   │                                 litho sync on body add/rename/delete; toSeriesJson emits the
│   │                                 Topography element youngest + (optionally) the assignment;
│   │                                 unboundedAbove retained-but-unwritten.
│   ├── projectOps.ts        [CHANGED] add Topography (reject 2nd); reorder guard keeps topography
│   │                                 youngest; catalog CRUD ops.
│   ├── LithologyCatalog.tsx [NEW]     catalog panel: Air fixed, Unknown fixed, inferred (from
│   │                                 bodies), user-added; add/rename/recolour/delete per provenance.
│   ├── HierarchyPanel.tsx   [CHANGED] render the unit/contact ladder from DerivedView.units; unit
│   │                                 row → rename + lithology dropdown; contact row → existing
│   │                                 point modal, no rename; HIDE the "fills upward (floor)"
│   │                                 checkbox (@195-204).
│   ├── AddSurfaceMenu.tsx   [CHANGED] add a Topography choice; disable/hide it when one exists.
│   ├── dto.ts               [CHANGED] DerivedView gains `units` (hand-mirror of schema.rs).
│   ├── store.ts / dragResolver.ts / tables.tsx / SurfaceDataModal.tsx / SurfaceMenu.tsx [EXISTS]
│   └── hierarchy.css        [CHANGED] styles for the ladder, catalog panel, the toggle.
├── protocol.ts             [CHANGED] RenderMode already present (@33); SurfacePayload may gain an
│                                     optional lithology id tag (or stay index-only + UI lookup).
├── worker.ts               [CHANGED] solid-mode collection: per-unit solids, skip Air, tag each
│                                     payload by lithology (UI-side lookup) or pass coordinate; a
│                                     re-mesh-without-refit path for the toggle.
├── Viewer.tsx              [CHANGED] colorBy: "series" | "lithology"; in solid mode colour from the
│                                     catalog hex, skip Air.
├── exportObj.ts           [CHANGED] group by lithology in solid mode.
├── main.tsx               [CHANGED] render-mode state + Surfaces⇄Solids toggle; catalog state;
│                                     colorBy plumbing; toggle → re-mesh (not re-fit).
└── e2e/                    [NEW specs]
    ├── topography.spec.ts        # add topography → above-ground is Air, older clipped, no 2nd
    ├── unit-ladder.spec.ts       # N contacts → N+1 unit rows; rename unit, not contact; no flag UI
    ├── lithology-catalog.spec.ts # Air/Unknown fixed; inferred sync; user CRUD; many-to-one; round-trip
    └── solids-toggle.spec.ts     # toggle → watertight solids by lithology, Air omitted, no refit,
                                  #   OBJ grouped by lithology
```

**Structure Decision**: Cross-cutting feature on two existing trees — the **BSD `geofield` crate** (topography variant, clip-derived unit count, `DerivedView.units`, `add_topography`) and the **`web3/` testbed** (ladder UI, catalog, solids toggle). No new crate or package; the EUPL kernel and the second wasm artifact are untouched. The `HierarchyPanel/` boundary is preserved so the authoring + catalog UI stays liftable into the product later, exactly as 002 established.

## Complexity Tracking

> No constitution gate is violated; this table is intentionally empty. The Rust scope is larger than the prior feature but is the minimum the named-volume goal requires (a topographic surface and a clip-derived unit count are not expressible UI-side), and each piece reuses an existing mechanism (erode masker, capped-Scene mesher, `DerivedView` channel, `RenderMode`).

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| (none)    | —          | —                                    |

## Phase 0 — Research

See [research.md](./research.md). All Technical Context items resolved from the spec, the design brief, the constitution, and a Phase-0 code map of the touched surfaces; no item is left as NEEDS CLARIFICATION. Two findings *correct the brief's model of the existing code* and are called out where they change the plan's risk: **R2** (the inert-flag change is net-new clip-derivation logic, not a refactor that "falls out") and **R4** (no Rust reorder API exists; the youngest/singleton invariant is split between the `add_topography` builder and the UI resolver/compile seam).

## Phase 1 — Design & Contracts

- [data-model.md](./data-model.md) — the `Topography` variant + invariants, `DerivedView.units` (the unit coordinate the core exposes), the `LithologyCatalog` (entries, provenance, the `(element, unit) → lithology` assignment), and the inert-flag deprecation state.
- [contracts/engine-abi.md](./contracts/engine-abi.md) — the engine seam: `add_topography`, `DerivedView.units`, the clip-derived keep/drop contract, the accepted-but-ignored `unbounded_above` slot, and per-unit solid tagging.
- [contracts/catalog-and-render.md](./contracts/catalog-and-render.md) — the UI seam: catalog model + assignment key, inferred-litho sync rules, the render-mode toggle (re-mesh-without-refit), `colorBy`, and OBJ-by-lithology.
- [quickstart.md](./quickstart.md) — build both wasm artifacts, run the dev server, and run both gates (engine invariant/parity tests + the Playwright behaviour specs); the end-to-end validation scenarios.

Agent context updated: `CLAUDE.md`'s SPECKIT block repoints to this plan.
