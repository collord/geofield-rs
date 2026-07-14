//! Generator for the "Erosion over stack (imported meshes)" web3 template (feature 004).
//!
//! Writes five ~100k-vertex `.obj` meshes into `web3/public/meshes/`, each a layered-value-noise
//! perturbation of one analytic surface from the parametric "Erosion over conformal stack — repro"
//! template (`web3/src/scenarios.ts`). The imported-mesh template then references these by
//! `objPath`, so the same geology — a 2-unit erode cover over a 3-bed conformal stack — is rebuilt
//! from irregular imported surfaces instead of fitted parametric ones.
//!
//! `#[ignore]` because it writes into the repo as a side effect (like `imported_mesh_perf`'s
//! reality-check). Run explicitly:
//!
//! ```text
//! cargo test -p geofield --test gen_erosion_meshes -- --ignored --nocapture
//! ```
//!
//! The output `*.obj` are gitignored (per-machine assets, ~9 MB each — see
//! `web3/public/meshes/.gitignore`); regenerate them with this command on any checkout.

use geofield::obj::{normalize_and_check, parse_obj};
use std::fmt::Write as _;
use std::path::PathBuf;

/// Grid resolution: `N+1` samples per axis. N=316 → 317² = 100,489 verts, 2·316² = 199,712 tris.
const N: usize = 316;
/// Domain footprint (matches the source template's `EXTENT` xy = [0,1000]²).
const EXTENT: f64 = 1000.0;

/// One surface to generate: its analytic base-z function and its output filename + noise seed.
struct Surface {
    file: &'static str,
    seed: u32,
    /// Analytic base elevation from the parametric template (before perturbation).
    base: fn(f64, f64) -> f64,
    /// Peak perturbation amplitude (units). Kept well under the 120-unit bed spacing so a
    /// perturbed bed never crosses its neighbour.
    amp: f64,
}

/// The five source surfaces, mirroring `web3/src/scenarios.ts` "Erosion over conformal stack".
/// Beds: flat at z=200/320/440. Erosion: z=600+0.35·(x−500). Cover: z=800+0.1·(x−500).
fn surfaces() -> Vec<Surface> {
    vec![
        Surface { file: "erosion-stack-bed1.obj", seed: 0x1111, base: |_x, _y| 200.0, amp: 22.0 },
        Surface { file: "erosion-stack-bed2.obj", seed: 0x2222, base: |_x, _y| 320.0, amp: 22.0 },
        Surface { file: "erosion-stack-bed3.obj", seed: 0x3333, base: |_x, _y| 440.0, amp: 22.0 },
        Surface {
            file: "erosion-stack-erosion.obj",
            seed: 0x4444,
            base: |x, _y| 600.0 + 0.35 * (x - 500.0),
            // Erosion surfaces are rougher; still bounded so it stays between bed3 (≤~462) and
            // the cover, and never dips below the stack.
            amp: 40.0,
        },
        Surface {
            file: "erosion-stack-cover.obj",
            seed: 0x5555,
            base: |x, _y| 800.0 + 0.1 * (x - 500.0),
            amp: 30.0,
        },
    ]
}

// --- Deterministic seeded value noise (no `rand` dep) ---------------------------------------

/// Integer hash → f64 in [0,1). A cheap xorshift-style mix; deterministic across platforms.
fn hash01(mut h: u32) -> f64 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h as f64) / (u32::MAX as f64)
}

/// Hashed lattice value at integer cell (ix, iy) for a given seed, in [0,1).
fn lattice(ix: i32, iy: i32, seed: u32) -> f64 {
    let h = seed
        .wrapping_add((ix as u32).wrapping_mul(0x9e37_79b1))
        .wrapping_add((iy as u32).wrapping_mul(0x85eb_ca77));
    hash01(h)
}

fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Smoothstep-interpolated value noise at continuous (x,y) on a unit lattice, in [0,1).
fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let ix = x0 as i32;
    let iy = y0 as i32;
    let fx = smoothstep(x - x0);
    let fy = smoothstep(y - y0);
    let v00 = lattice(ix, iy, seed);
    let v10 = lattice(ix + 1, iy, seed);
    let v01 = lattice(ix, iy + 1, seed);
    let v11 = lattice(ix + 1, iy + 1, seed);
    let a = v00 + (v10 - v00) * fx;
    let b = v01 + (v11 - v01) * fx;
    a + (b - a) * fy
}

/// Layered (fractal) value noise centred on 0, in roughly [−1, 1]. Four octaves, amplitude halving
/// and frequency doubling — smooth undulation with a little fine detail. `x,y` are domain coords.
fn fbm(x: f64, y: f64, seed: u32) -> f64 {
    // Base wavelength ~ EXTENT/4 (a few broad swells across the footprint).
    let base_freq = 4.0 / EXTENT;
    let mut freq = base_freq;
    let mut amp = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for k in 0..4 {
        // Distinct seed per octave so octaves don't align.
        let v = value_noise(x * freq, y * freq, seed.wrapping_add(k * 0x1000_0001));
        sum += amp * (v * 2.0 - 1.0); // recentre [0,1] → [−1,1]
        norm += amp;
        freq *= 2.0;
        amp *= 0.5;
    }
    sum / norm
}

