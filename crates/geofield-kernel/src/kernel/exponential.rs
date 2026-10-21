//! gempy's `exponential` function triple, transcribed verbatim from KERNEL_MATH.md §1c.
//!
//! Despite gempy's name this is the **Gaussian** (squared-exponential) covariance
//! `exp(−r²/(2a²))`. It is gempy's only `consume_sq_distance=True` kernel: all three
//! functions take the **raw, un-floored squared distance `r²`**, not `r`
//! (see [`super::KernelFunction::arg`]). Passing `r` here is silently wrong.

/// `C = exp(−r²/(2a²))` (KERNEL_MATH.md §1c.1). Argument is `sq_r = r²`.
#[inline]
pub fn exponential_c(sq_r: f64, a: f64) -> f64 {
    (-(sq_r / (2.0 * a * a))).exp()
}

/// `C'(r)/r = −(1/a²)·exp(−r²/(2a²))` (KERNEL_MATH.md §1c.2). Argument is `r²`.
#[inline]
pub fn exponential_p_div_r(sq_r: f64, a: f64) -> f64 {
    -(1.0 / (a * a)) * (-(sq_r / (2.0 * a * a))).exp()
}

/// `C''(r) = (r²/a⁴ − 1/a²)·exp(−r²/(2a²))` (KERNEL_MATH.md §1c.3). Argument is `r²`.
#[inline]
pub fn exponential_a(sq_r: f64, a: f64) -> f64 {
    let a2 = a * a;
    let a4 = a2 * a2;
    (sq_r / a4 - 1.0 / a2) * (-(sq_r / (2.0 * a2))).exp()
}
