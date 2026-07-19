# Spec — Permissive exact-arithmetic mesh corefiner / boolean (`exact_arr`)

**One-line goal.** Replace the float-construction `mesh_clip.rs` corefiner with an exact-arithmetic
3D arrangement layer that is (a) geometrically exact (output vertices are the correctly-rounded
nearest f64 to the true intersection, not a tolerance-snapped approximation), (b) deterministic
native↔wasm by construction, (c) pure-Rust / wasm-native, and (d) permissively licensed (MIT/Apache,
no GPL/LGPL, no FFI). This is the "industry-standard exactness, owned outright" path. It is the
highest-effort option and the spec is written to make the effort *visible and stageable*, not to
hide it.

## Why this exists (the problem it actually solves)

The current `mesh_clip.rs` uses exact *predicates* (Shewchuk via the CDT) but *inexact constructions*
(intersection points computed in f64). CGAL's own docs name this exact failure mode: with exact
predicates and inexact constructions, edges split at intersections but the f64-rounded point position
can create self-intersections and inconsistencies between overlapping triangles. That is the root of
the wasm-determinism cascade in `docs/09` (the corefined-solid path produces garbage in the browser):
the octree→clip pipeline's float intersection points diverge sub-ULP across targets, and the keep/CDT
decisions cascade. Exact constructions remove the cascade at its source — the intersection point is
represented *exactly* (as the exact solution of the plane system), every predicate on it is decided
with certainty, and the only rounding is the final f64 snapshot for the GPU buffer, bounded by ½ ULP
*at the true location*.

This also unlocks the imported-mesh and B-rep-export futures (`docs/02`, `docs/07`): an exact
arrangement is the correct foundation for both, and for any certified-output requirement (volumes,
tonnage, regulatory sign-off) where "topologically manifold but geometrically unbounded" (the
Manifold/boolmesh guarantee) is not defensible.

## Non-goals / explicit scope boundaries

- NOT a general CAD kernel. No NURBS, no fillets, no offset surfaces. Triangle meshes only.
- NOT a replacement for the octree field-domain contour. The octree still produces the input
  surfaces; this layer corefines/combines them exactly. Field-domain CSG-union contouring stays as
  the fast path; this is the exact-seam path.
- NOT required to beat CGAL on performance. Target: correct + deterministic + fast-enough for the
  interactive solid-mode rebuild (seconds, not milliseconds). Performance hardening is a late stage.
- NOT a clean-room reimplementation of CGAL source. Work from the *papers* (cited below) and the
  permissive primitives only. Do NOT read GPL CGAL internals and reimplement them — that creates a
  derivative-work exposure on our own code. Algorithm-level learning from publications is fine;
  source-level reimplementation is not. (If GPL CGAL is used to generate a golden-output test corpus
  during prototyping, that test *data* is not a derivative work, but keep the implementation
  independent.)

## The numerical foundation (the part that is NOT the hard part)

The arithmetic primitives are available permissively; do not build these from scratch.

- **Exact predicates:** `geometry-predicates` (elrnv, MIT, no-std). orient2d/orient3d/incircle/
  insphere + the adaptive-precision floating-point *expansion* primitives, which it exposes
  explicitly "for extensions to other predicates or exact geometric constructions." This is the
  Shewchuk expansion arithmetic — the building block for both predicates and lazy-exact constructions.
  - Caveat from its README: predicates lose accuracy for |coords| < 1e-142 or > 1e201 (exponent
    overflow, not handled). At UTM/geological scales we are nowhere near this. Add a debug assertion
    that input coords are within a safe exponent band so this can never silently bite.
- **Exact rational fallback (for constructions the expansions can't close):** choose ONE, behind a
  trait, decided by Stage 0 benchmark:
  - **Pure-Rust (preferred for wasm/license):** `dashu` (rational + bigint, MIT/Apache, pure Rust,
    wasm-clean) or `num-rational`/`num-bigint`. Zero FFI, cleanest licensing, slower than GMP.
  - **GMP-backed (only if pure-Rust rationals are too slow):** `rug` (GMP/MPFR, LGPL, C FFI). Faster
    but reintroduces an FFI + LGPL dependency and complicates the wasm build — directly against the
    pure-Rust/wasm goal. Treat as a fallback to revisit only if Stage 0 shows pure-Rust rationals
    dominate runtime. **Default assumption: pure-Rust. Do not reach for GMP without a benchmark
    justifying it.**

**Lazy-exact strategy (the performance-critical design choice).** Do NOT compute everything in
rationals — that is correct but slow. Mirror CGAL's lazy/filtered kernel *concept* (from the papers,
not the source):
1. Carry each constructed quantity as an **interval** (f64 lower/upper bound) PLUS a lazily-evaluated
   exact representation (a DAG of the operations that produced it).
