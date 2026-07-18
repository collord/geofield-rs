// Feature 002 (enhanced-authoring) — the client-side PROJECT DOCUMENT and its single
// compile seam to the unchanged fit path.
//
// This is the source of truth for AUTHORING only. It lives in the browser, NEVER crosses
// the wasm boundary, and compiles DOWN to the existing `SeriesJson[]` the worker already
// consumes (protocol.ts is unchanged). The CORE remains the source of truth for everything
// geological (effective relations, cross-cut sets, fitted-result validation), read back via
// the unchanged `CoreUpdate.derived`.
//
// Design source of record: docs/enhanced_authoring_spec.md; plan/contracts under
// specs/002-enhanced-authoring/. No Rust/worker/ABI/schema changes accompany this file.

import type { SeriesJson } from "../protocol";
import { derivePoles, meanPole, selfCrosses } from "../section/digitizeDerive";
import { PLANE_EPS_FRACTION, extentDiagonal } from "../section/frame";

// ============================================================================
// T003 — data model types (data-model.md)
// ============================================================================

/** Authoring-facing surface type. Maps to (container kind + Axis-A relation). The user
 *  picks one of these on "Add Surface"; container membership is then DERIVED (R4), never
 *  declared. */
export type SurfaceType = "conformable" | "erosional" | "onlap" | "vein" | "intrusion" | "topography";

// ============================================================================
// Lithology catalog (feature 003 §4) — UI-side identity metadata. NEVER crosses the wasm
// boundary; the core supplies only unit COORDINATES (DerivedView.units). Behaviour in US3.
// ============================================================================

export type LithologyId = string;

/** A named, coloured rock type. `kind` is provenance → editability (data-model §3):
 *  air/unknown are fixed singletons; inferred is synced from a vein/intrusion body; user
 *  is fully CRUD. */
export interface Lithology {
  id: LithologyId;
  name: string;
  color: string; // hex (e.g. "#a8794f"); "air" is rendered transparent
  kind: "air" | "unknown" | "inferred" | "user";
}

/** The catalog + the many-to-one `(element, unit) → lithology` assignment. Assignment keys
 *  are `${elementId}:${unitIndex}` (the core's volume coordinate). */
export interface LithologyCatalog {
  lithologies: Lithology[]; // includes the fixed Air + Unknown members
  assignment: Record<string, LithologyId>; // key = `${elementId}:${unitIndex}`
}

/** The two fixed catalog members, present in every catalog (non-deletable). */
export const AIR_ID: LithologyId = "air";
export const UNKNOWN_ID: LithologyId = "unknown";

/** One XYZ contact point on a surface. */
export interface PointRow {
  x: number;
  y: number;
  z: number;
}

/** One orientation reading, RAW as the user typed it. dip = degrees from horizontal;
 *  dipDir = degrees clockwise from North (+Y). Converted to a pole ONLY at compile time
 *  (toSeriesJson → dipDirToPole), so the table round-trips exactly what was entered. */
export interface OrientationRow {
  x: number;
  y: number;
  z: number;
  dip: number;
  dipDir: number;
}

// ============================================================================
// Cross-sections & digitized lines (feature 005) — data-model.md / contracts/project-json.md.
// A SECTION SET is a VIEW construct (never geology: creating/moving/deleting one never sets
// dirty, never re-fits — spec FR-009). A DIGITIZED LINE is GEOMETRY (a deferred commit edit).
// ============================================================================

/** A family of parallel vertical section planes from one plan-view baseline A→B. count = 1
 *  ⇒ a single section. Per-plane geometry (frames, planes) is DERIVED in section/frame.ts,
 *  never stored (Constitution III). */
export interface SectionSet {
  id: string;
  name: string;
  a: [number, number]; // baseline start, world XY
  b: [number, number]; // baseline end; |b − a| > 0 guaranteed by the creation gesture
  spacing: number; // > 0 when count > 1; plane k sits at offset + k·spacing along ŝ
  count: number; // integer ≥ 1
  offset?: number; // default 0: start offset of plane 0 from the baseline along ŝ
  showInScene?: boolean; // the 3D translucent-quad toggle (US-SECT-5); default false
}

/** An ordered polyline digitized on one section plane for one surface. SELF-SUFFICIENT: it
 *  snapshots its plane (`frame`), so deleting the section (a view construct) never orphans
 *  the geometry; `sectionRef` is a soft UI-grouping ref that may dangle. VERTEX ORDER IS
 *  LOAD-BEARING — the drawn direction encodes younging (contracts/younging-derivation.md §2:
 *  pole m = ŝ × t̂, "youngs to the LEFT" face-on). */
export interface DigitizedLine {
  id: string;
  frame: { origin: [number, number, number]; s: [number, number, number] }; // ŝ unit, horizontal
  vertices: [number, number, number][]; // world-space, all on the plane (guarded on import)
  sectionRef?: { setId: string; index: number };
}

