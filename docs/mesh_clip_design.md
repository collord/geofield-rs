# `mesh_clip` — mesh–mesh seam corefinement (design note)

**Status:** design, pre-implementation. Reviewed against `weld.rs`, `octree.rs`, and the
committed snap at `35e5005`. Awaiting sign-off before any clip code is written.

**Goal.** A standalone, field-agnostic module `crates/geofield/src/mesh_clip.rs` that takes
two already-contoured triangle meshes — a clipped surface **C** (older, low-angle contact)
and a clipping surface **W** (younger, high-angle: fault hanging wall or erosion surface) —
and produces a watertight seam where C terminates against W, with **seam vertices shared by
index** and **no interior vertex of either surface moved off its own plane**. That last
property is the whole point: it is what kills the scalloping that `snap_free_boundary_to_mesh`
produces and the staircase that the CSG-field-union produces.

This is the *keep-one-side* half of a mesh boolean (corefine + discard one side of C), not a
full CSG. It is also the path that *imported, field-less* surfaces must use.

---

## 0. Where this sits relative to what already exists

This is not greenfield. Three seam strategies already coexist; `mesh_clip` is the principled
replacement for all of them on the C-against-W case, but it lands *alongside* them and only
replaces them once its tests pass (the snap at `35e5005` is the watertight floor we must not
regress).

