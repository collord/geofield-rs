//! Assembly of the symmetric-indefinite cokriging matrix `K` and RHS `b` for a
//! **single series**, degree-1 drift (KERNEL_MATH.md §3–§4, §6).
//!
//! Index layout (the §7 open item — confirmed by the block diff in step 5):
//!   - gradient functionals first, interleaved by dimension as gempy does:
//!     row(dim, ori) = dim * n_ori + ori, for dim in 0..3, ori in 0..n_ori;
//!     total `3 * n_ori` rows.
//!   - surface-point increment rows next: one per non-reference surface point,
//!     in input order; total `n_sp − n_interfaces` rows.
//!   - universal drift rows last: `n_uni_eq = 3` for degree 1 (basis {x,y,z}).
//!
//! Everything is f64. The matrix is symmetric **indefinite** (the drift `0`
//! border kills positive-definiteness) → Bunch-Kaufman at the solve step.

use crate::data::{Series, Transform};
use crate::geometry::{debug_assert_within_range, diff, hu, hv, perp_matrix, r2};
use crate::kernel::KernelFunction;

/// Live kernel options (read from the validation fixture, never hardcoded in
/// Iterative, residual-guided surface-point decimation (a kriging "coreset").
///
/// Borehole datasets oversample smooth contacts: thousands of nearly-redundant points
/// blow up the dense cokriging matrix (~N³ solve) AND wreck its conditioning (the
/// near-redundant rows form a large near-null-space). This thins the points WITHOUT
/// discarding information: fit a coarse seed subset, then **re-add only the points the
/// coarse field mis-predicts** by more than `threshold_frac · isovalue_spread` (the
/// fold hinges / anomalies the seed missed), and iterate. A point in a flat redundant
/// cluster has ~zero residual against the coarse field and stays out; a genuinely
/// informative point always generates residual and is kept. The data is honored to a
/// tolerance, not uniformly subsampled.
///
/// `None` decimation (the default) keeps ALL points — the gempy-bit-exact path the
/// validation suite pins.
#[derive(Debug, Clone, Copy)]
pub struct DecimateConfig {
    /// Seed grid cell size in NORMALIZED coordinates (the rescaled box has side ~0.5).
    /// One seed point per occupied cell per interface; the interface reference point is
    /// always kept. ~`range/20` (e.g. 0.08) is a reasonable coarse seed.
    pub seed_cell: f64,
    /// Re-add threshold as a fraction of the interface's recovered isovalue spread: a
    /// point omitted from the current fit is re-added if the field mis-predicts it by
    /// more than `threshold_frac · spread`. Smaller → keep more points (closer to exact).
    pub threshold_frac: f64,
    /// Hard cap on kept points per block (stops the loop even if residuals remain). The
    /// solve cost bound. `0` = no budget.
    pub point_budget: usize,
    /// Max re-add iterations (each one refits). Small (3–6) is plenty in practice.
    pub max_iters: usize,
}

impl Default for DecimateConfig {
    fn default() -> Self {
        Self {
            seed_cell: 0.08,
            threshold_frac: 0.1,
            point_budget: 0,
            max_iters: 5,
        }
    }
}

/// tests — KERNEL_MATH.md §7). Defaults mirror gempy's octree defaults.
#[derive(Debug, Clone, Copy)]
pub struct KernelOptions {
    /// Covariance kernel (gempy `kernel_function`). Default [`KernelFunction::Cubic`],
    /// the original validated path.
    pub kernel: KernelFunction,
    /// Kernel range `a`.
    pub range: f64,
    /// Covariance scaling constant `c_o`.
    pub c_o: f64,
    /// SP-block resolution weight (fixed `4.0`).
    pub i_res: f64,
    /// Cross-block resolution weight (fixed `2.0`).
    pub gi_res: f64,
    /// Universal drift degree (only `1` implemented).
    pub uni_degree: u32,
    /// Nugget added on the gradient sub-block diagonal.
    pub grad_nugget: f64,
    /// Nugget added on the surface-point sub-block diagonal.
    pub sp_nugget: f64,
    /// Optional iterative surface-point decimation. `None` keeps all points (gempy
    /// bit-exact); `Some` thins them via the residual-guided coreset.
    pub decimate: Option<DecimateConfig>,
}

impl Default for KernelOptions {
    fn default() -> Self {
        Self {
            kernel: KernelFunction::Cubic,
            range: 1.7,
            c_o: 10.0,
            i_res: 4.0,
            gi_res: 2.0,
            uni_degree: 1,
            grad_nugget: 1e-5,
            sp_nugget: 1e-5,
            decimate: None,
        }
    }
}

