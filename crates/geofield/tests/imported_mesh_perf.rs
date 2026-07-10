//! Feature 004 — reality-check timing on a REAL large lidar `.obj` (native-only; reads an absolute
//! path, so it is `#[ignore]` — run with `cargo test -p geofield --test imported_mesh_perf -- --ignored --nocapture`).
//! Times parse → normalize → BVH build → LOCAL-sign contour (the footprint-only default path). No
//! co-kriging (the huge fit is subsampled in the wasm ABI; the native path here uses the local sign).

use geofield::obj::{normalize_and_check, parse_obj};
use geofield::octree::{MeshLocalField, OctreeConfig, TriMesh};
use std::time::Instant;

const LIDAR: &str = "/Users/collord/openlfview/dmea23_fan.extracted/obj/lidar_2ft_toldxf_wireframesTR.obj";

// A tiny local copy of `below_solid`'s contour so the test needs no crate-private access: contour
// {field<0} clipped to the mesh footprint (xy) × a z span, as a closed solid.
fn contour_footprint(mesh: &TriMesh, depth: u32) -> usize {
    use geofield::octree::{octree_dual_contour_scene, AxisBox, Scene};
    let (bmin, bmax) = mesh.bounds().unwrap();
    let local = MeshLocalField::new(mesh);
    let fmin = [bmin[0], bmin[1], bmin[2] - 50.0];
    let fmax = [bmax[0], bmax[1], bmax[2] + 50.0];
    let fsize = (fmax[0] - fmin[0]).max(fmax[1] - fmin[1]).max(fmax[2] - fmin[2]);
    let cfg = OctreeConfig { max_depth: depth, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let cell = fsize / (1u32 << depth) as f64;
    let m = cell * 3.0;
    let wmin = [fmin[0] - m, fmin[1] - m, fmin[2] - m];
    let box_b = AxisBox::from_bounds(fmin, fmax);
    let scene = Scene::new(&local, 0.0, vec![&box_b]);
    let mesh_out = octree_dual_contour_scene(&scene, wmin, fsize + 2.0 * m, cfg);
    mesh_out.triangle_count()
}

#[test]
#[ignore]
fn lidar_reality_check() {
    let Ok(bytes) = std::fs::read(LIDAR) else {
        eprintln!("SKIP: lidar file not present at {LIDAR}");
        return;
    };
    eprintln!("file: {:.1} MB", bytes.len() as f64 / 1e6);

    let t = Instant::now();
    let obj = parse_obj(&bytes).expect("parses");
    eprintln!("parse:        {:>7.2}s  ({} verts, {} tris)", t.elapsed().as_secs_f64(), obj.vertices.len(), obj.triangles.len());

    let t = Instant::now();
    let (verts, tris, health) = normalize_and_check(&obj).expect("meshable");
    eprintln!("normalize:    {:>7.2}s  (boundary_edges={}, non_manifold={})", t.elapsed().as_secs_f64(), health.boundary_edges, health.non_manifold_edges);

    let t = Instant::now();
    let mesh = TriMesh::new(verts, tris); // builds the BVH
    eprintln!("BVH build:    {:>7.2}s", t.elapsed().as_secs_f64());

    for depth in [6u32, 7] {
        let t = Instant::now();
        let ntris = contour_footprint(&mesh, depth);
        eprintln!("contour d={depth}: {:>7.2}s  ({ntris} output tris)", t.elapsed().as_secs_f64());
    }
}
