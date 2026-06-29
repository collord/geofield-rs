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
use crate::stack::{Envelope, Intrusion, Relation, Stack, StackElement, StackedSeries, Topography, Vein};
use crate::{BlockModel, MeshModel};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

/// Stage 0 determinism gate (docs/12 §2.3). Runs the exact tri-tri spike in the browser's wasm and
/// returns its FNV-1a digest as hex. The native golden test pins the same value
/// (`exact_arr::tri_tri::spike_digest_is_deterministic_and_pinned`); `web3/e2e/exact-determinism.spec.ts`
/// asserts this browser value equals that constant — the cross-target half the `#[cfg(test)]` suite
/// cannot run here (no `wasm-bindgen-test` harness; see memory `geofield-no-wasm-cargo-test-harness`).
/// Inert unless built with `--features exact_arr`; never wired into production meshing.
#[cfg(feature = "exact_arr")]
#[wasm_bindgen]
pub fn exact_spike_digest_hex() -> String {
    format!("{:016X}", crate::exact_arr::spike_digest())
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

/// A mesh tagged with the `(series, interface)` it belongs to — the return shape of the
/// shared-crease cross-cut meshing, so the worker can place each contact in its own payload and
/// SKIP re-meshing it on the series path. `kind` is "wall" for the body's own surface, "contact"
/// for a cut series' contact.
#[wasm_bindgen]
pub struct WasmTaggedMesh {
    series: usize,
    interface: usize,
    is_contact: bool,
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
}

#[wasm_bindgen]
impl WasmTaggedMesh {
    #[wasm_bindgen(getter)]
    pub fn series(&self) -> usize {
        self.series
    }
    #[wasm_bindgen(getter)]
    pub fn interface(&self) -> usize {
        self.interface
    }
    /// `true` if this is a cut series' contact (route it to the series path's payload and skip the
    /// duplicate); `false` if it's the cross-cutting body's own wall.
    #[wasm_bindgen(getter)]
    pub fn is_contact(&self) -> bool {
        self.is_contact
    }
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A closed SOLID tagged with the `(series, block, unit)` it belongs to — the return shape of the
/// COREFINED cross-cut solid meshing ([`WasmStack::mesh_cross_cut_solids_corefined_octree`]). The
/// cross-cutting body's interior solid is tagged at its series with `block = 0, unit = 0`; each cut
/// rock unit carries its own `(series, block, unit)`, so the worker can SKIP re-meshing those units
/// on the per-series path. Empty Vec means the corefined path declined (pathological wall) and the
/// worker should fall back to the independent path.
#[wasm_bindgen]
pub struct WasmTaggedSolid {
    series: usize,
    block: usize,
    unit: usize,
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
}

#[wasm_bindgen]
impl WasmTaggedSolid {
    #[wasm_bindgen(getter)]
    pub fn series(&self) -> usize {
        self.series
    }
    #[wasm_bindgen(getter)]
    pub fn block(&self) -> usize {
        self.block
    }
    #[wasm_bindgen(getter)]
    pub fn unit(&self) -> usize {
        self.unit
    }
    pub fn positions(&self) -> Vec<f32> {
        self.positions.clone()
    }
    pub fn normals(&self) -> Vec<f32> {
        self.normals.clone()
    }
    pub fn indices(&self) -> Vec<u32> {
        self.indices.clone()
    }
    #[wasm_bindgen(getter)]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

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

/// Map a JS relation code to an Axis-A [`Relation`]. `0=Erode 1=Onlap 2=Conformable`;
/// anything else falls back to `Erode` (the Lajaunie–Calcagno / GemPy / LoopStructural
/// default). This is ONLY the seam-below axis; whether a series is the floor is the
/// separate `unbounded_above` parameter (two-axis model, feature 001 / contract D1).
/// The legacy `3=Basement` code is retired — the floor is now `unbounded_above=true`.
fn relation_from_code(code: u32) -> Relation {
    match code {
        1 => Relation::Onlap,
        2 => Relation::Conformable,
        _ => Relation::Erode,
    }
}

/// A multi-series stack exposed to JS (Lajaunie–Calcagno potential-field
/// unconformities, as in GemPy / LoopStructural — erode/onlap composed by
/// per-series scalar-field clipping), over the two-wasm
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

// Private helpers (NOT exported — kept out of the `#[wasm_bindgen]` impl).
impl WasmStack {
    /// Fit a [`RemoteModel`] from the flat ABI inputs + optional fault. Shared by
    /// `add_series` / `add_vein` / `add_intrusion` so the marshalling lives in one place.
    #[allow(clippy::too_many_arguments)]
    fn fit_model(
        &self,
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
    ) -> Result<RemoteModel, JsError> {
        let fault = fault_from_flat(fault_point, fault_normal);
        let buf = build_fit_buffer(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault,
        )?;
        Ok(if fault.is_some() {
            remote::fit_faulted(&buf)
        } else {
            remote::fit(&buf)
        })
    }
}

#[wasm_bindgen]
impl WasmStack {
    /// An empty stack. Add series youngest→oldest with [`WasmStack::add_series`].
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmStack {
        install_panic_hook();
        WasmStack { inner: Stack::new(Vec::new()) }
    }

    /// Fit and append one series (the next-oldest), in the **two-axis model**
    /// (`docs/stack_hierarchy_spec.md` §1–§2 / contract D1). Arrays and kernel options
    /// match [`WasmModel::new`]. The two axes are independent:
    ///   * `relation` — Axis A, the seam below: `0=Erode 1=Onlap 2=Conformable` (see
    ///     [`relation_from_code`]). Ignored when `unbounded_above` is set (the floor has
    ///     no series beneath it).
    ///   * `unbounded_above` — Axis B: `true` makes this the floor / fill-upward series
    ///     ([`StackedSeries::floor`]); `false` is a normally-bounded series.
    ///
    /// `fault_point`/`fault_normal` apply a single planar fault to THIS series' fit (a
    /// degenerate normal = no fault). Appends in call order, so the FIRST `add_series` is
    /// the youngest. (The legacy `relation=3=Basement` code is retired; pass
    /// `unbounded_above=true` instead.)
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
        unbounded_above: bool,
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<(), JsError> {
        install_panic_hook();
        let model = self.fit_model(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault_point, fault_normal,
        )?;
        // Axis B decides the constructor; Axis A (relation) applies only to a bounded series.
        self.inner.series.push(if unbounded_above {
            StackedSeries::floor(model).into()
        } else {
            StackedSeries::new(model, relation_from_code(relation)).into()
        });
        Ok(())
    }

    /// Fit and insert the singleton **topography** surface as the youngest element
    /// (`docs/topography_lithology_solids_spec.md` §2). Fit inputs match
    /// [`WasmStack::add_series`] (one interface — the ground); it carries no relation or
    /// `unbounded_above`. Pinned at index 0 (youngest) regardless of call order, and **at
    /// most one** per stack — a second call is rejected (the singleton invariant is enforced
    /// here, structurally, not left to the UI). Above the ground is Air; every older element
    /// is clipped to below the ground via the erode masker.
    #[allow(clippy::too_many_arguments)]
    pub fn add_topography(
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
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<(), JsError> {
        install_panic_hook();
        if self.inner.series.iter().any(|e| e.is_topography()) {
            return Err(JsError::new("a stack may have at most one topography surface"));
        }
        let model = self.fit_model(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault_point, fault_normal,
        )?;
        // Pinned youngest (index 0) regardless of when it was added.
        self.inner.series.insert(0, StackElement::Topography(Topography { model }));
        Ok(())
    }

    /// Fit and append a cross-cutting **vein** (contract D2). Fit inputs match
    /// [`WasmStack::add_series`]; the vein is the tabular body BETWEEN interfaces
    /// `foot_wall` and `hanging_wall` (indices into the fitted interfaces). It cross-cuts
    /// everything older it intersects within that band — no Axis-A relation.
    #[allow(clippy::too_many_arguments)]
    pub fn add_vein(
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
        foot_wall: usize,
        hanging_wall: usize,
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<(), JsError> {
        install_panic_hook();
        let model = self.fit_model(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault_point, fault_normal,
        )?;
        self.inner
            .series
            .push(StackElement::Vein(Vein::new(model, foot_wall, hanging_wall)));
        Ok(())
    }

    /// Fit and append a cross-cutting **intrusion** (contract D2). Fit inputs match
    /// [`WasmStack::add_series`]. `envelope_inner == envelope_outer` ⇒ a single-envelope
    /// body (interior = below that contact); otherwise a shell between the two interfaces.
    #[allow(clippy::too_many_arguments)]
    pub fn add_intrusion(
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
        envelope_inner: usize,
        envelope_outer: usize,
        decimate_frac: f64,
        fault_point: &[f64],
        fault_normal: &[f64],
    ) -> Result<(), JsError> {
        install_panic_hook();
        let model = self.fit_model(
            sp_xyz, sp_interface, ori_xyz, ori_pole, range, c_o, i_res, gi_res, uni_degree,
            nugget, decimate_frac, fault_point, fault_normal,
        )?;
        let envelope = if envelope_inner == envelope_outer {
            Envelope::Single(envelope_inner)
        } else {
            Envelope::Shell { inner: envelope_inner, outer: envelope_outer }
        };
        self.inner
            .series
            .push(StackElement::Intrusion(Intrusion::new(model, envelope)));
        Ok(())
    }

    /// Number of series in the stack.
    #[wasm_bindgen(getter)]
    pub fn series_count(&self) -> usize {
        self.inner.len()
    }

    /// The hierarchy round-trip payload (contract D3+D5): the canonical declared state
    /// (`model`) plus the read-only derived view (`derived` — `effectiveRelation`,
    /// `validation`, `cutsBelow`) as a JSON string. The UI worker parses this into
    /// `CoreUpdate` after any mutation and treats `derived` as a read-only cache
    /// (core-api.md invariants). Meshes travel on the typed-array mesh ABI, not here.
    ///
    /// One getter rather than N flat derived-getters keeps the boundary a single
    /// serialized DTO (the design doc §7 contract) — the same payload the round-trip
    /// purity test (`schema::tests`) pins.
    pub fn core_update_json(&self) -> String {
        serde_json::to_string(&self.inner.core_update())
            .unwrap_or_else(|e| format!("{{\"error\":\"serialize CoreUpdate: {e}\"}}"))
    }

    /// Number of fault blocks in the OUTER partition (contract D4). **Currently 1** — the
    /// fault-network partition geometry is a deferred stub ([`crate::faulted`], research R3),
    /// so the whole stack instances into a single whole-domain block. This getter is the
    /// real, honest ABI surface for the partition; per-block overrides + the fault sub-panel
    /// UI (tasks T046/T047) arrive WITH the >1-block partition geometry, not before — wiring
    /// them against the single-block stub would be UI chrome over no observable behaviour
    /// (Constitution Principle IV/VI). The interface shape is real so callers can already
    /// iterate blocks.
    pub fn fault_block_count(&self) -> usize {
        crate::faulted::FaultNetwork::default().blocks().len()
    }

    /// Number of interfaces (meshable contacts) in series `series`. `0` if out of range.
    pub fn interface_count(&self, series: usize) -> usize {
        self.inner
            .series
            .get(series)
            .map(|s| s.model().interface_isovalues().len())
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

    /// Number of fault blocks in `series` (1 if that series is not faulted; 0 if out of
    /// range). Loop `block` over this for [`WasmStack::mesh_series_block_interface`].
    pub fn series_block_count(&self, series: usize) -> usize {
        self.inner.series_block_count(series)
    }

    /// Adaptive octree mesh of interface `interface_idx` of series `series`, **within a
    /// single fault block `block`** — the per-block open-sheet view (matches `web/`): the
    /// contact is contoured from that block's own smooth field and cut flush at the fault
    /// plane, so a faulted contact renders as separate sheets per block rather than one
    /// surface stretched across the offset. Still clipped to the series' visible region
    /// (younger erosions). EMPTY mesh if any index is out of range or the block is
    /// field-less.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_series_block_interface(
        &self,
        series: usize,
        block: usize,
        interface_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
        cap: bool,
    ) -> WasmMesh {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, cap);
        self.inner
            .mesh_one_block(
                series, block, interface_idx, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg,
            )
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// Same as [`Self::mesh_series_block_interface`] but close the seam against younger ERODE
    /// surfaces with the mesh–mesh COREFINER (`SeamMode::Clip`) instead of the field-half-space
    /// masker — the A/B counterpart the "clip seam" toggle drives for stratigraphic erode scenes
    /// (today that toggle only affects cross-cut bodies). Unfaulted erode series only; a faulted one
    /// falls back to the masker path inside [`Stack::mesh_one_block_erode_clipped`].
    ///
    /// SEPARATE method (not a flag on the snap one) for the same reason as the cross-cut clip pair: a
    /// trailing flag arg is silently dropped by this wasm-bindgen toolchain. No `cap` arg either —
    /// the corefiner erode path contours the contact box-only itself. The worker picks this method
    /// when the checkbox is on.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_series_block_interface_erode_clip(
        &self,
        series: usize,
        block: usize,
        interface_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> WasmMesh {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, false);
        self.inner
            .mesh_one_block_erode_clipped(
                series, block, interface_idx, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg,
                crate::stack::SeamMode::Clip,
            )
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// Mesh ONE wall of a CROSS-CUTTING body (vein/intrusion) at element `series` as an open
    /// contact sheet — the body's own surface, clipped to the extent box and any younger body
    /// above it. EMPTY mesh if `series` is a stratigraphic series or any index is out of range.
    /// (Veins have two walls: foot wall = interface 0, hanging wall = 1.)
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_surface_octree(
        &self,
        series: usize,
        interface_idx: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
        weld: bool,
    ) -> WasmMesh {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, false);
        // When `weld` is set, the wall's T-junctions against the contacts it cuts are welded shut
        // (the surface-mode slivers fix); `false` is the escape hatch to the old independent mesh.
        self.inner
            .mesh_cross_cut_surface_welded(
                series, interface_idx, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg, weld,
            )
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// SHARED-CREASE cross-cut meshing (the watertight-by-construction fix for the contact↔wall
    /// see-through holes). Co-contours the body's walls AND the contacts it cuts on ONE octree, and
    /// returns them tagged: each [`WasmTaggedMesh`] carries its `(series, interface)` and an
    /// `is_contact` flag. The worker emits the walls (body's own surfaces) and the contacts, and
    /// SKIPS re-meshing the returned `(series, interface)` contacts on the series path. Empty Vec
    /// for a stratigraphic series / out-of-range body.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_shared_octree(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> Vec<WasmTaggedMesh> {
        self.cross_cut_shared(
            series, min_x, min_y, min_z, size, ext_x, ext_y, ext_z, max_depth, min_depth,
            qef_threshold, crate::stack::SeamMode::Snap,
        )
    }

    /// Same as [`Self::mesh_cross_cut_shared_octree`] but closes the contact↔wall seam with the
    /// mesh–mesh CLIP corefiner (shared seam, no see-through dashes) instead of the legacy snap.
    ///
    /// This is a SEPARATE method rather than a flag on the snap one ON PURPOSE: a trailing flag
    /// argument (tried as both `bool` and `u32`) was silently arriving `0`/`false` in Rust across
    /// this wasm-bindgen/wasm-pack toolchain — an ABI desync on the last arg. Two no-flag methods
    /// avoid the marshalling entirely; the worker picks one by the checkbox.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_shared_octree_clip(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> Vec<WasmTaggedMesh> {
        self.cross_cut_shared(
            series, min_x, min_y, min_z, size, ext_x, ext_y, ext_z, max_depth, min_depth,
            qef_threshold, crate::stack::SeamMode::Clip,
        )
    }

    /// Shared body of the two `mesh_cross_cut_shared_octree*` bindings — contours the body's walls
    /// and the contacts it cuts on one tree, closing the seam per `seam`. Not exported.
    #[allow(clippy::too_many_arguments)]
    fn cross_cut_shared(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
        seam: crate::stack::SeamMode,
    ) -> Vec<WasmTaggedMesh> {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, false);
        let Some((walls, contacts)) = self.inner.mesh_cross_cut_shared(
            series,
            [min_x, min_y, min_z],
            size,
            [ext_x, ext_y, ext_z],
            cfg,
            seam,
        ) else {
            return Vec::new();
        };
        let mut out: Vec<WasmTaggedMesh> = Vec::new();
        for (k, m) in walls.into_iter().enumerate() {
            out.push(WasmTaggedMesh {
                series,
                interface: k,
                is_contact: false,
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            });
        }
        for (j, k, m) in contacts {
            out.push(WasmTaggedMesh {
                series: j,
                interface: k,
                is_contact: true,
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            });
        }
        out
    }

    /// Closed watertight INTERIOR SOLID of a cross-cutting body (vein/intrusion) — its
    /// namesake-lithology volume, clipped to the box and every younger element (feature 003).
    /// EMPTY mesh for a stratigraphic series / topography / out-of-range index.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_solid_octree(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> WasmMesh {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, true);
        self.inner
            .mesh_cross_cut_solid(series, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg)
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }

    /// SHARED-TREE cross-cut SOLID meshing (Strategy A, `docs/volume_mesh_seam.md`): the body's
    /// interior solid + every rock UNIT solid it cuts, co-contoured on ONE octree so they share the
    /// wall vertex-for-vertex (REAL fusion, no daylight). Returns the body (tagged unit 0) + the cut
    /// rock units tagged `(series, block, unit)`; the worker emits all and SKIPS re-meshing those
    /// units on the per-series path. Empty for a stratigraphic series / topography / out-of-range /
    /// wall-less body. This is the `Snap` (per-cell crease) variant.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_solids_shared_octree(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> Vec<WasmTaggedSolid> {
        self.shared_solids(series, min_x, min_y, min_z, size, ext_x, ext_y, ext_z, max_depth, min_depth, qef_threshold, crate::stack::SeamMode::Snap)
    }

    /// Same as [`Self::mesh_cross_cut_solids_shared_octree`] but with the A.2 (`docs/09`) deterministic
    /// crease-ring projection: the wall∩bed junction is SMOOTH (the analytic intersection curve)
    /// instead of a per-cell staircase, with NO `clip` (so it's wasm-deterministic and keeps the
    /// shared-tree fusion). SEPARATE no-flag method (the trailing-flag-arg ABI desync, see the
    /// contact path); the worker picks one by the "clip seam" checkbox.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_solids_shared_clip_octree(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> Vec<WasmTaggedSolid> {
        self.shared_solids(series, min_x, min_y, min_z, size, ext_x, ext_y, ext_z, max_depth, min_depth, qef_threshold, crate::stack::SeamMode::Clip)
    }

    /// Shared body of the two `mesh_cross_cut_solids_shared*` bindings. Not exported.
    #[allow(clippy::too_many_arguments)]
    fn shared_solids(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
        seam: crate::stack::SeamMode,
    ) -> Vec<WasmTaggedSolid> {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, true);
        let Some((body, rocks)) = self.inner.mesh_cross_cut_solids_shared(
            series, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg, seam,
        ) else {
            return Vec::new();
        };
        let mut out: Vec<WasmTaggedSolid> = Vec::with_capacity(1 + rocks.len());
        out.push(WasmTaggedSolid { series, block: 0, unit: 0, positions: body.positions, normals: body.normals, indices: body.indices });
        for (j, b, u, m) in rocks {
            out.push(WasmTaggedSolid { series: j, block: b, unit: u, positions: m.positions, normals: m.normals, indices: m.indices });
        }
        out
    }

    /// WALL-FREE host UNIT SLABS for a cross-cutting body's cut units, co-contoured on ONE shared
    /// octree so curved beds are followed and adjacent units' bed contacts coincide (docs/13 §5
    /// follow-up). The Manifold A/B path differences the band from these CLEAN slabs (the band is the
    /// first tagged solid of `mesh_cross_cut_solids_shared_*`). Returns the body interior solid
    /// tagged `(series, 0, 0)` FIRST, then each cut unit `(series, block, unit)`. Empty for a
    /// stratigraphic series / topography / wall-less / out-of-range body. This exists because the
    /// independent per-unit [`Self::mesh_series_block_unit_solid`] leaves LOD-transition T-junctions
    /// that Manifold rejects — the shared-tree contour avoids them by construction.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_cross_cut_host_slabs_octree(
        &self,
        series: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> Vec<WasmTaggedSolid> {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, true);
        let Some(slabs) = self.inner.mesh_cross_cut_host_slabs(
            series, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg,
        ) else {
            return Vec::new();
        };
        slabs
            .into_iter()
            .map(|(j, b, u, m)| WasmTaggedSolid {
                series: j,
                block: b,
                unit: u,
                positions: m.positions,
                normals: m.normals,
                indices: m.indices,
            })
            .collect()
    }

    /// Number of lithology UNIT solids in `block` of `series` (`interfaces + 1`; 0 if out
    /// of range or field-less). This includes the topmost VOID unit above the series'
    /// bounding contact — for the renderable rock units use
    /// [`WasmStack::present_unit_count`].
    pub fn block_unit_count(&self, series: usize, block: usize) -> usize {
        self.inner.block_unit_count(series, block)
    }

    /// Number of PRESENT (rock-bearing) lithology units in `block` of `series` — the units
    /// to mesh as closed solids (`block_unit_count - 1`; the topmost unit is the void above
    /// the series' bounding contact, owned by a younger series). Loop `unit` over this for
    /// [`WasmStack::mesh_series_block_unit_solid`].
    pub fn present_unit_count(&self, series: usize, block: usize) -> usize {
        self.inner.present_unit_count(series, block)
    }

    /// Adaptive octree mesh of ONE lithology UNIT of `block` of `series` as a watertight
    /// CLOSED solid — the slab between consecutive contacts, capped by the box, the block's
    /// fault planes, and every younger erosion (so it daylights against the erosion surface
    /// instead of being trimmed away like the open-sheet view). The `cap` flag is implied
    /// `true`; the octree params mirror the other meshers. EMPTY mesh if any index is out
    /// of range or the block is field-less.
    #[allow(clippy::too_many_arguments)]
    pub fn mesh_series_block_unit_solid(
        &self,
        series: usize,
        block: usize,
        unit: usize,
        min_x: f64,
        min_y: f64,
        min_z: f64,
        size: f64,
        ext_x: f64,
        ext_y: f64,
        ext_z: f64,
        max_depth: u32,
        min_depth: u32,
        qef_threshold: f64,
    ) -> WasmMesh {
        let cfg = octree_cfg(max_depth, min_depth, qef_threshold, true);
        self.inner
            .mesh_one_block_unit_solid(
                series, block, unit, [min_x, min_y, min_z], size, [ext_x, ext_y, ext_z], cfg,
            )
            .map(Into::into)
            .unwrap_or_else(EMPTY_MESH)
    }
}

impl Default for WasmStack {
    fn default() -> Self {
        Self::new()
    }
}
