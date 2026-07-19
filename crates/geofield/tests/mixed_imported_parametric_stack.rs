//! A conformable stack with element types INTERLEAVED — imported .obj contacts and fitted
//! parametric series — must form non-overlapping SLAB volumes and classify points to the
//! stratigraphically correct element, exactly as a same-type stack does.
//!
//! Regression for the `conformal-stack-mixed` web3 template (measured 60.8% domain overlap): the
//! slab-floor machinery was SAME-TYPE-ONLY. The imported arm's floor scan matched only older
//! `ImportedSurface` siblings (skipping a parametric series between two imported contacts →
//! flooring on the wrong, deeper contact), and a parametric series' oldest unit had no cross-
//! element floor at all (flooding full-height to the domain bottom). The fix is the unified floor
//! oracle (`Stack::floor_element` / `floor_prim` / `below_floor`): ONE next-older-surface
//! selection, rendered per source — an imported floor as its exact `close_sheet_below` prism, a
//! fitted floor as `below_solid(field, iso_top)` — consumed by both the solid recipe and
//! `global_unit`.
//!
//! As in the sibling `imported_mesh_stack` tests, the native side asserts on the RECIPE (the
//! Manifold booleans run on the JS main thread) plus `global_unit` ground truth up a z-column.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::octree::{OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement, StackedSeries};
use geofield::Model;

/// A flat imported CONTACT sheet at constant `z`, spanning [0,1000]² (outward normal +Z).
fn flat_imported_contact(z: f64) -> ImportedSurface<Model> {
    let verts = vec![[0.0, 0.0, z], [1000.0, 0.0, z], [1000.0, 1000.0, z], [0.0, 1000.0, z]];
    let tris = vec![[0usize, 1, 2], [0, 2, 3]];
    let up = [0.0, 0.0, 1.0];
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts.iter().map(|&p| Orientation::new(p, up)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    ImportedSurface {
        model,
        mesh: TriMesh::new(verts, tris),
        role: ImportRole::Contact,
        relation: Some(Relation::Conformable),
        extend_to_bounds: false,
    }
}

/// A flat fitted (parametric) conformable series with its single contact at constant `z`.
fn flat_parametric_series(z: f64) -> StackedSeries<Model> {
    let mut sps = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z], 0));
        }
    }
    let oris = vec![Orientation::new([500.0, 500.0, z], [0.0, 0.0, 1.0])];
    StackedSeries::new(Model::fit(&Series::new(sps, oris), KernelOptions::default()), Relation::Conformable)
}

/// Youngest-first: imported(700) / parametric(500) / imported(300) — every stacking boundary
/// crosses an element-type change, the exact interleave the mixed template exercises.
fn mixed_stack() -> Stack<Model> {
    Stack::new(vec![
        StackElement::ImportedSurface(flat_imported_contact(700.0)),
        StackElement::Series(flat_parametric_series(500.0)),
        StackElement::ImportedSurface(flat_imported_contact(300.0)),
    ])
}

fn cfg() -> OctreeConfig {
    OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true }
}

/// Max Z of a primitive mesh (its top face = the source contact's z, for a flat sheet).
fn top_z(positions: &[f32]) -> f64 {
    positions.chunks_exact(3).map(|p| p[2] as f64).fold(f64::MIN, f64::max)
}

#[test]
fn mixed_stack_floors_across_element_types() {
    let stack = mixed_stack();
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg());

    let prim_top_z: std::collections::HashMap<u32, f64> = ops
        .iter()
        .filter(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, id, m)| (*id, top_z(&m.positions)))
        .collect();
    let subtract_zs = |i: usize| -> Vec<f64> {
        ops.iter()
            .filter(|(si, _, u, op, _, _)| *si == i && *u == 0 && *op == 2)
            .map(|(_, _, _, _, id, _)| prim_top_z[id])
            .collect::<Vec<_>>()
    };

    // The imported z=700 contact must floor on the PARAMETRIC z=500 series (a `below_solid`
    // contour, so its top carries up to a cell of discretization) — NOT skip it to the deeper
    // z=300 imported contact.
    let s0 = subtract_zs(0);
    assert!(
        s0.iter().any(|&z| (z - 500.0).abs() < 20.0),
        "the imported z=700 contact must slab-subtract the parametric z=500 floor (got tops {s0:?})"
    );
    assert!(
        !s0.iter().any(|&z| (z - 300.0).abs() < 20.0),
        "the imported z=700 contact must NOT skip the parametric series and floor on z=300 (got tops {s0:?})"
    );

    // The parametric z=500 series' oldest unit must floor on the IMPORTED z=300 contact (an exact
    // `close_sheet_below` prism, top exactly 300).
    let s1 = subtract_zs(1);
    assert!(
        s1.iter().any(|&z| (z - 300.0).abs() < 1.0),
        "the parametric z=500 series must slab-subtract the imported z=300 floor (got tops {s1:?})"
    );

    // The oldest element floors to the domain bottom — no slab-subtract.
    let s2 = subtract_zs(2);
    assert!(
        !s2.iter().any(|&z| z > 1.0),
        "the oldest contact must not subtract a below-sibling prism (got tops {s2:?})"
    );

    // No above-void units in a stack: the space above each surface is owned by the younger slabs.
    let has_above_void = ops.iter().any(|(_, _, u, _, _, _)| *u == 1);
    assert!(!has_above_void, "no element in a stack may emit an above-void unit (index 1)");
}

#[test]
fn mixed_stack_global_unit_column_is_stratigraphic() {
    // Ground truth up the z-column at plan center: each element claims ONLY its slab — the
    // youngest-present-wins claim is bounded below by the floor surface, so a deep point is NOT
    // swallowed by the youngest element (whose raw `present_at` is true everywhere below it).
    let stack = mixed_stack();
    let col = |z: f64| stack.global_unit([500.0, 500.0, z]).0;
    assert_eq!(col(200.0), 2, "z=200 lies in the oldest (z=300 imported) element's slab");
    assert_eq!(col(400.0), 1, "z=400 lies in the parametric (z=500) series' slab");
    assert_eq!(col(600.0), 0, "z=600 lies in the youngest (z=700 imported) element's slab");
    assert_eq!(col(800.0), 3, "z=800 is above every surface → Air (element-count sentinel)");
}
