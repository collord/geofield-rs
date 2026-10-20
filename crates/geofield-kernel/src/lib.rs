//! geofield-kernel — the universal-cokriging scalar-field kernel.
//!
//! This crate OWNS the cokriging [`Model`]: it assembles and solves one potential
//! field per fault-bounded block, recovers per-block lithology, and evaluates the
//! scalar field, its gradient, and the lithology unit at any world point. A faulted
//! model dispatches each query to the block the point lies in, so the field is
//! discontinuous across the fault.
//!
//! Math source of truth: `KERNEL_MATH.md` in the repo root. Where this code and
//! that document disagree on a formula, sign, constant, or epsilon, the document
//! wins.
//!
//! The meshing / stacking / wasm layers live in the outer `geofield` crate, built on
//! top of this kernel's public API ([`Model::block_field`], [`Model::block_half_spaces`],
//! [`Model::block_lithology`], the [`ScalarField`] trait, …).
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
pub mod solve;

/// C-ABI exports for the standalone-wasm-module build (the two-wasm licensing split).
/// `geofield.wasm` (the BSD engine) imports these to query the field oracle across the
/// module boundary, so no kernel code is linked into the engine artifact.
#[cfg(feature = "wasm-abi")]
pub mod abi;

use assemble::{assemble, KernelOptions, System};
use data::{Orientation, Series, SurfacePoint};
use evaluate::Field;
use fault::{BlockIndex, SingleBlock};
use litho::Lithology;
use solve::{solve, Solution};

pub use fault::{FaultStack, OneFault, PlanarFault};
pub use kernel::KernelFunction;

/// The field-oracle contract — defined in the permissive `geofield-types` interface
/// crate (the boundary between this EUPL kernel and the BSD meshing engine), re-exported
/// here so kernel callers keep using `geofield_kernel::ScalarField`. Implemented below by
/// [`Model`] (per-block dispatch) and [`BlockField`] (one block's smooth field).
pub use geofield_types::ScalarField;

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
            // Optional residual-guided decimation BEFORE the (expensive, ~N³) solve. The
            // coreset honors the data to a tolerance; `None` keeps every point (gempy-exact).
            let block_points = std::mem::take(&mut sp_buckets[b]);
            let block_points = match opts.decimate {
                Some(cfg) => decimate_block(&block_points, &oris, opts, cfg),
                None => block_points,
            };
            let sub = Series::new(block_points, oris);
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

    /// Number of meshable interfaces in block `b` (0 if `b` is out of range or
    /// field-less).
    pub fn block_interface_count(&self, b: usize) -> usize {
        self.block_lithology(b).map_or(0, |l| l.interfaces.len())
    }

    /// Number of lithology UNITS in block `b` — the volumes between consecutive
    /// contacts, `interfaces + 1` (0 if `b` is out of range or field-less).
    pub fn block_unit_count(&self, b: usize) -> usize {
        match self.block_interface_count(b) {
            0 => 0, // a field-less block has no units
            n => n + 1,
        }
    }

    /// A view of block `b`'s smooth field as a [`ScalarField`] (no per-block dispatch),
    /// or `None` if `b` is field-less. Used by the meshing layer (outer crate).
    pub fn block_field(&self, b: usize) -> Option<BlockField<'_>> {
        self.blocks.get(b).and_then(|x| x.as_ref()).map(|block| BlockField { block })
    }

    /// The fault half-spaces bounding block `b` (plain plane geometry the meshing layer
    /// turns into clip boundaries).
    pub fn block_half_spaces(&self, b: usize) -> Vec<fault::HalfSpace> {
        self.index.block_half_spaces(b)
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

    /// Whether this fit is trustworthy enough to mesh, or should be flagged to the host.
    ///
    /// The ONLY sound fit-time signal is finiteness. Empirically (see the abi fit-failure notes),
    /// the nugget-regularized cokriging solve keeps `residual` and `isovalue_spread` finite and
    /// near-zero even for wildly under-constrained or contradictory input — it interpolates
    /// *through* the data, so the recovered isovalues always self-agree. A 2-point vein whose level
    /// set FOLDS is geologically wrong but produces a perfectly finite, self-consistent field; that
    /// pathology lives in the field GEOMETRY, not in any solve diagnostic, so no cheap scalar here
    /// detects it (detecting it needs field evaluation — out of scope for a fit-time gate).
    ///
    /// So this reports exactly what is detectable: a non-finite residual or interface spread, which
    /// only arises from a genuinely broken system (e.g. non-finite input that slipped past ingress).
    /// The host warns on `false` (review 4.2, narrowed to the reproducible case).
    pub fn fit_is_finite(&self) -> bool {
        self.residual.is_finite()
            && self.isovalue_spread.is_finite()
            && self
                .representative_lithology
                .interfaces
                .iter()
                .all(|i| i.isovalue.is_finite())
    }
}

