# Phase 0 Research: Enhanced Authoring

**Feature**: 002-enhanced-authoring | **Date**: 2026-06-19

No Technical Context item is unresolved. Every choice below was decidable from the spec, the design brief ([docs/enhanced_authoring_spec.md](../../docs/enhanced_authoring_spec.md), whose §1 locks the decisions), and the existing `web3/` baseline read during planning. This file records the decisions and why, so the plan starts from settled ground.

---

## R1 — The authoring↔fit seam: one pure compile function

- **Decision**: Introduce a client-side `Project` document and a single pure function `toSeriesJson(project): SeriesJson[]` as the only bridge between authoring and the existing fit path. The worker, `protocol.ts`, `kernel-bridge.ts`, and the `add_series`/`add_vein`/`add_intrusion` ABI are unchanged.
- **Rationale**: The existing worker already consumes `SeriesJson[]` (youngest-first) via `runStack`. Compiling the project down to that shape means zero core/wasm/Rust churn (Constitution V, VI) and keeps the geological derivation in the core (Constitution III). `SeriesJson` was confirmed during planning to carry everything the project needs to emit: `relation` code (0=Erode/1=Onlap/2=Conformable), `unboundedAbove`, flattened `sp_xyz`/`sp_if`, `ori_xyz`/`ori_pole`, no-fault `fault_*` defaults, and an optional `crossCut` discriminated union for vein/intrusion.
- **Alternatives considered**: (a) Editing `SeriesJson[]` directly in the UI — rejected: it has no place for raw dip/dip-direction (only poles), no per-surface identity for editing, and no persistence shape. (b) Adding a Rust-side authoring model — rejected: violates "no core changes" and Constitution V/VI; the authoring model never needs to cross the wasm boundary.

## R2 — Dirty/Apply vs. immediate round-trip

- **Decision**: Geometry-affecting edits (modal commit, add/delete surface, reorder) set `dirty=true` and defer the fit to an explicit panel **Apply**; **Cancel** reverts to a `lastApplied` snapshot. Cheap declared-state edits (seam `relation`, `unbounded_above`) keep the existing immediate `runStack` and never set `dirty`.
- **Rationale**: Re-fitting is the expensive operation; batching many edits into one Apply (brief §2 decision 2) is the whole point. The two existing cheap edits already round-trip live and re-fit at once, so nothing is ever stale for them — keeping their behavior avoids a regression and matches the brief. The existing hard-terminate-on-new-run cancellation is retained for both paths.
- **Alternatives considered**: Re-fit on every edit — rejected: makes multi-surface authoring pay N fits and removes the user's control over when the cost is incurred; also defeats the stale-scrim's reason to exist.

## R3 — Stale signal: a positioned scrim div over the canvas

- **Decision**: `Viewer` gains a `stale?: boolean` prop; when set, render a positioned translucent grey `<div>` (~40% grey, `pointer-events: none`) over the canvas mount so orbiting still works but the view reads as stale. Tests assert on the overlay element (a known test-id), not pixels.
- **Rationale**: The current viewer mounts the three.js canvas into a single `<div ref={mountRef}>` with no overlay anchor (confirmed during planning). Wrapping that mount in a positioned container and adding a sibling scrim div is the smallest change (Constitution VI) and gives a stable DOM hook for the Playwright `stale-scrim.spec` (Constitution I — assert on an element, not a screenshot). `pointer-events: none` preserves camera interaction.
- **Alternatives considered**: A WebGL post-effect / shader tint — rejected: heavier, harder to assert in a functional test, and unnecessary for a "render is stale" signal.

## R4 — Type-on-add → container membership (single rule, derived once)

- **Decision**: The five surface types map to containers by one rule applied at add-time: conformable → append to the adjacent conformable series if one is at the insert position, else a new series; erosional/onlap → new single-surface series with that relation; vein/intrusion → new cross-cutting container. A singleton series renders flat; multi-surface series render nested. The user never names a "container."
- **Rationale**: Container membership is *implied* by type + position, not a free choice — declaring it separately would be a second, drift-prone source of truth (Constitution III). Deciding it once at add-time and presenting singletons flat hides the two-level model exactly as the brief's §1 decision 1 specifies, while the underlying model is unchanged.
- **Alternatives considered**: Letting the user pick a container explicitly — rejected: re-introduces the abstraction the brief is deliberately hiding and creates a declarable redundancy.

## R5 — Dual-table data entry: hand entry + clipboard paste, no file import

- **Decision**: A modal with two tables — Points (X,Y,Z, required) and Orientations (X,Y,Z,Dip,DipDir, optional). Hand entry per cell with Tab/Enter navigation (Enter on the last row adds a row); clipboard paste of TSV/CSV mapped to columns in order (3 cols points, 5 cols orientations). Rows with the wrong column count are flagged, not dropped or truncated.
- **Rationale**: Paste is the bulk-entry path for a demo (brief §5.2); platform Clipboard/`paste` events and a whitespace/comma split cover it with no dependency (Constitution VI). Flagging bad rows rather than silently dropping them honors Constitution IV (no silent coverage bounds). The modal commits to the project only (cheap, local) and never runs the worker — the panel Apply does that.
- **Alternatives considered**: CSV file upload — explicitly out of scope this iteration (spec Non-Goals); the JSON project file is the only file path.

## R6 — Orientations as dip/dip-direction → pole, converted in one place

