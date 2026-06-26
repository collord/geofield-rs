//! One exact triangle–triangle intersection, end-to-end — the Stage 0 determinism spike (docs/12 §2).
//!
//! This mirrors the *combinatorics* of [`mesh_clip::tri_tri_intersection`] but replaces its single
//! f64 construction primitive ([`mesh_clip::edge_cross`], `t = da/(da−db); a + t·(b−a)`) with an
//! **exact** construction: each segment endpoint is the exact rational solution of the edge/plane
//! system, snapshotted to f64 only at the very end. Two consequences fall out for free, and they are
//! the whole point:
//!
//!   * The f64 path's tolerances **vanish**. Parallel is `n_c × n_w == 0` *exactly* (no
//!     `COPLANAR_SIN2`); a graze is `hi_s == lo_s` *exactly* (no `seg_len_eps`); a vertex is
//!     above/on/below a plane with *certainty* (no `snap0`). No near-zero quantity can flip the
//!     combinatorics differently on two targets.
//!   * The output f64 bits come from rounding an exact value, not from accumulated f64 ops — so they
//!     are **bit-identical native↔wasm by construction**. [`spike_digest`] pins those bits; the
//!     native test below and the browser gate (docs/12 §2.3) compare the same digest across targets.
//!
//! Coplanar overlaps return `None` here (deferred, exactly as `mesh_clip` defers them per docs/05).
//! The lazy interval fast-leg is a later increment and must not change a single output bit.
//!
//! [`mesh_clip::tri_tri_intersection`]: crate::mesh_clip
//! [`mesh_clip::edge_cross`]: crate::mesh_clip

use core::cmp::Ordering;

use dashu::rational::RBig;

use super::scalar::{abs_lt, cross, dot, pt, sign, sub, to_f64_pt, V3};

/// The exact transversal intersection segment of two triangles.
pub struct ExactSeg {
    pub a: V3,
    pub b: V3,
}

/// A triangle's parametric crossing interval with the other plane, projected on `axis` to order the
/// endpoints. `lo_s`/`hi_s` are the exact projected coordinates; `lo_p`/`hi_p` the exact 3D points.
struct Interval {
    lo_s: RBig,
    hi_s: RBig,
    lo_p: V3,
    hi_p: V3,
}

/// Exact transversal triangle–triangle intersection segment. `None` for disjoint, parallel,
/// vertex/edge-touching, or coplanar pairs (coplanar deferred). Inputs are f64; the result is exact.
pub fn exact_tri_tri_segment(ct: [[f64; 3]; 3], wt: [[f64; 3]; 3]) -> Option<ExactSeg> {
    let c = [pt(ct[0]), pt(ct[1]), pt(ct[2])];
    let w = [pt(wt[0]), pt(wt[1]), pt(wt[2])];

    let n_c = cross(&sub(&c[1], &c[0]), &sub(&c[2], &c[0]));
    let n_w = cross(&sub(&w[1], &w[0]), &sub(&w[2], &w[0]));

    // Degenerate (zero-area) triangle has no plane — exact zero-normal test.
    if is_zero_vec(&n_c) || is_zero_vec(&n_w) {
        return None;
    }

    // Signed distances (× |n_c|) of W's verts to C's plane, and their exact signs.
    let d_c = -dot(&n_c, &c[0]);
    let dw = [
        dot(&n_c, &w[0]) + &d_c,
        dot(&n_c, &w[1]) + &d_c,
        dot(&n_c, &w[2]) + &d_c,
    ];
    let sw = [sign(&dw[0]), sign(&dw[1]), sign(&dw[2])];
    if same_nonzero_sign(&sw) {
        return None; // all of W strictly on one side of C's plane
    }

    let d_w = -dot(&n_w, &w[0]);
    let dc = [
        dot(&n_w, &c[0]) + &d_w,
        dot(&n_w, &c[1]) + &d_w,
        dot(&n_w, &c[2]) + &d_w,
    ];
    let sc = [sign(&dc[0]), sign(&dc[1]), sign(&dc[2])];
    if same_nonzero_sign(&sc) {
        return None; // all of C strictly on one side of W's plane
    }

    // Cross-line direction. EXACT parallel test: dir == 0 ⇒ parallel/coplanar (deferred), no epsilon.
    let dir = cross(&n_c, &n_w);
    if is_zero_vec(&dir) {
        return None;
    }
    let axis = dominant_axis(&dir);

    let ic = tri_interval(axis, &c, &dc, &sc)?;
    let iw = tri_interval(axis, &w, &dw, &sw)?;

    // Overlap the two intervals: lo = the larger of the two lows, hi = the smaller of the two highs
    // (mirrors mesh_clip::tri_tri_intersection). Comparisons are exact rational `Ord`.
    let (lo_s, lo_p) = if ic.lo_s >= iw.lo_s { (ic.lo_s, ic.lo_p) } else { (iw.lo_s, iw.lo_p) };
    let (hi_s, hi_p) = if ic.hi_s <= iw.hi_s { (ic.hi_s, ic.hi_p) } else { (iw.hi_s, iw.hi_p) };

    match hi_s.cmp(&lo_s) {
        // Disjoint intervals (planes cross, triangles don't) OR an exact touch (zero-length) — both
        // are "no transversal segment". Exact equality replaces the f64 graze epsilon.
        Ordering::Less | Ordering::Equal => None,
        Ordering::Greater => Some(ExactSeg { a: lo_p, b: hi_p }),
    }
}

