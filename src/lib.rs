//! geofield-rs — universal-cokriging scalar field for a single, non-faulted series.
//!
//! Math source of truth: `KERNEL_MATH.md` in the repo root. Where this code and
//! that document disagree on a formula, sign, constant, or epsilon, the document
//! wins.
//!
//! Public entry point: [`Model::fit`], then [`Model::scalar_field`],
//! [`Model::scalar_field_gradient`], and [`Model::lithology_unit`].

pub mod assemble;
pub mod data;
pub mod evaluate;
pub mod fault;
pub mod geometry;
pub mod kernel;
pub mod litho;
pub mod mesh;
pub mod octree;
pub mod solve;
pub mod stack;

#[cfg(feature = "wasm")]
pub mod wasm;

use assemble::{assemble, KernelOptions, System};
use data::{Orientation, Series, SurfacePoint};
use evaluate::Field;
use mesh::normalize;
use fault::{BlockIndex, SingleBlock};
use litho::Lithology;
use solve::{solve, Solution};

pub use fault::{FaultStack, OneFault, PlanarFault};
pub use stack::{Relation, Stack, StackedSeries};

/// A potential field solved independently over one fault-bounded **block** —
/// its assembled system, solved weights, and the lithology isovalues recovered from
/// that block's own surface points. A non-faulted model has exactly one block.
struct Block {
    system: System,
    solution: Solution,
    lithology: Lithology,
}

impl Block {
    fn field(&self) -> Field<'_> {
        Field::new(&self.system.geom, &self.solution)
    }
}

/// A fitted potential-field model. In the common case it is one smooth cokriging
/// field; with a fault ([`Model::fit_faulted`]) it holds one independent field per
/// fault-bounded block and dispatches each query to the block the point lies in, so
/// the field is **discontinuous across the fault** (a contact is offset, GemPy's
/// finite-fault behaviour). Self-contained so it can cross an FFI / WASM boundary.
///
/// All eval methods (`scalar_field`, `scalar_field_gradient`, `lithology_unit`)
/// keep their signatures, so a faulted model is a drop-in everywhere a model is
/// used — including inside a [`Stack`]'s erode/onlap masking.
pub struct Model {
    /// One field per block; index `b` is the block `index.block_at` returns. A block
    /// with no surface points (a fault region with no data) is `None` and reports no
    /// contact there (field `+∞`, lithology unit 0).
    blocks: Vec<Option<Block>>,
    index: Box<dyn BlockIndex>,
    /// Representative interfaces for the whole model — the first non-empty block's
    /// lithology (block 0 in the common case). [`Model::lithology`] returns this; the
    /// stack reads it for a series' bounding contact while per-point sign tests route
    /// per-block. Empty only if every block is data-less.
    representative_lithology: Lithology,
    /// Largest within-interface isovalue disagreement at fit time (diagnostic, max
    /// over blocks).
    pub isovalue_spread: f64,
    /// Solve residual ‖K w − b‖₂ (diagnostic, max over blocks).
    pub residual: f64,
}

impl Model {
    /// Assemble, solve, and recover isovalues for a series — a single (non-faulted)
    /// block. Exactly `fit_faulted` with the trivial [`SingleBlock`] index, so all
    /// existing callers are unchanged.
    pub fn fit(series: &Series, opts: KernelOptions) -> Self {
        Self::fit_faulted(series, Box::new(SingleBlock), opts)
    }

