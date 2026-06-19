// The worker CLIENT: turn the editable series + rows into a `run` command, drive the solver
// worker (which owns both wasm modules), collect the streamed surfaces, and resolve a
// MeshData the Viewer / ObjectTree consume. The heavy fit+mesh runs in worker.ts off the UI
// thread; this module is the thin async bridge + the surface-shaping conversions that mirror
// src/bin/geofield_cli.rs.

import {
  SurfacePointRow,
  OrientationRow,
  SeriesDef,
  FaultConfig,
  seriesFormations,
} from "./data";
import type { Command, RenderMode, SeriesJson, WorkerEvent } from "./protocol";
import { relationInfo, interfaceColor, cssHex } from "./relations";

export type { RenderMode } from "./protocol";

/** gempy azimuth/dip/polarity → pole (gradient) vector, in degrees.
 *  gx = sin(dip)·sin(az)·pol, gy = sin(dip)·cos(az)·pol, gz = cos(dip)·pol.
 *  Verified against gempy: az=90, dip=0 → [0,0,1]. */
export function azimuthDipToPole(
  azimuthDeg: number,
  dipDeg: number,
  polarity: number,
): [number, number, number] {
  const az = (azimuthDeg * Math.PI) / 180;
  const dip = (dipDeg * Math.PI) / 180;
  return [
    Math.sin(dip) * Math.sin(az) * polarity,
    Math.sin(dip) * Math.cos(az) * polarity,
    Math.cos(dip) * polarity,
  ];
}

/** One dual-contoured contact surface (or closed unit slab), as the Viewer consumes it. */
export interface SurfaceMesh {
  /** Series this surface belongs to (0 = youngest). */
  series: number;
  /** The owning series' relation (0=Erode 1=Onlap 2=Conformable 3=Basement). */
  relation: number;
  /** Interface index within the series (contact) or the unit's lower-contact index (solid);
   *  -1 for a unit with no lower contact (the bottom slab). */
  interfaceIndex: number;
  /** Human label for this surface (the formation name within its series). */
  formation: string;
  /** Fault block this surface belongs to. Each block's copy of a contact is a SEPARATE,
   *  unconnected surface (cut flush at the fault). */
  block: number;
  /** Lithology unit index, for the closed-solid view (a watertight slab). -1 for a contact
   *  sheet. */
  unit: number;
  positions: Float32Array; // 3 * V
  normals: Float32Array; // 3 * V — exact per-vertex normals (field gradient)
  indices: Uint32Array; // 3 * T
  triangleCount: number;
}

export interface MeshData {
  /** Every meshed surface across the stack (per series, block, and interface/unit). */
  surfaces: SurfaceMesh[];
  triangleCount: number; // total across surfaces
  /** Fault blocks summed across series (a coarse "how faulted is this" readout). */
  blockCount: number;
}

/** A stable id for one surface object, used to toggle its visibility from the object tree.
 *  Encodes series + block + (unit | interface) so it survives re-solves and never collides
 *  across series. */
export function surfaceKey(s: SurfaceMesh): string {
  const part = s.unit >= 0 ? `u${s.unit}` : `i${s.interfaceIndex}`;
  return `s${s.series}:b${s.block}:${part}`;
}

/** Human label for a surface in the object tree. */
export function surfaceLabel(s: SurfaceMesh): string {
  if (s.unit >= 0) return `${s.formation} (unit ${s.unit})`;
  return s.formation;
}

/** Render color for one surface: the series' relation family, separated per interface so
 *  contacts within a series stay individually legible. Closed-unit solids reuse their lower
 *  contact's color. */
export function pickColor(surf: SurfaceMesh): number {
  const idx = surf.interfaceIndex >= 0 ? surf.interfaceIndex : 0;
  return interfaceColor(surf.relation, idx);
}

/** "#rrggbb" CSS string for a surface color (object-tree swatch). */
export function pickColorCss(surf: SurfaceMesh): string {
  return cssHex(pickColor(surf));
}

export interface SolveResult {
  mesh: MeshData;
  /** A hard failure (the solve threw / bad input) — nothing was produced. */
  error: string | null;
  /** A non-fatal note (e.g. a fault stranded a block) — a mesh may still exist. */
  warning: string | null;
  /** Fit time reported by the worker (ms). */
  fitMs: number;
}

// ---------------------------------------------------------------------------
// Building the worker `run` command from the editable state.
// ---------------------------------------------------------------------------

/** Turn the flat editable rows + series list into the per-series JSON the worker fits.
 *  Interface ids are assigned per series, in ascending first-seen order of formation
 *  (`seriesFormations`), so each series' contacts are 0,1,2,… in deposition order. */