/** One interface's interpolation data — the unit a user adds and edits. */
export interface InterfaceData {
  id: string; // stable client id
  name: string;
  points: PointRow[]; // XYZ contact points (>=1 to be valid; >=3 to fit a plane well)
  orientations: OrientationRow[]; // optional; empty is valid
  /** Negate the orientation poles at compile time (the field's "which side is up" sense).
   *  A SEPARATE declared bit so the raw dip/dipDir the user typed still round-trip exactly;
   *  flipping is reversible and never rewrites the entered values. Absent ⇒ not flipped.
   *  Applies to TYPED and DIGITIZED poles through the same XOR (flattenInterfaces). */
  flipped?: boolean;
  /** Digitized section polylines (feature 005). Their vertices ARE this surface's contact
   *  points and their drawn directions its orientations — derived at compile/render time,
   *  never copied into `points`/`orientations` (one dataset, two views — spec FR-012). */
  digitized?: DigitizedLine[];
}

/** An authoring container. A `series` with exactly one interface is PRESENTED FLAT (as a
 *  plain surface); with more than one it nests. Vein/intrusion always present their named
 *  constituent surfaces. The two-level model underneath is unchanged. */
export type ProjectElement =
  | {
      kind: "series";
      id: string;
      name: string;
      relation: "erode" | "onlap" | "conformable";
      unboundedAbove: boolean; // Axis B (DEPRECATED/inert — fill-upward is universal, feature 003 §3)
      interfaces: InterfaceData[]; // 1..n, one co-kriged field
    }
  | { kind: "vein"; id: string; name: string; footWall: InterfaceData; hangingWall: InterfaceData }
  | {
      kind: "intrusion";
      id: string;
      name: string;
      /** "solid" = a filled body bounded by ONE envelope (a stock/pluton — engine `Single`);
       *  "shell" = a hollow-cored body between an inner and outer envelope (a ring dike / cored
       *  pluton — engine `Shell`). In "solid" mode `envelopeInner` is retained but unused (not
       *  shown, not compiled). Absent on old saved projects ⇒ treated as "shell" (back-compat). */
      mode: "solid" | "shell";
      envelopeInner: InterfaceData;
      envelopeOuter: InterfaceData; // "shell": the outer wall; "solid": the sole envelope
    }
  | {
      // The singleton topography surface (feature 003 §2). One interface — the ground; Air
      // above it. Pinned youngest; the compile seam emits it first and the worker pins it
      // via add_topography (insert-at-0), so its array position is robust.
      kind: "topography";
      id: string;
      name: string;
      ground: InterfaceData;
    }
  | {
      // An IMPORTED mesh surface (feature 004). It carries the `.obj` TEXT and its role; the engine
      // parses + fits a field through the mesh. For a contact/erosion role it also carries the Axis-A
      // relation bit. No authored interfaces — the geometry IS the mesh.
      kind: "importedSurface";
      id: string;
      name: string;
      role: "contact" | "erosion" | "veinWall" | "topography";
      relation: "erode" | "onlap" | "conformable";
      /** The `.obj` file contents. Always present in the LIVE project (needed to fit) and in
       *  localStorage autosave. On EXPORT it is dropped when `objPath` is set (a lidar mesh is ~MBs);
       *  the loader re-hydrates it by fetching `objPath`. May be "" transiently between load and
       *  hydration for a path-only imported file. */
      objText: string;
      /** OPTIONAL external reference: a path under the app's static `meshes/` dir (e.g.
       *  `meshes/lidar.obj`), fetched at `/<objPath>`. When present, EXPORT omits `objText` and the
       *  loader re-fetches from here — so a big mesh lives as a file, not inline JSON. Absent for a
       *  mesh with no external copy (then `objText` is the only source and always round-trips). */
      objPath?: string;
      /** Extend the surface smoothly to the domain box beyond its footprint (US2). Default false. */
      extendToBounds: boolean;
    };

export interface Project {
  version: 1;
  name: string;
  extent: [number, number, number, number, number, number]; // [x0,x1,y0,y1,z0,z1]
  elements: ProjectElement[]; // YOUNGEST FIRST (matches add_* call order / SeriesJson order)
  /** Optional per-UNIT display names (feature 003 §1), keyed `${elementId}:${unitIndex}`.
   *  Units are DERIVED (not authored) — only their NAME is UI metadata. Absent ⇒ a default
   *  "Unit N" label. Serialized so names round-trip with the project. */
  unitNames?: Record<string, string>;
  /** The lithology catalog + per-unit assignment (feature 003 §4). Optional so older saved
   *  projects load; a fresh/loaded project without one gets the default catalog (US3). */
  catalog?: LithologyCatalog;
  /** Cross-section sets (feature 005). VIEW/AUTHORING constructs, not geology: mutations
   *  save immediately without dirty/re-fit (the setRelation pattern). Absent ⇒ none. */
  sections?: SectionSet[];
  /** Optional saved VIEW state — captured on Export and re-applied on Import so a viewpoint
   *  round-trips and an exact view can be shared for repro. UI-only (never sent to the
   *  worker as geology). Covers the camera AND the render toggles, so "what I'm looking at"
   *  (Solids vs Surfaces, clip seam, edges) reproduces, not just the angle. */
  view?: {
    camera: [number, number, number];
    target: [number, number, number];
    up: [number, number, number];
    fov: number;
    /** Render mode: "contact" = Surfaces, "solid" = Solids. */
    renderMode?: "contact" | "solid";
    /** The "clip seam" toggle (cross-cut seam closer). */
    seamClip?: boolean;
    /** The "edges" (wireframe) toggle. */
    edges?: boolean;
    /** The authored points/normals overlay toggle (parametric surfaces). */
    overlay?: boolean;
    /** Size multipliers for the points/normals overlay (each defaults to 1.0). All three scale a
     *  width/100 baseline, so they're scale-invariant across models of any world size. */
    overlayPointScale?: number;
    overlayArrowScale?: number;
    overlayLabelScale?: number;
    /** The open section (feature 005), restored on import so a project reopens on the same
     *  slice (spec FR-010/SC-005). Index clamped to [0, count) on load. Absent ⇒ 3D view. */
    section?: { setId: string; index: number };
  };
}

