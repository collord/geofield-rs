# geofield-web

Interactive browser harness for `geofield-rs`: edit surface points and
orientations in tables, and watch the dual-contoured isosurface re-solve and
re-render live in a three.js viewport.

- **Upper area** — editable tables seeded from gempy's `jan_models/model1`
  (12 surface points, 2 orientations), in gempy's CSV columns
  (X,Y,Z,formation and X,Y,Z,azimuth,dip,polarity,formation).
- **Lower area** — three.js rendering: one **contact surface per interface**,
  dual contoured (in Rust/WASM) at that interface's *recovered isovalue* so each
  surface passes through its input points — the geological boundaries, matching
  gempy (not an arbitrary level set). Surfaces are colored by formation; surface
  points are spheres, orientation poles are arrows, inside the model extent box.
  Z is up.
- Edits **auto re-solve** after a ~300 ms debounce. An octree-depth slider (2–7)
  caps adaptive refinement.

## Architecture

```
edit tables (React)
   → solveAndMesh()  [model.ts]
      → WasmModel(...)            fit the cokriging system in WASM
      → mesh_isosurface(...)      uniform dual contouring (Rust src/mesh.rs)
   → Viewer (three.js)           BufferGeometry from positions + indices
```

The mesh is **adaptive octree dual contouring** over the WASM-evaluated scalar
field: refine toward surface detail (QEF residual), 2:1-balance, contour
crack-free by minimal-edge iteration. One QEF vertex per cell from the field's
analytic gradient (Hermite data). The **octree-depth slider** caps refinement
(finest cell = extent / 2^depth); flat regions stay coarse, curved regions
refine. (A uniform contourer also exists in the crate; the harness uses the
octree path.)

The harness meshes **one surface per interface** at the isovalue recovered at
that interface's input points (`WasmModel::interface_isovalues()`). For model1
the rock2 contact (isovalue +0.125) meshes to z≈600 and the rock1 contact
(isovalue −0.125) to z≈400 — passing through their points, as gempy renders them.
Verified: surface z-means land at 600.0 and 400.0.

## Running

The WASM package is built from the Rust crate into `src/wasm/` (gitignored).
Build it first, then run Vite:

```bash
# from this directory (web/)
npm install
npm run wasm     # wasm-pack build ../  --target web --features wasm → src/wasm/
npm run dev      # http://localhost:5173
```

`npm run wasm` requires `wasm-pack` and the `wasm32-unknown-unknown` target.
`npm run build` produces a static bundle in `dist/`.

## Notes

- Orientation azimuth/dip/polarity is converted to a pole vector with the same
  formula as `src/bin/geofield_cli.rs` (verified: az=90, dip=0 → [0,0,1]).
- Kernel options use the crate defaults (range 1.7, c_o 10, i_res 4, gi_res 2,
  uni_degree 1, nugget 1e-5) — passed as NaN to the WASM constructor.
- Degenerate inputs (e.g. too few points) surface as a solve error banner rather
  than crashing the viewer.
