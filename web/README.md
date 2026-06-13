# geofield-web

Interactive browser harness for `geofield-rs`: edit surface points and
orientations in tables, and watch the dual-contoured isosurface re-solve and
re-render live in a three.js viewport.

- **Upper area** — editable tables seeded from gempy's `jan_models/model1`
  (12 surface points, 2 orientations), in gempy's CSV columns
  (X,Y,Z,formation and X,Y,Z,azimuth,dip,polarity,formation).
- **Lower area** — three.js rendering: the zero-isosurface mesh (dual contoured
  in Rust/WASM), surface points as spheres, orientation poles as arrows, inside
  the model extent box. Z is up.
- Edits **auto re-solve** after a ~300 ms debounce. A grid slider (8³–96³) sets
  the dual-contouring resolution.

## Architecture

```
edit tables (React)
   → solveAndMesh()  [model.ts]
      → WasmModel(...)            fit the cokriging system in WASM
      → mesh_isosurface(...)      uniform dual contouring (Rust src/mesh.rs)
   → Viewer (three.js)           BufferGeometry from positions + indices
```

The mesh is **uniform dual contouring** over the WASM-evaluated scalar field,
placing one QEF vertex per sign-changing cell from the field's analytic gradient
(Hermite data). It is the first increment toward adaptive octree dual contouring;
no octree/adaptivity yet (the grid slider stands in for it).

`mesh_isosurface` renders a single isosurface at the zero level set. For model1
(horizontal layers) that sheet sits at z≈500, midway between the rock2 contact
(z=600) and rock1 (z=400) — verified.

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
