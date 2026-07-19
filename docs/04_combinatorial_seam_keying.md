# Task 04 — Replace the quantized seam weld with combinatorial seam keying

**Priority:** Medium. This is the refinement the module's own docs say is "planned" — it removes the
last spatial-hash fragility in an otherwise index-shared corefiner.

## Context

The module docstring (≈ line 19–32) describes the intended Step B as keying each seam endpoint "by
its combinatorial incidence (which W/C edge or vertex it lies on) so two triangles meeting at a seam
point agree on one shared index — **not a spatial-hash weld**."

But the implemented Step B (≈ line 175–190 and `SeamBuilder` ≈ line 454) does a **quantized
spatial weld**: `seam_quantum` (≈ line 927) is `extent * 1e-7`, and `SeamBuilder::intern` rounds
endpoints to that grid. The code comments are candid that this is "the backstop weld; the
combinatorial WEdge/CEdge keying is the planned refinement."

Why it matters for imported meshes specifically: the quantum is a fraction of the **global** extent.
An imported mesh with a large bounding box but fine local features (a long fault sheet, say) makes
`extent * 1e-7` coarse relative to the feature size near the seam, risking either (a) two genuinely
distinct seam points collapsing to one id, or (b) two computations of the *same* crossing landing in
adjacent cells and NOT collapsing — the latter re-introduces the T-junction the weld exists to
prevent. Generated meshes have well-conditioned extents; imports do not.

## What to build

Combinatorial keying for seam endpoints. Each triangle–triangle intersection endpoint lies on a
specific feature of each triangle: either ON an edge of C and an edge of W, or AT a shared
vertex, etc. Key the endpoint by the canonical pair `(C-feature, W-feature)` where a feature is an
edge (an unordered vertex-index pair) or a vertex (a single index). Two triangles computing the same
crossing reference the same `(C-edge, W-edge)` pair and therefore the same id — exact, no tolerance.

The `tri_tri_segment` function (≈ line 310) already knows which edges it crossed implicitly (it
computes `edge_cross` on specific edges in `tri_interval`). Thread that incidence information out of
Step A so Step B can key on it instead of on the position.

## Concrete steps

1. Extend the Step A return so each endpoint carries its incidence: which C-edge (or C-vertex) and
   which W-edge (or W-vertex) it lies on. `tri_interval`/`lone_vertex` already identify the crossed
   edges; surface those indices.
2. Replace `SeamBuilder`'s quantized map with a map keyed on the canonical combinatorial key. Keep
   storing the first-seen position (the existing "store unquantized, share id" behavior is correct —
   only the *key* changes).
3. Keep the quantized weld available as a fallback for the rare endpoint that is genuinely interior
   to both triangles (shouldn't happen for tri-tri segment endpoints, but guard it), and log if the
   fallback fires so you can tell whether the combinatorial key is complete.
4. Delete or down-scope `seam_quantum` once it's only a fallback.

## Acceptance

- The `m2_staircase_*` tests stay green (they are the watertightness backstop).
- A NEW test with a deliberately ill-conditioned extent: place the same seam geometry but translate
  the whole scene by a large offset (e.g. +1e6 in x) so `extent * 1e-7` becomes coarse relative to
  the seam feature size. Assert the seam is still watertight — this should FAIL on the quantized weld
  and PASS on combinatorial keying, demonstrating the import-relevant improvement.
- Determinism / native==wasm parity preserved (combinatorial keys are integer index pairs, so this
  should be strictly more deterministic than the float weld — assert identical output across targets
  if you have that harness).

## Watch-out

- Coplanar/colinear seam endpoints (a seam point exactly at a shared mesh vertex) need a canonical
  ordering so C-side and W-side agree on the key. Define the canonicalization once and reuse it on
  both sides.
