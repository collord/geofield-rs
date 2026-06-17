//! C-ABI export layer for the two-wasm split (feature = "wasm-abi").
//!
//! When `geofield-kernel` is built as its OWN `cdylib` wasm module, these `extern "C"`
//! exports are the boundary the `geofield` engine module calls across (via wasm
//! imports). The kernel keeps the fitted [`Model`]s in its own linear memory behind
//! integer handles; the engine holds only `u32` handles and queries the field oracle
//! pointwise (`gf_eval` / `gf_eval_grad`) during contouring.
//!
//! Only scalars and explicit byte buffers cross the boundary (the two wasm instances
//! have separate memories — no Rust pointers are shared). `gf_alloc`/`gf_free` let the
//! host stage input bytes into kernel memory and read output bytes back.

use crate::data::{Orientation, Series, SurfacePoint};
use crate::fault::{OneFault, PlanarFault};
use crate::{KernelOptions, Model, ScalarField};
use std::cell::RefCell;

// ---------------------------------------------------------------------------
// Handle slab — fitted Models live here, in the kernel's own memory.
// (wasm is single-threaded, so a thread-local RefCell is the simplest store.)
// ---------------------------------------------------------------------------

thread_local! {
    static MODELS: RefCell<Vec<Option<Model>>> = const { RefCell::new(Vec::new()) };
}

const WHOLE_MODEL: u32 = u32::MAX;

fn put_model(m: Model) -> u32 {
    MODELS.with(|s| {
        let mut s = s.borrow_mut();
        // Reuse a freed slot if any, else append.
        if let Some(i) = s.iter().position(|x| x.is_none()) {
            s[i] = Some(m);
            i as u32
        } else {
            s.push(Some(m));
            (s.len() - 1) as u32
        }
    })
}

/// Run `f` with the model behind `handle`; returns the closure's value or `default` if
/// the handle is stale/invalid.
fn with_model<T>(handle: u32, default: T, f: impl FnOnce(&Model) -> T) -> T {
    MODELS.with(|s| {
        let s = s.borrow();
        match s.get(handle as usize).and_then(|x| x.as_ref()) {
            Some(m) => f(m),
            None => default,
        }
    })
}

// ---------------------------------------------------------------------------
// Byte-buffer helpers so the host can stage inputs / read outputs.
// ---------------------------------------------------------------------------

/// Allocate `len` bytes in kernel memory; returns the pointer (host writes inputs here).
#[no_mangle]
pub extern "C" fn gf_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer previously returned by `gf_alloc`.
///
/// # Safety
/// `ptr`/`len` must be exactly a prior `gf_alloc(len)` result, freed once.
#[no_mangle]
pub unsafe extern "C" fn gf_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

