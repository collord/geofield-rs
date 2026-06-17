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

use std::sync::Once;
use wasm_bindgen::prelude::*;

use crate::assemble::KernelOptions;
use crate::data::{Orientation, Series, SurfacePoint};
use crate::fault::{BlockIndex, OneFault, PlanarFault};
use crate::mesh::{dual_contour, MeshGrid};
use crate::octree::{octree_dual_contour, OctreeConfig};
use crate::stack::{Relation, Stack, StackedSeries};
use crate::{MeshModel, Model};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

/// Install a panic hook (once) that forwards Rust panic messages to JS
/// `console.error`. Without this a Rust panic surfaces to JS only as the opaque
/// `RuntimeError: unreachable` trap with no message; with it the actual panic
/// location + message appears in the browser console. Called at the top of every
/// fallible WASM entry point, so any panic is diagnosable.
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            error(&format!("geofield-rs panic: {info}"));
        }));
    });
}

/// Build (validate + marshal + fit) a single-series [`Model`] from the flat typed
/// arrays the JS side passes. Shared by [`WasmModel::new`]/[`WasmModel::new_faulted`]
/// and [`WasmStack::add_series`]. If `fault` is `Some`, the model is fit per-block
/// (a faulted, laterally-discontinuous field); otherwise it is one smooth field.
/// Kernel options default (mirroring `OctreeConfig`/`KernelOptions::default`) where a
/// value is NaN / `uni_degree == 0`.
#[allow(clippy::too_many_arguments)]
fn fit_from_flat(
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
    fault: Option<Box<dyn BlockIndex>>,
) -> Result<Model, JsError> {
    install_panic_hook();
    if !sp_xyz.len().is_multiple_of(3)
        || !ori_xyz.len().is_multiple_of(3)
        || !ori_pole.len().is_multiple_of(3)
    {
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
    Ok(match fault {
        Some(index) => Model::fit_faulted(&series, index, opts),
        None => Model::fit(&series, opts),
    })
}

/// Build a single-planar-fault [`BlockIndex`] from a JS point+normal, or `None` if
/// the normal is degenerate (all ~zero / non-finite) — the "no fault" signal the JS
/// side uses to mean an un-faulted fit.
fn planar_fault_from_flat(point: &[f64], normal: &[f64]) -> Option<Box<dyn BlockIndex>> {
    if point.len() < 3 || normal.len() < 3 {
        return None;
    }
    let n2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
    if !n2.is_finite() || n2 < 1e-12 {
        return None;
    }
    let fault = PlanarFault::new(
        [point[0], point[1], point[2]],
        [normal[0], normal[1], normal[2]],
    );
    Some(Box::new(OneFault::new(fault)))
}

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
        let inner = fit_from_flat(
            sp_xyz,
            sp_interface,
            ori_xyz,
            ori_pole,
            range,
            c_o,
            i_res,
            gi_res,
            uni_degree,
            nugget,
            None,
        )?;
        Ok(WasmModel { inner })
    }

    /// Fit a model with a single **planar fault** offsetting the contacts. Same
    /// arrays/options as [`WasmModel::new`], plus the fault plane as
    /// `fault_point` (a 3-element `Float64Array`, a point on the plane) and
    /// `fault_normal` (a 3-element `Float64Array`). The field solves independently on
    /// each side of the plane, so contacts are laterally offset across it. A
    /// degenerate (≈zero) normal means "no fault" → identical to [`WasmModel::new`].
    #[allow(clippy::too_many_arguments)]
    pub fn new_faulted(
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
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<WasmModel, JsError> {
        let fault = planar_fault_from_flat(fault_point, fault_normal);
        let inner = fit_from_flat(
            sp_xyz,
            sp_interface,
            ori_xyz,
            ori_pole,
            range,
            c_o,
            i_res,
            gi_res,
            uni_degree,
            nugget,
            fault,
        )?;
        Ok(WasmModel { inner })
    }

    /// Number of fault-bounded blocks (1 if the model is not faulted).
    #[wasm_bindgen(getter)]
    pub fn block_count(&self) -> usize {
        self.inner.nblocks()
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

    /// Number of meshable interfaces (contacts) in fault block `block`. For a faulted
    /// model each block has its own contacts; for a non-faulted model block 0 holds
    /// them all. `0` if `block` is out of range or field-less.
    pub fn block_interface_count(&self, block: usize) -> usize {
        self.inner.block_interface_count(block)
    }

    /// Mesh interface `interface_idx` of fault block `block`, **clipped to that
    /// block's region** (the open-sheet view) — so a faulted contact renders as
    /// SEPARATE, unconnected sheets per block (Leapfrog-style), not one mesh stitched
    /// across the fault. Octree params mirror [`WasmModel::mesh_isosurface_octree`];
    /// `cap` is forced off (the per-block view is always the open sheet). Returns an
    /// EMPTY mesh for a field-less block / out-of-range interface.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_block_interface_octree(
        &self,
        block: usize,
        interface_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> WasmMesh {
        let cfg = OctreeConfig {
            max_depth: max_depth.max(1),
            min_depth: min_depth.min(max_depth),
            qef_threshold: if qef_threshold.is_finite() && qef_threshold > 0.0 {
                qef_threshold
            } else {
                OctreeConfig::default().qef_threshold
            },
            cap: false,
        };
        match self
            .inner
            .mesh_block_interface(block, interface_idx, [min_x, min_y, min_z], size, cfg)
        {
            Some(m) => WasmMesh {
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            },
            None => WasmMesh {
                positions: Vec::new(),
                normals: Vec::new(),
                indices: Vec::new(),
            },
        }
    }

    /// Number of lithology UNITS (volumes between contacts) in fault `block` — the
    /// closed-solid count for the faulted close-domain view. `0` if out of range.
    pub fn block_unit_count(&self, block: usize) -> usize {
        self.inner.block_unit_count(block)
    }

    /// A watertight CLOSED solid for one lithology UNIT of one fault block — the slab
    /// between consecutive contacts, capped by the box AND the fault plane (so the
    /// layered solids stitch to the fault). The faulted close-domain view loops
    /// `block × unit`. Octree params mirror [`WasmModel::mesh_block_interface_octree`];
    /// `cap` is forced true. EMPTY mesh for a field-less block / out-of-range unit.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_block_unit_solid_octree(
        &self,
        block: usize,
        unit_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> WasmMesh {
        let cfg = OctreeConfig {
            max_depth: max_depth.max(1),
            min_depth: min_depth.min(max_depth),
            qef_threshold: if qef_threshold.is_finite() && qef_threshold > 0.0 {
                qef_threshold
            } else {
                OctreeConfig::default().qef_threshold
            },
            cap: true,
        };
        match self
            .inner
            .mesh_block_unit_solid_welded(block, unit_idx, [min_x, min_y, min_z], size, cfg)
        {
            Some(m) => WasmMesh {
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            },
            None => WasmMesh {
                positions: Vec::new(),
                normals: Vec::new(),
                indices: Vec::new(),
            },
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

    /// Interface ids of fault `block` (parallel to `mesh_block_interface_octree`'s
    /// `interface_idx` for that block), as a `Float64Array`. Empty if the block is
    /// out of range or field-less. Lets the web name a block's contacts (e.g. the
    /// rock2 contact in each block).
    pub fn block_interface_ids(&self, block: usize) -> Vec<f64> {
        match self.inner.block_lithology(block) {
            Some(l) => l.interfaces.iter().map(|iv| iv.interface_id as f64).collect(),
            None => Vec::new(),
        }
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

/// Map a JS relation code to a [`Relation`]. `0=Erode 1=Onlap 2=Conformable
/// 3=Basement`; anything else falls back to `Erode` (GemPy's default).
fn relation_from_code(code: u32) -> Relation {
    match code {
        1 => Relation::Onlap,
        2 => Relation::Conformable,
        3 => Relation::Basement,
        _ => Relation::Erode,
    }
}

/// A multi-series stack exposed to JS (GemPy-style unconformities).
///
/// Build it by repeatedly calling [`WasmStack::add_series`] **youngest series
/// first**, each with its own surface points/orientations and a relation code, then
/// query per-point lithology ([`WasmStack::lithology_unit`]) or mesh a single
/// interface clipped to its series' visible region ([`WasmStack::mesh_interface_octree`]).
#[wasm_bindgen]
pub struct WasmStack {
    inner: Stack,
}

#[wasm_bindgen]
impl WasmStack {
    /// An empty stack. Add series youngest→oldest with [`WasmStack::add_series`].
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmStack {
        install_panic_hook();
        WasmStack {
            inner: Stack::new(Vec::new()),
        }
    }

    /// Fit and append one series (the next-oldest). Arrays and kernel options match
    /// [`WasmModel::new`]; `relation` is the code in [`relation_from_code`]
    /// (`0=Erode 1=Onlap 2=Conformable 3=Basement`). Appends in call order, so the
    /// FIRST `add_series` is the youngest.
    #[allow(clippy::too_many_arguments)]
    pub fn add_series(
        &mut self,
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
        relation: u32,
    ) -> Result<(), JsError> {
        let model = fit_from_flat(
            sp_xyz,
            sp_interface,
            ori_xyz,
            ori_pole,
            range,
            c_o,
            i_res,
            gi_res,
            uni_degree,
            nugget,
            None,
        )?;
        self.inner
            .series
            .push(StackedSeries::new(model, relation_from_code(relation)));
        Ok(())
    }

    /// Number of series in the stack.
    #[wasm_bindgen(getter)]
    pub fn series_count(&self) -> usize {
        self.inner.len()
    }

    /// Number of interfaces (meshable contacts) in series `series`. `0` if out of range.
    pub fn interface_count(&self, series: usize) -> usize {
        self.inner
            .series
            .get(series)
            .map(|s| s.model.lithology().interfaces.len())
            .unwrap_or(0)
    }

    /// The flattened global lithology label at a point, encoding the
    /// `(series, unit_within_series)` pair as `series * 1000 + unit`. JS can decode
    /// with `Math.floor(v/1000)` and `v % 1000`. (1000 units/series is far above any
    /// realistic interface count.)
    pub fn lithology_label(&self, x: f64, y: f64, z: f64) -> u32 {
        let (s, u) = self.inner.global_unit([x, y, z]);
        (s as u32) * 1000 + (u as u32)
    }

    /// Just the winning series index at a point (which series is visible there).
    pub fn series_at(&self, x: f64, y: f64, z: f64) -> usize {
        self.inner.global_unit([x, y, z]).0
    }

    /// Adaptive octree mesh of interface `interface_idx` of series `series`, clipped
    /// to that series' visible region (the domain box ∩ outside every younger
    /// erosion). Returns an EMPTY mesh if indices are out of range. Octree params
    /// mirror [`WasmModel::mesh_isosurface_octree`].
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_interface_octree(
        &self,
        series: usize,
        interface_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
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
            cap,
        };
        match self
            .inner
            .mesh_one(series, interface_idx, [min_x, min_y, min_z], size, cfg)
        {
            Some(m) => WasmMesh {
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            },
            None => WasmMesh {
                positions: Vec::new(),
                normals: Vec::new(),
                indices: Vec::new(),
            },
        }
    }
}

impl Default for WasmStack {
    fn default() -> Self {
        Self::new()
    }
}