2. Decide every predicate on the interval first. If the interval is sign-unambiguous (the common
   case), you never touch rationals — full f64 speed.
3. Only when the interval straddles zero do you evaluate the exact DAG (expansions, escalating to
   rationals if needed) to get the certain sign.
This is what makes exact arithmetic tractable at mesh scale. It is also a real subsystem — see
"Where the multi-year risk concentrates" below.

## The algorithm (the part that IS the hard part)

Implement the Weiler-model / 3D-arrangement boolean. Primary reference:

- **Lévy, "Exact predicates, exact constructions and combinatorics for mesh CSG"** (arXiv 2405.12949,
  rev. 2025) — the cleanest published recipe; follow its structure.
- Supporting: **Cherchi et al., "Fast and Robust Mesh Arrangements using Floating-point Arithmetic"**
  (SIGGRAPH Asia 2020) and **"Interactive and Robust Mesh Booleans"** (2022) for the arrangement +
  radial-sort + ray-classification pipeline and the *deterministic* constrained-triangulation lineage
  (Livesu et al., "Deterministic Linear Time Constrained Triangulation using Simplified Earcut",
  2022 — directly relevant to the wasm-determinism requirement).

Pipeline stages (this is also the work breakdown):

1. **Exact intersection construction.** For every intersecting triangle pair, construct the
   intersection segment endpoints as *exact* points (each is the exact solution of the relevant
   plane/segment system, held lazily per above). This replaces `tri_tri_segment`'s f64 endpoint.
   Keep the existing combinatorial seam-keying idea (`mesh_clip.rs` already does
   `(C-feature, W-feature)` keying) — it composes well with exact points and gives exact shared-vertex
   identity for free.
2. **2D constrained triangulation with exact coordinates.** Per face, retriangulate to conform to the
   exact intersection polylines. Use exact incircle/orient2d. This is the "2D CDT with exact
   coordinates representing the intersections" that the Lévy paper calls its main contribution. The
   existing CDT (`cdt`/`spade`) gives exact predicates but inexact point coords — the change is
   threading exact coords through. Evaluate whether to extend the current CDT or implement the
   simplified-earcut deterministic CDT from Livesu 2022 (the latter is purpose-built for determinism).
3. **Arrangement / radial sort.** Build the Weiler model: around each non-manifold intersection edge,
   radially sort the incident facets using exact predicates. This is the step pairwise corefinement
   *cannot* do consistently and is the reason to go full-arrangement — it gives consistent triple
   points (wall∩bed∩box corners) by construction, which the current pairwise `clip` does not
   guarantee.
4. **Cell decomposition + classification.** Decompose space into volumetric regions; classify each
   region in/out per the boolean expression (union/diff/intersect, and the multi-operand CSG-stack
   expressions the geology needs). Cherchi's ray-cast classifier is the efficient method — note it
   *requires* the exact arrangement from step 3 (it explicitly cannot be applied to inexact
   arrangements). For the geology stack, classification is driven by the lithology/CSG expression,
   not a single boolean.
5. **Rounding + output.** Snapshot exact vertices to f64 for the GPU buffer (½-ULP correct rounding).
   Emit indexed mesh + the seam/shared structure the renderer and `mesh_test_util` invariants expect.

## Where the multi-year risk concentrates (read before estimating)

The arithmetic primitives (foundation section) are a few weeks. The risk is NOT there. It is here:

1. **Lazy-exact construction kernel with consistent caching across an operation chain.** The single
   hardest sub-problem and the reason CGAL took years. A construction that is exact *in isolation* can
   become *inconsistent* when its result is reused across multiple later operations (the same point
   referenced by two operations must produce identical predicate answers everywhere). Getting the DAG
   caching, the interval re-use, and the exact-fallback escalation consistent across a long chain is
   subtle, and the failure mode is silent (a rare inconsistency that tears one seam in one dataset).
   Budget the majority of the risk here.
2. **Coplanar / degenerate handling.** The current `mesh_clip` defers coplanar pairs (`docs/05`).
   Exact arrangement must handle them (coplanar overlap boundary in the shared plane), and degeneracies
   are where exact arithmetic earns its keep but also where the combinatorics get hard. Symbolic
   perturbation (Simulation of Simplicity) is the standard tool; it is itself a careful subsystem.
3. **The arrangement combinatorics (radial sort + cell decomposition).** Correct in the general
   position case is moderate; correct at every degeneracy (coincident triple points, facets meeting
   at a shared edge, near-tangent surfaces from under-constrained kriged fields) is where the long
   tail lives. The geology *generates* these (the dipping-vein 2-point fit is degeneracy-prone).
