# Quickstart: Enhanced Authoring (validation guide)

**Feature**: 002-enhanced-authoring | **Date**: 2026-06-19

How to build, run, and **gate** the authoring testbed. This is a run/validation guide — implementation details live in the contracts, the data model, and (later) `tasks.md`.

---

## Prerequisites

- Node + npm (the `web3/` harness).
- Rust toolchain with the `wasm32-unknown-unknown` target and `wasm-pack` — **only** needed to (re)build the two WASM artifacts. This feature changes **no** Rust, so if `web3/src/wasm-kernel/geofield_kernel.wasm` and `web3/src/wasm/` are already present (they are, in-repo), you can skip the wasm build.

## Build & run

```bash
cd web3
npm install
# (only if the wasm artifacts are missing or you changed Rust — this feature does not)
# npm run wasm
npm run dev          # vite dev server
```

The compile gate (run before calling any UI change done — Constitution I):

```bash
cd web3
npm run build        # tsc -b && vite build  — the COMPILE gate (necessary, not sufficient)
```

## The behavior gate (Playwright — the gate of record)

```bash
cd web3
npm run test:e2e     # playwright test — the BEHAVIOR gate (sufficient)
```

A UI change is **not done** until both pass. If the environment can't run Playwright this pass, that is a **logged, un-closed gap** (Constitution IV) — never reported as verified.

---

## End-to-end validation scenarios

Each maps to a spec user story and the Playwright spec that pins it (brief §10). These describe *what to observe*, not how to implement.

### V1 — Author a new surface (US1 → `authoring-add.spec.ts`)
1. Start from a blank or seeded project.
2. Click **+ Add Surface**, pick **Erosional** → the data modal opens for a new single-surface series.
3. Paste a TSV block of XYZ rows into the Points table → one point row per line; the count shows.
4. **Modal Apply** → modal closes; the panel shows the new surface and a **dirty** indicator; the 3D view shows the **stale scrim**.
5. **Panel Apply** → status reaches `done`; the surface appears in the derived summary; the scrim clears.
- **Pass**: the surface is present in the rendered model and the derived summary; `status: done`.

### V2 — Edit & re-apply (US2 → covered by authoring-add + modal-validation)
1. Open **Edit data…** on an existing surface → the modal shows its current points/orientations.
2. Change points; **modal Cancel** → no change. Reopen; change points; **modal Apply** → dirty, no fit yet.
3. Edit a second surface the same way; press **panel Apply once** → a single re-fit reflects both edits.
- **Pass**: both edits visible after one Apply; Cancel discarded edits.

### V3 — Stale scrim (US3 → `stale-scrim.spec.ts`)
1. Make a geometry edit → assert the scrim overlay element (by `data-testid`) is present and the Apply/Cancel bar shows.
2. Orbit the camera → the camera still responds (scrim is `pointer-events: none`).
3. **Apply** → assert the scrim element is gone.
4. Toggle a **seam relation** (cheap edit) → assert the view re-fits immediately and **no** scrim appears.
- **Pass**: scrim present for pending geometry edits, absent after Apply, never present for cheap edits. (Assert on the element, not pixels.)

### V4 — Reorder chronology (US4 → `reorder.spec.ts`)
1. Reorder two containers (drive the wired `onReorderContainers` via a test hook / assert on the resolver + a programmatic reorder — **no real `@dnd-kit` drag**, mirroring `drag-affordance.spec`).
2. The panel order changes and **dirty** shows.
3. **Apply** → the derived `effectiveRelation`/order changes as expected (mirror the `round-trip.spec` pattern).
4. Attempt a cross-container surface move → assert the declined-action toast appears and the project is unchanged.
- **Pass**: reorder round-trips through the fit; the move is declined with a message (not a silent noop).

### V5 — Export OBJ (US6 → `export-obj.spec.ts`)
1. With a successful fit and **not dirty**, trigger **Export → OBJ**.
2. Capture the download; parse the blob.
- **Pass**: the OBJ has non-trivial `v` and `f` counts (> 0) and per-surface `g` groups. (With unapplied changes, the export action is unavailable.)

### V6 — Modal validation (US1/edge cases → `modal-validation.spec.ts`)
1. A new surface with **zero points** → **modal Apply** is disabled; the panel **Apply** is blocked with a reason.
2. Add points → both enable.
3. A series with **no orientation anywhere** → the amber advisory shows but Apply still succeeds.
- **Pass**: zero-point hard-block with reason; no-orientation soft-advisory non-blocking.

### V7 — Persistence & templates (US5)
1. Author a model; **reload** → it is restored from localStorage.
2. **Export** the project; **New** (blank); **Import** the file → the model matches; importing while dirty asks to confirm first.
3. **Seed from template** (each of the three former scenarios + blank) → a valid starting project loads.
- **Pass**: survives reload and export→import round-trip; templates produce valid projects.

---

## Round-trip anchor (the seam's correctness check)

The strongest non-pixel check that `toSeriesJson` is correct: a project **seeded** from a former scenario template must compile to a `SeriesJson[]` that fits to the **same** derived result as the original `scenarios.ts` entry produced. This reuses the existing `round-trip.spec` pattern and anchors the new compile seam against known-good geology — without introducing any GemPy reference (Constitution I: the engine has no GemPy oracle; correctness here is the round-trip invariant, not a reference diff).