- **Decision**: Keep raw `dip`/`dipDir` in `OrientationRow` (so the table round-trips what the user typed); convert to the ABI's outward pole unit vector **only** inside `toSeriesJson`, via a single `dipDirToPole(dipDeg, dipDirDeg)` helper transcribed sign-for-sign from brief §5.3, with the axis convention (X=East, Y=North, Z=Up; dip-dir clockwise from +Y) documented in a code comment at the conversion site. A tooltip states the convention to the user.
- **Rationale**: This is the one bit of geology in the UI and the one place a wrong sign silently yields an upside-down field (brief §5.3, Constitution II). One conversion site = one place to get right and one place to test. Keeping raw values means reopening the editor shows what the user entered, not a lossy re-derivation.
- **Alternatives considered**: Storing only the pole — rejected: lossy on re-edit and hides the convention from the user. Converting at entry time — rejected: scatters the conversion and the round-trip loss across the table code.

## R7 — Reorder: wire the existing, already-correct resolver

- **Decision**: Wire `dragResolver.ts`'s output: `onReorderContainers(from,to)` reorders `project.elements` (chronology) and sets dirty; `onReorderWithin(elementId,from,to)` reorders a series' `interfaces` and sets dirty; `onMoveSurface` stays a **declined** action with a user-facing toast (delete-and-re-add workaround), not a silent noop.
- **Rationale**: The resolver and `@dnd-kit` affordances already exist and are correct (confirmed during planning; `resolveDrop` returns a typed `DropAction`). The current code passes `noop` for these three callbacks — the work is wiring, not building (Constitution VI). Reorder now changes geometry/chronology, so it joins the dirty/Apply flow (replacing the old immediate-round-trip expectation, per brief §6).
- **Alternatives considered**: Implementing cross-container move now — out of scope (spec Non-Goals); the seam is the existing `onMoveSurface` callback, left as a documented decline.

## R8 — Persistence: localStorage autosave + JSON project file

- **Decision**: Debounced (~500 ms) `localStorage` autosave of the whole `Project` on committed edits, keyed by project name; restore on reload. Export downloads `<name>.geoproj.json`; import (file input) replaces the current project with a confirm if dirty.
- **Rationale**: localStorage is the zero-dependency durable store for a single-user testbed; the JSON file is the portable "save file." Both are platform APIs (Constitution VI). Keying by name and confirming-on-dirty-import match the brief §2.2.
- **Alternatives considered**: A backend / IndexedDB — rejected: over-scoped for a single-project testbed (Constitution VI).

## R9 — OBJ export from the already-held meshes

- **Decision**: `exportObj.ts` serializes the `SurfacePayload[]` already in `App` state (positions/normals/indices per surface) into one OBJ: `v`/`vn` from positions/normals, `f` with `//` normal refs, accumulating the vertex-index offset across surfaces, grouped per surface (`g series{si}_block{b}_iface{k}`). Enabled only when a fit produced surfaces and `!dirty` (export what is rendered). Materials (`.mtl`) optional in v1 — a logged coverage bound.
- **Rationale**: The meshes are already in memory; OBJ is a trivial text format requiring no dependency and no worker round-trip (Constitution VI). Gating on `!dirty` guarantees the export matches the rendered geometry (spec FR-022). Deferring materials is logged, not silent (Constitution IV).
- **Alternatives considered**: glTF / a mesh-export library — rejected: heavier and unnecessary; OBJ is the portable demo artifact the brief asks for.

## R10 — Author-time validation vs. core-derived validation (no drift)

- **Decision**: `validate(project): ProjectValidation` computes **structural** preconditions in the UI that block the fit from running at all — zero interfaces in a series, zero points on a surface, a cross-cutting body missing a required part (hard-invalid, blocks Apply with a reason); plus soft advisories (<3 points, no orientation anywhere). The existing **core-derived** per-container validation (`CoreUpdate.derived.validation`) is preserved unchanged and remains the authority on the *fitted* result.
- **Rationale**: These two validations answer different questions and must not be confused (Constitution III): author-time validation gates whether a fit can even be attempted (the core can't tell you a surface has zero points — there'd be nothing to fit); core validation judges the geology of a successful fit. Keeping author-time validation strictly structural prevents it from re-encoding any core geological rule. The plan states this boundary so the two cannot silently drift.
- **Alternatives considered**: Pushing all validation into the core — rejected: a zero-point surface can't be sent to a fit to be told it's invalid; the precondition check must live before the fit. Duplicating geological validation in the UI — rejected outright (Constitution III).

## R11 — Testing approach: Playwright behavior gate + the no-real-drag pattern

- **Decision**: Five new Playwright specs (brief §10) cover each new behavioral acceptance criterion against the running app. For reorder, drive the wired `onReorderContainers` via a test hook / assert on the resolver + a programmatic reorder (real `@dnd-kit` drag is flaky), consistent with the existing `drag-affordance.spec`. Assert on DOM elements (scrim test-id, download blob content), never pixels. `tsc -b` + `vite build` remain the compile gate.
- **Rationale**: Constitution I makes the Playwright functional test the behavior gate of record for `web*/`; SC-008 binds every new criterion to one. Mirroring the existing repo's no-real-drag pattern keeps the reorder test deterministic, and the substitution is documented in the test (Constitution I/IV). **If the environment cannot run Playwright this pass, that is logged as an un-closed gap, never reported as verified.**
- **Alternatives considered**: Manual verification — explicitly insufficient under Constitution I (a smoke check, never the gate).
