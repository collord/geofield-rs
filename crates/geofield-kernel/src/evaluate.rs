//! Dual evaluation: `scalar_field` and `scalar_field_gradient` from the solved
//! weights (KERNEL_MATH.md §6).
//!
//! The kriging predictor is `Z(x) = Σ_i w_i · cov(L_i, Z(x))`, where `L_i` is
//! datum functional `i` and `w_i` its solved weight. **Crucially, the same
//! `cov(L_i, ·)` scaling that appears in `K` must appear here** — otherwise the
//! predictor is inconsistent with the weights. So each evaluation term mirrors
//! the corresponding assembly block's factors (`c_o`, `gi_res`, `i_res`, the
//! `huv` form), with `x` treated as a value point:
//!
//!   - gradient functional (dim, ori) vs value Z(x):
//!       c_o · gi_res · huv(x − ori, dim) · (C'(r)/r),   r = ‖x − ori‖
//!     (the CG_SP cross form §3.2 with the SP increment collapsed to a single
//!      value point — no ref subtraction).
//!   - surface-point increment k vs value Z(x):
//!       c_o · i_res · ( C(‖x − rest_k‖) − C(‖x − ref_k‖) )
//!     (the SP_SP four-term §3.3 with one side collapsed to the value point).
//!   - drift u: w_uni[u] · x[u]   (degree-1 linear basis {x, y, z}, §4).
//!
//! Evaluation runs in the **normalized** box: the query point is transformed by
//! `Transform::apply` first, exactly as the data were.
//!
//! ⚠ KERNEL_MATH.md §6/§7 zone: the exact eval-side factors (gi_res/i_res/c_o
//! and the huv doubling) are the part that is "untestable except against a gempy
//! dump". They are transcribed to MIRROR the assembly so weights and predictor
//! stay self-consistent; the reference-model fixture (step 9) is the external
//! arbiter. The gradient-consistency test (∇Z vs FD of Z) is self-checking for
//! internal consistency regardless of any global factor error.

use crate::assemble::AssembledGeometry;
use crate::geometry::{kernel_arg_dist, r2};
use crate::solve::Solution;

/// A field evaluator: borrows the assembled geometry and the solved weights.
pub struct Field<'a> {
    pub geom: &'a AssembledGeometry,
    pub weights: &'a [f64],
}

impl<'a> Field<'a> {
    pub fn new(geom: &'a AssembledGeometry, solution: &'a Solution) -> Self {
        Self {
            geom,
            weights: &solution.weights,
        }
    }

    /// Scalar field `Z(x)` at a **world** query point.
    pub fn scalar_field(&self, xyz: [f64; 3]) -> f64 {
        let x = self.geom.transform.apply(xyz);
        self.scalar_field_norm(x)
    }