/** Author-time validation (computed, never stored). DISTINCT from `CoreUpdate.derived.
 *  validation`, which judges the FITTED result — see validate()'s note and research.md R10. */
export interface ProjectValidation {
  hardErrors: { elementId: string; interfaceId?: string; reason: string }[];
  /** `lineId` + `fix` tag a warning that carries a one-action remedy (feature 005:
   *  "flip this line to match" reverses that digitized line's vertex order). */
  softWarnings: { elementId: string; interfaceId?: string; lineId?: string; fix?: "flipLine"; reason: string }[];
  canApply: boolean;
}

// ============================================================================
// T004 — dip/dip-direction -> pole (the one bit of geology in the UI)
// ============================================================================

// THE ONE PLACE a sign error silently produces an upside-down field. Transcribed
// sign-for-sign from docs/enhanced_authoring_spec.md §5.3.
//
// AXIS CONVENTION: X=East, Y=North, Z=Up. dip-direction is the azimuth of steepest
// descent, measured CLOCKWISE from North (+Y). The returned pole is a UNIT vector pointing
// UP-dip, out of the footwall. dip (delta) in degrees from horizontal; dipDir (alpha) in
// degrees clockwise from +Y.
export function dipDirToPole(dipDeg: number, dipDirDeg: number): [number, number, number] {
  const d = (dipDeg * Math.PI) / 180;
  const a = (dipDirDeg * Math.PI) / 180;
  const nx = Math.sin(d) * Math.sin(a);
  const ny = Math.sin(d) * Math.cos(a);
  const nz = Math.cos(d);
  return [nx, ny, nz]; // already unit; points up out of the footwall
}

/** The GEOMETRIC up-pointing surface normal for a plane of dip `dipDeg` / dip-direction `dipDirDeg`
 *  (same X=East, Y=North, Z=Up, azimuth-clockwise-from-North convention). This is the true pole ⊥
 *  the plane, for VISUALIZATION only — NOT the fit constraint.
 *
 *  It differs from `dipDirToPole` by the SIGN of the horizontal component: the fit vector leans
 *  DOWN-dip (toward the dip azimuth), whereas the geometric up-normal leans UP-dip (away from it),
 *  which is what a "normal arrow" over a surface should show. They coincide only at dip 0 (both
 *  straight up) and are horizontally opposite at dip 90 (a vertical plane's two horizontal normals).
 *  Do NOT feed this to the kriging fit — use `dipDirToPole` there. */
export function dipDirToNormal(dipDeg: number, dipDirDeg: number): [number, number, number] {
  const d = (dipDeg * Math.PI) / 180;
  const a = (dipDirDeg * Math.PI) / 180;
  return [-Math.sin(d) * Math.sin(a), -Math.sin(d) * Math.cos(a), Math.cos(d)];
}

// ============================================================================
// T005 — compile to the worker ABI (contracts/project-model.md)
// ============================================================================

/** RELATION codes (Axis A), mirroring relation_from_code in wasm.rs / scenarios.ts. */
const RELATION_CODE: Record<"erode" | "onlap" | "conformable", number> = {
  erode: 0,
  onlap: 1,
  conformable: 2,
};

const NO_FAULT = { fault_point: [0, 0, 0], fault_normal: [0, 0, 0] };

/** Flatten an ordered list of interfaces into the SeriesJson point/orientation arrays.
 *  The interface's position in the list becomes its `sp_if` id. Orientations convert
 *  dip/dipDir -> pole HERE and nowhere else. */
