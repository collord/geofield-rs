// The typed message contract between the UI (main thread) and the solver worker.
//
// Cancellation is by hard terminate: the main thread calls worker.terminate() and
// respawns. A single fit/mesh is one uninterruptible synchronous wasm call, so there is
// no cooperative cancel — killing the worker is the only way to stop mid-fit, and it is
// always instant. The worker therefore needs no cancel handling; it just streams events.

export interface SeriesJson {
  name: string;
  relation: number; // 0=Erode 1=Onlap 2=Conformable 3=Basement
  faulted: boolean;
  sp_xyz: number[];
  sp_if: number[];
  ori_xyz: number[];
  ori_pole: number[];
}

export interface Scenario {
  extent: [number, number, number, number, number, number];
  fault: {
    point: number[];
    normal: number[];
    strike_deg: number;
    dip_deg: number;
    offset_ft: number;
  };
  interface_names: Record<string, string>;
  series: SeriesJson[];
  stats: Record<string, number>;
}

// ---------------------------------------------------------------------------
// Main -> worker
// ---------------------------------------------------------------------------

/** How surfaces are meshed for the viewer:
 *  - "contact" — open contact sheets (one per interface), the default;
 *  - "solid"   — watertight CLOSED lithology-unit slabs (one per unit), which daylight
 *                against the erosion surface instead of being trimmed back. */
export type RenderMode = "contact" | "solid";

export interface RunCommand {
  type: "run";
  scenario: Scenario;
  /** Re-add threshold as a fraction of the field's isovalue range; NaN = keep all. */
  decimateFrac: number;
  /** Whether to fit a FULL (no-decimation) baseline first for the speedup comparison. */
  fullBaseline: boolean;
  /** Octree depths to sweep-time. */
  depths: number[];
  /** Octree depth to mesh the surfaces for the 3D viewer. */
  viewDepth: number;
  /** Which surface representation to mesh for the viewer. */
  renderMode: RenderMode;
}

/** Re-mesh the ALREADY-FITTED stack in a new render mode, without re-fitting. Lets the UI
 *  flip contact↔solid instantly (a refit would be ~minutes). No-op if nothing is fitted. */
export interface RemeshCommand {
  type: "remesh";
  renderMode: RenderMode;
  viewDepth: number;
}

export type Command = RunCommand | RemeshCommand;

// ---------------------------------------------------------------------------
// Worker -> main (streamed status + results)
// ---------------------------------------------------------------------------

/** A coarse phase marker, for the headline status line + progress bar. */
export interface PhaseEvent {
  type: "phase";
  phase: "loading-wasm" | "loading-scenario" | "fitting" | "meshing" | "rendering" | "done";
  /** Human label for the status line. */
  label: string;
  /** 0..1 overall progress, best-effort. */
  progress: number;
}

/** One series' fit completed (the cross-boundary cokriging solve). */
export interface FitDoneEvent {
  type: "fit-done";
  /** "full" baseline pass or the "decimated" pass. */
  pass: "full" | "decimated";
  seriesIndex: number;
  seriesName: string;
  points: number;
  ms: number;
}

/** Clear all viewer surfaces — sent before a remesh stream so the scene starts empty. */
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
  seriesIndex: number;
  /** Which fault block this surface belongs to (0-based). A faulted surface is streamed as
   *  one event per block, each cut flush at the fault — matching the web/ harness. */
  block: number;
  /** Interface index within the block (contact mode) or the unit's lower-contact index
   *  (solid mode); -1 for a unit with no lower contact (the bottom slab). */
  interfaceIndex: number;
  /** Color key: the contact's interface id (contact mode) or the unit's lower-contact id
   *  (solid mode; the series' first id for the bottom slab). */
  interfaceId: number;
  /** Lithology unit index within the block, for solid mode (`-1` in contact mode). */
  unit: number;
  /** The owning series' relation (0=Erode 1=Onlap 2=Conformable 3=Basement). */
  relation: number;
  positions: Float32Array;
  normals: Float32Array;
  indices: Uint32Array;
}

/** One octree-depth timing row from the sweep. */
export interface DepthRowEvent {
  type: "depth-row";
  depth: number;
  meshMs: number;
  tris: number;
  verts: number;
  evals: number;
}

/** Running count of cross-boundary gf_eval calls (the live ticker). */
export interface EvalTickEvent {
  type: "eval-tick";
  count: number;
}

/** Terminal: the whole run finished. */
export interface DoneEvent {
  type: "done";
  fullFitMs: number | null;
  decimatedFitMs: number;
  surfaceCount: number;
  dataPointCount: number;
}

/** Terminal: an error was thrown in the worker. */
export interface ErrorEvent {
  type: "error";
  message: string;
}

export type WorkerEvent =
  | PhaseEvent
  | SurfacesClearedEvent
  | FitDoneEvent
  | SurfaceEvent
  | DepthRowEvent
  | EvalTickEvent
  | DoneEvent
  | ErrorEvent;
