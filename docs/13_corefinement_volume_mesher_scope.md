# Scope — corefinement-based (boundary-representation) volume mesher

**Purpose.** Scope a volume mesher that is *generally correct* — it produces watertight rock-unit
solids by **stitching the bounding surface meshes** (walls, beds, faults, topography, box), with **no
single-valued / monotonic assumption** about any surface. This is the architecture the field-domain
octree+seam path is an approximation of, and the path the peek-a-boo/junction work
(`docs/volume_mesh_wstar.md`) measured into being necessary. It builds on `docs/11` (exact corefiner
spec) and `docs/12` (Stage-0 result) and makes the choice concrete against the current code.

The framing question that motivated this (paraphrased): *the surface meshes look fine — why does the
volume mesher go back to the field instead of just identifying which surface segments bound a volume
and stitching them?* The answer below: that **is** the right approach; the "segments" don't pre-exist
(you must compute the surface intersections to create them), and that intersection step — done
**generally** (N-way, no monotonicity) — is the whole job.

---

## 1. What "stitch the surfaces" concretely requires

You cannot select-and-assemble patches directly: the wall mesh and a bed mesh are contoured
independently, so near their crossing their vertices are *different points*, not a shared curve. The
pipeline that turns "a pile of surface meshes" into "watertight labelled volumes" is:

1. **Corefine (arrange) the surfaces.** Compute every surface–surface intersection curve and **split
   every mesh along it**, so all incident surfaces share vertices on the curve. Generally this is
   **N-way** (wall, two beds, faults, box at once), because the hard cases are **triple points** —
   wall∩bed∩box corners — where three surfaces must agree on one vertex.
2. **Build the arrangement / cell complex.** Around each intersection edge, radially sort the incident
   facets (the Weiler model). This is what makes triple points consistent *by construction*.
3. **Classify cells by the geology's CSG/lithology expression.** Each volumetric cell → a label
   (rock unit / vein / Air). The geology is **multi-label**, not a single boolean — classification is
   driven by the lithology stack, naturally an arrangement+labelling problem (`docs/11:108-113`).
4. **Assemble + round.** For each label, gather its boundary facets into a closed shell; snapshot exact
   coords to f64 for the GPU buffer.

Steps 1–2 are the substance; 3–4 are comparatively mechanical. **None of this assumes any surface is a
graph over a plane** — it's purely combinatorial+geometric, so it handles arbitrarily contorted walls.

---

## 2. Current state (what exists, and the ceiling)

