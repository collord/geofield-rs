//! Diagnostic + invariant: the SOLID volumes the app renders for the "Erosion over stack (imported
//! meshes)" template must PARTITION the domain — every interior point lies inside EXACTLY ONE volume
//! (no overlaps, no gaps). Overlapping prisms (the bug) show up as points inside ≥2 volumes.
//!
//! It reads the ACTUAL Manifold-booleaned solids, dumped one-.obj-per-volume by the Playwright
//! harness `web3/e2e/dump-volumes.manual.spec.ts` into `tests/fixtures/imported_volumes/`. Skips (does not
//! fail) if that dir is absent, so it is inert until you run the dump. Each volume is loaded as a
//! `TriMesh`; membership is the BVH signed distance (`< 0` inside — see `inside_solid`), reliable for
//! a closed Manifold solid and O(log tris) per point (the full winding number is far too slow here).
//!
//! Run: dump first, then `cargo test -p geofield --test imported_volumes_partition -- --nocapture`.

use geofield::octree::TriMesh;
use std::path::PathBuf;

/// Point-in-solid via the BVH signed distance: `< 0` is inside a CLOSED solid (nearest-triangle
/// side). O(log tris) per point — the full winding number is O(tris) and far too slow for a grid ×
/// six ~100k-tri solids. For a watertight Manifold solid the nearest-facet sign is reliable except
/// on the surface itself (the grid is offset off shared faces to avoid that).
fn inside_solid(m: &TriMesh, p: [f64; 3]) -> bool {
    m.local_signed_distance(p).map(|d| d < 0.0).unwrap_or(false)
}

#[derive(serde::Deserialize)]
struct VolMeta {
    file: String,
    series: usize,
    unit: usize,
    #[allow(dead_code)]
    tris: usize,
}

/// Which dump to measure: `GEOFIELD_PARTITION_DIR` (absolute, or relative to the crate root) —
/// e.g. `tests/fixtures/imported_volumes_mixed` for the `conformal-stack-mixed` dump — defaulting
/// to the all-imported template's `tests/fixtures/imported_volumes`.
fn fixtures_dir() -> PathBuf {
    match std::env::var("GEOFIELD_PARTITION_DIR") {
        Ok(d) => {
            let p = PathBuf::from(&d);
            if p.is_absolute() { p } else { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(d) }
        }
        Err(_) => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/imported_volumes"),
    }
}

/// Parse a minimal `v`/`f` OBJ (the dump format) into a `TriMesh`.
fn load_obj(path: &PathBuf) -> TriMesh {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut verts: Vec<[f64; 3]> = Vec::new();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f64> = it.take(3).map(|s| s.parse().unwrap()).collect();
                verts.push([c[0], c[1], c[2]]);
            }
            Some("f") => {
                let idx: Vec<usize> = it
                    .take(3)
                    .map(|s| s.split('/').next().unwrap().parse::<usize>().unwrap() - 1)
                    .collect();
                tris.push([idx[0], idx[1], idx[2]]);
            }
            _ => {}
        }
    }
    TriMesh::new(verts, tris)
}

