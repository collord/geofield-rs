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

/** One dual-contoured contact surface, at a recovered interface isovalue. */
export interface SurfaceMesh {
  interfaceId: number;
  formation: string; // formation name for this interface id (best-effort)
  isovalue: number;
  positions: Float32Array; // 3 * V
  indices: Uint32Array; // 3 * T
  triangleCount: number;
}

export interface MeshData {
  /** One mesh per interface, at that interface's recovered isovalue. */
  surfaces: SurfaceMesh[];
  triangleCount: number; // total across surfaces
  residual: number;
  isovalueSpread: number;
}

export interface SolveResult {
  mesh: MeshData;
  error: string | null;
}

/** Fit the model from the current rows and dual-contour ONE surface per
 *  interface, each at its recovered isovalue (so the meshes pass through the
 *  input surface points, as gempy renders contacts — not an arbitrary level set).
 *  `res` is cells per axis. Returns meshes or an error string without throwing. */
export function solveAndMesh(
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
  res: number,
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

  // Mesh ONE surface per interface, at its recovered isovalue. These pass
  // through the input surface points (the geological contacts), matching gempy —
  // not an arbitrary level set.
  const isovalues = Array.from(model.interface_isovalues());
  const ids = Array.from(model.interface_ids());
  const nameOf = (id: number): string => SERIES_ORDER[id] ?? `interface ${id}`;

  const surfaces: SurfaceMesh[] = [];
  let totalTris = 0;
  for (let s = 0; s < isovalues.length; s++) {
    const wm = model.mesh_isosurface(
      minX,
      minY,
      minZ,
      maxX,
      maxY,
      maxZ,
      res,
      isovalues[s],
    );
    surfaces.push({
      interfaceId: ids[s],
      formation: nameOf(ids[s]),
      isovalue: isovalues[s],
      positions: wm.positions(),
      indices: wm.indices(),
      triangleCount: wm.triangle_count,
    });
    totalTris += wm.triangle_count;
    wm.free();
  }

  const mesh: MeshData = {
    surfaces,
    triangleCount: totalTris,
    residual: model.residual(),
    isovalueSpread: model.isovalue_spread(),
  };
  model.free();
  return { mesh, error: null };
}

function emptyMesh(): MeshData {
  return {
    surfaces: [],
    triangleCount: 0,
    residual: NaN,
    isovalueSpread: NaN,
  };
}
