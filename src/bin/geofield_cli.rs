//! geofield-cli — validation driver (feature = "cli").
//!
//! Reads a gempy-produced fixture directory, fits the geofield-rs model from the
//! SAME inputs and live kernel options gempy used, evaluates on the SAME grid
//! points gempy evaluated, and writes the results back as .npy for the Python
//! side (`validation/run_model1_validation.py`) to diff.
//!
//! Contract (see `--help`):
//!   geofield-cli --fixture <DIR>
//!   reads:
//!     <DIR>/input_surface_points.csv   gempy format: X,Y,Z,formation
//!     <DIR>/input_orientations.csv     gempy format: X,Y,Z,azimuth,dip,polarity,formation
//!     <DIR>/kernel_options.json        live range,c_o,i_res,gi_res,uni_degree,(nugget)
//!     <DIR>/series.json                ordered formation list (bottom→top per series)
//!     <DIR>/grid.npy                   (N,3) f64 grid points gempy evaluated, world coords
//!   writes:
//!     <DIR>/rust_scalar_field.npy      (N,) f64, same point order as gempy_scalar_field
//!     <DIR>/rust_litho.npy             (N,) f64 unit ids, same order
//!     <DIR>/rust_isovalues.json        recovered per-interface isovalues (the §7.4 check)
//!
//! Reading the LIVE kernel options + the exact grid points (not a re-derived
//! grid) is deliberate: it removes the two biggest sources of spurious diff —
//! hardcoded options and grid-ordering convention mismatches.

use std::path::{Path, PathBuf};