/// `Model` as a [`ScalarField`] for the contourer — per-block dispatch (the field is
/// discontinuous across faults). Delegates to the inherent methods.
impl ScalarField for Model {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        Model::scalar_field(self, p)
    }
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        Model::scalar_field_gradient(self, p)
    }
}

/// A view of ONE fault block's field as a [`ScalarField`] — a single smooth cokriging
/// field with NO per-block dispatch, so contouring it produces a continuous contact
/// everywhere (the block's contact extrapolated across the whole domain). The per-block
/// mesh then drops the extrapolated part outside the block. This is what lets a faulted
/// contact mesh as clean separate sheets without the cross-fault discontinuity crumpling
/// the contour.
pub struct BlockField<'a> {
    block: &'a Block,
}

impl ScalarField for BlockField<'_> {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        self.block.field().scalar_field(p)
    }
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        self.block.field().scalar_field_gradient(p)
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
/// Iterative residual-guided decimation of ONE block's surface points (a kriging
/// coreset). Returns the kept subset. See [`assemble::DecimateConfig`] for the rationale:
/// seed a coarse grid-thinned subset, fit it, then re-add only the omitted points the
/// coarse field mis-predicts by more than `threshold_frac · isovalue_spread`, and repeat.
///
/// Orientations are passed through unchanged (they are few and each anchors a gradient).
/// Each interface's reference point (the first seen — the increment anchor) is always
/// kept. A degenerate fit during the loop falls back to keeping all points (safe).
fn decimate_block(
    points: &[SurfacePoint],
    orientations: &[Orientation],
    opts: KernelOptions,
    cfg: assemble::DecimateConfig,
) -> Vec<SurfacePoint> {
    use std::collections::{BTreeMap, BTreeSet};

    if points.len() <= 8 {
        return points.to_vec(); // nothing to gain
    }

    // Normalized coordinates for gridding (same transform the assembly uses).
    let transform = data::Transform::from_input_points(points, orientations);
    let cell = cfg.seed_cell.max(1e-6);
    let bucket = |p: [f64; 3]| -> (i64, i64, i64) {
        let n = transform.apply(p);
        (
            (n[0] / cell).floor() as i64,
            (n[1] / cell).floor() as i64,
            (n[2] / cell).floor() as i64,
        )
    };

    // Reference point index per interface — always kept (the increment anchor).
    let mut ref_idx: BTreeMap<u32, usize> = BTreeMap::new();
    for (i, sp) in points.iter().enumerate() {
        ref_idx.entry(sp.interface_id).or_insert(i);
    }

    // Seed: one point per (interface, grid cell) plus every interface reference.
    let mut kept: BTreeSet<usize> = ref_idx.values().copied().collect();
    let mut seen_cells: BTreeSet<(u32, i64, i64, i64)> = BTreeSet::new();
    for (i, sp) in points.iter().enumerate() {
        let (bx, by, bz) = bucket(sp.xyz);
        if seen_cells.insert((sp.interface_id, bx, by, bz)) {
            kept.insert(i);
        }
    }

    // Refit-and-re-add loop.
    for _ in 0..cfg.max_iters.max(1) {
        if cfg.point_budget > 0 && kept.len() >= cfg.point_budget {
            break;
        }
        let kept_pts: Vec<SurfacePoint> = kept.iter().map(|&i| points[i]).collect();
        // Need ≥1 point per interface that has any; Series picks references from these.
        let series = Series::new(kept_pts, orientations.to_vec());
        let system = assemble(&series, opts);
        let solution = solve(&system);
        if !solution.residual.is_finite() {
            return points.to_vec(); // degenerate seed → keep all (safe)
        }
        let field = Field::new(&system.geom, &solution);
        let (lithology, _spread) = Lithology::recover(&series, &field);
        let iso_by_iface: BTreeMap<u32, f64> = lithology
            .interfaces
            .iter()
            .map(|iv| (iv.interface_id, iv.isovalue))
            .collect();
        // Threshold scale: the field's natural dynamic range — the spread of recovered
        // isovalues ACROSS interfaces (how far apart the contacts sit in field value).
        // This is STABLE as the seed improves (unlike per-interface kept-point spread,
        // which collapses to ~0 on a good seed and then re-admits everything). A point
        // mis-predicted by < frac·range carries no contact-scale information.
        let (mut gmin, mut gmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for &iso in iso_by_iface.values() {
            gmin = gmin.min(iso);
            gmax = gmax.max(iso);
        }
        // Fall back to the magnitude of a representative isovalue if there is a single
        // interface (range 0): use |iso| so a one-contact field still has a real scale.
        let scale = if gmax > gmin {
            gmax - gmin
        } else {
            iso_by_iface.values().next().map_or(1.0, |v| v.abs().max(1e-3))
        };
        let threshold = cfg.threshold_frac * scale;

        // Score every OMITTED point; collect those whose field misses its interface's
        // isovalue by more than the threshold.
        let mut worst: Vec<(f64, usize)> = Vec::new();
        for (i, sp) in points.iter().enumerate() {
            if kept.contains(&i) {
                continue;
            }
            let Some(&iso) = iso_by_iface.get(&sp.interface_id) else {
                worst.push((f64::INFINITY, i)); // unseen interface → definitely informative
                continue;
            };
            let miss = (field.isovalue_at(sp.xyz) - iso).abs();
            if miss > threshold {
                worst.push((miss, i));
            }
        }
        if worst.is_empty() {
            break; // the kept subset explains all points to tolerance
        }
        // Re-add worst-first, but GROW GRADUALLY: cap each iteration's additions to ~50%
        // of the current kept count (min 32). This converges to a MINIMAL sufficient set
        // — adding the worst misses first means the few informative points come in early,
        // and the loop stops as soon as the rest fall under threshold, instead of
        // overshooting back to the full set in one jump.
        worst.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let add_cap = (kept.len() / 2).max(32);
        let mut added = 0;
        for (_, i) in worst {
            if added >= add_cap {
                break;
            }
            if cfg.point_budget > 0 && kept.len() >= cfg.point_budget {
                break;
            }
            kept.insert(i);
            added += 1;
        }
    }

    kept.iter().map(|&i| points[i]).collect()
}

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

/// Unit vector, or the zero vector for a near-zero input. (Kernel-local copy of the
/// meshing layer's `normalize` so `infer_orientation` / `flattest_plane_normal` have
/// no dependency on the outer crate.)
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-30 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / n, v[1] / n, v[2] / n]
    }
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
    // `total_cmp` is a total order over all f64 (NaN sorts last) so this never panics on a
    // non-finite eigenvalue — which a NaN input coordinate would produce. A NaN then flows on to
    // the (non-finite) fitted field, which `Model::fit_is_finite` reports loudly, instead of
    // trapping the wasm kernel here (code_review §4.6 residual).
    order.sort_by(|&i, &j| evals[i].total_cmp(&evals[j]));
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
mod tests {
    use super::*;
    use crate::data::{Orientation, SurfacePoint};
    use crate::fault::{OneFault, PlanarFault};

