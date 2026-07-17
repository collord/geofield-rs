//! Feature 004 follow-up (code_review §3.4): an ONLAP series must pinch out against an older
//! IMPORTED stratigraphic surface (contact / topography), not only an older fitted `Series`.
//!
//! Before the fix, `clip_maskers`' onlap branch matched only `StackElement::Series`, so a younger
//! onlap lapping onto an older imported contact passed straight through it (no pinch-out). The fix
//! extends the branch to imported Contact/Topography roles, clipping the onlap against the EXACT
//! mesh (a `MeshHalfspace`, survive ABOVE the sheet) — mirroring how the younger-clips-older path
//! already treats imports, and avoiding the decimated-krige drift a fitted-field clip would carry.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::octree::{OctreeConfig, TriMesh};
use geofield::stack::{ImportRole, ImportedSurface, Relation, Stack, StackElement, StackedSeries};
use geofield::Model;

/// A DIPPING imported contact: a quad rising z = 300 + 0.6·(x−0) across the domain (z=300 at x=0,
/// z=900 at x=1000). Built inline (not a fixture) so the slope is discriminating — an onlap lapping
/// ABOVE it survives on the low side and pinches out on the high side. The co-kriged `model` is only
/// for the isovalue/type; the clip uses the exact mesh.
fn dipping_imported_contact() -> ImportedSurface<Model> {
    let z = |x: f64| 300.0 + 0.6 * x;
    let verts = vec![
        [0.0, 0.0, z(0.0)],
        [1000.0, 0.0, z(1000.0)],
        [1000.0, 1000.0, z(1000.0)],
        [0.0, 1000.0, z(0.0)],
    ];
    let tris = vec![[0usize, 1, 2], [0, 2, 3]];
    // Fit a field through the quad (upward-ish normal), matching the ingress path.
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    // Plane normal of z = 300 + 0.6x is (−0.6, 0, 1) normalized.
    let n = (0.6f64 * 0.6 + 1.0).sqrt();
    let pole = [-0.6 / n, 0.0, 1.0 / n];
    let oris: Vec<Orientation> = verts.iter().map(|&p| Orientation::new(p, pole)).collect();
    let model = Model::fit(&Series::new(sps, oris), KernelOptions::default());
    ImportedSurface {
        model,
        mesh: TriMesh::new(verts, tris),
        role: ImportRole::Contact,
        relation: Some(Relation::Conformable),
        extend_to_bounds: false,
    }
}

/// A younger flat contact at z=600, fit natively, with the given relation to the older element.
fn younger(relation: Relation) -> StackedSeries<Model> {
    let mut sps = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            sps.push(SurfacePoint::new([i as f64 * 300.0 + 50.0, j as f64 * 300.0 + 50.0, 600.0], 0));
        }
    }
    let oris = vec![Orientation::new([500.0, 500.0, 600.0], [0.0, 0.0, 1.0])];
    StackedSeries::new(Model::fit(&Series::new(sps, oris), KernelOptions::default()), relation)
}

fn cfg() -> OctreeConfig {
    OctreeConfig { max_depth: 6, min_depth: 2, qef_threshold: 1e-4, cap: true }
}

/// The x-extent of the younger series' (index 0) first interface CONTACT vertices (near z=600).
fn younger_contact_xrange(stack: &Stack<Model>) -> (f64, f64) {
    let meshes = stack.mesh_surfaces([0.0, 0.0, 0.0], 1000.0, cfg());
    let (_iid, mesh) = &meshes[0].interfaces[0];
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in mesh.positions.chunks_exact(3) {
        let (x, z) = (v[0] as f64, v[2] as f64);
        // Only the open contact sheet (near z=600), not box-cap vertices.
        if (z - 600.0).abs() < 60.0 {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo, hi)
}

#[test]
fn onlap_pinches_out_against_older_imported_contact() {
    // Older = a dipping imported contact rising left→right (z=300 at x=0 → z=900 at x=1000). It
    // crosses the younger flat contact (z=600) at x≈500: left of that the imported surface is BELOW
    // 600, right of it ABOVE. An onlap survives only on its depositional side (ABOVE the older slope),
    // so it must pinch out on the LEFT (where the imported surface has already risen past it)… i.e.
    // its surviving extent is a STRICT SUBSET of a conformable younger contact, which is unclipped.
    let conformable = Stack::new(vec![
        younger(Relation::Conformable).into(),
        StackElement::ImportedSurface(dipping_imported_contact()),
    ]);
    let onlap = Stack::new(vec![
        younger(Relation::Onlap).into(),
        StackElement::ImportedSurface(dipping_imported_contact()),
    ]);

    let (c_lo, c_hi) = younger_contact_xrange(&conformable);
    let (o_lo, o_hi) = younger_contact_xrange(&onlap);
    eprintln!("conformable younger x∈[{c_lo:.0},{c_hi:.0}]  onlap younger x∈[{o_lo:.0},{o_hi:.0}]");

    // Sanity: the unclipped (conformable) younger contact spans most of the domain.
    assert!(c_hi - c_lo > 700.0, "sanity: conformable younger contact spans the domain");
    // The onlap is pinched out against the older imported slope → its extent is strictly smaller.
    // (Before the fix the imported contact contributed no masker and the two were identical.)
    assert!(
        (o_hi - o_lo) < (c_hi - c_lo) - 100.0,
        "onlap must pinch out against the older imported contact: onlap span {:.0} vs conformable {:.0}",
        o_hi - o_lo,
        c_hi - c_lo,
    );
}
