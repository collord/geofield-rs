// Feature 005 (cross-sections) — the SECTION FRAME math, transcribed from
// specs/005-cross-section-digitizing/contracts/younging-derivation.md §1. This module is
// PURE and viewer-agnostic (no React/three imports): the Viewer, the compile path, and
// validation all read the same functions, so the frame rule lives exactly once.
//
// Coverage bound (Principle IV, research.md R9): web3 has no unit-test framework; these
// helpers are pinned through the Playwright e2e hooks (__sections / __sectionTraces), not
// by native unit tests.

import type { SectionSet } from "../HierarchyPanel/project";

export type Vec3 = [number, number, number];

/** The derived frame of ONE section plane (plane k of a set). All vectors unit:
 *  `u` = in-plane horizontal (baseline direction, "screen right" face-on), `w` = +Z
 *  ("screen up"), `s = u × w` = out-of-plane normal — s points TOWARD the face-on viewer
 *  (the camera sits on the s side looking −s). `origin` lies on the plane at z = 0. */
export interface SectionFrame {
  origin: Vec3;
  u: Vec3;
  w: Vec3; // always [0,0,1]
  s: Vec3;
  /** Baseline length |b − a| (world units) — the plane's u-extent for framing. */
  uLen: number;
}

/** Reject a degenerate two-click baseline (spec edge case): shorter than this fraction of
 *  the extent diagonal, the gesture ignores the second click. */
export const BASELINE_MIN_FRACTION = 1e-3;

/** On-plane tolerance for digitized vertices, as a fraction of the extent diagonal
 *  (data-model.md invariants; guarded on import, FR-019). */
export const PLANE_EPS_FRACTION = 1e-6;

export function extentDiagonal(extent: [number, number, number, number, number, number]): number {
  const [x0, x1, y0, y1, z0, z1] = extent;
  return Math.hypot(x1 - x0, y1 - y0, z1 - z0) || 1;
}

/** u and s from a plan-view baseline A→B. Contract §1: u = normalize([Bx−Ax, By−Ay, 0]),
 *  s = u × w with w = +Z — concretely s = [uy, −ux, 0] (horizontal, unit). Throws on a
 *  zero-length baseline (the gesture guards before calling). */
export function frameFromBaseline(a: [number, number], b: [number, number]): { u: Vec3; s: Vec3; uLen: number } {
  const dx = b[0] - a[0];
  const dy = b[1] - a[1];
  const len = Math.hypot(dx, dy);
  if (!(len > 0)) throw new Error("degenerate section baseline (A = B)");
  const u: Vec3 = [dx / len, dy / len, 0];
  // s = u × w, w = (0,0,1): (ux,uy,0)×(0,0,1) = (uy·1−0, 0−ux·1, 0) = (uy, −ux, 0).
  const s: Vec3 = [u[1], -u[0], 0];
  return { u, s, uLen: len };
}

/** The frame of plane `k` (0-based) of a set: through `a + s·(offset + k·spacing)`,
 *  normal s, origin at z = 0 (so the in-plane vertical coordinate dw ≡ world z). */
export function planeK(set: SectionSet, k: number): SectionFrame {
  const { u, s, uLen } = frameFromBaseline(set.a, set.b);
  const d = (set.offset ?? 0) + k * set.spacing;
  const origin: Vec3 = [set.a[0] + s[0] * d, set.a[1] + s[1] * d, 0];
  return { origin, u, w: [0, 0, 1], s, uLen };
}

/** Signed out-of-plane distance of a world point (positive on the s / viewer side). */
export function distToPlane(frame: SectionFrame, p: Vec3): number {
  return (p[0] - frame.origin[0]) * frame.s[0] + (p[1] - frame.origin[1]) * frame.s[1] + (p[2] - frame.origin[2]) * frame.s[2];
}

/** World → in-plane (du, dw). du along the (offset) baseline from its start; dw ≡ z. */
export function toInPlane(frame: SectionFrame, p: Vec3): [number, number] {
  const du = (p[0] - frame.origin[0]) * frame.u[0] + (p[1] - frame.origin[1]) * frame.u[1];
  return [du, p[2]];
}

/** In-plane (du, dw) → world (exactly on the plane). */
export function fromInPlane(frame: SectionFrame, du: number, dw: number): Vec3 {
  return [frame.origin[0] + frame.u[0] * du, frame.origin[1] + frame.u[1] * du, dw];
}

/** Project a world point onto a plane (kill its out-of-plane component). Takes the
 *  structural `{origin, s}` subset so a DigitizedLine's stored frame snapshot works
 *  directly. Used when a points-table edit moves a digitized vertex: the vertex stays ON
 *  its line's plane (data-model.md — the documented resolution of the table/plane tension). */
export function projectToPlane(plane: { origin: Vec3; s: Vec3 }, p: Vec3): Vec3 {
  const d =
    (p[0] - plane.origin[0]) * plane.s[0] +
    (p[1] - plane.origin[1]) * plane.s[1] +
    (p[2] - plane.origin[2]) * plane.s[2];
  return [p[0] - plane.s[0] * d, p[1] - plane.s[1] * d, p[2] - plane.s[2] * d];
}

/** Backdrop slab tolerance (research.md R8): authored points within this out-of-plane
 *  distance are drawn in the section view. Half the spacing for a serial set (each point
 *  appears on exactly one nearest slice); 2.5% of the extent diagonal for a standalone. */
export function slabTol(set: SectionSet, extent: [number, number, number, number, number, number]): number {
  if (set.count > 1 && set.spacing > 0) return set.spacing / 2;
  return 0.025 * extentDiagonal(extent);
}
