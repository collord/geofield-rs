# Enhanced Authoring Demo: Generic Stack Configuration

**Status:** design spec / build brief
**Scope:** the `web3/` testbed (Playwright-gated, in-repo). Not the product UI — but authoring components stay inside the reusable `HierarchyPanel/` boundary so they're liftable later.
**Baseline:** the current `web3/` code (HierarchyPanel, dragResolver, dto, store, worker, scenarios, protocol). This spec is a **delta** against it.
**Audience:** Claude Code.

---

## 0. What changes, in one paragraph

Today the testbed fits one of three canned `SeriesJson` scenarios chosen from a dropdown; the panel can toggle the seam relation and the floor flag (cheap declared-state edits that round-trip live) but its reorder/move/add callbacks are `noop`. This spec turns the testbed into a **generic stack authoring tool**: an editable in-memory **project document** (containers + per-interface point/orientation tables) that compiles to `SeriesJson[]` and re-fits on an explicit **Apply**; an **Add Surface → pick type** flow where the chosen type silently decides container membership; a per-surface **"…" menu** opening an **edit-and-apply dual-table modal** (XYZ points + optional dip/dip-direction orientations, hand entry + clipboard paste); **live container reordering** (chronology); a **stale-state scrim** over the 3D view whenever a geometry-affecting edit is pending; and **export-to-OBJ**. The canned scenarios survive only as **seed templates**. Faults and cross-container surface moves are explicitly out of scope this iteration.

---

## 1. Decisions locked (read first — these shape everything below)

1. **Type-on-add determines container membership.** The user never says the word "container." When they Add a Surface and pick a type:
   - **Conformable** → appends an interface to the *adjacent conformable series* (grows one co-kriged field) if one is in the insert position; otherwise starts a new conformable series.
   - **Erosional / Onlap** → starts a **new** single-interface series with that `relation`.
   - **Vein / Intrusion** → starts a **new** cross-cutting container of that kind.
   A container holding one interface is presented **flat** (as just "a surface"); a multi-interface series is presented **nested** (the surfaces under a series header). The two-level model is unchanged underneath; only the presentation collapses the singleton case.
2. **Apply / Cancel for geometry edits; live round-trip for declared-state edits.** Editing point/orientation tables, adding/deleting a surface, or reordering containers sets a **`dirty`** flag and requires **Apply** (re-fit) to take effect, with **Cancel** to discard. The existing cheap edits — seam `relation`, `unbounded_above` — keep their immediate round-trip and do **not** set `dirty` (they re-fit at once, so nothing is stale).
3. **Stale-state scrim.** While `dirty`, a translucent grey scrim blends over the 3D view to signal the render is stale; Apply clears `dirty` and the scrim on a successful re-fit.
4. **Orientations are entered as dip / dip-direction**, converted to the ABI's pole vector under the hood (§5.3). Lossless to `ori_pole`.
5. **Reorder containers is wired live; cross-container surface move stays punted.** Within-series isovalue reorder (already in the resolver) is also wired.
6. **Faults are out of scope** this iteration. The per-series `fault_point`/`fault_normal` fields keep their no-fault defaults; no authoring UI for them.
7. **Export to OBJ** is in scope (§8).

---

## 2. The project document (new) — `HierarchyPanel/project.ts`

