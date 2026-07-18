// Feature 005 (cross-sections) — plane ∩ mesh slicing, entirely UI-side (research.md R1/R2):
// the solver worker is terminate-per-fit, so sections re-slice the ALREADY-RETURNED mesh
// payloads on the main thread — zero engine calls, which is what lets section create/step
// never re-fit (spec FR-009). Pure module: typed arrays in, polylines/loops out; no three.js.
//
// Two consumers:
//  • contact mode — `sectionPolylines`: open traces where each surface sheet crosses the plane;
//  • solid mode  — `sectionLoops` + `classifyLoops`: closed outlines of each unit solid on the
//    plane, holes classified even-odd, ready for a 2D fill triangulation.
//
// Coverage bound (Principle IV, research.md R2): the solid FILL requires watertight input.
// With the diagnostic Manifold toggle OFF (legacy field-domain solids) loop assembly may not
// close; unclosed chains are returned in `open` and are drawn as unfilled boundary segments
// — a logged fallback, not a defect.

import { type SectionFrame, toInPlane } from "./frame";

type P2 = [number, number];

/** One plane-crossing segment of one triangle, in-plane coordinates. */
interface Seg {
  a: P2;
  b: P2;
}

/** All plane-crossing segments of an indexed triangle mesh, in-plane (du, dw).
 *  Per-triangle walk: signed distances of the 3 vertices; an edge with opposite signs
 *  contributes one interpolated crossing; a triangle crossing the plane yields 2 points.
 *  Degenerate grazes (a vertex exactly on the plane) are nudged by ±eps so every crossing
 *  triangle still yields a clean 2-point segment. */
export function sectionSegments(
  positions: ArrayLike<number>,
  indices: ArrayLike<number>,
  frame: SectionFrame,
  eps: number,
): Seg[] {
  const segs: Seg[] = [];
  const ox = frame.origin[0];
  const oy = frame.origin[1];
  const oz = frame.origin[2];
  const sx = frame.s[0];
  const sy = frame.s[1];
  const sz = frame.s[2];
  const nTri = Math.floor(indices.length / 3);
  const d = [0, 0, 0];
  const vx = [0, 0, 0];
  const vy = [0, 0, 0];
  const vz = [0, 0, 0];
  for (let t = 0; t < nTri; t++) {
    for (let c = 0; c < 3; c++) {
      const i = indices[3 * t + c] * 3;
      vx[c] = positions[i];
      vy[c] = positions[i + 1];
      vz[c] = positions[i + 2];
      let dd = (vx[c] - ox) * sx + (vy[c] - oy) * sy + (vz[c] - oz) * sz;
      if (Math.abs(dd) < eps) dd = eps; // nudge on-plane vertices to one side (stable grazes)
      d[c] = dd;
    }
    if ((d[0] > 0) === (d[1] > 0) && (d[1] > 0) === (d[2] > 0)) continue; // no crossing
    const pts: P2[] = [];
    for (let c = 0; c < 3; c++) {
      const n = (c + 1) % 3;
      if ((d[c] > 0) === (d[n] > 0)) continue;
      const t01 = d[c] / (d[c] - d[n]);
      const px = vx[c] + (vx[n] - vx[c]) * t01;
      const py = vy[c] + (vy[n] - vy[c]) * t01;
      const pz = vz[c] + (vz[n] - vz[c]) * t01;
      pts.push(toInPlane(frame, [px, py, pz]));
    }
    if (pts.length === 2) segs.push({ a: pts[0], b: pts[1] });
  }
  return segs;
}

/** Chain raw segments into polylines by snapping endpoints to a `quantum` grid. Closed
 *  chains (end meets start) are flagged. Zero-length segments are dropped. */
