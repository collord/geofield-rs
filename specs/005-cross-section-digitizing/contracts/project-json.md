# Contract — `.geoproj.json` Additive Schema (005)

Extensions to the version-1 project file (`exportProjectJson`/`importProjectJson`,
`web3/src/HierarchyPanel/project.ts:531,547`) and the localStorage autosave (same shape). **All new
fields are optional**; version stays `1`. A pre-005 file loads unchanged; a 005 file loaded by a
pre-005 build silently drops the new fields (accepted for the prototype — no forward-compat claim).

## Project

```ts
interface Project {
  // ...existing fields unchanged (extent, elements, unitNames, catalog, view)...

  /** NEW — section sets (view/authoring constructs, not geology). Absent ⇒ none. */
  sections?: SectionSet[];
}

interface SectionSet {
  id: string;
  name: string;
  a: [number, number];        // baseline start, world XY (plan view)
  b: [number, number];        // baseline end; |b − a| > 0 guaranteed by the creation gesture
  spacing: number;            // > 0 when count > 1; plane k offset = offset + k·spacing along ŝ
  count: number;              // integer ≥ 1; count = 1 ⇒ a single section
  offset?: number;            // default 0; start offset of plane 0 from the baseline along ŝ
  showInScene?: boolean;      // default false; the US-SECT-5 translucent quad toggle
}
```

## Saved view (US-PROJ-4 extension)

```ts
interface ProjectView {
  // ...existing camera/target/up/fov/render-toggle fields unchanged...

  /** NEW — the open section, restored on import (FR-010 / SC-005). Absent ⇒ 3D view. */
  section?: { setId: string; index: number };  // index clamped to [0, count) on load
}
```

## InterfaceData (per surface)

```ts
interface InterfaceData {
  // ...existing: id, name, points, orientations, flipped?...

  /** NEW — digitized polylines (geometry: participates in the fit). Absent ⇒ none. */
  digitized?: DigitizedLine[];
}

interface DigitizedLine {
  id: string;
  /** Plane snapshot at digitize time — the line is self-sufficient; sections may be deleted. */
  frame: { origin: [number, number, number]; s: [number, number, number] };  // ŝ unit, horizontal
  /** Ordered world-space vertices; ORDER IS LOAD-BEARING (drawn direction ⇒ younging).
   *  Import guard (hard error): every v must satisfy |dot(v − origin, ŝ)| ≤ εplane. */
  vertices: [number, number, number][];
  /** Soft UI grouping ref; may dangle after section deletion — never used for geometry. */
  sectionRef?: { setId: string; index: number };
}
```

## Import-time guards (extends `importProjectJson` validation)

| Condition | Handling |
|---|---|
| `SectionSet` with `count < 1`, or `count > 1` with `spacing ≤ 0` | reject field with readable error (file names the set) |
| `view.section.setId` unknown / `index` out of range | drop / clamp (view-state is best-effort) |
| `DigitizedLine.frame.s` non-unit or non-horizontal beyond ε | renormalize; reject if degenerate |
| digitized vertex off its plane beyond `εplane` | **hard validation error** (FR-019) — surfaces in `validate()`, blocks Apply, does not block load |
| `vertices.length < 2` | loads, then hard validation error (same as authoring state) |

## Compile seam (unchanged ABI)

`toSeriesJson` remains the single seam and its output type `SeriesJson` (`protocol.ts:14`) is
**unchanged**. Digitized data enters as additional `sp_xyz`/`sp_if`/`ori_xyz`/`ori_pole` entries per
[younging-derivation.md](younging-derivation.md) §2–3. No worker/protocol/wasm change.

## E2E hook contract (test surface, `window.__*` — dev/preview builds)

*(As implemented — the design sketch's `surfaceId` params became explicit
`(elementId, interfaceId)` pairs, and payloads are identified by worker series index.)*

| Hook | Shape | Purpose |
|---|---|---|
| `__sections()` | `→ SectionSet[]` | assert creation/serial params without canvas scraping |
| `__activeSection()` | `→ { setId, index } \| null` | active-slice assertions (persistence) |
| `__addSectionSet(partial)` | `→ setId` | programmatic set creation (gesture path covered once via canvas clicks) |
| `__openSection(setId, index)` | opens the section view | drive the view without panel clicks |
| `__sectionTraces(setId, index)` | `→ { series, block, iface, polylines: [x,y,z][][] }[]` | the land-on-line loft assertion + trace-presence checks |
| `__digitizeLine(elementId \| null, interfaceId \| null, setId, index, verts: [du,dw][])` | commits a line as if drawn (null ids ⇒ creates a new digitized surface); `→ { elementId, interfaceId, lineId }` | programmatic digitizing for loft/validation specs |
| `__project()` | `→ Project` | data-level assertions (vertex order, flip bit, round-trips) |
| `__exportState()` | `→ Project` (the exact Export object incl. `view.section`) | persistence round-trip without a file download |

Existing hooks (`__loadProject`, `__getView`, `__surfaces`, `__overlayCounts`) unchanged.
Reliability note (the stale-preview gotcha): after Apply, wait for the **apply-bar to unmount**
(dirty cleared on success), not for "status: done" (still satisfied by the previous fit's text);
for non-dirty refits (relation change, render toggle) wait for `__statusLog` growth instead.
