// The typed message contract between the UI (main thread) and the editor's solver worker.
// Scoped down from web2/src/protocol.ts to what the interactive editor needs: a `run`
// command that carries the full editable stack, a `remesh` for instant contact↔solid
// flips, and a streamed result (phase + surfaces + done/error). No bench-only bits
// (depth sweep, full baseline, eval ticker).
//
// Cancellation is by hard terminate: the main thread calls worker.terminate() and
// respawns. A single fit/mesh is one uninterruptible synchronous wasm call, so there is
// no cooperative cancel — killing the worker is the only way to stop mid-fit, and it is
// always instant. This also supersedes an in-flight run when the user edits again.

/** One series, flattened for transfer to the worker. Built from the editable tables +
 *  the series list. Unlike web2 (one shared fault), each series carries its OWN fault —
 *  a degenerate normal ([0,0,0]) means "no fault on this series". */
export interface SeriesJson {
  name: string;
  relation: number; // 0=Erode 1=Onlap 2=Conformable 3=Basement
  sp_xyz: number[]; // 3 * nPoints
  sp_if: number[]; // nPoints, interface id within the series
  ori_xyz: number[]; // 3 * nOrientations
  ori_pole: number[]; // 3 * nOrientations, gradient (pole) vectors
  fault_point: number[]; // [x,y,z] on the plane
  fault_normal: number[]; // outward normal; [0,0,0] = no fault
}

/** How surfaces are meshed for the viewer:
 *  - "contact" — open contact sheets (one per interface), the default;
 *  - "solid"   — watertight CLOSED lithology-unit slabs (one per unit), which daylight
 *                against a younger erosion surface instead of being trimmed back. */
export type RenderMode = "contact" | "solid";

// ---------------------------------------------------------------------------
// Main -> worker
// ---------------------------------------------------------------------------

export interface RunCommand {
  type: "run";
  series: SeriesJson[]; // youngest first (WasmStack.add_series call order)
  extent: [number, number, number, number, number, number];
  /** Octree max depth (finest cell = extent / 2^depth). */
  maxDepth: number;
  /** Which surface representation to mesh for the viewer. */
  renderMode: RenderMode;
}

/** Re-mesh the ALREADY-FITTED stack in a new render mode, without re-fitting. Lets the UI
 *  flip contact↔solid instantly (a refit re-runs the cokriging solve). No-op if nothing is
 *  fitted yet. */
export interface RemeshCommand {
  type: "remesh";
  renderMode: RenderMode;
  maxDepth: number;
}

export type Command = RunCommand | RemeshCommand;

// ---------------------------------------------------------------------------
// Worker -> main (streamed status + results)
// ---------------------------------------------------------------------------

/** A coarse phase marker for the headline status line. */
export interface PhaseEvent {
  type: "phase";
  phase: "loading-wasm" | "fitting" | "meshing" | "done";
  label: string;
}

/** Clear all viewer surfaces — sent at the start of a surface stream so the scene starts
 *  empty (a remesh, or the next run, replaces what's there). */
export interface SurfacesClearedEvent {
  type: "surfaces-cleared";
}

/** A meshed surface, streamed as it is produced (rendered progressively). The typed arrays
 *  are TRANSFERRED (zero-copy) — do not read them on the worker after post. In "contact"
 *  mode this is one open sheet per interface; in "solid" mode one closed slab per unit. */
export interface SurfaceEvent {
  type: "surface";
  /** "contact" = open sheet (keyed by interface); "solid" = closed unit slab. */
  kind: RenderMode;
  /** Which series (0 = youngest) this surface belongs to. */
  seriesIndex: number;
  /** The owning series' relation (0=Erode 1=Onlap 2=Conformable 3=Basement). */
  relation: number;
  /** Which fault block this surface belongs to (0-based). A faulted surface is streamed as
   *  one event per block, each cut flush at the fault. */
  block: number;
  /** Interface index within the series (contact mode) or the unit's lower-contact index
   *  (solid mode); -1 for a unit with no lower contact (the bottom slab). */
  interfaceIndex: number;
  /** The interface's formation name within the series (for the object-tree label). */
  formation: string;
  /** Lithology unit index within the block, for solid mode (-1 in contact mode). */
  unit: number;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
  triangleCount: number;
}

/** Terminal: the whole run (or remesh) finished. */
export interface DoneEvent {
  type: "done";
  fitMs: number;
  surfaceCount: number;
  /** Number of fault blocks summed across series (for the status line). */
  totalBlocks: number;
  /** A non-fatal note (e.g. a fault stranded a block so nothing meshed). */
  warning: string | null;
}

/** Terminal: an error was thrown in the worker (a bad fit / degenerate input). */
export interface ErrorEvent {
  type: "error";
  message: string;
}

export type WorkerEvent =
  | PhaseEvent
  | SurfacesClearedEvent
  | SurfaceEvent
  | DoneEvent
  | ErrorEvent;
