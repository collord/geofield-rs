//! geofield-rs — universal-cokriging scalar field for a single, non-faulted series.
//!
//! Math source of truth: `KERNEL_MATH.md` in the repo root. Where this code and
//! that document disagree on a formula, sign, constant, or epsilon, the document
//! wins.
//!
//! Build is staged (see `BUILD_geofield_rs.md`). Currently only the cubic kernel
//! triple (step 1) is implemented.

pub mod assemble;
pub mod data;
pub mod geometry;
pub mod kernel;
pub mod solve;