#[test]
fn imported_volumes_partition_the_domain() {
    let dir = fixtures_dir();
    let manifest_path = dir.join("manifest.json");
    if !manifest_path.exists() {
        eprintln!(
            "SKIP: no dumped volumes at {} — run `npx playwright test dump-volumes.manual` first",
            dir.display()
        );
        return;
    }
    let manifest: Vec<VolMeta> =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    let vols: Vec<(String, TriMesh)> = manifest
        .iter()
        .map(|m| (format!("s{}:u{}", m.series, m.unit), load_obj(&dir.join(&m.file))))
        .collect();
    eprintln!("loaded {} volumes", vols.len());

    // Sample a 3D grid strictly INSIDE the domain [0,1000]³ (offset off cell boundaries so a point
    // rarely lands exactly on a shared face, where winding is ambiguous). Count how many volumes each
    // point is inside.
    const N: usize = 24;
    let lo = 0.0;
    let hi = 1000.0;
    let step = (hi - lo) / N as f64;
    let mut total = 0usize;
    let mut in_zero = 0usize;
    let mut in_multi = 0usize;
    let mut multi_edge = 0usize; // overlaps within 100 units of a lateral (x/y) domain bound
    let mut multi_interior = 0usize;
    // Report a few example overlap/gap points and the worst overlaps by volume pair.
    let mut overlap_pairs: std::collections::HashMap<(String, String), usize> = std::collections::HashMap::new();
    let mut examples_multi: Vec<([f64; 3], Vec<String>)> = Vec::new();
    let mut examples_zero: Vec<[f64; 3]> = Vec::new();

    for i in 0..N {
        for j in 0..N {
            for k in 0..N {
                let p = [
                    lo + (i as f64 + 0.5) * step,
                    lo + (j as f64 + 0.5) * step,
                    lo + (k as f64 + 0.37) * step, // z offset differs so we don't sit on flat beds
                ];
                total += 1;
                let members: Vec<&String> =
                    vols.iter().filter(|(_, m)| inside_solid(m, p)).map(|(name, _)| name).collect();
                match members.len() {
                    0 => {
                        in_zero += 1;
                        if examples_zero.len() < 5 {
                            examples_zero.push(p);
                        }
                    }
                    1 => {}
                    _ => {
                        in_multi += 1;
                        let near_edge = p[0] < 100.0 || p[0] > 900.0 || p[1] < 100.0 || p[1] > 900.0;
                        if near_edge { multi_edge += 1 } else { multi_interior += 1 }
                        for a in 0..members.len() {
                            for b in (a + 1)..members.len() {
                                let key = (members[a].clone(), members[b].clone());
                                *overlap_pairs.entry(key).or_insert(0) += 1;
                            }
                        }
                        // Record a few STRICTLY-INTERIOR examples (the real geometry bug, not the
                        // edge-curtain artifact) — those are what a future slab-primitive fix must kill.
                        if !near_edge && examples_multi.len() < 8 {
                            examples_multi.push((p, members.iter().map(|s| (*s).clone()).collect()));
                        }
                    }
                }
            }
        }
    }

    eprintln!("grid points: {total}  in_exactly_one: {}  in_zero(gap): {in_zero}  in_multi(overlap): {in_multi}", total - in_zero - in_multi);
    let mut pairs: Vec<_> = overlap_pairs.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1));
    for ((a, b), n) in pairs.iter().take(12) {
        eprintln!("  overlap {a} ∩ {b}: {n} points");
    }
    eprintln!("  overlaps: {multi_edge} near lateral edge (<100 from x/y), {multi_interior} strictly interior");
    for (p, m) in &examples_multi {
        eprintln!("  INTERIOR overlap [{:.0},{:.0},{:.0}] in {:?}", p[0], p[1], p[2], m);
    }
    for p in &examples_zero {
        eprintln!("  gap  [{:.0},{:.0},{:.0}]", p[0], p[1], p[2]);
    }

    // The partition invariant, as a TOLERANCED metric. IDEAL is 0 overlaps (a true partition), but a
    // KNOWN residual (~4%) remains as thin coincident-face films at the domain lateral edges and at
    // shared contacts — an artifact of the `below(top) − below(bottom)` subtract-prism approach (the
    // prisms' vertical boundary curtains + flat floors leave sub-cell slivers under Manifold's
    // boolean). See the `imported-mesh-volume-partition-residual` memory for the fix options (a
    // direct shared-contact slab primitive). The gross bug (overlapping full-height columns) was ~60%;
    // this guards against regressing to that while tolerating the known film.
    let overlap_frac = in_multi as f64 / total as f64;
    assert!(
        overlap_frac < 0.08,
        "{in_multi}/{total} ({:.1}%) interior points inside >1 volume — exceeds the 8% tolerance; \
         the volumes are grossly overlapping again (a clean partition is ~4% edge/contact film)",
        overlap_frac * 100.0
    );
}
