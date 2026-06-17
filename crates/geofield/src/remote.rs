//! WASM-only remote field oracle: the engine's view of the separately-loaded kernel
//! module (`geofield_kernel.wasm`).
//!
//! The engine and kernel are two distinct wasm instances with **separate linear
//! memories**. The kernel keeps fitted `Model`s in its own memory behind `u32` handles;
//! this module holds only handles and reaches the field oracle through wasm imports
//! (module `"kernel"`). The octree/mesh code is written against [`ScalarField`] and a
//! [`BlockModel`]; [`RemoteField`]/[`RemoteModel`] implement those by forwarding across
//! the boundary, so NONE of the kernel's (EUPL) code is linked into `geofield.wasm`.
//!
//! ## The cross-memory contract
//!
//! Scalar-in/scalar-out calls (`gf_eval`, counts) are raw wasm→wasm imports — values
//! cross fine. But any kernel function that *writes a buffer* writes into the KERNEL's
//! memory, which this instance cannot read. So the buffer-returning imports
//! (`gf_eval_grad`, `gf_block_interface_isovalues`, `gf_block_half_spaces`) are
//! **host-bridged**: the host (JS) supplies them as shims that call the real kernel
//! export into a kernel scratch buffer, then copy the result bytes into THIS instance's
//! memory at the `out_ptr` we pass (an offset into engine memory). From Rust's side they
//! look like ordinary imports writing to our own pointer.

use crate::{BlockModel, HalfSpace, ScalarField, SeriesModel};

