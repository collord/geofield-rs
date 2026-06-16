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
            // makes one side of a fault silently vanish — fall back to a default
            // FLAT-LYING orientation (pole +Z, horizontal contact) at the block's
            // surface-point centroid. This is the most common geological prior, is
            // invisible when real orientations exist, and keeps both fault blocks
            // meshable. (A fault plane that strands all orientations on one side is
            // exactly the harness's default-fault case.)
            let oris = std::mem::take(&mut ori_buckets[b]);
            let oris = if oris.is_empty() {
                vec![default_flat_orientation(&sp_buckets[b])]
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

/// A default FLAT-LYING orientation (pole +Z) at the centroid of `points` — the
/// fallback gradient anchor for a fault block that has surface points but no
/// orientation of its own. Flat-lying is the standard geological default; it keeps
/// the block's field well-posed without inventing a dip direction.
fn default_flat_orientation(points: &[SurfacePoint]) -> Orientation {
    let n = points.len().max(1) as f64;
    let mut c = [0.0; 3];
    for p in points {
        c[0] += p.xyz[0];
        c[1] += p.xyz[1];
        c[2] += p.xyz[2];
    }
    Orientation::new([c[0] / n, c[1] / n, c[2] / n], [0.0, 0.0, 1.0])
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
