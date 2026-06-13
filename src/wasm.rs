//! WASM bindings (feature = "wasm"). Typed-array in/out.
//!
//! Inputs are passed as flat `Float64Array`s to avoid per-point JS object
//! marshalling:
//!   - `sp_xyz`:        `n_sp * 3` surface-point coordinates (x,y,z, …)
//!   - `sp_interface`:  `n_sp` interface ids (as f64, cast to u32)
//!   - `ori_xyz`:       `n_ori * 3` orientation coordinates
//!   - `ori_pole`:      `n_ori * 3` orientation poles
//!
//! Build: `wasm-pack build --target web -- --features wasm`.

use wasm_bindgen::prelude::*;

use crate::assemble::KernelOptions;
use crate::data::{Orientation, Series, SurfacePoint};
use crate::Model;

/// A fitted model exposed to JS.
#[wasm_bindgen]
pub struct WasmModel {
    inner: Model,
}

#[wasm_bindgen]
impl WasmModel {
    /// Fit a model from flat typed arrays. `range`, `c_o`, `i_res`, `gi_res`,
    /// `uni_degree`, and `nugget` are the kernel options (pass gempy's live
    /// values; defaults mirror the octree defaults if you pass NaN).
    #[wasm_bindgen(constructor)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sp_xyz: &[f64],
        sp_interface: &[f64],
        ori_xyz: &[f64],
        ori_pole: &[f64],
        range: f64,
        c_o: f64,
        i_res: f64,
        gi_res: f64,
        uni_degree: u32,
        nugget: f64,
    ) -> Result<WasmModel, JsError> {
        if sp_xyz.len() % 3 != 0 || ori_xyz.len() % 3 != 0 || ori_pole.len() % 3 != 0 {
            return Err(JsError::new("xyz/pole arrays must have length divisible by 3"));
        }
        if sp_xyz.len() / 3 != sp_interface.len() {
            return Err(JsError::new("sp_interface length must equal sp count"));
        }
        if ori_xyz.len() != ori_pole.len() {
            return Err(JsError::new("ori_xyz and ori_pole must have equal length"));
        }

        let surface_points: Vec<SurfacePoint> = sp_xyz
            .chunks_exact(3)
            .zip(sp_interface.iter())
            .map(|(c, &id)| SurfacePoint::new([c[0], c[1], c[2]], id as u32))
            .collect();
        let orientations: Vec<Orientation> = ori_xyz
            .chunks_exact(3)
            .zip(ori_pole.chunks_exact(3))
            .map(|(c, p)| Orientation::new([c[0], c[1], c[2]], [p[0], p[1], p[2]]))
            .collect();

        let d = KernelOptions::default();
        let or = |v: f64, dflt: f64| if v.is_nan() { dflt } else { v };
        let opts = KernelOptions {
            range: or(range, d.range),
            c_o: or(c_o, d.c_o),
            i_res: or(i_res, d.i_res),
            gi_res: or(gi_res, d.gi_res),
            uni_degree: if uni_degree == 0 { d.uni_degree } else { uni_degree },
            grad_nugget: or(nugget, d.grad_nugget),
            sp_nugget: or(nugget, d.sp_nugget),
        };

        let series = Series::new(surface_points, orientations);
        Ok(WasmModel {
            inner: Model::fit(&series, opts),
        })
    }

    /// Scalar field at a single world point.
    pub fn scalar_field(&self, x: f64, y: f64, z: f64) -> f64 {
        self.inner.scalar_field([x, y, z])
    }

    /// Gradient at a single world point, returned as a length-3 `Float64Array`.
    pub fn scalar_field_gradient(&self, x: f64, y: f64, z: f64) -> Vec<f64> {
        self.inner.scalar_field_gradient([x, y, z]).to_vec()
    }

    /// Lithology unit index at a single world point.
    pub fn lithology_unit(&self, x: f64, y: f64, z: f64) -> usize {
        self.inner.lithology_unit([x, y, z])
    }

    /// Evaluate the field over a flat `n*3` grid; returns an `n`-long
    /// `Float64Array`.
    pub fn scalar_field_grid(&self, grid_xyz: &[f64]) -> Vec<f64> {
        grid_xyz
            .chunks_exact(3)
            .map(|c| self.inner.scalar_field([c[0], c[1], c[2]]))
            .collect()
    }

    /// Solve residual ‖K w − b‖₂ (diagnostic).
    pub fn residual(&self) -> f64 {
        self.inner.residual
    }

    /// Largest within-interface isovalue spread at fit time (diagnostic).
    pub fn isovalue_spread(&self) -> f64 {
        self.inner.isovalue_spread
    }
}
