//! Minimal end-to-end example: fit a dipping single-layer model and sample the
//! field, gradient, and lithology.
//!
//! Run with: `cargo run --example single_layer`

use geofield_rs::assemble::KernelOptions;
use geofield_rs::data::{Orientation, Series, SurfacePoint};
use geofield_rs::Model;

fn main() {
    // One interface dipping in +x, with two matching orientations.
    let surface_points = vec![
        SurfacePoint::new([0.0, 0.0, 1.0], 0),
        SurfacePoint::new([1.0, 0.0, 1.3], 0),
        SurfacePoint::new([0.0, 1.0, 1.05], 0),
        SurfacePoint::new([1.0, 1.0, 1.35], 0),
    ];
    let orientations = vec![
        Orientation::new([0.25, 0.5, 1.1], [-0.3, 0.0, 1.0]),
        Orientation::new([0.75, 0.5, 1.25], [-0.3, 0.0, 1.0]),
    ];

    let series = Series::new(surface_points, orientations);
    let model = Model::fit(&series, KernelOptions::default());

    println!("solve residual      : {:.3e}", model.residual);
    println!("isovalue spread     : {:.3e}", model.isovalue_spread);
    println!("interface isovalues : {:?}", model.lithology().interfaces);

    println!("\n   x     y     z   |   Z(x)       ∇Z                          unit");
    println!("---------------------------------------------------------------------");
    for &z in &[0.8_f64, 1.1, 1.4] {
        for &x in &[0.0_f64, 0.5, 1.0] {
            let p = [x, 0.5, z];
            let f = model.scalar_field(p);
            let g = model.scalar_field_gradient(p);
            let u = model.lithology_unit(p);
            println!(
                "{:5.2} {:5.2} {:5.2} | {:9.4}  [{:6.3},{:6.3},{:6.3}]   {}",
                p[0], p[1], p[2], f, g[0], g[1], g[2], u
            );
        }
    }
}