| Existing | What it does | Why it's not enough |
|---|---|---|
| **CSG-field-union** (`Scene` `g = max(field−iso, b)`, `mesh_cross_cut_shared` in `stack.rs`) | Co-contours W and the cut contacts on one octree; the crease is a watertight DC crease. | Staircased — the seam is sampled on the octree grid, not on the true C∩W line. Field-only (can't do imported meshes). The reverted `clip_mesh_to_field` tried to fix this in the field and regressed to teeth/gaps. **Do not reopen.** |
| **point-weld** (`weld::weld_points_into_mesh`) | Inserts C's *existing* free-boundary points into W by splitting the W triangle/edge each lands on → shared on W's side. | The inserted points are C's **rim** points *projected onto the nearest W triangle*, not the true C∩W intersection. So C's rim is wherever the contour happened to cut it, then snapped to W — approximate, and C's own rim verts still sit short of / past the true line. |
| **snap** (`weld::snap_free_boundary_to_mesh`) | Moves C's free-boundary verts onto the closest point of W. | **Moves C's vertices** — exactly the scalloping we are trying to remove. |

`mesh_clip` differs from `weld` in the one respect that matters: it computes the **true
triangle–triangle intersection polyline** between C and W, inserts *those* points (which lie
on both surfaces by construction) as shared vertices, and re-triangulates C's cut row to
terminate *on* them. `weld`'s W-side triangle-splitting primitives
(`split_interior` / `split_on_edge` / `neighbour_across` / the `TriGrid` spatial hash) are
directly reusable for step 3 below — **we reuse those, we do not re-implement them.**

Reuse decision: `mesh_clip` will `use crate::weld::{TriGrid, ...}` for the W-side insertion
machinery, promoting the needed `weld` internals to `pub(crate)`. The *novel* code is the
T–T intersection-segment computation and the per-C-triangle constrained re-triangulation.

---

## 1. Public surface

```rust
/// A mesh as the module sees it: world-space verts + CCW triangles. Both inputs and the
/// output use this; no ScalarField, Model, or field method is ever called here.
pub struct ClipMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
}

/// Corefine `c` against `w` and keep the side of `w` selected by `keep`. Returns the
/// re-triangulated C and W, sharing seam vertices by index in a single combined buffer
/// (or two buffers with an explicit shared-index map — see §4).
///
/// `refine`: optional field hook. When `Some(f)`, a newly inserted seam vertex MAY be
/// nudged along the seam onto `f`'s zero set for sub-triangle accuracy. When `None`
/// (the default, and the only mode imported meshes ever use) NOTHING structural changes —
/// the seam is purely the C∩W polyline. Off by default; changes no topology either way.
pub fn clip(
    c: &ClipMesh,
    w: &ClipMesh,
    keep: KeepSide,
    refine: Option<&dyn ScalarField>,
) -> ClipResult;
```

`KeepSide` selects which side of W to retain for C (signed by W's outward normal:
`Above`/`Below`, or `Inside`/`Outside` if W is closed). The `refine` hook is the *only*
place a field can enter, and only to move a seam vertex *along the already-fixed seam line*
— never to move an interior C or W vertex, and never to change which triangles exist. Its
default is `None`; it exists so generated meshes can optionally get sub-triangle seam
accuracy without making the module field-dependent.

**Empty-seam behaviour is NOT a blanket no-op — it branches on `keep`.** When step A finds
zero C∩W segments, *all* of C is on one side of W, so one representative-C-vertex side test
against W decides the whole mesh:

| Segments | C vs `keep` side | Result for C |
|---|---|---|
| ≥1 | straddles W | cut + keep the `keep` side (the corefine path) |
| 0 | entirely on **keep** side | C returned unchanged — the true no-op |
| 0 | entirely on **discard** side | C returned **empty** (fully removed) |

The discard-side case is real geology: an erosion surface W that has already removed everything
above it leaves a contact C sitting wholly above W with zero crossings — C must *vanish*, not
pass through. W is returned unchanged in every branch (we never discard W); `ClipResult.map` is
empty whenever no seam vertices were created. **Decided (return shape):** `ClipResult` is the
**two-buffer form** — `{ c: ClipMesh, w: ClipMesh, shared: Vec<(usize /*c idx*/, usize /*w
idx*/)> }` — matching how `stack.rs` emits C and W as separate meshes; `shared` pairs the seam
vertices that are the same point in each buffer.

---

## 2. Algorithm (corefine + keep-one-side), mirroring established methods

The robust, non-improvised pattern is mesh boolean **co-refinement**: exact-or-carefully-
epsilon'd triangle–triangle intersection segments, then **constrained Delaunay
re-triangulation** of each cut face with those segments as constraints. References to mirror
(not improvise around): CGAL PMP `corefine()`, Cherchi et al. 2020 (*Fast and Robust Mesh
Arrangements using Floating-point Arithmetic*), and libigl `remesh_self_intersections` /
`mesh_boolean` as the readable implementation. We need only the corefine + keep-one-side
half.

### Step A — intersection polyline (Möller segment form)

For every candidate pair (C-triangle `tc`, W-triangle `tw`) — narrowed by an AABB grid so we
don't do O(|C|·|W|) — compute the **intersection segment** (Möller, *A Fast Triangle–Triangle
Intersection Test*, segment/interval form, not the boolean form):

1. Signed distances of `tw`'s three verts to `tc`'s plane (`du0,du1,du2 = n_c·U + d_c`) and
   vice-versa (`dv*`). Snap `|d| < EPS` to 0. **Early reject:** if `du0·du1 > 0 && du0·du2 >
   0` all of `tw` is on one side of `tc`'s plane → no intersection (same for `dv`).
