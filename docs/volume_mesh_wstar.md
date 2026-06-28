# W\* — shared wall triangulation for cross-cut solids (design decision)

Design doc for **Track 1 / W\*** (spec: the "shared wall triangulation" task). This is the
"decide-first" deliverable. It records the Strategy A vs B decision **with the current code grounded**,
and corrects two places where the idealized spec is lighter than the real machinery. Implementation
follows this doc; the gating measurement is called out so it runs the moment the partition produces
output.

**Decision: Strategy A** (contour the wall on the shared octree tree, partition by region) for the
first cut. Rationale at the end; B (reuse the surface path's `combined_wall`) is the imported-mesh
generalization, deferred.

**Domain reminder.** Viz / f32 / field-domain / octree only. NO exact arithmetic, NO `mesh_clip`
dependency (Strategy A), NO Track 2/3 coupling. The determinism investigation (docs/12) measured the
shipping contour AND `mesh_cross_cut_solids_shared` bit-identical native↔wasm via `CONTACT_GRAZE_EPS`
+ f32; W\* must preserve that (Acceptance, determinism gate).

---

## Current mechanism, grounded (what W\* replaces)

[`mesh_cross_cut_solids_shared`](../crates/geofield/src/stack.rs#L1498) builds, on **one shared
octree** ([`octree_dual_contour_shared`](../crates/geofield/src/octree.rs#L1179)):

- **Body band solid** — primary = the vein field at `body_upper` (the **hanging** wall iso); the
  **foot** wall enters as the `body_lower` `ContactHalfSpace` *cap* contributor
  ([stack.rs:1579,1655](../crates/geofield/src/stack.rs#L1579)). So the body's lateral surface is
  **two walls**, and only the hanging wall is the scene *primary*.
- **Each rock-unit solid** — primary = that unit's **bed** field; the vein enters as a
  [`Masker::SlabExterior`](../crates/geofield/src/stack.rs#L450) →
  [`SlabComplement`](../crates/geofield/src/octree.rs#L255) masker that keeps the rock *outside* the
  vein band `[lo, hi]` ([stack.rs:1666](../crates/geofield/src/stack.rs#L1666)). So each rock unit
  **re-contours both walls** as masker-crossing faces, against a *different* primary (its bed).

**The defect (spec's diagnosis, confirmed by the code):** the wall is contoured **two different
ways** — body-primary (`+∇`) vs rock-masker (`−∇`), against different primaries. For a **planar**
wall the QEF is exact → both land bit-identical (clean). For a **curved** (dipping) wall they diverge
sub-cell → two coincident wall triangulations interpenetrate → the speckle / "z-fighting" /
flaps. [`project_crease_ring`](../crates/geofield/src/stack.rs#L1983) only snaps the wall∩bed *ring*,
not the wall interior, so the two sheets persist between the rings.

### Two complications the spec underplays (must be in the design)

1. **The vein is a BAND — two walls, not "the wall".** Both the foot (`lo`) and hanging (`hi`) wall
   are body↔rock interface. A horizontal bed crossed by a dipping vein is split into a foot-side
   piece and a hanging-side piece; each borders one wall. W\* must share **both** wall surfaces and
   partition rock pieces to the correct wall.

2. **`contact_face` only tags the PRIMARY wall.**
   [`octree_dual_contour_shared_tagged`](../crates/geofield/src/octree.rs#L1212) tags
   `contact_face[t] = true` where the scene **primary** crosses. In the body scene that is the
   hanging wall *only*; the foot wall is a contributor-cap face (untagged). So "keep the wall faces
   via the tag" gets one wall. **W\* identifies wall faces GEOMETRICALLY** — normal ∥ `∇vein` and
   centroid on a wall iso — exactly the test
   [`project_crease_ring`](../crates/geofield/src/stack.rs#L2016) already uses
   (`|n·∇̂vein| > 0.7 ∧ |vein−iso|/|∇vein| ≤ tol`). That test catches **both** walls (both have
   normal ∥ `∇vein`) and is the W\* wall selector. The `contact_face` tag is still useful as a cheap
   pre-filter / cross-check, not the sole selector.

---

## What W\* is (Strategy A)

Contour the vein body once; treat **its wall faces as the single shared wall `W*`**. Do **not** let
rock units re-contour the wall. Assemble:

- **Body** keeps all of `W*` (oriented body-outward = wall `+∇`), plus its band caps and box faces —
  unchanged from today.
- **Each rock unit** is still contoured *with* its `SlabComplement` masker (to get the correct
  extent, bed caps, and box faces), then its **own wall faces are removed and replaced by the `W*`
  strip** bordering that unit (oriented rock-outward = wall `−∇`). Bed caps + box faces are kept —
  and note they **already match** across units (the diagnostic: same bed driver), so only the wall is
  replaced.

Every wall face then has incidence 2 (body `+∇` on one side, exactly one rock `−∇` on the other) with
**no coincident second wall triangulation** to interpenetrate. This is the docs/09 item-B / "W\*"
deferral, and the "contact-anchored single-evaluation" of the determinism discussion: the wall is
contoured once and consumed by reference, so body and rock cannot disagree about it.

### The wall∩bed boundary is the new frontier (and the gating measurement)

W\* makes the wall **interior** shared by construction. It does **not** by itself make the
**wall∩bed boundary** consistent: the `W*` strip's edge along a bed and the rock unit's bed-cap edge
along that bed are still two independently-triangulated curves that must meet at the wall∩bed line.
This is the **triple-point line** (wall ∧ bed ∧ unit-boundary). After W\*,
[`project_crease_ring`](../crates/geofield/src/stack.rs#L1983) **stops being a reconciler of two wall
sheets** and becomes a **smoother** of that shared ring onto the analytic `{vein=iso ∧ bed=iso}`
curve — but whether the wall-strip edge and the bed-cap edge actually share vertices there is the
open question W\* cannot answer a priori. **Hence the gating measurement.**

---

## Concrete steps (grounded; refines the spec)

1. **Wall extraction + partition primitive.**
   `fn partition_wall(body_solid, vein_field, wall_isos, &rock_slabs, cell) -> (WallFaces, Vec<FaceStrip>)`:
   - Select `W*` faces from the body solid by the geometric wall test (normal ∥ `∇vein`, centroid on
     a wall iso) — **both** walls. (Cross-check against the body scene's `contact_face` tag for the
     primary wall as a debug assertion.)
   - For each `W*` face, classify the rock unit bordering its `−∇` (back) side: offset the centroid a
     fraction of `cell` along `−∇̂vein` and test which rock unit's `[lower_iso, upper_iso]` bed slab
     it lands in (point-sample each `rb_field[k]` slab). The body borders the `+∇` (front) side of
     every wall face. A wall face whose back side is the body's own band edge (no rock behind) is a
     body-only face.
   - **Determinism:** the classification is a pure function of position + fields; collect strips into
     `Vec<Vec<usize>>` indexed by unit (no HashMap-iteration into output order). Audit per the gate.

2. **Body assembly** — already uses all wall faces; the change is to *retain* the extracted `W*` face
   set as the canonical wall and ensure steps 3 reference the same vertex indices (shared by identity,
   not re-emitted).

3. **Rock-unit assembly** — for each unit: contour with the masker (as today), **delete its wall
   faces** (same geometric selector), **splice in** that unit's `W*` strip with rock-outward winding
   (the same faces as the body's, reversed). The strip's vertices are the body's wall vertices
   (shared by index after a merge into the unit buffer, or referenced via a shared vertex pool).

4. **Re-scope `project_crease_ring`** — wall faces are now exactly the spliced `W*` strip, so wall
   identification is trivial (the strip is known, no per-face normal test needed for selection). Its
   role narrows to: smooth the **shared** ring onto the analytic curve. Confirm it moves an O(ring)
   set, not an O(cap) patch ([the docs/10 over-selection guard](../crates/geofield/src/stack.rs#L1995)
   stays); if it still moves large sets, the partition didn't share the ring → bug in step 1, not a
   reason to widen the projection.

5. **Adjacent rock units along the wall.** Where the wall crosses a bed contact, three faces meet
   (unit-k strip, unit-(k+1) strip, the bed contact). Because both strips are sub-strips of the SAME
   `W*`, they share the wall∩bed edge **by construction** — assert it (this is the triple-point line;
   the gating measurement lives here).

---

## THE GATING MEASUREMENT (run the moment steps 1–3 produce output)

Zoom the **wall∩bed triple points**. The wall interior is shared (clean by construction); the
question is whether the triple points are consistent:

- **Clean** (wall∩bed junctions match vertex-for-vertex, no tear/gap) → **W\* is sufficient**. Viz
  track is structurally correct; Track 3 (exact corefiner) stays a pure certified-geometry question
  with no structural debt pulling it forward.
- **Torn** (crack / T-junction / body·rock·bed disagree on the corner) → the **triple-point
  consistency problem** — what pairwise sharing does NOT fix and what exact-arrangement methods
  compute all-at-once to avoid. That is the **first measured evidence** the certified path needs exact
  arrangement — earned, not assumed — and it scopes Track 3.

Report this explicitly. Do not fold it into the watertightness pass; look at the junctions
specifically (zoom + count incidence-1 edges that are wall∩bed, not box-rim).

---

## Acceptance

- **Visual smell test** — dipping cross-cut vein renders clean (no speckle / interpenetration /
  flaps; wall is one surface between body and each rock). Before/after capture.
- **Merged watertightness** — `assert_volume_watertight` on the **merged body∪rock** solid: every
  wall face incidence-2 (body + exactly one rock); no incidence-1 off the box rim; **no incidence-3
  wall edge** (a surviving coincident second wall triangulation). The current path never asserts this.
- **`vein_body_meets_the_cross_cut_rock`** passes with wall-agreement tolerance at **float noise**,
  not `0.25*cell` — body and rock reference identical wall vertices, so the disagreement is *exactly
  zero*. Tighten the tolerance to assert it.
- **Triple-point measurement** reported (clean or torn).
- **Determinism gate** — hash the merged solid on native + wasm (as docs/12 did); bit-identical.
  Audit step 1's partition for HashMap-iteration-order leaks into index allocation (the docs/08
  hazard). **`Snap` mode must stay byte-identical to its current output** — W\* changes the
  `Clip`/shared assembly; `Snap` is untouched.
- Native and wasm `cargo test` green.

## Watch-outs

- **No render-layer fix** (polygonOffset / depth bias) — it would mask whether W\* actually removed
  the second wall.
- **No `mesh_clip` / exact corefiner** — Strategy A is pure octree. (B would route through `mesh_clip`,
  which is measured-deterministic, but it's still heavier and deferred.)
- **Orientation** — body uses wall `+∇`-outward, rock `−∇`-outward: the SAME faces, OPPOSITE winding.
  A flipped strip shows up as an incidence error in `assert_volume_watertight` — the invariant catches
  it.
- **Back-side sampling offset** (step 1) — far enough along `−∇` to land unambiguously in a rock slab,
  not so far it crosses into the next unit; scale to `cell`; the band-edge wall face (no rock behind)
  is body-only.
- **Crease projection honesty** — after W\* it is a SMOOTHER, not a reconciler. Large vertex moves ⇒
  the partition didn't share the ring (step-1 bug), not a reason to widen the projection.

## Why A over B

| | A (contour wall on shared tree, partition) | B (reuse surface-path `combined_wall`) |
|---|---|---|
| Machinery | existing shared-tree octree + the geometric wall selector already in `project_crease_ring` | the surface path's [`combined_wall`](../crates/geofield/src/stack.rs#L1229) corefined via `mesh_clip` |
| `mesh_clip` dep | **none** | yes (measured-deterministic, but heavier) |
| Determinism story | trivially intact (pure octree, already measured bit-identical) | intact but rides the clip path |
| Imported-mesh future | needs a later bridge | **closer** (an imported wall enters exactly here) |

A is the smaller intervention and keeps the determinism story trivial; B is the imported-mesh
generalization, recorded for when imported walls land. **First cut: A.**

## Open risks (honest)

- **The wall∩bed triple-point line** is the real unknown (the gating measurement) — W\* shares the
  wall *interior* by construction but not the wall∩bed *boundary*; the crease projection smooths it
  but doesn't guarantee shared vertices there.
- **Partition correctness at the band edges** (where a wall face has body on both sides, or grazes a
  bed) needs the back-side sampling to be unambiguous — a misclassified face is an incidence error,
  caught by the watertight assert but worth fuzzing.
- **Foot vs hanging wall assignment** — both walls pass the geometric selector; the partition must
  route foot-side rock pieces to the foot wall and hanging-side to the hanging wall via the `−∇`
  back-side slab test, not by wall iso alone.

---

# Junction sewing — the unsolved part (scope)

**Status correction (2026-06-26).** W\* (wall *interior* sharing) was built, wired into the Clip path
(`1e6fef0`), and **reverted** (`fbf75a9`) after a screenshot showed it OPENS each rock at the wall∩bed
junction (see-through holes — worse than the sliver). Two over-claims, corrected: the acceptance test
checked the **merged** solid (gap-free — wrong criterion; merging hides what separately-rendered rocks
can't), and the "99% coincident triple-point" was wrong. **The wall∩bed junction is the real
triple-point problem.** This section scopes its fix; nothing is implemented.

## The problem, measured (`wstar_naive_splice_opens_rocks_so_not_wired`)

Each rock unit is rendered as a SEPARATE closed solid (per-lithology colour), so each must be
individually watertight. Snap/production rocks: **0 open edges**. After the naive W\* face-swap:
**~200+ open edges per rock**, and **not weldable** — the body's wall strip (body verts) and the
rock's bed cap (rock verts) sample the wall∩bed curve with DIFFERENT triangulations (different vert
counts/positions), so a position-weld doesn't merge them. The strip's wall∩bed boundary and the
bed-cap's wall-side boundary are two distinct polylines of the same curve that must become ONE shared
polyline.

## Why the existing corefined path is NOT the drop-in answer

[`mesh_cross_cut_solids_corefined`](../crates/geofield/src/stack.rs#L1746) already does true fusion
(clip corefines the rock against the shared wall → seam shared by index, junction closed). But its
own test [`corefined_falls_back_on_wavy_wall`](../crates/geofield/src/stack.rs#L5860) shows it
**returns `None` on exactly the wavy 2-point dipping fit** (the peek-a-boo case): the under-constrained
wall waves sub-cell between host triangles, so `clip` never sees the crossing and a unit fails to
close. So it declines on the geometry we need — and the worker falls back to the shared path (the
peek-a-boo). The corefined path is the principled fusion but it can't resolve this wall; the octree
(W\*) can. So the fix is **octree W\* (resolves the wavy wall) + a junction-sewing step**, not the
corefiner.

## Two disciplines the corefined path has and my W\* integration lacked (adopt both)

1. **Per-SOLID watertightness, not merged.** The corefined tests assert
   [`assert_mesh_watertight`](../crates/geofield/src/mesh_test_util.rs#L76) on the body AND each rock
   individually. That is the criterion that catches open rocks; the merged check does not.
2. **Decline rather than ship a hole.** The corefined path returns `None` (caller falls back to the
   watertight independent path) if any solid fails to close. W\*+sewing must do the same: if the
   junction doesn't sew watertight, **fall back to the current shared path** (peek-a-boo, but closed)
   — never emit a holey solid.

## The sewing approach (grounded in an existing precedent)

The SURFACE path already sews a wall to contact creases:
[`free_boundary_points`](../crates/geofield/src/weld.rs) of each contact →
[`weld_points_into_mesh(&mut wall, &crease, weld_eps, cell)`](../crates/geofield/src/stack.rs#L1046)
(insert points into a mesh, splitting the edge they land on, so they become shared — T-junction
removal). The solid junction sewing is the analog: make the W\* strip and each rock's bed-cap boundary
**share the wall∩bed polyline** by mutual point insertion —
- weld the strip's wall∩bed boundary verts into the rock's bed-cap edges, AND
- weld the bed-cap's wall-side boundary verts into the strip's edges,
so every vertex on either polyline becomes shared on both → the rock closes. (Mutual, because each
polyline has verts the other lacks; one-directional welding leaves the other side's verts as
T-junctions.)

**Open risk:** mutual point-insertion may still leave residual T-junctions or near-degenerate slivers
where the two polylines interleave finely (the strip is finer than the bed cap — 704 vs 512 verts
measured). If it does not reach per-solid watertight, the alternative is a **constrained
re-triangulation** of the bed-cap's wall-side strip against the wall∩bed polyline (a 2D CDT in the
bed plane, the `mesh_clip` pattern) — heavier, and if *that* is intractable it is the first real
evidence the certified/exact-arrangement path (`docs/11`) is needed at the junction.

> **DE-RISK RESULT (2026-06-26, measured — `wstar_junction_sew_derisk`, commit `ebf362a`): the mutual
> point-insertion sew is INSUFFICIENT.** post-splice per-rock open ~[192,317,130] → post-sew
> ~[188,377,126], essentially unchanged. The welds DO insert (~340 of ~400 boundary points), but
> inserting one polyline's verts onto the *other's CHORDS* (straight bed-cap edges) lands them OFF the
> curved wall∩bed line, so they don't coincide and don't merge. **The two curve samplings cannot be
> zipped by insertion+weld — they need RE-TRIANGULATION to a merged vertex set (the CDT/arrangement
> option above).** This is measured evidence the wall∩bed junction is the genuine triple-point
> arrangement problem (correcting the "99% clean" claim). The remaining paths are the **field-domain
> CDT re-triangulation** of the bed-cap wall-side strip, or **the certified/exact-arrangement path**
> (`docs/11`) — both substantial; neither is a quick patch.

## The DE-RISKING measurement — do this BEFORE any production wiring (the gate I skipped)

On the dipping fixture (`vertical_vein_over_column`), after W\* splice **+ the junction sew**:
- **per-rock `assert_mesh_watertight`** (each rock individually closed, incidence-2 off the box rim) —
  the criterion that would have caught the regression;
- if any rock is not watertight → the sew is insufficient → **do not wire**; either escalate to the
  CDT option or accept the certified-path evidence.

Only if per-solid watertight passes: wire into the Clip path **with the decline-on-failure gate**, then
**render and look** (the visual smell test — load the dipping-vein scene, clip seam on, zoom the
junction) BEFORE claiming it done. The render is the acceptance gate the green test was not.

## Acceptance (corrected from the W\* acceptance above)

- **Per-solid watertight** — body AND each rock individually (`assert_mesh_watertight`), not the merged
  soup.
- **Decline-on-failure** — if the junction doesn't sew watertight, fall back to the shared path; never
  ship a hole.
- **Render-and-look** — the dipping cross-cut vein renders with the wall ONE surface AND the wall∩bed
  junction sealed (no notch). Capture before/after.
- **Determinism hash** — the W\*+sewn output bit-identical native↔wasm (the sew must be a pure function
  of position + fields; `weld_points_into_mesh` is deterministic if fed a deterministic point set).
- **Snap byte-identical** — W\*+sewing stays Clip-only.

## Process lessons (write these into the discipline, not just the code)

- Per-SOLID, not merged, watertightness — the renderer draws solids separately.
- **Look before claiming.** I claimed "peek-a-boo closed" off a green merged-gap test without rendering;
  the user's eyes were the gate. Render-and-look is mandatory before "done" on a visual fix.
- **Decline, don't degrade.** A closed solid with a known sliver beats an open solid with a hole.

---

# Field-domain RIBBON conform — built + measured (2026-06-27)

The junction re-triangulation (the task the DE-RISK above scoped) was built as `conform_caps_to_strip`
(`crates/geofield/src/stack.rs`, with the shared `build_chains` helper): after `splice_shared_wall`,
extract each rock cap's wall-side boundary chains and the strip's wall∩bed boundary chains, then ZIP a
thin triangle ribbon between them — keeping every original face, referencing the strip's own vertex
indices (so cap and strip share the junction by construction), partitioning each cap arc by nearest
strip fragment so foot/hanging take disjoint portions. It reuses `mesh_clip::cdt_face` (now `pub(crate)`).
De-risk gate: `wstar_cap_conform_closes_rocks`.

## Result (measured, per-SOLID raw incidence — the criterion `assert_mesh_watertight` uses)

| Fixture | per-rock open: splice → conform | verdict |
|---|---|---|
| **well-conditioned** (15-pt fit) | `[252,508,252] → [0,0,0]` | every rock + body PER-SOLID watertight, **zero non-manifold** ✅ |
| **degenerate 2-pt dipping fit** (the user's scene) | `[377,756,440] → [0,128,186]` | floor + z=350 close; **~314 strip-edge holes at z=650** |

**The ribbon conform is correct and robust** (zero non-manifold in raw index space; full closure of a
well-formed wall). Two corrections to the earlier framing:

1. **The "non-manifold" panic that derailed the first CDT attempts was a `merge_dedup` ARTIFACT.** The
   ribbon deliberately references the strip's own indices, which sit ON the cap's wall∩bed line; a
   position-dedup collapses those coincident-but-distinct verts into false incidence-3. The RAW per-solid
   incidence (what the renderer and `assert_mesh_watertight` see) has **zero** non-manifold. Always
   measure per-solid raw, never merge-deduped, for this.
2. **A full-cap region CDT is the WRONG decomposition.** Re-triangulating the whole cap forces
   reproducing the box perimeter, where sub-cell-jittered verts project to degenerate slivers that
   collide with the unchanged box-side faces (it broke even already-closed caps). The RIBBON — keep
   every face, only stitch the wall-side gap — never touches the box perimeter. This is the load-bearing
   architectural choice.

## Why the dipping fit DOESN'T close (the escalation evidence, now mechanism-pinned)

At the degenerate z=650 the strip's wall∩bed free boundary is **not a clean curve but a 3-LAYER FOLDED
band** — three coincident free-boundary chains (x≈675, z≈650, y∈[0,1000], 64 verts each). The
under-constrained 2-point fit (control points only at z=200/800) folds the dual-contour wall worst far
from its controls. A single cap chain cannot stitch to a 3-deep folded strip boundary.

**Deeper octree makes it WORSE, not better** (`wstar_conform_depth_sweep`): depth 6 → `[0,128,186]`;
depth 7 → `[0,759,0]`; depth 8 → `[757,1530,765]`. Finer cells resolve MORE of the spurious wave → more
fold layers. **So Track B (adaptive refinement) does NOT fix this junction** — the fold is intrinsic to
the FIT'S CONDITIONING, not the resolution.

## Conclusion / what closes the dipping junction

This is the **first measured evidence the exact-arrangement path is needed AT THE JUNCTION** — a
geometry/topology need (a clean wall∩bed arrangement on a folded wall), confirmed NOT to be a
determinism or resolution problem. Two ways forward, in cost order:

1. **Better-conditioned vein fit** (more control points across y AND z on each wall). PROVEN to work: the
   15-point `well_conditioned_vein_over_column` closes fully at depth 6 with the existing conform. This
   is a data/authoring fix, not a meshing fix — likely the cheapest real answer for the user's scene.
2. **Exact-arrangement path** (`docs/11`/`docs/12` Stage 0) — compute a certified wall∩bed arrangement
   that doesn't fold, then the ribbon (or a corefine) closes cleanly. The substantial certified-geometry
   investment, now earned by measurement.

Production is **unchanged** — the conform is NOT wired (the dipping gate isn't green; that scene must
decline per the discipline). The primitives + de-risk tests stand as the evidence.
