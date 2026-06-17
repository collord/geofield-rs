//! Increment-consistency regression probe (necessary guard — NOT a correctness
//! oracle).
//!
//! ## What this proves — and the hard lesson about what it does NOT
//! In the increment formulation, all surface points of one interface constrain
//! the scalar field toward a common value for that interface, so a sane dual
//! system keeps the within-interface field spread small. This probe checks that.
//!
//! ⚠ FOUND INSUFFICIENT-ALONE (the U_SP/RHS misdiagnosis, 2026-06-13). An earlier
//! session used this probe to *validate* a sign/scale fix. That was the precise
//! error it now warns against. Two bugs — a `1/scale`-scaled RHS and a `−gi_res`-
//! dropped U_SP border — were **mutually compensating**: together they produced a
//! field that was a *valid potential* (smooth, solvable, self-consistent: spread
//! ≈ 1e-9) but was NOT gempy's potential. The probe passed at 1e-9 *because the
//! system was wrong in a self-cancelling way*. Only the block diff against gempy
//! (`validation/dump_kernel.py`) localized it.
//!
//! The corrected, gempy-faithful assembly (raw-pole RHS, `−gi_res` U_SP per §4.1,
//! single-axis CG_SP, nuggets inside c_o) has within-interface spread on the
//! order of `~2e-6` — *larger* than the wrong system's artificial 1e-9. That is
//! the tell: a tiny non-flatness is the real field's behavior; near-perfect
//! flatness was the symptom of the compensating-error field. So this probe is
//! demoted to one NECESSARY condition among several. The AUTHORITIES are:
//!   - `validation/dump_kernel.py` — block diff vs gempy's K/b (assembly);
//!   - `validation/run_model1_validation.py` — field/litho diff vs gempy (eval).
//! Do NOT use a pass here to conclude a sign/scale convention is correct.
//! See the cross-referenced history note in `src/assemble.rs`.

use geofield_kernel::assemble::{assemble, KernelOptions, System};
use geofield_kernel::data::{Orientation, Series, SurfacePoint};
use geofield_kernel::evaluate::Field;
use geofield_kernel::solve::{solve, Solution};

/// Within-interface spread the corrected (gempy-faithful) system stays under.
/// Calibrated to the CORRECT field's behavior (~2e-6 on the tiny model), with
/// headroom — NOT to the wrong system's artificial 1e-9. `1e-4` catches the
/// `~7e-3` gross-defect class while admitting the real field's small spread.
/// Tightening this toward 1e-9 would (perversely) re-prefer the compensating-
/// error field; do not.
const TOL_INCREMENT: f64 = 1e-4;

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
// NEGATIVE CONTROL — INVERTED (regression lock on the U_SP border convention).
//
// The original control asserted the `−gi_res` U_SP form *broke* the increment
// probe — i.e. it locked in the WRONG (step-7) fix. The block diff proved the
// opposite: gempy uses `−gi_res·(rest − ref)` (§4.1) and that is correct; the
// plain `(rest − ref)` form (the step-7 fix) is the defect.
//
// So this control is inverted: it pins that the production assembler emits
// gempy's `−gi_res·(rest − ref)` U_SP border, and that the rejected plain
// `(rest − ref)` form would DIFFER from it. This keeps the regression lock on
// the invariant that was actually in question — U_SP must match gempy's border,
// which `validation/dump_kernel.py` confirmed to ratio 1.0 / max|Δ|=0. The
// rejected form lives only here, never in the production path.
// ---------------------------------------------------------------------------

#[test]
fn negative_control_usp_border_is_gempy_minus_gi_res_form() {
    let series = models()
        .into_iter()
        .find(|m| m.name == "dipping")
        .unwrap()
        .series;

    let sys = assemble(&series, KernelOptions::default());
    let gi_res = sys.geom.opts.gi_res;

    let mut any_nonzero = false;
    for k in 0..sys.geom.n_sp_inc {
        let (rest, refp) = sys.geom.sp_increments[k];
        let si = sys.geom.sp_index(k);
        for u in 0..3 {
            let ui = sys.geom.uni_index(u);
            let produced = sys.at(si, ui);
            let gempy_form = -gi_res * (rest[u] - refp[u]); // §4.1, the correct one
            let rejected_form = rest[u] - refp[u]; // the plain step-7 fix
            if gempy_form.abs() > 1e-12 {
                any_nonzero = true;
            }

            // Production must emit gempy's −gi_res form, bit-for-bit.
            assert!(
                (produced - gempy_form).abs() < 1e-12,
                "U_SP[{si},{ui}] = {produced} != gempy −gi_res form {gempy_form}. \
                 The drift border must match gempy (validation/dump_kernel.py is \
                 the authority); do not revert to the plain step-7 form."
            );
            // And where the entry is nonzero it must NOT equal the rejected plain
            // form (gi_res = 2 ≠ 1, so they differ wherever rest != ref — the lock
            // that catches a re-regression). They legitimately coincide at 0 when
            // rest[u] == ref[u], so only check nonzero entries.
            if gempy_form.abs() > 1e-12 {
                assert!(
                    (produced - rejected_form).abs() > 1e-12,
                    "U_SP[{si},{ui}] equals the REJECTED plain (rest−ref) form — the \
                     step-7 misdiagnosis has been reintroduced. See assemble.rs history."
                );
            }
        }
    }
    assert!(any_nonzero, "control vacuous: all U_SP entries zero");
}
