# Phase 1 Data Model: Imported Mesh Surfaces

Entities and their derived representation. Declared state is minimal (Principle III); everything else is core-derived and never stored twice.

---

## Imported Surface (declared)

A model element whose geometry comes from an external triangle mesh instead of fitted points/orientations. Occupies the same hierarchy slot as a fitted surface.

| Field | Type | Notes |
|---|---|---|
| `id` | stable id | element identity (as other elements) |
| `name` | string | user label |
| `role` | enum | `contact` \| `erosion` \| `veinWall` \| `topography` — which surface role it plays |
| `relation` | enum (when `role = contact`/`erosion`) | the ONE declared bit: `erode` \| `onlap` \| `conformable` — reuses the existing relation model, not a new flag |
| `mesh` | triangle geometry | vertices `[[x,y,z]…]` + triangle faces `[[i,j,k]…]` (+ optional vertex normals) — the imported bytes, parsed once |

**Validation rules** (structural, per FR-011; do NOT re-encode geological rules — Principle III):

- Mesh MUST have ≥ 1 triangle and ≥ 3 distinct vertices (else: rejected with a clear message).
- Degenerate (zero-area) triangles and duplicate vertices are tolerated (welded/skipped), not fatal.
- Holes / non-manifold edges / self-intersections are **detected and reported** (a warning surfaced to the UI), not auto-repaired (Non-Goal).
- Winding need not be consistent on import — it is normalized per connected component before use (FR-008).

**State**: an imported surface is inert until it is fit + meshed (like any element); editing the mesh (out of scope) or its role/relation re-fits on Apply, exactly as a geometry edit does for fitted surfaces.

---

## Coverage Region (derived)

The **mesh's axis-aligned bounding box** (`AxisBox`) — the boundary between exact geometry and smooth projection. Derived from the mesh vertices; never declared or stored. Drives the exact-vs-smooth patch split (D3) and the seam location.

---

## Field-Backed Representation (derived — the core of the feature)

Built by the engine from the declared mesh; never stored in the project (only the mesh + role/relation are). Three derived pieces:

1. **Exact field** — a `MeshSdf` over the winding-normalized `TriMesh`; its zero set IS the mesh. Used inside the Coverage Region.
2. **Smooth field** — a co-kriged `Model` fit through the mesh vertices (points) + normals (orientations), single interface, with `DecimateConfig.point_budget` bounding the solve. Used outside the Coverage Region, extrapolate-then-clip to the domain box exactly like a fitted surface.
3. **Stitched surface/solid** — the exact patch (contour of #1 clipped to the AABB) welded to the smooth patch (contour of #2 clipped to [outside AABB → box]) at the shared AABB seam via `mesh_clip::clip` (index-shared → watertight). This is what participates in unit derivation, clipping, lithology, and export — identical downstream to a fitted surface.

**Invariants on the derived representation** (the correctness oracle — Principle I):

- Watertight closed unit solids (zero holes / zero non-manifold edges) — SC-002/003.
- Inside the Coverage Region, the stitched surface coincides with the imported triangles within tolerance — SC-004 (the exact patch is `MeshSdf`, exact by construction; tolerance = octree resolution).
- Across the AABB seam, the surface is continuous (no gap; crease below the stated angular tolerance) — SC-005.
- Consistent winding, gradient-coherent normals, flush to the true extent — FR-010.
- Native↔wasm `f64` parity — SC-007.

---

## Relationships

- An **Imported Surface** sits in the same stack as fitted **Series**, **Vein**, **Intrusion**, **Topography** elements and interacts with them through the **same** youngest-present-wins / `clip_maskers` machinery (Principle III — one source of truth). Its `role`+`relation` determine how it clips and is clipped, reusing the existing per-role behavior.
- The **Coverage Region** and **Field-Backed Representation** are pure functions of the Imported Surface + the domain box; recomputed on fit, cached only as long as any element, never serialized.

---

## DTO round-trip (FR-012)

- Project JSON (`.geoproj`) gains an imported-surface element: `{ kind, id, name, role, relation?, mesh }`. The mesh geometry is embedded (or referenced) so the project reopens intact.
- The core `StackElementDto` (`schema.rs`) gains an `ImportedSurface` variant so `to_dtos`/`from` round-trip it, mirroring the other element DTOs. The **derived** field/representation is NOT in the DTO (it is recomputed) — Principle III.
