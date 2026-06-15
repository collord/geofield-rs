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
use crate::mesh::{dual_contour, MeshGrid};
use crate::octree::{octree_dual_contour, OctreeConfig};
use crate::Model;

/// A dual-contoured triangle mesh handed to JS: flat position + index buffers.
#[wasm_bindgen]
pub struct WasmMesh {
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
}

#[wasm_bindgen]
impl WasmMesh {
    /// `3 * vertexCount` world-space positions (x,y,z, …) as a `Float32Array`.
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    /// `3 * vertexCount` per-vertex unit normals (from the analytic field gradient)
    /// as a `Float32Array`, parallel to positions. Empty if the contourer didn't
    /// produce them. Prefer these over face-averaged normals for smooth shading.
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    /// `3 * triangleCount` triangle indices as a `Uint32Array`.
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }
    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

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

    /// Dual-contour the isosurface `scalar_field == isovalue` over the axis-
    /// aligned box `[min, max]` at `res` cells per axis. Returns a triangle mesh
    /// (positions + indices) ready for a three.js BufferGeometry.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_isosurface(
        &self,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        max_x: f64,
        max_y: f64,
        max_z: f64,
        res: usize,
        isovalue: f64,
    ) -> WasmMesh {
        let grid = MeshGrid::new([min_x, min_y, min_z], [max_x, max_y, max_z], res.max(1));
        let m = dual_contour(&self.inner, &grid, isovalue);
        WasmMesh {
            positions: m.positions,
            normals: m.normals,
            indices: m.indices,
        }
    }

    /// Adaptive **octree** dual contouring of `scalar_field == isovalue` over the
    /// cube `[min, min+size]` (a cube is required for a clean octree — pass the
    /// extent's bounding cube). Refines toward surface detail (QEF residual >
    /// `qef_threshold`) between `min_depth` and `max_depth`, 2:1-balances, and
    /// contours crack-free. Coarse where the surface is flat, fine where it curves.
    ///
    /// `cap = false` returns the bounded contact surface (an open sheet clipped to
    /// the domain). `cap = true` additionally contours the domain-boundary cap,
    /// returning a watertight closed solid (sheet + walls/bottom).
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_isosurface_octree(
        &self,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        isovalue: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
        cap: bool,
    ) -> WasmMesh {
        let cfg = OctreeConfig {
            max_depth: max_depth.max(1),
            min_depth: min_depth.min(max_depth),
            qef_threshold: if qef_threshold.is_finite() && qef_threshold > 0.0 {
                qef_threshold
            } else {
                OctreeConfig::default().qef_threshold
            },
            // `false` → bounded open sheet (just the contact surface); `true` →
            // watertight closed solid (sheet + domain-boundary cap).
            cap,
        };
        let m = octree_dual_contour(&self.inner, [min_x, min_y, min_z], size, isovalue, cfg);
        WasmMesh {
            positions: m.positions,
            normals: m.normals,
            indices: m.indices,
        }
    }

    /// Recovered per-interface isovalues, ascending by isovalue, as a
    /// `Float64Array`. These are the scalar-field levels at which each interface's
    /// contact lives — mesh each one to render the actual geological surfaces (as
    /// opposed to an arbitrary level set like 0).
    pub fn interface_isovalues(&self) -> Vec<f64> {
        self.inner
            .lithology()
            .interfaces
            .iter()
            .map(|iv| iv.isovalue)
            .collect()
    }

    /// Interface ids matching `interface_isovalues()` order, as a `Float64Array`
    /// (ids are small integers; f64 carries them losslessly).
    pub fn interface_ids(&self) -> Vec<f64> {
        self.inner
            .lithology()
            .interfaces
            .iter()
            .map(|iv| iv.interface_id as f64)
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
