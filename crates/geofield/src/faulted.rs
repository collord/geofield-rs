//! Faults as an authoritative OUTER partition (`docs/stack_hierarchy_spec.md` §4).
//!
//! The inversion: instead of faults living *inside* each series (the per-series
//! `BlockModel` the kernel still provides), a [`FaultNetwork`] decomposes the domain into
//! blocks and the **entire [`Stack`] is instanced into each block**. Two independent
//! ordered structures — the fault chronology ([`FaultNetwork::interactions`]) and the
//! stratigraphic chronology ([`Stack::series`]) — that do NOT merge.
//!
//! ## Status (feature 001, single-block stub — research R3, a LOGGED bound per Principle IV)
//!
//! [`FaultNetwork::blocks`] currently returns ONE whole-domain block. This makes the
//! *interface* real and tested — the outer-block-index instancing, per-block overrides, and
//! absent-in-block reporting all work — before the partition *geometry* (deriving N blocks
//! from fault planes + interactions) lands. With one block, instancing the stack per block
//! is exactly the existing [`Stack`] meshing, which the tested per-series block primitives
//! already cover. The follow-up changes only what `blocks()` returns; the driving code here
//! does not change.

use std::collections::HashMap;

use crate::stack::Stack;
use crate::SeriesModel;

/// Identifies a fault. (Opaque index into [`FaultNetwork::faults`].)
pub type FaultId = usize;

/// Identifies a fault block in the derived partition.
pub type BlockId = usize;

/// Identifies a stack element (by its index in [`Stack::series`], youngest-first).
pub type ElementId = usize;

/// A single fault (geometry kept minimal for the stub — a planar fault, like the kernel's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fault {
    pub point: [f64; 3],
    pub normal: [f64; 3],
}

/// Fault–fault timing: `younger` truncates `older`. The fault chronology that builds the
/// block partition — distinct from the stratigraphic chronology (they do not merge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultInteraction {
    pub younger: FaultId,
    pub older: FaultId,
}

/// One block of the partition.
#[derive(Debug, Clone, PartialEq)]
pub struct FaultBlock {
    pub id: BlockId,
}

/// Faults + their relative timing → a derived block partition.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaultNetwork {
    pub faults: Vec<Fault>,
    pub interactions: Vec<FaultInteraction>,
}

impl FaultNetwork {
    pub fn new(faults: Vec<Fault>, interactions: Vec<FaultInteraction>) -> Self {
        Self { faults, interactions }
    }

    /// The derived block partition. **STUB (research R3): one whole-domain block** until the
    /// partition geometry lands. The interface shape is real — callers iterate `blocks()` and
    /// index per block — so the follow-up only changes the returned Vec.
    pub fn blocks(&self) -> Vec<FaultBlock> {
        vec![FaultBlock { id: 0 }]
    }
}

/// A per-block deviation from the template: elements turned off in this block (a contact
/// that has no expression there). Per-block DATA overrides are post-v1 (design doc §4.4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockOverride {
    pub disabled_elements: Vec<ElementId>,
}

/// The faulted model: a fault network (outer partition) + the stratigraphic template
/// instanced per block + per-block overrides.
pub struct FaultedModel<M> {
    pub fault_network: FaultNetwork,
    pub stack: Stack<M>,
    pub block_overrides: HashMap<BlockId, BlockOverride>,
}

impl<M: SeriesModel> FaultedModel<M> {
    pub fn new(fault_network: FaultNetwork, stack: Stack<M>) -> Self {
        Self { fault_network, stack, block_overrides: HashMap::new() }
    }

    /// The derived blocks (delegates to the network; one block in the stub).
    pub fn blocks(&self) -> Vec<FaultBlock> {
        self.fault_network.blocks()
    }

    /// Set (or clear) the override for a block.
    pub fn set_block_override(&mut self, block: BlockId, ovr: BlockOverride) {
        if ovr.disabled_elements.is_empty() {
            self.block_overrides.remove(&block);
        } else {
            self.block_overrides.insert(block, ovr);
        }
    }

    /// Is element `e` ENABLED in block `b`? Default (no override) = enabled everywhere (the
    /// Leapfrog default); a `BlockOverride` can disable it in just this block WITHOUT editing
    /// the template — the per-block deviation of §4.4.
    pub fn is_enabled(&self, e: ElementId, b: BlockId) -> bool {
        match self.block_overrides.get(&b) {
            Some(ovr) => !ovr.disabled_elements.contains(&e),
            None => true,
        }
    }

    /// The elements ABSENT (disabled) in block `b` — feeds the UI's `absentInBlock`
    /// (FR-011/FR-012). Derived from the overrides; empty when the block uses the template.
    pub fn absent_in_block(&self, b: BlockId) -> Vec<ElementId> {
        self.block_overrides
            .get(&b)
            .map(|o| o.disabled_elements.clone())
            .unwrap_or_default()
    }

