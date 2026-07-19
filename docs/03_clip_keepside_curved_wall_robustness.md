# Task 03 — Robust keep-side / empty-seam classification against a curved imported `W`

**Priority:** High. Depends conceptually on Task 02 (winding) but can be implemented in parallel.

## Context

`clip`'s `Keep::Side(KeepSide)` mode and its empty-seam branch classify a face by the **signed
distance to a W plane**:

- Cut faces: `retriangulate_c_face` (≈ line 567) keeps a sub-triangle when its centroid is on the
  keep side of *every* cutting W plane (`cut_w_planes.iter().all(...)`).
- Uncut faces and the empty-seam branch use `nearest_w_signed` (≈ line 698), which is the
  clamped-closest-point-on-triangle test — already the *good* version, with a careful doc comment
  explaining why a nearest-centroid + infinite-plane test mis-signs a bent W.

So the uncut path is robust, but the **cut-face path is not**: `cut_w_planes` are infinite planes,
and the `.all()` over them is exactly the logic the `wedge_gap` regression
(`bent_wall_*`, the comment ≈ line 1394) was added to defend. For a generated W the
`Keep::Predicate` (field) path sidesteps this — every production caller in `stack.rs` uses
`Keep::Predicate`, never `Keep::Side`. But an **imported, field-less** wall has no field to build a
predicate from, so it MUST use `Keep::Side`, and that is precisely the path with the weakest
classification.

The module comment already acknowledges this: the fix for the wedge bug "classifies each sub-triangle
against the NEAREST cutter" — confirm whether that fix actually landed in the cut-face path or only in
`nearest_w_signed`. From the current `retriangulate_c_face` it looks like cut faces still use
`.all()` over infinite planes, so the nearest-cutter fix may be incomplete on the cut path.

## What to fix

1. Make the cut-face keep test for `Keep::Side` use the **same clamped nearest-cutter classification**
   as `nearest_w_signed`, restricted to the W triangles that actually cut THIS face (the `segs`),
   rather than `.all()` over their infinite planes. This removes the splayed-neighbor wedge carve for
   the imported case, not just the generated one.
2. Provide a **geometric `Keep::Predicate` constructor for field-less walls** so imported meshes get
   the same robustness the field path enjoys: e.g. a helper that, given a winding-normalized `W`,
   returns `move |p| nearest_w_signed(W, p) >= 0.0` for the `Above` side (and the band-complement
   analog `nearest_signed(lo_wall) < 0 || nearest_signed(hi_wall) > 0` for two imported walls). This
   is the imported-mesh equivalent of the `SlabComplement` predicate the generated path uses.
   Document it as the recommended way to clip against imports.

## Concrete steps

1. Add a field-agnostic `signed_distance_to_mesh(w, p) -> f64` (the body of `nearest_w_signed`
   without the precomputed `w_planes`, or refactor so both share it). Keep it private to the module.
2. Rework `retriangulate_c_face`'s `Keep::Side` branch to classify each sub-triangle centroid by the
   nearest CUTTING W triangle's clamped-closest signed distance, not `.all()` of planes.
3. Add the public predicate constructor(s) for imported single-wall and band-complement cases.
4. Update `docs/mesh_clip_band_complement.md` to describe the imported (field-less) predicate
   alongside the existing field predicate, so the two are visibly parallel.

## Acceptance

- A field-agnostic version of the bent-wall wedge test driven through `Keep::Side` (NOT
  `Keep::Predicate`) — i.e. reproduce the wedge-carve on an imported-style splayed wall and assert it
  is watertight in BOTH bend orientations. The existing wedge test uses the field path; this one must
  exercise the `Keep::Side`/imported path specifically.
- A test that clips an imported (field-less) `C` against a curved imported `W` using the new
  geometric predicate and asserts no scalloping (anti-scallop check like
  `m2_staircase_watertight_and_no_scallop_field_agnostic`) and a watertight seam.
- Existing field-path tests unchanged.

## Watch-out

- `KeepSide` is signed by W's CCW normal, which only makes sense once Task 02's winding normalization
  has run. If you ship the geometric predicate, state in its doc that it assumes a winding-normalized
  W and ideally `debug_assert!` it (reuse `w_is_consistently_wound`).
