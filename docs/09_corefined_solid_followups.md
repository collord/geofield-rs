# Task 09 — Corefined cross-cut SOLIDS: open problems + path forward

**Status (2026-06-24, uncommitted).** Task 01 §4 (corefined closed-solid seam) is BUILT as
`Stack::mesh_cross_cut_solids_corefined` + the wasm binding `mesh_cross_cut_solids_corefined_octree`
+ web3 `collectSolids` wiring (try corefined → fall back to the independent path on decline). It
**works natively** (vein body + rock units watertight, no daylight, smooth wall∩bed seam on
well-conditioned walls) but does **not yet deliver in the browser** for the cross-cut-vein scenarios.
This note records what's wrong, what was found, and how to proceed.

## What the user sees (two problems, both on `cross-cutting-vein-dipping`, Solids mode)

1. **Stairstep edges where the flat conformant bed contact reaches the curved vein wall** — the
   wall∩bed seam is a per-cell octree crease, not the smooth corefined polyline.
2. **Systematically incorrect closure of the volumes against the domain (box) walls** — solids
   overhang / step against the domain faces instead of closing flush.

## Investigation findings (what is and isn't true)

- **`clip` (the corefiner) is SOUND.** `clip_clean_wall_shares_every_boundary_vert` proves a clean
  box slab clipped against a clean flat overhanging wall shares EVERY boundary vert (0 unshared).
  Leaks are never a clip bug.
- **Corefined succeeds NATIVELY at the web3 default depth (6).** `diag_dipping_corefine_by_depth`
  (run `cargo test --release -- --ignored --nocapture`): the dipping vein returns
  `Some(3 units)` at depth 6 (2.3 s) and 7 (7.5 s), with **clean in-box bounding boxes** (no
  overhang) and watertight solids. It **declines at depth 8** (one unit goes non-manifold, nm=2).
  So success is **depth-dependent and NON-monotonic** — a brittleness signal.
- **CRITICAL: in WASM the corefined path DECLINES for EVERY cross-cut scenario** — both the WAVY
  `cross-cutting-vein-dipping` AND the CLEAN grid-aligned `cross-cutting-vein` return `tagged=0` from
  the binding (verified via a temporary `console.log(tagged.length)` in `collectSolids`, chromium,
  depth 6). So **the corefined feature delivers NOTHING in the browser** — `collectSolids` always
  falls back to the independent path. (An earlier "vertical vein renders clean" screenshot was the
  FALLBACK producing clean output for a planar wall, not corefined.)
- **The wasm corefined output is GARBAGE, not slightly-holed.** With the gate temporarily disabled
  (ship-anyway), the dipping vein renders as a chaotic mass of overlapping fans/sheets (the wall
  untrimmed, topology broken). So `solid_watertight` is correctly REJECTING genuinely-broken output —
  **(A) is NOT "relax the gate."** A tolerant gate would ship garbage.
- **Root cause: the octree→clip pipeline is not bit-deterministic native↔wasm.** Native and wasm
  IEEE-double basic ops match, but transcendentals / FMA / reordering produce sub-ULP differences
  that flip the octree's near-zero/grid-graze sign decisions (cf. the `CONTACT_GRAZE_EPS` fix). The
  INDEPENDENT path tolerates these (a vertex shifts a hair); the CLIP-based corefined assembly
  AMPLIFIES them — `clip`'s exact Shewchuk predicates are deterministic, but its float intersection
  points and centroid-based keep decisions diverge when the octree inputs differ by a hair, so the
  CDT keeps/sub-triangles cascade into broken topology. Even the grid-aligned clean wall trips this
  (its corners graze cell boundaries exactly).
- **Therefore both user-visible problems are the INDEPENDENT/fallback path's** behaviour
  (`mesh_cross_cut_solid` + `mesh_one_block_unit_solid`), the pre-corefiner state. Problem 1 is the
  field-CSG per-cell crease; problem 2 is the independent path's domain/box closure.
- **The vein field is genuinely WAVY** (the `cross-cutting-vein-dipping` 2-point under-constrained
  fit — both points at y=500, so the kriged wall waves in y). The wall *surface* is wavy regardless
  of meshing; corefining only smooths the intersection *curve* (wall∩bed), not the surface shape.
- **`on_box` uses the octree cube `size`, not `ext_max`** (`stack.rs`, in the corefined method and
  `corefine_one_solid`: `p[d] > min[d] + size - 1.0`). EXTENT is cubic today (0–1000³) so it's inert,
  but it is a **latent bug for a non-cubic domain** — the watertight gate would mis-locate the box rim.

## Root causes → suggested path (in priority order)