function flattenInterfaces(
  interfaces: InterfaceData[],
  defaultFlip = false,
): Pick<SeriesJson, "sp_xyz" | "sp_if" | "ori_xyz" | "ori_pole"> {
  const sp_xyz: number[] = [];
  const sp_if: number[] = [];
  const ori_xyz: number[] = [];
  const ori_pole: number[] = [];
  interfaces.forEach((iface, ifId) => {
    for (const p of iface.points) {
      sp_xyz.push(p.x, p.y, p.z);
      sp_if.push(ifId);
    }
    // `defaultFlip` is the relation's natural "up" sense: an ERODE series deposits ABOVE its
    // unconformity (eroding DOWN into what's below), so its field points the opposite way to a
    // conformable bed's. The per-interface `flipped` toggle is then an XOR override on top of
    // that default — so an UNflipped erode contact is already the correct deposit-above state,
    // not a double-negative the author has to flip in. (Confirmed convention: erode ⇒ deposit-above.)
    const s = (!!iface.flipped !== defaultFlip) ? -1 : 1; // flip negates the pole (field "up" sense)
    for (const o of iface.orientations) {
      ori_xyz.push(o.x, o.y, o.z);
      const [nx, ny, nz] = dipDirToPole(o.dip, o.dipDir);
      ori_pole.push(s * nx, s * ny, s * nz);
    }
    // Digitized section lines (feature 005): each vertex is a contact point; each segment
    // yields one orientation at its midpoint with pole m = ŝ × t̂ (the drawn direction sets
    // the younger side — contracts/younging-derivation.md §2). The SAME flip XOR `s` applies
    // (contract §3): per-surface flip negates typed AND digitized poles at this one site.
    for (const line of iface.digitized ?? []) {
      for (const v of line.vertices) {
        sp_xyz.push(v[0], v[1], v[2]);
        sp_if.push(ifId);
      }
      for (const d of derivePoles(line.vertices, line.frame.s)) {
        ori_xyz.push(d.pos[0], d.pos[1], d.pos[2]);
        ori_pole.push(s * d.pole[0], s * d.pole[1], s * d.pole[2]);
      }
    }
  });
  return { sp_xyz, sp_if, ori_xyz, ori_pole };
}

// ============================================================================
// Authored points/normals overlay payload (viewer-agnostic; parametric surfaces only)
// ============================================================================

/** One authored contact point, world-space, tagged with its 1-based row number (matching the
 *  data modal's table) and a color index (the element's position, for the viewer palette). */
export interface OverlayPoint {
  pos: [number, number, number];
  row: number;
  colorIndex: number;
}
/** One authored orientation reading, rendered as a surface-normal arrow.
 *
 *  ⚠️ CONVENTION — READ BEFORE CHANGING. The data TABLE stores the geologic standard: `dip` (degrees
 *  below horizontal) and `dipDir` (azimuth of steepest DESCENT, clockwise from North). Those are NOT
 *  vector components. `dir` here is the true GEOMETRIC up-normal (⊥ the surface), from
 *  `dipDirToNormal(dip, dipDir)`. Three distinct vectors are easy to confuse:
 *    • dip vector      — down-slope INTO the surface (what "dips 30° toward 090°" pictures).
 *    • up-normal (dir) — ⊥ the surface, pointing UP-and-up-dip. What this arrow draws.
 *    • fit pole        — `dipDirToPole`, the kriging constraint; leans DOWN-dip, horizontal-flipped
 *                        from the up-normal. Do NOT draw this — it lies flat & faces down-dip at δ=90.
 *  Flip is intentionally NOT applied, so it's the normal of the datum as typed, before the element's
 *  relation/flip sign. */
export interface OverlayNormal {
  pos: [number, number, number]; // world position of the reading (arrow tail)
  dir: [number, number, number]; // true up-normal (unit), from dipDirToNormal — NOT the fit pole or dip vector
  row: number;
  colorIndex: number;
}
export interface AuthoredOverlay {
  points: OverlayPoint[];
  normals: OverlayNormal[];
}

/** The parametric interfaces of an element, in presentation order (matches `interfacesOf` in
 *  projectOps + the panel's contact rows). Imported-mesh surfaces have NONE (geometry is the mesh). */
function overlayInterfaces(el: ProjectElement): InterfaceData[] {
  switch (el.kind) {
    case "series":
      return el.interfaces;
    case "vein":
      return [el.footWall, el.hangingWall];
    case "intrusion":
      return (el.mode ?? "shell") === "solid"
        ? [el.envelopeOuter]
        : [el.envelopeInner, el.envelopeOuter];
    case "topography":
      return [el.ground];
    case "importedSurface":
      return [];
  }
}

/** Flatten every parametric surface's authored points + orientations into a viewer-agnostic,
 *  world-space overlay payload. Row numbers are 1-based PER INTERFACE (matching the data modal).
 *
 *  Normals: the table's geologic `dip`/`dipDir` are converted to the true geometric up-normal via
 *  `dipDirToNormal` (⊥ the surface — NOT the fit's `dipDirToPole`, which leans down-dip and lies flat
 *  at δ=90; see OverlayNormal's note). No flip is applied, so it's the normal of the datum as typed,
 *  before the element's relation/flip sign — an inspection view of the reading itself. Pure: same
 *  project ⇒ identical payload. */
