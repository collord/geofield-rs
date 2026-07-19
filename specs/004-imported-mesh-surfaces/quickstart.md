# Quickstart: Imported Mesh Surfaces — Validation Guide

Runnable scenarios that prove the feature end-to-end. Implementation details live in `tasks.md`; this is the run/validate guide. References: [data-model.md](./data-model.md), [contracts/](./contracts/).

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown`; `cargo` for native tests.
- `web3` deps installed; `npm run wasm:engine` (build both wasm artifacts) → `npm run build` (dist — **required before e2e**, preview serves `dist/`).
- Test fixtures: a few small `.obj` meshes under `web3/e2e/fixtures/` — (a) a domain-spanning bed, (b) a bed strictly inside the domain (for projection), (c) a winding-inconsistent mesh, (d) an empty/degenerate mesh.

## Spike first (the one real risk — D3)

Before broad implementation, validate patch-stitch in a native harness (the scratch-verify pattern):

1. Build a `MeshSdf` for fixture (b); contour it clipped to its AABB → the **exact patch**.
2. Fit the co-kriged field through (b)'s vertices+normals (decimated); contour it clipped to [outside AABB → box] → the **smooth patch**.
3. `mesh_clip::clip` the two at the AABB seam.
4. **Assert**: watertight (edge incidence), seam shared by index (INV-4), exact-inside within tolerance (INV-3), no spurious sheet (single surface component per unit).
5. Decide whether the no-op `refine` hook must be implemented for INV-4, and pick the decimation `point_budget`.

Gate: if the spike can't hit INV-2/3/4 on fixture (b), revisit D3 before building the element plumbing.

## Native validation

```
cargo test -p geofield          # includes the new imported-surface invariant tests (INV-1..6)
```

Expected: watertight domain-spanning + under-covering imports; exact-inside within tolerance; continuous seam; per-role behavior (erosion truncates, veinWall bounds a body, topography caps to Air). Native↔wasm parity pinned (INV-7).

## Behavioral (Playwright) validation — SC-009

```
npm run wasm:engine && npm run build      # rebuild dist AFTER any engine change
npx playwright test imported-mesh          # the new spec
```

Expected scenarios (from [contracts/protocol.md](./contracts/protocol.md)):

1. Import an `.obj` via the distinct mesh-import control (not the JSON Import) → an imported-surface element appears and renders.
2. Assign it the **erosion** role → older units truncate at the imported surface.
3. Assign it as **topography** → model caps to Air above / rock below.
4. Save then reload the project → the imported surface + role persist (FR-012).
5. Toggle Solids → the imported surface's units close watertight and export grouped by lithology.

## What "done" looks like

- All INV-1..7 pass; the Playwright `imported-mesh` spec is green against a freshly-built `dist/`.
- `docs/07_imported_mesh_field_proxy.md` amended (the deliberate Principle-II amendment: full-parity imports adopt a smooth far-field, confined to beyond the mesh bounds).
- Non-goals (formats beyond `.obj`, mesh repair, coordinate transform, concave coverage, imported faults) left as documented seams — none silently half-built.