/// The crossing interval of triangle `v` with the other plane: edge crossings from the lone vertex,
/// ordered by their projection on `axis`.
fn tri_interval(axis: usize, v: &[V3; 3], d: &[RBig; 3], s: &[i32; 3]) -> Option<Interval> {
    let lone = lone_vertex(s)?;
    let (i, j, k) = (lone, (lone + 1) % 3, (lone + 2) % 3);
    let pj = edge_cross(&v[i], &v[j], &d[i], &d[j]);
    let pk = edge_cross(&v[i], &v[k], &d[i], &d[k]);
    let sj = pj[axis].clone();
    let sk = pk[axis].clone();
    if sj <= sk {
        Some(Interval { lo_s: sj, lo_p: pj, hi_s: sk, hi_p: pk })
    } else {
        Some(Interval { lo_s: sk, lo_p: pk, hi_s: sj, hi_p: pj })
    }
}

/// Index of the vertex whose sign is alone on one side of the plane; `None` only for the all-zero
/// (coplanar) case the caller has excluded. Mirrors [`mesh_clip::lone_vertex`] on exact signs.
///
/// [`mesh_clip::lone_vertex`]: crate::mesh_clip
fn lone_vertex(s: &[i32; 3]) -> Option<usize> {
    if s == &[0, 0, 0] {
        return None;
    }
    for i in 0..3 {
        let (j, k) = ((i + 1) % 3, (i + 2) % 3);
        if s[j] == s[k] && s[j] != 0 && s[i] != s[j] {
            return Some(i);
        }
    }
    (0..3).find(|&i| s[i] != 0)
}

/// Exact point where segment `a→b` crosses the plane, from the endpoints' signed distances:
/// `p = (da·b − db·a)/(da − db)`, per coordinate. `da`/`db` have opposite signs (or one is zero, the
/// on-plane vertex), so `da − db ≠ 0`. This is the exact analog of [`mesh_clip::edge_cross`]'s
/// `a + t·(b − a)` with `t = da/(da − db)` — algebraically identical, but with no intermediate
/// rounding.
///
/// [`mesh_clip::edge_cross`]: crate::mesh_clip
fn edge_cross(a: &V3, b: &V3, da: &RBig, db: &RBig) -> V3 {
    let denom = da - db;
    [
        (da * &b[0] - db * &a[0]) / &denom,
        (da * &b[1] - db * &a[1]) / &denom,
        (da * &b[2] - db * &a[2]) / &denom,
    ]
}

/// Index of the largest-magnitude component (ties → lowest index): a deterministic projection axis.
fn dominant_axis(v: &V3) -> usize {
    let mut best = 0;
    for i in 1..3 {
        if abs_lt(&v[best], &v[i]) {
            best = i;
        }
    }
    best
}

fn is_zero_vec(v: &V3) -> bool {
    v.iter().all(|x| sign(x) == 0)
}

fn same_nonzero_sign(s: &[i32; 3]) -> bool {
    s[0] != 0 && s[0] == s[1] && s[1] == s[2]
}

// =================================================================================================
// Determinism digest — the native↔wasm gate (docs/12 §2.3).
// =================================================================================================

/// A small fixed corpus of crossing triangle pairs with deliberately non-round coordinates, so the
/// exact→f64 rounding is genuinely exercised. A base triangle in `z = 0` and several tilted triangles
/// each straddling it over its interior (one vertex below, two above) — guaranteed transversal
/// crossings whose intersection points land on non-dyadic rationals (e.g. a `1/3` parameter).
fn fixture_pairs() -> Vec<([[f64; 3]; 3], [[f64; 3]; 3])> {
    let c = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
    let ws = [
        [[1.0, 1.0, -1.0], [3.0, 1.0, 1.0], [1.0, 3.0, 2.0]],
        [[2.0, 1.0, -2.0], [4.0, 1.5, 1.0], [1.5, 3.5, 3.0]],
        [[0.7, 0.7, -0.3], [2.3, 1.1, 0.9], [1.1, 2.9, 1.7]],
    ];
    ws.iter().map(|w| (c, *w)).collect()
}

