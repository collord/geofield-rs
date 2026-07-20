// Drillhole database import (prototype): a pasted POST-DESURVEY lithology interval table →
// contact points at unit transitions → an auto-built conformable stack.
//
// The table is desurveyed upstream (an external library does the collar/survey math), so every
// interval arrives with its 3D endpoints and contact extraction is pure bookkeeping: within a
// hole, each unit transition's contact point is the shared interval boundary (`prev.end` ≈
// `next.start`), grouped under the LOWER unit as its "top of <unit>" surface (the standard
// deposit-contact grouping). Unit chronology is inferred from superposition (who-is-above-whom
// votes across holes) with a mean-elevation fallback. Everything here is pure — the modal calls
// `parseDrillholeTable` live and `applyDrillholeImport` on Apply; the React/commit wiring lives
// in main.tsx.

import type { DrillholeInterval, InterfaceData, PointRow, Project, ProjectElement } from "./project";
import { newId } from "./project";
import { catalogOf, unitKey } from "./projectOps";

export interface DrillholeParseResult {
  intervals: DrillholeInterval[];
  /** Skipped data rows, flagged (never silently dropped — mirrors tables.tsx parsePaste). */
  flagged: { line: number; raw: string; reason: string }[];
  /** Fatal problems (missing required columns / no data rows): Apply must stay disabled. */
  errors: string[];
}

/** Header synonyms, matched after lowercasing + stripping non-alphanumerics. */
const HEADER_SYNONYMS: Record<string, string[]> = {
  holeId: ["holeid", "hole", "bhid", "dhid"],
  unit: ["unitname", "unit", "lith", "lithology", "geol", "rock"],
  sx: ["startx", "fromx", "xstart", "xfrom", "x1"],
  sy: ["starty", "fromy", "ystart", "yfrom", "y1"],
  sz: ["startz", "fromz", "zstart", "zfrom", "z1"],
  ex: ["endx", "tox", "xend", "xto", "x2"],
  ey: ["endy", "toy", "yend", "yto", "y2"],
  ez: ["endz", "toz", "zend", "zto", "z2"],
};

/** Tokenize one line with DELIMITER PRIORITY: tab, else comma, else whitespace. (A deliberate
 *  departure from tables.tsx parsePaste's `\s+` fallback, which would split a multi-word unit
 *  name like "Cover sand" pasted from a comma/tab table.) */
function splitLine(line: string): string[] {
  const cells = line.includes("\t") ? line.split("\t") : line.includes(",") ? line.split(",") : line.split(/\s+/);
  return cells.map((c) => c.trim());
}

/** Parse a pasted interval table: first non-empty line is the header (synonym + order tolerant,
 *  unrecognized columns like geolfrom/geolto ignored), the rest are data rows. */
export function parseDrillholeTable(text: string): DrillholeParseResult {
  const lines = text.split(/\r?\n/);
  const flagged: DrillholeParseResult["flagged"] = [];
  const errors: string[] = [];

  let headerIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() !== "") {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx < 0) return { intervals: [], flagged, errors: ["empty input"] };

  const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]/g, "");
  const headerCells = splitLine(lines[headerIdx]).map(norm);
  const col: Record<string, number> = {};
  for (const [field, synonyms] of Object.entries(HEADER_SYNONYMS)) {
    const idx = headerCells.findIndex((h) => synonyms.includes(h));
    if (idx < 0) errors.push(`missing column: ${synonyms[0]}`);
    else col[field] = idx;
  }
  if (errors.length > 0) return { intervals: [], flagged, errors };
  const maxIdx = Math.max(...Object.values(col));

  const intervals: DrillholeInterval[] = [];
  for (let i = headerIdx + 1; i < lines.length; i++) {
    const raw = lines[i];
    if (raw.trim() === "") continue;
    const cells = splitLine(raw);
    if (cells.length <= maxIdx) {
      flagged.push({ line: i + 1, raw, reason: `expected ≥${maxIdx + 1} columns, got ${cells.length}` });
      continue;
    }
    const holeId = cells[col.holeId];
    const unit = cells[col.unit];
    const nums = (["sx", "sy", "sz", "ex", "ey", "ez"] as const).map((f) => Number(cells[col[f]]));
    if (holeId === "" || unit === "") {
      flagged.push({ line: i + 1, raw, reason: "empty holeid or unit name" });
      continue;
    }
    if (nums.some((n) => !Number.isFinite(n))) {
      flagged.push({ line: i + 1, raw, reason: "non-numeric coordinate" });
      continue;
    }
    const [sx, sy, sz, ex, ey, ez] = nums;
    intervals.push({ holeId, unit, start: { x: sx, y: sy, z: sz }, end: { x: ex, y: ey, z: ez } });
  }
  if (intervals.length === 0) errors.push("no valid data rows");
  return { intervals, flagged, errors };
}

