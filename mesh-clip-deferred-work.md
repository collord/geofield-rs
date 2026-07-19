---
name: mesh-clip-deferred-work
description: "mesh_clip.rs (mesh-mesh seam corefine) — two pieces deliberately deferred past the first milestone; both are \"get to it\", not \"maybe\""
metadata: 
  node_type: memory
  type: project
  originSessionId: 9e829c9e-346b-4e77-a2b4-7dbbea65cbb3
---

The `mesh_clip` module (`crates/geofield/src/mesh_clip.rs`, design note
`docs/mesh_clip_design.md`) computes the true triangle–triangle intersection polyline between a
clipped contact C and a clipping wall/erosion W, re-triangulates each cut face with a CDT (the
`cdt` crate, exact Shewchuk predicates via `geometry-predicates`), and shares seam vertices by
index — replacing the staircased CSG-field-union and the scalloping `weld::snap_*`. Two pieces
were scoped OUT of the first milestone on 2026-06-22 and must not be lost:

1. **Import-side winding normalization.** `clip`'s `KeepSide` test assumes W is a coherently
   oriented sheet. Our octree-contoured W already is; IMPORTED W meshes must be winding-normalized
   BEFORE reaching `clip`. Wild-mesh complication the user flagged from experience: a single
   imported file can hold MULTIPLE PARTS whose normals do not share a semantic sense (one part
   "up", another "down"), so normalization is per-connected-component with a chosen reference
   sense, NOT one global flip. This belongs in the IMPORT path, not in `clip`. `clip` only
   asserts the precondition (sampled adjacent-normal agreement) and trusts it.

2. **Coplanar C/W triangle-pair handling.** Milestone 1 logs + skips a coplanar pair
   (`|n_c×n_w|` < angle eps). The user understands it and it's "not hard" — scheduled as the
   FIRST follow-up after Milestones 1–2 pass: standard coplanar branch = project both tris to 2D
   (drop dominant axis), compute 2D polygon overlap (edge–edge crossings + point-in-triangle),
   feed the overlap boundary as constraint edges into the same per-face CDT.

3. **Dipping-vein band-union slivers (KNOWN ISSUE, parked 2026-06-22).** On `cross-cutting-vein-dipping`
   with "clip seam" ON, the clipped sediment contacts show 1–8-triangle ISOLATED fragments (connected-
   component probe: beds have 3–6 components vs the correct 2; snap baseline = 2, 0 slivers). They cluster
   at the DOMAIN-RIM CORNERS (y≈0/1000) where the oblique seam meets the box face, plus a few genuinely
   INSIDE the band (x between the foot/hanging walls). The VERTICAL vein is clean — it's specific to the
   OBLIQUE wall. Root is the two-wall band handling: a vein clips C against foot wall (keep `field<lo`,
   Below) AND hanging wall (keep `field>hi`, Above) as TWO independent `clip` calls, then CONCATENATES
   (stack.rs ~1300, `clip_walls` loop). The two pieces have INDEPENDENT seam vertices, so near the band
   ends / box-rim corners a sub-triangle is orphaned (size-1 component). Keep-sides log correct
   (foot=Below, hang=Above); walls are healthy (full z-extent). Likely fixes: weld/dedup the two pieces
   by shared seam index, or clip the band as ONE `SlabComplement` instead of two halfspaces; the rim-
   corner orphans may also need T-junction tolerance work in `build_face_cdt_input`'s on-edge insertion
   (the `len2*1e-18` test, mesh_clip.rs ~624). NOTE: the `.all()`-over-cutters keep test
   (`retriangulate_c_face` ~545) is NOT the cause — tried nearest-cutter THREE times, zero effect.

**Why:** the user explicitly said "make a note so we get to it" for the coplanar case and "make a
note" for the multi-part import winding — these are cross-session obligations, not in-PR TODOs.
The dipping-vein slivers were parked by the user (tiny + rim-localized, bulk render correct) after
the erode-clip unification + vertical-vein normals fix landed and verified.
**How to apply:** when `mesh_clip` Milestones 1–2 are green, pick up coplanar handling next;
treat import winding as an import-path task gated by `clip`'s assertion. See
[[mesh-clip-corefine-design]], [[erode-seam-uses-field-csg-not-corefiner]].
