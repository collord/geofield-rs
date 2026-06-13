//! Increment-consistency regression probe (self-consistency guard).
//!
//! ## What this proves
//! In the increment formulation, all surface points of one interface constrain
//! the scalar field to a single common (unknown) value for that interface. So a
//! correctly assembled dual system MUST satisfy `Z(rest) = Z(ref)` — i.e. equal
//! recovered field value across every surface point on an interface — to ~1e-9.
//! This is the *defining property* of the formulation, not a gempy convention,
//! so it holds independently of any reference implementation.
//!
//! ## Why it exists (the finding, 2026-06-13)
//! `KERNEL_MATH.md` §4.1 transcribes the U_SP drift border as
//! `−selector·gi_res·(usp_rest − usp_ref)`. With that `−gi_res` factor the
//! assembled system is internally inconsistent: the solved predictor cannot
//! reproduce `Z(rest) = Z(ref)`, leaving the within-interface spread at
//! `≈ 7e-3`. Switching U_SP to the plain basis difference `(rest[u] − ref[u])`
//! drives the spread to `≈ 1e-9` (machine precision). The production assembler
//! (`src/assemble.rs::assemble_drift_border`) deliberately uses the plain form;
//! see the loud DIVERGENCE comment there. This test pins that decision so no
//! future change (degree-2 drift, multi-series, a border refactor, a different
//! nugget) can silently reintroduce the defect.
//!
//! ## Scope — read before trusting a pass
//! This probe is **invariant to the global scale and level of the field**: it
//! only checks that points on one interface agree with each other, which any
//! affine rescaling of Z preserves. It therefore does NOT prove agreement with
//! gempy's isovalue *magnitudes*. It is a self-consistency guard only. It must
//! never substitute for:
//!   - the step-5 block-by-block diff against gempy's assembled K/border, which
//!     remains the authority on whether the U_SP divergence is gempy-faithful or
//!     merely internally consistent; nor
//!   - the isovalue-magnitude / RHS-scaling check (KERNEL_MATH.md §7.4).
//! A pass here does NOT close the U_SP open item flagged in `assemble.rs`.

use geofield_rs::assemble::{assemble, KernelOptions, System};
use geofield_rs::data::{Orientation, Series, SurfacePoint};
use geofield_rs::evaluate::Field;
use geofield_rs::solve::{solve, Solution};

/// Within-interface spread that a correctly assembled system must stay under.
/// The correct form reaches ≈1e-9; 1e-7 leaves headroom for larger models and
/// conditioning while staying ~5 orders below the 7e-3 defect, so a regression
/// fails loudly. Do NOT loosen this to accommodate a red test — if it goes red
/// the assembly is wrong, not the tolerance.
const TOL_INCREMENT: f64 = 1e-7;

/// A named reference model in the progression. Inputs mirror the geometries
/// `validation/make_fixtures.py` emits (tiny / flat3 / dipping); we build them
/// in-test rather than reading fixtures so the probe runs before any gempy dump
/// exists (and must, per the build plan — it is independent of reference match).
struct Model {
    name: &'static str,
    series: Series,
}

fn models() -> Vec<Model> {
    vec![
        // tiny: 1 interface (2 surface points) + 1 orientation. The step-5 gate
        // model. Multi-point interface, so non-vacuous.
        Model {
            name: "tiny",
            series: Series::new(
                vec![
                    SurfacePoint::new([0.0, 0.0, 0.0], 0),
                    SurfacePoint::new([1.0, 0.0, 0.2], 0),
                ],
                vec![Orientation::new([0.5, 0.0, 0.1], [0.0, 0.0, 1.0])],
            ),
        },
        // flat3: three flat interfaces, each with 3 surface points; one
        // orientation. Exercises multi-interface + multi-point increments.
        Model {
            name: "flat3",
            series: Series::new(
                vec![
                    SurfacePoint::new([0.0, 0.0, 1.0], 0),
                    SurfacePoint::new([1.0, 0.0, 1.0], 0),
                    SurfacePoint::new([0.0, 1.0, 1.0], 0),
                    SurfacePoint::new([0.0, 0.0, 2.0], 1),
                    SurfacePoint::new([1.0, 0.0, 2.0], 1),
                    SurfacePoint::new([0.0, 1.0, 2.0], 1),
                    SurfacePoint::new([0.0, 0.0, 3.0], 2),
                    SurfacePoint::new([1.0, 0.0, 3.0], 2),
                    SurfacePoint::new([0.0, 1.0, 3.0], 2),
                ],
                vec![Orientation::new([0.5, 0.5, 2.0], [0.0, 0.0, 1.0])],
            ),
        },
        // dipping: one interface (4 points) dipping in +x, two matching
        // orientations. Same model used by reference_model.rs.
        Model {
            name: "dipping",
            series: Series::new(
                vec![
                    SurfacePoint::new([0.0, 0.0, 1.0], 0),
                    SurfacePoint::new([1.0, 0.0, 1.3], 0),
                    SurfacePoint::new([0.0, 1.0, 1.05], 0),
                    SurfacePoint::new([1.0, 1.0, 1.35], 0),
                ],
                vec![
                    Orientation::new([0.25, 0.5, 1.1], [-0.3, 0.0, 1.0]),
                    Orientation::new([0.75, 0.5, 1.25], [-0.3, 0.0, 1.0]),
                ],
            ),
        },
    ]
}