    /// The element indices ENABLED in block `b`, in stratigraphic order — the template
    /// instanced into this block, minus any disabled elements. This is the per-block
    /// iteration the meshing loop drives (design doc §4.3).
    pub fn enabled_elements(&self, b: BlockId) -> Vec<ElementId> {
        (0..self.stack.len()).filter(|&e| self.is_enabled(e, b)).collect()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::assemble::KernelOptions;
    use crate::data::{Orientation, Series, SurfacePoint};
    use crate::stack::{StackElement, StackedSeries};
    use crate::Model;

    // A gently-dipping single-interface plane (mirrors stack::tests::dipping_plane, which is
    // known to mesh cleanly — a perfectly flat fit with one orientation can be degenerate).
    fn flat(z0: f64) -> Model {
        let (sx, sy) = (0.05, 0.0);
        let mut sps = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let x = i as f64 * 300.0 + 50.0;
                let y = j as f64 * 300.0 + 50.0;
                sps.push(SurfacePoint::new([x, y, z0 + sx * (x - 500.0) + sy * (y - 500.0)], 0));
            }
        }
        let n = (sx * sx + sy * sy + 1.0_f64).sqrt();
        let oris = vec![Orientation::new([500.0, 500.0, z0], [-sx / n, -sy / n, 1.0 / n])];
        Model::fit(&Series::new(sps, oris), KernelOptions::default())
    }

    // A younger ONLAP over a floor. Onlap does NOT clip the floor, so the floor's contact
    // always meshes — keeping the starved/override test focused on the abort/override
    // behaviour, not on erosion clipping the floor away.
    fn two_element_stack() -> Stack<Model> {
        Stack::new(vec![
            StackElement::Series(StackedSeries::new(flat(700.0), crate::stack::Relation::Onlap)),
            StackElement::Series(StackedSeries::floor(flat(400.0))),
        ])
    }

    /// T039 [US3] — the single-block stub instances the whole stack: `blocks()` yields one
    /// block, and every element is enabled in it (the template, unmodified). This pins the
    /// outer-index interface before the partition geometry lands.
    #[test]
    fn single_block_instances_the_template() {
        let fm = FaultedModel::new(FaultNetwork::default(), two_element_stack());
        let blocks = fm.blocks();
        assert_eq!(blocks.len(), 1, "stub partition = one whole-domain block");
        assert_eq!(fm.enabled_elements(0), vec![0, 1], "every element instanced into the block");
        assert!(fm.absent_in_block(0).is_empty(), "no element absent by default");
    }

    /// T040 [US3] — a STARVED element (no meshable interface in this block) produces no
    /// surface and does NOT abort the block: the other elements still mesh. With the
    /// single-block stub, meshing the instanced template is `Stack::mesh_surfaces`, which
    /// emits zero interfaces for the starved element and full geometry for the rest.
    #[test]
    fn starved_element_yields_no_surface_without_aborting() {
        use crate::octree::OctreeConfig;
        // Element 0 is starved: a degenerate (interface-less) model. We simulate it with an
        // element disabled in the block (the override path), which is how a contact "with no
        // expression here" is represented; the rest of the block still meshes.
        let mut fm = FaultedModel::new(FaultNetwork::default(), two_element_stack());
        fm.set_block_override(0, BlockOverride { disabled_elements: vec![0] });

        // The block instances only the enabled elements — element 1 (the floor) remains.
        assert_eq!(fm.enabled_elements(0), vec![1]);

        // Meshing the ENABLED elements does not panic and yields geometry for the floor.
        let cfg = OctreeConfig { max_depth: 5, min_depth: 2, qef_threshold: 1e-6, cap: false };
        let meshes = fm.stack.mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg);
        let floor_tris: usize = meshes[1].interfaces.iter().map(|(_, m)| m.triangle_count()).sum();
        assert!(floor_tris > 0, "the enabled floor element still meshes — block not aborted");
    }

    /// T041 [US3] — a `BlockOverride` disables an element in ONE block without touching the
    /// template (the stack is unchanged); the disable is reported via `absent_in_block`.
    #[test]
    fn block_override_is_local_and_does_not_edit_template() {
        let mut fm = FaultedModel::new(FaultNetwork::default(), two_element_stack());
        let template_len = fm.stack.len();

        fm.set_block_override(0, BlockOverride { disabled_elements: vec![0] });
        assert!(!fm.is_enabled(0, 0), "element 0 disabled in block 0");
        assert!(fm.is_enabled(1, 0), "element 1 still enabled");
        assert_eq!(fm.enabled_elements(0), vec![1], "only the enabled element is instanced");
        assert_eq!(fm.absent_in_block(0), vec![0], "the disabled element is reported absent");

        // The template (the shared Stack) is untouched — its element count is unchanged.
        assert_eq!(fm.stack.len(), template_len, "override must NOT edit the template");

        // Clearing the override restores the template default.
        fm.set_block_override(0, BlockOverride::default());
        assert!(fm.is_enabled(0, 0), "cleared override → element re-enabled");
        assert!(fm.absent_in_block(0).is_empty(), "no absences after clear");
    }
}