export function buildAuthoredOverlay(project: Project): AuthoredOverlay {
  const points: OverlayPoint[] = [];
  const normals: OverlayNormal[] = [];
  project.elements.forEach((el, colorIndex) => {
    for (const iface of overlayInterfaces(el)) {
      iface.points.forEach((p, i) => {
        points.push({ pos: [p.x, p.y, p.z], row: i + 1, colorIndex });
      });
      iface.orientations.forEach((o, i) => {
        // The true GEOMETRIC up-normal (⊥ the surface), NOT the fit's dipDirToPole constraint —
        // so the arrow reads as perpendicular to the surface it annotates. See dipDirToNormal.
        normals.push({ pos: [o.x, o.y, o.z], dir: dipDirToNormal(o.dip, o.dipDir), row: i + 1, colorIndex });
      });
      // Digitized lines (feature 005, FR-012): their vertices ARE contact points and their
      // per-segment poles orientations — the overlay is the second view of the one dataset.
      // Row numbering continues after the typed rows (matching the modal's union table).
      // Poles are pre-flip, like the typed normals ("the datum as drawn").
      let row = iface.points.length;
      let orow = iface.orientations.length;
      for (const line of iface.digitized ?? []) {
        for (const v of line.vertices) {
          row += 1;
          points.push({ pos: [v[0], v[1], v[2]], row, colorIndex });
        }
        for (const d of derivePoles(line.vertices, line.frame.s)) {
          orow += 1;
          normals.push({ pos: [d.pos[0], d.pos[1], d.pos[2]], dir: [d.pole[0], d.pole[1], d.pole[2]], row: orow, colorIndex });
        }
      }
    }
  });
  return { points, normals };
}

/** The SINGLE seam between the authoring model and the existing fit path. Pure: same
 *  project => identical SeriesJson[]. Precondition: validate(project).canApply (Apply is
 *  disabled otherwise) — a zero-point surface can't produce a meaningful SeriesJson. */
export function toSeriesJson(project: Project): SeriesJson[] {
  // Topography is youngest regardless of its slot in `elements`: emit it first so the
  // worker's add_topography (pinned index 0) and add_series (appended after) agree on order.
  const ordered = [...project.elements].sort((a, b) => {
    const at = a.kind === "topography" ? 0 : 1;
    const bt = b.kind === "topography" ? 0 : 1;
    return at - bt;
  });
  return ordered.map((el) => {
    if (el.kind === "topography") {
      return {
        name: el.name,
        relation: RELATION_CODE.erode, // ignored for topography
        unboundedAbove: false, // ignored
        ...flattenInterfaces([el.ground]),
        ...NO_FAULT,
        topography: true,
      };
    }
    if (el.kind === "series") {
      return {
        name: el.name,
        relation: RELATION_CODE[el.relation],
        unboundedAbove: el.unboundedAbove,
        // Erode deposits above its unconformity: the relation carries that "up" sense as the
        // interface default, so the erosion contact is correct UNflipped (see flattenInterfaces).
        ...flattenInterfaces(el.interfaces, el.relation === "erode"),
        ...NO_FAULT,
      };
    }
    if (el.kind === "importedSurface") {
      // The geometry IS the mesh — no authored points/orientations. The engine parses the .obj
      // bytes and fits a field through it; the fit arrays are empty and unused.
      return {
        name: el.name,
        relation: RELATION_CODE[el.relation],
        unboundedAbove: false,
        sp_xyz: [],
        sp_if: [],
        ori_xyz: [],
        ori_pole: [],
        ...NO_FAULT,
        importedMesh: {
          role: el.role,
          bytes: new TextEncoder().encode(el.objText),
          extend: el.extendToBounds,
        },
      };
    }
    // Cross-cutting bodies: relation/unboundedAbove are ignored by the core but emitted
    // with valid defaults; the two interfaces become ids 0/1.
    if (el.kind === "vein") {
      return {
        name: el.name,
        relation: RELATION_CODE.erode,
        unboundedAbove: false,
        ...flattenInterfaces([el.footWall, el.hangingWall]),
        ...NO_FAULT,
        crossCut: { kind: "vein", footWall: 0, hangingWall: 1 },
      };
    }
    // intrusion. "solid" ⇒ one envelope, compiled as equal indices (the worker's documented
    // single-envelope encoding, protocol.ts §crossCut). "shell" ⇒ distinct inner/outer walls.
    if ((el.mode ?? "shell") === "solid") {
      return {
        name: el.name,
        relation: RELATION_CODE.erode,
        unboundedAbove: false,
        ...flattenInterfaces([el.envelopeOuter]),
        ...NO_FAULT,
        crossCut: { kind: "intrusion", envelopeInner: 0, envelopeOuter: 0 },
      };
    }
    return {
      name: el.name,
      relation: RELATION_CODE.erode,
      unboundedAbove: false,
      ...flattenInterfaces([el.envelopeInner, el.envelopeOuter]),
      ...NO_FAULT,
      crossCut: { kind: "intrusion", envelopeInner: 0, envelopeOuter: 1 },
    };
  });
}

// ============================================================================
// T006 — author-time validation (data-model.md "Validation rules")
// ============================================================================

