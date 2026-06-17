//! WASM bindings (feature = "wasm"). Typed-array in/out.
//!
//! Inputs are passed as flat `Float64Array`s to avoid per-point JS object
//! marshalling:
//!   - `sp_xyz`:        `n_sp * 3` surface-point coordinates (x,y,z, …)
//!   - `sp_interface`:  `n_sp` interface ids (as f64, cast to u32)
//!   - `ori_xyz`:       `n_ori * 3` orientation coordinates
//!   - `ori_pole`:      `n_ori * 3` orientation poles
//!
//! ## Two-wasm split
//!
//! This (`geofield`) is the BSD engine module. It does NOT link the cokriging kernel;
//! it fits/queries models by marshalling inputs into the separately-loaded EUPL kernel
//! module (`geofield_kernel.wasm`) and holding a [`RemoteModel`] handle. The octree/mesh
//! code runs HERE (BSD), querying the kernel's field oracle pointwise across the module
//! boundary (see [`crate::remote`]).
//!
//! Build: `wasm-pack build --target web -- --features wasm` (the host wires the kernel
//! module's exports as this module's `"kernel"` imports).

use std::sync::Once;
use wasm_bindgen::prelude::*;

use crate::octree::{octree_dual_contour, OctreeConfig};
use crate::remote::{self, RemoteModel};
use crate::stack::{Relation, Stack, StackedSeries};
use crate::{BlockModel, MeshModel};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

/// Install a panic hook (once) that forwards Rust panic messages to JS
/// `console.error`. Without this a Rust panic surfaces to JS only as the opaque
/// `RuntimeError: unreachable` trap with no message; with it the actual panic
/// location + message appears in the browser console.
fn install_panic_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            error(&format!("geofield panic: {info}"));
        }));
    });
}

/// Marshal flat input arrays + kernel options into the kernel ABI's f64 buffer:
///   `[n_sp, n_ori, range, c_o, i_res, gi_res, uni_degree, grad_nugget, sp_nugget,
///     sp_xyz(3·n_sp), sp_if(n_sp), ori_xyz(3·n_ori), ori_pole(3·n_ori)]`
/// followed by an optional 6-f64 fault plane (point xyz + normal xyz). NaN/0 options are
/// passed through; the kernel substitutes its own defaults. Returns `Err` on shape
/// mismatch.
#[allow(clippy::too_many_arguments)]
fn build_fit_buffer(
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
    decimate_frac: f64,
    fault: Option<([f64; 3], [f64; 3])>,
) -> Result<Vec<f64>, JsError> {
    if !sp_xyz.len().is_multiple_of(3)
        || !ori_xyz.len().is_multiple_of(3)
        || !ori_pole.len().is_multiple_of(3)
    {
        return Err(JsError::new("xyz/pole arrays must have length divisible by 3"));
    }
    let n_sp = sp_xyz.len() / 3;
    let n_ori = ori_xyz.len() / 3;
    if n_sp != sp_interface.len() {
        return Err(JsError::new("sp_interface length must equal sp count"));
    }
    if ori_xyz.len() != ori_pole.len() {
        return Err(JsError::new("ori_xyz and ori_pole must have equal length"));
    }

    // Decimation: `decimate_frac` > 0 enables the coreset with that re-add threshold
    // (fraction of the field's isovalue range); NaN / ≤ 0 disables it (keep all points).
    let dec_on = decimate_frac.is_finite() && decimate_frac > 0.0;

    let mut buf = Vec::with_capacity(13 + 4 * n_sp + 6 * n_ori + 6);
    buf.push(n_sp as f64);
    buf.push(n_ori as f64);
    buf.push(range);
    buf.push(c_o);
    buf.push(i_res);
    buf.push(gi_res);
    buf.push(uni_degree as f64);
    buf.push(nugget); // grad_nugget
    buf.push(nugget); // sp_nugget
    buf.push(if dec_on { 1.0 } else { 0.0 }); // decimate_on
    buf.push(f64::NAN); // seed_cell → kernel default
    buf.push(if dec_on { decimate_frac } else { f64::NAN }); // threshold_frac
    buf.push(f64::NAN); // point_budget → default (none)
    buf.extend_from_slice(sp_xyz);
    buf.extend_from_slice(sp_interface);
    buf.extend_from_slice(ori_xyz);
    buf.extend_from_slice(ori_pole);
    if let Some((point, normal)) = fault {
        buf.extend_from_slice(&point);
        buf.extend_from_slice(&normal);
    }
    Ok(buf)
}