/// Group a series' surface-point indices by interface id, in first-seen order.
fn interfaces(series: &Series) -> Vec<(u32, Vec<usize>)> {
    let mut order: Vec<u32> = Vec::new();
    let mut groups: std::collections::BTreeMap<u32, Vec<usize>> = std::collections::BTreeMap::new();
    for (i, sp) in series.surface_points.iter().enumerate() {
        groups
            .entry(sp.interface_id)
            .or_insert_with(|| {
                order.push(sp.interface_id);
                Vec::new()
            })
            .push(i);
    }
    order.into_iter().map(|id| (id, groups.remove(&id).unwrap())).collect()
}

/// For a solved system, the maximum within-interface spread of the recovered
/// scalar field, evaluated at the **normalized** (rescaled) surface-point
/// coordinates the solve ran in — avoiding a world↔normalized round-trip that
/// could mask the effect. Returns (max spread over interfaces, per-interface).
fn within_interface_spreads(series: &Series, sys: &System, sol: &Solution) -> Vec<(u32, f64)> {
    let field = Field::new(&sys.geom, sol);
    interfaces(series)
        .into_iter()
        .map(|(id, idxs)| {
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for i in idxs {
                let xn = sys.geom.transform.apply(series.surface_points[i].xyz);
                let z = field.scalar_field_norm(xn);
                lo = lo.min(z);
                hi = hi.max(z);
            }
            (id, hi - lo)
        })
        .collect()
}

#[test]
fn within_interface_field_is_consistent_across_models() {
    let models = models();

    // Non-vacuity guard: at least one interface across the suite must have >= 2
    // surface points, or the property holds trivially and proves nothing.
    let max_interface_points = models
        .iter()
        .flat_map(|m| interfaces(&m.series).into_iter().map(|(_, v)| v.len()))
        .max()
        .unwrap_or(0);
    assert!(
        max_interface_points >= 2,
        "probe is vacuous: no interface in the suite has >= 2 surface points"
    );

    for m in &models {
        let sys = assemble(&m.series, KernelOptions::default());
        let sol = solve(&sys);
        assert!(
            sol.residual < 1e-9,
            "[{}] solve residual {} too large",
            m.name,
            sol.residual
        );

        let spreads = within_interface_spreads(&m.series, &sys, &sol);
        for (id, spread) in &spreads {
            let n_pts = interfaces(&m.series)
                .into_iter()
                .find(|(g, _)| g == id)
                .map(|(_, v)| v.len())
                .unwrap_or(0);
            eprintln!(
                "[{}] interface {id}: {n_pts} pts, within-interface spread = {spread:.3e}",
                m.name
            );
            assert!(
                *spread <= TOL_INCREMENT,
                "[{}] interface {id} ({n_pts} pts): within-interface spread {spread:e} \
                 exceeds TOL_INCREMENT {TOL_INCREMENT:e} — the dual system does not \
                 reproduce Z(rest)=Z(ref). The assembly is wrong (suspect the U_SP \
                 drift border; see src/assemble.rs DIVERGENCE note). Do NOT loosen \
                 the tolerance.",
                m.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// NEGATIVE CONTROL — regression lock on the probe itself.
//
// Re-assembles the dipping model, then overwrites ONLY the U_SP drift border
// with the REJECTED `−gi_res·(rest[u] − ref[u])` form from KERNEL_MATH.md §4.1,
// re-solves, and asserts the within-interface spread blows up (> 1e-4). This
// proves the probe can actually DETECT the defect — so a future refactor cannot
// quietly neuter the positive test into always-passing. The rejected form lives
// only here, never in the production assembly path.
// ---------------------------------------------------------------------------

/// Overwrite the U_SP border (SP-row ↔ drift-col block, both symmetric halves)
/// of an already-assembled system with the rejected `−gi_res`-scaled form.
fn apply_rejected_usp(sys: &mut System) {
    let gi_res = sys.geom.opts.gi_res;
    let dim = sys.dim;
    for k in 0..sys.geom.n_sp_inc {
        let (rest, refp) = sys.geom.sp_increments[k];
        let si = sys.geom.sp_index(k);
        for u in 0..3 {
            let ui = sys.geom.uni_index(u);
            let v = -gi_res * (rest[u] - refp[u]); // the rejected §4.1 form
            sys.k[si * dim + ui] = v;
            sys.k[ui * dim + si] = v;
        }
    }
}

#[test]
fn negative_control_rejected_usp_breaks_consistency() {
    // Dipping model (multi-point interface) — same geometry as the positive test.
    let series = models()
        .into_iter()
        .find(|m| m.name == "dipping")
        .unwrap()
        .series;

    let mut sys = assemble(&series, KernelOptions::default());
    apply_rejected_usp(&mut sys); // inject the defect
    let sol = solve(&sys);

    let spreads = within_interface_spreads(&series, &sys, &sol);
    let max_spread = spreads.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    eprintln!("negative control (rejected −gi_res U_SP): max spread = {max_spread:.3e}");

    assert!(
        max_spread > 1e-4,
        "NEGATIVE CONTROL FAILED: the rejected −gi_res U_SP form should break \
         increment consistency (spread > 1e-4) but the spread was only {max_spread:e}. \
         The probe can no longer detect the defect it was written to catch — \
         investigate before trusting the positive test."
    );
}