// IMPORTANT (Constitution III / research.md R10): this checks STRUCTURAL preconditions that
// must hold for a fit to even RUN (zero points => nothing to fit; missing cross-cut part =>
// no body). It does NOT re-encode any CORE geological rule — effective relations, cross-cut
// sets, and fitted-result validation stay derived by the core and read via CoreUpdate.derived.
// Keeping this strictly structural is what prevents the two validations from drifting.
export function validate(project: Project): ProjectValidation {
  const hardErrors: ProjectValidation["hardErrors"] = [];
  const softWarnings: ProjectValidation["softWarnings"] = [];
  // On-plane tolerance for digitized vertices (data-model.md; scale-relative).
  const epsPlane = Math.max(PLANE_EPS_FRACTION * extentDiagonal(project.extent), 1e-9);

  const checkInterface = (elementId: string, iface: InterfaceData) => {
    const digitizedVerts = (iface.digitized ?? []).reduce((n, l) => n + l.vertices.length, 0);
    const totalPoints = iface.points.length + digitizedVerts;
    if (totalPoints === 0) {
      hardErrors.push({ elementId, interfaceId: iface.id, reason: `"${iface.name}" has no points — add at least one` });
    } else if (totalPoints < 3) {
      softWarnings.push({ elementId, interfaceId: iface.id, reason: `"${iface.name}" needs ≥3 points to fit well` });
    }
    checkDigitized(elementId, iface);
  };

  // Feature 005 digitizing rules (data-model.md "Validation rules" 1–6). STRUCTURAL only,
  // like everything else here: can the fit even run / is the input self-consistent.
  const checkDigitized = (elementId: string, iface: InterfaceData) => {
    const lines = iface.digitized ?? [];
    if (lines.length === 0) return;

    // Rule 1 (hard): a line needs ≥2 vertices to be a line.
    for (const line of lines) {
      if (line.vertices.length < 2) {
        hardErrors.push({
          elementId,
          interfaceId: iface.id,
          reason: `"${iface.name}": a digitized line has ${line.vertices.length} vertex${line.vertices.length === 1 ? "" : "es"} — a line needs at least 2`,
        });
      }
    }

    // Rule 3 (hard, import guard): every vertex on its line's plane. The plane-locked tool
    // makes this unreachable interactively; hand-edited/foreign files are caught here.
    for (const line of lines) {
      const [ox, oy, oz] = line.frame.origin;
      const [sx, sy, sz] = line.frame.s;
      const off = line.vertices.some(
        (v) => Math.abs((v[0] - ox) * sx + (v[1] - oy) * sy + (v[2] - oz) * sz) > epsPlane,
      );
      if (off) {
        hardErrors.push({
          elementId,
          interfaceId: iface.id,
          reason: `"${iface.name}": a digitized vertex is off its section plane (corrupt import?)`,
        });
      }
    }

    // Co-planarity of the whole digitized set (vs line 0's plane).
    const [ox, oy, oz] = lines[0].frame.origin;
    const [sx, sy, sz] = lines[0].frame.s;
    const allOnOnePlane = lines.every((l) =>
      l.vertices.every((v) => Math.abs((v[0] - ox) * sx + (v[1] - oy) * sy + (v[2] - oz) * sz) <= epsPlane),
    );
    const hasOtherData = iface.points.length > 0 || iface.orientations.length > 0;
    if (allOnOnePlane && !hasOtherData) {
      // Rule 2 (hard): a single coplanar trace cannot define a bounded 3D surface (§11.5).
      hardErrors.push({
        elementId,
        interfaceId: iface.id,
        reason: `"${iface.name}": all digitized data lies on ONE section plane — digitize on ≥2 parallel sections (a single trace can't define a 3D surface)`,
      });
    } else if (allOnOnePlane) {
      // Rule 4a (soft): under-constrained — the off-section shape rides on the other data.
      softWarnings.push({
        elementId,
        interfaceId: iface.id,
        reason: `"${iface.name}": digitized lines are all on one section — ≥2 (ideally 3) sections determine the surface`,
      });
    } else {
      // Rule 4b (soft): parallel sections but NO across-section variation — the fit is a
      // pure extrusion. Cheap proxy: every line has line 0's vertex count and the same
      // in-plane profile within tolerance (in each line's own frame; u = [−sy, sx, 0]).
      const profile = (l: DigitizedLine): [number, number][] => {
        const u: [number, number, number] = [-l.frame.s[1], l.frame.s[0], 0];
        return l.vertices.map((v) => [
          (v[0] - l.frame.origin[0]) * u[0] + (v[1] - l.frame.origin[1]) * u[1],
          v[2],
        ]);
      };
      const p0 = profile(lines[0]);
      const identical =
        lines.length >= 2 &&
        lines.every((l) => l.vertices.length === p0.length) &&
        lines.slice(1).every((l) =>
          profile(l).every((p, i) => Math.abs(p[0] - p0[i][0]) <= epsPlane && Math.abs(p[1] - p0[i][1]) <= epsPlane),
        );
      if (identical) {
        softWarnings.push({
          elementId,
          interfaceId: iface.id,
          reason: `"${iface.name}": digitized lines have no across-section variation — the surface is a pure extrusion`,
        });
      }
    }

    // Rule 5 (soft + fix): inconsistent younging — a sibling line drawn the "wrong way"
    // describes a self-contradictory field. Reference = line 0's mean pole; an opposed
    // line gets the one-action "flip this line to match" (reverses its vertex order).
    const m0 = meanPole(lines[0].vertices, lines[0].frame.s);
    if (m0) {
      for (const line of lines.slice(1)) {
        const m = meanPole(line.vertices, line.frame.s);
        if (m && m0[0] * m[0] + m0[1] * m[1] + m0[2] * m[2] < 0) {
          softWarnings.push({
            elementId,
            interfaceId: iface.id,
            lineId: line.id,
            fix: "flipLine",
            reason: `"${iface.name}": a digitized line's drawn direction youngs the OPPOSITE way from its siblings`,
          });
        }
      }
    }

    // Rule 6 (soft): a self-crossing/back-tracking chain has an ambiguous tangent.
    for (const line of lines) {
      const u: [number, number, number] = [-line.frame.s[1], line.frame.s[0], 0];
      const pts2d = line.vertices.map(
        (v): [number, number] => [
          (v[0] - line.frame.origin[0]) * u[0] + (v[1] - line.frame.origin[1]) * u[1],
          v[2],
        ],
      );
      if (selfCrosses(pts2d)) {
        softWarnings.push({
          elementId,
          interfaceId: iface.id,
          lineId: line.id,
          reason: `"${iface.name}": a digitized line self-crosses — its direction (younging) is ambiguous`,
        });
      }
    }
  };

  for (const el of project.elements) {
    if (el.kind === "series") {
      if (el.interfaces.length === 0) {
        hardErrors.push({ elementId: el.id, reason: `"${el.name}" has no surfaces` });
      }
      el.interfaces.forEach((iface) => checkInterface(el.id, iface));
      const anyOrientation = el.interfaces.some((iface) => iface.orientations.length > 0);
      if (el.interfaces.length > 0 && !anyOrientation) {
        softWarnings.push({ elementId: el.id, reason: `"${el.name}" has no orientation — fit may be ambiguous` });
      }
    } else if (el.kind === "vein") {
      checkInterface(el.id, el.footWall);
      checkInterface(el.id, el.hangingWall);
    } else if (el.kind === "intrusion") {
      // "solid" mode uses only the outer envelope; the retained inner is not compiled.
      checkInterface(el.id, el.envelopeOuter);
      if ((el.mode ?? "shell") === "shell") checkInterface(el.id, el.envelopeInner);
    } else if (el.kind === "topography") {
      // topography — one ground surface
      checkInterface(el.id, el.ground);
    } else {
      // importedSurface — the geometry is the mesh; it is valid iff it carries .obj text.
      if (!el.objText || el.objText.trim().length === 0) {
        hardErrors.push({ elementId: el.id, reason: "imported surface has no mesh" });
      }
    }
  }

  return { hardErrors, softWarnings, canApply: hardErrors.length === 0 };
}

