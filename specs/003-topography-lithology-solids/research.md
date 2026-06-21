# Phase 0 — Research: Topography, Lithology Catalog & Solid Volumes

**Feature**: `003-topography-lithology-solids` · **Date**: 2026-06-20

This feature is a **delta on an existing two-wasm engine + web3 testbed**, so "research" is
primarily *reading the baseline to confirm what exists and where the seams are*, not surveying
external technology. All Technical Context items resolved from the spec, the design brief
([docs/topography_lithology_solids_spec.md](../../docs/topography_lithology_solids_spec.md)), the
constitution, and a code map of the touched surfaces. No item remains `NEEDS CLARIFICATION`.

Each entry: **Decision** / **Rationale** / **Alternatives considered** — and, where the baseline
contradicts the brief's mental model, an explicit **Correction** note.

---

## R1. How Topography enters the type system

**Decision**: Add a fourth `StackElement` variant — `Topography` — alongside `Series`/`Vein`/`Intrusion`
([stack.rs:250](../../crates/geofield/src/stack.rs#L250)). It owns one `Model` (a single ground
surface) and reuses the existing erode `Masker::Halfspace` clip ([stack.rs:328](../../crates/geofield/src/stack.rs#L328),
[stack.rs:415-467](../../crates/geofield/src/stack.rs#L415)) to truncate every older element to below it.

**Rationale**: The brief (§2.1) and Constitution Principle III (no smuggled semantics) both argue
for a structural variant over a flagged youngest series: topography's invariants (singleton,
youngest, air-above, erode-clip) are *structural*, so they belong in a variant the engine enforces,
not a convention a flag pretends. Erode is already "younger truncates older"; topography is exactly
that with the upper unit pinned to Air.

**Alternatives considered**: (a) *A flagged youngest `Series`* — rejected: re-creates the
`unbounded_above`/`Basement` category-error pattern the constitution names as a cautionary tale, and
makes "exactly one, always youngest" a runtime check on a flag rather than a type invariant. (b)
*A new top-level field on `Stack` (not a `StackElement`)* — rejected: topography participates in the
youngest-first chronological masking exactly like an erode element, so it belongs *in* the element
list (at index 0), reusing `clip_maskers`; pulling it out would duplicate the masking machinery.

---

## R2. Making `unbounded_above` inert — and the correction to the brief

**Decision**: Drive the keep-vs-drop-top-unit decision from **whether a younger element (including
topography) caps the series at that point**, computed in the core, replacing the flag read in
`present_unit_count`. Keep the `unbounded_above` slot in the schema/ABI **accepted-but-ignored**.

**Correction (the brief's model is slightly off here — flag this in the plan)**: The brief (§3.2)
says the flag read "falls out of the clip result" as if the clip already carries the keep/drop
information. It does **not**. Today `present_unit_count` ([stack.rs:711-723](../../crates/geofield/src/stack.rs#L711))
decides purely from the flag — `n` if `unbounded_above`, else `n - 1` — with **no clip-awareness at
the unit-count level**. `global_unit` ([stack.rs:388-396](../../crates/geofield/src/stack.rs#L388))
similarly short-circuits `present_at` when the flag is set. So "derive it from the clip" is
**net-new derivation logic**, not a refactor that falls out for free. This is the single highest-risk
Rust change and the place Principle III is most load-bearing.

**Rationale**: The honest rule is "a series keeps its top unit where nothing younger caps it
(the floor; an exposed top under open sky), and drops it where something younger does (topography
cutting below; a younger erode)." With topography supplying the universal cap, this *is* derivable —
the same `clip_maskers(i)` set that already bounds the solid mesh tells us, at a point, whether a
younger element is present above the series' top contact. The flag becomes a redundant shadow of the
missing topographic surface (exactly what the as-built foresaw).

**Alternatives considered**: (a) *Leave the flag, special-case topography* — rejected: keeps two
sources of truth for "does this fill upward" (Principle III), and the spec's universal-default
requires the flag be inert for *all* series, not just under topography. (b) *Delete the slot now* —
rejected by the brief and spec: a half-removed flag mid-refactor breaks serialized scenarios and the
Rust test suite; staged-inert is the safe middle. Slot removal is later cleanup (Out of Scope).

**Test obligation**: Add a Rust test that a series with `unbounded_above = false` **and** no younger
cap still fills upward (the new universal default), proving the flag is inert; and confirm the
existing `present_unit_count` tests still pass because the *behaviour* (floor keeps its top; capped
series drop theirs) is preserved, now clip-driven. (SC-009.)

---

## R3. Exposing per-element units to the boundary

**Decision**: Add a `units` field to `DerivedView` ([schema.rs:110-120](../../crates/geofield/src/schema.rs#L110),
mirrored in [dto.ts:41-45](../../web3/src/HierarchyPanel/dto.ts#L41)) carrying, per element, an
ordered list of unit coordinates (`{ index }`, youngest→oldest within the element). The core provides
only the *coordinates*; it knows nothing of lithology names.

**Rationale**: The UI must render the unit/contact ladder (spec Story 2) and key assignments on
`(elementId, unitIndex)` (spec Story 3). The core already computes unit counts
(`block_unit_count`/`present_unit_count`); `DerivedView` is the existing read-only channel for
core-derived facts the UI must not recompute (Principle III). Putting unit *coordinates* there — and
keeping lithology *names/colours/assignments* entirely UI-side — places the seam exactly where the
brief (§5.1) and the constitution want it: the core derives geometry, the catalog is the UI's.

**Alternatives considered**: (a) *UI recomputes unit count from interface count* — rejected: N+1 is
only the *conformable* count before clipping; topography/younger-erode clipping removes units, and
the UI must not re-encode the clip rule (Principle III, "no second drift-prone copy"). (b) *A separate
ABI query per element* — rejected: `DerivedView` is already round-tripped whole on every core update;
a parallel query path is redundant surface.

---

## R4. Singleton/youngest/reorder invariants — where each half lives

**Decision**: Enforce topography's invariants in **two places, by construction**:
- **Engine**: `add_topography` (new ABI entry, sibling of `add_series`) inserts at index 0 and the
  builder refuses a second topography. The variant existing only at index 0 is the structural
  guarantee `global_unit`/`clip_maskers` rely on.
- **UI (web3)**: the add-surface picker hides/disables Topography when one exists (spec FR-003); the
  drag/reorder resolver refuses to move any element above topography or topography below anything
  (spec FR-002).

**Correction**: There is **no reorder mutation API in Rust today** — `Stack` is immutable, built
append-only via `add_series`/`add_vein`/`add_intrusion` ([wasm.rs:477-579](../../crates/geofield/src/wasm.rs#L477)),
and reorder lives UI-side in `dragResolver.ts` (it recompiles the project and rebuilds the stack).
So "reorder refuses to move topography" is a **UI-resolver invariant**, and "topography is always
youngest" is an **add-order invariant** the compile seam (`toSeriesJson`) must honour by emitting
topography first (youngest). The engine's job is only to *reject a second* topography and to *place
it at index 0* when built. State this split in the plan so neither half is assumed to be the other's
job.

**Alternatives considered**: A Rust-side reorder API enforcing the invariant — rejected as scope
creep (Principle VI): no reorder API exists to extend, and the project is rebuilt-from-scratch on
every Apply, so the invariant is naturally a compile-time property of `toSeriesJson` plus a
resolver guard.

---

## R5. Solid meshing reuse — topography as universal top masker

**Decision**: Reuse the existing capped-Scene path verbatim
(`mesh_one_block_unit_solid` → `octree_dual_contour_scene` with `cfg.cap = true`,
[stack.rs:736-798](../../crates/geofield/src/stack.rs#L736), [octree.rs:877](../../crates/geofield/src/octree.rs#L877)).
Add topography to the `clip_maskers(i)` set as a universal top masker (it *is* the youngest erode
surface — the mechanism already exists). Skip emitting any solid for the Air (above-topography) unit.

**Rationale**: The brief (§5.2) and Principle VI both demand reuse over rebuild. The masker
infrastructure (`Masker::Halfspace` for erode, `SlabComplement` for vein bands) already produces
watertight non-convex volumes; topography is one more `Halfspace` contributor. No new meshing
architecture.

**Alternatives considered**: A dedicated topography-clipped mesher — rejected: it would duplicate
`clip_maskers`/`Scene` assembly for no geometric difference (topography clips identically to any
erode top).

**Watertightness test obligation**: The brief (§5.2) and spec SC-005 name the multi-clip case —
a unit capped by topography above, its floor contact below, **and** punched by an intrusion — as the
one to test. Precedent (`SlabComplement` + capped Scenes) suggests it's covered; *don't pre-solve* —
add the watertightness invariant test for that exact volume and let it tell us.

---

## R6. Lithology id attachment — how a solid carries its lithology

**Decision**: The core's per-unit solid path already knows `(series, block, unit)` at mesh time and
returns one `SurfacePayload` per cell carrying those indices ([protocol.ts:55-62](../../web3/src/protocol.ts#L55),
[worker.ts:70-119](../../web3/src/worker.ts#L70)). **The lithology id is applied UI-side**: the
worker tags each solid's `SurfacePayload` by looking up the catalog assignment keyed on
`(elementId, unitIndex)`. No lithology concept crosses into the Rust mesh struct.

**Rationale**: Meshes carry no colour today (colour is `series % palette` at render time,
[Viewer.tsx:149](../../web3/src/Viewer.tsx#L149)). The cleanest seam: the core emits geometry tagged
with the *unit coordinate* it already has; the UI maps coordinate → lithology → colour via the
catalog. This keeps lithology entirely UI-side (Principle III: the core knows nothing about
lithology names) and reuses the existing per-payload index plumbing.

**Alternatives considered**: (a) *Pass the assignment into the worker and tag in Rust* — rejected:
pushes a UI concept (the catalog) across the wasm boundary for no benefit; the UI already has the
`(elementId → series index)` mapping from the compile seam. (b) *Per-vertex colour in the Mesh
struct* — rejected: unnecessary; one lithology per solid, tagged at the payload level, suffices and
matches how OBJ grouping already works ([exportObj.ts:21](../../web3/src/exportObj.ts#L21)).

---

## R7. Catalog model, provenance, and persistence

**Decision**: A UI-side `LithologyCatalog` in `project.ts`: a list of `Lithology { id, name, color,
kind }` (kind ∈ air | inferred | user, plus fixed Unknown) and an `assignment: Record<\`${elementId}:${unitIndex}\`, LithologyId>`.
Air and Unknown are fixed members; inferred members are synced from vein/intrusion bodies on
add/rename/delete; user members are CRUD. Persistence is **live + serialized into the exported
project JSON** (no localStorage/db beyond the existing project autosave that already carries the
project document).

**Rationale**: Matches the brief (§4) and the spec exactly. The catalog is identity metadata over
the core's volumes; it has no business in the engine. Serializing it into the project JSON the
testbed already round-trips ([project.ts:234-287](../../web3/src/HierarchyPanel/project.ts#L234))
gives the round-trip the spec requires (FR-021, SC-007) with no new persistence layer.

**Alternatives considered**: A separate catalog file / storage key — rejected by the spec
(live-only + project-JSON, no separate persistence); the project JSON is the single save artifact.

---

## R8. The render-mode toggle and re-mesh-without-refit

**Decision**: Wire the existing `RenderMode = "contact" | "solid"` ([protocol.ts:33](../../web3/src/protocol.ts#L33),
already carried in `RunCommand` but hardcoded `"contact"`) to a project-bar toggle. Toggling re-runs
**only mesh collection** in the chosen mode against the already-fitted stack — no re-fit. In solid
mode the worker collects per-unit solids (skipping Air), tags each by lithology, and the viewer
colours by lithology; OBJ export groups by lithology.

**Rationale**: The protocol already models the distinction; the fit and the mesh extraction are
already separable (mesh collection reads the fitted `WasmStack`). The spec (FR-025, SC-006) requires
no kernel solve on toggle; the baseline supports it because `collectSurfaces` is a read over the
fitted stack. If a clean re-collect-without-refit hook doesn't already exist in the worker, adding
one is in scope (it's a read path, not a refit).

**Alternatives considered**: Treating the toggle as a full Apply (refit) — acceptable fallback the
brief allows ("if cheap enough, switch without refit; otherwise treat as a normal apply"), but the
preferred path is re-mesh-only since the fit is unchanged. Plan for re-mesh-only; fall back only if a
worker constraint forces it, and log that as a coverage note (Principle IV).

---

## R9. Validation gates (which gate backs which claim)

**Decision**:
- **Engine correctness** (topography clip, unit derivation, inert flag, watertight solids): Rust unit
  tests + the **mesh invariant** tests (no holes / manifold / winding) for the multi-clip volume, and
  **native↔wasm parity** on the new evaluation/enumeration (Principles I, V). No GemPy reference is
  introduced or reachable.
- **UI/behavioural** (topography add + cap visible, unit ladder, catalog CRUD + round-trip, solids
  toggle + lithology colour, OBJ-by-lithology): **Playwright** e2e specs driving the running app
  ([web3/e2e/](../../web3/e2e), [playwright.config.ts](../../web3/playwright.config.ts) already
  present), with `tsc -b` + `vite build` as the compile gate.

**Rationale**: This is the constitution's explicit gate split (Principle I): geometry by invariant +
parity, UI by functional test, never by manual walk or screenshot. Both halves of this feature have a
named gate; building any missing gate is in scope, not a follow-up.

**Alternatives considered**: None — the gate assignment is constitutionally fixed.
