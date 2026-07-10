//! Feature 004 — T003 PATCH-STITCH SPIKE (architecture gate, plan.md / research.md D3).
//!
//! Decides whether the exact-inside / smooth-outside surface can be joined watertight+continuous
//! before any element plumbing is built. The join's continuity is driven by ONE number: how well the
//! smooth (cokriged) field agrees with the imported mesh AT the mesh's axis-aligned-bounds seam. If
//! the smooth field's zero-height ≈ the mesh height at the AABB wall, the two patches abut with no
//! step (a continuous seam / watertight stitch). It also checks that the exact `MeshSdf` reproduces
//! the mesh inside the bounds, and that the smooth field extends SANELY (finite, drift-clipped) beyond.
//!
//! This is a spike, not the feature: it measures the fields directly (no contour/clip yet), which is
//! the decisive architectural evidence. If the seam gap is small here, patch-stitch is viable and the
//! `refine` hook is unnecessary; if large, US2 must implement `refine`. Findings are asserted so a
//! regression is loud.

use geofield::assemble::KernelOptions;
use geofield::data::{Orientation, Series, SurfacePoint};
use geofield::obj::parse_obj;
use geofield::octree::{MeshSdf, TriMesh};
use geofield::{Model, ScalarField};

const UNDER: &[u8] = include_bytes!("fixtures/mesh-under-covering.obj");

/// Axis-aligned bounds of a point set.
fn aabb(v: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for p in v {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

/// Scan a vertical column at (x,y) for the z where `f(x,y,z)` crosses `iso`, linear-interpolating the
/// bracket. Returns `None` if it never crosses in [z0,z1]. Coarse scan (1 unit) then refine.
fn zero_height(f: &dyn ScalarField, x: f64, y: f64, iso: f64, z0: f64, z1: f64) -> Option<f64> {
    let step = 1.0;
    let mut prev_z = z0;
    let mut prev = f.scalar_field([x, y, z0]) - iso;
    let mut z = z0 + step;
    while z <= z1 {
        let cur = f.scalar_field([x, y, z]) - iso;
        if prev == 0.0 {
            return Some(prev_z);
        }
        if (prev < 0.0) != (cur < 0.0) {
            // linear interp of the crossing
            let t = prev / (prev - cur);
            return Some(prev_z + t * (z - prev_z));
        }
        prev_z = z;
        prev = cur;
        z += step;
    }
    None
}

#[test]
fn patch_stitch_seam_gap_and_exactness() {
    // --- build the imported mesh + its exact SDF field ---
    let obj = parse_obj(UNDER).expect("fixture parses");
    let verts = obj.vertices.clone();
    let tris: Vec<[usize; 3]> = obj.triangles.clone();
    let normals = obj.normals.clone().expect("fixture carries vn");
    let sdf = MeshSdf::new(TriMesh::new(verts.clone(), tris.clone()));
    let (lo, hi) = aabb(&verts); // [200,200,500]..[800,800,500]
    let bed_z = 500.0;

    // --- fit the SMOOTH field: cokriging through the mesh vertices (points) + normals (orientations),
    //     ONE interface — the reuse decided in research.md D1. (No decimation needed for 4 points.) ---
    let sps: Vec<SurfacePoint> = verts.iter().map(|&p| SurfacePoint::new(p, 0)).collect();
    let oris: Vec<Orientation> = verts
        .iter()
        .zip(&normals)
        .map(|(&p, &n)| Orientation::new(p, n))
        .collect();
    let smooth = Model::fit(&Series::new(sps, oris), KernelOptions::default());

    // The bed's recovered isovalue = the field value at the constraint points (they share one iso).
    let iso = verts.iter().map(|&p| smooth.scalar_field(p)).sum::<f64>() / verts.len() as f64;

    // --- (c) EXACTNESS inside the bounds: MeshSdf's zero-set is the mesh (z=500) within the AABB ---
    let mut max_exact_err: f64 = 0.0;
    for i in 1..10 {
        for j in 1..10 {
            let x = lo[0] + (hi[0] - lo[0]) * i as f64 / 10.0;
            let y = lo[1] + (hi[1] - lo[1]) * j as f64 / 10.0;
            // MeshSdf(x,y,500) should be ~0 (on the mesh).
            max_exact_err = max_exact_err.max(sdf.scalar_field([x, y, bed_z]).abs());
        }
    }
    assert!(
        max_exact_err < 1.0,
        "exact patch must reproduce the mesh inside the bounds: max |MeshSdf| on the bed = {max_exact_err}"
    );

    // --- (a) SEAM GAP: at the AABB footprint perimeter (the seam), the smooth field's zero-height must
    //     agree with the mesh height. The mesh boundary vertices are kriging constraints, so this is
    //     where continuity is won or lost. ---
    let mut max_seam_gap: f64 = 0.0;
    let n = 12;
    for t in 0..=n {
        let f = t as f64 / n as f64;
        // walk the four AABB edges (x=lo[0], x=hi[0], y=lo[1], y=hi[1])
        for &(x, y) in &[
            (lo[0], lo[1] + (hi[1] - lo[1]) * f),
            (hi[0], lo[1] + (hi[1] - lo[1]) * f),
            (lo[0] + (hi[0] - lo[0]) * f, lo[1]),
            (lo[0] + (hi[0] - lo[0]) * f, hi[1]),
        ] {
            if let Some(z) = zero_height(&smooth, x, y, iso, 0.0, 1000.0) {
                max_seam_gap = max_seam_gap.max((z - bed_z).abs());
            } else {
                panic!("smooth field has no bed crossing at seam ({x},{y}) — fit degenerate");
            }
        }
    }

    // --- (b) FAR-FIELD sanity: beyond the bounds (domain edge x=0), the smooth field still crosses the
    //     bed at a FINITE, sane height (drift, not garbage / no NaN) — the smooth patch extends out. ---
    let far = zero_height(&smooth, 0.0, 500.0, iso, 0.0, 1000.0);
    assert!(
        far.map(|z| z.is_finite() && (z - bed_z).abs() < 400.0).unwrap_or(false),
        "smooth far-field must extend sanely to the domain edge (got {far:?})"
    );

    eprintln!(
        "SPIKE T003: exact_err={max_exact_err:.3}  seam_gap={max_seam_gap:.3}  far_z={:?}",
        far
    );

    // The architecture gate: a small seam gap ⇒ patch-stitch is continuous by construction and the
    // `refine` hook is NOT needed (US2 T021). Tolerance is generous here (spike); US2 pins the number.
    assert!(
        max_seam_gap < 5.0,
        "SEAM GAP {max_seam_gap} too large — patch-stitch would crease; US2 must implement the mesh_clip refine hook"
    );
}