/// Normalized geometry: input data mapped into the rescaled box, plus the
/// derived index layout. Built once and shared between assembly and evaluation.
pub struct AssembledGeometry {
    pub transform: Transform,
    pub opts: KernelOptions,
    /// Orientation positions in normalized coords.
    pub ori_pos: Vec<[f64; 3]>,
    /// Orientation poles in the normalized box (gempy `dip_gradients`):
    /// `Transform::apply_gradient` — magnitude-preserving; equals the raw pole when
    /// the scale is isotropic. See build of this field.
    pub ori_pole: Vec<[f64; 3]>,
    /// Per non-reference surface point: (rest position, ref position), both
    /// normalized, in input order.
    pub sp_increments: Vec<([f64; 3], [f64; 3])>,
    pub n_ori: usize,
    pub n_sp_inc: usize,
    pub n_uni: usize,
}

impl AssembledGeometry {
    pub fn build(series: &Series, opts: KernelOptions) -> Self {
        assert_eq!(opts.uni_degree, 1, "only degree-1 drift implemented");
        let transform =
            Transform::from_input_points(&series.surface_points, &series.orientations);

        let ori_pos: Vec<[f64; 3]> = series
            .orientations
            .iter()
            .map(|o| transform.apply(o.xyz))
            .collect();
        // gempy's `dip_gradients`: the pole through `transform_gradient(...,
        // preserve_magnitude=True)` (§5.1) — the inverse-transpose direction at the input
        // magnitude. For an isotropic scale that is the raw pole (which is why an earlier
        // isotropic-only block diff read it as "raw"; the plain inverse-transpose without
        // the magnitude step was off by exactly 1/scale). For an anisotropic scale the
        // direction tilts, and the RHS must carry the tilted pole.
        let ori_pole: Vec<[f64; 3]> =
            series.orientations.iter().map(|o| transform.apply_gradient(o.pole)).collect();

        // Increment rows: every surface point that is NOT its interface's
        // reference, in input order.
        let mut sp_increments = Vec::new();
        for (i, sp) in series.surface_points.iter().enumerate() {
            let ref_idx = series.reference_per_interface[&sp.interface_id];
            if i == ref_idx {
                continue;
            }
            let rest = transform.apply(sp.xyz);
            let refp = transform.apply(series.surface_points[ref_idx].xyz);
            sp_increments.push((rest, refp));
        }

        let n_ori = ori_pos.len();
        let n_sp_inc = sp_increments.len();
        let n_uni = 3; // degree 1

        Self {
            transform,
            opts,
            ori_pos,
            ori_pole,
            sp_increments,
            n_ori,
            n_sp_inc,
            n_uni,
        }
    }

    /// Total matrix dimension.
    pub fn dim(&self) -> usize {
        3 * self.n_ori + self.n_sp_inc + self.n_uni
    }

    /// Row/col index of gradient functional (dim, ori) — interleaved by
    /// dimension (gempy layout).
    #[inline]
    pub fn grad_index(&self, dim: usize, ori: usize) -> usize {
        dim * self.n_ori + ori
    }

    /// Row/col index of surface-point increment `k`.
    #[inline]
    pub fn sp_index(&self, k: usize) -> usize {
        3 * self.n_ori + k
    }

    /// Row/col index of universal drift equation `u` (u in 0..3).
    #[inline]
    pub fn uni_index(&self, u: usize) -> usize {
        3 * self.n_ori + self.n_sp_inc + u
    }
}

/// The assembled system.
pub struct System {
    pub geom: AssembledGeometry,
    /// Dense symmetric-indefinite matrix, row-major, `dim*dim`.
    pub k: Vec<f64>,
    /// RHS vector, length `dim`.
    pub b: Vec<f64>,
    pub dim: usize,
}

impl System {
    #[inline]
    fn set(&mut self, i: usize, j: usize, v: f64) {
        self.k[i * self.dim + j] = v;
    }
    #[inline]
    fn add(&mut self, i: usize, j: usize, v: f64) {
        self.k[i * self.dim + j] += v;
    }
    #[inline]
    pub fn at(&self, i: usize, j: usize) -> f64 {
        self.k[i * self.dim + j]
    }

    /// Dump the full matrix (row-major) and RHS for the validation diff.
    pub fn dump_matrix(&self) -> (&[f64], &[f64], usize) {
        (&self.k, &self.b, self.dim)
    }
}