### A. Make the corefined path WORK IN WASM (UNBLOCKS everything; bigger than first framed). HIGH.
NOT "relax the gate" — the gate correctly rejects genuinely-broken wasm output. The corefined
ASSEMBLY is not bit-deterministic native↔wasm, so it produces garbage in the browser for every
scenario. Real options, roughly increasing soundness / effort:
1. **Make the octree+clip pipeline bit-deterministic native↔wasm.** Extend the `CONTACT_GRAZE_EPS`
   near-zero snapping to every corner/edge decision the corefined path's contours rely on, so the
   octree output is BIT-identical on both targets → clip (exact predicates + same-float ops on
   identical inputs) then matches too. This is the *root* fix and would harden the whole codebase,
   but it is open-ended whack-a-mole and may not fully close (transcendentals/FMA differences beyond
   grazes). Validate by hashing the octree mesh native vs a wasm dump.
2. **Pivot to a CLIP-FREE, wasm-robust smooth-seam approach.** The independent + Strategy-A
   (shared-tree) paths are wasm-robust (octree only, no clip). Keep Strategy-A's watertight shared
   wall and SMOOTH only the wall∩bed crease deterministically — e.g. snap the crease ring vertices
   onto the analytic intersection (`vein_field=edge ∧ bed_field=iso`, solved per-vertex by Newton on
   the two fields, fully deterministic) or a constrained Laplacian along the ring. Avoids `clip`
   entirely, so no float-cascade. Most promising for actually shipping; needs design.
3. **Native-only corefined for OBJ export.** If a server/native render path ever exists, corefined
   works there; the browser keeps the independent path. web3 is browser-only today, so this defers
   the visible win indefinitely.
**Verify**: the worker `[corefine] tagged=N` log must be non-zero for BOTH cross-cut scenarios in
chromium, and the rendered seam must be smooth (not the fallback staircase).

### B. True body↔rock wall fusion ("W*"). HIGH (also fixes the smoothness *visually*).
Even when corefined succeeds, the **body** is Strategy-A's `hosts[0]` (box-tree octree contour of the
vein iso) while each **rock** unit's wall is the strip of `combined_wall` (overhang-tree contour) —
two *different triangulations of the same iso-surface*. They coincide geometrically but aren't welded,
so at the body↔rock boundary they step/z-fight (re-creating a staircase look at zoom). Fix: build ONE
shared re-triangulated wall `W*` (`combined_wall` folded-clipped against every host so it carries all
bed/box/fault seams), partition `W*` by region, and have the body use ALL `W*` faces (flipped) and
each rock unit its band's faces — so every wall face is incidence-2 (body + one rock). Deferred from
the last turn; it is the only path to a topologically-shared (not merely coincident) wall.

### C. Robustness across depth + the overhang margin. MEDIUM.
Corefined works at depth 6/7 but fails at 8. Suspect the fixed `margin = 3*cell` overhang and the
component-majority strip selection interacting at fine resolution. Sweep depths, characterise the
nm=2 failure at depth 8 (re-add the `[unit FAIL]` diagnostic in `corefine_one_solid`), and make the
overhang/strip selection scale-stable.

### D. Diagnose problem 2 (domain closure) on its own terms. MEDIUM.
Once A lands and the corefined path actually runs in wasm, re-check whether the domain-wall closure
is fixed. If it persists, it's an **independent-path** issue (the fallback) — investigate
`mesh_one_block_unit_solid` (welded) vs `mesh_cross_cut_solid` (un-welded) closing the box
independently, and whether the vein body protrudes past the rock at the box face. Capture a
wireframe screenshot framed on the box∩wall∩bed corner.

### E. Fix the `on_box` `size`→`ext_max` latent bug. LOW (do alongside A).
In the corefined method and `corefine_one_solid`, the rim predicate should test `ext_max[d]`, not
`min[d] + size`. Inert for cubic domains; wrong for non-cubic.

## Deferred items carried from the previous turn (still open)

- **Pathological-wall early-out**: a wavy/under-resolved wall makes the corefined path run the
  expensive clips, *then* decline → the fallback pays both costs (slow). Detect under-resolution
  cheaply (e.g. wall facet-normal variance, or a coarse pre-pass) and skip straight to fallback.
- **Intrusion (`Single`/`Shell`) and faulted-block coverage**: coded (single-wall predicate;
  `PlaneHalfSpace` caps) but untested. Add fixtures + harness assertions.
- **Perf**: corefined re-mesh is ~6 s in wasm (vs ~1 s native; ~1–2 s native release) — heavier than
  the independent path. The `solids-toggle` e2e poll was bumped to 25 s to accommodate. Profile the
  clips (the overhang wall enlarges `|W|`) once A makes the path actually run.