/// A degenerate (all ~zero / non-finite) normal is the "no fault" signal the JS side
/// uses to mean an un-faulted fit.
fn fault_from_flat(point: &[f64], normal: &[f64]) -> Option<([f64; 3], [f64; 3])> {
    if point.len() < 3 || normal.len() < 3 {
        return None;
    }
    let n2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
    if !n2.is_finite() || n2 < 1e-12 {
        return None;
    }
    Some((
        [point[0], point[1], point[2]],
        [normal[0], normal[1], normal[2]],
    ))
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
    /// produce them.
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

impl From<crate::mesh::Mesh> for WasmMesh {
    fn from(m: crate::mesh::Mesh) -> Self {
        WasmMesh {
            positions: m.positions,
            normals: m.normals,
            indices: m.indices,
        }
    }
}

const EMPTY_MESH: fn() -> WasmMesh = || WasmMesh {
    positions: Vec::new(),
    normals: Vec::new(),
    indices: Vec::new(),
};

/// A fitted model exposed to JS. Wraps a [`RemoteModel`] handle into the kernel module.
#[wasm_bindgen]
pub struct WasmModel {
    inner: RemoteModel,
}

/// Build an [`OctreeConfig`] from the JS octree params (shared by the mesh methods).
fn octree_cfg(max_depth: u32, min_depth: u32, qef_threshold: f64, cap: bool) -> OctreeConfig {
    OctreeConfig {
        max_depth: max_depth.max(1),
        min_depth: min_depth.min(max_depth),
        qef_threshold: if qef_threshold.is_finite() && qef_threshold > 0.0 {
            qef_threshold
        } else {
            OctreeConfig::default().qef_threshold
        },
        cap,
    }
}

#[wasm_bindgen]
impl WasmModel {
    /// Fit a model from flat typed arrays. `range`, `c_o`, `i_res`, `gi_res`,
    /// `uni_degree`, and `nugget` are the kernel options (pass gempy's live values;
    /// NaN/0 falls back to the kernel defaults).
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
        decimate_frac: f64,
    ) -> Result<WasmModel, JsError> {
        install_panic_hook();
        let buf = build_fit_buffer(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, None,
        )?;
        Ok(WasmModel { inner: remote::fit(&buf) })
    }

    /// Fit a model with a single **planar fault** offsetting the contacts. Same
    /// arrays/options as [`WasmModel::new`], plus the fault plane as `fault_point`
    /// (a point on the plane) and `fault_normal` (3-element `Float64Array`s). A
    /// degenerate (≈zero) normal means "no fault".
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
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<WasmModel, JsError> {
        install_panic_hook();
        let fault = fault_from_flat(fault_point, fault_normal);
        let buf = build_fit_buffer(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault,
        )?;
        // A degenerate fault → no trailing plane → kernel falls back to a plain fit, so
        // `gf_fit_faulted` is still safe to call (it tolerates a missing/zero plane).
        Ok(WasmModel { inner: remote::fit_faulted(&buf) })
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
        self.inner.lithology_unit([x, y, z]) as usize
    }

    /// Evaluate the field over a flat `n*3` grid; returns an `n`-long `Float64Array`.
    pub fn scalar_field_grid(&self, grid_xyz: &[f64]) -> Vec<f64> {
        grid_xyz
            .chunks_exact(3)
            .map(|c| self.inner.scalar_field([c[0], c[1], c[2]]))
            .collect()
    }

    /// Adaptive **octree** dual contouring of `scalar_field == isovalue` over the cube
    /// `[min, min+size]`. `cap=false` → bounded open sheet; `cap=true` → watertight
    /// closed solid.
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
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, cap);
        let field = self.inner.whole_field();
        octree_dual_contour(&field, [min_x, min_y, min_z], size, isovalue, cfg).into()
    }

    /// Number of meshable interfaces (contacts) in fault block `block`.
    pub fn block_interface_count(&self, block: usize) -> usize {
        self.inner.block_interface_count(block)
    }

    /// Mesh interface `interface_idx` of fault block `block`, **clipped to that block's
    /// region** (the open-sheet view). EMPTY mesh for a field-less block / out-of-range
    /// interface.
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
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, false);
        self.inner
            .mesh_block_interface(block, interface_idx, [min_x, min_y, min_z], size, cfg)
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// Number of lithology UNITS in fault `block`. `0` if out of range.
    pub fn block_unit_count(&self, block: usize) -> usize {
        self.inner.block_unit_count(block)
    }

    /// A watertight CLOSED solid for one lithology UNIT of one fault block, capped by
    /// the box AND the fault plane, with shared contacts welded. EMPTY mesh for a
    /// field-less block / out-of-range unit.
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
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, true);
        self.inner
            .mesh_block_unit_solid_welded(block, unit_idx, [min_x, min_y, min_z], size, cfg)
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// Recovered per-interface isovalues, ascending, as a `Float64Array`.
    pub fn interface_isovalues(&self) -> Vec<f64> {
        self.inner.interface_isovalues()
    }

    /// Interface ids matching `interface_isovalues()` order, as a `Float64Array`.
    pub fn interface_ids(&self) -> Vec<f64> {
        self.inner.interface_ids()
    }

    /// Interface ids of fault `block` (parallel to `mesh_block_interface_octree`'s
    /// `interface_idx` for that block), as a `Float64Array`. Empty if out of range.
    pub fn block_interface_ids(&self, block: usize) -> Vec<f64> {
        self.inner.block_interface_ids(block)
    }

    /// Solve residual ‖K w − b‖₂ (diagnostic).
    pub fn residual(&self) -> f64 {
        self.inner.residual()
    }

    /// Largest within-interface isovalue spread at fit time (diagnostic).
    pub fn isovalue_spread(&self) -> f64 {
        self.inner.isovalue_spread()
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

/// A multi-series stack exposed to JS (GemPy-style unconformities), over the two-wasm
/// split: each series is fitted in the kernel module and held here as a [`RemoteModel`]
/// handle, so the stack's erode/onlap meshing runs in the BSD engine while the cokriging
/// lives in the EUPL kernel.
///
/// Build it by calling [`WasmStack::add_series`] **youngest series first**, each with its
/// own surface points/orientations, a relation code, and an optional planar fault, then
/// query per-point lithology ([`WasmStack::lithology_label`]) or mesh one interface
/// clipped to its series' visible region ([`WasmStack::mesh_interface_octree`]).
#[wasm_bindgen]
pub struct WasmStack {
    inner: Stack<RemoteModel>,
}

#[wasm_bindgen]
impl WasmStack {
    /// An empty stack. Add series youngest→oldest with [`WasmStack::add_series`].
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmStack {
        install_panic_hook();
        WasmStack { inner: Stack::new(Vec::new()) }
    }

    /// Fit and append one series (the next-oldest). Arrays and kernel options match
    /// [`WasmModel::new`]; `relation` is the code in [`relation_from_code`]
    /// (`0=Erode 1=Onlap 2=Conformable 3=Basement`). `fault_point`/`fault_normal` apply a
    /// single planar fault to THIS series' fit (a degenerate normal = no fault). Appends
    /// in call order, so the FIRST `add_series` is the youngest.
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
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<(), JsError> {
        install_panic_hook();
        let fault = fault_from_flat(fault_point, fault_normal);
        let buf = build_fit_buffer(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault,
        )?;
        let model = if fault.is_some() {
            remote::fit_faulted(&buf)
        } else {
            remote::fit(&buf)
        };
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
            .map(|s| s.model.interface_isovalues().len())
            .unwrap_or(0)
    }

    /// The flattened global lithology label at a point, encoding `(series, unit)` as
    /// `series * 1000 + unit`. JS decodes with `Math.floor(v/1000)` and `v % 1000`.
    pub fn lithology_label(&self, x: f64, y: f64, z: f64) -> u32 {
        let (s, u) = self.inner.global_unit([x, y, z]);
        (s as u32) * 1000 + (u as u32)
    }

    /// Just the winning series index at a point (which series is visible there).
    pub fn series_at(&self, x: f64, y: f64, z: f64) -> usize {
        self.inner.global_unit([x, y, z]).0
    }

    /// Adaptive octree mesh of interface `interface_idx` of series `series`, clipped to
    /// that series' visible region (the domain box ∩ outside every younger erosion).
    /// EMPTY mesh if indices are out of range. Octree params mirror
    /// [`WasmModel::mesh_isosurface_octree`].
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
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, cap);
        self.inner
            .mesh_one(series, interface_idx, [min_x, min_y, min_z], size, cfg)
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }
}

impl Default for WasmStack {
    fn default() -> Self {
        Self::new()
    }
}
