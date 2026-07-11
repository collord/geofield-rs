//! Repro: model bounds (domain box) sit LATERALLY INSIDE the imported mesh's footprint — the mesh
//! extends past the domain on every side (the user's lidar case: a surface much larger than the
//! model volume). The recipe must still produce a below-contact primitive that COVERS the domain,
//! so the main-thread `∩ domainBox` yields a non-empty rock unit.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::obj::{normalize_and_check, parse_obj};
use geofield::octree::{OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement};
use geofield::Model;

const SPAN: &[u8] = include_bytes!("fixtures/mesh-span-domain.obj"); // flat bed at z=500, [0,1000]²

fn imported_contact() -> ImportedSurface<Model> {
    let obj = parse_obj(SPAN).unwrap();
    let (verts, tris, _h) = normalize_and_check(&obj).unwrap();
    let normals = obj.normals.clone().unwrap();
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> =
        verts.iter().zip(&normals).map(|(&p, &n)| Orientation::new(p, n)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    ImportedSurface {
        model,
        mesh: TriMesh::new(verts, tris),
        role: ImportRole::Contact,
        relation: Some(Relation::Conformable),
        extend_to_bounds: false,
    }
}

#[test]
fn domain_inside_mesh_footprint_still_meshes_units() {
    // Mesh spans [0, 1000]²; domain is [250, 750]² — fully inside the footprint laterally.
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported_contact())]);

    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let min = [250.0, 250.0, 0.0];
    let size = 500.0;
    let ext_max = [750.0, 750.0, 1000.0];
    let ops = stack.mesh_volume_recipe(min, size, ext_max, cfg);

    let prim = ops
        .iter()
        .find(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, _, m)| m)
        .expect("a primitive is emitted for the imported contact");
    assert!(prim.triangle_count() > 0, "the below-sheet primitive has geometry");

    // INV-1: the primitive is a watertight closed solid.
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in prim.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    let open = inc.values().filter(|&&c| c == 1).count();
    assert_eq!(open, 0, "watertight closed solid: {open} open edges");

    // The primitive must COVER the domain box in XY — else `∩ domainBox` yields an empty unit and
    // the volume view shows nothing (the reported bug).
    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for v in prim.positions.chunks_exact(3) {
        xmin = xmin.min(v[0] as f64);
        xmax = xmax.max(v[0] as f64);
        ymin = ymin.min(v[1] as f64);
        ymax = ymax.max(v[1] as f64);
    }
    assert!(xmin <= 250.0 && xmax >= 750.0, "primitive spans the domain in X: [{xmin}, {xmax}]");
    assert!(ymin <= 250.0 && ymax >= 750.0, "primitive spans the domain in Y: [{ymin}, {ymax}]");
}

/// A TILTED sheet whose footprint exceeds the domain and whose Z sits inside a TALL domain (the lidar
/// case: mesh z≈6200–7800 inside domain z=[3000, 9000]). The below-contact prism must still reach the
/// domain floor and top out exactly at the ramp, even though the domain overhangs the mesh vertically.
fn tilted_sheet(ext: f64, z_lo: f64, z_hi: f64) -> (TriMesh, Vec<[f64; 3]>) {
    // z ramps with x across [0, ext]; a coarse grid so the surface is a real triangulated ramp.
    let n = 8usize;
    let mut verts = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let x = i as f64 / n as f64 * ext;
            let y = j as f64 / n as f64 * ext;
            let z = z_lo + (x / ext) * (z_hi - z_lo);
            verts.push([x, y, z]);
        }
    }
    let idx = |i: usize, j: usize| j * (n + 1) + i;
    let mut tris = Vec::new();
    for j in 0..n {
        for i in 0..n {
            // Upward-facing (CCW from +Z) → below the ramp is rock.
            tris.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            tris.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let pts = verts.clone();
    (TriMesh::new(verts, tris), pts)
}

#[test]
fn tilted_mesh_larger_than_tall_domain_meshes_units() {
    // Mesh spans [0, 3000]² with z ramping 6200→7800; domain [800, 2200]² × z[3000, 9000] — inside
    // the footprint laterally, but TALLER than the mesh vertically (mirrors the reported project).
    let (mesh, obj_pts) = tilted_sheet(3000.0, 6200.0, 7800.0);
    let sps: Vec<SurfacePoint> = obj_pts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> =
        obj_pts.iter().map(|&p| Orientation::new(p, [0.0, 0.0, 1.0])).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    let imported = ImportedSurface {
        model,
        mesh,
        role: ImportRole::Contact,
        relation: Some(Relation::Conformable),
        extend_to_bounds: false,
    };
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported)]);

    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([800.0, 800.0, 3000.0], 1400.0, [2200.0, 2200.0, 9000.0], cfg);

    let prim = ops
        .iter()
        .find(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, _, m)| m)
        .expect("a primitive is emitted for the tilted imported contact");
    assert!(prim.triangle_count() > 0, "the below-ramp primitive has geometry");

    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in prim.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    let open = inc.values().filter(|&&c| c == 1).count();
    assert_eq!(open, 0, "watertight closed solid: {open} open edges");

    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
    for v in prim.positions.chunks_exact(3) {
        xmin = xmin.min(v[0] as f64);
        xmax = xmax.max(v[0] as f64);
        ymin = ymin.min(v[1] as f64);
        ymax = ymax.max(v[1] as f64);
        zmin = zmin.min(v[2] as f64);
        zmax = zmax.max(v[2] as f64);
    }
    assert!(xmin <= 800.0 && xmax >= 2200.0, "spans domain X: [{xmin}, {xmax}]");
    assert!(ymin <= 800.0 && ymax >= 2200.0, "spans domain Y: [{ymin}, {ymax}]");
    // The prism fills from below the domain floor (z0=3000) UP to the ramp — reaching the floor and
    // topping AT the ramp (the sealed prim's z-max IS the exact mesh top), not the box top.
    assert!(zmin < 3000.0, "prism floor sits below the domain floor: zmin={zmin}");
    assert!(zmax >= 6200.0 && zmax <= 7800.0, "prism tops out at the ramp, not the box: zmax={zmax}");
}