### What's already here and reusable
- **A pairwise float corefiner**: `mesh_clip::clip(c, w, keep, refine)` ([mesh_clip.rs:172](../crates/geofield/src/mesh_clip.rs#L172)).
  Corefines **one** contact `C` against **one** wall `W`, re-triangulates each cut face with a `spade`
  CDT (exact Shewchuk *predicates*, **inexact f64 constructions**), keeps one side of C, keeps all of W.
- **Combinatorial seam-keying** — the genuinely valuable asset. Each seam vertex is keyed on
  `(C-feature, W-feature)` where `Feature = Vertex|Edge|Face` ([mesh_clip.rs:803](../crates/geofield/src/mesh_clip.rs#L803),
  `SeamBuilder` ~860). Two computations of the same crossing derive the **identical** key → exact shared
  identity, **no tolerance**. This composes directly with exact coordinates.
- **`ClipResult { c, w, shared: Vec<(c_idx,w_idx)>, coplanar_pairs }`** ([mesh_clip.rs:153](../crates/geofield/src/mesh_clip.rs#L153)):
  separate buffers + the seam pairing; the caller unifies by index.
- **Solid assembly** `mesh_cross_cut_solids_corefined` ([stack.rs:1801](../crates/geofield/src/stack.rs#L1801))
  + `corefine_one_solid` ([stack.rs:2928](../crates/geofield/src/stack.rs#L2928)): co-contour wall-free
  host slabs on one shared tree, one `clip` per unit, component-classify the in-host wall strip,
  assemble, and **decline (`None`) unless `solid_watertight`** ([stack.rs:3038](../crates/geofield/src/stack.rs#L3038)).
- **Keep/classification**: `nearest_w_signed` (clamped-closest-point signed distance, bent-wall-safe),
  `keep_side_of_mesh`, `keep_outside_band` ([mesh_clip.rs:312,334,1163](../crates/geofield/src/mesh_clip.rs#L312)).
- **The exact-arithmetic foundation (Stage 0, default-off)**: `exact_arr` feature → `exact_tri_tri_segment`
  over `dashu` rationals, **proven bit-identical native↔wasm** (`docs/12`). Coplanar deferred; no lazy
  interval filter yet; **zero production callers**.

### The two ceilings (why production uses the field path instead)
- **Pairwise, not N-way.** `clip` takes exactly one C and one W. Triple points (wall∩bed∩box) are an
  **unhandled gap, avoided not solved**: `mesh_cross_cut_solids_corefined` contours the wall
  *overhanging the box* specifically to "dodge the edge-on-face degeneracy at the box∩wall∩bed corners"
  ([stack.rs:1793](../crates/geofield/src/stack.rs#L1793)). `docs/11:103-107` is explicit: consistent
  triple points are "the step pairwise corefinement *cannot* do consistently and is the reason to go
  full-arrangement."
- **Float constructions decline on sub-cell features.** On the under-constrained wall the surface waves
  *between* host triangles, so the float intersection never sees the crossing → the unit fails to close
  → `None` ([stack.rs:1796](../crates/geofield/src/stack.rs#L1796), `corefined_falls_back_on_wavy_wall`).
  This is the **same wall** the W\* ribbon couldn't seal — the junction is a genuine arrangement problem.

### The input-surface problem (orthogonal, and load-bearing)
Even a perfect boolean needs faithful inputs. The octree contour is **dual contouring = one vertex per
cell**, which *assumes the surface crosses each cell once*. A sub-cell **fold** (two sheets in one cell)
can't be represented — `octree.rs:35-37` names the fix as **Manifold Dual Contouring's per-cell vertex
split (Schaefer/Ju/Warren 2007), "left as a follow-up."** The 3-layer wall∩bed boundary measured in the
dipping fixture (`docs/volume_mesh_wstar.md`) is this: the dual-contour mis-represents a sub-cell fold.
So the bounding surface you'd hand a boolean engine is itself defective there. **This must be fixed
regardless of which boolean engine is chosen** (garbage surface in → garbage volume out).

---

## 3. The fork — three engine options (choose deliberately)

The arrangement+classification *algorithm* is the same idea in all three; they differ in **who owns the
robust geometry kernel** and **whether output is geometrically exact or perturbed**.

| | A — adopt a robust boolean engine | B — build the exact arrangement (`docs/11`) | C — license CGAL |
|---|---|---|---|
| Geometry | topologically watertight, **perturbed** (ULP-scale, not cell-scale) | **geometrically exact** (½-ULP-correct), deterministic by construction | exact, industry standard |
| Effort | **weeks–months** (integrate + surface-prep + wasm) | **multi-quarter → multi-year** (own correctness forever; `docs/11:117-145`) | weeks integ. + license cost |
| Determinism native↔wasm | must verify per-engine | **proven foundation** (`docs/12`); proof obligation at each stage | depends on build |
| License / FFI | all permissive (see candidates below) — Manifold Apache-2.0, Cherchi MIT, csgrs MIT, boolmesh MPL-2.0 | pure-Rust, MIT/Apache, FFI-free | **GPL** or commercial |
| Multi-label geology | as a sequence of pairwise booleans (works; more ops) | native multi-label cell classification | PMP corefinement, multi-label |

**Option-A candidates (verified 2026-06, live).** The decisive axis is **input tolerance** — does the
engine ingest the contorted/self-intersecting surfaces directly, or demand clean manifold input (which
re-imposes the §4 input-quality track *before* the boolean)?

| Engine | License | Lang / integration | Input requirement | Maturity |
|---|---|---|---|---|
| **[Manifold](https://github.com/elalish/manifold)** | Apache-2.0 | C++; `manifold-3d` npm (wasm, callable straight from the TS worker); Rust bindings `manifold-csg` | **clean manifold only** (`Merge` fixes *slightly* off; rejects self-intersecting soup) | battle-tested (OpenSCAD/Blender/web CAD) |
| **[Cherchi InteractiveAndRobustMeshBooleans](https://github.com/gcherchi/InteractiveAndRobustMeshBooleans)** | MIT | C++ (no off-the-shelf wasm/Rust binding — build it) | **arbitrary self-intersecting soup** — its whole point; same Cherchi/Attene lineage `docs/11` cites | research-grade (SIGGRAPH'22) |
| **[csgrs](https://crates.io/crates/csgrs)** | MIT | **pure Rust**, wasm-ready | BSP-based; `is_manifold()` is a check, not a guarantee | most-adopted pure-Rust (v0.20) |
| **[boolmesh](https://crates.io/crates/boolmesh)** | MPL-2.0 | **pure Rust** (`glam` only), wasm-clean | needs manifold input; Manifold-inspired | **v0.1.x, months old** — promising, a bet |

Key consequence: **Manifold/boolmesh guarantee watertight *output* but assume valid *input*** — a folded
wall is rejected, so they pair with the §4 prep. **Cherchi's arrangement ingests arbitrary contortion**
directly, so it's the closest match to "doesn't matter how contorted" — and adopting its MIT C++
(wasm-compiled) is a *middle path* between Option A and building `docs/11` from scratch (most of the
robustness for integration effort, but you own the wasm build and must prove native↔wasm determinism).

**Decision criterion (the only one that matters):** *is geometric exactness a product requirement?*
- **If the deliverable is interactive viz + authoritative numbers computed in the field domain** →
  Option A is proportionate. A robust boolean engine handles arbitrary contortion and gives watertight
  volumes for a fraction of B's cost. The domain's "tolerance geometry smears thin features" concern
  (memory `dipping-wall-fixed-peekaboo-needs-exact-corefiner`) is about **cell-scale** welds (hundreds
  of feet at model scale); a real boolean engine perturbs at **float scale** (sub-millimeter), which
  does *not* smear a 2-ft feature — so A is likely acceptable for the domain, **pending verification**.
- **If certified geometry is required** (volumes/tonnage for regulatory sign-off, B-rep export,
  acquisition diligence on the geometry itself) → "topologically manifold but geometrically perturbed"
  is not defensible (`docs/11:26-27`), and B (or C) is the path. `docs/11`'s own decision context
  (`:196-206`) says build B *only* when owning the exact engine is strategically valuable and
  license/FFI rules out A and C — not by default.

This is the question to settle **before** committing engineering. It's a product/strategy call, not a
technical one.

---

## 4. The orthogonal input-quality track (required for A, B, or C)

A correct boolean of a defective surface is still wrong. Independently of the engine:

- **Faithful wall surface at the feature.** Pick one: (a) **Manifold Dual Contouring** (per-cell vertex
  split — the `octree.rs:35-37` follow-up) so a sub-cell fold is meshed as two sheets, not a tangle;
  (b) **adaptive feature refinement** (`docs/12` Track B — subdivide until the feature is one-sheet-
  per-cell) — but note depth alone made the field-domain conform *worse*, so this needs the per-cell
  split or exact iso-surfacing, not just more uniform depth; (c) **better-conditioned fits** (more
  control points) so the field doesn't fold sub-cell in the first place (proven sufficient for the
  conform by the `well_conditioned` fixture).
- **Surface cleanliness contract.** A boolean engine wants clean 2-manifold (or coherently-wound)
  inputs. The repo already has the prep: `mesh::normalize_winding_per_component`
  ([mesh.rs:525](../crates/geofield/src/mesh.rs#L525)), `weld_coincident`, the T-junction welders
  (`weld.rs`). Reuse these as the pre-pass.

**Open question the de-risk spike answers:** is the dipping wall's z=650 problem a *true surface fold*
(needs the per-cell split / exact iso-surfacing) or a *meshing tangle* (cleanable by the prep above)?
That determines how much of this track is needed. (Offered earlier; still the cheapest decisive probe.)

---

## 5. Concrete next step — a days-scale de-risk spike (before committing to A or B)

Do **not** start building an arrangement engine cold. Run the cheapest test that discriminates the
options, on the two existing fixtures (`vertical_vein_over_column` degenerate, `well_conditioned_vein_over_column`):

1. **Export the bounding surfaces** the volume mesher would stitch — the combined wall
   (`combined_wall`, [stack.rs:1229](../crates/geofield/src/stack.rs#L1229)), the bed contact sheets
   (`mesh_surfaces`), and the 6 box faces — as `.off`/`.ply`, after running the existing winding/weld prep.
2. **Feed them to `manifold-3d`** (the npm/wasm build — callable directly from the worker, the
   lowest-friction probe) and ask for the watertight rock-unit volumes. It guarantees manifold output
   but **demands manifold input**, which makes it a sharp test of input quality.
3. **Measure** with the existing `assert_mesh_watertight` / `solid_watertight` invariants, per solid.
   - Well-conditioned closes → Option A is viable; the remaining question is wasm-build + determinism.
   - Manifold **rejects the dipping surfaces as non-manifold**, or the volumes don't close → the
     **input-quality track (§4) is the real blocker**, not the engine. Then the discriminator is: does
     the soup-tolerant **Cherchi arrangement** ingest the contorted surfaces directly (→ Option A with
     Cherchi, or its lineage built exact per `docs/11`), or is the surface itself a tangle that needs
     §4 repair first? That is the fold-characterization probe (§4 open question).
4. **In parallel**, the Stage-0 exact foundation (`exact_arr`) is already proven; Option B's next
   increment is `docs/11` Stage 1 (exact pairwise corefiner, drop-in for `clip`) — independently
   valuable (it fixes the float-decline + the wasm cascade for the *surface* paths) and a prerequisite
   to B's Stage 2 arrangement.

This spike costs days and tells you whether general correctness is a **library-integration** project
(weeks) or an **exact-kernel** project (quarters+) — which is exactly the fork in §3.

### SPIKE RESULT (2026-06-28, ran) — Option A validated for the closed-solid-difference workflow

Ran `manifold-3d` 3.5.1 in node/V8 on both fixtures. **The spike CORRECTED its own step-1 framing:**
feeding Manifold the *open* surfaces is wrong — Manifold wants **closed solids**. The right shape is
**closed-solid primitives differenced by the engine**: export the octree's watertight vein-**band solid**
(the `body` of `mesh_cross_cut_solids_shared`), build the unit slabs (flat beds → axis-aligned boxes),
and compute `slab_k.subtract(band)` per unit. Results:

- **Both bands accepted clean** — `ofMesh(band).status() = NoError`, genus 0, for BOTH the degenerate
  2-point fit and the well-conditioned fit. An engine-independent Rust self-intersection probe
  (`spike_band_self_intersection`, `#[ignore]`) confirms the **degenerate band is a CLEAN embedded fold,
  not a tangle**: 0 non-adjacent self-intersections, 0 collapsed-vertex clusters, even in the dense
  z=650/x≈675 fold region. So the dual-contour does NOT tangle in the *closed-solid* path.
- **All `slab − band` rock units close watertight** (Manifold's guarantee + independent edge-incidence
  check) and the **volumes partition the box EXACTLY** (band + 3 units = full 1000³) → no gap/overlap →
  **no peek-a-boo, including on the degenerate fixture the field-domain ribbon could not seal.**

**The reframe this forces:** the peek-a-boo (and W\*/conform/exact-arrangement) only exist because the
field path **re-contours the shared wall independently per solid and patches the seams**. Building
closed-solid primitives and letting a boolean engine difference them makes the wall **shared by
construction** — Manifold does the corefinement internally; no seam-patching, no exact arithmetic, no
corefiner to write. (The repo originally avoided a boolean engine because its *own* float corefiner
wasn't wasm-deterministic, docs/09 — a mature engine revisits that; node/V8 == the browser engine, so
determinism is plausible, **unverified**.)

**What this did NOT settle (untested generality):** only **flat-bed boxes** were exercised. Curved/
dipping beds (non-box slabs), faults, topography, erosion, the multi-unit lithology stack, and the real
**wasm-in-worker + native↔wasm parity** are all unproven. Next probes, cheap: (1) a curved-bed fixture
through the same closed-primitive difference; (2) the full multi-unit stack with lithology labels;
(3) `manifold-3d` in the actual web3 worker on a real scene. Evidence kept: `#[ignore]` tests
`spike_export_band_solids_obj` + `spike_band_self_intersection` in `stack.rs`; JS at `scratchpad/spike.mjs`.

---

## 6. Reuse / retire summary

- **Reuse:** the combinatorial seam-keying (`Feature`/`classify_in_triangle`/`SeamBuilder`), `ClipMesh`/
  `ClipResult`/`shared`, the `cdt_face` CDT wrapper, the `keep_*` predicates + `nearest_w_signed`
  classifier, the shared-tree host co-contouring, the `corefine_one_solid`+`solid_watertight`
  assembly/decline pattern, the winding/weld surface-prep (`mesh.rs`/`weld.rs`), and the `exact_arr`
  Stage-0 foundation.
- **Retire (eventually):** the field-domain seam patches whose correctness rests on a single-valued
  junction — `project_crease_ring`, `conform_caps_to_strip` (W\*), the shared-tree peek-a-boo
  machinery — kept only as the fast preview path, not the correctness path.
- **Build:** the N-way arrangement + consistent triple points + multi-label cell classification (the
  step pairwise corefinement structurally cannot do), via Option A's engine or `docs/11` Stage 2; plus
  the §4 input-quality track under whichever engine.

---

## 7. The honest bottom line

The current architecture is **pairwise + monotonicity-assuming by avoidance**; general correctness
requires an **N-way arrangement** that is indifferent to contortion. That algorithm is well-understood
and the repo already owns the hard-won pieces (exact seam-keying, exact tri-tri, surface prep). The
real decision is **§3's fork** — adopt a robust boolean engine (proportionate if the field supplies the
authoritative numbers) vs. build the exact kernel (only if certified geometry is the product) — and it
should be made on the product requirement, then de-risked with the **§5 spike**, before any arrangement
code is written. Either way, the **§4 input-quality (fold) track** is non-optional.