- **Smoothness metric is weak**: `seam_ring_smoothness` returns a noisy `max_turn` (≈170° even on
  good output); the tests gate on watertightness + `ring>0`, not smoothness. A reliable seam-quality
  metric would let the harness *prove* the visual win, not just non-leakage.

## Update — findings from acting on the review (2026-06-24)

A detailed review of this note prompted three checks; all landed:

- **CONFIRMED: contributor signs are unsnapped.** `sample_corners` (octree.rs:1288-1289) graze-snaps
  only the PRIMARY field corner; `contributors_inside(p)` — every `ContactHalfSpace`/`SlabComplement`/
  box/plane `inside` = a bare `field < iso` — is NOT snapped. A contributor graze can flip
  `has_boundary_crossing → is_surface → which cells are leaves`, perturbing TREE TOPOLOGY (discrete),
  not just a vertex. This is a real determinism gap worth closing regardless of A.1/A.2.
- **CONFIRMED: native "success" is NOT real fusion — it's coincident sheets.** New harness assertion
  (`corefined_solids_watertight_fused_and_smooth`): of the body↔rock WALL edges, **fused(shared)=0,
  single(coincident-separate)=26347**. The body (`hosts[0]`, box-tree) and rock strips (overhang-tree)
  are different triangulations of the same iso, so they coincide but share ZERO edges. `open==0` /
  `assert_mesh_watertight` pass anyway because each solid is closed in isolation — a criterion WEAKER
  than rendering needs (coincident opaque sheets z-fight). So **B (W* fusion) is NOT a follow-on; the
  current corefined path never actually fuses body↔rock, even natively.**
