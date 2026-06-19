# Contract: UI Callback & Component Surface

**Feature**: 002-enhanced-authoring | **Date**: 2026-06-19

The authoring UI stays inside the reusable `HierarchyPanel/` boundary (except `exportObj.ts`). This contract pins the props/callbacks that change, are newly wired, or are added — measured against the **current** baseline read during planning.

---

## `HierarchyPanel` props

**Baseline today** (`web3/src/HierarchyPanel/HierarchyPanel.tsx`):

```ts
interface HierarchyPanelProps {
  update: CoreUpdate;
  onReorderContainers: (from: string, to: string) => void;       // passed noop today
  onReorderWithin: (elementId: string, from: number, to: number) => void; // passed noop today
  onMoveSurface: (surfaceElementId: string, surfaceIndex: number, toElementId: string) => void; // passed noop today
  onSetRelation: (elementId: string, rel: RelationDto) => void;  // wired (immediate runStack)
  onSetUnboundedAbove: (elementId: string, v: boolean) => void;  // wired (immediate runStack)
}
```

**Changes this feature**:

| Callback | Baseline | This feature |
|----------|----------|--------------|
| `onReorderContainers` | passed `noop` | **wired**: reorder `project.elements`, `setDirty(true)` |
| `onReorderWithin` | passed `noop` | **wired**: reorder the series' `interfaces`, `setDirty(true)` |
| `onMoveSurface` | passed `noop` | **declined** (not silent): show a toast — "moving a surface between containers isn't supported — delete and re-add"; no project change |
| `onSetRelation` | wired, immediate | **unchanged** (cheap edit, immediate `runStack`, `dirty` stays false) |
| `onSetUnboundedAbove` | wired, immediate | **unchanged** (cheap edit, immediate `runStack`, `dirty` stays false) |

**New props (added)** — entry points for authoring, read by the new "…" menus and add affordances inside the panel:

```ts
  onAddSurfaceClick: () => void;                       // opens the AddSurfaceMenu (type picker)
  onEditSurface: (elementId: string, interfaceId: string) => void;  // opens SurfaceDataModal
  onRenameSurface: (elementId: string, interfaceId: string, name: string) => void;
  onDuplicateSurface: (elementId: string, interfaceId: string) => void;
  onDeleteSurface: (elementId: string, interfaceId: string) => void;
  onAddInterface: (seriesElementId: string) => void;  // multi-interface series "+ interface"
  onRenameElement: (elementId: string, name: string) => void;
  onDeleteElement: (elementId: string) => void;
  validation: ProjectValidation;                       // author-time, surfaced per row + aggregate
```

**Rendering changes** (the panel itself): singleton-series flattening (one-interface series renders as a plain surface row, no header); per-surface `SurfaceMenu` ("…") with Edit data… / Rename… / Duplicate / Delete; series-header "…" with rename/delete/add-interface for multi-interface series; the "+ interface" affordance. **Unchanged**: the seam control, floor marker, derived validation badge, and derived summary — all still read from `CoreUpdate.derived`.

---

## `Viewer` props

**Baseline today** (`web3/src/Viewer.tsx`):

```ts
function Viewer(props: { surfaces: SurfacePayload[]; extent: Extent }): JSX.Element
```

**This feature** adds one prop:

```ts
  stale?: boolean;   // when true, render a translucent grey scrim over the canvas
```

**Contract**: when `stale` is true, a positioned `<div>` overlay (~40% grey, `pointer-events: none`, a stable `data-testid` for the Playwright `stale-scrim.spec`) renders over the canvas mount; camera interaction continues to work. When false/absent, no overlay. The canvas mount is wrapped in a positioned container so the scrim is a sibling overlay — the smallest DOM change that gives a stable test hook. `surfaces`/`extent` behavior is unchanged.

---

## New components (all under `HierarchyPanel/` except noted)

| Component | Role | Key contract |
|-----------|------|--------------|
| `AddSurfaceMenu.tsx` | five-type picker popover | on pick: create the element (one empty interface, or the required pair/envelope) under a default name, insert at the oldest end, `setDirty(true)`, open its `SurfaceDataModal`. |
| `SurfaceDataModal.tsx` | dual-table edit-and-apply modal | reads `InterfaceData` into local state; **modal Apply** commits to the project (sets `dirty`) and closes; **modal Cancel** discards. Never runs the worker. Apply disabled when zero points. |
| `SurfaceMenu.tsx` | per-surface "…" menu | Edit data… / Rename… / Duplicate / Delete; series header gets rename / delete series / add interface. |
| `tables.tsx` | reusable points/orientations table | per-cell numeric entry, Tab/Enter nav (Enter on last row adds a row), per-row delete, clipboard paste (TSV/CSV → columns in order; wrong-width rows flagged, not dropped), row-count/validity summary. |
| `exportObj.ts` (in `web3/src/`) | OBJ serialization | `SurfacePayload[]` → one OBJ string (`v`/`vn`/`f //`, per-surface groups, accumulated index offset); triggers `<project>.obj` download; pure client-side. |

---

## `App` (`main.tsx`) state & bars

New top-level state (per [data-model.md](../data-model.md) state transitions):

```ts
const [project, setProject] = useState<Project>(/* loadProject() ?? blankTemplate() */);
const [dirty, setDirty] = useState(false);
const [applying, setApplying] = useState(false);
const lastApplied = useRef<Project>(project);
```

New UI in the left panel (top→bottom): **Project bar** (name, New / Import / Export, Seed-from-template ▾, Export-mesh→OBJ); **Apply/Cancel bar** (shown only when `dirty`; Apply primary, disabled-with-reason when `!validation.canApply`; Cancel reverts to `lastApplied`); the enhanced `HierarchyPanel`; the existing status line. `Viewer` receives `stale={dirty}`.

**Apply** → `setApplying(true)` → `runStack(toSeriesJson(project))` → on worker `done`: `lastApplied.current = project; setDirty(false); setApplying(false)`. **Cancel** → `setProject(lastApplied.current); setDirty(false)`. The existing hard-terminate-on-new-run cancellation in `runStack` is unchanged.
