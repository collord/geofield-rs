# Contract — Catalog & Render seam (UI)

**Feature**: `003-topography-lithology-solids` · **Phase 1** · web3 testbed only (no wasm crossing)

This is the **UI-side** contract: the lithology catalog, the `(element, unit) → lithology` assignment,
the render-mode toggle, and lithology colouring/grouping. Lithology **never crosses the wasm boundary**
(Principle III); the core supplies only unit *coordinates* (`DerivedView.units`, see
[engine-abi.md](./engine-abi.md) C3).

---

## D1. LithologyCatalog model

**Lives in**: `web3/src/HierarchyPanel/project.ts`, a field on the `Project` document
([project.ts:75](../../../web3/src/HierarchyPanel/project.ts#L75)).

```ts
type LithologyId = string;
interface Lithology { id: LithologyId; name: string; color: string; kind: "air" | "unknown" | "inferred" | "user"; }
interface LithologyCatalog {
  lithologies: Lithology[];                  // includes fixed Air + Unknown
  assignment: Record<string, LithologyId>;   // key = `${elementId}:${unitIndex}`
}
```

**Contract guarantees**:
1. A fresh catalog contains exactly the fixed **Air** (`kind:"air"`, transparent, non-deletable,
   non-renameable) and **Unknown** (`kind:"unknown"`, neutral grey, non-deletable) members.
2. Every below-topography unit defaults to **Unknown** until assigned; every above-topography volume
   is **Air**. There is no unit without a lithology (FR-020, SC-004).
3. A dangling `assignment` value (its lithology was removed) resolves to **Unknown** at read time —
   never an error.

---

## D2. Inferred-lithology sync (body lifecycle)

**Lives in**: `web3/src/HierarchyPanel/projectOps.ts` (catalog ops invoked on body CRUD).

**Contract guarantees** (the inferred provenance is managed *via the body*, never directly):
1. Adding a vein/intrusion named "X" **creates** an inferred `Lithology { name:"X", kind:"inferred" }`
   and **auto-assigns** it to that body's interior unit.
2. Renaming the body to "Y" **re-syncs** the inferred lithology's name to "Y".
3. Deleting the body **removes** the inferred lithology; assignments pointing at it fall back to
   Unknown (D1.3).
4. Inferred entries are **not** user-deletable in the catalog UI (they go with their body); their
   colour **is** user-editable (recolour allowed).

---

## D3. User-lithology CRUD + the unit dropdown

**Lives in**: `web3/src/HierarchyPanel/LithologyCatalog.tsx` (new) + unit rows in `HierarchyPanel.tsx`.

**Contract guarantees**:
1. The user can **add** a lithology (name + colour, `kind:"user"`); it appears in **every** unit row's
   dropdown immediately.
2. The user can **rename/recolour** user and inferred entries (Air/Unknown are fixed; renaming an
   inferred entry is done via its body — D2.2).
3. The user can **delete** user entries; deleting one reverts its assignments to Unknown (D1.3).
4. Each unit row's dropdown lists **all** catalog entries; selecting one writes
   `assignment["${elementId}:${unitIndex}"] = lithologyId`.
5. **Many-to-one** is allowed: two distinct `(element, unit)` keys may select the same `LithologyId`
   (the entire correlation mechanism — no auto-detection; FR-019, spec §8).

---

## D4. Persistence / round-trip

**Reuses** the existing project JSON export/import + localStorage autosave
([project.ts:234-287](../../../web3/src/HierarchyPanel/project.ts#L234)) — the `LithologyCatalog` is a
field on `Project`, so it rides along.

**Contract guarantees**:
1. Exporting a project then re-importing it restores **100%** of lithology names, colours, and
   per-unit assignments (FR-021, SC-007).
2. No separate catalog file or storage key is introduced (live + project-JSON only).
3. Inferred entries are re-derived/re-synced from the bodies on import (a re-import re-runs D2.1 for
   each body and reconciles with the serialized catalog).

---

## D5. Render-mode toggle — re-mesh without re-fit

**Lives in**: `web3/src/main.tsx` (toggle + state), `web3/src/protocol.ts` (`RenderMode` already
present at [protocol.ts:33](../../../web3/src/protocol.ts#L33), carried in `RunCommand`),
`web3/src/worker.ts` (mode-aware mesh collection).

**Contract guarantees**:
1. A **Surfaces ⇄ Solids** toggle in the project bar switches `RenderMode` between `"contact"` and
   `"solid"`.
2. **Surfaces** (default): open contact sheets coloured **by series** (today's behaviour, unchanged).
3. **Solids**: closed watertight unit solids coloured **by lithology**, **Air omitted**.
4. Toggling **does not re-fit** — it re-runs **only mesh collection** against the already-fitted
   stack (FR-025, SC-006). *(Fallback per the brief: if a worker constraint forces a refit, treat the
   toggle as a normal Apply and **log that coverage bound** — Principle IV — rather than silently
   refitting.)*
5. In solid mode the worker tags each emitted `SurfacePayload` with its `lithologyId` (UI-side lookup
   of `assignment["${elementId}:${unitIndex}"]`); Air payloads are **not** emitted.

---

## D6. Viewer colouring + OBJ grouping

**Lives in**: `web3/src/Viewer.tsx` (colour), `web3/src/exportObj.ts` (grouping).

**Contract guarantees**:
1. `Viewer` accepts `colorBy: "series" | "lithology"`. In `"series"` mode it colours by
   `series % palette` (today's behaviour, [Viewer.tsx:149](../../../web3/src/Viewer.tsx#L149),
   unchanged). In `"lithology"` mode it colours each solid from `catalog.lithologies` by the payload's
   `lithologyId`, and **skips Air** (no Air payloads exist to draw — D5.5).
2. OBJ export ([exportObj.ts:21](../../../web3/src/exportObj.ts#L21)) groups by **lithology** in solid
   mode (group name = lithology), and keeps the series/block/interface grouping in surface mode. No
   Air group appears (SC-008).

---

## Behavioural gate map (Playwright)

Per Principle I, each UI guarantee is pinned by a functional test driving the running app
([web3/e2e/](../../../web3/e2e), [playwright.config.ts](../../../web3/playwright.config.ts)):

| Spec (new) | Asserts |
|------------|---------|
| `topography.spec.ts` | add topography → above-ground Air + older clipped (C1/C2 visible); 2nd topography disabled |
| `unit-ladder.spec.ts` | N contacts → N+1 unit rows; rename unit not contact; "fills upward" control absent (C3 + D3) |
| `lithology-catalog.spec.ts` | Air/Unknown fixed; inferred sync on body add/rename/delete; user CRUD; many-to-one; **export→import round-trip** (D1–D4) |
| `solids-toggle.spec.ts` | toggle → solids coloured by lithology, Air omitted, **no refit** (assert no fit triggered), OBJ grouped by lithology (D5/D6) |

`tsc -b` + `vite build` are the compile gate; the four specs are the behaviour gate; both run before
"done." If the environment cannot run Playwright this pass, that is a **logged, un-closed gap**
(Principle IV), never a silent "verified."
