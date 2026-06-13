// Editable input rows + the gempy model1 seed data.
//
// Surface points: X,Y,Z,formation.  Orientations: X,Y,Z,azimuth,dip,polarity,
// formation — gempy's CSV format (validation/fixtures/model1).

export interface SurfacePointRow {
  x: number;
  y: number;
  z: number;
  formation: string;
}

export interface OrientationRow {
  x: number;
  y: number;
  z: number;
  azimuth: number;
  dip: number;
  polarity: number;
  formation: string;
}

// jan_models/model1 — horizontal stratigraphy (the validated reference).
export const SEED_SURFACE_POINTS: SurfacePointRow[] = [
  { x: 100, y: 200, z: 600, formation: "rock2" },
  { x: 500, y: 200, z: 600, formation: "rock2" },
  { x: 900, y: 200, z: 600, formation: "rock2" },
  { x: 100, y: 800, z: 600, formation: "rock2" },
  { x: 500, y: 800, z: 600, formation: "rock2" },
  { x: 900, y: 800, z: 600, formation: "rock2" },
  { x: 100, y: 200, z: 400, formation: "rock1" },
  { x: 500, y: 200, z: 400, formation: "rock1" },
  { x: 900, y: 200, z: 400, formation: "rock1" },
  { x: 100, y: 800, z: 400, formation: "rock1" },
  { x: 500, y: 800, z: 400, formation: "rock1" },
  { x: 900, y: 800, z: 400, formation: "rock1" },
];

export const SEED_ORIENTATIONS: OrientationRow[] = [
  { x: 500, y: 500, z: 600, azimuth: 90, dip: 0, polarity: 1, formation: "rock2" },
  { x: 500, y: 500, z: 400, azimuth: 90, dip: 0, polarity: 1, formation: "rock1" },
];

// Formation → interface id, in the series order rock2, rock1 (bottom→top per the
// gempy mapping). The order determines interface ids; the WASM model assigns ids
// 0,1,... in this order.
export const SERIES_ORDER = ["rock2", "rock1"];

// World bounding box for meshing (the model1 extent).
export const EXTENT: [number, number, number, number, number, number] = [
  0, 1000, 0, 1000, 0, 1000,
];
