//! Step 1 gate (BUILD_geofield_rs.md): the cubic triple must be mutually
//! consistent before any assembly code is trusted.
//!
//! Verifies, per KERNEL_MATH.md §1:
//!   - `C(0) = 1`
//!   - `cubic_p_div_r(r,a) * r` equals `d/dr C(r)` (central FD)
//!   - `cubic_a(r,a)` equals `d²/dr² C(r)` (central FD)
//! across several `r ∈ (0, a)` and several `a`.

use geofield_kernel::kernel::{cubic_a, cubic_c, cubic_p_div_r};

/// Central finite difference of `C` (first derivative).
fn fd_first(r: f64, a: f64, h: f64) -> f64 {
    (cubic_c(r + h, a) - cubic_c(r - h, a)) / (2.0 * h)
}

/// Central finite difference of `C` (second derivative).
fn fd_second(r: f64, a: f64, h: f64) -> f64 {
    (cubic_c(r + h, a) - 2.0 * cubic_c(r, a) + cubic_c(r - h, a)) / (h * h)
}

#[test]
fn c_at_zero_is_one() {
    for a in [0.7_f64, 1.0, 1.7, 3.0] {
        assert!((cubic_c(0.0, a) - 1.0).abs() < 1e-15, "C(0) != 1 for a={a}");
    }
}

#[test]
fn p_div_r_times_r_matches_dc_dr() {
    // h chosen to balance truncation vs roundoff for a polynomial; FD of a
    // smooth polynomial is very accurate.
    let h = 1e-6;
    for a in [0.7_f64, 1.0, 1.7, 3.0] {
        // Sample several r strictly inside (0, a).
        for frac in [0.05, 0.2, 0.35, 0.5, 0.75, 0.95] {
            let r = frac * a;
            let analytic = cubic_p_div_r(r, a) * r; // = C'(r)
            let numeric = fd_first(r, a, h);
            let tol = 1e-6 * (1.0 + analytic.abs());
            assert!(
                (analytic - numeric).abs() < tol,
                "C'(r) mismatch: a={a} r={r} analytic={analytic} numeric={numeric}"
            );
        }
    }
}

#[test]
fn cubic_a_matches_d2c_dr2() {
    let h = 1e-4; // second-difference roundoff grows like 1/h^2; keep h moderate.
    for a in [0.7_f64, 1.0, 1.7, 3.0] {
        for frac in [0.05, 0.2, 0.35, 0.5, 0.75, 0.95] {
            let r = frac * a;
            let analytic = cubic_a(r, a); // = C''(r)
            let numeric = fd_second(r, a, h);
            let tol = 1e-4 * (1.0 + analytic.abs());
            assert!(
                (analytic - numeric).abs() < tol,
                "C''(r) mismatch: a={a} r={r} analytic={analytic} numeric={numeric}"
            );
        }
    }
}

#[test]
fn factored_a_matches_expanded_a() {
    // Cross-check the source factored form against the expanded form in §1.3.
    fn expanded(r: f64, a: f64) -> f64 {
        let a2 = a * a;
        let a3 = a2 * a;
        let a5 = a3 * a2;
        let a7 = a5 * a2;
        let r3 = r * r * r;
        let r5 = r3 * r * r;
        -14.0 / a2 + (105.0 / 2.0) * (r / a3) - 70.0 * (r3 / a5) + (63.0 / 2.0) * (r5 / a7)
    }
    for a in [0.7_f64, 1.0, 1.7, 3.0] {
        for frac in [0.0, 0.2, 0.5, 0.95, 1.3] {
            let r = frac * a;
            assert!(
                (cubic_a(r, a) - expanded(r, a)).abs() < 1e-12 * (1.0 + expanded(r, a).abs()),
                "factored vs expanded C'' mismatch: a={a} r={r}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The gempy matern_5_2 and exponential triples (KERNEL_MATH.md §1b, §1c): the same
// three consistency gates, expressed through `KernelFunction` so the distance
// convention (`r` vs raw `r²`, gempy's `consume_sq_distance`) is exercised too.
// ---------------------------------------------------------------------------

use geofield_kernel::KernelFunction;

/// `C` as a function of the geometric distance `r`, through the kernel's own argument.
fn c_of_r(kf: KernelFunction, r: f64, a: f64) -> f64 {
    kf.c(kf.arg(r * r), a)
}

#[test]
fn new_kernels_c_at_zero_is_one() {
    for kf in [KernelFunction::Matern52, KernelFunction::Exponential] {
        for a in [0.7_f64, 1.0, 1.7, 3.0] {
            // The raw kernel functions at argument 0 (no 1e-10 floor) are exactly 1.
            assert_eq!(kf.c(0.0, a), 1.0, "{kf:?}: C(0) != 1 for a={a}");
        }
    }
}

/// Orientation data needs `C'/r` and `C''` finite at `r → 0` (the self-covariance
/// of a gradient functional). These are the limits the kernels must reach.
#[test]
fn new_kernels_derivatives_are_finite_at_zero() {
    for a in [0.7_f64, 1.7] {
        let m = KernelFunction::Matern52;
        assert!((m.p_div_r(0.0, a) + 5.0 / (3.0 * a * a)).abs() < 1e-12);
        assert!((m.a(0.0, a) + 5.0 / (3.0 * a * a)).abs() < 1e-12);
        let e = KernelFunction::Exponential;
        assert_eq!(e.p_div_r(0.0, a), -1.0 / (a * a));
        assert!((e.a(0.0, a) + 1.0 / (a * a)).abs() < 1e-15);
    }
}

#[test]
fn new_kernels_p_div_r_times_r_matches_dc_dr() {
    let h = 1e-6;
    for kf in [KernelFunction::Matern52, KernelFunction::Exponential] {
        for a in [0.7_f64, 1.0, 1.7, 3.0] {
            for frac in [0.05, 0.2, 0.35, 0.5, 0.75, 0.95, 1.5] {
                let r = frac * a;
                let analytic = kf.p_div_r(kf.arg(r * r), a) * r; // = C'(r)
                let numeric = (c_of_r(kf, r + h, a) - c_of_r(kf, r - h, a)) / (2.0 * h);
                let tol = 1e-6 * (1.0 + analytic.abs());
                assert!(
                    (analytic - numeric).abs() < tol,
                    "{kf:?}: C'(r) mismatch a={a} r={r} analytic={analytic} numeric={numeric}"
                );
            }
        }
    }
}

#[test]
fn new_kernels_a_matches_d2c_dr2() {
    let h = 1e-4;
    for kf in [KernelFunction::Matern52, KernelFunction::Exponential] {
        for a in [0.7_f64, 1.0, 1.7, 3.0] {
            for frac in [0.05, 0.2, 0.35, 0.5, 0.75, 0.95, 1.5] {
                let r = frac * a;
                let analytic = kf.a(kf.arg(r * r), a); // = C''(r)
                let numeric = (c_of_r(kf, r + h, a) - 2.0 * c_of_r(kf, r, a)
                    + c_of_r(kf, r - h, a))
                    / (h * h);
                let tol = 1e-4 * (1.0 + analytic.abs());
                assert!(
                    (analytic - numeric).abs() < tol,
                    "{kf:?}: C''(r) mismatch a={a} r={r} analytic={analytic} numeric={numeric}"
                );
            }
        }
    }
}
