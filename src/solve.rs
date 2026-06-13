//! Factor the symmetric-indefinite system and hold the dual weights
//! (KERNEL_MATH.md §6).
//!
//! `K` is symmetric **indefinite** (the drift `0` border kills positive-
//! definiteness). We factor once with **Bunch–Kaufman / LDLᵀ** (faer `Lblt`) and
//! solve for `w = [w_grad, w_sp, w_drift]` against `b = [b_grad, 0, 0]`.
//!
//! A Cholesky (LLT) that "succeeds" would mean the drift border was assembled
//! wrong — we deliberately do NOT use LLT here.

use faer::linalg::solvers::Solve;
use faer::{Mat, Side};

use crate::assemble::System;

/// The factored system plus its solved dual weights.
pub struct Solution {
    /// Dual weights `w`, in our index layout (grad rows, then sp, then drift).
    pub weights: Vec<f64>,
    /// Residual `‖K w − b‖₂` (a faithfulness check on the solve).
    pub residual: f64,
}

impl Solution {
    /// Number of dual weights (= matrix dimension).
    pub fn len(&self) -> usize {
        self.weights.len()
    }
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }
}

/// Factor `sys.k` (Bunch–Kaufman) and solve for the dual weights against `sys.b`.
pub fn solve(sys: &System) -> Solution {
    let n = sys.dim;

    // Build a faer dense matrix from our row-major storage. K is symmetric; we
    // pass the full matrix and let Lblt read the lower side.
    let k = Mat::from_fn(n, n, |i, j| sys.k[i * n + j]);
    let mut rhs = Mat::from_fn(n, 1, |i, _| sys.b[i]);

    // Bunch–Kaufman factorization of the symmetric indefinite matrix.
    let lblt = k.lblt(Side::Lower);
    lblt.solve_in_place(rhs.as_mut());

    let weights: Vec<f64> = (0..n).map(|i| rhs[(i, 0)]).collect();

    // Residual ‖K w − b‖₂ using the original (unfactored) K.
    let mut residual_sq = 0.0;
    for i in 0..n {
        let mut row = 0.0;
        for j in 0..n {
            row += sys.k[i * n + j] * weights[j];
        }
        let r = row - sys.b[i];
        residual_sq += r * r;
    }

    Solution {
        weights,
        residual: residual_sq.sqrt(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::{assemble, KernelOptions};
    use crate::data::{Orientation, Series, SurfacePoint};

    fn tiny_series() -> Series {
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([1.0, 0.0, 0.2], 0),
        ];
        let oris = vec![Orientation::new([0.5, 0.0, 0.1], [0.0, 0.0, 1.0])];
        Series::new(sps, oris)
    }

    #[test]
    fn solve_residual_is_tiny() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        let sol = solve(&sys);
        assert_eq!(sol.len(), sys.dim);
        assert!(
            sol.residual < 1e-9,
            "solve residual too large: {}",
            sol.residual
        );
    }

    #[test]
    fn solving_a_known_indefinite_system() {
        // Sanity-check the faer Lblt path on a tiny hand-built symmetric
        // indefinite system: [[0,1],[1,0]] x = [2,3] -> x = [3,2].
        let k = Mat::from_fn(2, 2, |i, j| if i == j { 0.0 } else { 1.0 });
        let mut rhs = Mat::from_fn(2, 1, |i, _| if i == 0 { 2.0 } else { 3.0 });
        let lblt = k.lblt(Side::Lower);
        lblt.solve_in_place(rhs.as_mut());
        let x0: f64 = rhs[(0, 0)];
        let x1: f64 = rhs[(1, 0)];
        assert!((x0 - 3.0).abs() < 1e-12);
        assert!((x1 - 2.0).abs() < 1e-12);
    }
}