/// Assemble `K` and `b` for a single series (KERNEL_MATH.md §3–§4, §6).
pub fn assemble(series: &Series, opts: KernelOptions) -> System {
    let geom = AssembledGeometry::build(series, opts);
    let dim = geom.dim();
    let a = opts.range;
    let kf = opts.kernel;

    let mut sys = System {
        k: vec![0.0; dim * dim],
        b: vec![0.0; dim],
        geom,
        dim,
    };

    assemble_cov_grad(&mut sys, kf, a);
    assemble_cov_grad_sp(&mut sys, kf, a);
    assemble_cov_sp(&mut sys, kf, a);
    // Nuggets are added to the covariance blocks BEFORE the c_o scaling — gempy
    // folds them into cov_grad / cov_surface_points, then multiplies the sum by
    // c_o (`cov = c_o*(cov_grad + cov_sp + cov_grad_sp) + drift`). Adding them
    // after c_o (as a prior version did) left the diagonal short by a factor of
    // c_o on the nugget term (the 0.9981 CG_CG block-diff ratio). See Step C.
    add_nuggets(&mut sys);
    scale_by_c_o(&mut sys);
    assemble_drift_border(&mut sys);
    assemble_rhs(&mut sys);

    sys
}

/// §3.1 CG_CG — `_get_cov_grad`. Writes the raw block (pre-`c_o`, pre-nugget).
///
/// `cov_grad = (hu·hv)/(r²+1e-5)·(−k_p_ref + k_a) − k_p_ref·perp` — with the divisor
/// per [`KernelFunction::cg_cg_divisor`] (`r⁴+1e-5` for gempy's `exponential`, §1c.4).
fn assemble_cov_grad(sys: &mut System, kf: KernelFunction, a: f64) {
    let n_ori = sys.geom.n_ori;
    for oa in 0..n_ori {
        for ob in 0..n_ori {
            let pa = sys.geom.ori_pos[oa];
            let pb = sys.geom.ori_pos[ob];
            let d = diff(pa, pb);
            let rr2 = r2(pa, pb);
            if kf.needs_in_range_precondition() {
                debug_assert_within_range(rr2, a);
            }
            let r = kf.arg(rr2);
            let k_p_ref = kf.p_div_r(r, a); // C'(r)/r
            let k_a = kf.a(r, a); // C''(r)
            let div = kf.cg_cg_divisor(rr2); // r² + 1e-5 (r⁴ + 1e-5 for exponential)
            for di in 0..3 {
                for dj in 0..3 {
                    let huv_hu = hu(d, di);
                    let huv_hv = hv(d, dj);
                    let perp = perp_matrix(di, dj);
                    let val = (huv_hu * huv_hv) / div * (-k_p_ref + k_a) - k_p_ref * perp;
                    let i = sys.geom.grad_index(di, oa);
                    let j = sys.geom.grad_index(dj, ob);
                    sys.set(i, j, val);
                }
            }
        }
    }
}

/// §3.2 CG_SP — `_get_cross_cov_grad_sp`. Writes both the (grad, sp) block and
/// its symmetric (sp, grad) transpose.
///
/// `cov_grad_sp = gi_res·(huv_ref·k_p_ref − huv_rest·k_p_rest)`.
///
/// CG_SP `huv` convention (confirmed by the gempy block diff + source
/// `_kernels_assembler._compute_distances_new`): for the gradient↔SP cross
/// block, `huv_ref` reduces to the **single-axis** separation `(ori − point)[di]`
/// along the gradient functional's axis `di` — factor **1**, signed `ori − point`.
/// This is NOT the CG_CG `huv = hu − hv = 2·d` reduction (that helper conflates
/// the two blocks' selector conventions). Using `huv()` here produced a uniform
/// `−2×` error vs gempy; this single-axis `(ori − point)[di]` form matches.
fn assemble_cov_grad_sp(sys: &mut System, kf: KernelFunction, a: f64) {
    let gi_res = sys.geom.opts.gi_res;
    for oa in 0..sys.geom.n_ori {
        let ori = sys.geom.ori_pos[oa];
        for k in 0..sys.geom.n_sp_inc {
            let (rest, refp) = sys.geom.sp_increments[k];

            let r_ref = kf.arg(r2(refp, ori));
            let r_rest = kf.arg(r2(rest, ori));
            let k_p_ref = kf.p_div_r(r_ref, a);
            let k_p_rest = kf.p_div_r(r_rest, a);

            for di in 0..3 {
                // huv = (ori − point)[di], single axis, factor 1.
                let huv_ref = ori[di] - refp[di];
                let huv_rest = ori[di] - rest[di];
                let val = gi_res * (huv_ref * k_p_ref - huv_rest * k_p_rest);
                let gi = sys.geom.grad_index(di, oa);
                let si = sys.geom.sp_index(k);
                sys.set(gi, si, val);
                sys.set(si, gi, val);
            }
        }
    }
}