/// Read a little-endian f64 slice from a host-staged buffer.
///
/// # Safety
/// `ptr` must point to `n*8` valid bytes in kernel memory.
unsafe fn read_f64s(ptr: *const u8, n: usize) -> Vec<f64> {
    let bytes = std::slice::from_raw_parts(ptr, n * 8);
    (0..n)
        .map(|i| {
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
            f64::from_le_bytes(b)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Input decoding. The host stages one f64 buffer laid out as:
//   [n_sp, n_ori,  range, c_o, i_res, gi_res, uni_degree, grad_nugget, sp_nugget,
//    sp_xyz (3*n_sp), sp_if (n_sp), ori_xyz (3*n_ori), ori_pole (3*n_ori)]
// (everything as f64 — ids/degree are small integers carried losslessly). A trailing
// optional fault plane (6 f64: point xyz + normal xyz) is read by `gf_fit_faulted`.
// ---------------------------------------------------------------------------

struct FitInput {
    series: Series,
    opts: KernelOptions,
    consumed: usize, // f64s consumed (so the fault plane can follow)
}

/// # Safety
/// `ptr` must point to a valid host-staged f64 buffer of at least the encoded length.
unsafe fn decode_fit(ptr: *const u8, len_f64: usize) -> FitInput {
    let v = read_f64s(ptr, len_f64);
    let n_sp = v[0] as usize;
    let n_ori = v[1] as usize;
    let d = KernelOptions::default();
    let or = |x: f64, dflt: f64| if x.is_nan() { dflt } else { x };
    let opts = KernelOptions {
        range: or(v[2], d.range),
        c_o: or(v[3], d.c_o),
        i_res: or(v[4], d.i_res),
        gi_res: or(v[5], d.gi_res),
        uni_degree: if v[6] as u32 == 0 { d.uni_degree } else { v[6] as u32 },
        grad_nugget: or(v[7], d.grad_nugget),
        sp_nugget: or(v[8], d.sp_nugget),
    };
    let mut off = 9;
    let sps: Vec<SurfacePoint> = (0..n_sp)
        .map(|i| {
            let p = [v[off + 3 * i], v[off + 3 * i + 1], v[off + 3 * i + 2]];
            p
        })
        .enumerate()
        .map(|(i, p)| SurfacePoint::new(p, v[off + 3 * n_sp + i] as u32))
        .collect();
    off += 4 * n_sp;
    let oris: Vec<Orientation> = (0..n_ori)
        .map(|i| {
            Orientation::new(
                [v[off + 3 * i], v[off + 3 * i + 1], v[off + 3 * i + 2]],
                [
                    v[off + 3 * n_ori + 3 * i],
                    v[off + 3 * n_ori + 3 * i + 1],
                    v[off + 3 * n_ori + 3 * i + 2],
                ],
            )
        })
        .collect();
    off += 6 * n_ori;
    FitInput {
        series: Series::new(sps, oris),
        opts,
        consumed: off,
    }
}

// ---------------------------------------------------------------------------
// Exports.
// ---------------------------------------------------------------------------

/// Fit a non-faulted model from a host-staged f64 buffer (`len_f64` f64 values).
/// Returns a model handle.
///
/// # Safety
/// `ptr` must point to `len_f64*8` valid bytes staged by the host.
#[no_mangle]
pub unsafe extern "C" fn gf_fit(ptr: *const u8, len_f64: usize) -> u32 {
    let fi = decode_fit(ptr, len_f64);
    put_model(Model::fit(&fi.series, fi.opts))
}

/// Fit a single-planar-fault model. Same buffer as `gf_fit`, with 6 trailing f64
/// (fault point xyz + normal xyz) consumed after the series. A degenerate (~0) normal
/// falls back to a non-faulted fit.
///
/// # Safety
/// `ptr` must point to `len_f64*8` valid bytes staged by the host.
#[no_mangle]
pub unsafe extern "C" fn gf_fit_faulted(ptr: *const u8, len_f64: usize) -> u32 {
    let fi = decode_fit(ptr, len_f64);
    let v = read_f64s(ptr, len_f64);
    let off = fi.consumed;
    let model = if off + 6 <= len_f64 {
        let point = [v[off], v[off + 1], v[off + 2]];
        let normal = [v[off + 3], v[off + 4], v[off + 5]];
        let n2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if n2.is_finite() && n2 >= 1e-12 {
            let fault = Box::new(OneFault::new(PlanarFault::new(point, normal)));
            Model::fit_faulted(&fi.series, fault, fi.opts)
        } else {
            Model::fit(&fi.series, fi.opts)
        }
    } else {
        Model::fit(&fi.series, fi.opts)
    };
    put_model(model)
}

/// THE field oracle (hot path). `block == WHOLE_MODEL` (u32::MAX) queries the whole
/// faulted model (per-block dispatch); otherwise it queries block `block`'s smooth
/// field. Returns `+∞` for a stale handle / field-less block.
#[no_mangle]
pub extern "C" fn gf_eval(handle: u32, block: u32, x: f64, y: f64, z: f64) -> f64 {
    with_model(handle, f64::INFINITY, |m| {
        if block == WHOLE_MODEL {
            m.scalar_field([x, y, z])
        } else {
            match m.block_field(block as usize) {
                Some(bf) => bf.scalar_field([x, y, z]),
                None => f64::INFINITY,
            }
        }
    })
}

/// Gradient oracle. Writes 3 f64 (world-space ∇Z) to `out_ptr`.
///
/// # Safety
/// `out_ptr` must point to 24 valid writable bytes in kernel memory.
#[no_mangle]
pub unsafe extern "C" fn gf_eval_grad(handle: u32, block: u32, x: f64, y: f64, z: f64, out_ptr: *mut u8) {
    let g = with_model(handle, [0.0; 3], |m| {
        if block == WHOLE_MODEL {
            m.scalar_field_gradient([x, y, z])
        } else {
            match m.block_field(block as usize) {
                Some(bf) => bf.scalar_field_gradient([x, y, z]),
                None => [0.0; 3],
            }
        }
    });
    let out = std::slice::from_raw_parts_mut(out_ptr, 24);
    for (i, &c) in g.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&c.to_le_bytes());
    }
}

/// Lithology unit index at a point (per-block classification).
#[no_mangle]
pub extern "C" fn gf_lithology_unit(handle: u32, x: f64, y: f64, z: f64) -> u32 {
    with_model(handle, 0, |m| m.lithology_unit([x, y, z]) as u32)
}

/// Number of fault-bounded blocks (1 if non-faulted).
#[no_mangle]
pub extern "C" fn gf_nblocks(handle: u32) -> u32 {
    with_model(handle, 0, |m| m.nblocks() as u32)
}

/// Number of meshable interfaces in block `b`.
#[no_mangle]
pub extern "C" fn gf_block_interface_count(handle: u32, b: u32) -> u32 {
    with_model(handle, 0, |m| m.block_interface_count(b as usize) as u32)
}

/// Number of lithology units in block `b`.
#[no_mangle]
pub extern "C" fn gf_block_unit_count(handle: u32, b: u32) -> u32 {
    with_model(handle, 0, |m| m.block_unit_count(b as usize) as u32)
}

/// Write block `b`'s interface isovalues (ascending) as f64 to `out_ptr` (capacity
/// `cap` f64). Returns the count written (≤ cap).
///
/// # Safety
/// `out_ptr` must point to `cap*8` valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_block_interface_isovalues(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32 {
    with_model(handle, 0, |m| {
        let Some(l) = m.block_lithology(b as usize) else {
            return 0;
        };
        let n = l.interfaces.len().min(cap as usize);
        let out = std::slice::from_raw_parts_mut(out_ptr, n * 8);
        for (i, iv) in l.interfaces.iter().take(n).enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&iv.isovalue.to_le_bytes());
        }
        n as u32
    })
}

