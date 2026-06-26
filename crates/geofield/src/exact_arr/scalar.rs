//! The exact-rational leg of the corefiner kernel.
//!
//! Increment 1 (the determinism kill-test, docs/12 §5) carries every constructed quantity as an
//! exact [`dashu`] rational ([`RBig`]) — no f64 arithmetic between input and output. Each finite
//! f64 input is a dyadic rational, so `f64 → RBig` is **exact**; the only rounding is the single
//! correctly-rounded `RBig → f64` snapshot at output ([`to_f64`]). Because `RBig` arithmetic is pure
//! integer work (no platform floats until that final snapshot, and wasm mandates IEEE-754
//! round-to-nearest-even), the whole construction is **bit-identical native↔wasm by construction** —
//! which is exactly the premise the spike exists to verify.
//!
//! The lazy *interval* fast-leg (f64 bound + escalation to expansions, then to these rationals only
//! when a sign is ambiguous) is a later increment; it must not change any output bit, only the speed.
//! Keeping the exact leg behind these small free functions makes that drop-in clean.

use dashu::rational::RBig;

/// A point/vector in exact rational coordinates.
pub type V3 = [RBig; 3];

/// Exact `f64 → RBig`. Every finite f64 is a dyadic rational, so this is lossless. Non-finite input
/// is a caller bug (mesh vertices are finite); we surface it rather than silently produce garbage.
#[inline]
pub fn q(x: f64) -> RBig {
    debug_assert!(x.is_finite(), "exact_arr: non-finite coordinate {x} reached the rational kernel");
    // Guard the geometry-predicates exponent band (docs/12 §1 caveat) so a future expansion-based
    // fast leg can never silently lose accuracy on the same inputs. RBig itself is unconditional.
    debug_assert!(
        x == 0.0 || (x.abs() >= 1e-142 && x.abs() <= 1e201),
        "exact_arr: coordinate {x} outside the safe exponent band [1e-142, 1e201]"
    );
    RBig::try_from(x).expect("finite f64 is an exact rational")
}

/// Lift an f64 point to exact coordinates.
#[inline]
pub fn pt(p: [f64; 3]) -> V3 {
    [q(p[0]), q(p[1]), q(p[2])]
}

/// Correctly-rounded (round-to-nearest-even) `RBig → f64`. Deterministic across targets: the
/// conversion is integer long-division with a fixed rounding rule, no platform float ops.
#[inline]
pub fn to_f64(x: &RBig) -> f64 {
    x.clone().to_f64().value()
}

/// Snapshot an exact point to the f64 output buffer.
#[inline]
pub fn to_f64_pt(p: &V3) -> [f64; 3] {
    [to_f64(&p[0]), to_f64(&p[1]), to_f64(&p[2])]
}

#[inline]
pub fn sub(a: &V3, b: &V3) -> V3 {
    [&a[0] - &b[0], &a[1] - &b[1], &a[2] - &b[2]]
}

#[inline]
pub fn dot(a: &V3, b: &V3) -> RBig {
    &a[0] * &b[0] + &a[1] * &b[1] + &a[2] * &b[2]
}

#[inline]
pub fn cross(a: &V3, b: &V3) -> V3 {
    [
        &a[1] * &b[2] - &a[2] * &b[1],
        &a[2] * &b[0] - &a[0] * &b[2],
        &a[0] * &b[1] - &a[1] * &b[0],
    ]
}

/// Sign of an exact value: −1, 0, or +1. Exact and total — no epsilon, unlike the f64 [`snap0`]
/// path it replaces. This is where exact arithmetic earns its keep: a vertex is above / on / below a
/// plane with *certainty*, so no near-zero graze can flip the combinatorics across targets.
///
/// [`snap0`]: crate::mesh_clip
#[inline]
pub fn sign(x: &RBig) -> i32 {
    if x > &RBig::ZERO {
        1
    } else if x < &RBig::ZERO {
        -1
    } else {
        0
    }
}

/// `|a| < |b|` on exact values (used only to pick a deterministic projection axis).
#[inline]
pub fn abs_lt(a: &RBig, b: &RBig) -> bool {
    let aa = if sign(a) < 0 { -a.clone() } else { a.clone() };
    let bb = if sign(b) < 0 { -b.clone() } else { b.clone() };
    aa < bb
}
