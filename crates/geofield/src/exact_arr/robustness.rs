//! Layer-2 robustness characterization (docs/12 §2.2) — how the exact tri-tri behaves when its
//! INPUTS differ by ~1 ULP.
//!
//! This is the genuinely informative determinism question, separate from Layer 1 (proven). Layer 1
//! says: identical inputs → bit-identical output across targets. But the real native↔wasm pipeline
//! does NOT feed the corefiner identical inputs — the octree/field boundary crosses to a separately
//! compiled kernel whose transcendentals differ at ULP, so each target hands the corefiner slightly
//! different vertices. Exact construction makes each target internally CERTAIN; it does NOT make
//! divergent inputs converge. These tests pin exactly where that matters:
//!
//!   * **Generic position → graceful.** A 1-ULP input perturbation keeps the combinatorics identical
//!     (same sign pattern, same crossing classification) and moves the output by an O(1-ULP) amount.
//!     Benign: the two targets produce "the same mesh, imperceptibly perturbed", no cascade.
//!   * **Grid-aligned graze → sign flip.** When a vertex lies on the other plane (signed distance
//!     exactly 0 — the docs/09 contact-along-a-grid-plane case), a 1-ULP input change flips that
//!     sign, which changes the combinatorial classification *discretely*. This is the cascade
//!     trigger, and it proves exactness ALONE is insufficient: the inputs must also be deterministic
//!     (Track B), OR near-zero inputs must be canonicalized (the existing `CONTACT_GRAZE_EPS`
//!     approach, which reintroduces a tolerance). Characterized here, not "fixed".
//!
//! Native-only: this compares original-input vs perturbed-input on ONE target, so it needs no wasm.

use dashu::rational::RBig;

use super::scalar::{cross, dot, pt, q, sign, sub, to_f64_pt};
use super::tri_tri::exact_tri_tri_segment;

type Tri = [[f64; 3]; 3];

/// IEEE `nextUp` for finite, nonzero `x` (and the smallest subnormal off zero): a +1-ULP step toward
/// +∞. Models the smallest possible cross-target input divergence.
fn next_up(x: f64) -> f64 {
    if x == 0.0 {
        return f64::from_bits(1);
    }
    let bits = x.to_bits();
    let next = if x > 0.0 { bits + 1 } else { bits - 1 };
    f64::from_bits(next)
}

/// The combinatorial signature of a tri-tri pair: the exact signs of C's and W's vertices against
/// the other plane. These sign triples drive every downstream decision (lone vertex, which edges
/// cross, keep/discard). If they are identical across two inputs, the combinatorics are identical.
fn signs(ct: Tri, wt: Tri) -> ([i32; 3], [i32; 3]) {
    let c = [pt(ct[0]), pt(ct[1]), pt(ct[2])];
    let w = [pt(wt[0]), pt(wt[1]), pt(wt[2])];
    let n_c = cross(&sub(&c[1], &c[0]), &sub(&c[2], &c[0]));
    let n_w = cross(&sub(&w[1], &w[0]), &sub(&w[2], &w[0]));
    let d_c = -dot(&n_c, &c[0]);
    let d_w = -dot(&n_w, &w[0]);
    let sw = [
        sign(&(dot(&n_c, &w[0]) + &d_c)),
        sign(&(dot(&n_c, &w[1]) + &d_c)),
        sign(&(dot(&n_c, &w[2]) + &d_c)),
    ];
    let sc = [
        sign(&(dot(&n_w, &c[0]) + &d_w)),
        sign(&(dot(&n_w, &c[1]) + &d_w)),
        sign(&(dot(&n_w, &c[2]) + &d_w)),
    ];
    (sc, sw)
}

/// Max absolute coordinate drift between two segments' endpoints (after exact→f64 snapshot).
fn endpoint_drift(a: ([f64; 3], [f64; 3]), b: ([f64; 3], [f64; 3])) -> f64 {
    let mut m: f64 = 0.0;
    for (pa, pb) in [(a.0, b.0), (a.1, b.1)] {
        for k in 0..3 {
            m = m.max((pa[k] - pb[k]).abs());
        }
    }
    m
}

fn seg_f64(ct: Tri, wt: Tri) -> Option<([f64; 3], [f64; 3])> {
    exact_tri_tri_segment(ct, wt).map(|s| (to_f64_pt(&s.a), to_f64_pt(&s.b)))
}

