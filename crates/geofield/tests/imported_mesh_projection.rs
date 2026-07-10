//! Feature 004 — US2 T017: an UNDER-COVERING imported mesh is projected out to the domain boundary.
//! The stitched below-contact solid must (INV-2) be watertight AND reach the domain box faces
//! (smooth projection beyond the mesh footprint), while (INV-3) still coinciding with the exact
//! triangles INSIDE the footprint. Drives the real recipe path.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::obj::{normalize_and_check, parse_obj};
use geofield::octree::{Boundary, OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement};
use geofield::Model;

const UNDER: &[u8] = include_bytes!("fixtures/mesh-under-covering.obj");

fn imported_contact(bytes: &[u8]) -> (ImportedSurface<Model>, TriMesh) {
    let obj = parse_obj(bytes).expect("parses");
    let (verts, tris, _h) = normalize_and_check(&obj).expect("meshable");
    let normals = obj.normals.clone().expect("fixture has vn");
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts.iter().zip(&normals).map(|(&p, &n)| Orientation::new(p, n)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    let check = TriMesh::new(verts.clone(), tris.clone());
    (
        ImportedSurface { model, mesh: TriMesh::new(verts, tris), role: ImportRole::Contact, relation: Some(Relation::Conformable), extend_to_bounds: true },
        check,
    )
}

#[test]
fn under_covering_mesh_projects_to_boundary_watertight_and_exact() {
    let (imported, check_mesh) = imported_contact(UNDER); // footprint [200,800]^2, strictly inside
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported)]);

    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg);
    let prim = ops
        .iter()
        .find(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, _, m)| m)
        .expect("primitive emitted");
    assert!(prim.triangle_count() > 0);

    // INV-2a: WATERTIGHT (closed solid — no open, no non-manifold edges).
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in prim.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    assert_eq!(inc.values().filter(|&&c| c > 2).count(), 0, "no non-manifold edges");
    assert_eq!(inc.values().filter(|&&c| c == 1).count(), 0, "watertight (no open edges)");

    // INV-2b: the below-contact surface REACHES the domain footprint edges (the smooth projection
    // extends the bed from [200,800] out toward the box) — there is contact geometry at x well
    // beyond the mesh footprint (e.g. x < 100 and x > 900).
    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    for v in prim.positions.chunks_exact(3) {
        // consider only vertices near the bed height (the contact face, z≈500), not the box floor.
        if (v[2] as f64 - 500.0).abs() < 60.0 {
            min_x = min_x.min(v[0] as f64);
            max_x = max_x.max(v[0] as f64);
        }
    }
    assert!(min_x < 100.0 && max_x > 900.0, "the bed is projected beyond its footprint to the box (x∈[{min_x:.0},{max_x:.0}])");

    // INV-3: INSIDE the footprint, the contact coincides with the EXACT mesh (vertices on the mesh
    // sit at z≈500). Sample the primitive vertices that lie on the imported triangles.
    let mut contact_verts = 0;
    let mut max_dz: f64 = 0.0;
    for v in prim.positions.chunks_exact(3) {
        let p = [v[0] as f64, v[1] as f64, v[2] as f64];
        if check_mesh.surface_distance(p).unwrap_or(f64::MAX) < 2.0 {
            contact_verts += 1;
            max_dz = max_dz.max((p[2] - 500.0).abs());
        }
    }
    assert!(contact_verts > 0 && max_dz < 2.0, "exact inside the footprint: {contact_verts} verts, max |dz| = {max_dz}");
}
