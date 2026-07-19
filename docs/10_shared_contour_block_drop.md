# 10 — Strategy-A shared contour drops a rock block (the "teal cone")

**Status:** FIXED (2026-06-25, Option B, uncommitted). Shared-tree `refine_shared_contacts` pass
(block drop) + `project_bed_caps` (inter-unit gap) — teal shows two full blocks meeting the unit
below on the bed (no gap), at default `min_depth=2`; native==wasm; all gates green (below).
**Date:** 2026-06-25. **Repro:** `stack.rs` test `diag_vertical_vein_three_beds` (`#[ignore]`).

## Fix implemented (Option B — shared-tree refinement)

The handoff's candidate 1 ("OR a single-surface `any_surface_crosses` into the build stop-criterion")
is INSUFFICIENT for the exact failure here, and the reason is the crux the diagnosis under-stated:
the teal slab (z∈[500,700]) is **thinner than the coarse [500,750] cell**, and its lower bed (z=500)
**grazes that cell's bottom face**. The graze is no strict edge sign-change, so the lone z=700 upper
bed is the cell's only crossing — a single planar QEF that fits with ~0 residual, so `build` (and a
single-surface gate) leaves the cell at `min_depth` regardless. A slab with no corner strictly inside
it collapses to one dual vertex no matter how the *crossing* flag is set.

The working fix is a WHOLE-CELL, QEF-independent **double-surface** force-refinement, the shared-path
companion to `refine_double_crossed`:

- `Scene::cell_double_surface(min, size)` (`octree.rs`): true when **two or more** distinct scene
  surfaces (primary contact + any contributor) straddle the cell — a surface counting as straddling
  when its sign is merely non-uniform over the 8 corners, **inclusive** (an exactly-on-contact corner
  is put on one fixed side by `inside`/`<= 0`). The inclusive test is what catches the z=500 graze
  (and box-bounded thin units) that the strict per-edge `edge_double_crossed` misses.
