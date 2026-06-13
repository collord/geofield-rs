//! Dump native reference field values for the WASM browser harness to check
//! against. Run: cargo run --example dump_reference > examples/web/reference.json
use geofield_rs::assemble::KernelOptions;
use geofield_rs::data::{Orientation, Series, SurfacePoint};
use geofield_rs::Model;
fn main() {
    let sps = vec![
        SurfacePoint::new([0.0,0.0,1.0],0),
        SurfacePoint::new([1.0,0.0,1.3],0),
        SurfacePoint::new([0.0,1.0,1.05],0),
        SurfacePoint::new([1.0,1.0,1.35],0),
    ];
    let oris = vec![
        Orientation::new([0.25,0.5,1.1],[-0.3,0.0,1.0]),
        Orientation::new([0.75,0.5,1.25],[-0.3,0.0,1.0]),
    ];
    let m = Model::fit(&Series::new(sps,oris), KernelOptions::default());
    let pts = [[0.0,0.5,0.8],[0.5,0.5,1.1],[1.0,0.5,1.4],[0.3,0.7,1.2]];
    print!("{{\"points\":[");
    for (i,p) in pts.iter().enumerate() {
        if i>0 { print!(","); }
        let g = m.scalar_field_gradient(*p);
        print!("{{\"p\":[{},{},{}],\"field\":{},\"grad\":[{},{},{}],\"unit\":{}}}",
            p[0],p[1],p[2], m.scalar_field(*p), g[0],g[1],g[2], m.lithology_unit(*p));
    }
    println!("]}}");
}
