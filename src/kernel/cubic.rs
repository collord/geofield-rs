//! The cubic function triple, transcribed verbatim from KERNEL_MATH.md §1.
//!
//! These are the *symbolic* (non-factorized) forms bound into gempy's
//! `AvailableKernelFunctions.cubic`. All take the **raw distance `r`** (not `r²`)
//! and the kernel range `a`.
//!
//! Per KERNEL_MATH.md §1.1 there is **no clamping** for `r >= a`: the polynomial
//! is evaluated everywhere to match gempy bit-for-bit. The implicit precondition
//! (rescaling keeps all pairwise normalized distances well inside `a`) is enforced
//! by a debug assertion at the assembly site, not here.

/// `C(r)` — base covariance (KERNEL_MATH.md §1.1).
///
/// ```text
/// C(r) = 1 − 7 (r/a)² + (35/4)(r³/a³) − (7/2)(r⁵/a⁵) + (3/4)(r⁷/a⁷)
/// ```
///
/// `C(0) = 1`.
#[inline]
pub fn cubic_c(r: f64, a: f64) -> f64 {
    let ra = r / a;
    let ra2 = ra * ra;
    let ra3 = ra2 * ra;
    let ra5 = ra3 * ra2;
    let ra7 = ra5 * ra2;
    1.0 - 7.0 * ra2 + (35.0 / 4.0) * ra3 - (7.0 / 2.0) * ra5 + (3.0 / 4.0) * ra7
}

/// `C'(r) / r` — first derivative pre-divided by `r` (KERNEL_MATH.md §1.2).
///
/// Implemented directly as the factored polynomial so the gradient-block assembly
/// never divides by `r`:
///
/// ```text
/// C'(r)/r = −14/a² + (105/4)(r/a³) − (35/2)(r³/a⁵) + (21/4)(r⁵/a⁷)
/// ```
#[inline]
pub fn cubic_p_div_r(r: f64, a: f64) -> f64 {
    let a2 = a * a;
    let a3 = a2 * a;
    let a5 = a3 * a2;
    let a7 = a5 * a2;
    let r2 = r * r;
    let r3 = r2 * r;
    let r5 = r3 * r2;
    -14.0 / a2 + (105.0 / 4.0) * (r / a3) - (35.0 / 2.0) * (r3 / a5) + (21.0 / 4.0) * (r5 / a7)
}

/// `C''(r)` — second derivative (KERNEL_MATH.md §1.3).
///
/// Source factored form:
/// ```text
/// C_a(r) = 7 (9 r⁵ − 20 a² r³ + 15 a⁴ r − 4 a⁵) / (2 a⁷)
/// ```
/// which expands to:
/// ```text
/// C''(r) = −14/a² + (105/2)(r/a³) − 70 (r³/a⁵) + (63/2)(r⁵/a⁷)
/// ```
///
/// This is the **plain** second derivative, NOT a pre-folded `C'' − C'/r`
/// (verified in KERNEL_MATH.md §1.3 by the surviving `−14/a²` constant term).
#[inline]
pub fn cubic_a(r: f64, a: f64) -> f64 {
    let a2 = a * a;
    let a5 = a2 * a2 * a;
    let a7 = a5 * a2;
    let r2 = r * r;
    let r3 = r2 * r;
    let r5 = r3 * r2;
    7.0 * (9.0 * r5 - 20.0 * a2 * r3 + 15.0 * a2 * a2 * r - 4.0 * a5) / (2.0 * a7)
}