/// §3.3 SP_SP — `_get_cov_surface_points`. The four-term second difference.
///
/// `cov_sp = i_res·(k_rest_rest − k_rest_ref − k_ref_rest + k_ref_ref)`.
fn assemble_cov_sp(sys: &mut System, kf: KernelFunction, a: f64) {
    let i_res = sys.geom.opts.i_res;
    for ka in 0..sys.geom.n_sp_inc {
        let (rest_a, ref_a) = sys.geom.sp_increments[ka];
        for kb in 0..sys.geom.n_sp_inc {
            let (rest_b, ref_b) = sys.geom.sp_increments[kb];

            let k_rest_rest = kf.c(kf.arg(r2(rest_a, rest_b)), a);
            let k_rest_ref = kf.c(kf.arg(r2(rest_a, ref_b)), a);
            let k_ref_rest = kf.c(kf.arg(r2(ref_a, rest_b)), a);
            let k_ref_ref = kf.c(kf.arg(r2(ref_a, ref_b)), a);

            let val = i_res * (k_rest_rest - k_rest_ref - k_ref_rest + k_ref_ref);
            let i = sys.geom.sp_index(ka);
            let j = sys.geom.sp_index(kb);
            sys.set(i, j, val);
        }
    }
}

/// `cov = c_o · (cov_grad + cov_sp + cov_grad_sp)` (§3 top). Applied to the
/// covariance sub-block only (the drift border is added outside `c_o`, so this
/// runs before the border is written).
fn scale_by_c_o(sys: &mut System) {
    let c_o = sys.geom.opts.c_o;
    let n_cov = 3 * sys.geom.n_ori + sys.geom.n_sp_inc;
    for i in 0..n_cov {
        for j in 0..n_cov {
            let idx = i * sys.dim + j;
            sys.k[idx] *= c_o;
        }
    }
}

/// Nuggets on the diagonals: gradient nugget on the gradient sub-block diagonal,
/// SP nugget on the SP sub-block diagonal (§3.1, §3.3). Added to the covariance
/// blocks BEFORE `c_o` scaling — gempy folds them into cov_grad /
/// cov_surface_points and then scales the whole sum by `c_o`, so the nugget
/// terms are themselves `c_o`-scaled. Values come from `KernelOptions` (read
/// live from the fixture by the CLI; never hardcoded).
fn add_nuggets(sys: &mut System) {
    let gn = sys.geom.opts.grad_nugget;
    let sn = sys.geom.opts.sp_nugget;
    for dim in 0..3 {
        for o in 0..sys.geom.n_ori {
            let i = sys.geom.grad_index(dim, o);
            sys.add(i, i, gn);
        }
    }
    for k in 0..sys.geom.n_sp_inc {
        let i = sys.geom.sp_index(k);
        sys.add(i, i, sn);
    }
}

/// Universal drift border (§4), degree 1. Added **outside** `c_o`.
///
/// U_G (§4.2): gradient row (dim, ori) ↔ drift col `u` gets `1` iff `dim == u`.
/// U_SP (§4.1): SP increment row `k` ↔ drift col `u` gets
///   `−gi_res·(rest[u] − ref[u])`.
/// The drift–drift diagonal block is `0` (this is what makes `K` indefinite).
///
/// HISTORY — this is the §4.1 form, and it is CORRECT (confirmed by the step-5
/// block diff against gempy's actual border: `validation/dump_kernel.py`, and by
/// gempy source `_get_universal_sp_terms`: `usp = −selector·gi_res·(usp_rest −
/// usp_ref)`). A step-7 session wrongly removed the `−gi_res` factor here,
/// reasoning that it broke increment consistency. That was a MISDIAGNOSIS: the
/// `−gi_res` U_SP and the (then-wrong) `1/scale`-scaled RHS were *mutually
/// compensating* — two errors that together produced a locally self-consistent
/// but non-gempy field. The increment-consistency probe passed because it tests
/// "is this a valid potential" (scale/level-invariant), NOT "is this gempy's
/// potential". The block diff is the only thing that localized it. Lesson: do
/// not trust internal-consistency probes to validate sign/scale conventions.
fn assemble_drift_border(sys: &mut System) {
    let gi_res = sys.geom.opts.gi_res;

    // U_G: ug = e_dim · e_u = (dim == u).
    for dim in 0..3 {
        for o in 0..sys.geom.n_ori {
            let gi = sys.geom.grad_index(dim, o);
            for u in 0..3 {
                let ui = sys.geom.uni_index(u);
                let v = if dim == u { 1.0 } else { 0.0 };
                sys.set(gi, ui, v);
                sys.set(ui, gi, v);
            }
        }
    }

    // U_SP (§4.1): −gi_res·(rest[u] − ref[u]).
    for k in 0..sys.geom.n_sp_inc {
        let (rest, refp) = sys.geom.sp_increments[k];
        let si = sys.geom.sp_index(k);
        for u in 0..3 {
            let ui = sys.geom.uni_index(u);
            let v = -gi_res * (rest[u] - refp[u]);
            sys.set(si, ui, v);
            sys.set(ui, si, v);
        }
    }
    // Drift–drift 0 block is already 0 (left untouched).
}