// ============================================================================
// T007 — persistence (contracts/project-model.md)
// ============================================================================

const STORAGE_PREFIX = "geoproj:";
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/** Debounced (~500 ms) localStorage write, keyed by project name. */
export function saveProject(p: Project): void {
  if (typeof localStorage === "undefined") return;
  if (saveTimer) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    try {
      localStorage.setItem(STORAGE_PREFIX + p.name, JSON.stringify(p));
      localStorage.setItem(STORAGE_PREFIX + "@last", p.name);
    } catch {
      // storage full / unavailable — autosave is best-effort for the testbed.
    }
  }, 500);
}

/** Restore the last-autosaved project on startup; null => start from a blank/seed template. */
export function loadProject(): Project | null {
  if (typeof localStorage === "undefined") return null;
  try {
    const last = localStorage.getItem(STORAGE_PREFIX + "@last");
    if (!last) return null;
    const raw = localStorage.getItem(STORAGE_PREFIX + last);
    if (!raw) return null;
    const p = JSON.parse(raw) as Project;
    return p.version === 1 ? p : null;
  } catch {
    return null;
  }
}

/** Slim a project for EXPORT: for any imported surface with an external `objPath`, drop the inline
 *  `objText` (the mesh lives as a file under `public/meshes/`; the loader re-fetches it). A mesh with
 *  NO `objPath` keeps its `objText` so it still round-trips self-contained. Purely for serialization —
 *  the in-memory / autosaved project is never slimmed (it needs `objText` to fit without a fetch). */
export function slimForExport(p: Project): Project {
  return {
    ...p,
    elements: p.elements.map((el) =>
      el.kind === "importedSurface" && el.objPath ? { ...el, objText: "" } : el,
    ),
  };
}