    #[test]
    fn well_posed_and_underconstrained_fits_are_finite() {
        // The nugget-regularized solve keeps a fit finite even for degenerate/contradictory input
        // (it interpolates THROUGH the data). `fit_is_finite` therefore reports true here — the
        // geologic wrongness of an under-constrained fit lives in field geometry, not a diagnostic
        // scalar (see abi::store_fitted). This test pins that contract so a future regression that
        // makes a normal fit report non-finite is caught.
        let good = Model::fit(
            &Series::new(
                vec![
                    SurfacePoint::new([0.0, 0.0, 100.0], 0),
                    SurfacePoint::new([100.0, 0.0, 100.0], 0),
                    SurfacePoint::new([0.0, 100.0, 100.0], 0),
                ],
                vec![Orientation::new([50.0, 50.0, 100.0], [0.0, 0.0, 1.0])],
            ),
            KernelOptions::default(),
        );
        assert!(good.fit_is_finite(), "a well-posed fit is finite");

        let two_point = Model::fit(
            &Series::new(
                vec![
                    SurfacePoint::new([0.0, 0.0, 0.0], 0),
                    SurfacePoint::new([100.0, 0.0, 0.0], 0),
                ],
                vec![Orientation::new([50.0, 0.0, 0.0], [0.0, 0.0, 1.0])],
            ),
            KernelOptions::default(),
        );
        // Documents the empirical finding: this DOES fit finitely (no false alarm), even though the
        // resulting vein wall is under-constrained.
        assert!(two_point.fit_is_finite(), "regularization keeps a 2-point fit finite");
    }

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

