# Engine & UI Roadmap — literature-grounded next steps

**Companion to `code_review.md`.** The review found that the engine composes *well-conditioned,
simple* stacks correctly but degrades — often silently — on *arbitrary* ones (many/mutually-
crossing bodies, imported meshes, under-constrained fits, large extents). This document surveys the
academic literature and comparable software to recommend next steps that (a) close those
reliability gaps at their root and (b) expand geological expressiveness toward parity with the
field.

**How to read this.** Part A is **engine completeness**, split into a *geometry/robustness
foundation* (which fixes the deepest review bugs) and *geological-method completeness* (faults,
anisotropy, uncertainty, folds). Part B is **UI/UX features** drawn from the reference tools. Part C
is a phased plan. Every recommendation cites a verifiable source; Part D lists them.

**Verification caveats carried from the research.** Vendor documentation hosts (Seequent, Intrepid,
Loop, Maptek) returned HTTP 403 to automated fetch, so their feature descriptions rest on
search-index excerpts of the exact official pages plus peer-reviewed papers where available; the
LoopStructural faults paper (GMD 14:6197) and Matt Keeter's 2026 meshing note could not be read
firsthand; and the maturity of the Rust Manifold/winding-number bindings named below must be
verified before adoption. These are flagged again inline where they matter.

---

## Progress & verification status (updated 2026-07-18)