4. **Determinism proof obligation.** Exact arithmetic gives determinism *if* every float-path
   short-circuit (the lazy interval filter) makes the same decision on both targets. The interval
   bounds must be computed identically native↔wasm, or the filter takes a different branch and you are
   back to divergence. This needs the same hash-output-on-both-targets test from `docs/09`, applied to
   the exact pipeline, as a permanent gate.

Honest estimate framing: stages 1–2 (exact construction + exact CDT) are a *quarter* of focused work
to a usable prototype. Stages 3–4 (arrangement + classification) plus the degeneracy long tail and the
lazy-kernel consistency are the part that is *open-ended* — this is the "multi-year, you own
correctness forever" cost. Do not commit to it without Stage 0.

## Staging (each stage is independently valuable / abortable)

**Stage 0 — De-risk spike (1–2 weeks). Do this before anything else.**
- Benchmark pure-Rust rationals (`dashu`) vs interval-filtered f64 on representative intersection
  constructions to confirm the lazy strategy keeps the rational fallback rate low enough (target:
  <1% of predicates hit rationals on real geology data). If pure-Rust rationals in the fallback are
  fast enough at that rate, GMP is off the table — confirm or refute.
- Generate a **golden-output corpus** from GPL CGAL corefinement during this window (test data, owned,
  no GPL residue). This is the oracle for every later stage.
- Build ONE exact tri-tri intersection construction end-to-end (lazy interval + exact DAG + f64
  snapshot) and verify bit-identical output native↔wasm. This single result either validates or kills
  the determinism premise cheaply.
- **Gate:** if Stage 0 shows the lazy filter doesn't hold (rational rate too high → too slow) OR
  native↔wasm determinism doesn't hold even with exact construction, STOP and reconsider (CGAL-server
  or boolmesh-with-conditioning). Do not proceed to the arrangement layer on a broken foundation.

**Stage 1 — Exact pairwise corefiner (drop-in for `mesh_clip::clip`).**
- Pipeline stages 1–2 only (exact construction + exact CDT), pairwise (one C, one W), reusing the
  existing combinatorial seam-keying and the `ClipResult`/`shared` output shape so callers in
  `stack.rs` don't change.
- Validate against the existing `mesh_test_util` invariants (`assert_seam_watertight`) AND against the
  Stage-0 golden corpus for *geometric* agreement (output vertices within ½ ULP of CGAL's).
- This alone fixes the wasm-determinism cascade for the surface paths and is shippable independently.

**Stage 2 — Volume / multi-shell arrangement.**
- Pipeline stages 3–4: the Weiler arrangement, radial sort, cell decomposition, CSG-expression
  classification. This is the part that gives consistent triple points and replaces the
  `mesh_cross_cut_solids_*` machinery's fragility (`docs/09`, `docs/10`) with a principled exact
  assembly.
- Validate merged body∪rock watertightness (`assert_volume_watertight`) and golden-corpus agreement on
  whole solids.

**Stage 3 — Degeneracy hardening + perf.**
- Coplanar handling, Simulation-of-Simplicity perturbation, the degeneracy long tail. Fuzz against
  randomized + the known-degenerate geology fixtures (dipping-vein 2-point fit, grid-aligned grazes).
- Performance: spatial acceleration for the all-pairs intersection (the BVH from `docs/05`), parallelism
  where wasm allows.

## Acceptance (whole effort)

- Bit-identical output native↔wasm on a fixed corpus (the determinism gate, permanent).
- Geometric agreement with the CGAL golden corpus to ½ ULP on output vertices.
- All existing `mesh_test_util` invariants green; new exact-specific invariants (no self-intersection,
  verified with exact predicates) green.
- The `docs/09` / `docs/10` failure scenarios (corefined solid in wasm, teal-unit collapse) produce
  correct watertight output.
- License audit: zero GPL/LGPL/AGPL in the dependency tree; FFI-free (or, if `rug`/GMP was chosen in
  Stage 0 against the default, the LGPL/FFI implications are documented and accepted).

## Decision context (why you might NOT build this)

This is the right path ONLY if exact seam geometry is a true product requirement (certified volumes,
B-rep export, regulatory/acquisition diligence on the geometry itself). If the deliverable is
interactive visualization with authoritative numbers computed in the field domain, this is
over-engineering — boolmesh's topological robustness (pure-Rust/wasm/MPL, far less work) or the
field-domain contour + ring projection is proportionate. And the one-time cost of a CGAL commercial
PMP license (get the GeometryFactory quote) may undercut the multi-year build entirely, with a cleaner
acquisition-diligence story ("we licensed the industry standard" vs "we built our own exact kernel").
Build this when ownership of the exact engine is itself strategically valuable and the license/FFI
constraints rule out the alternatives — not by default.