    /// A heavily OVERSAMPLED flat contact: 40×40 points on z=500 (1600 points) for a
    /// surface a handful of points would define. Decimation must (a) keep far fewer
    /// points and (b) leave the field essentially unchanged versus the full fit.
    fn oversampled_flat() -> Series {
        let mut sps = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                let x = i as f64 * 25.0 + 10.0;
                let y = j as f64 * 25.0 + 10.0;
                sps.push(SurfacePoint::new([x, y, 500.0], 0));
            }
        }
        let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
        Series::new(sps, oris)
    }

    #[test]
    fn decimation_thins_an_oversampled_contact_but_preserves_the_field() {
        let series = oversampled_flat();
        let full = Model::fit(&series, KernelOptions::default());

        let mut opts = KernelOptions::default();
        opts.decimate = Some(assemble::DecimateConfig {
            seed_cell: 0.08,
            threshold_frac: 0.2,
            point_budget: 0,
            max_iters: 5,
        });
        let dec = Model::fit(&series, opts);

        // The decimated fit must agree with the full fit across the domain (it honors the
        // data to a tolerance — a flat contact is the easy case, so agreement is tight).
        let mut max_err = 0.0_f64;
        for &x in &[100.0, 300.0, 500.0, 700.0, 900.0] {
            for &y in &[100.0, 500.0, 900.0] {
                for &z in &[300.0, 500.0, 700.0] {
                    let e = (full.scalar_field([x, y, z]) - dec.scalar_field([x, y, z])).abs();
                    max_err = max_err.max(e);
                }
            }
        }
        // Field scale ~O(1) after rescaling; agreement well under 5% of that.
        assert!(max_err < 0.05, "decimated field drifted from full: max_err={max_err}");

        // Both classify a below-contact point the same way (the geology is preserved).
        assert_eq!(
            full.lithology_unit([500.0, 500.0, 400.0]),
            dec.lithology_unit([500.0, 500.0, 400.0]),
            "decimation changed the lithology below the contact"
        );
    }

    #[test]
    fn decimation_none_is_bit_identical_to_no_decimation() {
        // The default (None) path must be byte-for-byte the existing behavior — this is
        // what keeps the gempy validation suite exact.
        let series = oversampled_flat();
        let a = Model::fit(&series, KernelOptions::default());
        let mut opts = KernelOptions::default();
        opts.decimate = None;
        let b = Model::fit(&series, opts);
        for &z in &[400.0, 500.0, 600.0] {
            assert_eq!(
                a.scalar_field([500.0, 500.0, z]),
                b.scalar_field([500.0, 500.0, z]),
                "decimate=None must be identical to the default"
            );
        }
    }
}
