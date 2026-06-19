# geofield-web

Interactive browser harness for `geofield-rs`: edit a **stacked, multi-series
geomodel** — surface points, orientations, and the series/relation structure — in
tables, and watch the dual-contoured surfaces re-solve and re-render live in a
three.js viewport. Runs on the **two-WASM split** (EUPL kernel + BSD engine) with the
solve in a **web worker**.

- **Stack panel** — the ordered list of **series** (youngest at the top). Each series
  has a **relation** (Erode / Onlap / Conformable / Basement) that says how it interacts
  with the older series beneath it, and an optional per-series **fault** (strike +
  offset). This is the structure that makes the model "an erosional surface cutting a
  conformable stack" rather than a bag of surfaces — it mirrors web2's read-only
  relations schematic, but **editable**.
- **Tables** — editable surface points (X,Y,Z,formation,series) and orientations
  (X,Y,Z,azimuth,dip,polarity,formation,series), seeded from gempy's `jan_models/model1`
  (12 surface points, 2 orientations) as a single Basement series. Each row picks which
  series it belongs to; `formation` names its interface within that series.
- **Viewer** — three.js rendering: contacts dual-contoured (in Rust/WASM) at each
  interface's *recovered isovalue* so each surface passes through its input points, each
  series **clipped to its visible region** (the domain ∩ outside every younger erosion).
  Surfaces are colored by their series' relation family; data points are spheres,
  orientation poles are arrows, inside the model extent box. Z is up. An **object tree**
  groups surfaces by series → fault block and toggles their visibility.
- **Surfaces: contacts / closed solids** — flip the representation. *Contacts* are open
  sheets (one per interface). *Closed solids* are watertight lithology-unit slabs capped
  by the box, the fault, and every younger erosion (so an eroded unit daylights against
  the erosion surface). The flip is an **instant re-mesh** of the already-fitted stack
  (no refit).
- Edits **auto re-solve** after a ~300 ms debounce. An octree-depth slider (2–7) caps
  adaptive refinement.

## Architecture: the two-WASM split, driven from a worker

The codebase is split into two crates / two WASMs:

- **`geofield_kernel.wasm`** (EUPL) — the cokriging interpolation kernel (assemble /
  solve / evaluate). Imports nothing.
- **`geofield.wasm`** (BSD engine) — meshing (octree dual contouring) + multi-series
  stacking (`WasmStack`). It imports its field oracle from the wasm-import-module
  `"kernel"`.

```
edit tables + stack panel (React, App.tsx)
   → solveAndMesh() / remesh()  [model.ts]   — the worker CLIENT
        ↓ postMessage(run / remesh)
   ┌─ worker.ts ─────────────────────────────────────────────┐
   │  initKernel(geofield_kernel.wasm)   ← kernel first       │
   │  init(geofield.wasm); setEngineMemory(...)               │
   │  WasmStack.add_series(...) per series  (youngest first)  │
   │  mesh series → block → interface | present_unit          │
   │  ↑ engine's "kernel" imports resolve to kernel-bridge.ts │
   └─────────── stream SurfaceEvents (transferred) ───────────┘
        ↓ onmessage
   collect into MeshData → Viewer (three.js)
```

- **The `"kernel"` import is resolved by `kernel-bridge.ts`.** wasm-bindgen lowers the
  engine's `#[link(wasm_import_module = "kernel")]` imports to an ES import
  `import * from "kernel"`; `vite.config.ts` aliases the bare specifier `"kernel"` to the
  bridge, which owns the kernel instance and copies buffers across the two separate linear
  memories (the engine passes pointers into *its* memory; buffer-returning kernel calls
  are bridged through a kernel scratch buffer). This is what keeps the EUPL code in its
  own artifact: the engine never links it, it imports it.
- **The fit + mesh run in a web worker** (`worker.ts`), off the UI thread, and **stream**
  surfaces back transferred zero-copy (rendered progressively). The page never freezes.
- **Cancellation / supersede is a hard `worker.terminate()` + respawn** — a single fit is
  one uninterruptible wasm call, so killing the worker is the only way to stop it (and it
  is instant). A new edit supersedes any in-flight solve the same way. The cost is a ~1 s
  cold wasm re-init on the next run, accepted (editor fits are otherwise milliseconds).

The mesh is **adaptive octree dual contouring** over the WASM-evaluated scalar field:
refine toward surface detail (QEF residual), 2:1-balance, contour crack-free by
minimal-edge iteration; one QEF vertex per cell from the analytic gradient (Hermite
data). The octree-depth slider caps refinement (finest cell = extent / 2^depth).

## Running

The two WASM artifacts are built from the Rust crates into `src/wasm/` (engine, via
`wasm-pack`) and `src/wasm-kernel/` (kernel) — both gitignored. Build them first, then
run Vite:

```bash
# from this directory (web/)
npm install
npm run wasm     # builds BOTH: kernel (cargo → src/wasm-kernel/) + engine (wasm-pack → src/wasm/)
npm run dev      # http://localhost:5173
```

`npm run wasm` requires `wasm-pack` and the `wasm32-unknown-unknown` target.
`npm run build` produces a static bundle in `dist/` (the worker bundles to its own chunk).

## Notes

- `add_series` is called **youngest series first** (the order `WasmStack` fits the
  stack). Interface ids are assigned per series, in ascending first-seen order of
  `formation`.
- The editor fits without decimation (`decimate_frac = NaN`, gempy-exact) — its inputs
  are small. The web2 scale benchmark is where decimation matters.
- An **Erode** series clips the older series beneath it to *outside* the erosion body —
  which side is "the body" follows the erosion orientation's **dip/polarity sign**. A
  naively-signed erosion can clip the wrong side and make older contacts disappear; flip
  the orientation polarity if an eroded layer vanishes (this is the same pole convention
  web2's scenario generator applies).
- Orientation azimuth/dip/polarity → pole vector uses the same formula as
  `src/bin/geofield_cli.rs` (verified: az=90, dip=0 → [0,0,1]).
- Kernel options use the crate defaults (passed as NaN to the ABI); uni_degree = 1.
- Degenerate inputs (e.g. a fault that strands a series' data on one side) surface as a
  solve-error / note banner rather than crashing the viewer.