    /// Fit a **faulted** model: partition the series' points/orientations by
    /// `index.block_at(position)`, solve one cokriging field per block over only that
    /// block's data, and recover per-block lithology. Eval dispatches to the block at
    /// the query point, so the field is discontinuous across the fault.
    ///
    /// A block that ends up with no surface points is left field-less (it has no
    /// contact); a query there returns `+∞` (above every interface → unit 0).
    pub fn fit_faulted(series: &Series, index: Box<dyn BlockIndex>, opts: KernelOptions) -> Self {
        let nblocks = index.nblocks();
        // Bucket points and orientations into their blocks by their own position.
        let mut sp_buckets: Vec<Vec<SurfacePoint>> = vec![Vec::new(); nblocks];
        let mut ori_buckets: Vec<Vec<Orientation>> = vec![Vec::new(); nblocks];
        for sp in &series.surface_points {
            sp_buckets[index.block_at(sp.xyz)].push(*sp);
        }
        for ori in &series.orientations {
            ori_buckets[index.block_at(ori.xyz)].push(*ori);
        }

        let mut blocks: Vec<Option<Block>> = Vec::with_capacity(nblocks);
        let mut isovalue_spread = 0.0_f64;
        let mut residual = 0.0_f64;
        for b in 0..nblocks {
            // A block with no surface points has no contact — field-less (`None`).
            if sp_buckets[b].is_empty() {
                blocks.push(None);
                continue;
            }
            // A block with points but NO orientation is under-constrained (cokriging
            // is singular without a gradient anchor). Rather than drop it — which
            // makes one side of a fault silently vanish — INFER an orientation from
            // the points' own geometry (`infer_orientation`): the best-fit plane
            // normal when ≥3 points define a plane, the flattest plane through the
            // line for 2/collinear points, flat (+Z) for a single point. This is
            // invisible when real orientations exist and keeps both fault blocks
            // meshable. (A fault plane that strands all orientations on one side is
            // exactly the harness's default-fault case.)
            let oris = std::mem::take(&mut ori_buckets[b]);
            let oris = if oris.is_empty() {
                vec![infer_orientation(&sp_buckets[b])]
            } else {
                oris
            };
            let sub = Series::new(std::mem::take(&mut sp_buckets[b]), oris);
            let system = assemble(&sub, opts);
            let solution = solve(&system);
            residual = residual.max(solution.residual);
            let (lithology, spread) = {
                let field = Field::new(&system.geom, &solution);
                Lithology::recover(&sub, &field)
            };
            isovalue_spread = isovalue_spread.max(spread);
            blocks.push(Some(Block {
                system,
                solution,
                lithology,
            }));
        }

        // Representative interfaces: the first non-empty block's (block 0 in the
        // common case). Empty Lithology only if every block was data-less.
        let representative_lithology = blocks
            .iter()
            .find_map(|b| b.as_ref().map(|b| b.lithology.clone()))
            .unwrap_or_else(|| Lithology {
                interfaces: Vec::new(),
            });

        Self {
            blocks,
            index,
            representative_lithology,
            isovalue_spread,
            residual,
        }
    }

    /// The lithology recovered for block `b` (the fault-bounded domain), or `None`
    /// if `b` is out of range or data-less. For a non-faulted model, block 0 is the
    /// whole model.
    pub fn block_lithology(&self, b: usize) -> Option<&Lithology> {
        self.blocks.get(b).and_then(|x| x.as_ref()).map(|x| &x.lithology)
    }

    /// Number of fault-bounded blocks (1 if non-faulted).
    pub fn nblocks(&self) -> usize {
        self.blocks.len()
    }

    /// The block containing `p`, if it has a field.
    fn block_at(&self, p: [f64; 3]) -> Option<&Block> {
        self.blocks.get(self.index.block_at(p)).and_then(|b| b.as_ref())
    }

    /// Scalar field `Z(x)` at a world query point. In an empty (data-less) block,
    /// returns `+∞` — no contact, so every interface threshold places it in unit 0.
    pub fn scalar_field(&self, xyz: [f64; 3]) -> f64 {
        match self.block_at(xyz) {
            Some(b) => b.field().scalar_field(xyz),
            None => f64::INFINITY,
        }
    }

    /// Analytic gradient `∇Z` at a world query point (world coordinates). Zero in an
    /// empty block (the field is constant `+∞` there — no gradient).
    pub fn scalar_field_gradient(&self, xyz: [f64; 3]) -> [f64; 3] {
        match self.block_at(xyz) {
            Some(b) => b.field().scalar_field_gradient(xyz),
            None => [0.0, 0.0, 0.0],
        }
    }

    /// Lithology unit index at a world query point — classified by the block at `p`
    /// against THAT block's own recovered isovalues (correct across an offsetting
    /// fault). Unit 0 in an empty block.
    pub fn lithology_unit(&self, xyz: [f64; 3]) -> usize {
        match self.block_at(xyz) {
            Some(b) => b.lithology.unit_at(&b.field(), xyz),
            None => 0,
        }
    }

