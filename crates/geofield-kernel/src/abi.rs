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
use crate::{KernelFunction, KernelOptions, Model, ScalarField};
use std::cell::RefCell;

// ---------------------------------------------------------------------------
// Handle slab — fitted Models live here, in the kernel's own memory.
// (wasm is single-threaded, so a thread-local RefCell is the simplest store.)
// ---------------------------------------------------------------------------

thread_local! {
    static MODELS: RefCell<Vec<Option<Model>>> = const { RefCell::new(Vec::new()) };
}

const WHOLE_MODEL: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// Host diagnostic sink — a GENERIC kernel→host message channel.
//
// The kernel cdylib otherwise imports nothing, but a kernel-side event (a panic, a
// singular-fit warning, future telemetry) is invisible to JS: a wasm trap surfaces only
// as an opaque `RuntimeError: unreachable` with no message (review 4.1). This one import
// lets the kernel hand the host a UTF-8 message + a level; the host reads the bytes out of
// KERNEL memory (the same copy-out the buffer-returning exports already do) and routes it
// (console.info/warn/error, and error-level onward to the app UI). The panic hook is just
// the first caller — the channel is deliberately not panic-specific.
//
// level: 0 = info, 1 = warn, 2 = error. On native (tests/CLI) there is no host, so the
// import is stubbed to a no-op and the message is written to stderr instead.
#[cfg(all(feature = "wasm-abi", target_arch = "wasm32"))]
#[link(wasm_import_module = "kernel_host")]
extern "C" {
    /// Host diagnostic sink. `ptr`/`len` point at UTF-8 bytes in KERNEL linear memory.
    fn kernel_log(level: u32, ptr: *const u8, len: usize);
}

#[cfg(not(all(feature = "wasm-abi", target_arch = "wasm32")))]
unsafe fn kernel_log(level: u32, ptr: *const u8, len: usize) {
    // Native fallback: no host to forward to — write to stderr so tests/CLI still see it.
    let bytes = std::slice::from_raw_parts(ptr, len);
    if let Ok(s) = std::str::from_utf8(bytes) {
        eprintln!("[kernel level={level}] {s}");
    }
}

/// Forward a diagnostic message to the host sink (safe wrapper).
pub(crate) fn host_log(level: u32, msg: &str) {
    let bytes = msg.as_bytes();
    // SAFETY: `bytes` is a live UTF-8 slice in this (kernel) memory; the host copies it out
    // synchronously and does not retain the pointer.
    unsafe { kernel_log(level, bytes.as_ptr(), bytes.len()) };
}

/// Install a panic hook ONCE that forwards the panic message to the host sink at error level,
/// so a kernel panic reaches the browser/app instead of only an opaque `unreachable` trap.
/// NOTE: this improves the *message*; it does NOT un-poison the trapped instance — the actual
/// trap paths are prevented upstream (non-finite `.obj` rejection, NaN-safe comparators).
pub(crate) fn install_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            host_log(2, &format!("kernel panic: {info}"));
        }));
    });
}

