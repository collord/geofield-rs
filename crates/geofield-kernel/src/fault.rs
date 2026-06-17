//! Fault domains: partitioning the model volume into **blocks** so a contact field
//! solves independently in each block, making the field *discontinuous across the
//! fault* (GemPy's finite-fault behaviour — a fault offsets and truncates contacts
//! rather than smoothly bending them).
//!
//! This is the **field-assembly** layer, deliberately separate from the mesh-time
//! `octree::Boundary` in the `geofield` crate (which clips an already-solved field).
//! Block membership is a pure sign test: "which side of the fault is `p` on?" A
//! [`BlockIndex`] maps a point to a block id; [`crate::Model::fit_faulted`] solves
//! one cokriging field per block and dispatches eval to the block at the query
//! point. Surface points/orientations are assigned to a block by the SAME
//! `block_at` test on their own position — no per-point block field.
//!
//! Composition with the stack: a fault is internal to one [`crate::Model`], so any
//! [`crate::StackedSeries`] can be faulted or not, independently. The stack reads a
//! `Model` only through `scalar_field` / `lithology` (both already per-block after
//! `fit_faulted`), so faulting composes with erode/onlap masking for free.

/// Maps a world point to the block (fault-bounded domain) it lies in.
///
/// Block ids are `0..nblocks()`. The non-faulted case is [`SingleBlock`] (always
/// block 0). One planar fault is [`OneFault`] (2 blocks); multiple faults compose
/// as a sign-tuple ([`FaultStack`]).
pub trait BlockIndex {
    /// Which block contains `p`.
    fn block_at(&self, p: [f64; 3]) -> usize;
    /// Total number of blocks (so the fitter can allocate per-block buckets).
    fn nblocks(&self) -> usize;
    /// The plane half-spaces whose intersection IS block `block`'s region — used to
    /// CLIP a per-block contact mesh to that block (so a faulted contact renders as
    /// separate, unconnected sheets, not one stitched across the fault). Returns
    /// plain plane data (no mesh-layer dependency); the contourer turns each into a
    /// clip `Boundary`. An empty result means "no clip" (the whole domain — the
    /// non-faulted block 0).
    fn block_half_spaces(&self, block: usize) -> Vec<HalfSpace>;
}

/// A plane half-space, as plain geometry: the side of `point`+`normal` to KEEP.
/// `keep_positive` keeps `(p − point)·normal >= 0`, else the negative side. Returned
/// by [`BlockIndex::block_half_spaces`] and turned into a clip `Boundary` by the
/// contourer (`octree::PlaneHalfSpace`).
#[derive(Debug, Clone, Copy)]
pub struct HalfSpace {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub keep_positive: bool,
}

/// The trivial single-block index — everything is block 0. The non-faulted default,
/// so [`crate::Model::fit`] is exactly `fit_faulted` with this index.
pub struct SingleBlock;

impl BlockIndex for SingleBlock {
    fn block_at(&self, _p: [f64; 3]) -> usize {
        0
    }
    fn nblocks(&self) -> usize {
        1
    }
    fn block_half_spaces(&self, _block: usize) -> Vec<HalfSpace> {
        Vec::new() // whole domain, no clip
    }
}

/// An (infinite) planar fault through `point` with unit `normal`. A point is on the
/// **positive** side (block 1) when `(p − point) · normal >= 0`, else the negative
/// side (block 0). The plane offsets the contacts on either side independently.
///
/// This is the minimal fault geometry; a triangulated fault surface can implement
/// [`BlockIndex`] the same way (sign of a signed-distance / winding query) without
/// touching the fitter.
pub struct PlanarFault {
    point: [f64; 3],
    normal: [f64; 3],
}

impl PlanarFault {
    /// A planar fault through `point` with `normal` (need not be unit — only the
    /// sign of the dot product matters, but it is normalized for clarity/reuse).
    pub fn new(point: [f64; 3], normal: [f64; 3]) -> Self {
        let n = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        let normal = if n > 0.0 {
            [normal[0] / n, normal[1] / n, normal[2] / n]
        } else {
            [0.0, 0.0, 1.0]
        };
        Self { point, normal }
    }

    /// Signed distance-ish: `(p − point) · normal`. `>= 0` is the positive side.
    pub fn signed(&self, p: [f64; 3]) -> f64 {
        (p[0] - self.point[0]) * self.normal[0]
            + (p[1] - self.point[1]) * self.normal[1]
            + (p[2] - self.point[2]) * self.normal[2]
    }