    /// The recovered per-interface isovalues (sorted ascending).
    ///
    /// For a faulted model these are **block 0's** isovalues, the representative set:
    /// [`Stack`] masking reads this for a series' bounding contact, while the
    /// per-point sign test routes through `scalar_field` (which is per-block), so the
    /// masking surface tracks the fault offset where it matters. Use
    /// [`Model::block_lithology`] for a specific block.
    pub fn lithology(&self) -> &Lithology {
        &self.representative_lithology
    }
}

/// Infer a fallback orientation (gradient anchor) for a block that has surface
/// points but no orientation of its own — from the geometry of the points
/// themselves, not a blind flat assumption.
///
/// The pole is taken from the **best-constrained interface** (the one with the most
/// points in this block — one good contact anchors the whole series gradient):
///   * **≥3 non-collinear points** → the best-fit plane normal (the points define a
///     plane; its normal is the dip).
///   * **2 points, or ≥3 collinear points** → the *flattest* plane containing that
///     line: the normal closest to vertical (`pole = P × H`, `P` the line direction,
///     `H` the horizontal in-line direction). The line constrains strike but not dip,
///     so the flattest dip is the prior.
///   * **1 point (or fully degenerate)** → flat-lying (`+Z`), the bare geological
///     default.
///
/// The pole is oriented "up" (`pole·+Z ≥ 0`) to match the field's sign convention,
/// and anchored at the chosen interface's centroid.
fn infer_orientation(points: &[SurfacePoint]) -> Orientation {
    // Group by interface; pick the interface with the most points in this block.
    let mut by_iface: std::collections::BTreeMap<u32, Vec<[f64; 3]>> =
        std::collections::BTreeMap::new();
    for p in points {
        by_iface.entry(p.interface_id).or_default().push(p.xyz);
    }
    let best = by_iface
        .values()
        .max_by_key(|v| v.len())
        .cloned()
        .unwrap_or_default();

    let centroid = |pts: &[[f64; 3]]| -> [f64; 3] {
        let n = pts.len().max(1) as f64;
        let mut c = [0.0; 3];
        for p in pts {
            c[0] += p[0];
            c[1] += p[1];
            c[2] += p[2];
        }
        [c[0] / n, c[1] / n, c[2] / n]
    };

    let up = |mut n: [f64; 3]| {
        if n[2] < 0.0 {
            n = [-n[0], -n[1], -n[2]];
        }
        n
    };

    let loc = centroid(&best);
    let pole = match best.len() {
        0 | 1 => [0.0, 0.0, 1.0], // bare default
        _ => {
            let c = centroid(&best);
            // Covariance (about centroid) of the chosen interface's points.
            let mut cov = [[0.0_f64; 3]; 3];
            for p in &best {
                let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                for r in 0..3 {
                    for k in 0..3 {
                        cov[r][k] += d[r] * d[k];
                    }
                }
            }
            let (evals, evecs) = sym3_eig(cov); // ascending eigenvalues
            // Spread along the smallest vs largest principal axis. If the second-
            // smallest eigenvalue is negligible relative to the largest, the points
            // span only a LINE (2 points, or collinear) → flattest-plane-through-line.
            let lambda_max = evals[2].max(1e-300);
            let planar = evals[1] / lambda_max > 1e-6;
            if planar {
                // Best-fit plane normal = smallest-eigenvalue eigenvector.
                up([evecs[0][0], evecs[1][0], evecs[2][0]])
            } else {
                // Line direction P = largest-eigenvalue eigenvector; flattest plane
                // normal through it = P × H, H the horizontal perpendicular to P_xy.
                let p_dir = [evecs[0][2], evecs[1][2], evecs[2][2]];
                up(flattest_plane_normal(p_dir))
            }
        }
    };
    Orientation::new(loc, normalize(pole))
}

/// Normal of the *flattest* plane (steepest-toward-horizontal, i.e. normal closest
/// to vertical) containing the line direction `p`. `pole = P × H` where `H` is the
/// unit horizontal vector perpendicular to `P`'s horizontal projection. Degenerates
/// to `+Z` when `P` is vertical (the line is vertical → flat is the only prior).
fn flattest_plane_normal(p: [f64; 3]) -> [f64; 3] {
    let pn = normalize(p);
    let p_xy = (pn[0] * pn[0] + pn[1] * pn[1]).sqrt();
    if p_xy < 1e-12 {
        return [0.0, 0.0, 1.0]; // vertical line → flat
    }
    // Horizontal direction perpendicular to P's map projection.
    let h = normalize([-pn[1], pn[0], 0.0]);
    // pole = P × H.
    [
        pn[1] * h[2] - pn[2] * h[1],
        pn[2] * h[0] - pn[0] * h[2],
        pn[0] * h[1] - pn[1] * h[0],
    ]
}

