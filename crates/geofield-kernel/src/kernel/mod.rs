//! Covariance kernels (KERNEL_MATH.md §1, §1b, §1c).
//!
//! The cokriging assembly and the dual evaluator consume a kernel only through the
//! triple `C`, `C'/r`, `C''` plus gempy's distance convention for it, so a kernel is
//! fully described by a [`KernelFunction`] variant. The variants are exactly gempy's
//! `AvailableKernelFunctions` that geofield validates against (cubic, matern_5_2,
//! exponential); a kernel with no gempy reference does not belong here (Constitution I).

mod cubic;
mod exponential;
mod matern;

pub use cubic::{cubic_a, cubic_c, cubic_p_div_r};
pub use exponential::{exponential_a, exponential_c, exponential_p_div_r};
pub use matern::{matern52_a, matern52_c, matern52_p_div_r};

use crate::geometry::{cg_cg_divisor, kernel_arg_dist};

/// gempy's CG_CG divisor epsilon, applied to the squared **kernel argument**
/// (KERNEL_MATH.md §1c.4). Same value as [`crate::geometry`]'s `CG_CG_EPS`.
const CG_CG_EPS: f64 = 1e-5;

/// Which covariance kernel a fit uses — gempy's `KernelOptions.kernel_function`.
///
/// `Cubic` is the default and the original validated path; it stays bit-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KernelFunction {
    /// gempy `cubic` — compactly supported polynomial (§1). Takes floored `r`.
    #[default]
    Cubic,
    /// gempy `matern_5_2` — Matérn ν=5/2 (§1b). Takes floored `r`.
    Matern52,
    /// gempy `exponential` — the **Gaussian** `exp(−r²/2a²)` (§1c). Takes raw `r²`.
    Exponential,
}

impl KernelFunction {
    /// All variants, in ABI-code order.
    pub const ALL: [KernelFunction; 3] = [
        KernelFunction::Cubic,
        KernelFunction::Matern52,
        KernelFunction::Exponential,
    ];

    /// ABI code carried in the fit header (`abi.rs`): 0 cubic, 1 matern_5_2,
    /// 2 exponential.
    pub fn code(self) -> u32 {
        match self {
            KernelFunction::Cubic => 0,
            KernelFunction::Matern52 => 1,
            KernelFunction::Exponential => 2,
        }
    }

    /// Inverse of [`code`](Self::code); `None` for an unknown code.
    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == code)
    }

    /// gempy's `AvailableKernelFunctions` member name.
    pub fn gempy_name(self) -> &'static str {
        match self {
            KernelFunction::Cubic => "cubic",
            KernelFunction::Matern52 => "matern_5_2",
            KernelFunction::Exponential => "exponential",
        }
    }

    /// Inverse of [`gempy_name`](Self::gempy_name).
    pub fn from_gempy_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.gempy_name() == name)
    }

    /// The kernel argument for a raw squared distance — gempy's
    /// `consume_sq_distance` switch (`_compute_distances_generic`):
    ///   - `False` (cubic, matern_5_2): `sqrt(r² + 1e-10)` — the floored distance;
    ///   - `True` (exponential): `r²` itself, **un-floored**.
    #[inline]
    pub fn arg(self, r2: f64) -> f64 {
        match self {
            KernelFunction::Exponential => r2,
            KernelFunction::Cubic | KernelFunction::Matern52 => kernel_arg_dist(r2),
        }
    }

    /// `C` at a kernel argument from [`arg`](Self::arg).
    #[inline]
    pub fn c(self, arg: f64, a: f64) -> f64 {
        match self {
            KernelFunction::Cubic => cubic_c(arg, a),
            KernelFunction::Matern52 => matern52_c(arg, a),
            KernelFunction::Exponential => exponential_c(arg, a),
        }
    }

    /// `C'(r)/r` at a kernel argument from [`arg`](Self::arg).
    #[inline]
    pub fn p_div_r(self, arg: f64, a: f64) -> f64 {
        match self {
            KernelFunction::Cubic => cubic_p_div_r(arg, a),
            KernelFunction::Matern52 => matern52_p_div_r(arg, a),
            KernelFunction::Exponential => exponential_p_div_r(arg, a),
        }
    }

    /// `C''(r)` at a kernel argument from [`arg`](Self::arg).
    #[inline]
    pub fn a(self, arg: f64, a: f64) -> f64 {
        match self {
            KernelFunction::Cubic => cubic_a(arg, a),
            KernelFunction::Matern52 => matern52_a(arg, a),
            KernelFunction::Exponential => exponential_a(arg, a),
        }
    }

    /// The CG_CG divisor for a raw squared distance (KERNEL_MATH.md §3.1, §1c.4).
    ///
    /// gempy writes `hu·hv / (r_ref_ref**2 + 1e-5)` where `r_ref_ref` is the **kernel
    /// argument**. For the distance-argument kernels that is `r² + 1e-5` (the
    /// validated cubic form, unchanged). For `exponential` the argument is already
    /// `r²`, so gempy's divisor is **`r⁴ + 1e-5`** — not the `r²` the Hessian
    /// decomposition calls for. This is a gempy quirk reproduced deliberately for
    /// parity; see the §1c.4 landmine before "fixing" it.
    #[inline]
    pub fn cg_cg_divisor(self, r2: f64) -> f64 {
        match self {
            KernelFunction::Exponential => r2 * r2 + CG_CG_EPS,
            KernelFunction::Cubic | KernelFunction::Matern52 => cg_cg_divisor(r2),
        }
    }

    /// Whether the kernel is compactly supported with gempy's no-clamp polynomial,
    /// i.e. whether the §1.1 in-range precondition applies.
    #[inline]
    pub fn needs_in_range_precondition(self) -> bool {
        matches!(self, KernelFunction::Cubic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_names_round_trip() {
        for k in KernelFunction::ALL {
            assert_eq!(KernelFunction::from_code(k.code()), Some(k));
            assert_eq!(KernelFunction::from_gempy_name(k.gempy_name()), Some(k));
        }
        assert_eq!(KernelFunction::from_code(3), None);
        assert_eq!(KernelFunction::from_gempy_name("gaussian"), None);
        assert_eq!(KernelFunction::default(), KernelFunction::Cubic);
    }

    #[test]
    fn exponential_consumes_raw_squared_distance() {
        assert_eq!(KernelFunction::Exponential.arg(0.25), 0.25);
        assert_eq!(KernelFunction::Cubic.arg(0.25), kernel_arg_dist(0.25));
        assert_eq!(KernelFunction::Matern52.arg(0.25), kernel_arg_dist(0.25));
        // The gempy r⁴ CG_CG divisor quirk (§1c.4).
        assert_eq!(KernelFunction::Exponential.cg_cg_divisor(0.5), 0.25 + 1e-5);
        assert_eq!(KernelFunction::Cubic.cg_cg_divisor(0.5), 0.5 + 1e-5);
    }
}
