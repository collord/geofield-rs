# Contract: Geometric Invariants (the correctness oracle)

Per Constitution Principle I, imported-surface correctness is established against **geometric invariants + native↔wasm parity**, never "it looks right" and never a GemPy reference. These are the acceptance contracts the mesher output MUST satisfy; each maps to a Success Criterion and gets an invariant test (native) and/or a Playwright behavior test.

| ID | Invariant | Verified by | SC |
|---|---|---|---|
| INV-1 | Unit solids are watertight — zero open edges, zero non-manifold edges — for a domain-spanning import. | native invariant test (edge incidence) | SC-002 |
| INV-2 | Unit solids are watertight AND reach every domain box face they should — for an under-covering import (no hole from under-coverage). | native invariant test + partition sweep | SC-003 |
| INV-3 | Inside the mesh AABB, the stitched surface coincides with the imported triangles within tolerance (≤ octree cell size). | native test: sample points on input tris, measure distance to output surface | SC-004 |
| INV-4 | Across the AABB seam, the surface is continuous — no gap; dihedral crease ≤ stated angular tolerance. | native test: seam-edge incidence = 2, seam-vertex shared by index; normal-angle check | SC-005 |
| INV-5 | Consistent winding + gradient-coherent normals + flush to true extent (no geometry beyond `ext_max`). | reuse existing invariant checks (winding, extent-clip) | FR-010 |
| INV-6 | An imported surface in a role (erosion/veinWall/topography) reproduces the role's characteristic result (truncation / tabular body / Air cap). | native per-role test mirroring the fitted-equivalent invariant | SC-006 |
| INV-7 | Native and wasm produce byte-identical meshes for the same import. | pinned natively; asserted as a design invariant; Playwright cross-target | SC-007 |

**Guard vs oracle (Principle I)**: a test that merely re-runs our own mesher and compares to itself is a *guard*, not an oracle — labelled as such. The oracles here are the geometry-independent facts: edge incidence (watertight), distance-to-input-triangles (exactness), shared-by-index seam (continuity), and box-flush extent.

**Tolerances (to pin in tasks / spike)**: INV-3 deviation and INV-4 crease angle are calibrated against the octree cell size at the working depth and the engine's existing seam thresholds — stated as numbers in the tests, not left vague.
