# Phase 1 Data Model: Enhanced Authoring

**Feature**: 002-enhanced-authoring | **Date**: 2026-06-19

The authoring data model is **UI-only**: it lives in the browser, never crosses the wasm boundary, and compiles down to the existing `SeriesJson[]` the worker already consumes. It is the new source of truth for *authoring*; the **core remains the source of truth for everything geological** (effective relations, cross-cut sets, fitted-result validation), read back through the unchanged `CoreUpdate.derived`.

All types below live in the new `web3/src/HierarchyPanel/project.ts`.

---

## Entities

### Project

The whole editable document. Persisted to localStorage and to the `.geoproj.json` file.

| Field | Type | Notes |
|-------|------|-------|
| `version` | `1` | schema version literal; bump if the shape changes |
| `name` | `string` | user-editable; localStorage autosave key |
| `extent` | `[number,number,number,number,number,number]` | `[x0,x1,y0,y1,z0,z1]`; passed straight to `RunCommand.extent` |
| `elements` | `ProjectElement[]` | **youngest first** (matches `add_*` call order / `SeriesJson[]` order) |

### ProjectElement (discriminated union on `kind`)

One authoring container. A `series` with exactly one interface is **presented flat** (as a plain surface); with more than one it is presented **nested**. Vein/intrusion always present their named constituent surfaces.

**`kind: "series"`** — one co-kriged field (1..n interfaces):

| Field | Type | Notes |
|-------|------|-------|
| `id` | `string` | stable client id |
| `name` | `string` | display name |
| `relation` | `"erode" \| "onlap" \| "conformable"` | Axis-A seam; the one free declared bit per boundary |
| `unboundedAbove` | `boolean` | Axis B; `true` ⇒ the floor |
| `interfaces` | `InterfaceData[]` | 1..n; ordered by ascending isovalue (within-series reorder reorders this array) |

**`kind: "vein"`** — tabular cross-cutting body:

| Field | Type | Notes |
|-------|------|-------|
| `id` | `string` | stable client id |
| `name` | `string` | display name |
| `footWall` | `InterfaceData` | interface id 0 in the emitted `SeriesJson` |
| `hangingWall` | `InterfaceData` | interface id 1 |

**`kind: "intrusion"`** — volumetric cross-cutting body:

| Field | Type | Notes |
|-------|------|-------|
| `id` | `string` | stable client id |
| `name` | `string` | display name |
| `envelopeInner` | `InterfaceData` | interface id 0 |
| `envelopeOuter` | `InterfaceData` | interface id 1; **equal to inner ⇒ a single envelope** |

### InterfaceData

One surface's interpolation data — the unit the user adds and edits.

| Field | Type | Notes |
|-------|------|-------|
| `id` | `string` | stable client id |
| `name` | `string` | display name |
| `points` | `PointRow[]` | XYZ contact points; **≥1 to be valid, ≥3 to fit a plane well** |
| `orientations` | `OrientationRow[]` | optional; empty is valid |

### PointRow

| Field | Type |
|-------|------|
| `x` / `y` / `z` | `number` |

### OrientationRow

Raw, as the user typed it — converted to a pole **only** at compile time.

| Field | Type | Notes |
|-------|------|-------|
| `x` / `y` / `z` | `number` | location of the reading |
| `dip` | `number` | degrees from horizontal |
| `dipDir` | `number` | degrees clockwise from North (+Y) |

### ProjectValidation (computed, not stored)

Returned by `validate(project)`; surfaced per surface/container and aggregated on the Apply control. Distinct from `CoreUpdate.derived.validation` (which judges the *fitted* result — see [research.md R10](./research.md)).

| Field | Type | Notes |
|-------|------|-------|
| `hardErrors` | `{ elementId: string; interfaceId?: string; reason: string }[]` | blocks Apply |
| `softWarnings` | `{ elementId: string; interfaceId?: string; reason: string }[]` | advisory only |
| `canApply` | `boolean` | `hardErrors.length === 0` |

---

## Relationships

```
Project 1───* ProjectElement
                 ├─ series      1───1..n InterfaceData
                 ├─ vein        1───2     InterfaceData (footWall, hangingWall)
                 └─ intrusion   1───2     InterfaceData (envelopeInner, envelopeOuter)
InterfaceData    1───* PointRow
InterfaceData    1───* OrientationRow
```

`elements` order **is** chronology (youngest first). Reordering `elements` is a chronology edit; reordering a series' `interfaces` is an isovalue-order edit within one field.

---

## Validation rules

From spec FR-017. Computed by `validate(project)` on the in-memory project, **before** any fit.

**Hard-invalid (blocks panel Apply, reason shown):**
- a `series` with zero `interfaces`;
- any `InterfaceData` with zero `points`;
- a `vein`/`intrusion` missing one of its required interfaces.

**Soft (advisory, doesn't block):**
- an `InterfaceData` with `points.length < 3` → "needs ≥3 points to fit well";
- a `series` with no orientation anywhere → "no orientation — fit may be ambiguous".

The existing core-derived per-container badges (`CoreUpdate.derived.validation`) are preserved unchanged and remain authoritative for the fitted result.

---

## State transitions (App-level, `main.tsx`)

```
                 add / delete surface
                 modal commit
                 reorder containers / within-series
   ┌────────────────────────────────────────────────┐
   │                                                  ▼
[clean] ──cheap edit (relation / unbounded)──▶ [clean]   (immediate runStack; dirty stays false)
   ▲                                              │
   │                                              ▼
   │                                          [dirty]  (scrim shown; Apply/Cancel bar shown)
   │                                          /     \
   │                            Apply (fit ok)       Cancel
   └──────────────────────────────┘                 │ revert project to lastApplied
                                                     └───────────────▶ [clean]
```

- **Geometry-affecting edit** → mutate `project`, `setDirty(true)`; **do not** run the worker.
- **Apply** → `setApplying(true)` → `runStack(toSeriesJson(project))` → on `done`: snapshot `lastApplied = project`, `setDirty(false)`, `setApplying(false)`, clear scrim.
- **Cancel** → `setProject(lastApplied)`, `setDirty(false)`, clear scrim.
- **Cheap declared-state edit** → mutate `project` *and* immediately `runStack`; `dirty` stays false (never stale).

New App state (per brief §3): `project: Project`, `dirty: boolean`, `applying: boolean`, and a `lastApplied: Project` ref. Existing `surfaces`, `status`, `extent`, `state` (HierarchyState), and the worker ref are retained.

---

## Compile mapping → `SeriesJson[]` (`toSeriesJson`)

Pure function, the single seam to the unchanged fit path. For each `element` in order:

- **series** → one `SeriesJson`: `relation` as code (`erode`→0, `onlap`→1, `conformable`→2), `unboundedAbove`, `sp_xyz`/`sp_if` flattened across its interfaces (interface index → `sp_if`), `ori_xyz`/`ori_pole` concatenated (poles via `dipDirToPole`), `fault_point`/`fault_normal` = no-fault defaults, `crossCut` absent.
- **vein** → `SeriesJson` with `crossCut: { kind:"vein", footWall:0, hangingWall:1 }`, points/orientations from the two interfaces (ids 0/1). `relation`/`unboundedAbove` are ignored by the core for a cross-cutting body but emitted with valid defaults.
- **intrusion** → `crossCut: { kind:"intrusion", envelopeInner:0, envelopeOuter:1 }`, equal inner/outer ⇒ single envelope.

The full compile + conversion contract (including `dipDirToPole`) is in [contracts/project-model.md](./contracts/project-model.md).