// Optional boundary-crossing instrumentation (feature = "wasm-abi-stats"). Counts every
// gf_eval / gf_eval_grad — the per-sample wasm-module-boundary calls the octree makes —
// so a scale harness can report how many crossings a mesh costs.
#[cfg(feature = "wasm-abi-stats")]
thread_local! {
    static EVAL_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[inline(always)]
fn bump_eval_count() {
    #[cfg(feature = "wasm-abi-stats")]
    EVAL_COUNT.with(|c| c.set(c.get() + 1));
}

/// Total field evals (gf_eval + gf_eval_grad) since the last reset. Always present so the
/// host can call it unconditionally; returns 0 when stats are disabled.
#[no_mangle]
pub extern "C" fn gf_eval_count() -> u64 {
    #[cfg(feature = "wasm-abi-stats")]
    {
        return EVAL_COUNT.with(|c| c.get());
    }
    #[cfg(not(feature = "wasm-abi-stats"))]
    0
}

/// Reset the field-eval counter to 0 (no-op when stats are disabled).
#[no_mangle]
pub extern "C" fn gf_eval_count_reset() {
    #[cfg(feature = "wasm-abi-stats")]
    EVAL_COUNT.with(|c| c.set(0));
}

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

/// The allocation layout `gf_alloc`/`gf_free` agree on for a `len`-byte host buffer. Align 8 covers
/// the f64 payload the host stages. Returns `None` for `len == 0` (no allocation) or a `len` that
/// overflows when aligned.
fn buffer_layout(len: usize) -> Option<std::alloc::Layout> {
    if len == 0 {
        return None;
    }
    std::alloc::Layout::from_size_align(len, 8).ok()
}

/// Allocate `len` bytes in kernel memory; returns the pointer (host writes inputs here).
///
/// Uses a fixed [`std::alloc::Layout`] so `gf_free` deallocates with the *identical* layout. The
/// former `Vec::with_capacity(len)` + `from_raw_parts(ptr, 0, len)` freed with the requested length
/// as the capacity, which is UB whenever the allocator over-allocates (code_review §4.4).
#[no_mangle]
pub extern "C" fn gf_alloc(len: usize) -> *mut u8 {
    match buffer_layout(len) {
        // A zero-length buffer needs no allocation; hand back a well-aligned dangling non-null ptr
        // (the host must not read/write it), which `gf_free` treats as a no-op.
        None => std::ptr::NonNull::<u64>::dangling().as_ptr() as *mut u8,
        // SAFETY: `layout` has non-zero size.
        Some(layout) => unsafe { std::alloc::alloc(layout) },
    }
}

/// Free a buffer previously returned by `gf_alloc`.
///
/// # Safety
/// `ptr`/`len` must be exactly a prior `gf_alloc(len)` result, freed once.
#[no_mangle]
pub unsafe extern "C" fn gf_free(ptr: *mut u8, len: usize) {
    if let Some(layout) = buffer_layout(len) {
        if !ptr.is_null() {
            std::alloc::dealloc(ptr, layout);
        }
    }
    // len == 0 → the dangling ptr from `gf_alloc`; nothing to free.
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
//    decimate_on, seed_cell, threshold_frac, point_budget, kernel,
//    sp_xyz (3*n_sp), sp_if (n_sp), ori_xyz (3*n_ori), ori_pole (3*n_ori)]
// (everything as f64 — ids/degree/budget are small integers carried losslessly). A
// trailing optional fault plane (6 f64: point xyz + normal xyz) is read by
// `gf_fit_faulted`. `decimate_on` ≠ 0 enables the coreset; NaN seed_cell/threshold use
// the DecimateConfig defaults. `kernel` is the `KernelFunction::code` (0 cubic, 1 matern_5_2,
// 2 exponential); NaN → the cubic default.
// ---------------------------------------------------------------------------

/// Header length in f64 scalars (everything before `sp_xyz`). The engine's encoder
/// (`geofield::wasm::build_fit_buffer`) writes exactly this many.
const HEADER: usize = 14;

struct FitInput {
    series: Series,
    opts: KernelOptions,
    consumed: usize, // f64s consumed (so the fault plane can follow)
}

/// A well-defined empty (data-less) fit for a malformed/short input buffer — `Model::fit` of an
/// empty series yields a "no contact" model (field `+∞`, unit 0) the host handles, rather than an
/// OOB read (review 4.3). `consumed = HEADER` so the faulted path's trailing-plane read is also bounded.
fn empty_fit() -> FitInput {
    FitInput {
        series: Series::new(Vec::new(), Vec::new()),
        opts: KernelOptions::default(),
        consumed: HEADER,
    }
}

/// The header's kernel slot → [`KernelFunction`]. NaN (the host's "default") → cubic. An
/// unknown code also degrades to cubic rather than trapping (like `uni_degree`, 4.6), but
/// is logged: silently fitting a different kernel than the one asked for must be visible.
fn decode_kernel(x: f64) -> KernelFunction {
    if x.is_nan() {
        return KernelFunction::default();
    }
    match (x >= 0.0 && x.fract() == 0.0).then(|| KernelFunction::from_code(x as u32)).flatten() {
        Some(k) => k,
        None => {
            host_log(2, "fit: unknown kernel code; using the cubic default");
            KernelFunction::default()
        }
    }
}

/// # Safety
/// `ptr` must point to a valid host-staged f64 buffer of at least the encoded length.
unsafe fn decode_fit(ptr: *const u8, len_f64: usize) -> FitInput {
    // The header must be present before we can read the point/orientation counts.
    if len_f64 < HEADER {
        return empty_fit();
    }
    let v = read_f64s(ptr, len_f64);
    // `n_sp`/`n_ori` are attacker/host-controlled f64s (NaN→0, huge→usize::MAX). Validate that the
    // encoded body actually fits in the staged buffer BEFORE indexing `v`, or a length desync reads
    // OOB kernel memory and aborts (review 4.3). On any mismatch, decode an empty (data-less) fit —
    // a well-defined "no contact" model the host already handles — instead of trapping.
    if !v[0].is_finite() || !v[1].is_finite() || v[0] < 0.0 || v[1] < 0.0 {
        return empty_fit();
    }
    let n_sp = v[0] as usize;
    let n_ori = v[1] as usize;
    // HEADER + 4·n_sp (xyz + interface) + 6·n_ori (xyz + pole), guarding overflow.
    let body = n_sp
        .checked_mul(4)
        .and_then(|a| n_ori.checked_mul(6).and_then(|b| a.checked_add(b)))
        .and_then(|ab| ab.checked_add(HEADER));
    let need = match body {
        Some(need) if need <= len_f64 => need,
        _ => return empty_fit(),
    };
    // Reject a non-finite coordinate / interface id / orientation at the ABI edge — the same
    // guard the `.obj` ingress applies (`ObjError::NonFinite`). A NaN/inf datum otherwise flows
    // into the covariance eigensolve and the kriging system and fits a garbage field; decoding to
    // an empty (data-less) fit and warning is loud and attributable at input time, rather than
    // relying on the downstream field going non-finite (code_review §4.6 residual). `need` bounds
    // the body; the header scalars were validated above.
    if v[HEADER..need].iter().any(|x| !x.is_finite()) {
        host_log(2, "fit input rejected: non-finite coordinate or orientation");
        return empty_fit();
    }
    let d = KernelOptions::default();
    let or = |x: f64, dflt: f64| if x.is_nan() { dflt } else { x };
    let dd = crate::assemble::DecimateConfig::default();
    let decimate = if v[9] != 0.0 && !v[9].is_nan() {
        Some(crate::assemble::DecimateConfig {
            seed_cell: or(v[10], dd.seed_cell),
            threshold_frac: or(v[11], dd.threshold_frac),
            point_budget: if v[12].is_nan() || v[12] < 0.0 { dd.point_budget } else { v[12] as usize },
            max_iters: dd.max_iters,
        })
    } else {
        None
    };
    let opts = KernelOptions {
        range: or(v[2], d.range),
        c_o: or(v[3], d.c_o),
        i_res: or(v[4], d.i_res),
        gi_res: or(v[5], d.gi_res),
        // Only degree-1 drift is implemented; `assemble` asserts it. Clamp any other value at the
        // ABI edge so a stray host arg degrades to the default instead of trapping the kernel (4.6).
        uni_degree: if v[6] as u32 == 1 { 1 } else { d.uni_degree },
        kernel: decode_kernel(v[13]),
        grad_nugget: or(v[7], d.grad_nugget),
        sp_nugget: or(v[8], d.sp_nugget),
        decimate,
    };
    let mut off = HEADER;
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

/// Store a fitted model, warning the host if its solve did not converge. A non-finite residual
/// means a singular/degenerate system (the field is NaN and meshes to nothing) — the host also
/// detects this via `gf_diagnostics`/`WasmStack.residual()`, but forwarding it through the generic
/// sink surfaces a readable error at fit time (review 4.2) rather than a silent empty render.
fn store_fitted(m: Model) -> u32 {
    if !m.fit_is_finite() {
        host_log(2, "fit failed: non-finite field (singular/degenerate surface)");
    }
    put_model(m)
}

/// Fit a non-faulted model from a host-staged f64 buffer (`len_f64` f64 values).
/// Returns a model handle.
///
/// # Safety
/// `ptr` must point to `len_f64*8` valid bytes staged by the host.
#[no_mangle]
pub unsafe extern "C" fn gf_fit(ptr: *const u8, len_f64: usize) -> u32 {
    install_panic_hook();
    let fi = decode_fit(ptr, len_f64);
    store_fitted(Model::fit(&fi.series, fi.opts))
}

/// Fit a single-planar-fault model. Same buffer as `gf_fit`, with 6 trailing f64
/// (fault point xyz + normal xyz) consumed after the series. A degenerate (~0) normal
/// falls back to a non-faulted fit.
///
/// # Safety
/// `ptr` must point to `len_f64*8` valid bytes staged by the host.
#[no_mangle]
pub unsafe extern "C" fn gf_fit_faulted(ptr: *const u8, len_f64: usize) -> u32 {
    install_panic_hook();
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
    store_fitted(model)
}

/// THE field oracle (hot path). `block == WHOLE_MODEL` (u32::MAX) queries the whole
/// faulted model (per-block dispatch); otherwise it queries block `block`'s smooth
/// field. Returns `+∞` for a stale handle / field-less block.
#[no_mangle]
pub extern "C" fn gf_eval(handle: u32, block: u32, x: f64, y: f64, z: f64) -> f64 {
    bump_eval_count();
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
    bump_eval_count();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage `vals` as a little-endian f64 buffer and run `decode_fit` over it, returning the
    /// decoded series' point/orientation counts. `len_override` lets a test claim a length that
    /// disagrees with the header counts (the OOB desync the bounds check must catch).
    fn decode_counts(vals: &[f64], len_override: Option<usize>) -> (usize, usize) {
        let bytes: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let len = len_override.unwrap_or(vals.len());
        // SAFETY: `bytes` outlives the call; `len` is bounded by the test.
        let fi = unsafe { decode_fit(bytes.as_ptr(), len) };
        (fi.series.surface_points.len(), fi.series.orientations.len())
    }

    #[test]
    fn decode_fit_rejects_short_and_mismatched_buffers() {
        // A buffer shorter than the header → empty (data-less) fit, no OOB.
        assert_eq!(decode_counts(&[0.0; 5], None), (0, 0));

        // Header claims 100 surface points but the body is not there. Without the bounds check
        // this indexes `v[HEADER + ...]` far past the end → OOB abort. With it → empty fit.
        let mut v = vec![0.0; HEADER];
        v[0] = 100.0; // n_sp
        v[1] = 0.0; // n_ori
        assert_eq!(decode_counts(&v, None), (0, 0), "header-count desync yields empty, not OOB");

        // NaN / negative counts must not become huge usizes.
        let mut nanv = vec![0.0; HEADER];
        nanv[0] = f64::NAN;
        assert_eq!(decode_counts(&nanv, None), (0, 0), "NaN count → empty");
        let mut negv = vec![0.0; HEADER];
        negv[1] = -1.0;
        assert_eq!(decode_counts(&negv, None), (0, 0), "negative count → empty");
    }

    #[test]
    fn gf_alloc_free_roundtrips_various_sizes() {
        // Exercises the exact alloc/free layout contract natively (the wasm host path isn't in this
        // suite). Writing the whole buffer proves at least `len` usable bytes; freeing with the same
        // `len` must match the allocation layout (no capacity-mismatch UB).
        for &len in &[0usize, 1, 7, 8, 64, 4096] {
            let p = gf_alloc(len);
            if len != 0 {
                assert!(!p.is_null(), "gf_alloc({len}) returned null");
                // SAFETY: `p` points to `len` writable bytes just allocated.
                unsafe { std::ptr::write_bytes(p, 0xAB, len) };
            }
            // SAFETY: `p`/`len` are exactly this `gf_alloc(len)` result, freed once.
            unsafe { gf_free(p, len) };
        }
    }

    #[test]
    fn decode_fit_rejects_non_finite_coordinates() {
        // A well-formed-LENGTH buffer (1 sp + 1 ori) but with a NaN surface-point coordinate must
        // decode to an empty fit — the NaN must never reach the eigensolve / kriging system.
        let mut v = vec![0.0; HEADER + 4 + 6];
        v[0] = 1.0; // n_sp
        v[1] = 1.0; // n_ori
        v[6] = 1.0; // uni_degree
        v[HEADER + 0] = f64::NAN; // sp x
        v[HEADER + 1] = 2.0;
        v[HEADER + 2] = 3.0;
        v[HEADER + 9] = 1.0; // pole z
        assert_eq!(decode_counts(&v, None), (0, 0), "NaN coordinate → empty fit, no panic");

        // An inf orientation pole is likewise rejected.
        let mut w = vec![0.0; HEADER + 4 + 6];
        w[0] = 1.0;
        w[1] = 1.0;
        w[6] = 1.0;
        w[HEADER + 0] = 1.0;
        w[HEADER + 1] = 2.0;
        w[HEADER + 2] = 3.0;
        w[HEADER + 7] = f64::INFINITY; // pole x
        assert_eq!(decode_counts(&w, None), (0, 0), "inf pole → empty fit");
    }

    #[test]
    fn decode_fit_accepts_a_well_formed_buffer() {
        // HEADER + 1 surface point (3 xyz + 1 interface = 4) + 1 orientation (3 xyz + 3 pole = 6).
        let mut v = vec![0.0; HEADER + 4 + 6];
        v[0] = 1.0; // n_sp
        v[1] = 1.0; // n_ori
        v[6] = 1.0; // uni_degree
                    // sp xyz at H..H+3, sp interface at H+3, ori xyz at H+4..H+7, ori pole at H+7..H+10
        v[HEADER + 0] = 1.0;
        v[HEADER + 1] = 2.0;
        v[HEADER + 2] = 3.0;
        v[HEADER + 3] = 0.0; // interface id
        v[HEADER + 7] = 0.0; // pole x
        v[HEADER + 8] = 0.0; // pole y
        v[HEADER + 9] = 1.0; // pole z
        assert_eq!(decode_counts(&v, None), (1, 1));
    }

    #[test]
    fn decode_kernel_maps_codes_and_defaults_to_cubic() {
        assert_eq!(decode_kernel(f64::NAN), KernelFunction::Cubic, "NaN → default");
        for k in KernelFunction::ALL {
            assert_eq!(decode_kernel(k.code() as f64), k);
        }
        // Unknown / non-integer / negative codes degrade to the default (logged), never trap.
        for bad in [3.0, 1.5, -1.0, f64::INFINITY] {
            assert_eq!(decode_kernel(bad), KernelFunction::Cubic, "code {bad}");
        }
        // The header slot reaches the decoded options.
        let mut v = vec![0.0; HEADER + 4 + 6];
        v[0] = 1.0;
        v[1] = 1.0;
        v[6] = 1.0;
        v[13] = KernelFunction::Matern52.code() as f64;
        v[HEADER + 9] = 1.0; // pole z
        let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        // SAFETY: `bytes` outlives the call and holds `v.len()` f64s.
        let fi = unsafe { decode_fit(bytes.as_ptr(), v.len()) };
        assert_eq!(fi.opts.kernel, KernelFunction::Matern52);
    }
}
