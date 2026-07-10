//! Feature 004 — US3 T022/T024: an imported surface plays a ROLE. An imported EROSION surface must
//! truncate the older elements beneath it (INV-6), exactly as a fitted erosion does — verified by
//! the older floor losing volume when the imported erosion is present vs. a conformable contact.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::obj::{normalize_and_check, parse_obj};
use geofield::octree::{OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement, StackedSeries};
use geofield::Model;

const SPAN: &[u8] = include_bytes!("fixtures/mesh-span-domain.obj"); // flat bed at z=500, full footprint

fn imported(role: ImportRole) -> ImportedSurface<Model> {
    let obj = parse_obj(SPAN).unwrap();
    let (verts, tris, _h) = normalize_and_check(&obj).unwrap();
    let normals = obj.normals.clone().unwrap();
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts.iter().zip(&normals).map(|(&p, &n)| Orientation::new(p, n)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    ImportedSurface { model, mesh: TriMesh::new(verts, tris), role, relation: Some(Relation::Erode), extend_to_bounds: false }
}

/// A flat floor with three beds (z=300/500/700), fit natively.
fn floor() -> StackedSeries<Model> {
    let mut sps = Vec::new();
    for (id, z0) in [(0u32, 300.0), (1, 500.0), (2, 700.0)] {
        for i in 0..4 {
            for j in 0..4 {
                sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, z0], id));
            }
        }
    }
    let oris = vec![Orientation::new([500.0, 500.0, 500.0], [0.0, 0.0, 1.0])];
    StackedSeries::floor(Model::fit(&Series::new(sps, oris), KernelOptions::default()))
}

/// |signed volume| of a triangle soup (divergence theorem).
fn volume(pos: &[f32], idx: &[u32]) -> f64 {
    let mut v = 0.0;
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        let p = |i: usize| [pos[3 * i] as f64, pos[3 * i + 1] as f64, pos[3 * i + 2] as f64];
        let (a, b, c) = (p(a), p(b), p(c));
        v += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0;
    }
    v.abs()
}

/// Total meshed volume of the floor's units (index 1) via the field-path mesher, which applies
/// `clip_maskers` — so a younger imported EROSION above it truncates it.
fn floor_volume(role: ImportRole) -> f64 {
    let stack = Stack::new(vec![StackElement::ImportedSurface(imported(role)), floor().into()]);
    let cfg = OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true };
    let mut total = 0.0;
    for u in 0..stack.block_unit_count(1, 0) {
        if let Some(m) = stack.mesh_one_block_unit_solid(1, 0, u, [0.0, 0.0, 0.0], 1000.0, [1000.0, 1000.0, 1000.0], cfg) {
            total += volume(&m.positions, &m.indices);
        }
    }
    total
}

#[test]
fn imported_erosion_truncates_older_floor() {
    // A CONTACT imported surface contributes NO clip — the floor keeps its full volume.
    let with_contact = floor_volume(ImportRole::Contact);
    // An EROSION imported surface truncates the floor (removes the material on the eroded side).
    let with_erosion = floor_volume(ImportRole::Erosion);

    assert!(with_contact > 1e8, "sanity: the floor has substantial volume under a contact");
    assert!(
        with_erosion < with_contact * 0.95,
        "an imported erosion must remove floor material: erosion={with_erosion:.3e} vs contact={with_contact:.3e}"
    );
}

#[test]
fn imported_topography_caps_older_floor_to_air() {
    // An imported TOPOGRAPHY surface caps the older floor BELOW it (Air above) — the floor loses
    // the material above the imported ground (INV-6 for the topography role).
    let with_contact = floor_volume(ImportRole::Contact);
    let with_topo = floor_volume(ImportRole::Topography);
    assert!(
        with_topo < with_contact * 0.95,
        "an imported topography must cap the floor to Air above it: topo={with_topo:.3e} vs contact={with_contact:.3e}"
    );
}