A single editable client-side model, the source of truth for authoring. It **compiles to `SeriesJson[]`** (the worker's existing input) and is the thing that persists.

```ts
/** Authoring-facing surface type. Maps to (container kind + Axis-A relation). */
export type SurfaceType = "conformable" | "erosional" | "onlap" | "vein" | "intrusion";

/** One interface's interpolation data, authored in the modal. */
export interface InterfaceData {
  id: string;                 // stable client id
  name: string;
  points: PointRow[];         // XYZ contact points (≥1 to be valid; ≥3 to fit a plane well)
  orientations: OrientationRow[]; // optional; dip/dip-dir entry
}
export interface PointRow { x: number; y: number; z: number }
export interface OrientationRow { x: number; y: number; z: number; dip: number; dipDir: number }

/** An authoring container. Singleton series render flat; multi-interface series nest. */
export type ProjectElement =
  | { kind: "series"; id: string; name: string;
      relation: "erode" | "onlap" | "conformable"; unboundedAbove: boolean;
      interfaces: InterfaceData[] }                       // 1..n interfaces, one co-kriged field
  | { kind: "vein"; id: string; name: string;
      footWall: InterfaceData; hangingWall: InterfaceData }
  | { kind: "intrusion"; id: string; name: string;
      envelopeInner: InterfaceData; envelopeOuter: InterfaceData };  // equal ⇒ single envelope

export interface Project {
  version: 1;
  name: string;
  extent: [number, number, number, number, number, number];
  elements: ProjectElement[];   // youngest first (matches add_* call order)
}
```

### 2.1 Compile to the worker ABI — `project.ts::toSeriesJson(project): SeriesJson[]`

Pure function. For each element, in order, emit the `SeriesJson` the worker already understands (`protocol.ts`):

- **series** → one `SeriesJson` with `relation` (code 0/1/2), `unboundedAbove`, and `sp_xyz`/`sp_if` flattened across its interfaces (interface index → `sp_if`), orientations concatenated, `crossCut` absent.
- **vein** → `SeriesJson` with `crossCut: { kind:"vein", footWall, hangingWall }`, points/orientations from the two interfaces (interface ids 0/1).
- **intrusion** → `crossCut: { kind:"intrusion", envelopeInner, envelopeOuter }`.

Orientation rows convert dip/dip-dir → pole here (§5.3). This function is the single seam between the authoring model and the existing fit path; the worker, kernel-bridge, and `add_series/add_vein/add_intrusion` ABI are **unchanged**.

### 2.2 Persistence

- **localStorage autosave** of the whole `Project` on every committed edit (debounced ~500 ms), keyed by project name; reload restores it.
- **Export / Import JSON**: download the `Project` as `<name>.geoproj.json`; import replaces the current project (with a confirm if dirty). This is the demo's "save file."

---

## 3. Data flow & state (delta to `main.tsx`)

Replace the scenario-centric flow with a project-centric one.

```
Project (editable, localStorage)
   │  user edits (modal apply / add / delete / reorder)  → set dirty=true
   ▼
toSeriesJson(project)  ──Apply──▶  worker.run  ──▶  CoreUpdate + meshes
   │                                                   │
   │  cheap edits (seam relation, unbounded_above)     ▼
   └──────────────immediate run (dirty stays false)───►  panel derived + viewer
```

New top-level state in `App`:

```ts
const [project, setProject] = useState<Project>(/* loaded from localStorage or blank */);
const [dirty, setDirty] = useState(false);     // geometry-affecting edits pending
const [applying, setApplying] = useState(false);
```

- **Geometry-affecting edit** (modal apply, add/delete surface, reorder containers): mutate `project`, `setDirty(true)`. Do **not** run the worker yet.
- **Apply button**: `setApplying(true)` → `runStack(toSeriesJson(project))` → on `done`, `setDirty(false); setApplying(false)`.
- **Cancel button**: revert `project` to the last-applied snapshot (keep a `lastApplied: Project` ref), `setDirty(false)`.
- **Cheap declared-state edit** (seam relation, unbounded): mutate `project` *and* immediately `runStack` (as today), leaving `dirty` false.

The existing worker hard-terminate-on-new-run cancellation is unchanged.

---

## 4. UI: the authoring shell (delta to `main.tsx` + new components)

### 4.1 Left panel layout (top → bottom)

1. **Project bar**: project name (editable), New / Import / Export buttons, "Seed from template ▾" (the three canned scenarios become templates here).
2. **Apply / Cancel bar**: shown only when `dirty`. Apply (primary) + Cancel. A dirty dot + "unapplied changes" label. Disabled-with-reason when hard-invalid (§7).
3. **`HierarchyPanel`** (enhanced): the tree, now with Add controls and the "…" menus.
4. **status** line (existing).

### 4.2 Add controls

- A top-level **"+ Add Surface"** button at the bottom of the hierarchy. Opens the **type picker** (§4.3).
- Insert position: by default appends to the oldest end (bottom). A future nicety (not v1) is insert-at-selection; for v1, after adding, the user reorders via drag.

### 4.3 Type picker (new) — `AddSurfaceMenu.tsx`

A small popover (not a full modal): five choices with one-line glosses.

| Choice | Result | Gloss |
|--------|--------|-------|
| Conformable | append interface to adjacent conformable series, else new series | "stacks conformably; shares a field with its neighbours" |
| Erosional | new series, `relation=erode` | "truncates older surfaces below it" |
| Onlap | new series, `relation=onlap` | "laps onto and pinches out against older topography" |
| Vein | new vein container (HW/FW pair) | "tabular body that cross-cuts what it passes through" |
| Intrusion | new intrusion container (envelope) | "volumetric body that cross-cuts; fills its interior" |

On pick: create the element with **one empty interface (or the required pair/envelope)**, named by a sensible default ("Surface 4", "Quartz vein"), insert it, `setDirty(true)`, and immediately open its data modal (§5) so the user can enter points. A surface with zero points is hard-invalid until they do.

### 4.4 Enhanced `HierarchyPanel` rendering

Keep the current structure; add:

- **Per-surface "…" menu** (new `SurfaceMenu.tsx`): Edit data… / Rename… / Duplicate / Delete. "Edit data…" opens the modal (§5). For a multi-interface series, each interface row gets its own "…"; the series header gets a "…" with series-level actions (rename, delete series, add interface).
- **Singleton flattening**: a `series` with exactly one interface renders as a single surface row (no nested list, no series header) — the "container is internal" simplification. A vein/intrusion always renders with its named sub-surfaces (HW/FW, inner/outer) since the pair is meaningful.
- **Add-interface affordance** inside a multi-interface series ("+ interface").
- Existing seam control, floor marker, validation badge, derived summary **unchanged** (they already read from `CoreUpdate.derived`).

---

## 5. The dual-table data modal (new) — `SurfaceDataModal.tsx`

Edit-and-apply. Opening reads the `InterfaceData` into local modal state; **Apply** writes it back to the project (and sets `dirty`); **Cancel** discards modal-local edits. The modal does **not** itself run the worker — it just commits to the project; the panel-level Apply re-fits.

> Note the two Apply levels: the **modal's** Apply commits table edits to the project (cheap, local) and closes the modal; the **panel's** Apply re-fits the whole stack. After a modal Apply, the panel shows `dirty` and its Apply becomes available. This is deliberate — editing several surfaces then re-fitting once.

### 5.1 Layout

Two side-by-side tables under a header naming the surface and its parent element:

- **Points** (required): columns `X | Y | Z | ⌫`. Add-row button; per-row delete. Row count badge.
- **Orientations** (optional): columns `X | Y | Z | Dip | Dip Dir | ⌫`. Add-row; per-row delete. Empty is valid.

Footer: validation summary (e.g. "12 points, 1 orientation — OK" / "0 points — add at least one"), **Apply** (disabled if zero points), **Cancel**.

### 5.2 Entry & clipboard paste

- **Hand entry**: numeric inputs per cell; Tab/Enter navigation; Enter on the last row adds a row.
- **Clipboard paste**: paste TSV/CSV into either table (a "Paste" affordance or Ctrl-V on a focused table). Parse rows by whitespace/comma; map to the table's columns in order (Points: 3 cols; Orientations: 5 cols). Extra/short rows are flagged, not silently dropped. This is the bulk-entry path for the demo (no file import in v1).

### 5.3 Dip/dip-direction → pole conversion (the one bit of geology in the UI)

The ABI wants an outward pole unit vector `ori_pole = (nx, ny, nz)`. Convert from dip (δ, degrees from horizontal) and dip-direction (α, degrees clockwise from North = +Y):

```ts
// geology convention: dip-direction is azimuth of steepest descent; pole points UP-dip.
function dipDirToPole(dipDeg: number, dipDirDeg: number): [number, number, number] {
  const d = dipDeg * Math.PI / 180, a = dipDirDeg * Math.PI / 180;
  // downward normal of the dipping plane, then negate to get the upward pole:
  const nx = Math.sin(d) * Math.sin(a);
  const ny = Math.sin(d) * Math.cos(a);
  const nz = Math.cos(d);
  return [nx, ny, nz]; // already unit; points up out of the footwall
}
```

State the axis convention in a tooltip (X=East, Y=North, Z=Up; dip-dir clockwise from +Y). Keep the raw dip/dipDir in `OrientationRow` (so the table round-trips what the user typed); convert only at `toSeriesJson` time. **Document this conversion in a code comment with the convention**, because it's the one place a wrong sign silently produces an upside-down field.

---

## 6. Reorder (wire the existing resolver) — delta to `main.tsx`

The resolver and CSS affordances already exist. Wire the callbacks:

- **`onReorderContainers(from, to)`**: reorder `project.elements`; `setDirty(true)`. (Chronology change → needs re-fit.) Maps to moving an entry in the youngest-first array.
- **`onReorderWithin(elementId, from, to)`**: reorder the `interfaces` array of that series; `setDirty(true)`. (Isovalue order within one field.)
- **`onMoveSurface`**: keep `noop` with a toast ("moving a surface between containers isn't supported — delete and re-add"). Documented punt.

Because reorder now changes geometry/chronology, it sets `dirty` and applies on the next Apply, consistent with §2 decision 2. (The current code's expectation that reorder round-trips immediately is replaced by the dirty/Apply flow.)

---

## 7. Validation (extend the existing advisory badges)

Keep the derived per-container badges (sourced from `CoreUpdate.derived`). Add **author-time** validation computed locally on the `Project` (before fit), since some invalids can't be fit at all:

- **Hard-invalid (blocks panel Apply, with reason shown):**
  - any series with zero interfaces;
  - any interface with zero points;
  - a vein/intrusion missing one of its required interfaces.
- **Soft (advisory, doesn't block):**
  - interface with < 3 points (plane underdetermined) → amber "needs ≥3 points to fit well";
  - series with no orientation anywhere → amber "no orientation — fit may be ambiguous" (the kernel can still attempt it).

Author-time validation lives in `project.ts::validate(project): ProjectValidation` and is surfaced both at the surface row and aggregated on the Apply button.

---

## 8. Export to OBJ (new) — `exportObj.ts`

Add an **Export mesh ▾ → OBJ** action in the project bar, enabled when a fit has produced surfaces and `!dirty` (you export what's rendered).

- Reuse the `SurfacePayload[]` already held in `App` state (positions/normals/indices per surface).
- Concatenate into one OBJ: `v`/`vn` from positions/normals, `f` with `//` normal refs, vertex-index offset accumulated across surfaces. Group per surface with `g series{si}_block{b}_iface{k}` and a `usemtl` per series index (emit a tiny companion `.mtl` with the `SERIES_COLORS`, or skip materials in v1 and just group).
- Trigger a browser download of `<project-name>.obj`. Pure client-side; no worker round-trip.

`v1 scope: geometry + groups; materials optional.`

---

## 9. Component/file ledger (what to add or change)

**New files (under `web3/src/HierarchyPanel/` unless noted — keep the reusable boundary):**
- `project.ts` — `Project`/`ProjectElement`/`InterfaceData` types, `toSeriesJson`, `validate`, localStorage load/save, JSON export/import.
- `AddSurfaceMenu.tsx` — the five-type picker popover.
- `SurfaceDataModal.tsx` — the dual-table edit-and-apply modal.
- `SurfaceMenu.tsx` — the per-surface "…" menu.
- `tables.tsx` — the reusable points/orientations table with clipboard paste.
- `exportObj.ts` (in `web3/src/`) — OBJ serialization from `SurfacePayload[]`.
- `dipDirToPole` helper (in `project.ts`).

**Changed files:**
- `main.tsx` — project-centric state (`project`, `dirty`, `applying`, `lastApplied`), Apply/Cancel bar, project bar, scrim, wire reorder callbacks, export action. Scenarios become seed templates.
- `HierarchyPanel.tsx` — singleton flattening, "…" menus, add-interface affordance, surface rows carry edit entry points. (Seam/floor/validation/derived rendering unchanged.)
- `scenarios.ts` — re-expose the three as **named seed templates** returning a `Project` (not the auto-loaded dropdown); add a "blank" template.
- `Viewer.tsx` — accept a `stale?: boolean` prop and render the translucent grey scrim overlay when set (a positioned `<div>` over the canvas, `pointer-events: none`, ~40% grey), so orbiting still works but the view reads as stale.
- `dragResolver.ts` — unchanged (already correct); just stop ignoring its output for containers/within.
- `dto.ts` / `store.ts` / `protocol.ts` / `worker.ts` / `kernel-bridge.ts` — **unchanged**. The authoring model compiles down to the existing `SeriesJson`/ABI. (This is the key constraint: no Rust/worker/ABI changes this iteration.)

---

## 10. Playwright coverage (extend `web3/e2e/`)

Keep the existing specs. Add:

- **authoring-add.spec.ts** — Add Surface → pick "Erosional" → modal opens → paste a TSV points block → modal Apply → panel shows the new surface + dirty → panel Apply → `status: done` and the surface appears in the derived summary.
- **stale-scrim.spec.ts** — after a geometry edit, the scrim overlay is present (a known test-id div over the canvas) and `dirty` affordances show; after Apply, the scrim is gone. (Assert on the overlay element, not pixels.)
- **reorder.spec.ts** — reordering two containers changes their order in the panel and, after Apply, changes the derived `effectiveRelation`/order (prove the round-trip, mirroring the existing round-trip.spec pattern). Drag simulation is flaky, so drive the wired `onReorderContainers` via a test hook or assert on the resolver + a programmatic reorder, consistent with how drag-affordance.spec avoids real drags.
- **export-obj.spec.ts** — trigger Export → OBJ; assert a download with non-trivial `v`/`f` content (parse the blob, check vertex+face counts > 0).
- **modal-validation.spec.ts** — a surface with zero points blocks modal Apply; adding points enables it; a series with no orientation shows the amber advisory but still applies.

---

## 11. Explicit non-goals (this iteration)

- **Faults / block model** — per-series fault fields keep no-fault defaults; the orthogonal fault-partition from the model spec is a later feature.
- **Cross-container surface moves** — punted (delete + re-add instead).
- **Insert-at-selection on Add** — append + reorder for v1.
- **In-modal 3D preview** — the main view updates on Apply.
- **CSV file import** — clipboard/TSV paste only; JSON project import is the file path.
- **ts-rs auto-generation** of the DTOs — still hand-mirrored (unchanged from baseline; the project model is UI-only and doesn't cross the wasm boundary).
- **Materials in OBJ** — geometry + groups first; `.mtl` optional.

---

## 12. Why this is the right increment (for reviewers)

The authoring model compiles to the **existing** `SeriesJson` ABI, so the entire enhancement is a TypeScript/React layer over an unchanged two-wasm core — no kernel, engine, worker, or schema churn. It makes the three concepts the model already supports (conformable series, erode/onlap unconformities, cross-cutting veins/intrusions) **fully authorable** rather than canned, with the container abstraction hidden behind a type-on-add verb so the user thinks in surfaces while the model keeps its co-kriging groups. The dirty/Apply/scrim loop makes the cost of a re-fit explicit and the stale render unmistakable; reorder is wired where it's cheap and punted where it implies field surgery; and OBJ export closes the loop from authored data to a portable artifact. It stays inside the reusable `HierarchyPanel` boundary, so the authoring UI lifts into the product later intact.

---

## 13. Build notes (as landed — feature 002 implement)

The implementation matches §1–§12 with two small, deliberate deviations from the §9 file ledger, recorded here so the document and the code agree (Constitution II/IV):

- **`HierarchyPanel/projectOps.ts` (new) was split out of `project.ts`.** §9 placed `toSeriesJson`/`validate`/`dipDirToPole` and the add/edit/reorder *operations* in one `project.ts`. As built, `project.ts` holds the types + the compile seam + validation + persistence; the pure mutation operations (`addSurface`, edit/rename/duplicate/delete/add-interface, reorder, `findInterface`, `applyInterfaceData`) live in a sibling `projectOps.ts`. Same `HierarchyPanel/` boundary, no behavioural change — just a readable separation of "model" from "edits."
- **Two `window.__reorderContainers` / `__moveSurface` test hooks on `App`.** §10's `reorder.spec` anticipated "a test hook" because real `@dnd-kit` drag is flaky; the implementation exposes the two wired callbacks on `window` (in a `useEffect`) so the spec drives the *same* code path the UI uses, and asserts the round-trip / the declined-move message deterministically. The hooks are unused by the UI itself.

Verification (both constitution gates, run green and deterministically): **compile gate** `npm run build` (`tsc -b && vite build`) passes; **behavior gate** `npm run test:e2e` passes 22/22 (7 new specs — authoring-add, modal-validation, stale-scrim, reorder, persistence, export-obj, flip-orientation — plus the 5 prior specs updated to the project-centric flow). The Playwright `workers` cap was lowered to 2 because every test drives a real WASM stack fit and concurrent fits starved each other on CPU; the suite still finishes in ~45 s. No file under `crates/`, `web3/src/wasm*`, `worker.ts`, `protocol.ts`, `dto.ts`, `store.ts`, or `kernel-bridge.ts` changed — the "no core change" constraint holds (verified by `git status`).

### Follow-up: per-surface "Flip orientation" + the erosion-surface hole

Reported against the seeded **erosional-unconformity** template: (1) the erosion surface's orientation was inverted, and (2) the blue unconformity surface had a hole/tear. Diagnosis (verified in the real 3D render, before/after, not a 2D metric): **#2 was a *symptom of* #1, not an independent mesher bug.** The authoring template always emitted the "below" up-pole `(−sx,−sy,1)`, but an erosional deposit caps its contact from *above* (feature-001's `scenarios.ts` encoded this with `depositSide:"above"`). With the inverted field, the octree mesher tore a hole exactly where the erosion surface crosses the floor; flipping the pole closed it deterministically (flipping back reproduces the hole).

Fix (smallest correct intervention):
- A new optional, reversible **`InterfaceData.flipped`** declared bit negates the orientation poles **at compile time** in `toSeriesJson` — the raw dip/dipDir the user typed are never rewritten (round-trip intact). It is a *separate* bit, not a mutation of the entered values, because `dipDirToPole` (dip ∈ [0,90]) can't represent a down-pole directly.
- A per-surface **"Flip orientation"** action in the "…" menu (`flipOrientation` in `projectOps.ts`), showing a ✓ when flipped — lets the user correct any inverted surface. It is a geometry edit (dirty → Apply re-fits), covered by `flip-orientation.spec.ts`.
- The **erosional-unconformity template** now seeds its erosion surface `flipped: true`, so it renders correctly out of the box (matching feature-001's `depositSide:"above"`).

A *residual* ragged stair-step along the brown/blue crease remains after the fix. It is the pre-existing octree open-sheet **termination** artifact (the subject of commit `ed21b40 fix(octree): terminate open contact sheets exactly at the truncation crease`), present in feature-001 too, at octree resolution — a separate, lower-priority item, not the reported hole. Logged here, not silently absorbed (Constitution IV).