/** Bounding-box diagonal of all interval endpoints (0 for a single point). */
function bboxDiagonal(intervals: DrillholeInterval[]): number {
  let lo = [Infinity, Infinity, Infinity];
  let hi = [-Infinity, -Infinity, -Infinity];
  for (const it of intervals) {
    for (const p of [it.start, it.end]) {
      lo = [Math.min(lo[0], p.x), Math.min(lo[1], p.y), Math.min(lo[2], p.z)];
      hi = [Math.max(hi[0], p.x), Math.max(hi[1], p.y), Math.max(hi[2], p.z)];
    }
  }
  return Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
}

/** Extract contact points at within-hole unit transitions, grouped under the LOWER unit ("top of
 *  <lower>"). Intervals are taken in TABLE ORDER per hole (a from/to log is ordered downhole);
 *  a boundary where `prev.end` and `next.start` disagree beyond `tol` contributes the midpoint
 *  and a warning (a gap/overlap in the log). Same-unit consecutive intervals yield no contact. */
export function extractContacts(
  intervals: DrillholeInterval[],
  tol?: number,
): { byUnit: Map<string, PointRow[]>; warnings: string[] } {
  const t = tol ?? Math.max(bboxDiagonal(intervals) * 1e-4, 1e-9);
  const byHole = new Map<string, DrillholeInterval[]>();
  for (const it of intervals) {
    let list = byHole.get(it.holeId);
    if (!list) byHole.set(it.holeId, (list = []));
    list.push(it);
  }
  const byUnit = new Map<string, PointRow[]>();
  const warnings: string[] = [];
  for (const [holeId, list] of byHole) {
    for (let i = 0; i + 1 < list.length; i++) {
      const prev = list[i];
      const next = list[i + 1];
      if (prev.unit === next.unit) continue;
      const gap = Math.hypot(next.start.x - prev.end.x, next.start.y - prev.end.y, next.start.z - prev.end.z);
      const p: PointRow =
        gap > t
          ? {
              x: (prev.end.x + next.start.x) / 2,
              y: (prev.end.y + next.start.y) / 2,
              z: (prev.end.z + next.start.z) / 2,
            }
          : { x: prev.end.x, y: prev.end.y, z: prev.end.z };
      if (gap > t) {
        warnings.push(`hole ${holeId}: ${prev.unit}/${next.unit} boundary endpoints differ by ${gap.toFixed(2)} — using midpoint`);
      }
      let pts = byUnit.get(next.unit);
      if (!pts) byUnit.set(next.unit, (pts = []));
      pts.push(p);
    }
  }
  return { byUnit, warnings };
}

/** Infer unit chronology (youngest → oldest) from superposition: every ordered pair in a hole's
 *  collapsed unit sequence votes "above" (ALL pairs, not just adjacent, so a hole where a middle
 *  unit pinches out still orders the outer two). Majority-direction edges feed Kahn's topological
 *  sort with a mean-interval-elevation tiebreak; a cycle (contradictory logs) is broken at the
 *  highest mean elevation and flagged. Units with no votes slot in by mean elevation. */
