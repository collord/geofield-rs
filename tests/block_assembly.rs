//! Step 5 — THE validation gate (BUILD_geofield_rs.md §5, KERNEL_MATH.md §6).
//!
//! On the smallest model (2 surface points on 1 interface + 1 orientation), diff
//! the assembled K and b **block-by-block** against a gempy_engine dump. A sign
//! error in `hv` or in the `(−k_p_ref + k_a)` ordering still yields a solvable
//! system and a smooth-but-wrong field — only the block diff localizes it.
//!
//! Fixtures are produced by `validation/make_fixtures.py` (run separately with a
//! gempy install). Until they exist, this test **skips cleanly** rather than
//! failing — so the suite stays green while the gempy run is pending. Once
//! `validation/fixtures/tiny/` is populated, the test becomes a hard gate.
//!
//! The fixture carries `index_layout.json` describing the meaning of every gempy
//! row, so we never guess gempy's interleaving: we build the gempy→ours
//! permutation from that file and permute gempy's matrix into our ordering before
//! diffing. This makes the test robust to the §7 open item (orientation-component
//! interleaving) — a layout mismatch is reported as such, not as a numeric diff.

use std::path::{Path, PathBuf};

use geofield_rs::assemble::{assemble, AssembledGeometry, KernelOptions};
use geofield_rs::data::{Orientation, Series, SurfacePoint};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("validation")
        .join("fixtures")
        .join("tiny")
}

/// One gempy row's meaning, mirrored from `index_layout.json`.
#[derive(Debug, Clone)]
enum RowKind {
    Grad { dim: usize, ori: usize },
    Sp { sp_inc: usize },
    Uni { uni: usize },
}

fn parse_layout(v: &serde_json::Value) -> Vec<RowKind> {
    v.as_array()
        .expect("index_layout must be an array")
        .iter()
        .map(|row| {
            let kind = row["kind"].as_str().expect("row.kind");
            match kind {
                "grad" => RowKind::Grad {
                    dim: row["dim"].as_u64().expect("grad.dim") as usize,
                    ori: row["ori"].as_u64().expect("grad.ori") as usize,
                },
                "sp" => RowKind::Sp {
                    sp_inc: row["sp_inc"].as_u64().expect("sp.sp_inc") as usize,
                },
                "uni" => RowKind::Uni {
                    uni: row["uni"].as_u64().expect("uni.uni") as usize,
                },
                other => panic!("unknown row kind {other:?}"),
            }
        })
        .collect()
}

/// Map a gempy row (its `RowKind`) to our index, using our geometry's layout.
fn our_index(geom: &AssembledGeometry, row: &RowKind) -> usize {
    match *row {
        RowKind::Grad { dim, ori } => geom.grad_index(dim, ori),
        RowKind::Sp { sp_inc } => geom.sp_index(sp_inc),
        RowKind::Uni { uni } => geom.uni_index(uni),
    }
}

/// Read the inputs gempy actually used (so the Rust assembly is from identical
/// data). Falls back to nothing if the fixture is absent.
fn load_series_and_opts(dir: &Path) -> Option<(Series, KernelOptions)> {
    let input_txt = std::fs::read_to_string(dir.join("input.json")).ok()?;
    let opts_txt = std::fs::read_to_string(dir.join("kernel_options.json")).ok()?;
    let input: serde_json::Value = serde_json::from_str(&input_txt).unwrap();
    let ko: serde_json::Value = serde_json::from_str(&opts_txt).unwrap();

    let sps = input["surface_points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let xyz = p["xyz"].as_array().unwrap();
            SurfacePoint::new(
                [
                    xyz[0].as_f64().unwrap(),
                    xyz[1].as_f64().unwrap(),
                    xyz[2].as_f64().unwrap(),
                ],
                p["interface_id"].as_u64().unwrap() as u32,
            )
        })
        .collect();
    let oris = input["orientations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let xyz = o["xyz"].as_array().unwrap();
            let pole = o["pole"].as_array().unwrap();
            Orientation::new(
                [
                    xyz[0].as_f64().unwrap(),
                    xyz[1].as_f64().unwrap(),
                    xyz[2].as_f64().unwrap(),
                ],
                [
                    pole[0].as_f64().unwrap(),
                    pole[1].as_f64().unwrap(),
                    pole[2].as_f64().unwrap(),
                ],
            )
        })
        .collect();

    let f = |k: &str, d: f64| ko[k].as_f64().unwrap_or(d);
    let opts = KernelOptions {
        range: f("range", 1.7),
        c_o: f("c_o", 10.0),
        i_res: f("i_res", 4.0),
        gi_res: f("gi_res", 2.0),
        uni_degree: ko["uni_degree"].as_u64().unwrap_or(1) as u32,
        grad_nugget: f("grad_nugget", ko["nugget"].as_f64().unwrap_or(1e-5)),
        sp_nugget: f("sp_nugget", ko["nugget"].as_f64().unwrap_or(1e-5)),
    };

    Some((Series::new(sps, oris), opts))
}

