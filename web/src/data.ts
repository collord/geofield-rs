// Editable input rows + the gempy model1 seed data, now SERIES-AWARE.
//
// A model is an ordered STACK of series (youngest → oldest, the order WasmStack fits in).
// Each surface point / orientation belongs to exactly one series (by `series` index) and
// carries a `formation` label that names its interface WITHIN that series. Each series has
// a geological RELATION (Erode/Onlap/Conformable/Basement) and an optional planar fault.
//
// Surface points: X,Y,Z,formation.  Orientations: X,Y,Z,azimuth,dip,polarity,formation —
// gempy's CSV format (validation/fixtures/model1), plus the `series` grouping field.

import { RELATION } from "./relations";

export interface SurfacePointRow {
  x: number;
  y: number;
  z: number;
  /** Interface name within the row's series (interface id is assigned per series, in
   *  ascending first-seen order). */
  formation: string;
  /** Index into the series list (youngest = 0). */
  series: number;
}

export interface OrientationRow {
  x: number;
  y: number;
  z: number;
  azimuth: number;
  dip: number;
  polarity: number;
  formation: string;
  series: number;
}

/** A single planar fault applied to ONE series' fit: a point on the plane + an outward
 *  normal. When present the field solves independently on each side, so that series'
 *  contacts are offset laterally across the plane (a finite fault). */
export interface FaultConfig {
  point: [number, number, number];
  normal: [number, number, number];
}

/** One series in the stack: a name, its relation to older series, and an optional fault.
 *  The series' surface points / orientations live in the row tables, tagged by index. */
export interface SeriesDef {
  name: string;
  relation: number; // RELATION code: 0=Erode 1=Onlap 2=Conformable 3=Basement
  fault: FaultConfig | null;
}

// --- The seed model ----------------------------------------------------------------
//
// jan_models/model1 — horizontal stratigraphy (the validated reference), as ONE
// depositional Basement series with two contacts (rock2 @ z=600, rock1 @ z=400). This
// reproduces the pre-migration single-model view exactly. Add more series in the UI to
// build a real stack (e.g. a younger Erode series that truncates these).

export const SEED_SERIES: SeriesDef[] = [
  { name: "sediments", relation: RELATION.BASEMENT, fault: null },
];

export const SEED_SURFACE_POINTS: SurfacePointRow[] = [
  { x: 100, y: 200, z: 600, formation: "rock2", series: 0 },
  { x: 500, y: 200, z: 600, formation: "rock2", series: 0 },
  { x: 900, y: 200, z: 600, formation: "rock2", series: 0 },
  { x: 100, y: 800, z: 600, formation: "rock2", series: 0 },
  { x: 500, y: 800, z: 600, formation: "rock2", series: 0 },
  { x: 900, y: 800, z: 600, formation: "rock2", series: 0 },
  { x: 100, y: 200, z: 400, formation: "rock1", series: 0 },
  { x: 500, y: 200, z: 400, formation: "rock1", series: 0 },
  { x: 900, y: 200, z: 400, formation: "rock1", series: 0 },
  { x: 100, y: 800, z: 400, formation: "rock1", series: 0 },
  { x: 500, y: 800, z: 400, formation: "rock1", series: 0 },
  { x: 900, y: 800, z: 400, formation: "rock1", series: 0 },
];

export const SEED_ORIENTATIONS: OrientationRow[] = [
  { x: 500, y: 500, z: 600, azimuth: 90, dip: 0, polarity: 1, formation: "rock2", series: 0 },
  { x: 500, y: 500, z: 400, azimuth: 90, dip: 0, polarity: 1, formation: "rock1", series: 0 },
];

// World bounding box for meshing (the model1 extent).
export const EXTENT: [number, number, number, number, number, number] = [
  0, 1000, 0, 1000, 0, 1000,
];

/** Domain center, for anchoring a fault plane. */
export const CENTER: [number, number, number] = [
  (EXTENT[0] + EXTENT[1]) / 2,
  (EXTENT[2] + EXTENT[3]) / 2,
  (EXTENT[4] + EXTENT[5]) / 2,
];

/** Largest extent span (the octree cube edge + fault-offset slider range). */
export const SPAN = Math.max(
  EXTENT[1] - EXTENT[0],
  EXTENT[3] - EXTENT[2],
  EXTENT[5] - EXTENT[4],
);

/** Distinct interface ids of a series, in ascending first-seen order — the order the
 *  kernel assigns interface ids and the order surfaces are meshed. The id of a formation
 *  is its index in this list (per series). */
export function seriesFormations(
  surfacePoints: SurfacePointRow[],
  series: number,
): string[] {
  const seen: string[] = [];
  for (const r of surfacePoints) {
    if (r.series === series && !seen.includes(r.formation)) seen.push(r.formation);
  }
  return seen;
}

/** A vertical fault plane from a map strike azimuth (deg from +Y/North) and a signed offset
 *  of the plane along its horizontal normal from the domain center. Normal = (cos az, −sin
 *  az, 0); the plane point = center + offset · normal. Used by the per-series fault controls.
 */
export function faultFromControls(azimuthDeg: number, offset: number): FaultConfig {
  const az = (azimuthDeg * Math.PI) / 180;
  const normal: [number, number, number] = [Math.cos(az), -Math.sin(az), 0];
  const point: [number, number, number] = [
    CENTER[0] + offset * normal[0],
    CENTER[1] + offset * normal[1],
    CENTER[2],
  ];
  return { point, normal };
}

/** Recover (azimuth°, offset) from a FaultConfig's plane (the inverse of `faultFrom
 *  controls`), so the series panel's sliders can be seeded from a stored fault. */
export function faultToControls(f: FaultConfig): { azimuth: number; offset: number } {
  const [nx, ny] = f.normal;
  let azimuth = (Math.atan2(-ny, nx) * 180) / Math.PI;
  if (azimuth < 0) azimuth += 360;
  const offset = (f.point[0] - CENTER[0]) * nx + (f.point[1] - CENTER[1]) * ny;
  return { azimuth, offset };
}