/// Eigendecomposition of a symmetric 3×3 matrix by cyclic Jacobi rotations.
/// Returns `(eigenvalues, eigenvectors)` with eigenvalues **ascending** and
/// `eigenvectors[r][c]` the `r`-th component of the `c`-th eigenvector (columns are
/// eigenvectors, parallel to the eigenvalue order). Robust for the small PSD
/// covariance matrices here; not performance-critical (called only on the fallback).
fn sym3_eig(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[0.0; 3]; 3];
    for i in 0..3 {
        v[i][i] = 1.0;
    }
    for _ in 0..50 {
        // Largest off-diagonal magnitude.
        let mut p = 0;
        let mut q = 1;
        let mut off = a[0][1].abs();
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > off {
                off = a[i][j].abs();
                p = i;
                q = j;
            }
        }
        if off < 1e-14 {
            break;
        }
        // Jacobi rotation zeroing a[p][q]. Tangent (Numerical-Recipes) form, which
        // avoids the angle-sign ambiguity: with θ_v = (aqq−app)/(2·apq),
        // t = tan θ = sign(θ_v)/(|θ_v| + √(θ_v²+1)), then c = 1/√(t²+1), s = t·c.
        let apq = a[p][q];
        let theta_v = (a[q][q] - a[p][p]) / (2.0 * apq);
        // sign(θv), treating θv == 0 as +1 so app==aqq still rotates by 45°.
        let sgn = if theta_v < 0.0 { -1.0 } else { 1.0 };
        let t = sgn / (theta_v.abs() + (theta_v * theta_v + 1.0).sqrt());
        let c = 1.0 / (t * t + 1.0).sqrt();
        let s = t * c;
        // A ← Jᵀ A J for J = R(p,q): rotate columns p,q, then rows p,q.
        for k in 0..3 {
            let akp = a[k][p];
            let akq = a[k][q];
            a[k][p] = c * akp - s * akq;
            a[k][q] = s * akp + c * akq;
        }
        for k in 0..3 {
            let apk = a[p][k];
            let aqk = a[q][k];
            a[p][k] = c * apk - s * aqk;
            a[q][k] = s * apk + c * aqk;
        }
        // Accumulate the rotation into the eigenvector columns.
        for k in 0..3 {
            let vkp = v[k][p];
            let vkq = v[k][q];
            v[k][p] = c * vkp - s * vkq;
            v[k][q] = s * vkp + c * vkq;
        }
    }
    let mut evals = [a[0][0], a[1][1], a[2][2]];
    // Sort ascending, permuting eigenvector columns with the eigenvalues.
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| evals[i].partial_cmp(&evals[j]).unwrap());
    let sorted_evals = [evals[order[0]], evals[order[1]], evals[order[2]]];
    let mut sorted_vecs = [[0.0; 3]; 3];
    for (col, &src) in order.iter().enumerate() {
        for row in 0..3 {
            sorted_vecs[row][col] = v[row][src];
        }
    }
    evals = sorted_evals;
    (evals, sorted_vecs)
}

#[cfg(test)]
mod fault_tests {
    use super::*;
    use crate::data::{Orientation, SurfacePoint};
    use crate::fault::{OneFault, PlanarFault};

    /// A flat contact OFFSET by a fault: z=600 on the negative side of the fault
    /// plane x=500, z=700 on the positive side (a 100-unit throw). The surface points
    /// reflect the offset; fitting with a `OneFault` at x=500 must reproduce each
    /// block's own contact and make the field discontinuous across the plane.
    fn faulted_offset_series() -> Series {
        let mut sps = Vec::new();
        let mut oris = Vec::new();
        for i in 0..5 {
            for j in 0..3 {
                let x = i as f64 * 200.0 + 100.0; // 100,300,500,700,900
                let y = j as f64 * 300.0 + 200.0;
                // Left of fault (x<500): contact at z=600; right (x>=500): z=700.
                let z = if x < 500.0 { 600.0 } else { 700.0 };
                sps.push(SurfacePoint::new([x, y, z], 0));
            }
        }
        // One orientation per side so each block's gradient is anchored.
        oris.push(Orientation::new([300.0, 500.0, 600.0], [0.0, 0.0, 1.0]));
        oris.push(Orientation::new([700.0, 500.0, 700.0], [0.0, 0.0, 1.0]));
        Series::new(sps, oris)
    }