2. Line **L** direction `d = n_c × n_w`. *The projection trick (Möller
   `compute_intervals_isectline`):* don't build L explicitly — take `axis = argmax(|d.x|,
   |d.y|, |d.z|)` and project any point P onto L by just taking `P[axis]`. For each triangle,
   the lone vertex on one side of the other plane plus its two straddling edges give a scalar
   interval `[isect0, isect1]` (via `t = D0/(D0−D1)` per straddling edge), and we also store
   the actual 3D `isectpoint = VTX0 + (VTX1−VTX0)·t` for each interval endpoint.
3. Sort each interval ascending. **Reject** if `isect1[1] < isect2[0] || isect2[1] <
   isect1[0]`. Otherwise the **segment** endpoints are the *inner* bounds —
   `start = max(isect1[0], isect2[0])`, `end = min(isect1[1], isect2[1])` — and we return the
   matching stored 3D `isectpoint`s (tracking which interval won each bound, like the
   reference's `smallest1/smallest2` swap flags), not a re-interpolation.

Each non-empty segment is a directed piece of the C∩W polyline. We tag it with `(tc, tw)`
so step C knows which C-triangle and which W-triangle to cut. We reuse `octree.rs`'s vector
helpers (`sub`/`dot`/`cross`/`norm`) and the Möller–Trumbore `ray_triangle` math as the
crib for the projections (it already lives in the crate).

**Coplanar C/W triangles** (`d ≈ 0`, `|n_c × n_w|` below an angle epsilon): handled
separately — a coplanar pair contributes a *polygon* overlap, not a segment. For our C-vs-W
case (low-angle contact vs high-angle wall) coplanar pairs are degenerate/rare; the first
milestone treats a coplanar pair as "no constraint" and logs it, and we decide on full
coplanar handling only if a real fixture needs it (see §6 honesty checkpoint).

### Step B — build the seam polyline with canonical shared vertices

Collect all segment endpoints. Two distinct `(tc, tw)` segments that share a geometric
endpoint (e.g. where the seam crosses a C edge into the next C triangle) must reference the
**same vertex index**. Keying strategy (see §4): segment endpoints are classified by *what
they lie on* — a W vertex, a W edge, a C edge, or a C-edge×W-edge crossing — and keyed by
that combinatorial identity, **not** by spatial hash, so two triangles computing "the point
where C-edge `(a,b)` crosses W" agree on one index exactly. A spatial hash is the fallback
only for the genuinely-coincident-but-not-combinatorially-equal case, with a tolerance tied
to the mesh's edge length.

### Step C — re-triangulate each cut face (2D CDT in the face plane)

For each C-triangle that owns one or more seam segments:

1. Project the triangle + its seam segment endpoints to 2D by **dropping the dominant axis**
   (`k = argmax(|n.x|,|n.y|,|n.z|)`, keep the other two) — the f64 equivalent of libigl's
   `Plane_3::to_2d`. Exact linear projection; it does not move points in 3D, only chooses a
   plane to triangulate in. **Track the winding:** dropping an axis can mirror orientation, so
   record the sign and flip output-triangle winding back to match the original face normal
   (libigl checks `orient2d` of the projected corners for exactly this).
2. `cdt::triangulate_with_edges(points, edges)` with the seam segments as fixed edges. Output
   is index triples into `points` (corners first, then seam endpoints), so welding is by index.
3. Lift each output vertex back to 3D. Corners and seam endpoints carry their original/computed
   3D coords; any *new* Steiner point the CDT introduces is lifted by solving its 2D barycentric
   coords against the projected corners and applying them to the 3D corners (the planar
   triangle's plane recovers the dropped coordinate exactly). Original C corners keep their
   **bit-exact input 3D coords** → **no interior C vertex moves off C's plane**.
4. Discard the sub-triangles on the *non-kept* side of W. Two consistent ways, pick per §7-Q?:
   (i) sign of W's signed distance at each sub-triangle centroid (works when W is a coherent
   oriented sheet), or (ii) **flood-fill** the sub-triangles using the seam edges as barriers
   and keep the component on the chosen side — more robust when the centroid test is ambiguous
   near a grazing seam (this is libigl's connected-component idea, minus the full
   winding-number machinery we don't need for keep-one-side). The kept sub-triangles replace
   the original C-triangle.

W is cut the symmetric way: each W-triangle that owns seam segments is re-triangulated with
those segments as constraints (so W carries the seam explicitly), but **all** of W's
sub-triangles are kept (we don't discard W). For the W side we can reuse `weld`'s existing
`split_interior` / `split_on_edge` machinery when the seam endpoint lands on a face/edge in
the simple single-point case, and fall back to the same CDT for the multi-segment case.

### Step D — weld to shared indices

C's kept sub-triangles and W's sub-triangles reference seam vertices through the canonical
index map from step B → the join is **watertight by construction**, not by post-hoc
epsilon-welding. A final `assert` (debug-only) verifies every seam edge has incidence 2
across the combined C∪W triangle set.

### CDT engine — FINAL: `spade` (reversed from `cdt` during implementation)

**Implementation outcome (Milestone 1):** the `cdt` crate decision below was REVERSED in favour of
`spade`, on hard evidence. `cdt` 0.1 silently returns an *incomplete* triangulation (drops whole
sub-regions, leaving holes) when a constraint endpoint lands exactly on a face boundary edge — and
in corefinement the seam enters/exits **every** cut face through an edge, so this case is the rule,
not the exception. Reproduced standalone on two real fixture faces (`cdt` → 0 or partial triangles;
`spade` → complete tiling). `spade`'s `add_constraint_and_split` honours a constraint endpoint on an
existing vertex/edge; its predicates come from the `robust` crate (adaptive-exact, pure-Rust), and
it builds clean to `wasm32-unknown-unknown` (verified), so the native/wasm sign-stability property
is preserved. `spade` returns triangles as vertex handles, but `handle.fix().index()` maps back to
our insertion-order input index — so the welding-by-index property still holds. The note below is
kept for the reasoning trail; `cdt` remains a fallback only if `spade` ever proves too heavy.

### CDT engine — superseded recommendation: the `cdt` crate (revised after research)

The crate is deliberately dependency-light and builds to `wasm32` (`cdylib`), and the memory
note `octree-qef-projection-spike-on-flipped-field` records that this exact code path has
already been bitten by **native-vs-wasm floating-point sign divergence**. My first instinct
was a hand-rolled dependency-free CDT to control epsilons identically on both targets.
**Research flipped this** — the right call is the `cdt` crate:

- **`cdt` v0.1.0** depends only on **`geometry-predicates`** (Shewchuk's robust adaptive
  predicates, *pure-Rust, `no_std`, no C bindings* → wasm-clean) and `thiserror`. Tiny
  surface, no threads/filesystem.
- **It dissolves the sign-divergence concern instead of managing it.** `geometry-predicates`
  computes `orient2d`/`incircle` **with the correct sign regardless of how near-singular the
  matrix is** — exact, not epsilon'd. That is *better* than hand-tuned matching epsilons: the
  classification is provably identical native vs wasm because it is exact, not approximately
  equal. Hand-rolling would mean re-deriving Shewchuk's predicates myself — the opposite of
  boring/obvious code, and the single highest-risk thing to get wrong (a bad `incircle` is
  exactly the "crack in one face out of thousands" failure that bit the last attempt).
- **Its output shape is the seam-welding API we want:** `cdt::triangulate_with_edges(points,
  edges)` returns **triangles as index triples into the point array we pass in**. So we keep
  our own vertex identities and weld by index directly — no translation from library vertex
  handles (the reason to prefer `cdt` over `spade` here; `spade`'s richer half-edge DS and
  auto-split-of-intersecting-constraints are overkill for one small face).
- Coordinate-range caveat of `geometry-predicates` (≈1e-142 .. 1e201) is a non-issue: our
  world coords are O(10³).

`spade` stays the fallback only if we need automatic resolution of *mutually crossing*
constraint segments inside one face (it inserts the crossing point for us); for the C-vs-W
seam the constraints are a near-collinear chain, so crossings inside one face are rare and we
pre-split them (§5). **Decision: add `cdt` as a dependency.** This is the one new dependency
the module needs and it earns its place — it removes a known bug class rather than adding one.

---

## 3. Data structures

```
SeamSegment { a: SeamVtx, b: SeamVtx, tc: usize, tw: usize }   // one directed C∩W piece
SeamVtx     { pos: [f64;3], key: SeamKey }                      // 3D point + combinatorial id
SeamKey:
    WVertex(usize)                 // coincides with an existing W vertex
    WEdge(usize, usize, OrderedF)  // on W edge (i<j) at parameter t  → quantized for keying
    CEdge(usize, usize, OrderedF)  // on C edge (i<j) at parameter t
    Cross(CEdge_id, WEdge_id)      // C-edge crosses W-edge (the generic interior case)
