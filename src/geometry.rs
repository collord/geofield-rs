//! Per-pair geometric quantities (KERNEL_MATH.md §2).
//!
//! gempy expands each orientation into one gradient functional **per dimension
//! component** (the interleaved layout). A gradient–gradient pair is therefore a
//! pair of axis selectors `(i, j)` at two locations; the per-pair reductions of
//! gempy's `Σ d·(sel_i*sel_j)` over one-hot selectors are the scalar `hu`, `hv`,
//! and `perp` below.
//!
//! Two epsilons live here and must never be confused (§0):
//!   - the cubic **kernel-argument** distance uses `sqrt(r² + 1e-10)`
//!     ([`kernel_arg_dist`]);
//!   - the CG_CG **divisor** uses `r² + 1e-5` ([`cg_cg_divisor`]).
//! They are separate functions so the two cannot be swapped at a call site.

/// Kernel-argument distance floor (cubic functions are evaluated here): `1e-10`.
const KERNEL_ARG_EPS: f64 = 1e-10;
/// CG_CG divisor floor (`hu*hv / (r² + eps)`): `1e-5`.
const CG_CG_EPS: f64 = 1e-5;

/// `hu = Σ d · (hu_sel_i · hu_sel_j)` for one-hot axis selector `i`
/// (KERNEL_MATH.md §2). With one-hot selectors this is the `i`-th component of
/// the separation `d = p1 − p2`.
#[inline]
pub fn hu(d: [f64; 3], i: usize) -> f64 {
    d[i]
}

/// `hv = − Σ d · (hv_sel_i · hv_sel_j)` for one-hot axis selector `j`
/// (KERNEL_MATH.md §2). **Leading minus**: `hv = −d[j]`.
#[inline]
pub fn hv(d: [f64; 3], j: usize) -> f64 {
    -d[j]
}

/// `perp_matrix = Σ (hu_sel_i · hv_sel_j)` — the same-component indicator between
/// two gradient functionals (KERNEL_MATH.md §2). `1.0` iff the two functionals
/// select the same axis, else `0.0` (gempy stores int8; we keep f64).
#[inline]
pub fn perp_matrix(i: usize, j: usize) -> f64 {
    if i == j {
        1.0
    } else {
        0.0
    }
}

/// Raw (un-floored) squared distance between two locations: `Σ (p1 − p2)²`.
#[inline]
pub fn r2(p1: [f64; 3], p2: [f64; 3]) -> f64 {
    let dx = p1[0] - p2[0];
    let dy = p1[1] - p2[1];
    let dz = p1[2] - p2[2];
    dx * dx + dy * dy + dz * dz
}

/// Coordinate difference `d = p1 − p2`.
#[inline]
pub fn diff(p1: [f64; 3], p2: [f64; 3]) -> [f64; 3] {
    [p1[0] - p2[0], p1[1] - p2[1], p1[2] - p2[2]]
}

/// Floored kernel-argument distance from a raw squared distance:
/// `r = sqrt(r² + 1e-10)` (KERNEL_MATH.md §0, §2). The cubic functions are
/// evaluated at this `r`.
#[inline]
pub fn kernel_arg_dist(r2: f64) -> f64 {
    (r2 + KERNEL_ARG_EPS).sqrt()
}

/// CG_CG block divisor: `r² + 1e-5` (KERNEL_MATH.md §0, §3.1). **Distinct** from
/// the kernel-argument floor — uses the squared distance and the `1e-5` epsilon.
#[inline]
pub fn cg_cg_divisor(r2: f64) -> f64 {
    r2 + CG_CG_EPS
}

/// The cross-block combined geometry `huv = hu − hv` for a surface-point /
/// orientation pair (KERNEL_MATH.md §2):
///
/// `huv_ref  = hu_ref  − hv_ref`, `huv_rest = hu_rest − hv_rest`.
///
/// `d` is the separation (surface point − orientation) and `j` is the
/// orientation's axis selector. `hu = d[j]`, `hv = −d[j]`, so for a single
/// orientation component this reduces to `huv = d[j] − (−d[j]) = 2·d[j]`.
/// Computed via [`hu`]/[`hv`] to keep the sign discipline visible.
#[inline]
pub fn huv(d: [f64; 3], j: usize) -> f64 {
    hu(d, j) - hv(d, j)
}

/// Debug-only precondition check (KERNEL_MATH.md §1.1 landmine): the no-clamp
/// cubic only produces sane covariance while every pairwise **normalized**
/// distance stays inside the range `a`. Panics in debug builds if violated;
/// compiled out in release.
#[inline]
pub fn debug_assert_within_range(r2: f64, a: f64) {
    debug_assert!(
        r2.sqrt() < a,
        "pairwise normalized distance {} >= range a={} — no-clamp cubic precondition violated; \
         data does not fit the normalized box (KERNEL_MATH.md §1.1)",
        r2.sqrt(),
        a
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hv_carries_leading_minus() {
        let d = [3.0, -2.0, 5.0];
        assert_eq!(hu(d, 0), 3.0);
        assert_eq!(hv(d, 0), -3.0);
        assert_eq!(hu(d, 1), -2.0);
        assert_eq!(hv(d, 1), 2.0);
    }

    #[test]
    fn perp_is_same_axis_indicator() {
        assert_eq!(perp_matrix(0, 0), 1.0);
        assert_eq!(perp_matrix(1, 1), 1.0);
        assert_eq!(perp_matrix(0, 1), 0.0);
        assert_eq!(perp_matrix(2, 0), 0.0);
    }

    #[test]
    fn huv_is_twice_component_for_single_orientation() {
        let d = [4.0, 7.0, -1.0];
        assert_eq!(huv(d, 0), 8.0);
        assert_eq!(huv(d, 1), 14.0);
        assert_eq!(huv(d, 2), -2.0);
    }

    #[test]
    fn distances_and_floors() {
        let p1 = [1.0, 2.0, 3.0];
        let p2 = [1.0, 2.0, 3.0];
        assert_eq!(r2(p1, p2), 0.0);
        // Coincident points: kernel arg is the sqrt of the floor, not zero.
        assert!((kernel_arg_dist(0.0) - 1e-10_f64.sqrt()).abs() < 1e-30);
        assert_eq!(cg_cg_divisor(0.0), 1e-5);

        let q = [4.0, 6.0, 3.0]; // dx=3, dy=4 -> r2 = 25
        assert_eq!(r2(p1, q), 25.0);
        assert!((kernel_arg_dist(25.0) - (25.0 + 1e-10_f64).sqrt()).abs() < 1e-12);
        assert_eq!(cg_cg_divisor(25.0), 25.0 + 1e-5);
    }

    #[test]
    fn two_epsilons_are_distinct() {
        // Guard against ever unifying the floors.
        assert_ne!(KERNEL_ARG_EPS, CG_CG_EPS);
        assert_eq!(KERNEL_ARG_EPS, 1e-10);
        assert_eq!(CG_CG_EPS, 1e-5);
    }

    #[test]
    fn within_range_passes_for_normalized_box() {
        // Normalized box half-extent ~0.25 -> max pairwise ~0.5 << a=1.7.
        debug_assert_within_range(0.5_f64 * 0.5, 1.7);
    }

    #[test]
    #[should_panic(expected = "no-clamp cubic precondition violated")]
    #[cfg(debug_assertions)]
    fn within_range_panics_when_exceeded() {
        debug_assert_within_range(4.0, 1.7); // r = 2.0 >= 1.7
    }
}
