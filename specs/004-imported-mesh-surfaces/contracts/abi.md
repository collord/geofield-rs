# Contract: Engine ABI — `add_imported_surface`

The new wasm-reachable entry point, alongside `WasmStack::add_series` / `add_vein` / `add_intrusion` / `add_topography` (`crates/geofield/src/wasm.rs`).

## `WasmStack::add_imported_surface`

**Intent**: accept a raw imported mesh (as bytes) + its role, parse it, build the derived field-backed representation (exact `MeshSdf` + decimated co-kriged fit), and push a `StackElement::ImportedSurface` onto the stack.

**Inputs** (shape — exact wasm-bindgen signature finalized in tasks; keep args flat per the wasm-bindgen trailing-arg constraint):

- `mesh_bytes: &[u8]` — the `.obj` file bytes (browser reads the file; engine parses — D4). *Or* pre-flattened `vertices: &[f64]`, `faces: &[u32]`, `normals: &[f64]` if the parse is done just before the call; **bytes preferred** so the parser lives once in the engine.
- `role: u32` — `0=contact, 1=erosion, 2=veinWall, 3=topography`.
- `relation: u32` — `0=erode, 1=onlap, 2=conformable` (ignored unless `role ∈ {contact, erosion}`).
- `decimate_frac: f64` — the co-kriging point-budget lever (reuses the existing `add_series` decimation ABI, D2).
- fault args — passed through as the other elements do (`[0,0,0]` normal = no fault).

**Behavior / preconditions**:

- Parses `v/vn/f`; welds duplicate vertices; skips degenerate triangles.
- Normalizes winding per connected component (FR-008) — precondition for `mesh_clip::clip`.
- Rejects an empty/degenerate mesh with a clear error status (FR-011); reports (does not repair) holes / non-manifold / self-intersection.
- Fits the smooth field via `remote::fit` across the **existing** kernel wasm boundary (Constitution V — no new seam).
- Idempotent per identical input; deterministic (native↔wasm parity).

**Output**: mutates the stack (like the other `add_*`); the derived meshes come back through the existing recipe/surface mesh ABI, tagged as this element, so the viewer and OBJ export treat it exactly like a fitted surface.

## Contract tests (native)

- Import a mesh spanning the domain → element present; contour coincides with input within tolerance; watertight solids (SC-002/004).
- Import a mesh strictly inside the domain → surface reaches the box faces; watertight; seam continuous (SC-003/005).
- Import a winding-inconsistent mesh → normalized; result closed (FR-008).
- Import an empty/degenerate mesh → clear rejection, no crash (FR-011).
- Same input, native vs wasm → byte-identical mesh (SC-007, pinned natively + Playwright cross-target).