SeamPolyline = Vec<SeamSegment>, plus a dedup map SeamKey -> shared vertex index
```

The whole seam is the set of `SeamSegment`s; `SeamKey` is how two triangles independently
arrive at the *same* index for a shared point.

---

## 4. Shared-vertex keying & welding (the watertightness contract)

**Exact index sharing, keyed combinatorially — not a spatial hash by default.** A point on
the seam is born from a specific geometric incidence (a C-edge crossing a W-edge, a point on
a W edge, etc.). Two adjacent cut triangles that both touch that point reach it through the
*same incidence*, so they produce the same `SeamKey`, so they look up the same index. This is
how we get "C and W reference the same vertex objects along the seam" without snapping. This
mirrors libigl's welding precisely: corner verts reuse the original mesh index; an
**edge-lying point is cached in an `edge_vertices` map keyed by the normalized edge
`(min(a,b), max(a,b))`**, so the same point inserted from the two faces sharing that edge gets
the same index → the seam is shared by index across neighbours; a final coordinate dedup
(`unique_rows`) is only the backstop. Our `SeamKey` *is* that map's key, generalized to the
C-edge / W-edge / crossing cases.

The parameter `t` in `WEdge`/`CEdge` keys is **quantized** (snap to a grid of, say,
1e-9·edge_length) only for hashing equality — the stored 3D position is the unquantized exact
crossing. This avoids two computations of the same crossing differing in the 53rd bit and
hashing apart. The quantization grid is the *only* epsilon in the keying, and it is tied to
edge length so it scales with the mesh.

A spatial-hash weld (`TriGrid`-style, reusing `weld`'s) is the **fallback** for the residual
case where two genuinely-coincident points have *different* combinatorial keys (e.g. a C
vertex that happens to land exactly on a W edge). That is rare and gets a tight,
edge-length-scaled tolerance.

---

## 5. Degenerate cases & their guards

| Case | Guard |
|---|---|
| **Coplanar C/W triangle pair** (`|n_c×n_w|` < angle eps) | Milestone-1: treat as no constraint, `log` it. Full polygon-overlap handling deferred until a fixture needs it. |
| **C vertex lands exactly on a W edge** | Detected as `SeamKey::CEdge`/`WEdge` coincidence; welded via the spatial-hash fallback to one shared index. |
| **Seam crosses a C edge into the next C triangle** | Both C triangles produce the same `SeamKey::CEdge(a,b,t)` for the crossing → one shared vertex; the seam is continuous across the C-edge. |
| **Seam exits the mesh boundary (open rim)** | The seam polyline simply terminates; the terminal C triangle is cut only partially. The open rim there is *expected* and excluded from the "no open edges except true rim" assertion (mirrors the `on_box` filter in `octree.rs`'s `contact_*` tests). |
| **Near-zero / grazing intersection** | An interval overlap below `~1e-12·edge_length` is treated as a touch (point), not a segment — no degenerate zero-length constraint. (Same spirit as the `CONTACT_GRAZE_EPS` snap in `sample_corners`.) |
| **Two constraint segments crossing inside one C triangle** | Pre-split the segments at their mutual crossing into one shared seam vertex before the CDT (since `cdt` does not auto-resolve crossing constraints — `spade` would); then feed non-crossing constraints. |
| **Empty seam (no C∩W crossings)** | NOT a blanket no-op: one representative-C-vertex side test against W decides — keep side → C unchanged; discard side → C emptied (§1 table). |
| **Empty seam where C overhangs an OPEN W's rim** | "Which side" past a finite erosion *surface*'s open rim is genuinely undefined. For the geologic C-vs-W (wall spans the domain) it can't arise; if a fixture ever hits it, STOP and surface it rather than guessing a side. |
| **Native vs wasm sign divergence** | The CDT's orientation/in-circle predicates come from `geometry-predicates` (exact Shewchuk) → identical sign on both targets *by construction*, not by matched epsilons. Our own T–T sign tests still route through one shared epsilon helper (no bare `< 0.0` on a near-zero determinant). Hard requirement — see the QEF-spike memory. |

---

## 6. Process — milestones, tests, and honesty checkpoints

**Milestone 1 — DONE (5 tests, green).** Two synthetic planes built directly as
`(Vec<[f64;3]>, Vec<[usize;3]>)` (no `Model`). Asserts: (a) no torn seam off the rim; (b) C
interior vertices bit-identical to input (exact, not a tolerance); (c) seam vertices shared by
index between C and W. **Refinement to the criterion (learned during impl):** "incidence 2
everywhere" is WRONG for C-meets-W — the seam is a legitimate **3-way junction** (C's half-sheet
+ W's through-sheet = incidence 3). The real test (`assert_seam_watertight`): no incidence-1 off
the rim, no incidence>2 OFF the seam, and seam edges are incidence 2 or 3.

**Milestone 2 — DONE (2 tests, green).** Two flavours, both run through `mesh_clip` and share
`assert_seam_watertight` + an anti-scallop check:
- **field-agnostic** (`m2_..._field_agnostic`): C and W as INDEPENDENTLY tessellated planes —
  coprime grid resolutions (23×11 vs 13×17) + deterministic in-plane jitter, so neither seam nor
  vertex aligns (the staircase condition, no `Model`). Anti-scallop = every kept C vertex within
  1e-3 of C's own **least-squares plane** (field-free).
- **real contoured** (`m2_..._on_real_contoured_meshes`, native-only): `octree_dual_contour` two
  crossing `dipping_plane` Models → the actual staircase tessellations, clip, assert watertight +
  anti-scallop via `|field−iso|/|grad| < 1 cell` (the `contact_*` idiom).

**Output compaction (added in M2).** `clip` now drops vertices no triangle references (e.g. C
corners on the discarded side) and remaps indices + the `shared` pairs — surfaced by the real-
contour test, where orphaned discard-side verts otherwise read as off-contact. Consumers never
see dead vertices.

**Honesty checkpoints (no patching toward green):**
- If the T–T intersection or the CDT produces **non-manifold** output on a fixture, stop and
  show the failing case rather than epsilon-nudging it closed.
- The committed snap at `35e5005` is the **watertight floor**. `mesh_clip` lands beside it
  and only replaces the snap/weld call sites in `stack.rs` once Milestones 1–2 pass. Until
  then, nothing in `stack.rs`/`Scene` changes.
- If the hand-rolled CDT turns out to be the wrong call (a real fixture breaks it), that is a
  *reported* finding that justifies pulling `spade`, not a silent swap.

---

## 7. Decisions & remaining questions

**Decided (user sign-off 2026-06-22):**
- **CDT engine → `spade`** (reversed from `cdt` during Milestone 1 — `cdt` 0.1 drops sub-regions
  when a seam vertex lands on a face edge, which is the rule in corefinement; `spade` handles it,
  builds to wasm32, and `handle.fix().index()` preserves welding-by-index). See §2.
- **Return shape → two buffers + map.** `ClipResult { c: ClipMesh, w: ClipMesh, shared:
  Vec<(usize, usize)> }` — matches `stack.rs` emitting C and W separately (§1).
- **`KeepSide` is a required parameter**, and empty-seam branches on it (keep-side → C
  unchanged, discard-side → C emptied) rather than being a blanket no-op (§1, §5).

**Decided — deferred work, do NOT lose these:**

1. **`KeepSide` for an open W → normalize winding on import (NOT in `clip`).** "Above/below"
   by W's outward normal is well-defined per-triangle but globally consistent only if W is a
   coherently-oriented sheet. Our octree-contoured W already is (winds toward increasing
   field). **DEFERRED (import-side):** imported meshes must be winding-normalized *before*
   reaching `clip`. Known wild-mesh complication to handle there: a single imported file may
   contain **multiple parts whose normals do not share a semantic sense** (one part wound
   "up", another "down") — so normalization is *per-connected-component with a chosen
   reference sense*, not one global flip. That logic belongs in the import path, not here.
   `clip`'s contract: it **asserts** W is consistently wound (debug assert on a sampled set of
   adjacent-triangle normal agreements) and trusts it; it does not repair winding. *(Tracked
   in MEMORY + below.)*
2. **Coplanar C/W triangle-pair handling → DEFERRED past Milestone 1, but DO build it.** For
   Milestone 1 a coplanar pair (`|n_c×n_w|` < angle eps) is logged + skipped (no constraint).
   This is understood and not hard: the fix is the standard coplanar branch — project both
   triangles to 2D (drop W's dominant axis), compute the 2D polygon overlap (edge–edge
   crossings + point-in-triangle), and feed the overlap boundary as constraint edges to the
   same per-face CDT. Schedule it as the first follow-up after Milestones 1–2 pass. *(Tracked
   below — this is a "get to it", not a "maybe".)*

**Deferred-work checklist (so these don't evaporate):**
- [ ] Import-side winding normalization, per-connected-component with a reference sense
      (multi-part meshes with mixed normal senses). Precondition `clip` asserts.
- [ ] Coplanar C/W pair handling via 2D polygon-overlap → CDT constraints. First follow-up.

---

*This note is the design-review artifact requested before any clip code is written. The
algorithm references (Möller segment form, CGAL corefine, Cherchi 2020, libigl
remesh_self_intersections) are to be mirrored, not improvised; the concrete recipe for each
is folded in above. Sign-off requested on §2 (algorithm), §4 (keying), and the §7 open
questions before implementation begins.*

---

## References (mirror these, don't improvise)

- **Möller, *A Fast Triangle–Triangle Intersection Test*** — segment/interval form
  (`compute_intervals_isectline`):
  paper <https://fileadmin.cs.lth.se/cs/Personal/Tomas_Akenine-Moller/pubs/tritri.pdf>,
  reference C with isectline
  <https://github.com/erich666/jgt-code/blob/master/Volume_08/Number_1/Shen2003/tri_tri_test/include/Moller97.c>.
- **CGAL PMP `corefine()`** — seam = constrained edges via `edge_is_constrained_map`;
  *co-refined ≠ shared-by-index* (stitch is a separate step):
  <https://doc.cgal.org/latest/Polygon_mesh_processing/group__PMP__corefinement__grp.html>.
- **Cherchi et al. 2020, *Fast and Robust Mesh Arrangements using Floating-point Arithmetic***
  — per-triangle sub-triangulation (insert points, then segments); robustness from indirect
  predicates / deciding topology before trusting coordinates:
  <https://github.com/gcherchi/FastAndRobustMeshArrangements>,
  <https://www.gianmarcocherchi.com/pdf/mesh_arrangement.pdf>.
- **libigl `remesh_self_intersections` / `mesh_boolean`** — the readable implementation; the
  `edge_vertices` normalized-edge welding key and per-face projected CDT we mirror:
  <https://github.com/libigl/libigl/blob/main/include/igl/copyleft/cgal/remesh_intersections.cpp>,
  <https://github.com/libigl/libigl/blob/main/include/igl/copyleft/cgal/SelfIntersectMesh.h>.
- **CDT crate** <https://docs.rs/cdt/> (index-triple output, `triangulate_with_edges`) over
  **`geometry-predicates`** <https://docs.rs/geometry-predicates/> (pure-Rust `no_std` exact
  Shewchuk predicates — what makes the seam classification native/wasm-identical). `spade`
  <https://docs.rs/spade/> is the fallback if mutually-crossing constraints inside one face
  need auto-resolution.