A round of fixes landed on `main` (merged PR #1 + follow-ups) and was **independently verified** for
this update — code read directly, and `cargo build -p geofield-kernel -p geofield` compiles clean
with the kernel (44), obj (10), and `onlap_against_imported` test suites green. (The web3 TS
type-check and Playwright e2e could not be *run* here — `node_modules` absent — so TS/feature
verdicts are by code reading + spec cross-check.) The picture is now considerably better than the
review's baseline; the remaining work is the *foundation and geology* items, which were never in the
scope of these commits.

**Done & verified (this round):**

| Item | Review ref | Verdict |
|------|-----------|---------|
| Manifold N-body compose moved to a dedicated **persistent worker** (off the main thread) | §2.1 | ✅ Fixed |
| Monotonic **run-token guard** — superseded fit/compose results discarded | §2.2 | ✅ Fixed (beyond the commit's stated scope) |
| Manifold-**dropped units surfaced** as a warning; compose failure → error status, not empty-as-"done" | §2.3 | ✅ Fixed |
| `worker.onerror` wired on both workers, clears `applying` | §4.7 | ✅ Fixed |
| Non-finite `.obj` vertices/normals **rejected at ingress** (`ObjError::NonFinite`, tested) | §4.1 | ✅ Fixed |
| ABI `decode_fit`/`read_f64s` **bounds-validate** header counts (overflow-guarded, tested) | §4.3 | ✅ Fixed |
| Octree `partial_cmp().unwrap()` → `unwrap_or(Equal)`; `litho` `total_cmp`; `uni_degree` clamped at ABI edge | §4.6 | ✅ Fixed |
| Kernel **panic hook + `kernel_log` host sink**; `Model::fit_is_finite` fit-failure warning forwarded to UI | §4.1/§4.2 | ✅ Fixed |
| Onlap **pinches out against older imported** contacts/topography (exact mesh; tested) | §3.4 | ✅ Fixed |
| **Cross-section views + polyline digitizing** (feature 005) — §10/§11 acceptance satisfied; younging math correct (uses the corrected `m = normalize(ŝ × t̂)`, "youngs to your left" holds; feeds the existing `sp_xyz`/`ori_pole` fit path with **no kernel change**) | roadmap **B7 + B8** | ✅ Shipped |

*One honest correction the maintainers made (and I concur):* review §4.2's premise — "a singular fit
yields a NaN field" — **did not reproduce**; the nugget-regularized solve interpolates through even
pathological data. So a "fit-failure sentinel" would be near-dead code. The genuine silent failure is
the **finite-but-geologically-wrong under-constrained fit** (§1.3), which no fit-time scalar detects —
that reframes the open item toward **A2.3 (structural anisotropy/trend, which better-conditions the
fit)** and a geometric-fold detector, not an ABI channel.

**Closed in a follow-up cleanup (verified: `cargo test` green — kernel 48, geofield 135, onlap 1 — and
both crates build clean for `wasm32-unknown-unknown`):**

| Item | Review ref | Fix |
|------|-----------|-----|
| Three.js geometry/material **leak in the *primary* mesh-rebuild path** | §7 | ✅ `Viewer.tsx` now `disposeMeshGroupChildren(st.group)` (geometry + material, incl. wireframe) before `clear()` — mirrors `disposeOverlay`. *(TS not type-checked here — `node_modules` absent — verified by review.)* |
| **NEW residual (I surfaced):** `sym3_eig` raw `.unwrap()` reachable from a host-staged NaN coordinate | §4.6 | ✅ Two layers: `sym3_eig` sort now uses `f64::total_cmp` (never panics on NaN), and `decode_fit` **rejects any non-finite coordinate/pole at the ABI edge** with a `host_log` warning + empty fit (loud + attributable, like the `.obj` guard). Tested (`decode_fit_rejects_non_finite_coordinates`). |
| `gf_alloc`/`gf_free` capacity-mismatch UB | §4.4 | ✅ Both now use a shared fixed `Layout` (`buffer_layout`) so alloc and free agree exactly; `len == 0` handled. Tested (`gf_alloc_free_roundtrips_various_sizes`). |
| Uncapped `.obj` parse (hostile-large-file OOM before the fit cap) | §4.5 | ✅ `parse_obj` now enforces a byte/vertex/face budget (`ObjError::TooLarge`); byte cap is O(1) up front. Tested (`rejects_oversize_input`). |

**Still open (verified NOT addressed):**

| Item | Review ref | Status / note |
|------|-----------|---------------|
| `global_unit` doesn't hoist Topography ("rock in the sky") | §3.1 | ❌ Open — a public-API correctness landmine; **UI item B2** would also surface it. |
| Inert imported `VeinWall` role | §3.3 | ❌ Open — pairs with **UI item B5** (vein pinch-out/termination). |
| Single-body seam-quality paths; fitted-erosion drift; coplanar drop-C | §3.2/§3.5/§3.6 | ❌ Open — the composition-quality cluster; **A1.1/A1.5** are the structural fix. |
| The exact-predicate / determinism / thin-feature / winding-number **foundation** | §5.x/§6.x/§7-perf | ❌ Open — **all of Part A1.1–A1.5 remain**, and are now the top of the queue (below). *(Note: the `robust` crate — georust's Shewchuk predicates — is already a transitive dep, so A1.3 has a head start.)* |

**Net:** the *reliability-surfacing* layer (make-failure-loud, off-thread compose, stale-guard), the
*cross-section authoring feature*, and now the *robustness residuals* (§4.4/§4.5/§4.6-residual/§7) are
**done and verified**. The **geometry/robustness foundation (A1.1–A1.5)** and the **geological-method
completeness (A2)** remain the real roadmap. Phasing in Part C is updated accordingly.

---

## The single most important through-line

The review's two deepest defects — a **corefiner that guarantees watertightness by *checking then
declining*** (`code_review.md §5.2`) and **native↔wasm topological divergence** (`§5.1`) — have the
*same* root cause and the *same* cure. The root cause is **branching geometric decisions made with
scale-variant floating-point epsilons** (`mesh_clip.rs` `snap0` at `1e-12`, `COPLANAR_*`, the f64
`edge_cross`). The cure, established across 25 years of computational geometry, is **exact /
indirect geometric predicates feeding a global mesh *arrangement*** rather than pairwise f64
corefinement:

- **Shewchuk (1997)** — adaptive, correctly-rounded predicates: the sign of an orientation test is
  platform-independent *by construction*, dissolving the native/wasm divergence.
- **Zhou et al. (2016)** — resolve the arrangement of *all* input triangles at once, label cells by
  winding number, extract any boolean: correct for *any number* of inputs, no general-position
  assumption, no perturbation. This is the Manifold library the engine already links.
- **Attene (2020) + Cherchi et al. (2020, 2022)** — keep exactness but store constructed
  intersection points *symbolically* and decide signs through a floating-point filter with exact
  fallback ("indirect predicates"), making exact arrangements ~10× faster and interactive.

Adopting this foundation simultaneously fixes review findings **§2 (N-body composition only in
Manifold), §3.2 (single-body seam paths), §5.1–5.6 (the whole f64-corefiner robustness cluster),
and the native/wasm determinism risk** — one investment, viewed several ways. It is therefore the
top engine recommendation, and everything else builds on it.

---

## Part A — Engine completeness

### A1. Geometry & robustness foundation (fixes the review's deepest bugs)

**A1.1 [HIGH] Make robust, exact-predicate, N-body arrangements the *only* CSG path; retire the
pairwise f64 corefiner.**
Root-causes `code_review.md §2, §3.2, §5.1–5.6`. Two routes, not mutually exclusive:
- *Consolidate on Manifold.* The engine already links it for the recipe path; make it the sole
  boolean engine and delete the bespoke `mesh_clip` corefiner as a robustness-critical component.
  Maintained Rust bindings exist (`manifold3d` / `manifold-csg`) that build for both
  `wasm32-unknown-emscripten` and bare `wasm32-unknown-unknown` — directly serving the native+wasm
  requirement. *(Verify binding maturity/version before committing.)*
- *Adopt Cherchi 2020/2022 (indirect predicates)* where you need exact arrangements or faster
  interactive edits; both repos are MIT/header-only C++ and FFI the same way Manifold does. Caveat:
  Cherchi 2022 wants clean manifold inputs, whereas Manifold tolerates messier soups — geological
  solids with slivers/self-intersections favor Manifold as the front line.
  *Sources:* Zhou et al. 2016 (SIGGRAPH); Cherchi et al. 2020 (SIGGRAPH Asia), 2022 (SIGGRAPH Asia);
  Attene 2020 (CAD); Shewchuk 1997.

**A1.2 [HIGH] Replace the O(n) winding/point-in-mesh test with a BVH-accelerated *fast winding
number*.**
Fixes `code_review.md §7` (unaccelerated `winding()`), and — crucially — the generalized winding
number is **defect-tolerant**, so it keeps working on the not-guaranteed-watertight meshes the
engine currently emits (unlike a ray-parity test). Pair the BVH nearest-triangle (unsigned distance)
with the GWN sign for a robust signed distance field that avoids the "sign from nearest-triangle
normal" failures near thin features (relevant to `MeshSdf`).
*Rust:* `parry3d`/`bvh` give mature AABB trees; no standalone Rust crate for Barill's accelerated
GWN was confirmed — FFI the SideFX reference (Apache-2.0) or port its tree-expansion (a few hundred
lines) onto the existing BVH. *(Verify crates.io before assuming a crate exists.)*
*Sources:* Barill et al. 2018 (SIGGRAPH); Jacobson et al. 2013 (generalized winding number).

**A1.3 [HIGH, comes largely free with A1.1] Route every *branching* geometric decision through
exact/robust predicates and pin the native build to match wasm; add a native-vs-wasm golden-hash CI
test.**
Fixes `code_review.md §5.1`. Wasm f64 is strict IEEE-754 (no x87, no implicit FMA contraction), which
*helps* reproducibility — but only if the native build disables fast-math/contraction, avoids x87
(SSE2), and avoids `mul_add` in predicate paths unless identical on both targets. The pure-Rust
`georust/robust` crate compiles bit-identically to native and `wasm32-unknown-unknown` for any
remaining in-house predicate needs.
*Sources:* Shewchuk 1997; Attene 2020; `georust/robust`.

**A1.4 [LOW effort] Regularize + in-cell-clamp the QEF to kill vertex spikes.**
Fixes `code_review.md §6.2` (`project_to_contact` 2-cell excursion). A small Tikhonov term pulling the
vertex toward the cell-mass centroid plus a hard bounding-box clamp eliminates the spike class
without touching the pipeline. (The in-solver QEF is *already* clamped — the review's residual spike
is the post-solve Newton projection, which this bounds.)
*Sources:* Ju et al. 2002; Kobbelt et al. 2001.

**A1.5 [MED–HIGH effort] Detect sub-cell features and guarantee manifold output.**
Fixes `code_review.md §6.1` (thin veins missed) and `§5.2` (watertight-by-decline):
- *Feature detection:* drive octree refinement by a **feature-size / edge-normal** criterion, not
  just corner sign changes — sample edge-midpoints/faces so a sheet that enters and exits the same
  cell face without flipping a corner still forces subdivision.
- *Manifold guarantee:* move from plain DC to **Manifold Dual Contouring** (multiple vertices per
  cell, one per connected surface component; topology-preserving clustering) so output is manifold
  *by construction*, retiring the check-then-decline logic. Alternatively **Dual Marching Cubes**
  reproduces thin features at low triangle count on the structured octree.
  *No mature Rust crate for Manifold DC was found — this is an in-house port from the TVCG paper.*
*Sources:* Schaefer et al. 2007 (Manifold DC); Schaefer & Warren 2004 (Dual MC); Ju et al. 2002.

**A1.6 [HIGH] Make failure loud (the review's top cross-cutting theme). — ✅ LARGELY DONE (2026-07-18).**
Landed and verified: non-finite `.obj` rejected at ingress; kernel panic hook + `kernel_log` sink +
`fit_is_finite` warning; ABI header-count bounds validation; the named octree/litho unwraps and the
`uni_degree` assert made safe; Manifold-dropped units surfaced; `worker.onerror` handled (see the
status table above). **The residuals are now also closed** in a follow-up: the `sym3_eig` NaN-`unwrap`
+ ABI-edge non-finite-coordinate rejection, the `gf_alloc/gf_free` capacity UB (§4.4), and the
uncapped `.obj` parse (§4.5) — all with tests, all building for `wasm32`. Note the "fit-failure
channel" premise was corrected — the one genuinely-open failure here is detecting the
*finite-but-wrong* under-constrained fit, which routes to **A2.3** (anisotropy/trend better-conditions
the fit) and a geometric-fold detector, not an ABI sentinel. Still pairs with UI item **B3**
(validation/QC).

### A2. Geological-method completeness

**A2.1 [HIGH] Fault networks via structural frames + slip functions + an ordered truncation graph.**
This is the single biggest *capability* gap (the engine has a single-block fault stub). The state of
the art represents each fault with its own **fault frame** — a local curvilinear coordinate system
(fault surface / slip-direction / extent axes) from three coupled scalar fields — and applies
**displacement as a slip function that tapers to zero toward the fault tips**, giving finite/listric
faults rather than infinite planes. Faults are time-ordered and can offset each other, so a
**truncation graph** lets faults cut series *and* one another — faults as first-class CSG
discontinuities. Even a first cut (finite planar faults + linear-decay slip + an ordered cut list)
unlocks horsts/grabens and stepped systems a single discontinuity cannot express. GemPy's pattern —
each fault a scalar field fed as an additional drift/offset into younger series — is a concept you
can copy directly. The LoopStructural faults paper is a near-complete, open-source blueprint.
*Sources:* Grose et al. 2021 (GMD 14:6197 — *not read firsthand, 403*); Grose et al. 2021 (GMD
14:3915, time-aware); Godefroy et al. 2018/2019; de la Varga et al. 2019 (GemPy). Corroborated by
Leapfrog Fault System (per-block chronology) and GeoModeller finite faults.

**A2.2 [HIGH, near-free] Expose the co-kriging variance as an interface-position uncertainty band.**
Modern geomodeling treats uncertainty as intrinsic, not an add-on (Wellmann & Caumon's 120-page
review). The cheapest entry point is already in hand: the **kriging variance** of the potential field
maps directly to positional uncertainty of interfaces (Chilès et al. 2004) — you already compute the
kriging system, so this is largely a read-out + visualization task. It moves the tool from "one
deterministic model" to "model + confidence," which reviewers and users now expect.
*Sources:* Chilès et al. 2004; Wellmann & Caumon 2018.

**A2.3 [MED–HIGH] Global anisotropy tensor + drift/trend degree in the kernel — *also* a
reliability fix.**
Basic co-kriging assumes near-isotropic stationary covariance; real strata have a dominant planar
anisotropy (bedding) and large-scale trend. Adding an **anisotropic covariance + drift polynomial**
(already latent in Lajaunie/Hillier) is a small parameterization change with broad payoff — and it
**directly attacks `code_review.md §1.3`** (reliability gated by fit conditioning): a better-
conditioned, structurally-informed field folds less, so the corefiner declines less and thin
features survive. It is also the prerequisite for locally-varying anisotropy and fold frames.
*Sources:* Lajaunie et al. 1997; Hillier et al. 2014; Martin & Boisvert 2017 (LVA). Corroborated by
Leapfrog Global/Structural Trend.

**A2.4 [MED] Monte-Carlo input-perturbation ensembles (probability-of-lithology volumes).**
The next uncertainty step after A2.2: sample interface points/orientations from error distributions,
re-run the field, output probability-of-lithology volumes. Moderate effort, no new math. Full
Bayesian/MCMC inversion (GemPy's headline) is high-effort and needs vectorized/differentiable
evaluation — scope it later.
*Sources:* Wellmann & Caumon 2018; de la Varga & Wellmann 2016; de la Varga et al. 2019.

**A2.5 [MED] Fold frames (structural-trend folding of the interpolant).**
Reuses the frame infrastructure from A2.1: build a fold frame (axial-surface foliation, fold-axis
lineation, vergence) and fold the interpolation along it so a single scalar field reproduces
tight/overturned/poly-phase folds an unbiased interpolator smooths away. Higher-effort and domain-
specific — do it only if fold terrains are in scope.
*Sources:* Laurent et al. 2016 (EPSL); Grose et al. 2017 (JSG).

**A2.6 [LOW–MED] Tangent / on-surface constraints and model-topology output.**
Two cheap parity items: **tangent constraints** (Hillier) let surfaces be constrained from fold
hinges/map traces where dip is unknown; **model topology** (unit-adjacency graph, GemPy) is a cheap,
high-signal output that also enables uncertainty conditioning and QC.
*Sources:* Hillier et al. 2014; de la Varga et al. 2019.

---

## Part B — UI / workflow features

Drawn from the reference tools (Leapfrog Geo, GeoModeller, LoopStructural/map2loop, SKUA-GOCAD,
Micromine, RockWorks, Move). Ordered by reliability-and-expressiveness payoff for a browser
prototype that already has the ordered stack, contact points, dip table, bounding box, implicit
fit, and Surfaces/Solids views.

**B1 [HIGH] Per-surface *younging-direction* toggle (younger/older side, flippable).** Removes the
#1 silent error in stacks — which side a contact faces — and handles overturned geology. Trivial UI,
large correctness gain. *(Leapfrog Deposits/Erosions.)* The engine already has a flip toggle
(`US-DATA-2`); surface it as an explicit younging direction with a visual indicator.

**B2 [HIGH] A legible whole-stack cut-rule table/matrix.** Show how every element cuts every other
(erode/onlap/cross-cut) in one inspectable, editable place, not just as per-element types — catches
ordering mistakes and makes composed geology legible. This is also where the review's topography-
ordering landmine (`code_review.md §3.1`) becomes visible to the user. *(GeoModeller stratigraphic
pile Erode/Onlap; LoopStructural time-aware order.)*

**B3 [HIGH] Data-validation / QC pass with inline error badges.** The prototype has *no* QC today
(`code_review.md §4` shows failures are silent). Add pure-client-side validators — duplicate/near-
duplicate points, orientation-vs-contact mismatch, points outside the box, empty/degenerate
elements, coincident vein walls — flagged with a red badge on the offending element. This is the UI
half of A1.6 "make failure loud." *(Leapfrog red-exclamation validation; map2loop positional/
gradient/topological data typing.)*

**B4 [HIGH] Named unconformity/contact-geometry choice.** Make "which side defines the surface" and
disconformity-vs-angular-unconformity an explicit dropdown, disambiguating the erosional/onlap
contacts the engine already supports. One control, real expressiveness. *(LoopStructural's three
unconformity types; GeoModeller.)*

**B5 [HIGH] Vein pinch-out + vein-to-vein termination link.** The engine already has two-wall veins;
add an explicit pinch-out (walls deliberately cross so the vein dies out) and a "terminates against
vein X" relation for realistic vein systems. *(Leapfrog Veins / Vein Systems.)* Note this also gives
the inert imported `VeinWall` role (`code_review.md §3.3`) a real semantic to attach to.

**B6 [MED] Surface-driven structural trend / anisotropy control.** The UI front-end for A2.3: pick a
global ellipsoid or an existing mesh as a trend, with Strength + Range. The biggest single
expressiveness lever, and the prototype already meshes surfaces so a trend mesh is in hand.
*(Leapfrog Structural/Global Trend.)*

**B7 [MED] On-demand cross-section / slice view** — ✅ **SHIPPED (feature 005, 2026-07-18).** Vertical
sections from a two-click plan baseline + serial parallel sets, opened face-on with the current
model's surface traces as a backdrop, are view-only (never dirty the project). Verified against
`SPEC.md §10`. Follow-ons if wanted: arbitrary tilted/fence sections (currently vertical-only), and a
printable section layout. *(Leapfrog sections; RockWorks cross-sections/fence.)*

**B8 [MED] Polyline-on-section surface editing.** — ✅ **SHIPPED (feature 005, 2026-07-18).** Ordered
polylines digitized across parallel sections loft into one fitted surface; the **draw direction sets
the younging/normal** (verified: shipped code uses the corrected `m = normalize(ŝ × t̂)`, the affordance
reads the same sign the fit does, and it feeds the existing points+orientations path with no kernel
change). Verified against `SPEC.md §11` (acceptance §11.6 satisfied). Gap to close: no unit tests for
the `frame`/`slice`/`digitizeDerive` geometry helpers — coverage is Playwright-e2e only. *(Leapfrog
edit-with-polyline.)*

**B9 [MED] Sealed-contact / non-overlap report.** Turn the engine's seam/stitch work into a user-
visible guarantee: flag where surfaces fail to seal or bodies overlap (ties to A1.5 manifold output
and `code_review.md §8` overlap partition tests). *(SKUA sealed contacts; Micromine no-overlap
domains.)*

**B10 [MED] Form-interpolant / trend-field preview.** Visualize the orientation field implied by the
dip table *before* fitting, so users sanity-check structural data cheaply and avoid bad fits.
*(Leapfrog Form Interpolants.)*

**B11 [LOW / heavier] Fault authoring UI, uncertainty band display, and a drillhole data model.** The
UI counterparts to A2.1/A2.2 and the borehole→section→solid loop (collar/survey/interval + desurvey
with a dip-sign toggle). Large new subsystems; sequence after the foundation. *(Leapfrog Fault
System & drillhole import; RockWorks/Micromine.)*

---

## Part C — Suggested phasing

> **Phase 1 status (2026-07-18): the *surfacing* half is done; the *foundation* half is the live
> queue.** A1.6 make-failure-loud (✅, minus the residuals noted above), off-thread compose + run-token
> guard (§2.1/§2.2 ✅), and — jumping ahead — the cross-section authoring feature (B7/B8 ✅) all landed.
> **Not yet started:** A1.1 exact-predicate booleans, A1.2 fast winding number, A1.3 determinism test,
> A1.4/A1.5 meshing, and B3 validation/QC. Those are the front of the queue now.

**Phase 1 — Trustworthy foundation (do first; unblocks everything).**
~~A1.6 make-failure-loud~~ ✅ (incl. the §4.4/§4.5/§4.6-residual/§7 cleanup); **remaining:** B3
validation/QC; A1.1 consolidate on exact-predicate N-body booleans; A1.2 fast winding number; A1.3
determinism + native/wasm golden test; A1.4 QEF spike clamp.
*Outcome:* the "reliably compose arbitrary CSG" goal is actually met for the geology the engine
already models, and failures are visible instead of silent.

**Phase 2 — Cheap expressiveness & correctness wins.**
B1 younging toggle *(note: feature 005 already derives per-line younging from draw direction — B1
generalizes that to a per-surface toggle)*; B2 stack cut-rule table (exposes the §3.1 topography-order
landmine); B4 unconformity types; B5 vein pinch-out (pairs with the still-open §3.3 imported VeinWall);
A2.2 kriging-variance uncertainty band (near-free); A2.3 global anisotropy (also better-conditions the
fit — the real fix for the §1.3/§4.2 finite-but-wrong-fit failure); A1.5 sub-cell feature detection +
Manifold DC.

**Phase 3 — Structural completeness.**
A2.1 fault networks via structural frames (the biggest capability gap) + B11 fault UI; B6 structural-
trend control; ~~B7 cross-sections~~ ✅ (shipped — extend to tilted/fence sections); A2.6 tangent
constraints + topology output.

**Phase 4 — Frontier.**
A2.4 Monte-Carlo ensembles; A2.5 fold frames; ~~B8 polyline-on-section editing~~ ✅ (shipped — add
unit tests for the geometry helpers); B11 drillhole model; full Bayesian inversion / LVA / neural
meshing as research bets.

---

## Part D — References

*Method / geology*
- Lajaunie, Courrioux, Manuel (1997). *Foliation fields and 3D cartography in geology.* Mathematical
  Geology 29(4), 571–584. https://link.springer.com/article/10.1007/BF02775087
- Calcagno, Chilès, Courrioux, Guillen (2008). *Geological modelling … 3D potential-field
  interpolation and geological rules.* PEPI 171, 147–157. https://hal.science/hal-00532155/document
- Chilès, Aug, Guillen, Lees (2004). *Modelling the geometry of geological units and its uncertainty
  in 3D … the potential-field method.* AusIMM Orebody Modelling.
  https://www.researchgate.net/publication/237449268
- Hillier, Schetselaar, de Kemp, Perron (2014). *3D modelling of geological surfaces using generalized
  interpolation with radial basis functions.* Mathematical Geosciences 46(8), 931–953.
  https://link.springer.com/article/10.1007/s11004-014-9540-3 · SURFE: https://github.com/MichaelHillier/surfe
- Grose, Ailleres, Laurent, Jessell, Caumon, Armit (2021). *Modelling of faults in LoopStructural
  1.0.* GMD 14, 6197–6213. https://gmd.copernicus.org/articles/14/6197/2021/ *(not read firsthand — 403)*
- Grose, Ailleres, Laurent, Jessell (2021). *LoopStructural 1.0: time-aware geological modelling.*
  GMD 14, 3915–3937. https://gmd.copernicus.org/articles/14/3915/2021/
- Godefroy, Caumon, Ford, Laurent, Jackson (2018). *A parametric fault displacement model …*
  Interpretation 6(2), B1–B13. https://ui.adsabs.harvard.edu/abs/2018Int.....6B...1G/abstract
- Godefroy, Caumon, Laurent, Bonneau (2019). *Structural interpretation of sparse fault data using
  graph theory and geological rules.* Mathematical Geosciences 51, 1091–1107.
  https://link.springer.com/article/10.1007/s11004-019-09800-0
- Laurent, Ailleres, Grose, Caumon, Jessell, Armit (2016). *Implicit modeling of folds and
  overprinting deformation.* EPSL 456, 26–38. https://hal.univ-lorraine.fr/hal-01390858/file/implicit_folding_author.pdf
- Martin, Boisvert (2017). *Iterative refinement of implicit boundary models … locally varying
  anisotropy.* Computers & Geosciences. https://www.sciencedirect.com/science/article/abs/pii/S0098300416307555
- Wellmann, Caumon (2018). *3-D structural geological models: concepts, methods, and uncertainties.*
  Advances in Geophysics 59, 1–121. https://hal.science/hal-01921494
- de la Varga, Wellmann (2016). *Structural geologic modeling as an inference problem: A Bayesian
  perspective.* Interpretation 4(3), SM1–SM16.
- de la Varga, Schaaf, Wellmann (2019). *GemPy 1.0: open-source stochastic geological modeling and
  inversion.* GMD 12, 1–32. https://gmd.copernicus.org/articles/12/1/2019/
- Pakyuz-Charrier et al. / Schweizer, Wellmann et al. (2021). *Constraining stochastic 3-D models with
  topology … ABC in GemPy 2.1.* GMD 14, 3899–3913. https://gmd.copernicus.org/articles/14/3899/2021/
- GemPy v3 (2024). https://www.gempy.org/ · https://github.com/gempy-project/gempy/blob/main/WhatsNewGemPy3.md
- Thiele/Jessell — map2loop (2021). GMD 14, 5063. https://gmd.copernicus.org/articles/14/5063/2021/

*Geometry / meshing / robustness*
- Zhou, Grinspun, Zorin, Jacobson (2016). *Mesh Arrangements for Solid Geometry.* ACM TOG 35(4).
  http://www.cs.columbia.edu/cg/mesh-arrangements/
- Cherchi, Livesu, Scateni, Attene (2020). *Fast and Robust Mesh Arrangements using Floating-point
  Arithmetic.* ACM TOG 39(6). https://github.com/gcherchi/FastAndRobustMeshArrangements
- Cherchi, Pellacini, Attene, Livesu (2022). *Interactive and Robust Mesh Booleans.* ACM TOG 41(6).
  https://github.com/gcherchi/InteractiveAndRobustMeshBooleans
- Attene (2020). *Indirect Predicates for Geometric Constructions.* CAD 126.
  https://github.com/MarcoAttene/Indirect_Predicates
- Shewchuk (1997). *Adaptive Precision Floating-Point Arithmetic and Fast Robust Geometric
  Predicates.* Discrete & Comput. Geometry 18(3). https://www.cs.cmu.edu/~quake/robust.html
- Ju, Losasso, Schaefer, Warren (2002). *Dual Contouring of Hermite Data.* ACM TOG 21(3).
  https://www.cse.wustl.edu/~taoju/research/dualContour.pdf
- Schaefer, Ju, Warren (2007). *Manifold Dual Contouring.* IEEE TVCG 13(3).
  https://people.engr.tamu.edu/schaefer/research/dualsimp_tvcg.pdf
- Schaefer, Warren (2004). *Dual Marching Cubes: Primal Contouring of Dual Grids.* PG 2004.
  https://www.cs.rice.edu/~jwarren/papers/dmc.pdf
- Kobbelt, Botsch, Schwanecke, Seidel (2001). *Feature Sensitive Surface Extraction from Volume Data*
  (Extended MC). SIGGRAPH 2001. https://www.graphics.rwth-aachen.de/media/papers/feature1.pdf
- Barill, Dickson, Schmidt, Levin, Jacobson (2018). *Fast Winding Numbers for Soups and Clouds.* ACM
  TOG 37(4). https://www.dgp.toronto.edu/projects/fast-winding-numbers/ · ref:
  https://github.com/sideeffects/WindingNumber
- Jacobson, Kavan, Sorkine-Hornung (2013). *Robust Inside-Outside Segmentation using Generalized
  Winding Numbers.* SIGGRAPH 2013. https://igl.ethz.ch/projects/winding-number/
- `georust/robust` (Rust port of Shewchuk predicates): https://github.com/georust/robust
- Rust Manifold bindings *(maturity unverified)*: https://crates.io/crates/manifold3d ·
  https://github.com/zmerlynn/manifold-csg

*Software documentation (official pages; bodies 403-blocked to fetch — search-index-level confidence)*
- Leapfrog Geo (Seequent): Deposits/Erosions, Surface Chronology, Veins & Vein Systems, Trends &
  Anisotropy / Structural & Global Trends, Form Interpolants, Faulted Models, Drillhole import,
  Serial/Fence Sections, Polyline editing — under https://help.seequent.com/Geo/
- GeoModeller (Intrepid): pile Erode/Onlap, faults, stochastic inversion —
  https://www.intrepid-geophysics.com/products/geomodeller/ · https://docs.intrepid-geophysics.com/geomodeller/
- LoopStructural / map2loop: https://loop3d.org/LoopStructural/
- SKUA-GOCAD sealed implicit modelling: https://www.mirageoscience.com/ (GOCAD Mining Suite / SKUA)
- Micromine Implicit / Stratigraphic Hierarchy: https://webhelp.micromine.com/ ·
  https://global.micromine.com/the-stratigraphic-modelling-process/
- RockWorks (cross-sections, solid models, lithology-vs-stratigraphy): https://www.rockware.com/ ·
  https://help.rockware.com/rockworks17/
- Move (Petex) restoration/balancing: https://petex.com/pe-geology/move-suite/move/