use geofield_rs::assemble::KernelOptions;
use geofield_rs::data::{Orientation, Series, SurfacePoint};
use geofield_rs::Model;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut fixture: Option<PathBuf> = None;
    let mut dump_kernel_only = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--fixture" => {
                i += 1;
                fixture = args.get(i).map(PathBuf::from);
            }
            "--dump-kernel" => {
                // Assemble K/b for the fixture inputs and write rust_K.npy /
                // rust_b.npy. No grid needed — this is the step-5 block-diff path.
                dump_kernel_only = true;
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other => {
                eprintln!("unknown arg: {other}");
                print_help();
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let dir = fixture.unwrap_or_else(|| {
        eprintln!("error: --fixture <DIR> is required");
        print_help();
        std::process::exit(2);
    });

    let result = if dump_kernel_only {
        dump_kernel(&dir)
    } else {
        run(&dir)
    };
    if let Err(e) = result {
        eprintln!("geofield-cli: {e}");
        std::process::exit(1);
    }
}

fn print_help() {
    println!(
        "geofield-cli --fixture <DIR> [--dump-kernel]\n\n\
         Default: reads gempy fixture inputs, fits geofield-rs, evaluates on\n\
         gempy's grid, writes rust_scalar_field.npy / rust_litho.npy /\n\
         rust_isovalues.json.\n\
         --dump-kernel: assemble K/b for the inputs and write rust_K.npy /\n\
         rust_b.npy (the step-5 block-diff path; no grid.npy required).\n\
         Build with:  cargo build --release --features cli"
    );
}

/// Assemble the kriging system for the fixture inputs and write the dense K
/// (row-major (dim,dim)) and b ((dim,)) as .npy for the block diff.
fn dump_kernel(dir: &Path) -> Result<(), String> {
    use geofield_rs::assemble::assemble;
    let series = read_inputs(dir)?;
    let opts = read_kernel_options(dir)?;
    let sys = assemble(&series, opts);
    let (k, b, n) = sys.dump_matrix();
    eprintln!(
        "[cli] dump-kernel: dim={n}  range={} c_o={} gi_res={} i_res={} nugget(g/sp)={}/{}",
        opts.range, opts.c_o, opts.gi_res, opts.i_res, opts.grad_nugget, opts.sp_nugget
    );
    write_npy_2d(&dir.join("rust_K.npy"), k, n, n)?;
    write_npy_1d(&dir.join("rust_b.npy"), b)?;
    eprintln!("[cli] wrote rust_K.npy ({n}x{n}), rust_b.npy ({n})");
    Ok(())
}

fn run(dir: &Path) -> Result<(), String> {
    let series = read_inputs(dir)?;
    let opts = read_kernel_options(dir)?;
    let grid = read_grid(dir)?;

    eprintln!(
        "[cli] {} surface points, {} orientations, {} grid pts; range={} c_o={} nugget(g/sp)={}/{}",
        series.surface_points.len(),
        series.orientations.len(),
        grid.len(),
        opts.range,
        opts.c_o,
        opts.grad_nugget,
        opts.sp_nugget,
    );

    let model = Model::fit(&series, opts);
    eprintln!(
        "[cli] solve residual={:.3e}  isovalue spread={:.3e}",
        model.residual, model.isovalue_spread
    );

    // Scalar field + lithology unit at each gempy grid point, in gempy's order.
    //
    // gempy numbers lithology units 1..=n_units in DESCENDING scalar order
    // (highest scalar value → unit 1; basement → highest id). Our Lithology uses
    // 0-indexed ASCENDING (lowest scalar → unit 0). The geometry is identical;
    // only the label convention differs. Remap to gempy's so rust_litho.npy
    // compares directly: gempy_id = n_units − our_id  (verified 100% voxel
    // agreement on model1).
    let n_units = model.lithology().n_units();
    let mut field = Vec::with_capacity(grid.len());
    let mut litho = Vec::with_capacity(grid.len());
    for p in &grid {
        field.push(model.scalar_field(*p));
        let our_id = model.lithology_unit(*p);
        litho.push((n_units - our_id) as f64);
    }

    write_npy_1d(&dir.join("rust_scalar_field.npy"), &field)?;
    write_npy_1d(&dir.join("rust_litho.npy"), &litho)?;

    // Recovered per-interface isovalues (the isovalue-magnitude / §7.4 check).
    let isovals: Vec<(u32, f64)> = model
        .lithology()
        .interfaces
        .iter()
        .map(|iv| (iv.interface_id, iv.isovalue))
        .collect();
    let iso_json = format!(
        "{{\"isovalues\": [{}], \"residual\": {}, \"isovalue_spread\": {}}}",
        isovals
            .iter()
            .map(|(id, v)| format!("{{\"interface_id\": {id}, \"isovalue\": {v}}}"))
            .collect::<Vec<_>>()
            .join(", "),
        model.residual,
        model.isovalue_spread,
    );
    std::fs::write(dir.join("rust_isovalues.json"), iso_json)
        .map_err(|e| format!("writing rust_isovalues.json: {e}"))?;

    eprintln!("[cli] wrote rust_scalar_field.npy, rust_litho.npy, rust_isovalues.json");
    Ok(())
}

/// Read both gempy CSVs into a single-series model. Formation→interface id is
/// taken from `series.json` (ordered list); the reference point per interface is
/// the first surface point of that formation (Series::new's default).
fn read_inputs(dir: &Path) -> Result<Series, String> {
    let order = read_series_order(dir)?;
    let id_of = |formation: &str| -> Result<u32, String> {
        order
            .iter()
            .position(|f| f == formation)
            .map(|p| p as u32)
            .ok_or_else(|| format!("formation {formation:?} not in series.json {order:?}"))
    };

    // surface points: X,Y,Z,formation
    let sp_text = read_to_string(dir, "input_surface_points.csv")?;
    let mut surface_points = Vec::new();
    for (lineno, line) in sp_text.lines().enumerate() {
        if lineno == 0 || line.trim().is_empty() {
            continue; // header / blank
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 4 {
            return Err(format!("surface_points line {}: <4 fields", lineno + 1));
        }
        let xyz = parse_xyz(&f[0..3], lineno + 1)?;
        surface_points.push(SurfacePoint::new(xyz, id_of(f[3].trim())?));
    }

    // orientations: X,Y,Z,azimuth,dip,polarity,formation
    let ori_text = read_to_string(dir, "input_orientations.csv")?;
    let mut orientations = Vec::new();
    for (lineno, line) in ori_text.lines().enumerate() {
        if lineno == 0 || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 6 {
            return Err(format!("orientations line {}: <6 fields", lineno + 1));
        }
        let xyz = parse_xyz(&f[0..3], lineno + 1)?;
        let az = parse_f64(f[3], lineno + 1)?;
        let dip = parse_f64(f[4], lineno + 1)?;
        let pol = parse_f64(f[5], lineno + 1)?;
        orientations.push(Orientation::new(xyz, azimuth_dip_to_pole(az, dip, pol)));
    }

    if surface_points.is_empty() {
        return Err("no surface points read".into());
    }
    Ok(Series::new(surface_points, orientations))
}

/// gempy azimuth/dip/polarity → pole (gradient) vector.
/// azimuth measured clockwise from North (+Y), dip from horizontal, in degrees.
///   gx = sin(dip)·sin(azimuth)·polarity
///   gy = sin(dip)·cos(azimuth)·polarity
///   gz = cos(dip)·polarity
/// Verified against gempy: (az=90, dip=0, pol=1) → [0,0,1].
fn azimuth_dip_to_pole(azimuth_deg: f64, dip_deg: f64, polarity: f64) -> [f64; 3] {
    let az = azimuth_deg.to_radians();
    let dip = dip_deg.to_radians();
    [
        dip.sin() * az.sin() * polarity,
        dip.sin() * az.cos() * polarity,
        dip.cos() * polarity,
    ]
}

fn read_series_order(dir: &Path) -> Result<Vec<String>, String> {
    let txt = read_to_string(dir, "series.json")?;
    let v: serde_json::Value =
        serde_json::from_str(&txt).map_err(|e| format!("series.json parse: {e}"))?;
    // Accept either {"order": [...]} or a bare array.
    let arr = v
        .get("order")
        .or(Some(&v))
        .and_then(|x| x.as_array())
        .ok_or("series.json must be an array or {\"order\": [...]}")?;
    Ok(arr
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect())
}

fn read_kernel_options(dir: &Path) -> Result<KernelOptions, String> {
    let txt = read_to_string(dir, "kernel_options.json")?;
    let v: serde_json::Value =
        serde_json::from_str(&txt).map_err(|e| format!("kernel_options.json parse: {e}"))?;
    let d = KernelOptions::default();
    let f = |k: &str, dflt: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(dflt);
    // gempy stores orientation/sp nugget separately; accept several key spellings.
    let grad_nugget = v
        .get("grad_nugget")
        .or_else(|| v.get("orientations_nugget"))
        .and_then(|x| x.as_f64())
        .unwrap_or(d.grad_nugget);
    let sp_nugget = v
        .get("sp_nugget")
        .or_else(|| v.get("surface_points_nugget"))
        .and_then(|x| x.as_f64())
        .unwrap_or(d.sp_nugget);
    Ok(KernelOptions {
        range: f("range", d.range),
        c_o: f("c_o", d.c_o),
        i_res: f("i_res", d.i_res),
        gi_res: f("gi_res", d.gi_res),
        uni_degree: v
            .get("uni_degree")
            .and_then(|x| x.as_u64())
            .unwrap_or(d.uni_degree as u64) as u32,
        grad_nugget,
        sp_nugget,
    })
}

/// Read the (N,3) grid.npy gempy evaluated, in world coords, preserving row order.
///
/// CRITICAL: gempy's `regular_grid.values` is **Fortran-ordered**, and `np.save`
/// records that in the npy header. A Fortran (N,3) buffer is column-major —
/// `[all x, all y, all z]` — NOT row-major `[x0,y0,z0, x1,...]`. Reading it with
/// `chunks_exact(3)` (C-order assumption) scrambles every coordinate (the bug
/// that made the field look constant per column). We honor `npy.order()` and
/// de-interleave accordingly.
fn read_grid(dir: &Path) -> Result<Vec<[f64; 3]>, String> {
    let path = dir.join("grid.npy");
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let npy = npyz::NpyFile::new(&bytes[..]).map_err(|e| format!("grid.npy: {e}"))?;
    let shape = npy.shape().to_vec();
    let order = npy.order();
    if shape.len() == 2 && shape[1] != 3 {
        return Err(format!("grid.npy shape {shape:?} — expected (N,3)"));
    }
    let flat: Vec<f64> = npy.into_vec::<f64>().map_err(|e| format!("grid.npy data: {e}"))?;
    if flat.len() % 3 != 0 {
        return Err(format!("grid.npy length {} not divisible by 3", flat.len()));
    }
    let n = flat.len() / 3;
    let pts = match order {
        npyz::Order::C => flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect(),
        // Fortran/column-major: element (i,j) lives at index j*n + i.
        npyz::Order::Fortran => (0..n)
            .map(|i| [flat[i], flat[n + i], flat[2 * n + i]])
            .collect(),
    };
    Ok(pts)
}

fn write_npy_1d(path: &Path, data: &[f64]) -> Result<(), String> {
    npyz::to_file_1d(path, data.iter().copied())
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Write a row-major dense matrix as a (rows, cols) C-order .npy.
fn write_npy_2d(path: &Path, data: &[f64], rows: usize, cols: usize) -> Result<(), String> {
    use npyz::WriterBuilder;
    let file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    let mut w = std::io::BufWriter::new(file);
    let mut writer = npyz::WriteOptions::new()
        .default_dtype()
        .shape(&[rows as u64, cols as u64])
        .writer(&mut w)
        .begin_nd()
        .map_err(|e| format!("npy header {}: {e}", path.display()))?;
    writer
        .extend(data.iter().copied())
        .map_err(|e| format!("npy body {}: {e}", path.display()))?;
    writer
        .finish()
        .map_err(|e| format!("npy finish {}: {e}", path.display()))
}

fn read_to_string(dir: &Path, name: &str) -> Result<String, String> {
    let p = dir.join(name);
    std::fs::read_to_string(&p).map_err(|e| format!("reading {}: {e}", p.display()))
}

fn parse_xyz(fields: &[&str], lineno: usize) -> Result<[f64; 3], String> {
    Ok([
        parse_f64(fields[0], lineno)?,
        parse_f64(fields[1], lineno)?,
        parse_f64(fields[2], lineno)?,
    ])
}

fn parse_f64(s: &str, lineno: usize) -> Result<f64, String> {
    s.trim()
        .parse::<f64>()
        .map_err(|e| format!("line {lineno}: cannot parse {s:?} as f64: {e}"))
}