- `refine_shared_contacts(tree, scene, cfg)` (`octree.rs`): mirrors `refine_double_crossed` — splits to
  a fixpoint every leaf below `max_depth` that `cell_double_surface` flags. **Self-limiting**: two
  surfaces stop sharing a cell once it is finer than their separation, so only the near-seam /
  thin-slab regions refine (teal's z∈[500,700] band separates z=500 from z=700 at depth 3 = 125-cell),
  NOT a global floor. Far cheaper than the `min_depth=4/5` brute-force proof.
- Wired into `octree_dual_contour_shared` and `octree_dual_contour_shared_tagged` ONLY (after
  `refine_double_crossed`, before `balance`). The single-scene `octree_dual_contour_scene` (Snap path)
  is untouched → byte-identical, as required.

Determinism: the new pass collects leaves into a `HashMap`, then `to_split.sort_unstable()` before
splitting (same as `refine_double_crossed`), so native↔wasm stays bit-deterministic.

### Verification (all green, 2026-06-25)
- `diag_vertical_vein_three_beds`: teal = two full blocks at default `min_depth=2` (tris 4758→7304,
  the recovered left block).
- `a2_crease_projection_smooth_watertight_fused`, `cross_cut_solid_seam_is_watertight` (the latter on
  the DIPPING fixture — all 3 units present, each watertight, body∪rock fused inc≥3=8164, 0
  see-through): pass.
- Full `geofield --lib` suite: 116 passed (Snap path byte-identical).
- Native→wasm parity: `npm run wasm:engine` + e2e `cross-cut`, `solids-toggle` (5), `seam-clip-toggle`
  (dipping): all pass.
- Dipping vein: its units are 300-thick (never susceptible to the thin-slab collapse); the
  geometry-agnostic pass leaves it watertight + present. No sibling diag needed.

NOTE: `balance` was NOT changed (its surface-leaf criterion stays `is_surface`); watertightness held
without it because the refined bed regions are balanced among themselves and their coarse neighbours
sit in surface-free z-bands. If a future scene cracks at a refined/unrefined seam, the escape is to
make `balance`'s surface-leaf test union-aware on the shared path.

The refine predicate (`Scene::cell_double_surface`) INCLUDES the domain box. A far-left cell touches
the `x=0` face, so box + a non-grid-aligned bed = two surfaces ROBUSTLY (both cross cell interiors,
no graze). An earlier variant that SKIPPED the box and relied on the grid-aligned z=500 bed's own
straddle — whose graze sign is build-unstable — DROPPED the block again in wasm. Keep the box.

## Follow-up — the inter-unit GAP at the binary-aligned bed (same session, FIXED)

With the blocks recovered, a thin horizontal gap appeared at z=500 (between the middle two units),
on BOTH sides of the vein. Cause: z=500 is dyadic (½ the domain) so it lands on a cell FACE at every
depth. The unit-ABOVE's bottom is a CAP (a `ContactHalfSpace` contributor); the contributor's
strict-`<` `edge_crossing` MISSES the corner graze, so that cap gets no QEF crossing and its vertex
floats to the cell centre (~½ cell above the bed) — while the unit-BELOW owns z=500 as its primary,
whose `<=`-inclusive `edge_crossings` DO catch the graze and pin it on the bed. The two units thus
separate by ~½ cell. z=300/700 are non-dyadic → cross cell interiors → pinned → no gap (only the
dyadic bed shows it).

**Fix: `project_bed_caps` (`stack.rs`), run on every rock unit in BOTH seam modes, before
`project_crease_ring`.** A move-only Newton (single constraint, `dp = -(field-iso)/|∇|²·∇`) that
lands each horizontal cap vertex exactly on its nearest bed `field == iso`. SELF-CALIBRATING
selection: a candidate is a horizontal face (normal ∥ ∇bed) NOT on a wall face, and it projects only
when its float is under HALF the inter-bed spacing — which is depth-independent (works at whatever
depth the refine reached, so it does NOT need the bed refined to max-depth), excludes box caps (their
nearest bed is a whole unit away), and guarantees the nearest bed is the right one. Move-only ⇒
connectivity untouched ⇒ watertightness + body↔rock fusion preserved; a pure function of position +
the analytic fields ⇒ native==wasm deterministic — it MASKS the graze non-determinism (the floated
input vertex differs across builds; the projected output is identical).

A WELD (`ContactHeightField::weld`, as the independent path uses) was tried FIRST and REJECTED: it
merges coincident verts and produced NON-MANIFOLD edges on the shared mesh (incidence > 2) — exactly
why the shared path omits the weld. The projection is the watertight-safe analog.

Verified native AND wasm (fresh `dist` — see the stale-preview gotcha below): each rock unit's bottom
sits on its bed = the unit-below's top, no gap; blocks full on both sides; all solids watertight.
Guards: native `cross_cut_vertical_three_beds_keeps_left_block` (block presence + gap-close +
watertight) and e2e `web3/e2e/cross-cut-gap.spec.ts`.

> ⚠️ **Stale-preview gotcha (cost ~hours).** `web3/playwright.config.ts` runs `npm run preview`
> (serves the built `dist/`) with `reuseExistingServer`, so after `npm run wasm:engine` the e2e/dump
> still hits the OLD build → looks like a native↔wasm divergence that isn't real. After rebuilding
> wasm, `npm run build` AND `lsof -ti:4173 | xargs kill -9` before trusting any browser result. A
> wasm rebuild that finishes in ~3 s did not recompile geofield (real build ~11 s).

## Symptom

In web3 solid mode on `cross-cutting-vein` (the **non-curved**, vertical vein), one middle rock
unit (the "teal" slab the vein passes through, sediments unit 2, z∈[500,700]) renders as a jagged
inward-collapsing **cone** instead of a clean slab. Present with the "clip seam" toggle BOTH on and
off. The other units (maroon/blue/olive) close correctly.

This is a *different, deeper* defect than the crease over-selection fixed in `docs/09` — that fix is
correct (clean 256-vert crease ring, moves ≤ cell, 0 reverted) but operates on a layer that cannot
fix a missing solid component.

## What was ruled out (don't relearn)

1. **NOT a native↔wasm divergence.** The earlier native fixture mis-sampled the beds (16-pt grid +
   1 orientation; web3's `threeContactFloor` is **2 pts/bed at x=200,800 + one [0,0,1] orientation
   per bed**, beds at z=300/500/700; vein = vertical walls x=400/600, 2 pts each + one [1,0,0]
   orientation per wall, `uni_degree=1`). With the EXACT fit, native per-unit triangle counts match
   wasm **exactly** (unit0=5920, unit1=4112, **unit2=4758**, unit3=8286). The octree + shared
   contour are deterministic → `A.1`/contributor-sign-snap stays refuted (cf.
   `volume-solid-seam-already-watertight`).
2. **NOT the crease projection.** Instrumented `project_crease_ring`: every unit selects a clean
   256-vert ring (`at_bed=256`), moves it ≤ cell, `reverted=0`. No over-selection, no cone from it.

## Root cause

`octree_dual_contour_shared` (the Strategy-A multi-scene contour behind
`mesh_cross_cut_solids_shared`, used by BOTH the clip-ON `_shared_clip_octree` and clip-OFF
`_shared_octree` web3 bindings) **drops a whole rock block** for the teal unit. An ASCII (x→, z↑)
occupancy of the unit mesh, native, exact fit, depth 6:

```
teal — SHARED contour (Strategy A)         teal — INDEPENDENT path (single-scene + weld)
   #  ##       ################            #################       ################
      ##       # # #  # #  #  #            ## #### #### ####       ### #### #### ##
   #  ##       #  #    #      #            ##           ####       ###          ##
      ##       #  #    #                   ## #  # #  # ####       ### #  # #  # ##
      ##       #                           #################       ################
  (left block GONE — only a sliver         (BOTH blocks full, clean rectangles,
   by the foot wall + the right block)      vein band x∈[400,600] correctly empty)
```

- The shared contour keeps only the **right** rock block (x≳600) plus a thin sliver against the
  foot wall (x≈350–400); the **entire left rock block** (x∈[0,~350], z∈[500,700]) is absent. That
  collapse to a sliver IS the cone.
- The **independent** path (`mesh_one_block_unit_solid`, single-scene `octree_dual_contour_scene` +
  `ContactHeightField::weld`) renders the SAME unit correctly: both blocks, full, symmetric.
- Same scene definition (upper bed = primary iso, lower bed = `ContactHalfSpace` contributor,
  vein `SlabComplement` masker, box) feeds both. So the defect is in the **multi-scene shared
  contour's cell handling**, not the scene/sign/masker. Blue (unit 1) keeps both blocks on the
  shared contour; only the teal middle unit loses one.

### CONFIRMED mechanism (2026-06-25): far-left UNDER-REFINEMENT

`octree_dual_contour_shared` (`octree.rs:1140`) builds ONE tree from `build_scene`
(`build`+`refine_double_crossed`+`balance`), then for each output scene does `resample_leaves`
(re-sample corner values + `inside`) + `contour_on_tree` — it **never re-refines per output scene**.
So the subdivision is whatever `build_scene` produced.

Subdivision happens where `build` finds `is_surface(corners, inside) = has_field_crossing(corners)
|| has_boundary_crossing(inside)` (`octree.rs:1307`), where `inside = contributors_inside` = the AND
of every contributor's `inside` (`octree.rs:436`). In `mesh_cross_cut_solids_shared`, `build_scene`
(`stack.rs:~1617-1641`) has primary = **body/vein field** and contributors that include
`body_wall_refine` — the vein walls as `ContactHalfSpace`es (`stack.rs:1581`). Those make the AND
`contributors_inside` **false everywhere left of the vein** (it requires `vein_field > hi`). So on
the far left:
- `has_field_crossing(corners)` = false (the vein field doesn't cross its iso out there), and
- `has_boundary_crossing(inside)` = false (the AND is uniformly false, so crossing a BED — a
  contributor that would otherwise flag the cell — never flips it).

⇒ `is_surface` = false ⇒ the far-left is left at `min_depth` (cell ≈ 250 at min_depth 2). A rock
unit whose entire z-band lands inside ONE coarse cell layer there cannot be resolved by a single
dual vertex and **collapses** (teal, z∈[500,700] ⊂ the [500,750] coarse cell). A unit that straddles
a coarse boundary (blue, z∈[300,500], boundary at 500) survives. Asymmetric, unit-specific — matches.

**Proof:** forcing `min_depth` up (bypassing `is_surface`) restores the block — at `min_depth=5` the
teal unit is two full clean blocks again (see the `diag_vertical_vein_three_beds` ASCII at
`min_depth` 2 / 4 / 5). The bug is purely under-refinement of the vein-left region, NOT the contour,
the scene signs, or the masker.

The earlier `cross_cut_solid_seam_is_watertight` passes because the malformed teal solid still
closes in isolation (its sliver + sloped faces self-cap) — watertightness is a weaker criterion
than "renders the right shape," so it didn't catch this.

## Validated fix (independent solids + crease projection)

Running the `docs/09` crease projection on the **independent** teal solid (instead of the shared
contour): `moved=136`, `maxd=15.6 (=cell)`, and **both blocks stay intact** (ASCII unchanged but for
the smoothed wall∩bed ring). So:

> Mesh the body + each cut rock unit via the INDEPENDENT path (`mesh_cross_cut_solid` +
> `mesh_one_block_unit_solid`, which are correct + welded), then apply `project_crease_ring` to each
> under `Clip`. Retire `mesh_cross_cut_solids_shared` from the web3 solid path.

The shared contour's only advantage over independent was body↔rock wall vertex-*sharing* ("fusion").
That was already shown cosmetic (`volume-solid-seam-already-watertight`): each independent solid is
watertight, the body and rock walls coincide at the same max-depth vein-field crossing (planar QEF
exact for the vertical vein), and the crease projection lands body + rock creases on the same
analytic `{vein=iso ∧ bed=iso}` curve. So switching loses nothing real on the vertical vein.

**Caveat (honest):** for the **dipping** vein, body and each rock unit contour the curved wall on
SEPARATE trees, so there could be residual sub-cell daylight mid-wall on the curve (the vertical
vein is clean; both force max-depth at the shared crossing). If it appears, it's a smaller separate
follow-up (e.g. project the wall surface too, or a one-tree body+rock-only contour).

## Direction decision — **OPTION B chosen** (2026-06-25)

Debug + fix the shared-contour under-refinement in place, keeping the shared tree (so body↔rock
wall vertices stay shared — Strategy A's advantage, incl. for curved walls). The root cause is now
confirmed (far-left under-refinement, above), so this is no longer "unknown effort": the fix is to
make `build_scene`'s refinement subdivide wherever ANY output surface (esp. a bed) crosses a cell,
not only where the solid's AND-boundary flips. Candidate approaches in the handoff prompt
(`docs/10_HANDOFF_shared_contour_fix.md`).

(Option A — switch the clip path to independent solids + crease projection — was the validated
fallback; not taken, but it remains the escape hatch if the shared-tree fix proves intractable.)

Trail of evidence + repro: `diag_vertical_vein_three_beds` in `stack.rs` (ASCII at min_depth 2/4/5,
z-histograms, shared-vs-independent compare, independent+projection validation). Cleanup of the dead
clip-based `mesh_cross_cut_solids_corefined` path is still pending regardless.