export function chainSegments(
  segs: Seg[],
  quantum: number,
): { points: P2[]; closed: boolean }[] {
  const key = (p: P2) => `${Math.round(p[0] / quantum)}:${Math.round(p[1] / quantum)}`;
  // endpoint key → list of (segment index, which end)
  const ends = new Map<string, { seg: number; end: 0 | 1 }[]>();
  segs.forEach((s, i) => {
    if (key(s.a) === key(s.b)) return; // degenerate
    for (const end of [0, 1] as const) {
      const k = key(end === 0 ? s.a : s.b);
      let list = ends.get(k);
      if (!list) ends.set(k, (list = []));
      list.push({ seg: i, end });
    }
  });
  const used = new Array(segs.length).fill(false);
  const out: { points: P2[]; closed: boolean }[] = [];
  for (let i = 0; i < segs.length; i++) {
    if (used[i] || key(segs[i].a) === key(segs[i].b)) continue;
    used[i] = true;
    const chain: P2[] = [segs[i].a, segs[i].b];
    // grow forward from the tail, then backward from the head
    for (const dir of [1, -1] as const) {
      for (;;) {
        const tip = dir === 1 ? chain[chain.length - 1] : chain[0];
        const cands = ends.get(key(tip)) ?? [];
        const next = cands.find((c) => !used[c.seg]);
        if (!next) break;
        used[next.seg] = true;
        const s = segs[next.seg];
        const far = next.end === 0 ? s.b : s.a;
        if (dir === 1) chain.push(far);
        else chain.unshift(far);
        if (key(chain[0]) === key(chain[chain.length - 1]) && chain.length > 2) break; // closed
      }
    }
    const closed = chain.length > 2 && key(chain[0]) === key(chain[chain.length - 1]);
    if (closed) chain.pop(); // drop the duplicated closing point
    out.push({ points: chain, closed });
  }
  return out;
}

/** Contact-mode traces: the in-plane polylines where one surface payload crosses the plane. */
export function sectionPolylines(
  positions: ArrayLike<number>,
  indices: ArrayLike<number>,
  frame: SectionFrame,
  quantum: number,
): P2[][] {
  const segs = sectionSegments(positions, indices, frame, quantum * 1e-3);
  return chainSegments(segs, quantum).map((c) => c.points);
}

/** Solid-mode outlines: closed loops (fillable) and open chains (the Manifold-off
 *  watertightness fallback — drawn unfilled, see module note). */
export function sectionLoops(
  positions: ArrayLike<number>,
  indices: ArrayLike<number>,
  frame: SectionFrame,
  quantum: number,
): { closed: P2[][]; open: P2[][] } {
  const segs = sectionSegments(positions, indices, frame, quantum * 1e-3);
  const chains = chainSegments(segs, quantum);
  return {
    closed: chains.filter((c) => c.closed).map((c) => c.points),
    open: chains.filter((c) => !c.closed).map((c) => c.points),
  };
}

function signedArea(loop: P2[]): number {
  let a = 0;
  for (let i = 0; i < loop.length; i++) {
    const p = loop[i];
    const q = loop[(i + 1) % loop.length];
    a += p[0] * q[1] - q[0] * p[1];
  }
  return a / 2;
}

function pointInLoop(pt: P2, loop: P2[]): boolean {
  // even-odd ray cast along +du
  let inside = false;
  for (let i = 0, j = loop.length - 1; i < loop.length; j = i++) {
    const pi = loop[i];
    const pj = loop[j];
    if (pi[1] > pt[1] !== pj[1] > pt[1]) {
      const x = pj[0] + ((pt[1] - pj[1]) / (pi[1] - pj[1])) * (pi[0] - pj[0]);
      if (pt[0] < x) inside = !inside;
    }
  }
  return inside;
}

/** Even-odd hole classification: loops at even containment depth are OUTER shapes; each
 *  odd-depth loop becomes a hole of its immediate (depth−1) container. Winding is
 *  normalized: outers CCW (area > 0), holes CW — what a 2D fill triangulator expects. */
export function classifyLoops(loops: P2[][]): { outer: P2[]; holes: P2[][] }[] {
  const meaningful = loops.filter((l) => l.length >= 3);
  const depth = meaningful.map((l, i) => {
    let d = 0;
    for (let j = 0; j < meaningful.length; j++) {
      if (j !== i && pointInLoop(l[0], meaningful[j])) d++;
    }
    return d;
  });
  const out: { outer: P2[]; holes: P2[][] }[] = [];
  const outerIndex = new Map<number, number>(); // loop idx → out[] idx
  meaningful.forEach((l, i) => {
    if (depth[i] % 2 === 0) {
      const outer = signedArea(l) < 0 ? [...l].reverse() : l;
      outerIndex.set(i, out.length);
      out.push({ outer, holes: [] });
    }
  });
  meaningful.forEach((l, i) => {
    if (depth[i] % 2 !== 1) return;
    // immediate container: the containing even-depth loop with depth = depth[i] − 1
    let best = -1;
    for (let j = 0; j < meaningful.length; j++) {
      if (j !== i && depth[j] === depth[i] - 1 && pointInLoop(l[0], meaningful[j])) {
        best = j;
        break;
      }
    }
    if (best >= 0 && outerIndex.has(best)) {
      const hole = signedArea(l) > 0 ? [...l].reverse() : l;
      out[outerIndex.get(best)!].holes.push(hole);
    }
  });
  return out;
}
