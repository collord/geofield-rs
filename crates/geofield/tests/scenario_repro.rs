//! Reproduce the web2 scale scenario natively from public/scenario.json. Probe whether the
//! sediment contact sheets extend PAST the erosion surface (into the eroded region where
//! basalt is present) — the "thin wedges shooting out to the box corner, thousands of units
//! past the blue erosion" bug the screenshots show.

use geofield::octree::OctreeConfig;
use geofield::stack::{Relation, Stack, StackedSeries};
use geofield_kernel::assemble::{DecimateConfig, KernelOptions};
use geofield_kernel::data::{Orientation, Series, SurfacePoint};
use geofield_kernel::{Model, OneFault, PlanarFault};

/// The scenario.json relation code, mapped to the two-axis model. Code `3` (the old
/// `Basement`) and anything unrecognized denote the floor: no Axis-A relation and
/// `unbounded_above`. Returns `None` for the floor, `Some(rel)` for a bounded series.
fn relation_from_code(c: u64) -> Option<Relation> {
    match c {
        0 => Some(Relation::Erode),
        1 => Some(Relation::Onlap),
        2 => Some(Relation::Conformable),
        _ => None, // floor (was Basement)
    }
}

fn load_stack() -> (Stack<Model>, [f64; 6]) {
    let txt = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../web2/public/scenario.json"
    ))
    .expect("read scenario.json");
    let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
    let ext: Vec<f64> = v["extent"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let extent = [ext[0], ext[1], ext[2], ext[3], ext[4], ext[5]];
    let fault = &v["fault"];
    let fp: Vec<f64> = fault["point"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let fnv: Vec<f64> = fault["normal"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();

    let mut series = Vec::new();
    for s in v["series"].as_array().unwrap() {
        let sp_xyz: Vec<f64> = s["sp_xyz"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let sp_if: Vec<f64> = s["sp_if"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let ori_xyz: Vec<f64> = s["ori_xyz"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let ori_pole: Vec<f64> = s["ori_pole"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let faulted = s["faulted"].as_bool().unwrap();
        let relation = relation_from_code(s["relation"].as_u64().unwrap());
        // `None` (floor) → StackedSeries::floor; `Some(rel)` → a bounded series.
        let sps: Vec<SurfacePoint> = sp_xyz.chunks_exact(3).zip(sp_if.iter())
            .map(|(c, &id)| SurfacePoint::new([c[0], c[1], c[2]], id as u32)).collect();
        let oris: Vec<Orientation> = ori_xyz.chunks_exact(3).zip(ori_pole.chunks_exact(3))
            .map(|(c, p)| Orientation::new([c[0], c[1], c[2]], [p[0], p[1], p[2]])).collect();
        let sd = Series::new(sps, oris);
        let mut opts = KernelOptions::default();
        opts.decimate = Some(DecimateConfig { threshold_frac: 0.2, ..Default::default() });
        let model = if faulted {
            let f = Box::new(OneFault::new(PlanarFault::new([fp[0], fp[1], fp[2]], [fnv[0], fnv[1], fnv[2]])));
            Model::fit_faulted(&sd, f, opts)
        } else {
            Model::fit(&sd, opts)
        };
        series.push(match relation {
            Some(rel) => StackedSeries::new(model, rel).into(),
            None => StackedSeries::floor(model).into(),
        });
    }
    (Stack::new(series), extent)
}

#[test]
#[ignore]
fn sheets_past_erosion() {
    let (stack, extent) = load_stack();
    let [x0, x1, y0, y1, z0, z1] = extent;
    let size = (x1 - x0).max(y1 - y0).max(z1 - z0);
    let ext_max = [x1, y1, z1];
    let cfg = OctreeConfig { max_depth: 5, min_depth: 2, qef_threshold: 1e-6, cap: false };

    let young = stack.series[0].as_series().expect("series 0 is stratigraphic");
    let iso_top = young.iso_top().unwrap();

    // Dump every sediment-sheet vertex (x, z, signed-distance-to-erosion) + the basalt
    // erosion surface profile, so the "sheet poking above the erosion" region can be PLOTTED
    // and circled. Written to /tmp/sheet_dump.csv.
    {
        use std::io::Write;
        let mut out = std::fs::File::create("/tmp/sheet_dump.csv").unwrap();
        writeln!(out, "kind,block,iface,x,y,z,d").unwrap();
        for b in 0..stack.series_block_count(1) {
            for k in 0..3usize {
                let m = stack.mesh_one_block(1, b, k, [x0, y0, z0], size, ext_max, cfg).unwrap_or_default();
                let refd: std::collections::HashSet<u32> = m.indices.iter().copied().collect();
                for &vi in &refd {
                    let o = vi as usize * 3;
                    let p = [m.positions[o] as f64, m.positions[o + 1] as f64, m.positions[o + 2] as f64];
                    let fld = young.model.scalar_field(p) - iso_top;
                    let g = young.model.scalar_field_gradient(p);
                    let gm = (g[0]*g[0]+g[1]*g[1]+g[2]*g[2]).sqrt().max(1e-9);
                    writeln!(out, "sheet,{b},{k},{:.1},{:.1},{:.1},{:.1}", p[0], p[1], p[2], fld/gm).unwrap();
                }
            }
        }
        // Erosion surface profile along y = mid, per fault block: z where field_basalt = iso_top.
        // This is the SAME field the masker clips against — my "erosion reference". Tag each
        // sample as ON-SIDE (this block's own side of the fault — the part that's actually
        // rendered) or OFF-SIDE (extrapolated across the fault onto the other block).
        let ymid = (y0 + y1) * 0.5;
        for b in 0..stack.series_block_count(0) {
            let hs = <Model as geofield::BlockModel>::block_half_spaces(&young.model, b);
            let on_side = |p: [f64; 3]| -> bool {
                hs.iter().all(|h| {
                    let d = (p[0]-h.point[0])*h.normal[0] + (p[1]-h.point[1])*h.normal[1] + (p[2]-h.point[2])*h.normal[2];
                    if h.keep_positive { d >= 0.0 } else { d <= 0.0 }
                })
            };
            if let Some(bf) = <Model as geofield::BlockModel>::block_field(&young.model, b) {
                use geofield_kernel::ScalarField;
                for i in 0..=240 {
                    let x = x0 + (x1 - x0) * i as f64 / 240.0;
                    let (mut lo, mut hi) = (z0, z1);
                    let f = |z: f64| bf.scalar_field([x, ymid, z]) - iso_top;
                    if f(lo) * f(hi) <= 0.0 {
                        for _ in 0..40 { let mid=(lo+hi)*0.5; if f(lo)*f(mid)<=0.0 {hi=mid} else {lo=mid} }
                        let z = (lo+hi)*0.5;
                        let kind = if on_side([x, ymid, z]) { "erosion_field_on" } else { "erosion_field_off" };
                        writeln!(out, "{kind},{b},-1,{:.1},{:.1},{:.1},0.0", x, ymid, z).unwrap();
                    }
                }
            }
        }
        // The ACTUAL RENDERED erosion mesh (what the viewer draws as the blue surface) —
        // mesh_one_block(series 0 = basalt). Dump its vertices near the y=mid slice so we can
        // overlay it: if it coincides with the bisected field line, the field IS the real
        // erosion; if the rendered blue is absent/elsewhere in the east, the reference is wrong.
        let yslab = (y1 - y0) * 0.15;
        for b in 0..stack.series_block_count(0) {
            let em = stack.mesh_one_block(0, b, 0, [x0, y0, z0], size, ext_max, cfg).unwrap_or_default();
            let refd: std::collections::HashSet<u32> = em.indices.iter().copied().collect();
            for &vi in &refd {
                let o = vi as usize * 3;
                let (px, py, pz) = (em.positions[o] as f64, em.positions[o+1] as f64, em.positions[o+2] as f64);
                if (py - ymid).abs() < yslab {
                    writeln!(out, "erosion_mesh,{b},-1,{:.1},{:.1},{:.1},0.0", px, py, pz).unwrap();
                }
            }
        }
    }

    // A vertex is in the ERODED region (basalt present, should be removed) if
    // d = (field_basalt - iso_top)/|grad| is significantly NEGATIVE (above the erosion).
    for b in 0..stack.series_block_count(1) {
        for k in 0..3usize {
            let m = stack.mesh_one_block(1, b, k, [x0, y0, z0], size, ext_max, cfg).unwrap_or_default();
            if m.triangle_count() == 0 { continue; }
            let refd: std::collections::HashSet<u32> = m.indices.iter().copied().collect();
            // Cross-tabulate: of the FAR-EAST wedge verts (x > 0.8*x1), how many are PAST the
            // erosion (d<0, in the eroded zone, should be removed) vs BELOW it (d>0, kept)?
            // Report the wedge's z + d spread so we can locate it in the screenshot.
            let mut far_past = 0;
            let mut far_below = 0;
            let mut far_z = (f64::MAX, f64::MIN);
            let mut far_d = (f64::MAX, f64::MIN);
            let mut far_x_only = 0;
            for &vi in &refd {
                let o = vi as usize * 3;
                let p = [m.positions[o] as f64, m.positions[o + 1] as f64, m.positions[o + 2] as f64];
                let f = young.model.scalar_field(p) - iso_top;
                let g = young.model.scalar_field_gradient(p);
                let gm = (g[0]*g[0]+g[1]*g[1]+g[2]*g[2]).sqrt().max(1e-9);
                let d = f / gm;
                let east_frac = (p[0] - x0) / (x1 - x0);
                if east_frac > 0.8 {
                    far_x_only += 1;
                    far_z = (far_z.0.min(p[2]), far_z.1.max(p[2]));
                    far_d = (far_d.0.min(d), far_d.1.max(d));
                    if d < 0.0 { far_past += 1; } else { far_below += 1; }
                }
            }
            eprintln!(
                "sed block {b} iface {k}: {} verts | far-east(x>80%): {} verts, z∈[{:.0},{:.0}], d-to-erosion∈[{:.0},{:.0}] ({} PAST/above erosion, {} below/kept)",
                refd.len(), far_x_only, far_z.0, far_z.1, far_d.0, far_d.1, far_past, far_below
            );
        }
    }
}