export function buildSeriesJson(
  series: SeriesDef[],
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
): SeriesJson[] {
  return series.map((def, si) => {
    const formations = seriesFormations(surfacePoints, si);
    const idOf = (formation: string) => Math.max(0, formations.indexOf(formation));

    const sp = surfacePoints.filter((r) => r.series === si);
    const sp_xyz: number[] = [];
    const sp_if: number[] = [];
    for (const r of sp) {
      sp_xyz.push(r.x, r.y, r.z);
      sp_if.push(idOf(r.formation));
    }

    const ori = orientations.filter((r) => r.series === si);
    const ori_xyz: number[] = [];
    const ori_pole: number[] = [];
    for (const r of ori) {
      ori_xyz.push(r.x, r.y, r.z);
      const [gx, gy, gz] = azimuthDipToPole(r.azimuth, r.dip, r.polarity);
      ori_pole.push(gx, gy, gz);
    }

    const fault = def.fault;
    return {
      name: def.name,
      relation: def.relation,
      sp_xyz,
      sp_if,
      ori_xyz,
      ori_pole,
      fault_point: fault ? [...fault.point] : [0, 0, 0],
      fault_normal: fault ? [...fault.normal] : [0, 0, 0],
    };
  });
}

// ---------------------------------------------------------------------------
// The solver-worker driver. One persistent worker; a new solve hard-cancels the prior run
// by terminating + respawning (also the "supersede an in-flight edit" mechanism), matching
// web2's cancellation model. Editor fits are tiny (ms) so the ~1s cold re-init is rare and
// only happens after an actual cancel.
// ---------------------------------------------------------------------------

let worker: Worker | null = null;
/** Generation counter: a stale run's events (after a respawn) are ignored. */
let generation = 0;

function spawnWorker(): Worker {
  return new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
}

/** True once a stack has been fitted in the current worker (so a remesh is possible). */
let haveFit = false;

/** Fit + mesh the stack in the worker and resolve the assembled MeshData. Supersedes any
 *  in-flight solve: terminates the previous worker (instant hard cancel) and starts fresh.
 *  `onSurface` is called for each streamed surface (for progressive rendering); the resolved
 *  result also contains the full list. */
export function solveAndMesh(
  series: SeriesDef[],
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
  extent: [number, number, number, number, number, number],
  maxDepth: number,
  renderMode: RenderMode,
  onSurface?: (s: SurfaceMesh) => void,
): Promise<SolveResult> {
  // Hard-cancel the previous run: terminate + respawn. This is the supersede mechanism.
  worker?.terminate();
  worker = spawnWorker();
  haveFit = false;
  const gen = ++generation;
  const w = worker;

  const cmd: Command = {
    type: "run",
    series: buildSeriesJson(series, surfacePoints, orientations),
    extent,
    maxDepth,
    renderMode,
  };
  return drive(w, gen, cmd, onSurface);
}

/** Re-mesh the already-fitted stack in a new render mode, WITHOUT refitting. Falls back to a
 *  full solve if no fit is live yet. */
export function remesh(
  series: SeriesDef[],
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
  extent: [number, number, number, number, number, number],
  maxDepth: number,
  renderMode: RenderMode,
  onSurface?: (s: SurfaceMesh) => void,
): Promise<SolveResult> {
  if (!worker || !haveFit) {
    return solveAndMesh(series, surfacePoints, orientations, extent, maxDepth, renderMode, onSurface);
  }
  const gen = ++generation;
  const cmd: Command = { type: "remesh", renderMode, maxDepth };
  return drive(worker, gen, cmd, onSurface);
}

/** Wire up the worker's streamed events into a single resolved SolveResult. */
function drive(
  w: Worker,
  gen: number,
  cmd: Command,
  onSurface?: (s: SurfaceMesh) => void,
): Promise<SolveResult> {
  return new Promise<SolveResult>((resolve) => {
    const surfaces: SurfaceMesh[] = [];
    let totalTris = 0;

    w.onmessage = (e: MessageEvent<WorkerEvent>) => {
      if (gen !== generation) return; // superseded by a newer run — ignore
      const ev = e.data;
      switch (ev.type) {
        case "surfaces-cleared":
          surfaces.length = 0;
          totalTris = 0;
          break;
        case "surface": {
          const s: SurfaceMesh = {
            series: ev.seriesIndex,
            relation: ev.relation,
            interfaceIndex: ev.interfaceIndex,
            formation: ev.formation,
            block: ev.block,
            unit: ev.unit,
            positions: ev.positions,
            normals: ev.normals,
            indices: ev.indices,
            triangleCount: ev.triangleCount,
          };
          surfaces.push(s);
          totalTris += s.triangleCount;
          onSurface?.(s);
          break;
        }
        case "done": {
          haveFit = true;
          resolve({
            mesh: { surfaces, triangleCount: totalTris, blockCount: ev.totalBlocks },
            error: null,
            warning: ev.warning,
            fitMs: ev.fitMs,
          });
          break;
        }
        case "error":
          resolve({ mesh: emptyMesh(), error: ev.message, warning: null, fitMs: 0 });
          break;
        // "phase" events are status-only; the App could surface ev.label if desired.
      }
    };

    w.onerror = (e) => {
      if (gen !== generation) return;
      resolve({ mesh: emptyMesh(), error: e.message || "worker error", warning: null, fitMs: 0 });
    };

    w.postMessage(cmd);
  });
}

function emptyMesh(): MeshData {
  return { surfaces: [], triangleCount: 0, blockCount: 0 };
}

// Re-export so callers (App) can render relation glosses without a second import.
export { relationInfo };
export type { FaultConfig, SeriesDef };