fn load_npy_f64(path: &Path) -> Option<Vec<f64>> {
    let bytes = std::fs::read(path).ok()?;
    let npy = npyz::NpyFile::new(&bytes[..]).ok()?;
    Some(npy.into_vec::<f64>().expect("npy f64 read"))
}

#[test]
fn block_by_block_matches_gempy_tiny() {
    let dir = fixture_dir();
    let Some((series, opts)) = load_series_and_opts(&dir) else {
        eprintln!(
            "SKIP block_by_block_matches_gempy_tiny: no fixture at {} \
             (run validation/make_fixtures.py with a gempy install)",
            dir.display()
        );
        return;
    };
    let layout_txt = std::fs::read_to_string(dir.join("index_layout.json"))
        .expect("fixture present but index_layout.json missing");
    let layout = parse_layout(&serde_json::from_str(&layout_txt).unwrap());
    let gempy_k = load_npy_f64(&dir.join("K.npy")).expect("K.npy");
    let gempy_b = load_npy_f64(&dir.join("b.npy")).expect("b.npy");

    let sys = assemble(&series, opts);
    let n = sys.dim;
    assert_eq!(layout.len(), n, "fixture dim {} != ours {}", layout.len(), n);
    assert_eq!(gempy_k.len(), n * n, "K.npy size mismatch");
    assert_eq!(gempy_b.len(), n, "b.npy size mismatch");

    // gempy index -> our index.
    let perm: Vec<usize> = layout.iter().map(|r| our_index(&sys.geom, r)).collect();

    // Classify each of our indices into a block, for per-block reporting.
    let block_of = |idx: usize| -> &'static str {
        let ng = 3 * sys.geom.n_ori;
        let nsp = sys.geom.n_sp_inc;
        if idx < ng {
            "GRAD"
        } else if idx < ng + nsp {
            "SP"
        } else {
            "UNI"
        }
    };

    let tol = 1e-9_f64;
    let mut block_max: std::collections::BTreeMap<(&str, &str), f64> =
        std::collections::BTreeMap::new();
    let mut first_fail: Option<(usize, usize, f64, f64)> = None;

    for gi in 0..n {
        for gj in 0..n {
            let oi = perm[gi];
            let oj = perm[gj];
            let ours = sys.at(oi, oj);
            let theirs = gempy_k[gi * n + gj];
            let d = (ours - theirs).abs();
            let key = (block_of(oi), block_of(oj));
            let e = block_max.entry(key).or_insert(0.0);
            if d > *e {
                *e = d;
            }
            if d > tol && first_fail.is_none() {
                first_fail = Some((oi, oj, ours, theirs));
            }
        }
    }

    eprintln!("per-block max |K_ours − K_gempy| (our block labels):");
    for ((bi, bj), m) in &block_max {
        eprintln!("  {bi:>4} x {bj:<4}: {m:.3e}");
    }

    // RHS diff (permuted).
    let mut b_max = 0.0_f64;
    for gi in 0..n {
        let d = (sys.b[perm[gi]] - gempy_b[gi]).abs();
        if d > b_max {
            b_max = d;
        }
    }
    eprintln!("max |b_ours − b_gempy|: {b_max:.3e}");

    if let Some((oi, oj, ours, theirs)) = first_fail {
        panic!(
            "K block mismatch at our ({oi},{oj}) [{} x {}]: ours={ours} gempy={theirs} \
             (tol {tol:e}). See per-block table above to localize the flipped block.",
            block_of(oi),
            block_of(oj)
        );
    }
    assert!(
        b_max <= tol,
        "RHS mismatch (max {b_max:e} > tol {tol:e}) — see KERNEL_MATH.md §7.4 \
         (pole normalization / RHS multiplier is the open item)"
    );
}
