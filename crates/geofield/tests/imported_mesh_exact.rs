//! Feature 004 — US1 T009: an imported OPEN sheet used as a stratigraphic contact meshes to a
//! watertight solid whose contact face lies ON the imported triangles (INV-1 watertight, INV-3
//! exact-inside). Drives the real recipe path (`mesh_volume_recipe`) end-to-end in the engine.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::obj::{normalize_and_check, parse_obj};
use geofield::octree::{Boundary, OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement};
use geofield::Model;

const SPAN: &[u8] = include_bytes!("fixtures/mesh-span-domain.obj");

/// Build an `ImportedSurface` (native kriged fit through the mesh) as a Contact, plus a separate
/// `TriMesh` copy for the on-the-mesh distance check (the element owns its own mesh).
fn imported_contact(bytes: &[u8]) -> (ImportedSurface<Model>, TriMesh) {
    let obj = parse_obj(bytes).expect("parses");
    let (verts, tris, health) = normalize_and_check(&obj).expect("meshable");
    assert_eq!(health.non_manifold_edges, 0, "clean import");
    let normals = obj.normals.clone().expect("fixture has vn");
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts
        .iter()
        .zip(&normals)
        .map(|(&p, &n)| Orientation::new(p, n))
        .collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    let check_mesh = TriMesh::new(verts.clone(), tris.clone());
    let mesh = TriMesh::new(verts, tris);
    (
        ImportedSurface { model, mesh, role: ImportRole::Contact, relation: Some(Relation::Conformable), extend_to_bounds: false },
        check_mesh,
    )
}

#[test]
fn imported_contact_meshes_watertight_and_on_the_mesh() {
    let (imported, check_mesh) = imported_contact(SPAN);
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported)]);

    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let min = [0.0, 0.0, 0.0];
    let ext_max = [1000.0, 1000.0, 1000.0];
    let ops = stack.mesh_volume_recipe(min, 1000.0, ext_max, cfg);

    // The exact-patch PRIMITIVE (op 255) is the below(MeshSignedField) solid — the one our code
    // generates (Manifold composes the units from it on the main thread). Assert its invariants.
    let prim = ops
        .iter()
        .find(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, _, m)| m)
        .expect("a primitive is emitted for the imported contact");
    assert!(prim.triangle_count() > 0, "the exact patch has geometry");

    // INV-1: watertight closed solid — edge incidence 2 everywhere (box-rim edges may be 1).
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in prim.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    let open = inc.values().filter(|&&c| c == 1).count();
    let nonmanifold = inc.values().filter(|&&c| c > 2).count();
    assert_eq!(nonmanifold, 0, "no non-manifold edges");
    assert_eq!(open, 0, "watertight closed solid: {open} open edges");

    // INV-3: the CONTACT face (vertices on the mesh, distance ≈ 0) lies at z≈500 (the flat bed).
    let mut contact_verts = 0;
    let mut max_dz: f64 = 0.0;
    for v in prim.positions.chunks_exact(3) {
        let p = [v[0] as f64, v[1] as f64, v[2] as f64];
        if check_mesh.surface_distance(p).unwrap_or(f64::MAX) < 2.0 {
            contact_verts += 1;
            max_dz = max_dz.max((p[2] - 500.0).abs());
        }
    }
    assert!(contact_verts > 0, "the contact face is present");
    assert!(max_dz < 2.0, "contact face lies ON the imported mesh (z≈500): max |dz| = {max_dz}");

    // The imported contact exposes TWO units (below / above) via the derived view (lone element,
    // top unit kept — it fills upward as the honest 'no ground defined' state).
    let dv = stack.derived_view([0.0, 1000.0, 0.0, 1000.0, 0.0, 1000.0]);
    assert_eq!(dv.units[0].1.len(), 2, "an imported contact has 2 units (below / above)");
}