/// A DOMED (doubly-curved) sheet — the shape that actually broke the old contour path. A monotone tilt
/// signs consistently under nearest-triangle rules; a dome's nearest triangle flips across the crest, so
/// the old `MeshLocalField` contour tore ~open. The sealed-prism primitive is watertight regardless of
/// curvature (it reuses the mesh's own triangles), which this guards.
fn dome_sheet(ext: f64, z_base: f64, z_bulge: f64) -> (TriMesh, Vec<[f64; 3]>) {
    let n = 12usize;
    let mut verts = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let x = i as f64 / n as f64 * ext;
            let y = j as f64 / n as f64 * ext;
            let (u, v) = (i as f64 / n as f64 - 0.5, j as f64 / n as f64 - 0.5);
            let z = z_base + z_bulge * (std::f64::consts::PI * u).cos() * (std::f64::consts::PI * v).cos();
            verts.push([x, y, z]);
        }
    }
    let idx = |i: usize, j: usize| j * (n + 1) + i;
    let mut tris = Vec::new();
    for j in 0..n {
        for i in 0..n {
            tris.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            tris.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let pts = verts.clone();
    (TriMesh::new(verts, tris), pts)
}

#[test]
fn curved_dome_domain_inside_footprint_is_watertight_and_manifold() {
    // Dome spans [0, 3000]², z≈2500..4000; domain [800, 2200]² × z[1000, 6000] — inside the footprint,
    // curved (the failing shape), and taller than the mesh.
    let (mesh, obj_pts) = dome_sheet(3000.0, 2500.0, 1500.0);
    let sps: Vec<SurfacePoint> = obj_pts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> =
        obj_pts.iter().map(|&p| Orientation::new(p, [0.0, 0.0, 1.0])).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    let imported = ImportedSurface {
        model,
        mesh,
        role: ImportRole::Contact,
        relation: Some(Relation::Conformable),
        extend_to_bounds: false,
    };
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported)]);

    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let ops = stack.mesh_volume_recipe([800.0, 800.0, 1000.0], 1400.0, [2200.0, 2200.0, 6000.0], cfg);
    let prim = ops
        .iter()
        .find(|(_, _, _, op, _, _)| *op == 255)
        .map(|(_, _, _, _, _, m)| m)
        .expect("a primitive is emitted for the domed imported contact");
    assert!(prim.triangle_count() > 0, "the below-dome primitive has geometry");

    // WATERTIGHT + 2-MANIFOLD — the exact gate Manifold.ofMesh applies in the browser. The old contour
    // left ~1750 open edges on a real curved TIN (Manifold rejected it → empty volume view); the sealed
    // prism has every edge shared by exactly two triangles.
    let mut inc: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in prim.indices.chunks_exact(3) {
        for &(a, b) in &[(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let e = if a < b { (a, b) } else { (b, a) };
            *inc.entry(e).or_insert(0) += 1;
        }
    }
    assert_eq!(inc.values().filter(|&&c| c == 1).count(), 0, "watertight: no open edges");
    assert_eq!(inc.values().filter(|&&c| c > 2).count(), 0, "2-manifold: no edge with >2 faces");

    let (mut xmin, mut xmax, mut ymin, mut ymax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    let (mut zmin, mut zmax) = (f64::MAX, f64::MIN);
    for v in prim.positions.chunks_exact(3) {
        xmin = xmin.min(v[0] as f64);
        xmax = xmax.max(v[0] as f64);
        ymin = ymin.min(v[1] as f64);
        ymax = ymax.max(v[1] as f64);
        zmin = zmin.min(v[2] as f64);
        zmax = zmax.max(v[2] as f64);
    }
    assert!(xmin <= 800.0 && xmax >= 2200.0, "spans domain X: [{xmin}, {xmax}]");
    assert!(ymin <= 800.0 && ymax >= 2200.0, "spans domain Y: [{ymin}, {ymax}]");
    assert!(zmin < 1000.0, "prism floor sits below the domain floor: zmin={zmin}");
    // The prism top is the EXACT dome (crest z = 2500 + 1500 = 4000), well under the box top (z1=6000).
    assert!(zmax >= 3900.0 && zmax <= 4000.0, "prism tops out at the dome crest: zmax={zmax}");
}
