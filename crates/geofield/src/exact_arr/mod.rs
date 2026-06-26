//! Exact-arithmetic mesh corefiner — Stage 0 de-risk spike.
//!
//! Scoped in [`docs/12_exact_corefiner_stage0_scope.md`], specified in
//! [`docs/11_exact_corefiner_spec.md`]. This module is the **additive, default-off** beginning of the
//! exact-construction path that will eventually replace the float-construction corefiner in
//! [`crate::mesh_clip`]. It NEVER edits or replaces `mesh_clip::clip`; the shipping mesher is
//! unchanged until this path is proven (gated by `feature = "exact_arr"`).
//!
//! ## What this increment delivers (docs/12 §2, the determinism kill-test)
//!
//! ONE exact triangle–triangle intersection, end-to-end: f64 triangle pair → exact rational
//! construction → single correctly-rounded f64 snapshot → pinned digest. The digest is bit-identical
//! native↔wasm *by construction* (output bits come from rounding an exact value, not accumulated f64
//! ops), which is the cheapest, most decisive validation of the program's determinism premise.
//!
//! ## Not yet here (later increments)
//!
//!   * The lazy interval/expansion fast-leg (`geometry-predicates`) + the rational-fallback-rate
//!     benchmark (docs/12 §3). This increment is all-exact-rational on purpose — correctness and
//!     determinism first, speed second.
//!   * The pairwise CDT corefiner (Stage 1), the arrangement/classification (Stage 2), coplanar +
//!     degeneracy handling (Stage 3).
//!
//! [`docs/12_exact_corefiner_stage0_scope.md`]: https://example.invalid/docs/12
//! [`docs/11_exact_corefiner_spec.md`]: https://example.invalid/docs/11

mod scalar;
mod tri_tri;

pub use tri_tri::{exact_tri_tri_segment, spike_digest, ExactSeg};
