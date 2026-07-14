//! Feature 004 — a CONFORMABLE STACK of independent imported CONTACT surfaces must form
//! non-overlapping SLAB volumes, not interpenetrating full-height prisms.
//!
//! Regression for the "Erosion over stack (imported meshes)" template. Each imported contact emits a
//! below-prism down to a floor under the domain, so with no subtraction three stacked contacts would
//! each span the whole column below their OWN surface — three overlapping prisms. The fix
//! (`Stack::older_imported_contact_prism`, wired in `mesh_volume_recipe`) makes each conformable
//! contact's below-rock unit subtract the next-older contact's below-prism, turning it into the slab
//! between the two surfaces.
//!
//! The Manifold booleans run on the JS main thread, so the native side is asserted on the RECIPE it
//! emits (as the sibling `imported_mesh_domain_inside`/`imported_mesh_exact` tests do): the younger
//! contacts' below-rock unit must carry a Subtract op (kind 2) referencing an older contact's prism;
//! the oldest must not.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::octree::{OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement};
use geofield::Model;

/// A flat imported sheet at constant `z`, spanning [0,1000]² (outward normal +Z), in the given role.
fn flat_surface(z: f64, role: ImportRole, relation: Relation) -> ImportedSurface<Model> {
    let verts = vec![[0.0, 0.0, z], [1000.0, 0.0, z], [1000.0, 1000.0, z], [0.0, 1000.0, z]];
    let tris = vec![[0usize, 1, 2], [0, 2, 3]];
    let up = [0.0, 0.0, 1.0];
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts.iter().map(|&p| Orientation::new(p, up)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    ImportedSurface { model, mesh: TriMesh::new(verts, tris), role, relation: Some(relation), extend_to_bounds: false }
}

/// A flat imported CONTACT sheet at constant `z`.
fn flat_contact(z: f64) -> ImportedSurface<Model> {
    flat_surface(z, ImportRole::Contact, Relation::Conformable)
}

/// Max Z of a primitive mesh (its top face = the source contact's z, for a flat sheet).
fn top_z(positions: &[f32]) -> f64 {
    positions.chunks_exact(3).map(|p| p[2] as f64).fold(f64::MIN, f64::max)
}

#[test]
fn stacked_imported_contacts_subtract_the_older_sibling() {
    // Youngest-first: contacts at z = 700 / 500 / 300.
    let stack = Stack::new(vec![
        StackElement::ImportedSurface(flat_contact(700.0)),
        StackElement::ImportedSurface(flat_contact(500.0)),
        StackElement::ImportedSurface(flat_contact(300.0)),
    ]);
    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg);

    // Primitive definitions (op 255): id → mesh, to resolve what a Subtract op references.
    let prim_top_z: std::collections::HashMap<u32, f64> = ops
        .iter()
        .filter(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, id, m)| (*id, top_z(&m.positions)))
        .collect();

    // For each element i, the Subtract ops (kind 2) on its below-rock unit (unit 0), as the top-z of
    // the primitive each subtracts.
    let subtract_zs = |i: usize| -> Vec<f64> {
        ops.iter()
            .filter(|(si, _, u, op, _, _)| *si == i && *u == 0 && *op == 2)
            .map(|(_, _, _, _, id, _)| prim_top_z[id])
            .collect::<Vec<_>>()
    };

    // Element 0 (z=700) subtracts the z=500 contact's prism → slab 500..700.
    let s0 = subtract_zs(0);
    assert!(
        s0.iter().any(|&z| (z - 500.0).abs() < 1.0),
        "the z=700 contact's below-rock must subtract the z=500 contact (got tops {s0:?})"
    );
    // Element 1 (z=500) subtracts the z=300 contact's prism → slab 300..500.
    let s1 = subtract_zs(1);
    assert!(
        s1.iter().any(|&z| (z - 300.0).abs() < 1.0),
        "the z=500 contact's below-rock must subtract the z=300 contact (got tops {s1:?})"
    );
    // Element 2 (z=300) is the oldest — no older sibling, so NO slab-subtract (it floors to the
    // domain bottom). (Its unit 0 may still carry younger clip maskers, but none is a contact prism.)
    let s2 = subtract_zs(2);
    assert!(
        !s2.iter().any(|&z| z > 1.0),
        "the oldest contact must not subtract a below-sibling prism (got tops {s2:?})"
    );
}