    fn fault_x500() -> Box<OneFault> {
        Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])))
    }

    /// Fail-first guard: fitting the SAME offset data WITHOUT a fault (one smooth
    /// block) must NOT show the discontinuity — proving the discontinuity test pins
    /// the fault, not the data. A single smooth kriging field cannot represent the
    /// hard 100-unit offset, so it bends through it: the cross-x=500 jump collapses
    /// to the same order as within-block variation.
    #[test]
    fn unfaulted_offset_is_smooth_not_discontinuous() {
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        let z = 650.0;
        let just_left = model.scalar_field([499.0, 500.0, z]);
        let just_right = model.scalar_field([501.0, 500.0, z]);
        let jump = (just_right - just_left).abs();
        let within_left =
            (model.scalar_field([497.0, 500.0, z]) - model.scalar_field([499.0, 500.0, z])).abs();
        assert!(
            jump < 5.0 * within_left.max(1e-9),
            "unfaulted fit should be smooth across x=500: jump={jump} within={within_left}"
        );
    }

    /// A fault that leaves one side with NO orientations (both orientations sit on
    /// the plane / other side) must NOT panic. The orientation-starved block falls
    /// back to a default flat-lying orientation, so BOTH blocks mesh — no silent
    /// empty half. This is the harness's default fault (plane through the domain
    /// center, the seed orientations at x=500 landing on it).
    #[test]
    fn fault_with_orientation_starved_block_falls_back_flat() {
        // Seed-like data: contacts at z=600/z=400, both orientations at x=500.
        let mut sps = Vec::new();
        for &(x, y) in &[(100.0, 200.0), (500.0, 200.0), (900.0, 200.0), (500.0, 800.0)] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
            sps.push(SurfacePoint::new([x, y, 400.0], 1));
        }
        let oris = vec![
            Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0]),
            Orientation::new([500.0, 500.0, 400.0], [0.0, 0.0, 1.0]),
        ];
        let series = Series::new(sps, oris);
        // Vertical plane x=500, normal +x: orientations (x=500) go to the positive
        // side (block 1); the negative side (block 0) gets points but no orientation.
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());

        assert_eq!(model.nblocks(), 2);
        // BOTH blocks fit: the starved block 0 used a default flat orientation.
        assert!(model.block_lithology(0).is_some(), "starved block should fall back, not vanish");
        assert!(model.block_lithology(1).is_some(), "populated block should fit");
        // Both sides have a real (finite) field at the contact elevations.
        assert!(model.scalar_field([100.0, 500.0, 500.0]).is_finite());
        assert!(model.scalar_field([900.0, 500.0, 500.0]).is_finite());
    }

    /// A block with NO surface points at all (not just no orientation) is genuinely
    /// field-less — there is no contact to mesh. Sentinel `+∞` / unit 0 there.
    #[test]
    fn fault_block_with_no_points_is_field_less() {
        // All points on the positive side (x>500); the negative side is truly empty.
        let mut sps = Vec::new();
        for &(x, y) in &[(600.0, 200.0), (800.0, 200.0), (600.0, 800.0), (800.0, 800.0)] {
            sps.push(SurfacePoint::new([x, y, 600.0], 0));
        }
        let oris = vec![Orientation::new([700.0, 500.0, 600.0], [0.0, 0.0, 1.0])];
        let series = Series::new(sps, oris);
        let fault = Box::new(OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0])));
        let model = Model::fit_faulted(&series, fault, KernelOptions::default());
        assert!(model.block_lithology(0).is_none(), "empty side has no contact");
        assert!(model.block_lithology(1).is_some());
        assert!(model.scalar_field([100.0, 500.0, 500.0]).is_infinite());
    }

    fn sp(x: f64, y: f64, z: f64, id: u32) -> SurfacePoint {
        SurfacePoint::new([x, y, z], id)
    }
    fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn infer_orientation_three_points_best_fit_plane() {
        let pts = [sp(0.0, 0.0, 0.0, 0), sp(2.0, 0.0, 1.0, 0), sp(0.0, 2.0, 0.0, 0)];
        let o = infer_orientation(&pts);
        // Plane z = 0.5x has normal ∝ (-0.5, 0, 1) → (-0.4472,0,0.8944).
        assert!((o.pole[0] - (-0.4472)).abs() < 1e-3, "pole {:?}", o.pole);
        assert!(o.pole[1].abs() < 1e-3);
        assert!(o.pole[2] > 0.0, "pole points up");
        // Pole ⊥ the two in-plane edges.
        assert!(dot3(o.pole, [2.0, 0.0, 1.0]).abs() < 1e-9);
        assert!(dot3(o.pole, [0.0, 2.0, 0.0]).abs() < 1e-9);
    }

    /// >3 noisy roughly-flat points → ~+Z (the best-fit plane is horizontal).
    #[test]
    fn infer_orientation_many_flat_points() {
        let mut pts = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                pts.push(sp(i as f64 * 100.0, j as f64 * 100.0, 500.0, 0));
            }
        }
        let o = infer_orientation(&pts);
        assert!(o.pole[2] > 0.999, "near-flat → +Z, got {:?}", o.pole);
    }

    /// 2 points define only a LINE → the flattest plane through it: pole ⊥ the line
    /// and as vertical as possible. For a horizontal line, that is exactly +Z.
    #[test]
    fn infer_orientation_two_points_flattest() {
        // Horizontal line along a diagonal → flattest plane is horizontal → +Z.
        let o = infer_orientation(&[sp(0.0, 0.0, 500.0, 0), sp(100.0, 100.0, 500.0, 0)]);
        assert!(o.pole[2] > 0.999, "horizontal line → +Z, got {:?}", o.pole);

        // A line dipping in +x (from (0,0,0) to (1,0,1)): flattest plane normal is
        // (-1,0,1)/√2, and the pole must be ⊥ the line.
        let o2 = infer_orientation(&[sp(0.0, 0.0, 0.0, 0), sp(1.0, 0.0, 1.0, 0)]);
        assert!(dot3(o2.pole, [1.0, 0.0, 1.0]).abs() < 1e-9, "pole ⊥ line: {:?}", o2.pole);
        assert!((o2.pole[0] - (-0.7071)).abs() < 1e-3 && (o2.pole[2] - 0.7071).abs() < 1e-3);
    }

    /// ≥3 COLLINEAR points are still only a line (degenerate plane) → flattest plane,
    /// same as the 2-point case, NOT an arbitrary normal from a singular covariance.
    #[test]
    fn infer_orientation_collinear_points_fall_back_to_line() {
        // Collinear, horizontal along x → flattest is +Z.
        let pts = [
            sp(0.0, 0.0, 700.0, 0),
            sp(100.0, 0.0, 700.0, 0),
            sp(200.0, 0.0, 700.0, 0),
            sp(300.0, 0.0, 700.0, 0),
        ];
        let o = infer_orientation(&pts);
        assert!(o.pole[2] > 0.999, "collinear horizontal → +Z, got {:?}", o.pole);
    }

    /// A single point gives no geometry → flat (+Z), the bare default.
    #[test]
    fn infer_orientation_single_point_is_flat() {
        let o = infer_orientation(&[sp(123.0, 456.0, 789.0, 0)]);
        assert_eq!(o.pole, [0.0, 0.0, 1.0]);
        assert_eq!(o.xyz, [123.0, 456.0, 789.0]);
    }

    /// The inference picks the BEST-CONSTRAINED interface (most points), so a block
    /// mixing a well-sampled dipping contact with a single stray point of another
    /// interface uses the dipping contact's plane, not the mix.
    #[test]
    fn infer_orientation_uses_best_constrained_interface() {
        let pts = [
            // interface 0: a clear plane dipping in +x.
            sp(0.0, 0.0, 0.0, 0),
            sp(2.0, 0.0, 1.0, 0),
            sp(0.0, 2.0, 0.0, 0),
            sp(2.0, 2.0, 1.0, 0),
            // interface 1: a single stray point (should be ignored — fewer points).
            sp(50.0, 50.0, 999.0, 1),
        ];
        let o = infer_orientation(&pts);
        assert!(dot3(o.pole, [2.0, 0.0, 1.0]).abs() < 1e-9, "used iface 0 plane: {:?}", o.pole);
        assert!(o.pole[2] > 0.0);
    }

    /// `Model::fit` (no fault) is unchanged: one block, the whole field.
    #[test]
    fn unfaulted_fit_is_single_block() {
        let model = Model::fit(&faulted_offset_series(), KernelOptions::default());
        assert_eq!(model.nblocks(), 1);
        assert!(model.block_lithology(0).is_some());
    }

    /// Each block reproduces ITS OWN contact: the recovered isovalue on the left is
    /// the field at z=600, on the right at z=700 — and a point on each block's contact
    /// sits at its own field isovalue.
    #[test]
    fn each_block_reproduces_its_own_contact() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        assert_eq!(model.nblocks(), 2);

        // Block 0 = negative side (x<500), block 1 = positive (x>=500).
        let iso_left = model.block_lithology(0).unwrap().interfaces[0].isovalue;
        let iso_right = model.block_lithology(1).unwrap().interfaces[0].isovalue;

        // The contact sits at z≈600 on the left and z≈700 on the right: the field at
        // those points equals each block's recovered isovalue.
        let f_left_at_contact = model.scalar_field([300.0, 500.0, 600.0]);
        let f_right_at_contact = model.scalar_field([700.0, 500.0, 700.0]);
        assert!(
            (f_left_at_contact - iso_left).abs() < 1e-6,
            "left contact not at its isovalue: f={f_left_at_contact} iso={iso_left}"
        );
        assert!(
            (f_right_at_contact - iso_right).abs() < 1e-6,
            "right contact not at its isovalue: f={f_right_at_contact} iso={iso_right}"
        );
    }

    /// The field is DISCONTINUOUS across the fault: at the same (y,z), sampling just
    /// left vs just right of x=500 gives a jump (the two blocks' independent fields),
    /// far larger than the smooth variation within either block.
    #[test]
    fn field_is_discontinuous_across_the_fault() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());

        // Sample at a fixed z between the two contacts (z=650), y=500, straddling x=500.
        let z = 650.0;
        let just_left = model.scalar_field([499.0, 500.0, z]);
        let just_right = model.scalar_field([501.0, 500.0, z]);
        let jump = (just_right - just_left).abs();

        // Within-block variation over the same 2-unit x step, on the left block.
        let within_left = (model.scalar_field([497.0, 500.0, z]) - model.scalar_field([499.0, 500.0, z])).abs();

        assert!(
            jump > 20.0 * within_left.max(1e-9),
            "no discontinuity at the fault: jump={jump} within-block={within_left}"
        );
    }

    /// The field is CONTINUOUS within each block — no spurious jump away from the
    /// fault plane. Adjacent samples well inside block 0 differ only smoothly.
    #[test]
    fn field_is_continuous_within_a_block() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        let z = 650.0;
        let mut prev = model.scalar_field([100.0, 500.0, z]);
        // March across the left block (x: 100→480, all x<500) in small steps.
        for k in 1..=19 {
            let x = 100.0 + k as f64 * 20.0; // up to 480
            let cur = model.scalar_field([x, 500.0, z]);
            assert!(
                (cur - prev).abs() < 50.0,
                "spurious jump inside block 0 at x={x}: {prev} → {cur}"
            );
            prev = cur;
        }
    }

    /// Lithology is classified against the block's OWN isovalues, so a point below
    /// each block's offset contact lands in the same unit despite the throw — and the
    /// faulted model is a drop-in for `lithology_unit`.
    #[test]
    fn lithology_routes_per_block_across_the_offset() {
        let model = Model::fit_faulted(&faulted_offset_series(), fault_x500(), KernelOptions::default());
        // A point below the left contact (z=550 < 600) and below the right contact
        // (z=650 < 700) should be in the SAME unit (below the contact) on both sides.
        let unit_left = model.lithology_unit([300.0, 500.0, 550.0]);
        let unit_right = model.lithology_unit([700.0, 500.0, 650.0]);
        assert_eq!(
            unit_left, unit_right,
            "same structural position across the fault should be the same unit"
        );
        // And a point ABOVE each contact is a different unit from below it.
        let above_left = model.lithology_unit([300.0, 500.0, 650.0]); // above z=600
        assert_ne!(unit_left, above_left, "above vs below the contact must differ");
    }
}
