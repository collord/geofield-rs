// Feature 005 (cross-sections) — the digitized-line YOUNGING/POLE derivation, transcribed
// from specs/005-cross-section-digitizing/contracts/younging-derivation.md §2–§4. This is
// the SINGLE home of the rule (Constitution III): the Viewer's younging affordance, the
// compile path (project.ts flattenInterfaces), and validate() all derive from here.
//
// NORMATIVE RULE (spec FR-013): viewed face-on (section normal s toward the viewer, Z up),
// the surface youngs to the LEFT of the drawn direction — pole m = ŝ × t̂. The source
// fragment's `t × s` was a sign slip (see the contract's worked example); ŝ × t̂ is correct.
//
// Structurally typed on purpose (no imports): keeps section/ dependency-free and pure.

type V3 = [number, number, number];

function sub(a: V3, b: V3): V3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
}
function cross(a: V3, b: V3): V3 {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}
function norm(a: V3): number {
  return Math.hypot(a[0], a[1], a[2]);
}
function unit(a: V3): V3 | null {
  const n = norm(a);
  return n > 0 ? [a[0] / n, a[1] / n, a[2] / n] : null;
}

/** One derived orientation sample: per line SEGMENT (research.md R4 — a segment has an
 *  unambiguous tangent), positioned at the segment midpoint. `pole` is the UNIT vector to
 *  the younger side, BEFORE the per-surface flip sign (contract §3 applies that at the one
 *  existing XOR site in flattenInterfaces). */
export interface DerivedOrientation {
  pos: V3;
  pole: V3;
}

/** Per-segment poles of a digitized line on a plane with unit normal `s`:
 *  t̂ = segment direction (drawn order), m = ŝ × t̂. Zero-length segments contribute
 *  nothing (contract §4); a 2-vertex line yields exactly 1 sample. */
export function derivePoles(vertices: V3[], s: V3, minSegLen = 0): DerivedOrientation[] {
  const out: DerivedOrientation[] = [];
  for (let i = 0; i + 1 < vertices.length; i++) {
    const a = vertices[i];
    const b = vertices[i + 1];
    const seg = sub(b, a);
    if (norm(seg) <= minSegLen) continue;
    const t = unit(seg);
    if (!t) continue;
    const m = unit(cross(s, t));
    if (!m) continue; // t parallel to s can't happen for an in-plane segment, but stay safe
    out.push({ pos: [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2], pole: m });
  }
  return out;
}

/** The line's mean pole (unit), for the sibling-consistency check (validate() rule 5):
 *  two lines of one surface with opposed mean poles describe a self-contradictory field.
 *  Null for a line with no valid segment. */
export function meanPole(vertices: V3[], s: V3): V3 | null {
  const poles = derivePoles(vertices, s);
  if (poles.length === 0) return null;
  const sum: V3 = [0, 0, 0];
  for (const p of poles) {
    sum[0] += p.pole[0];
    sum[1] += p.pole[1];
    sum[2] += p.pole[2];
  }
  return unit(sum);
}

// --- in-plane self-intersection (validate() rule 6: "ambiguous direction") ---------------

type P2 = [number, number];

function segsIntersect(a: P2, b: P2, c: P2, d: P2): boolean {
  const o = (p: P2, q: P2, r: P2) => Math.sign((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]));
  const o1 = o(a, b, c);
  const o2 = o(a, b, d);
  const o3 = o(c, d, a);
  const o4 = o(c, d, b);
  return o1 !== o2 && o3 !== o4 && o1 !== 0 && o2 !== 0 && o3 !== 0 && o4 !== 0;
}

/** True when the drawn chain self-crosses (non-adjacent segment pairs intersect) — the
 *  drawn order gives an ambiguous tangent field. Warned, never blocked (spec FR-020). */
export function selfCrosses(points2d: P2[]): boolean {
  for (let i = 0; i + 1 < points2d.length; i++) {
    for (let j = i + 2; j + 1 < points2d.length; j++) {
      if (segsIntersect(points2d[i], points2d[i + 1], points2d[j], points2d[j + 1])) return true;
    }
  }
  return false;
}
