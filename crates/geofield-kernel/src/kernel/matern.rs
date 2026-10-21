//! The Matérn ν=5/2 function triple, transcribed verbatim from KERNEL_MATH.md §1b.
//!
//! These are gempy's `AvailableKernelFunctions.matern_5_2` functions
//! (`matern_function_5_2_numpy`, `…_p_div_r_numpy`, `…_a_numpy`). Like the cubic,
//! they take the **floored raw distance `r`** (`consume_sq_distance=False`) and the
//! kernel range `a`. No compact support: the kernel decays exponentially and needs
//! no in-range precondition.
//!
//! ⚠ The literals `2.2360679775` (≈ √5) and `1.6666666667` (≈ 5/3) are gempy's
//! **truncated** constants, kept verbatim for bit-for-bit parity. The derivatives use
//! the exact `5/3` algebra, so `C` and its derivatives disagree at ~1e-11 relative —
//! far below every gate, but do not "fix" one side without the other (KERNEL_MATH.md §1b).

/// gempy's truncated √5.
const S5: f64 = 2.2360679775;

/// `C(r) = (1 + √5·x + (5/3)·x²) · exp(−√5·x)`, `x = r/a` (KERNEL_MATH.md §1b.1).
#[inline]
pub fn matern52_c(r: f64, a: f64) -> f64 {
    let x = r / a;
    let s5_x = S5 * x;
    let poly = 1.0 + s5_x + (1.6666666667 * x * x);
    poly * (-s5_x).exp()
}

/// `C'(r)/r = −5·exp(−√5·x)·(a + √5·r) / (3a³)` (KERNEL_MATH.md §1b.2). Finite at
/// `r = 0` (`−5/(3a²)`), which is what lets this kernel carry orientation data.
#[inline]
pub fn matern52_p_div_r(r: f64, a: f64) -> f64 {
    let x = r / a;
    let term_exp = (-S5 * x).exp();
    -5.0 * term_exp * (a + S5 * r) / (3.0 * (a * a * a))
}

/// `C''(r) = −5·exp(−√5·x)·(a² + √5·a·r − 5r²) / (3a⁴)` (KERNEL_MATH.md §1b.3) —
/// the plain second derivative, like the cubic's `C''`.
#[inline]
pub fn matern52_a(r: f64, a: f64) -> f64 {
    let x = r / a;
    let term_exp = (-S5 * x).exp();
    let poly = (a * a) + (S5 * a * r) - (5.0 * r * r);
    -5.0 * term_exp * poly / (3.0 * (a * a * a * a))
}