const WHOLE_MODEL: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// Imports from the kernel module. The host wires these to the kernel instance's
// exports (scalar ones directly; buffer ones via memory-bridging shims).
// ---------------------------------------------------------------------------
#[link(wasm_import_module = "kernel")]
extern "C" {
    fn gf_fit(ptr: *const u8, len_f64: usize) -> u32;
    fn gf_fit_faulted(ptr: *const u8, len_f64: usize) -> u32;
    fn gf_eval(handle: u32, block: u32, x: f64, y: f64, z: f64) -> f64;
    /// Writes 3 f64 (world ∇Z) to `out_ptr` in ENGINE memory (host-bridged).
    fn gf_eval_grad(handle: u32, block: u32, x: f64, y: f64, z: f64, out_ptr: *mut u8);
    fn gf_lithology_unit(handle: u32, x: f64, y: f64, z: f64) -> u32;
    fn gf_nblocks(handle: u32) -> u32;
    fn gf_block_interface_count(handle: u32, b: u32) -> u32;
    fn gf_block_unit_count(handle: u32, b: u32) -> u32;
    /// Writes ≤`cap` f64 isovalues to `out_ptr` in ENGINE memory; returns the count.
    fn gf_block_interface_isovalues(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32;
    /// Writes ≤`cap` f64 interface ids (parallel to isovalues) to ENGINE memory; returns count.
    fn gf_block_interface_ids(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32;
    /// Writes ≤`cap` half-spaces (7 f64 each) to `out_ptr` in ENGINE memory; returns count.
    fn gf_block_half_spaces(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32;
    /// Number of REPRESENTATIVE (whole-model) interfaces.
    fn gf_interface_count(handle: u32) -> u32;
    /// Writes ≤`cap` representative isovalues to ENGINE memory; returns count.
    fn gf_interface_isovalues(handle: u32, out_ptr: *mut u8, cap: u32) -> u32;
    /// Writes ≤`cap` representative interface ids to ENGINE memory; returns count.
    fn gf_interface_ids(handle: u32, out_ptr: *mut u8, cap: u32) -> u32;
    /// Solve residual / isovalue-spread diagnostics (2 f64) to ENGINE memory.
    fn gf_diagnostics(handle: u32, out_ptr: *mut u8);
    /// Drop the model behind `handle`.
    fn gf_release(handle: u32);
}

// ---------------------------------------------------------------------------
// Allocation marshalling for fit inputs (staged into KERNEL memory).
// ---------------------------------------------------------------------------

/// Fit a non-faulted model from an f64 input buffer (engine-side `Vec<f64>`), returning
/// a kernel model handle. The bytes are staged into kernel memory via the host-bridged
/// `gf_fit` import (the host copies our slice into a kernel `gf_alloc` buffer).
pub fn fit(input: &[f64]) -> RemoteModel {
    let handle = unsafe { gf_fit(input.as_ptr() as *const u8, input.len()) };
    RemoteModel { handle }
}

/// Fit a single-planar-fault model (input buffer carries the 6 trailing fault f64).
pub fn fit_faulted(input: &[f64]) -> RemoteModel {
    let handle = unsafe { gf_fit_faulted(input.as_ptr() as *const u8, input.len()) };
    RemoteModel { handle }
}

// ---------------------------------------------------------------------------
// RemoteField — one block's smooth field as a `ScalarField`, forwarding to the kernel.
// ---------------------------------------------------------------------------

/// A handle into the kernel's field oracle: model `handle`, block `block`
/// (`WHOLE_MODEL` = the whole faulted model's per-block dispatch).
#[derive(Clone, Copy)]
pub struct RemoteField {
    handle: u32,
    block: u32,
}

impl ScalarField for RemoteField {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        unsafe { gf_eval(self.handle, self.block, p[0], p[1], p[2]) }
    }

    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        let mut out = [0.0f64; 3];
        unsafe {
            gf_eval_grad(self.handle, self.block, p[0], p[1], p[2], out.as_mut_ptr() as *mut u8);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// RemoteModel — a fitted model handle exposing the BlockModel surface the mesher needs.
// ---------------------------------------------------------------------------

/// A fitted model living in the kernel instance, addressed by handle. Implements
/// [`BlockModel`] so the SAME meshing code (the blanket `impl MeshModel`) drives it.
pub struct RemoteModel {
    handle: u32,
}

impl RemoteModel {
    /// The whole-model field oracle (per-block dispatch across faults).
    pub fn whole_field(&self) -> RemoteField {
        RemoteField { handle: self.handle, block: WHOLE_MODEL }
    }

    /// Lithology unit index at a point (per-block classification).
    pub fn lithology_unit(&self, p: [f64; 3]) -> u32 {
        unsafe { gf_lithology_unit(self.handle, p[0], p[1], p[2]) }
    }

    /// Scalar field of the whole model at a point (per-block dispatch).
    pub fn scalar_field(&self, p: [f64; 3]) -> f64 {
        unsafe { gf_eval(self.handle, WHOLE_MODEL, p[0], p[1], p[2]) }
    }

    /// Gradient of the whole model at a point.
    pub fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        self.whole_field().scalar_field_gradient(p)
    }

    /// The model's REPRESENTATIVE interface isovalues (whole-model set, ascending).
    pub fn interface_isovalues(&self) -> Vec<f64> {
        let cap = unsafe { gf_interface_count(self.handle) } as usize;
        if cap == 0 {
            return Vec::new();
        }
        let mut buf = vec![0.0f64; cap];
        let n = unsafe { gf_interface_isovalues(self.handle, buf.as_mut_ptr() as *mut u8, cap as u32) }
            as usize;
        buf.truncate(n);
        buf
    }

    /// The model's REPRESENTATIVE interface ids (parallel to `interface_isovalues`).
    pub fn interface_ids(&self) -> Vec<f64> {
        let cap = unsafe { gf_interface_count(self.handle) } as usize;
        if cap == 0 {
            return Vec::new();
        }
        let mut buf = vec![0.0f64; cap];
        let n =
            unsafe { gf_interface_ids(self.handle, buf.as_mut_ptr() as *mut u8, cap as u32) } as usize;
        buf.truncate(n);
        buf
    }

    /// Block `b`'s interface ids (parallel to `block_interface_isovalues(b)`).
    pub fn block_interface_ids(&self, b: usize) -> Vec<f64> {
        let cap = unsafe { gf_block_interface_count(self.handle, b as u32) } as usize;
        if cap == 0 {
            return Vec::new();
        }
        let mut buf = vec![0.0f64; cap];
        let n = unsafe {
            gf_block_interface_ids(self.handle, b as u32, buf.as_mut_ptr() as *mut u8, cap as u32)
        } as usize;
        buf.truncate(n);
        buf
    }

    /// Solve residual ‖K w − b‖₂ (diagnostic).
    pub fn residual(&self) -> f64 {
        self.diagnostics()[0]
    }

    /// Largest within-interface isovalue spread at fit time (diagnostic).
    pub fn isovalue_spread(&self) -> f64 {
        self.diagnostics()[1]
    }

    fn diagnostics(&self) -> [f64; 2] {
        let mut out = [0.0f64; 2];
        unsafe { gf_diagnostics(self.handle, out.as_mut_ptr() as *mut u8) };
        out
    }
}

impl SeriesModel for RemoteModel {
    type Field<'a> = RemoteField;
    fn field(&self) -> Self::Field<'_> {
        self.whole_field()
    }
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        RemoteModel::scalar_field(self, p)
    }
    fn lithology_unit(&self, p: [f64; 3]) -> usize {
        RemoteModel::lithology_unit(self, p) as usize
    }
    fn interface_isovalues(&self) -> Vec<f64> {
        RemoteModel::interface_isovalues(self)
    }
    fn interface_ids(&self) -> Vec<u32> {
        RemoteModel::interface_ids(self).into_iter().map(|x| x as u32).collect()
    }
}

impl Drop for RemoteModel {
    fn drop(&mut self) {
        unsafe { gf_release(self.handle) };
    }
}

impl BlockModel for RemoteModel {
    type Field<'a> = RemoteField;

    fn nblocks(&self) -> usize {
        unsafe { gf_nblocks(self.handle) as usize }
    }

    fn block_field(&self, b: usize) -> Option<Self::Field<'_>> {
        // A field-less block has no interfaces; mirror the kernel's `block_field` None.
        if unsafe { gf_block_interface_count(self.handle, b as u32) } == 0 {
            return None;
        }
        Some(RemoteField { handle: self.handle, block: b as u32 })
    }

    fn block_interface_isovalues(&self, b: usize) -> Vec<f64> {
        let cap = unsafe { gf_block_interface_count(self.handle, b as u32) } as usize;
        if cap == 0 {
            return Vec::new();
        }
        let mut buf = vec![0.0f64; cap];
        let n = unsafe {
            gf_block_interface_isovalues(self.handle, b as u32, buf.as_mut_ptr() as *mut u8, cap as u32)
        } as usize;
        buf.truncate(n);
        buf
    }

    fn block_interface_count(&self, b: usize) -> usize {
        unsafe { gf_block_interface_count(self.handle, b as u32) as usize }
    }

    fn block_unit_count(&self, b: usize) -> usize {
        unsafe { gf_block_unit_count(self.handle, b as u32) as usize }
    }

    fn block_half_spaces(&self, b: usize) -> Vec<HalfSpace> {
        // Over-provision: a single-fault block has ≤1 plane; a few is plenty. Read into a
        // 7-f64-per-half-space buffer, then unpack.
        const CAP: u32 = 8;
        let mut buf = vec![0.0f64; (CAP as usize) * 7];
        let n = unsafe {
            gf_block_half_spaces(self.handle, b as u32, buf.as_mut_ptr() as *mut u8, CAP)
        } as usize;
        (0..n)
            .map(|i| {
                let o = i * 7;
                HalfSpace {
                    point: [buf[o], buf[o + 1], buf[o + 2]],
                    normal: [buf[o + 3], buf[o + 4], buf[o + 5]],
                    keep_positive: buf[o + 6] != 0.0,
                }
            })
            .collect()
    }
}