export function inferUnitOrder(intervals: DrillholeInterval[]): { order: string[]; hadCycle: boolean } {
  // Mean interval-midpoint elevation per unit (the fallback/tiebreak key).
  const zSum = new Map<string, { sum: number; n: number }>();
  for (const it of intervals) {
    const e = zSum.get(it.unit) ?? { sum: 0, n: 0 };
    e.sum += (it.start.z + it.end.z) / 2;
    e.n += 1;
    zSum.set(it.unit, e);
  }
  const meanZ = (u: string) => {
    const e = zSum.get(u)!;
    return e.sum / e.n;
  };
  const units = [...zSum.keys()];

  // Superposition votes: net[a][b] > 0 ⇒ a above b (majority across holes).
  const votes = new Map<string, number>(); // "a b" → count of (a above b)
  const byHole = new Map<string, string[]>();
  for (const it of intervals) {
    let seq = byHole.get(it.holeId);
    if (!seq) byHole.set(it.holeId, (seq = []));
    if (seq[seq.length - 1] !== it.unit) seq.push(it.unit);
  }
  for (const seq of byHole.values()) {
    for (let i = 0; i < seq.length; i++) {
      for (let j = i + 1; j < seq.length; j++) {
        if (seq[i] === seq[j]) continue;
        const key = seq[i] + " " + seq[j];
        votes.set(key, (votes.get(key) ?? 0) + 1);
      }
    }
  }
  const above = new Map<string, Set<string>>(); // a → set of b it is above (majority edges)
  const indeg = new Map<string, number>(units.map((u) => [u, 0]));
  for (const a of units) {
    for (const b of units) {
      if (a >= b) continue;
      const ab = votes.get(a + " " + b) ?? 0;
      const ba = votes.get(b + " " + a) ?? 0;
      if (ab === ba) continue; // tie or unvoted → no edge
      const [hi, lo] = ab > ba ? [a, b] : [b, a];
      let set = above.get(hi);
      if (!set) above.set(hi, (set = new Set()));
      set.add(lo);
      indeg.set(lo, (indeg.get(lo) ?? 0) + 1);
    }
  }

  // Kahn with highest-mean-elevation tiebreak; cycles broken at the highest remaining elevation.
  const remaining = new Set(units);
  const order: string[] = [];
  let hadCycle = false;
  while (remaining.size > 0) {
    const ready = [...remaining].filter((u) => (indeg.get(u) ?? 0) === 0);
    let pick: string;
    if (ready.length > 0) {
      pick = ready.reduce((best, u) => (meanZ(u) > meanZ(best) ? u : best));
    } else {
      hadCycle = true;
      pick = [...remaining].reduce((best, u) => (meanZ(u) > meanZ(best) ? u : best));
    }
    order.push(pick);
    remaining.delete(pick);
    for (const b of above.get(pick) ?? []) {
      if (remaining.has(b)) indeg.set(b, (indeg.get(b) ?? 0) - 1);
    }
  }
  return { order, hadCycle };
}

/** The model extent fitted to the data: padded bbox, with a floor per axis so degenerate data
 *  (e.g. all holes on one section line) still yields a usable prism. */
export function dataExtent(
  intervals: DrillholeInterval[],
  padFraction = 0.1,
): [number, number, number, number, number, number] {
  let lo = [Infinity, Infinity, Infinity];
  let hi = [-Infinity, -Infinity, -Infinity];
  for (const it of intervals) {
    for (const p of [it.start, it.end]) {
      lo = [Math.min(lo[0], p.x), Math.min(lo[1], p.y), Math.min(lo[2], p.z)];
      hi = [Math.max(hi[0], p.x), Math.max(hi[1], p.y), Math.max(hi[2], p.z)];
    }
  }
  const diag = Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
  const out: number[] = [];
  for (let a = 0; a < 3; a++) {
    const pad = Math.max(padFraction * (hi[a] - lo[a]), 0.01 * diag, 1);
    out.push(lo[a] - pad, hi[a] + pad);
  }
  return [out[0], out[1], out[2], out[3], out[4], out[5]];
}

export interface DrillholeImportSummary {
  /** Inferred chronology, youngest → oldest. */
  unitOrder: string[];
  /** Names of the created "top of <unit>" elements, youngest first. */
  createdElements: string[];
  /** Contact-point count per created surface name. */
  contactCounts: Record<string, number>;
  warnings: string[];
}

/** Earth-tone lithology palette (the template catalog hexes, scenarios.ts). */
const LITHO_PALETTE = ["#d9b06a", "#6f7d55", "#9fb1c4", "#7a6b5d", "#c0554e", "#e6d18a", "#b7c99a", "#8a63b8"];