/// Perturbed elevation for one surface at (x, y): analytic base + fbm displacement, clamped into
/// the domain interior so no vertex escapes [10, 990].
fn elevation(s: &Surface, x: f64, y: f64) -> f64 {
    let z = (s.base)(x, y) + s.amp * fbm(x, y, s.seed);
    z.clamp(10.0, 990.0)
}

/// Build the `.obj` text for one surface: an (N+1)² grid of `v`/`vn` plus 2·N² triangle faces,
/// wound CCW as seen from +Z (outward normal +z), matching `TriMesh`'s convention.
fn build_obj(s: &Surface) -> String {
    let np1 = N + 1;
    let z = |i: usize, j: usize| {
        let x = (i as f64 / N as f64) * EXTENT;
        let y = (j as f64 / N as f64) * EXTENT;
        elevation(s, x, y)
    };
    // Preallocate: ~28 bytes/vert × 2 (v + vn) + ~30 bytes/face.
    let mut out = String::with_capacity(np1 * np1 * 56 + 2 * N * N * 30 + 128);
    let _ = writeln!(
        out,
        "# {} — value-noise perturbation of an analytic surface from 'Erosion over conformal stack'.",
        s.file
    );
    let _ = writeln!(out, "# {}×{} grid, wound CCW from +Z. Generated by gen_erosion_meshes.rs.", np1, np1);

    for j in 0..np1 {
        for i in 0..np1 {
            let x = (i as f64 / N as f64) * EXTENT;
            let y = (j as f64 / N as f64) * EXTENT;
            let _ = writeln!(out, "v {:.3} {:.3} {:.3}", x, y, z(i, j));
        }
    }
    // Per-vertex normals via central differences of the height field (interior); edges reuse the
    // one-sided neighbour. Normal of z=h(x,y): (−∂h/∂x, −∂h/∂y, 1) normalized.
    let dx = EXTENT / N as f64;
    for j in 0..np1 {
        for i in 0..np1 {
            let il = i.saturating_sub(1);
            let ir = (i + 1).min(N);
            let jl = j.saturating_sub(1);
            let jr = (j + 1).min(N);
            let dzdx = (z(ir, j) - z(il, j)) / ((ir - il) as f64 * dx);
            let dzdy = (z(i, jr) - z(i, jl)) / ((jr - jl) as f64 * dx);
            let (nx, ny, nz) = (-dzdx, -dzdy, 1.0);
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            let _ = writeln!(out, "vn {:.4} {:.4} {:.4}", nx / len, ny / len, nz / len);
        }
    }
    // 1-indexed vertex id at grid (i, j).
    let idx = |i: usize, j: usize| j * np1 + i + 1;
    for j in 0..N {
        for i in 0..N {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            // Two CCW-from-above triangles per quad; face uses `v//vn` (same id, shared indexing).
            let _ = writeln!(out, "f {a}//{a} {b}//{b} {c}//{c}");
            let _ = writeln!(out, "f {a}//{a} {c}//{c} {d}//{d}");
        }
    }
    out
}

/// Resolve `web3/public/meshes/` relative to this crate (CARGO_MANIFEST_DIR = crates/geofield).
fn meshes_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../../web3/public/meshes")
}

#[test]
#[ignore = "writes ~45 MB of .obj fixtures into web3/public/meshes/; run explicitly"]
fn gen_erosion_over_stack_meshes() {
    let dir = meshes_dir();
    assert!(
        dir.is_dir(),
        "expected meshes dir at {} — run from the repo checkout",
        dir.display()
    );
    eprintln!("writing meshes into {}", dir.display());

    for s in surfaces() {
        let text = build_obj(&s);
        let path = dir.join(s.file);
        std::fs::write(&path, &text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));

        // Sanity-fit: the generated bytes must parse + normalize into a meshable, hole-free surface
        // (the same ingress the wasm import runs). Fails the generator loudly if a mesh is garbage.
        let obj = parse_obj(text.as_bytes())
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", s.file));
        let (verts, tris, health) = normalize_and_check(&obj)
            .unwrap_or_else(|e| panic!("{} not meshable: {e}", s.file));
        assert!(!tris.is_empty(), "{} produced no triangles", s.file);
        eprintln!(
            "  {:<28} {:>7} verts  {:>7} tris  {:>5.1} MB  (boundary_edges={}, non_manifold={})",
            s.file,
            verts.len(),
            tris.len(),
            text.len() as f64 / 1e6,
            health.boundary_edges,
            health.non_manifold_edges,
        );
    }
    eprintln!("done — 5 meshes written.");
}
