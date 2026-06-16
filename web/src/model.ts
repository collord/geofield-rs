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
  /** Fault block this surface belongs to (undefined when not faulted). Each block's
   *  copy of a contact is a SEPARATE, unconnected surface (Leapfrog-style). */
  block?: number;
  /** Lithology unit index, for the faulted CLOSED-domain view (a watertight slab per
   *  block per unit). Undefined for contact-surface meshes. */
  unit?: number;
  positions: Float32Array; // 3 * V
  normals: Float32Array; // 3 * V — exact per-vertex normals (field gradient)
  indices: Uint32Array; // 3 * T
  triangleCount: number;
}

export interface MeshData {
  /** One mesh per interface, at that interface's recovered isovalue. */
  surfaces: SurfaceMesh[];
  triangleCount: number; // total across surfaces
  residual: number;
  isovalueSpread: number;
  /** Number of fault-bounded blocks the field was solved over (1 if not faulted). */
  blockCount: number;
}

/** A stable id for one surface object, used to toggle its visibility from the object
 *  tree. Encodes block + (unit | interface) so it survives re-solves. */
export function surfaceKey(s: SurfaceMesh): string {
  const blk = s.block === undefined ? "_" : `b${s.block}`;
  const part = s.unit !== undefined ? `u${s.unit}` : `i${s.interfaceId}`;
  return `${blk}:${part}`;
}

/** Human label for a surface in the object tree. */
export function surfaceLabel(s: SurfaceMesh): string {
  if (s.unit !== undefined) return `unit ${s.unit}`;
  return s.formation; // a contact: the formation name
}

const FORMATION_COLORS: Record<string, number> = {
  rock1: 0xe07b39,
  rock2: 0x3d9be9,
};

/** Render color for one surface, shared by the 3-D view and the object-tree swatch.
 *  Contacts use the formation palette (rock1/rock2). Closed lithology-unit solids use
 *  an evenly-spaced HSL hue per unit (so adjacent layers are maximally distinct, unlike
 *  a string hash where "unit 1"/"unit 2" collide), shifted in lightness per fault block
 *  so block A's and block B's layers also differ. */
export function pickColor(surf: SurfaceMesh): number {
  if (surf.unit === undefined) {
    return FORMATION_COLORS[surf.formation] ?? colorFor(surf.formation);
  }
  const hue = (surf.unit * 0.61803398875) % 1; // golden-angle hue spacing
  const block = surf.block ?? 0;
  const light = 0.45 + 0.12 * (block % 3); // per-block lightness shift
  return hslToHex(hue, 0.6, light);
}

/** "#rrggbb" CSS string for a surface color (object-tree swatch). */
export function pickColorCss(surf: SurfaceMesh): string {
  return "#" + pickColor(surf).toString(16).padStart(6, "0");
}

function hslToHex(h: number, s: number, l: number): number {
  const f = (n: number) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255);
  };
  return (f(0) << 16) | (f(8) << 8) | f(4);
}

/** Stable fallback color for any formation not in the palette. */
function colorFor(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return h & 0xffffff;
}

export interface SolveResult {
  mesh: MeshData;
  /** A hard failure (the solve threw / bad input) — nothing was produced. */
  error: string | null;
  /** A non-fatal note (e.g. a fault stranded a block) — a mesh may still exist. */
  warning: string | null;
}

/** A single planar fault: a point on the plane + an outward normal. When
 *  `enabled`, the field solves independently on each side so contacts are offset
 *  laterally across the plane (a finite fault). */
export interface FaultConfig {
  enabled: boolean;
  point: [number, number, number];
  normal: [number, number, number];
}

/** Fit the model from the current rows and adaptively (octree) dual-contour ONE
 *  surface per interface, each at its recovered isovalue (so the meshes pass
 *  through the input surface points, as gempy renders contacts — not an arbitrary
 *  level set). `maxDepth` caps octree refinement (finest cell = extent / 2^depth);
 *  coarse where the surface is flat, fine where it curves. Returns meshes or an
 *  error string without throwing. */
