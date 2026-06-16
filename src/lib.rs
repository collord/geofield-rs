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
use data::Series;
use evaluate::Field;
use litho::Lithology;
use solve::{solve, Solution};

pub use stack::{Relation, Stack, StackedSeries};

/// A fitted single-series potential-field model: owns the assembled system, the
/// solved dual weights, and the recovered lithology isovalues. Self-contained so
/// it can be handed across an FFI / WASM boundary.
pub struct Model {
    system: System,
    solution: Solution,
    lithology: Lithology,
    /// Largest within-interface isovalue disagreement at fit time (diagnostic).
    pub isovalue_spread: f64,
    /// Solve residual ‖K w − b‖₂ (diagnostic).
    pub residual: f64,
}

impl Model {
    /// Assemble, solve, and recover isovalues for a series.
    pub fn fit(series: &Series, opts: KernelOptions) -> Self {
        let system = assemble(series, opts);
        let solution = solve(&system);
        let residual = solution.residual;
        let (lithology, isovalue_spread) = {
            let field = Field::new(&system.geom, &solution);
            Lithology::recover(series, &field)
        };
        Self {
            system,
            solution,
            lithology,
            isovalue_spread,
            residual,
        }
    }

    fn field(&self) -> Field<'_> {
        Field::new(&self.system.geom, &self.solution)
    }

    /// Scalar field `Z(x)` at a world query point.
    pub fn scalar_field(&self, xyz: [f64; 3]) -> f64 {
        self.field().scalar_field(xyz)
    }

    /// Analytic gradient `∇Z` at a world query point (world coordinates).
    pub fn scalar_field_gradient(&self, xyz: [f64; 3]) -> [f64; 3] {
        self.field().scalar_field_gradient(xyz)
    }

    /// Lithology unit index at a world query point (see [`litho::Lithology`]).
    pub fn lithology_unit(&self, xyz: [f64; 3]) -> usize {
        self.lithology.unit_at(&self.field(), xyz)
    }

    /// The recovered per-interface isovalues (sorted ascending).
    pub fn lithology(&self) -> &Lithology {
        &self.lithology
    }
}