#[test]
fn imported_erosion_keeps_older_contact_wholly_below_it() {
    // The exact template bug: a bed lying WHOLLY BELOW an imported erosion must be PRESERVED, not
    // wiped. The erode clip is an INTERSECT (kind 1) with the erosion's OWN watertight below-prism
    // (`close_sheet_below`, Manifold-ingestible) — NOT a fitted-field subtract whose level set can
    // engulf the whole lower column, and NOT a fresh mesh-SDF contour Manifold rejects as non-
    // manifold (which dropped every older unit that referenced it). Youngest-first: erosion(600),
    // bed(300).
    let stack = Stack::new(vec![
        StackElement::ImportedSurface(flat_surface(600.0, ImportRole::Erosion, Relation::Erode)),
        StackElement::ImportedSurface(flat_contact(300.0)),
    ]);
    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg);

    // The bed (series 1) unit-0 must carry an INTERSECT (kind 1) referencing a watertight prim whose
    // top is the erosion surface (~z=600) — i.e. "keep older rock below the erosion". A fitted-field
    // subtract (kind 2) engulfing the column, or no clip at all, would be the regression.
    let intersects: Vec<(u32, f64)> = ops
        .iter()
        .filter(|(si, _, u, op, _, _)| *si == 1 && *u == 0 && *op == 1)
        .map(|(_, _, _, _, id, _)| *id)
        .map(|id| {
            let m = ops.iter().find(|(_, _, _, op, pid, _)| *op == 255 && *pid == id).unwrap();
            let top = m.5.positions.chunks_exact(3).map(|p| p[2] as f64).fold(f64::MIN, f64::max);
            let open = {
                let mut inc = std::collections::HashMap::<(u32, u32), u32>::new();
                for t in m.5.indices.chunks_exact(3) {
                    for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        let e = if a < b { (a, b) } else { (b, a) };
                        *inc.entry(e).or_insert(0) += 1;
                    }
                }
                inc.values().filter(|&&c| c == 1).count()
            };
            (open as u32, top)
        })
        .collect();

    assert!(
        intersects.iter().any(|&(_, top)| (top - 600.0).abs() < 5.0),
        "the bed must be INTERSECTED with the erosion's below-prism (top≈600); got {intersects:?}"
    );
    assert!(
        intersects.iter().all(|&(open, _)| open == 0),
        "the erode-clip prim must be watertight (Manifold-ingestible); got open edges {intersects:?}"
    );
}

#[test]
fn stack_units_do_not_span_the_whole_column() {
    // Pins the two fixes that took the domain-partition overlap from ~60% to a thin residual:
    //   (1) EVERY imported surface — erosion included — slab-subtracts the next-older surface, so no
    //       unit's below-prism runs full-height and overlaps the beds under it.
    //   (2) the above-void unit (index 1) is emitted ONLY for a LONE surface; in a stack the space
    //       above the oldest surface is owned by the younger slabs, so its above-void must NOT exist.
    // Template order youngest-first: cover(800) / erosion(600) / bed(300).
    let stack = Stack::new(vec![
        StackElement::ImportedSurface(flat_contact(800.0)),
        StackElement::ImportedSurface(flat_surface(600.0, ImportRole::Erosion, Relation::Erode)),
        StackElement::ImportedSurface(flat_contact(300.0)),
    ]);
    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg);

    // (1) The EROSION (series 1) unit 0 must carry a Subtract (kind 2) of a prim whose top ≈ the
    // next-older contact (z=300) — i.e. it is the slab bed..erosion, not the full below-erosion column.
    let has_subtract_of_top = |si: usize, want_top: f64| {
        ops.iter()
            .filter(|(s, _, u, op, _, _)| *s == si && *u == 0 && *op == 2)
            .any(|(_, _, _, _, id, _)| {
                let m = ops.iter().find(|(_, _, _, op, pid, _)| *op == 255 && *pid == *id).unwrap();
                let top = m.5.positions.chunks_exact(3).map(|p| p[2] as f64).fold(f64::MIN, f64::max);
                (top - want_top).abs() < 5.0
            })
    };
    assert!(
        has_subtract_of_top(1, 300.0),
        "the erosion unit must slab-subtract the next-older contact (top≈300), not run full-height"
    );

    // (2) NO unit index 1 (above-void) anywhere — every surface here has a younger and/or older
    // neighbour, so none is lone.
    let has_above_void = ops.iter().any(|(_, _, u, _, _, _)| *u == 1);
    assert!(!has_above_void, "no imported surface in a stack may emit an above-void unit (index 1)");
}

#[test]
fn lone_imported_surface_keeps_its_above_void() {
    // A LONE imported surface (no younger, no older) DOES keep its above-void unit → Air fills above.
    let stack = Stack::new(vec![StackElement::ImportedSurface(flat_contact(500.0))]);
    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg);
    let has_above_void = ops.iter().any(|(_, _, u, _, _, _)| *u == 1);
    assert!(has_above_void, "a lone imported surface must keep its above-void (index 1) unit");
}
