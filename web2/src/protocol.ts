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
}

export type Command = RunCommand;

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

/** A meshed contact surface, streamed as it is produced (rendered progressively). The
 *  typed arrays are TRANSFERRED (zero-copy) — do not read them on the worker after post. */
export interface SurfaceEvent {
  type: "surface";
  seriesIndex: number;
  /** Which fault block this sheet belongs to (0-based). A faulted contact is streamed as
   *  one SurfaceEvent per block, each cut flush at the fault — matching the web/ harness. */
  block: number;
  interfaceIndex: number;
  interfaceId: number;
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
  | FitDoneEvent
  | SurfaceEvent
  | DepthRowEvent
  | EvalTickEvent
  | DoneEvent
  | ErrorEvent;