/// Run the exact tri-tri on the fixture corpus and snapshot each segment's endpoints to f64.
fn fixture_segments() -> Vec<([f64; 3], [f64; 3])> {
    fixture_pairs()
        .iter()
        .filter_map(|(ct, wt)| {
            exact_tri_tri_segment(*ct, *wt).map(|s| (to_f64_pt(&s.a), to_f64_pt(&s.b)))
        })
        .collect()
}

/// FNV-1a 64-bit (same digest as `mesh_clip`'s determinism lock) of the canonical byte encoding of
/// the fixture's exact-then-rounded segment endpoints. **This value is the determinism gate.** It is
/// computed natively as a golden constant and re-computed in the browser (a wasm-exported wrapper);
/// equality proves the exact construction is bit-identical native↔wasm through the real wasm artifact.
pub fn spike_digest() -> u64 {
    let segs = fixture_segments();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(segs.len() as u64).to_le_bytes());
    for (a, b) in &segs {
        for p in [a, b] {
            for &coord in p {
                bytes.extend_from_slice(&coord.to_bits().to_le_bytes());
            }
        }
    }
    fnv1a(&bytes)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture corpus actually produces crossings (a guard against the fixture silently going
    /// empty and making the determinism digest a hash of nothing).
    #[test]
    fn fixture_corpus_is_nonempty() {
        let segs = fixture_segments();
        assert_eq!(segs.len(), fixture_pairs().len(), "every fixture pair should cross");
    }

    /// A hand-checkable crossing: base triangle in z=0, a tilted triangle with vertex 0 below and
    /// 1,2 above. Edge 0→2 crosses z=0 at parameter 1/3 (z: −1 → 2), a non-dyadic rational, so the y
    /// coordinate is exactly 1 + 5/3·(... ) — the point is the exact value, not a tolerance-snapped one.
    #[test]
    fn crossing_endpoint_is_exact_nondyadic() {
        let c = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
        let w = [[1.0, 1.0, -1.0], [3.0, 1.0, 1.0], [1.0, 3.0, 2.0]];
        let seg = exact_tri_tri_segment(c, w).expect("should cross");
        // Both endpoints lie in z = 0 (C's plane) exactly.
        assert_eq!(sign(&seg.a[2]), 0, "endpoint a off C's plane");
        assert_eq!(sign(&seg.b[2]), 0, "endpoint b off C's plane");
        // Edge 0→2 crossing: t = 1/3 ⇒ point (1, 1 + (3-1)/3, 0) = (1, 5/3, 0). Exactly.
        let five_thirds = RBig::from_parts(5.into(), 3u8.into());
        let on_edge02 = [&seg.a, &seg.b]
            .iter()
            .any(|p| sign(&p[0]) > 0 && p[0] == RBig::ONE && p[1] == five_thirds);
        assert!(on_edge02, "expected the exact 1/3-parameter crossing (1, 5/3, 0) on edge 0→2");
    }

    /// Parallel (here: coincident-plane) pairs are rejected with no epsilon — exact `dir == 0`.
    #[test]
    fn parallel_pair_returns_none() {
        let c = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
        let w = [[0.0, 0.0, 5.0], [6.0, 0.0, 5.0], [0.0, 6.0, 5.0]]; // same normal, offset in z
        assert!(exact_tri_tri_segment(c, w).is_none());
    }

    /// The determinism gate (native half). Run-twice identity is trivial (no HashMap in this path),
    /// but kept as the regression anchor; the pinned digest is what the browser gate re-checks.
    ///
    /// NOTE per `geofield-no-wasm-cargo-test-harness`: this crate has no `wasm-bindgen-test`, so the
    /// cross-target half runs in the browser (`web3/e2e/exact-determinism.spec.ts`), not here.
    #[test]
    fn spike_digest_is_deterministic_and_pinned() {
        const GOLDEN: u64 = 0x0E4B_9D0B_1F84_6BD1;
        let d1 = spike_digest();
        let d2 = spike_digest();
        assert_eq!(d1, d2, "exact spike digest not reproducible within a run");
        eprintln!("[exact-spike] determinism golden FNV-1a = 0x{d1:016X}");
        assert_eq!(
            d1, GOLDEN,
            "exact spike digest changed: 0x{d1:016X} != golden 0x{GOLDEN:016X} \
             (a native change, OR a native↔wasm divergence if seen only in the browser gate)"
        );
    }
}