    /// Is `p` on the positive side of the fault plane?
    pub fn positive_side(&self, p: [f64; 3]) -> bool {
        self.signed(p) >= 0.0
    }

    /// A point on the plane.
    pub fn point(&self) -> [f64; 3] {
        self.point
    }
    /// The plane's unit normal (points toward the positive side).
    pub fn normal(&self) -> [f64; 3] {
        self.normal
    }
}

/// One planar fault → two blocks: block 0 is the negative side, block 1 the positive.
pub struct OneFault {
    fault: PlanarFault,
}

impl OneFault {
    pub fn new(fault: PlanarFault) -> Self {
        Self { fault }
    }
}

impl BlockIndex for OneFault {
    fn block_at(&self, p: [f64; 3]) -> usize {
        self.fault.positive_side(p) as usize
    }
    fn nblocks(&self) -> usize {
        2
    }
    fn block_half_spaces(&self, block: usize) -> Vec<HalfSpace> {
        // Block 1 is the positive side, block 0 the negative.
        vec![HalfSpace {
            point: self.fault.point(),
            normal: self.fault.normal(),
            keep_positive: block == 1,
        }]
    }
}

/// `k` planar faults → up to `2^k` blocks, indexed by the sign tuple (bit `i` = the
/// positive side of fault `i`). Empty blocks (no data) are still allocated; the
/// fitter handles a block with no surface points by leaving it field-less (its
/// region simply has no contact — see [`crate::Model::fit_faulted`]).
pub struct FaultStack {
    faults: Vec<PlanarFault>,
}

impl FaultStack {
    pub fn new(faults: Vec<PlanarFault>) -> Self {
        assert!(
            faults.len() <= (usize::BITS as usize - 1),
            "FaultStack supports up to {} faults (sign-tuple fits in usize)",
            usize::BITS - 1
        );
        Self { faults }
    }
}

impl BlockIndex for FaultStack {
    fn block_at(&self, p: [f64; 3]) -> usize {
        let mut id = 0usize;
        for (i, f) in self.faults.iter().enumerate() {
            if f.positive_side(p) {
                id |= 1 << i;
            }
        }
        id
    }
    fn nblocks(&self) -> usize {
        1 << self.faults.len()
    }
    fn block_half_spaces(&self, block: usize) -> Vec<HalfSpace> {
        // One clip per fault; keep the side matching this block's sign-tuple bit.
        self.faults
            .iter()
            .enumerate()
            .map(|(i, f)| HalfSpace {
                point: f.point(),
                normal: f.normal(),
                keep_positive: (block >> i) & 1 == 1,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_block_is_always_zero() {
        let idx = SingleBlock;
        assert_eq!(idx.nblocks(), 1);
        assert_eq!(idx.block_at([1.0, 2.0, 3.0]), 0);
        assert_eq!(idx.block_at([-100.0, 0.0, 50.0]), 0);
    }

    #[test]
    fn planar_fault_splits_by_sign() {
        // Vertical fault plane x = 500, normal +x.
        let f = PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
        assert!(f.positive_side([600.0, 0.0, 0.0]), "x>500 is positive side");
        assert!(!f.positive_side([400.0, 0.0, 0.0]), "x<500 is negative side");
        // On the plane → positive side (>= 0).
        assert!(f.positive_side([500.0, 10.0, 20.0]));
    }

    #[test]
    fn one_fault_two_blocks() {
        let idx = OneFault::new(PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(idx.nblocks(), 2);
        assert_eq!(idx.block_at([400.0, 0.0, 0.0]), 0);
        assert_eq!(idx.block_at([600.0, 0.0, 0.0]), 1);
    }

    #[test]
    fn fault_stack_sign_tuple() {
        // Two orthogonal faults: x=500 (+x), y=500 (+y) → 4 quadrant blocks.
        let idx = FaultStack::new(vec![
            PlanarFault::new([500.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
            PlanarFault::new([0.0, 500.0, 0.0], [0.0, 1.0, 0.0]),
        ]);
        assert_eq!(idx.nblocks(), 4);
        assert_eq!(idx.block_at([400.0, 400.0, 0.0]), 0b00);
        assert_eq!(idx.block_at([600.0, 400.0, 0.0]), 0b01);
        assert_eq!(idx.block_at([400.0, 600.0, 0.0]), 0b10);
        assert_eq!(idx.block_at([600.0, 600.0, 0.0]), 0b11);
    }
}
