//! Interface types crossing the geofield kernel↔engine boundary.
//!
//! This crate is the documented contract between `geofield-kernel` (the EUPL
//! cokriging core) and `geofield` (the BSD meshing engine). It contains ONLY the
//! shapes of data that cross the boundary — the [`ScalarField`] oracle trait and
//! plain-old-data structs — and **no computation and no gempy-derived expression**.
//! Both crates depend on this one; neither depends on the other. It is permissively
//! licensed because it is the interface, not the math.

/// A scalar field sampled pointwise: value and analytic gradient at a world point.
///
/// This is the oracle the meshing engine interrogates while contouring. The kernel's
/// `Model` (per-block dispatch) and `BlockField` (one block's smooth field) implement
/// it in-process; across the two-wasm boundary, the engine's `RemoteField` implements
/// it by forwarding to the kernel module's exported `gf_eval` / `gf_eval_grad`.
pub trait ScalarField {
    fn scalar_field(&self, p: [f64; 3]) -> f64;
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3];
}

/// A shared reference to a field is itself a field — so a borrowed model/oracle can be
/// passed wherever a `ScalarField` is expected (e.g. the meshing `Scene`), without the
/// caller owning it. Lets a generic stack hand `&model` to the contourer uniformly.
impl<T: ScalarField + ?Sized> ScalarField for &T {
    fn scalar_field(&self, p: [f64; 3]) -> f64 {
        (**self).scalar_field(p)
    }
    fn scalar_field_gradient(&self, p: [f64; 3]) -> [f64; 3] {
        (**self).scalar_field_gradient(p)
    }
}

/// A plane half-space, as plain geometry — the side of `point`+`normal` to keep.
/// `keep_positive` keeps `(p − point)·normal >= 0`, else the negative side. Produced by
/// the kernel's fault `BlockIndex` to clip a per-block mesh to its region; carried
/// across the boundary as 7 plain numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HalfSpace {
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub keep_positive: bool,
}