export function solveAndMesh(
  surfacePoints: SurfacePointRow[],
  orientations: OrientationRow[],
  maxDepth: number,
  cap: boolean = false,
  fault: FaultConfig | null = null,
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
    if (fault && fault.enabled) {
      // Faulted fit: the field solves independently on each side of the plane, so
      // contacts are offset across it. A degenerate normal means "no fault".
      model = WasmModel.new_faulted(
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
        new Float64Array(fault.point),
        new Float64Array(fault.normal),
      );
    } else {
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
    }
  } catch (e) {
    return { mesh: emptyMesh(), error: String(e), warning: null };
  }

  const [minX, maxX, minY, maxY, minZ, maxZ] = EXTENT;
  // Octree needs a cube; use the largest extent span and anchor at the min corner.
  const size = Math.max(maxX - minX, maxY - minY, maxZ - minZ);

  const nameOf = (id: number): string => SERIES_ORDER[id] ?? `interface ${id}`;
  const minDepth = Math.min(2, maxDepth);
  const QEF_EPS = 1e-6;

  const surfaces: SurfaceMesh[] = [];
  let totalTris = 0;

  // CLOSED (cap on): one watertight solid per (block, lithology UNIT) — the disjoint
  //   slabs between consecutive contacts. This is the geological closed model and the
  //   slabs do NOT overlap. (The old per-INTERFACE closed path built nested
  //   "everything below contact X" solids, so the upper one fully enclosed/occluded the
  //   lower — the bug this replaces.) Works for 1 block (block 0 = whole model) or N.
  // OPEN (cap off): a FAULTED model renders one clean clipped contact per (block,
  //   interface) to avoid the cross-fault discontinuity crumpling the contour;
  //   a non-faulted model renders one contact surface per interface over the whole box.
  const blockCount = model.block_count;
  const closed = cap;
  const faultedOpen = blockCount > 1 && !cap;

  const pushSurface = (
    wm: ReturnType<WasmModel["mesh_isosurface_octree"]>,
    interfaceId: number,
    isovalue: number,
    block: number | undefined,
    unit?: number,
  ) => {
    // Closed-solid units color by unit (a distinct stable color per lithology layer);
    // contact surfaces color by their interface's formation.
    const formation = unit !== undefined ? `unit ${unit}` : nameOf(interfaceId);
    surfaces.push({
      interfaceId,
      formation,
      isovalue,
      block,
      unit,
      positions: wm.positions(),
      normals: wm.normals(),
      indices: wm.indices(),
      triangleCount: wm.triangle_count,
    });
    totalTris += wm.triangle_count;
    wm.free();
  };

  if (closed) {
    // One watertight solid per (block, lithology UNIT) — disjoint slabs (capped by the
    // box, and by the fault plane when faulted). Block 0 is the whole model when not
    // faulted. Per-block keeps the field smooth on each side of any fault.
    for (let b = 0; b < blockCount; b++) {
      const units = model.block_unit_count(b);
      const bIds = Array.from(model.block_interface_ids(b));
      const blockLabel = blockCount > 1 ? b : undefined;
      for (let u = 0; u < units; u++) {
        const wm = model.mesh_block_unit_solid_octree(
          b,
          u,
          minX,
          minY,
          minZ,
          size,
          maxDepth,
          minDepth,
          QEF_EPS,
        );
        if (wm.triangle_count > 0) {
          // Unit u sits above interface u-1; tag with that interface id when present.
          const lowerId = u > 0 ? bIds[u - 1] : -1;
          pushSurface(wm, lowerId, 0, blockLabel, u);
        } else {
          wm.free();
        }
      }
    }
  } else if (faultedOpen) {
    // One contact surface per (block, interface), each clipped to its block's region.
    for (let b = 0; b < blockCount; b++) {
      const bIds = Array.from(model.block_interface_ids(b));
      const bIsos = Array.from(model.interface_isovalues()); // per-block uses own iso internally
      for (let s = 0; s < bIds.length; s++) {
        const wm = model.mesh_block_interface_octree(
          b,
          s,
          minX,
          minY,
          minZ,
          size,
          maxDepth,
          minDepth,
          QEF_EPS,
        );
        if (wm.triangle_count > 0) {
          pushSurface(wm, bIds[s], bIsos[s] ?? 0, b);
        } else {
          wm.free();
        }
      }
    }
  } else {
    // Non-faulted OPEN sheet: one contact surface per interface over the whole domain.
    const isovalues = Array.from(model.interface_isovalues());
    const ids = Array.from(model.interface_ids());
    for (let s = 0; s < isovalues.length; s++) {
      const wm = model.mesh_isosurface_octree(
        minX,
        minY,
        minZ,
        size,
        isovalues[s],
        maxDepth,
        minDepth,
        QEF_EPS,
        false, // open sheet (closed solids go through the per-unit path above)
      );
      pushSurface(wm, ids[s], isovalues[s], undefined);
    }
  }

  const mesh: MeshData = {
    surfaces,
    triangleCount: totalTris,
    residual: model.residual(),
    isovalueSpread: model.isovalue_spread(),
    blockCount,
  };
  model.free();

  // A faulted fit where the plane stranded a block (no orientation on one side)
  // meshes only part of the domain — flag it so an empty half doesn't look broken.
  let warning: string | null = null;
  if (fault && fault.enabled && totalTris === 0) {
    warning =
      "fault left no meshable surface — the plane may strand all data on one side " +
      "(each side needs at least one orientation). Adjust strike/offset.";
  }
  return { mesh, error: null, warning };
}

function emptyMesh(): MeshData {
  return {
    surfaces: [],
    triangleCount: 0,
    residual: NaN,
    isovalueSpread: NaN,
    blockCount: 0,
  };
}
