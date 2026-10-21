//! Step 6 — solved dual weights vs gempy's dump (order-aware).
//!
//! Skips cleanly until `validation/fixtures/tiny/weights.npy` exists. Weights are
//! NOT order-invariant, so we permute gempy's weight vector into our layout using
//! `index_layout.json` (same machinery as the block-assembly gate) before
//! comparing. This only passes once the block-assembly gate passes — a wrong
//! matrix yields wrong weights.

use std::path::{Path, PathBuf};

use geofield_kernel::assemble::{assemble, AssembledGeometry, KernelOptions};
use geofield_kernel::data::{Orientation, Series, SurfacePoint};
use geofield_kernel::solve::solve;
use geofield_kernel::KernelFunction;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("validation")
        .join("fixtures")
        .join("tiny")
}

#[derive(Debug, Clone)]
enum RowKind {
    Grad { dim: usize, ori: usize },
    Sp { sp_inc: usize },
    Uni { uni: usize },
}

fn parse_layout(v: &serde_json::Value) -> Vec<RowKind> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|row| match row["kind"].as_str().unwrap() {
            "grad" => RowKind::Grad {
                dim: row["dim"].as_u64().unwrap() as usize,
                ori: row["ori"].as_u64().unwrap() as usize,
            },
            "sp" => RowKind::Sp {
                sp_inc: row["sp_inc"].as_u64().unwrap() as usize,
            },
            "uni" => RowKind::Uni {
                uni: row["uni"].as_u64().unwrap() as usize,
            },
            other => panic!("unknown row kind {other:?}"),
        })
        .collect()
}

fn our_index(geom: &AssembledGeometry, row: &RowKind) -> usize {
    match *row {
        RowKind::Grad { dim, ori } => geom.grad_index(dim, ori),
        RowKind::Sp { sp_inc } => geom.sp_index(sp_inc),
        RowKind::Uni { uni } => geom.uni_index(uni),
    }
}

fn load_series_and_opts(dir: &Path) -> Option<(Series, KernelOptions)> {
    let input: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("input.json")).ok()?).unwrap();
    let ko: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("kernel_options.json")).ok()?)
            .unwrap();

    let sps = input["surface_points"]
        .as_array()?
        .iter()
        .map(|p| {
            let x = p["xyz"].as_array().unwrap();
            SurfacePoint::new(
                [
                    x[0].as_f64().unwrap(),
                    x[1].as_f64().unwrap(),
                    x[2].as_f64().unwrap(),
                ],
                p["interface_id"].as_u64().unwrap() as u32,
            )
        })
        .collect();
    let oris = input["orientations"]
        .as_array()?
        .iter()
        .map(|o| {
            let x = o["xyz"].as_array().unwrap();
            let p = o["pole"].as_array().unwrap();
            Orientation::new(
                [
                    x[0].as_f64().unwrap(),
                    x[1].as_f64().unwrap(),
                    x[2].as_f64().unwrap(),
                ],
                [
                    p[0].as_f64().unwrap(),
                    p[1].as_f64().unwrap(),
                    p[2].as_f64().unwrap(),
                ],
            )
        })
        .collect();

    let f = |k: &str, d: f64| ko[k].as_f64().unwrap_or(d);
    let kernel_name = ko["kernel_function"].as_str().unwrap_or("cubic");
    let opts = KernelOptions {
        kernel: KernelFunction::from_gempy_name(kernel_name)
            .unwrap_or_else(|| panic!("fixture kernel_function {kernel_name:?} not implemented")),
        range: f("range", 1.7),
        c_o: f("c_o", 10.0),
        i_res: f("i_res", 4.0),
        gi_res: f("gi_res", 2.0),
        uni_degree: ko["uni_degree"].as_u64().unwrap_or(1) as u32,
        grad_nugget: f("grad_nugget", ko["nugget"].as_f64().unwrap_or(1e-5)),
        sp_nugget: f("sp_nugget", ko["nugget"].as_f64().unwrap_or(1e-5)),
        decimate: None,
    };
    Some((Series::new(sps, oris), opts))
}

fn load_npy_f64(path: &Path) -> Option<Vec<f64>> {
    let bytes = std::fs::read(path).ok()?;
    let npy = npyz::NpyFile::new(&bytes[..]).ok()?;
    Some(npy.into_vec::<f64>().unwrap())
}

#[test]
fn weights_match_gempy_tiny() {
    let dir = fixture_dir();
    let Some((series, opts)) = load_series_and_opts(&dir) else {
        eprintln!("SKIP weights_match_gempy_tiny: no fixture at {}", dir.display());
        return;
    };
    let gempy_w = match load_npy_f64(&dir.join("weights.npy")) {
        Some(w) => w,
        None => {
            eprintln!("SKIP weights_match_gempy_tiny: weights.npy missing");
            return;
        }
    };
    let layout = parse_layout(
        &serde_json::from_str(&std::fs::read_to_string(dir.join("index_layout.json")).unwrap())
            .unwrap(),
    );

    let sys = assemble(&series, opts);
    let sol = solve(&sys);
    let n = sys.dim;
    assert_eq!(gempy_w.len(), n, "weights.npy size mismatch");

    // gempy weight at gempy-index gi belongs to our index our_index(layout[gi]).
    let mut max_abs = 0.0_f64;
    for gi in 0..n {
        let oi = our_index(&sys.geom, &layout[gi]);
        let d = (sol.weights[oi] - gempy_w[gi]).abs();
        if d > max_abs {
            max_abs = d;
        }
    }
    eprintln!("max |w_ours − w_gempy| = {max_abs:.3e}; residual = {:.3e}", sol.residual);
    assert!(
        max_abs < 1e-6,
        "weights differ from gempy by {max_abs:e} (>1e-6). If the block-assembly \
         gate passes but this fails, suspect the RHS scaling (KERNEL_MATH.md §7.4)."
    );
}