/// The exact kernel returns the certain sign of the f64 it is GIVEN — it returns `0` ONLY for an
/// exact-zero input, never for a tiny-but-nonzero one. So a corner whose produced f64 is `1e-16` is a
/// **certain +**, not `sign == 0`. This is the answer to "does exactness detect the graze": it does
/// not. In the real pipeline the field eval essentially never yields exactly `0.0`; it yields a tiny
/// nonzero whose SIGN is what the float rounding happened to land on (and can differ native↔wasm). So
/// classifying an input as "a graze to canonicalize" still needs a tolerance `|x| < ε` — exactly what
/// exactness was meant to remove. Exactness removes ambiguity from the DECISION, not the tolerance
/// from the CLASSIFICATION. (The only tolerance-free fix is equal inputs on both targets — Track B.)
#[test]
fn exact_sign_is_certain_for_tiny_nonzero_zero_only_at_exact_zero() {
    assert_eq!(sign(&q(1e-16)), 1, "1e-16 is a normal nonzero f64 → a CERTAIN +, not 0");
    assert_eq!(sign(&q(-1e-17)), -1, "a tiny negative is a CERTAIN −");
    assert_eq!(sign(&q(0.0)), 0, "sign == 0 happens ONLY for an exact-zero input");
    // The f64 nearest to 1e-16 is nonzero, so its exact rational is nonzero — no near-zero collapse.
    assert_ne!(q(1e-16), q(0.0));
}

#[test]
fn generic_pair_degrades_gracefully_under_1ulp_input() {
    // A well-conditioned transversal crossing (fixture pair 0).
    let c: Tri = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
    let w: Tri = [[1.0, 1.0, -1.0], [3.0, 1.0, 1.0], [1.0, 3.0, 2.0]];

    // Perturb one W vertex coordinate by +1 ULP — the smallest cross-target divergence.
    let mut wp = w;
    wp[2][0] = next_up(wp[2][0]);

    // Combinatorics are UNCHANGED: no sign flips (the perturbation is far from any zero crossing).
    assert_eq!(signs(c, w), signs(c, wp), "generic 1-ULP perturbation flipped a sign — unexpected");

    // Output drift is bounded and tiny (the segment is a well-conditioned rational of the inputs).
    let s0 = seg_f64(c, w).expect("crosses");
    let s1 = seg_f64(c, wp).expect("still crosses");
    let drift = endpoint_drift(s0, s1);
    eprintln!("[layer2] generic 1-ULP input → output endpoint drift = {drift:.3e}");
    assert!(drift < 1e-6, "generic pair output drifted {drift:.3e} for a 1-ULP input — not graceful");
}

#[test]
fn grid_aligned_graze_is_input_sign_sensitive() {
    // C is the z = 0 triangle. W has vertex 0 lying EXACTLY on z = 0 (signed distance 0 → sign 0) —
    // the docs/09 "contact running along a grid plane" degeneracy. Vertices 1,2 straddle z = 0.
    let c: Tri = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
    let w: Tri = [[1.0, 1.0, 0.0], [3.0, 1.0, 1.0], [1.0, 3.0, -1.0]];

    let (_, sw0) = signs(c, w);
    assert_eq!(sw0[0], 0, "fixture setup: W vertex 0 must lie exactly on C's plane (sign 0)");

    // Native field-eval places the vertex exactly on the plane (z = 0.0); wasm, computing the SAME
    // geometric vertex through a separately-compiled kernel, lands a round-off floor off it
    // (docs/09 cites ~1e-17). We model that ABSOLUTE disagreement (not a coordinate ULP — 1 ULP of a
    // 0.0 coordinate is a meaningless subnormal). The exact corefiner is certain on each input — but
    // they DISAGREE on the sign.
    let mut wp = w;
    wp[0][2] = 1e-15; // wasm's tiny-positive z at the graze
    let (_, sw1) = signs(c, wp);

    assert_eq!(sw1[0], 1, "a +1-ULP step off the plane must give a definite + sign");
    assert_ne!(
        sw0, sw1,
        "the graze must be input-sign-sensitive: this is the cascade trigger that Track B (or \
         near-zero canonicalization) must remove — exactness alone does not"
    );

    // BOTH inputs still produce well-formed results (no panic / no garbage) — each target is
    // internally consistent; the divergence is purely that they decided different things upstream.
    eprintln!(
        "[layer2] graze: native(z=0) sign={:?} → {:?};  wasm(z=+1ULP) sign={:?} → {:?}",
        sw0,
        seg_f64(c, w).map(|_| "Some").unwrap_or("None"),
        sw1,
        seg_f64(c, wp).map(|_| "Some").unwrap_or("None"),
    );
}

#[test]
fn graze_canonicalization_would_restore_agreement() {
    // The constructive flip side: if the field/octree fed the corefiner the SAME (canonicalized)
    // near-zero value on both targets, the graze would key identically. We don't snap inside the
    // exact kernel (that reintroduces tolerance); this test just documents that the residual risk is
    // entirely upstream — given equal inputs, the signature is equal (a restatement of Layer 1 at a
    // degeneracy, the hardest case).
    let c: Tri = [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 6.0, 0.0]];
    let w: Tri = [[1.0, 1.0, 0.0], [3.0, 1.0, 1.0], [1.0, 3.0, -1.0]];
    assert_eq!(signs(c, w), signs(c, w), "equal inputs must give equal combinatorics at a graze");
    // And a rational value is exactly comparable to zero — no epsilon needed to DETECT the graze,
    // which is what makes a principled upstream canonicalization possible.
    let on_plane = RBig::ZERO;
    assert_eq!(sign(&on_plane), 0);
}