/// RHS `b = [b_grad, 0, 0]` (§6 / §7 RHS note). `b_grad` holds the rescaled
/// orientation pole components, interleaved by dimension to match the gradient
/// row layout.
///
/// NOTE (KERNEL_MATH.md §7.4 — genuine open item): pole normalization and any
/// `c_o`/`gi_res` RHS multiplier are NOT yet pinned from source. We write the
/// rescaled pole components directly; the step-5 RHS diff and the step-7
/// gradient-magnitude check confirm the scaling. A global RHS scale leaves the
/// field *shape* correct but corrupts isovalues/gradient magnitudes.
fn assemble_rhs(sys: &mut System) {
    for dim in 0..3 {
        for o in 0..sys.geom.n_ori {
            let i = sys.geom.grad_index(dim, o);
            sys.b[i] = sys.geom.ori_pole[o][dim];
        }
    }
    // SP and drift entries of b remain 0.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Orientation, SurfacePoint};

    /// Smallest model: 2 surface points on 1 interface + 1 orientation.
    fn tiny_series() -> Series {
        let sps = vec![
            SurfacePoint::new([0.0, 0.0, 0.0], 0),
            SurfacePoint::new([1.0, 0.0, 0.2], 0),
        ];
        let oris = vec![Orientation::new([0.5, 0.0, 0.1], [0.0, 0.0, 1.0])];
        Series::new(sps, oris)
    }

    #[test]
    fn dimensions_are_correct() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        // 3*1 grad + 1 sp increment + 3 drift = 7
        assert_eq!(sys.dim, 7);
        assert_eq!(sys.k.len(), 49);
        assert_eq!(sys.b.len(), 7);
    }

    #[test]
    fn matrix_is_symmetric() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        for i in 0..sys.dim {
            for j in 0..sys.dim {
                assert!(
                    (sys.at(i, j) - sys.at(j, i)).abs() < 1e-12,
                    "asymmetry at ({i},{j}): {} vs {}",
                    sys.at(i, j),
                    sys.at(j, i)
                );
            }
        }
    }

    #[test]
    fn drift_diagonal_block_is_zero() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        for u in 0..3 {
            for v in 0..3 {
                let iu = sys.geom.uni_index(u);
                let iv = sys.geom.uni_index(v);
                assert_eq!(sys.at(iu, iv), 0.0, "drift-drift block must be 0");
            }
        }
    }

    #[test]
    fn ug_border_is_identity_pattern() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        // Single orientation: grad row (dim,0) vs drift col u -> 1 iff dim==u.
        for dim in 0..3 {
            for u in 0..3 {
                let gi = sys.geom.grad_index(dim, 0);
                let ui = sys.geom.uni_index(u);
                let expect = if dim == u { 1.0 } else { 0.0 };
                assert_eq!(sys.at(gi, ui), expect, "U_G ({dim},{u})");
            }
        }
    }

    #[test]
    fn rhs_holds_rescaled_poles_and_zeros_elsewhere() {
        let s = tiny_series();
        let sys = assemble(&s, KernelOptions::default());
        let expect_pole = sys.geom.ori_pole[0];
        for dim in 0..3 {
            let i = sys.geom.grad_index(dim, 0);
            assert_eq!(sys.b[i], expect_pole[dim]);
        }
        // SP + drift rows of b are zero.
        assert_eq!(sys.b[sys.geom.sp_index(0)], 0.0);
        for u in 0..3 {
            assert_eq!(sys.b[sys.geom.uni_index(u)], 0.0);
        }
    }
}
