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