/** Build the imported stack into a NEW project object (pure; caller commits):
 *  - replaces prior same-named `top of <unit>` series (the re-paste loop) and their assignments,
 *  - matches/creates lithologies by unit name,
 *  - creates one conformable single-interface series per unit that has contacts (youngest first,
 *    appended at the OLDEST end — on a blank project that is the whole stack), each with ONE
 *    auto orientation (dip 0 at the points' centroid: the co-kriged fit needs an orientation to
 *    pin the field gradient; the user refines it in the surface modal),
 *  - assigns each element's unit 0 (its below-contact slab) to the lower unit's lithology,
 *  - refits `extent` to the padded data bbox and stores the raw `drillholes` intervals. */
export function applyDrillholeImport(
  project: Project,
  intervals: DrillholeInterval[],
): { project: Project; summary: DrillholeImportSummary } {
  const { order, hadCycle } = inferUnitOrder(intervals);
  const { byUnit, warnings } = extractContacts(intervals);
  if (hadCycle) warnings.push("contradictory unit order across holes — broke the tie by mean elevation");

  // Lithologies: match case-insensitively by name, else create from the palette.
  const catalog = catalogOf(project);
  const lithologies = [...catalog.lithologies];
  const lithoIdByUnit = new Map<string, string>();
  for (const unit of order) {
    const existing = lithologies.find((l) => l.name.toLowerCase() === unit.toLowerCase());
    if (existing) {
      lithoIdByUnit.set(unit, existing.id);
    } else {
      const id = newId("litho");
      lithologies.push({
        id,
        name: unit,
        color: LITHO_PALETTE[lithoIdByUnit.size % LITHO_PALETTE.length],
        kind: "user",
      });
      lithoIdByUnit.set(unit, id);
    }
  }

  // Elements: one per unit that has contacts, youngest-first (the youngest unit's own top is
  // unobserved — no element; its volume stays Air above the top contact for now).
  const surfacedUnits = order.filter((u) => (byUnit.get(u)?.length ?? 0) > 0);
  const surfaceName = (unit: string) => `top of ${unit}`;
  const newElements: ProjectElement[] = [];
  const assignment: Record<string, string> = { ...catalog.assignment };
  const contactCounts: Record<string, number> = {};
  for (const unit of surfacedUnits) {
    const points = byUnit.get(unit)!;
    const c = points.reduce((a, p) => ({ x: a.x + p.x, y: a.y + p.y, z: a.z + p.z }), { x: 0, y: 0, z: 0 });
    const iface: InterfaceData = {
      id: newId("iface"),
      name: surfaceName(unit),
      points,
      orientations: [{ x: c.x / points.length, y: c.y / points.length, z: c.z / points.length, dip: 0, dipDir: 0 }],
    };
    const el: ProjectElement = {
      kind: "series",
      id: newId("el"),
      name: surfaceName(unit),
      relation: "conformable",
      unboundedAbove: false,
      interfaces: [iface],
    };
    newElements.push(el);
    assignment[unitKey(el.id, 0)] = lithoIdByUnit.get(unit)!;
    contactCounts[surfaceName(unit)] = points.length;
    if (points.length < 2) {
      warnings.push(`${surfaceName(unit)} has only ${points.length} contact point — the fit may collapse; add points`);
    }
  }

  // Replace-on-reimport: drop prior series with the same generated names (and their assignments).
  const incoming = new Set(surfacedUnits.map(surfaceName));
  const kept = project.elements.filter((el) => !(el.kind === "series" && incoming.has(el.name)));
  for (const el of project.elements) {
    if (el.kind === "series" && incoming.has(el.name)) {
      for (const key of Object.keys(assignment)) {
        if (key.startsWith(el.id + ":")) delete assignment[key];
      }
    }
  }

  const next: Project = {
    ...project,
    elements: [...kept, ...newElements],
    catalog: { lithologies, assignment },
    extent: dataExtent(intervals),
    drillholes: intervals,
  };
  return {
    project: next,
    summary: { unitOrder: order, createdElements: newElements.map((e) => e.name), contactCounts, warnings },
  };
}