    /// Scalar field at a point already in the **normalized** box.
    ///
    /// Re-derived from gempy's evaluation kernel `create_scalar_kernel`
    /// (degree 1) so the predictor is the faithful dual of the *corrected*
    /// assembly (raw-pole RHS, −gi_res U_SP, single-axis CG_SP, nuggets inside
    /// c_o). Per gempy:
    ///   sigma_0_grad_sp = gi_res · hu_ref · k_p_ref          (hu_ref=(ori−x)[di])
    ///   sigma_0_sp      = −i_res · (k_rest_rest − k_ref_ref) (value-point form)
    ///   cov             = c_o·(sigma_0_sp + sigma_0_grad_sp) + uni_drift
    ///   uni_drift       = gi_res · x[u]                       (degree-1 linear basis)
    pub fn scalar_field_norm(&self, x: [f64; 3]) -> f64 {
        let kf = self.geom.opts.kernel;
        let a = self.geom.opts.range;
        let c_o = self.geom.opts.c_o;
        let gi_res = self.geom.opts.gi_res;
        let i_res = self.geom.opts.i_res;

        let mut z = 0.0;

        // Gradient functionals: c_o·gi_res·(ori − x)[dim]·(C'/r).
        for o in 0..self.geom.n_ori {
            let ori = self.geom.ori_pos[o];
            let k_p = kf.p_div_r(kf.arg(r2(x, ori)), a); // C'(r)/r
            for dim in 0..3 {
                let w = self.weights[self.geom.grad_index(dim, o)];
                z += w * c_o * gi_res * (ori[dim] - x[dim]) * k_p;
            }
        }

        // Surface-point increments: c_o·(−i_res)·(C(‖x−rest‖) − C(‖x−ref‖)).
        for k in 0..self.geom.n_sp_inc {
            let (rest, refp) = self.geom.sp_increments[k];
            let c_rest = kf.c(kf.arg(r2(x, rest)), a);
            let c_ref = kf.c(kf.arg(r2(x, refp)), a);
            let w = self.weights[self.geom.sp_index(k)];
            z += w * c_o * (-i_res) * (c_rest - c_ref);
        }

        // Degree-1 drift: gi_res · linear basis {x, y, z}.
        for u in 0..3 {
            let w = self.weights[self.geom.uni_index(u)];
            z += w * gi_res * x[u];
        }

        z
    }

    /// Analytic gradient `∇Z` at a **world** query point, returned in **world**
    /// coordinates (chain rule through the normalized-box transform).
    pub fn scalar_field_gradient(&self, xyz: [f64; 3]) -> [f64; 3] {
        let x = self.geom.transform.apply(xyz);
        let g_norm = self.scalar_field_gradient_norm(x);
        // ∂x_norm/∂x_world = scale (isotropic), so ∇_world = scale · ∇_norm.
        let s = self.geom.transform.scale;
        [g_norm[0] * s, g_norm[1] * s, g_norm[2] * s]
    }

    /// Analytic gradient w.r.t. the **normalized** coordinates.
    pub fn scalar_field_gradient_norm(&self, x: [f64; 3]) -> [f64; 3] {
        let kf = self.geom.opts.kernel;
        let a = self.geom.opts.range;
        let c_o = self.geom.opts.c_o;
        let gi_res = self.geom.opts.gi_res;
        let i_res = self.geom.opts.i_res;

        let mut g = [0.0_f64; 3];

        // Gradient functionals: term = c_o·gi_res·w·(ori − x)[dim]·k_p(r), r=‖x−ori‖.
        // Let d = x − ori, so (ori − x)[dim] = −d[dim]. ∂r/∂x_e = d[e]/r,
        // dk_p/dr = d(C'/r)/dr = (k_a − k_p)/r.
        // ∂/∂x_e [ −d[dim]·k_p ] = −δ(e,dim)·k_p − d[dim]·(dk_p/dr)·d[e]/r.
        // `r` here is the geometric (floored) distance; `k_p`/`k_a` are evaluated at
        // the kernel's own argument (`r²` for gempy `exponential`, §1c) — the same
        // values, so the chain rule is unchanged.
        for o in 0..self.geom.n_ori {
            let ori = self.geom.ori_pos[o];
            let rr2 = r2(x, ori);
            let r = kernel_arg_dist(rr2);
            let k_p = kf.p_div_r(kf.arg(rr2), a);
            let k_a = kf.a(kf.arg(rr2), a);
            let dkp_dr = (k_a - k_p) / r; // d(C'/r)/dr
            let d = [x[0] - ori[0], x[1] - ori[1], x[2] - ori[2]];
            for dim in 0..3 {
                let w = self.weights[self.geom.grad_index(dim, o)];
                let coeff = w * c_o * gi_res;
                for e in 0..3 {
                    let delta = if e == dim { 1.0 } else { 0.0 };
                    let dr_dxe = d[e] / r;
                    let term = -delta * k_p - d[dim] * dkp_dr * dr_dxe;
                    g[e] += coeff * term;
                }
            }
        }

        // Surface-point increments: term = c_o·(−i_res)·w·(C(‖x−rest‖) − C(‖x−ref‖)).
        // ∂/∂x_e C(‖x−p‖) = (C'/r)·(x−p)[e] = k_p·(x−p)[e].
        for k in 0..self.geom.n_sp_inc {
            let (rest, refp) = self.geom.sp_increments[k];
            let w = self.weights[self.geom.sp_index(k)];
            let kp_rest = kf.p_div_r(kf.arg(r2(x, rest)), a);
            let kp_ref = kf.p_div_r(kf.arg(r2(x, refp)), a);
            let coeff = w * c_o * (-i_res);
            for e in 0..3 {
                let g_rest = kp_rest * (x[e] - rest[e]);
                let g_ref = kp_ref * (x[e] - refp[e]);
                g[e] += coeff * (g_rest - g_ref);
            }
        }

        // Drift: ∂/∂x_e [ Σ_u w_u · gi_res · x_u ] = gi_res · w_e.
        for e in 0..3 {
            g[e] += gi_res * self.weights[self.geom.uni_index(e)];
        }

        g
    }