/** Download the project as `<name>.geoproj.json` (the demo's portable "save file"). */
export function exportProjectJson(p: Project): void {
  const blob = new Blob([JSON.stringify(slimForExport(p), null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `${p.name}.geoproj.json`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

/** Parse a `.geoproj.json` File back into a Project. Caller confirms-if-dirty before use. Any imported
 *  surface exported path-only (has `objPath`, empty `objText`) is RE-HYDRATED by fetching the mesh from
 *  the static `meshes/` dir — so a slimmed export is restored to a fittable project. A missing file
 *  throws with the offending path, so a broken reference is a clear error, not a silent blank render. */
export async function importProjectJson(file: File): Promise<Project> {
  const text = await file.text();
  const p = JSON.parse(text) as Project;
  if (p.version !== 1 || !Array.isArray(p.elements)) {
    throw new Error("not a valid .geoproj.json (version 1) file");
  }
  return hydrateMeshPaths(sanitizeSections(p));
}

/** Import-time guards for the feature-005 additive fields (contracts/project-json.md):
 *  malformed SectionSets are a readable error naming the set; digitized-line normals are
 *  renormalized (degenerate ⇒ error); a stale `view.section` is clamped/dropped (view-state
 *  is best-effort). An OFF-PLANE digitized vertex is deliberately NOT rejected here — it
 *  loads and surfaces as a hard validation error (blocks Apply, not load). Pre-005 files
 *  have none of these fields and pass through untouched. */
export function sanitizeSections(p: Project): Project {
  let sections = p.sections;
  if (sections !== undefined) {
    if (!Array.isArray(sections)) throw new Error("invalid project: `sections` is not an array");
    for (const set of sections) {
      const finite2 = (v: unknown): v is [number, number] =>
        Array.isArray(v) && v.length === 2 && v.every((n) => Number.isFinite(n));
      if (!finite2(set.a) || !finite2(set.b)) {
        throw new Error(`invalid section set "${set.name ?? set.id}": baseline a/b must be [x, y]`);
      }
      if (!Number.isInteger(set.count) || set.count < 1) {
        throw new Error(`invalid section set "${set.name ?? set.id}": count must be an integer ≥ 1`);
      }
      if (set.count > 1 && !(set.spacing > 0)) {
        throw new Error(`invalid section set "${set.name ?? set.id}": spacing must be > 0 for a serial set`);
      }
    }
  }
  // Renormalize digitized-line plane normals (horizontal + unit by contract).
  const elements = p.elements.map((el) => {
    const fixIface = (iface: InterfaceData): InterfaceData => {
      if (!iface.digitized) return iface;
      const digitized = iface.digitized.map((line) => {
        const [sx, sy] = line.frame.s;
        const n = Math.hypot(sx, sy);
        if (!(n > 0)) {
          throw new Error(`invalid digitized line on "${iface.name}": degenerate section normal`);
        }
        return { ...line, frame: { ...line.frame, s: [sx / n, sy / n, 0] as [number, number, number] } };
      });
      return { ...iface, digitized };
    };
    switch (el.kind) {
      case "series":
        return { ...el, interfaces: el.interfaces.map(fixIface) };
      case "vein":
        return { ...el, footWall: fixIface(el.footWall), hangingWall: fixIface(el.hangingWall) };
      case "intrusion":
        return { ...el, envelopeInner: fixIface(el.envelopeInner), envelopeOuter: fixIface(el.envelopeOuter) };
      case "topography":
        return { ...el, ground: fixIface(el.ground) };
      case "importedSurface":
        return el;
    }
  });
  // Clamp/drop a stale active section (best-effort view-state).
  let view = p.view;
  if (view?.section) {
    const set = sections?.find((s) => s.id === view!.section!.setId);
    if (!set) view = { ...view, section: undefined };
    else if (view.section.index < 0 || view.section.index >= set.count) {
      view = { ...view, section: { setId: set.id, index: Math.min(Math.max(view.section.index, 0), set.count - 1) } };
    }
  }
  return { ...p, sections, elements, view };
}

/** Fetch `objText` for any imported surface that carries an `objPath` but no inline text (a slimmed
 *  export). Resolved against the app's base URL (`meshes/<file>` → `<base>meshes/<file>`). */
export async function hydrateMeshPaths(p: Project): Promise<Project> {
  const base = (import.meta as { env?: { BASE_URL?: string } }).env?.BASE_URL ?? "/";
  const elements = await Promise.all(
    p.elements.map(async (el) => {
      if (el.kind !== "importedSurface" || !el.objPath || (el.objText && el.objText.length > 0)) {
        return el;
      }
      const url = base.replace(/\/$/, "") + "/" + el.objPath.replace(/^\//, "");
      const res = await fetch(url).catch(() => null);
      const ct = res?.headers.get("content-type") ?? "";
      // A dev/preview SPA server answers unknown routes with index.html (200 OK) — so a MISSING mesh
      // must be caught by content, not just status: reject an HTML body or one with no OBJ vertices.
      const body = res && res.ok ? await res.text() : null;
      if (body == null || ct.includes("text/html") || !/^\s*v\s/m.test(body)) {
        throw new Error(`mesh not found for "${el.name}": ${el.objPath} (expected an .obj under public/${el.objPath})`);
      }
      return { ...el, objText: body };
    }),
  );
  return { ...p, elements };
}

// ============================================================================
// id helpers — stable client ids without Date.now()/Math.random() dependence on order
// ============================================================================
let idCounter = 0;
export function newId(prefix = "id"): string {
  idCounter += 1;
  return `${prefix}-${idCounter}`;
}