/// Write block `b`'s interface ids (matching the isovalues order) as f64 to `out_ptr`.
///
/// # Safety
/// `out_ptr` must point to `cap*8` valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_block_interface_ids(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32 {
    with_model(handle, 0, |m| {
        let Some(l) = m.block_lithology(b as usize) else {
            return 0;
        };
        let n = l.interfaces.len().min(cap as usize);
        let out = std::slice::from_raw_parts_mut(out_ptr, n * 8);
        for (i, iv) in l.interfaces.iter().take(n).enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&(iv.interface_id as f64).to_le_bytes());
        }
        n as u32
    })
}

/// Write the model's REPRESENTATIVE interface isovalues (ascending) — the whole-model
/// set (`Model::lithology`), block 0's for a faulted model — as f64 to `out_ptr`
/// (capacity `cap` f64). Returns the count written. This backs the non-faulted web
/// path's `interface_isovalues()`.
///
/// # Safety
/// `out_ptr` must point to `cap*8` valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_interface_isovalues(handle: u32, out_ptr: *mut u8, cap: u32) -> u32 {
    with_model(handle, 0, |m| {
        let l = m.lithology();
        let n = l.interfaces.len().min(cap as usize);
        let out = std::slice::from_raw_parts_mut(out_ptr, n * 8);
        for (i, iv) in l.interfaces.iter().take(n).enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&iv.isovalue.to_le_bytes());
        }
        n as u32
    })
}

/// Write the model's REPRESENTATIVE interface ids (matching the isovalues order) as f64
/// to `out_ptr`. Returns the count written.
///
/// # Safety
/// `out_ptr` must point to `cap*8` valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_interface_ids(handle: u32, out_ptr: *mut u8, cap: u32) -> u32 {
    with_model(handle, 0, |m| {
        let l = m.lithology();
        let n = l.interfaces.len().min(cap as usize);
        let out = std::slice::from_raw_parts_mut(out_ptr, n * 8);
        for (i, iv) in l.interfaces.iter().take(n).enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(&(iv.interface_id as f64).to_le_bytes());
        }
        n as u32
    })
}

/// Number of representative interfaces (the whole-model set). Lets the host size the
/// `gf_interface_isovalues`/`gf_interface_ids` output buffers.
#[no_mangle]
pub extern "C" fn gf_interface_count(handle: u32) -> u32 {
    with_model(handle, 0, |m| m.lithology().interfaces.len() as u32)
}

/// Write block `b`'s fault half-spaces to `out_ptr` as 7 f64 each
/// (point xyz, normal xyz, keep_positive as 1.0/0.0); capacity `cap` half-spaces.
/// Returns the count written.
///
/// # Safety
/// `out_ptr` must point to `cap*7*8` valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_block_half_spaces(handle: u32, b: u32, out_ptr: *mut u8, cap: u32) -> u32 {
    with_model(handle, 0, |m| {
        let hs = m.block_half_spaces(b as usize);
        let n = hs.len().min(cap as usize);
        let out = std::slice::from_raw_parts_mut(out_ptr, n * 7 * 8);
        for (i, h) in hs.iter().take(n).enumerate() {
            let base = i * 7 * 8;
            let vals = [
                h.point[0], h.point[1], h.point[2],
                h.normal[0], h.normal[1], h.normal[2],
                if h.keep_positive { 1.0 } else { 0.0 },
            ];
            for (j, &val) in vals.iter().enumerate() {
                out[base + j * 8..base + j * 8 + 8].copy_from_slice(&val.to_le_bytes());
            }
        }
        n as u32
    })
}

/// Solve residual / isovalue-spread diagnostics, written as 2 f64 to `out_ptr`.
///
/// # Safety
/// `out_ptr` must point to 16 valid writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gf_diagnostics(handle: u32, out_ptr: *mut u8) {
    let (resid, spread) = with_model(handle, (f64::NAN, f64::NAN), |m| (m.residual, m.isovalue_spread));
    let out = std::slice::from_raw_parts_mut(out_ptr, 16);
    out[0..8].copy_from_slice(&resid.to_le_bytes());
    out[8..16].copy_from_slice(&spread.to_le_bytes());
}

/// Drop the model behind `handle`, freeing its kernel memory.
#[no_mangle]
pub extern "C" fn gf_release(handle: u32) {
    MODELS.with(|s| {
        if let Some(slot) = s.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}