    /// Recover the isovalue at a surface point of the given interface (the field
    /// evaluated at that surface point's world coordinates). For a well-posed
    /// system all surface points of one interface should yield (nearly) the same
    /// value — asserted by callers / litho.
    pub fn isovalue_at(&self, world_xyz: [f64; 3]) -> f64 {
        self.scalar_field(world_xyz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assemble::{assemble, KernelOptions};
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::solve::solve;

    fn dipping_series() -> Series {
        // A small dipping single-series model: one interface (3 points) + 2
        // orientations with a common dip. World coords; rescaling handles the box.
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 1.0], 0),
            SurfacePoint::new([1.0, 0.0, 1.3], 0),
            SurfacePoint::new([0.0, 1.0, 1.05], 0),
        ];
        let oris = vec![
            Orientation::new([0.3, 0.3, 1.1], [-0.3, 0.0, 1.0]),
            Orientation::new([0.7, 0.6, 1.2], [-0.3, 0.0, 1.0]),
        ];
        Series::new(sps, oris)
    }

    #[test]
    fn isovalues_at_one_interface_are_consistent() {
        let s = dipping_series();
        let sys = assemble(&s, KernelOptions::default());
        let sol = solve(&sys);
        let field = Field::new(&sys.geom, &sol);

        let vals: Vec<f64> = s
            .surface_points
            .iter()
            .map(|sp| field.scalar_field(sp.xyz))
            .collect();
        // All points are on the same interface -> same isovalue (within tol).
        let v0 = vals[0];
        for (i, v) in vals.iter().enumerate() {
            assert!(
                (v - v0).abs() < 1e-6,
                "isovalue mismatch at sp {i}: {v} vs {v0} (all={vals:?})"
            );
        }
    }

    #[test]
    fn analytic_gradient_matches_finite_difference() {
        let s = dipping_series();
        let sys = assemble(&s, KernelOptions::default());
        let sol = solve(&sys);
        let field = Field::new(&sys.geom, &sol);

        let h = 1e-6;
        for p in [
            [0.4, 0.4, 1.1],
            [0.2, 0.7, 1.0],
            [0.6, 0.3, 1.25],
            [0.5, 0.5, 1.15],
        ] {
            let analytic = field.scalar_field_gradient(p);
            for e in 0..3 {
                let mut pp = p;
                let mut pm = p;
                pp[e] += h;
                pm[e] -= h;
                let fd = (field.scalar_field(pp) - field.scalar_field(pm)) / (2.0 * h);
                let tol = 1e-4 * (1.0 + analytic[e].abs());
                assert!(
                    (analytic[e] - fd).abs() < tol,
                    "grad[{e}] at {p:?}: analytic={} fd={}",
                    analytic[e],
                    fd
                );
            }
        }
    }
}