- **The earlier "6.5× kernel divergence" was a FIT-MISMATCH ARTEFACT — the equitable check refutes
  it.** Dumping web3's EXACT dipping-vein fit (`sp_xyz`, `ori_pole`, …) exposed that the native
  fixture had a FLIPPED `ori_pole` z (−0.447 vs web3's +0.447) and a different column. Re-fitting the
  EXACT web3 arrays natively and fingerprinting the body solid (`mesh_cross_cut_solid`, octree only)
  vs the wasm body (`__surfaces` series 0 unit 0):
  - **native: tris=34724 verts=17364 poshash=662703939**
  - **wasm:   tris=34724 verts=17364 poshash=662703939** → **BIT-IDENTICAL.**
  So the octree contour of the kernel field is **fully deterministic native↔wasm** (to 0.01-unit
  poshash). **There is NO discrete octree divergence, and NO kernel-solve divergence**, for the same
  fit. The cokriging solve is reproducible across targets.
- **Therefore A.1 (snap contributor signs) is REFUTED, not rescued.** The octree — contributor signs
  included — is already deterministic here (the body solid exercises box + band-cap + masker
  contributors and still matches bit-for-bit), so snapping them changes nothing. The review's
  mechanistic point (contributor signs ARE unsnapped, octree.rs:1289) is correct as a *latent* gap
  worth closing, but it is NOT the cause of the wasm corefined failure.
- **The divergence that breaks corefined is CLIP-LOCAL float precision.** With bit-identical octree
  inputs, the only remaining difference is `clip` itself: sub-poshash (ULP) position drift amplified
  by clip's centroid-based keep decisions, and/or clip's own transcendentals (`keep_side_of_mesh`'s
  `sqrt` nearest-point, used by the component inside-host vote) resolving differently native↔wasm.
  This is continuous and clip-confined — exactly what A.2 (no clip) sidesteps and A.1 cannot touch.
- **FIXED now (the two correctness items):** `on_box` now tests `ext_max[d]` not `min[d]+size` (the
  non-cubic-domain bug), with a cell-relative rim; the dedup `q` is now `size*1e-9` (scale-relative,
  cf. `weld_coincident`), not an absolute `1e-6` that's below ULP at UTM coordinates.

**Net steer (settled by the equitable check): A.1 is OUT, A.2 is the path.** The same-fit experiment
the review asked for came back conclusively: the octree is bit-deterministic native↔wasm (no discrete
divergence to snap), AND the body↔rock "fusion" is fake (0 shared edges). The wasm failure is
clip-local float precision, which only A.2 sidesteps. So pivot to **A.2 (clip-free)**: Strategy-A's
shared-tree wall gives REAL fusion (shared verts, incidence-2) and is wasm-robust (octree-only, now
proven deterministic); smooth only the crease ring by analytic projection (`vein=edge ∧ bed=iso`,
per-vertex Newton, topology-preserving) — it cannot cascade and decouples seam-position from
mesh-connectivity. The contributor-sign snap (octree.rs:1289) remains a cheap latent-determinism win
worth banking independently, but it is NOT the rescue.

## A.2 IMPLEMENTED — works in wasm (2026-06-25)

Built and verified. `Stack::mesh_cross_cut_solids_shared` gained a `SeamMode` arg; under `Clip` it
runs `project_crease_ring` (stack.rs) after the shared-tree contour: a per-vertex Newton that moves
ONLY the wall∩bed crease verts (those within a cell of both a wall iso and a bed iso) onto the
analytic `vein=edge ∧ bed=iso` curve — minimal-norm step `dp = -Jᵀ(JJᵀ)⁻¹r`, with near-parallel
gradients (ill-conditioned) left in place. Connectivity is untouched → deterministic, no clip, keeps
Strategy-A watertightness + REAL fusion. Two no-flag wasm bindings (`mesh_cross_cut_solids_shared_
octree` = Snap, `_shared_clip_octree` = Clip), wired into web3 `collectSolids` gated by the "clip
seam" toggle (which now finally does something in solid mode).

- **Native** (`a2_crease_projection_smooth_watertight_fused`, the `cross-cutting-vein-dipping`
  geometry): Clip is watertight (`open==0`) + fused (`shared>0`) + same topology as Snap, and the
  body crease residual goes **snap=14.06 (≈ cell 15.6) → clip=0.00000** — crease verts land exactly
  on the analytic curve.
- **WASM**: the dipping vein renders 5 solids (body 34644 tris + 4 units) in ~4.3 s — **no decline,
  no fallback**. The vein is a smooth tabular body through the rock, no staircase, no daylight. This
  is the result the clip path could not produce in the browser. e2e green.

**Cap-collapse ("teal-unit cone") bug — FOUND + FIXED (2026-06-25).** First cut of `project_crease_ring`
selected crease verts by DISTANCE to both isosurfaces. But a rock unit's whole bed CAP lies on a bed
iso, so the bed gate was satisfied across the entire cap; only the wall-iso distance filtered, selecting
a 2-D STRIP of cap verts wherever the vein swept through the slab, and the Newton step collapsed that
strip onto the 1-D curve → the cap pinched into a cone (one unit per the user's screenshot). Confirmed
by a debug counter: `selected=640` (13% of verts, a patch not a ring), `max_disp=24.7` (≫ cell).
FIX: select STRUCTURALLY — a crease vert must be incident to a WALL FACE (triangle normal ∥ ∇vein +
centroid on a wall iso) AND within `tol` of a bed iso. The wall-face incidence excludes the bed-cap
interior (a cap-interior vert is on no wall face, even if near the wall by distance); the body has no
bed faces but DOES have wall faces, so it still projects its shared crease verts → fusion holds. Plus a
1.5·cell displacement guard (leave any vert whose step exceeds that). Post-fix: `selected` dropped to
O(ring) (640→384 on the worst unit), `max_disp ≤ 1.5·cell`, residual still 0, watertight + fused. Test
`a2_crease_projection_smooth_watertight_fused` now asserts a vertex-by-vertex max-move ≤ 1.5·cell
between Snap and Clip (cone-regression guard).

**Open polish (not blockers):** the wall *surface* still follows the wavy under-constrained field (a
modelling issue, not meshing); inter-unit bed-contact crispness (the old `ContactHeightField` weld
role) wasn't re-added — re-check if the horizontal contacts look chunky; normals at moved crease
verts are left stale (sub-cell move, negligible shading). Also: the dead `clip`-based corefined path
(`mesh_cross_cut_solids_corefined` + `corefine_one_solid` + their tests) is now superseded and
native-only — remove in a cleanup pass.

## Reusable assets already in place (don't rebuild)

`mesh_cross_cut_solids_corefined` + `corefine_one_solid` (stack.rs); `octree_dual_contour_shared_tagged`
(octree.rs, the per-triangle contact/cap tag — added for a body-cap idea later abandoned, but it's the
right primitive for the W* partition in B); the Stage-1 harness (`seam_ring_smoothness`,
`wall_strip_tiling`, `corefined_harness_sees_strategy_a_crease`); the soundness + scenario tests
(`clip_clean_wall_shares_every_boundary_vert`, `corefined_solids_watertight_fused_and_smooth`,
`corefined_vertical_grid_aligned_wall_terminates`, `corefined_falls_back_on_wavy_wall`,
`diag_dipping_corefine_by_depth` [#[ignore]]). See `docs/01_volume_mesh_clip_wiring.md`,
`docs/volume_mesh_seam.md`, and the `[[volume-solid-seam-already-watertight]]` memory.
