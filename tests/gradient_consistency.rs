//! Step 7 — gradient self-consistency (BUILD_geofield_rs.md §7).
//!
//! Two checks, both internal (no gempy fixture needed):
//!   1. analytic ∇Z vs central finite differences of Z, on a scatter of points;
//!   2. ∇Z evaluated at each orientation location reproduces its input pole
//!      **direction**. Magnitude reproduction is deferred — it depends on the
//!      RHS pole scaling, the genuine §7.4 open item, and is confirmed only once
//!      the gempy RHS dump is available.
//!
//! A failure in (1) means the analytic derivative is wrong; a failure in (2)
//! means the orientation constraint is not being honored by the field.

use geofield_rs::assemble::{assemble, KernelOptions};
use geofield_rs::data::{Orientation, Series, SurfacePoint};
use geofield_rs::evaluate::Field;
use geofield_rs::solve::solve;

fn dipping_series() -> Series {
    let sps = vec![
        SurfacePoint::new([0.0, 0.0, 1.0], 0),
        SurfacePoint::new([1.0, 0.0, 1.3], 0),
        SurfacePoint::new([0.0, 1.0, 1.05], 0),
    ];
    let oris = vec![
        Orientation::new([0.3, 0.3, 1.1], [-0.3, 0.0, 1.0]),
        Orientation::new([0.7, 0.6, 1.2], [-0.3, 0.0, 1.0]),
    ];
    Series::new(sps, oris)
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}

#[test]
fn analytic_gradient_matches_central_differences() {
    let s = dipping_series();
    let sys = assemble(&s, KernelOptions::default());
    let sol = solve(&sys);
    let field = Field::new(&sys.geom, &sol);

    let h = 1e-6;
    for p in [
        [0.4, 0.4, 1.1],
        [0.25, 0.6, 1.05],
        [0.6, 0.35, 1.2],
        [0.15, 0.15, 1.02],
        [0.8, 0.8, 1.25],
    ] {
        let g = field.scalar_field_gradient(p);
        for e in 0..3 {
            let mut pp = p;
            let mut pm = p;
            pp[e] += h;
            pm[e] -= h;
            let fd = (field.scalar_field(pp) - field.scalar_field(pm)) / (2.0 * h);
            let tol = 1e-4 * (1.0 + g[e].abs());
            assert!(
                (g[e] - fd).abs() < tol,
                "∇Z[{e}] at {p:?}: analytic={} fd={} (diff {:.2e})",
                g[e],
                fd,
                (g[e] - fd).abs()
            );
        }
    }
}

/// On a clean, symmetric single-orientation model the gradient constraint is
/// honored **exactly**: ∇Z at the orientation equals its pole. (With multiple
/// interacting orientations the least-norm interpolant tilts each pole slightly
/// — correct cokriging behavior, not an error — so exact reproduction is only
/// asserted on the isolated-orientation case.)
#[test]
fn gradient_at_isolated_orientation_equals_pole_exactly() {
    let sps = vec![
        SurfacePoint::new([0.0, 0.0, 1.0], 0),
        SurfacePoint::new([2.0, 0.0, 1.0], 0),
    ];
    let ori = Orientation::new([1.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
    let s = Series::new(sps, vec![ori]);
    let sys = assemble(&s, KernelOptions::default());
    let sol = solve(&sys);
    let field = Field::new(&sys.geom, &sol);

    let g = field.scalar_field_gradient(ori.xyz);
    let gd = normalize(g);
    let pd = normalize(ori.pole);
    let cos = gd[0] * pd[0] + gd[1] * pd[1] + gd[2] * pd[2];
    assert!(
        cos > 0.999999,
        "∇Z at isolated orientation not colinear with pole: cos={cos}, ∇Z={g:?}"
    );
}

/// With multiple interacting orientations the field's gradient at each pole is
/// *close* to the input pole but not exact — assert a loose colinearity to catch
/// gross errors while documenting that exact reproduction is not expected here.
/// The gempy reference fixture (step 9) is the authority on the precise tilt.
#[test]
fn gradient_direction_at_orientations_is_approximately_pole() {
    let s = dipping_series();
    let sys = assemble(&s, KernelOptions::default());
    let sol = solve(&sys);
    let field = Field::new(&sys.geom, &sol);

    for o in &s.orientations {
        let g = field.scalar_field_gradient(o.xyz);
        let gd = normalize(g);
        let pd = normalize(o.pole);
        let cos = gd[0] * pd[0] + gd[1] * pd[1] + gd[2] * pd[2];
        assert!(
            cos > 0.99,
            "∇Z at orientation {:?} grossly off pole {:?}: cos={cos}, ∇Z={g:?}",
            o.xyz,
            o.pole
        );
    }
}
