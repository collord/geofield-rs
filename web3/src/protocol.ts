// Typed message contract between the UI (main thread) and the solver worker.
//
// Adapted from web/src/protocol.ts to the TWO-AXIS model (feature 001): each series
// carries `unboundedAbove` (Axis B) separately from `relation` (Axis A, narrowed to
// 0=Erode 1=Onlap 2=Conformable — the legacy 3=Basement code is retired). The worker
// streams back a parsed `CoreUpdate` (declared state + derived view) alongside meshes.
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import type { CoreUpdate } from "./HierarchyPanel/dto";

/** One series, flattened for transfer to the worker. Mirrors the `WasmStack.add_series`
 *  ABI (D1): `relation` is the Axis-A code, `unboundedAbove` is Axis B. */
export interface SeriesJson {
  name: string;
  relation: number; // 0=Erode 1=Onlap 2=Conformable (ignored when unboundedAbove)
  unboundedAbove: boolean; // Axis B — true makes this the floor (StackedSeries::floor)
  sp_xyz: number[]; // 3 * nPoints
  sp_if: number[]; // nPoints, interface id within the series
  ori_xyz: number[]; // 3 * nOrientations
  ori_pole: number[]; // 3 * nOrientations
  fault_point: number[]; // [x,y,z] on the plane
  fault_normal: number[]; // outward normal; [0,0,0] = no fault
  /** Cross-cutting kind (contract D2). Absent ⇒ a stratigraphic series. A vein carries
   *  its foot-wall/hanging-wall interface indices; an intrusion its envelope indices
   *  (equal ⇒ single envelope). When set, `relation`/`unboundedAbove` are ignored. */
  crossCut?:
    | { kind: "vein"; footWall: number; hangingWall: number }
    | { kind: "intrusion"; envelopeInner: number; envelopeOuter: number };
  /** The singleton topography surface (feature 003 §2). When `true` the worker calls
   *  `add_topography` (pinned youngest, caps to Air); `relation`/`unboundedAbove`/`crossCut`
   *  are ignored. Its single interface is the ground. */
  topography?: boolean;
}

/** Open contact sheets vs. closed lithology-unit solids (the web/ render modes). */
export type RenderMode = "contact" | "solid";

// --- Main → worker ---------------------------------------------------------

export interface RunCommand {
  type: "run";
  series: SeriesJson[]; // youngest first (add_series call order)
  extent: [number, number, number, number, number, number];
  maxDepth: number;
  renderMode: RenderMode;
  /** Solid-mode lithology assignment, keyed by `${seriesIndex}:${unitIndex}` (the worker
   *  works in indices, not element ids). The UI compiles its `(elementId, unitIndex)`
   *  assignment to this index form alongside `toSeriesJson`. Absent ⇒ solids untagged
   *  (default Unknown colour applied UI-side). (feature 003 §5.3) */
  assignment?: Record<string, string>;
  /** Weld each cross-cutting wall's T-junctions against the contacts it cuts (the surface-mode
   *  sliver fix, `crate::weld`). Defaults to `true` when omitted. The UI passes `false` to A/B the
   *  old independent-mesh behavior. */
  weldTJunctions?: boolean;
}

export type Command = RunCommand;

// --- Worker → main (streamed status + results) -----------------------------

export interface StatusMessage {
  type: "status";
  phase: string;
}

/** One meshed surface (contact mode) or unit solid (solid mode) for the viewer. */
export interface SurfacePayload {
  series: number;
  block: number;
  interface: number;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  /** Solid mode only (feature 003): the lithology id this unit solid is assigned, for the
   *  viewer's colour-by-lithology + OBJ-by-lithology grouping. Absent in contact mode (and
   *  for Air, which is never emitted as a solid). */
  lithologyId?: string;
  /** Solid mode only: the unit index within the series, for diagnostics/grouping. */
  unit?: number;
}

/** The fit/mesh completed: the meshes for the viewer + the core's hierarchy round-trip
 *  payload (declared state + derived view) for the panel. */
export interface DoneMessage {
  type: "done";
  surfaces: SurfacePayload[];
  update: CoreUpdate; // parsed from WasmStack.core_update_json()
}

export interface ErrorMessage {
  type: "error";
  message: string;
}

export type WorkerMessage = StatusMessage | DoneMessage | ErrorMessage;
