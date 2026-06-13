// WASM glue: turn editable rows into a fitted WasmModel and a dual-contoured
// isosurface mesh. Mirrors the conversions in src/bin/geofield_cli.rs.

import init, { WasmModel } from "./wasm/geofield_rs.js";
import {
  SurfacePointRow,
  OrientationRow,
  SERIES_ORDER,
  EXTENT,
} from "./data";

let ready: Promise<void> | null = null;

/** Initialize the WASM module exactly once. */
export function initWasm(): Promise<void> {
  if (!ready) ready = init().then(() => undefined);
  return ready;
}

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

export interface MeshData {
  positions: Float32Array; // 3 * V
  indices: Uint32Array; // 3 * T
  vertexCount: number;
  triangleCount: number;
  residual: number;
  isovalueSpread: number;
}

export interface SolveResult {
  mesh: MeshData;
  error: string | null;
}

/** Fit the model from the current rows and dual-contour the isovalue-0 surface.
 *  `res` is cells per axis for the meshing grid. Returns a mesh or an error
 *  string (e.g. degenerate inputs) without throwing. */
export function solveAndMesh(
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
  res: number,
  isovalue: number,
): SolveResult {
  const idOf = (formation: string): number => {
    const i = SERIES_ORDER.indexOf(formation);
    return i < 0 ? 0 : i;
  };

  const sp_xyz = new Float64Array(surfacePoints.length * 3);
  const sp_if = new Float64Array(surfacePoints.length);
  surfacePoints.forEach((r, i) => {
    sp_xyz[3 * i] = r.x;
    sp_xyz[3 * i + 1] = r.y;
    sp_xyz[3 * i + 2] = r.z;
    sp_if[i] = idOf(r.formation);
  });

  const ori_xyz = new Float64Array(orientations.length * 3);
  const ori_pole = new Float64Array(orientations.length * 3);
  orientations.forEach((r, i) => {
    ori_xyz[3 * i] = r.x;
    ori_xyz[3 * i + 1] = r.y;
    ori_xyz[3 * i + 2] = r.z;
    const [gx, gy, gz] = azimuthDipToPole(r.azimuth, r.dip, r.polarity);
    ori_pole[3 * i] = gx;
    ori_pole[3 * i + 1] = gy;
    ori_pole[3 * i + 2] = gz;
  });

  const NAN = Number.NaN;
  let model: WasmModel;
  try {
    // NaN options → crate defaults (range 1.7, c_o 10, i_res 4, gi_res 2,
    // uni_degree 1, nugget 1e-5). uni_degree = 1.
    model = new WasmModel(
      sp_xyz,
      sp_if,
      ori_xyz,
      ori_pole,
      NAN,
      NAN,
      NAN,
      NAN,
      1,
      NAN,
    );
  } catch (e) {
    return { mesh: emptyMesh(), error: String(e) };
  }

  const [minX, maxX, minY, maxY, minZ, maxZ] = EXTENT;
  const wm = model.mesh_isosurface(
    minX,
    minY,
    minZ,
    maxX,
    maxY,
    maxZ,
    res,
    isovalue,
  );

  const mesh: MeshData = {
    positions: wm.positions(),
    indices: wm.indices(),
    vertexCount: wm.vertex_count,
    triangleCount: wm.triangle_count,
    residual: model.residual(),
    isovalueSpread: model.isovalue_spread(),
  };
  wm.free();
  model.free();
  return { mesh, error: null };
}

function emptyMesh(): MeshData {
  return {
    positions: new Float32Array(0),
    indices: new Uint32Array(0),
    vertexCount: 0,
    triangleCount: 0,
    residual: NaN,
    isovalueSpread: NaN,
  };
}
