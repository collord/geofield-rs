# Fault domains — design

> **Status (shipped, field level).** Steps 1–4 below are implemented in
> [`src/fault.rs`](../src/fault.rs) and [`src/lib.rs`](../src/lib.rs):
> `BlockIndex` + `SingleBlock` + `PlanarFault`/`OneFault`/`FaultStack`, and
> `Model::fit_faulted` (per-block assemble/solve, per-block eval dispatch, per-block
> lithology). `Model::fit` is unchanged (the `SingleBlock` case). Tests in
> `fault.rs` (membership) and `lib.rs::fault_tests` (a planar fault offsetting a flat
> contact: discontinuous across the plane, continuous within blocks, each block
> reproduces its own contact, lithology routes per block; plus a contrapositive that
> the same data fit *without* a fault is smooth). The mesh-seam split (step 5) and
> hand-authored on-fault block ids are NOT done — see notes below.
>
> **Reconciliation with the multi-series stack** (built after this doc): a fault is
> kept INTERNAL to one `Model` — any `StackedSeries` can be faulted independently.
> The stack reads a `Model` only through `scalar_field` / `lithology` (both per-block
> after `fit_faulted`), so faulting composes with erode/onlap masking for free; no
> change to `stack.rs`. A faulted contact's mesh already shows the lateral offset
> because `scalar_field` is discontinuous (the octree contours whatever the field
> gives). `Model::lithology()` returns block 0's interfaces as the representative set
> for stack masking; per-point sign tests route per-block via `scalar_field`. We did
> NOT add a stack-level `Relation::Fault` (a fault series offsetting every older
> series) — that couples faults into the masking engine and is deferred.

Goal: a fault surface partitions the model volume into **blocks**; the geological
contact field solves **independently** inside each block, so the scalar field is
*discontinuous across the fault* (gempy's "finite fault" behaviour — a fault
offsets and truncates contacts rather than smoothly bending them).

This is deliberately **separate from the `Boundary` work** in `octree.rs`. A
`Boundary` clips/closes the *mesh* of one already-solved field. Fault domains
change how the *field itself is assembled and evaluated*. They compose (a fault
mesh can also be a domain boundary) but they are different layers and should land
as different changes.

## Why per-block solve (not one solve with a discontinuity term)

The current solver builds one dense cokriging system over all surface points +
orientations and solves it once (`assemble` → `solve` → `Field`). A fault makes
the field discontinuous; you cannot represent a hard discontinuity inside a single
smooth kriging interpolant without special basis functions. gempy's approach, and
the minimal-surgery one here, is: **solve a separate field per block over only
that block's data, and at evaluation time pick the block the query point is in.**
Each block's field is smooth; the discontinuity is the block switch.

## The seam in the current code (from the assemble/solve map)

```
Series {pts, oris}  ──assemble──▶  System {K, b, geom}  ──solve──▶  Solution {w}
                                                                      │
                                          Field::new(geom, w) ────────┘  eval(p)
```

Key facts that make per-block clean (file:line from the code map):

* `SurfacePoint { xyz, interface_id }`, `Orientation { xyz, pole }` — **no**
  block/fault grouping today (`src/data.rs:14,33`).
* `Series::new(pts, oris)` **recomputes `reference_per_interface`** from whatever
  subset it's given (`src/data.rs:52-60`) — so a per-block `Series` built from a
  subset gets valid reference points automatically. No reference bookkeeping.
* `assemble(series, opts) -> System` and `solve(&System) -> Solution` are pure
  functions of their input (`src/assemble.rs:174`, `src/solve.rs:35`). Calling
  them N times on N subsets needs **no change to either**.
* `Field { geom, weights }` borrows one block's geometry + weights
  (`src/evaluate.rs:35`); `Model::scalar_field` already constructs a `Field` per
  call (`src/lib.rs:66`) — so routing to a per-block `Field` is a one-line change
  at the dispatch point.

So the whole feature inserts **between `Series` and `assemble`** (partition) and
**at `Model`'s eval dispatch** (route). Nothing in assemble/solve/evaluate/mesh
changes.

## Block membership

A fault divides space; "which block is `p` in?" is exactly a sign test — and we
already have the abstraction for it. **Reuse the `octree::Boundary` trait.** A
planar fault, or a fault delivered as a triangle mesh, is a `Boundary`:
`inside(p)` is the two-block sign test; `edge_crossing` we don't even need for
membership. For >2 blocks (multiple faults), block id is the tuple of per-fault
signs, mapped to a block index.

```rust
// Minimal: a fault is a Boundary; block id = which side.
trait BlockIndex {
    fn block_at(&self, p: [f64; 3]) -> usize;
    fn nblocks(&self) -> usize;
}

// One fault → 2 blocks.
struct OneFault<B: Boundary> { fault: B }
impl<B: Boundary> BlockIndex for OneFault<B> {
    fn block_at(&self, p: [f64;3]) -> usize { self.fault.inside(p) as usize }
    fn nblocks(&self) -> usize { 2 }
}

// k faults → up to 2^k blocks, indexed by the sign tuple (prune empties).
struct FaultStack { faults: Vec<Box<dyn Boundary>> }
```

Surface points/orientations are assigned to a block by the **same** `block_at`
test on their own position — no new field on `SurfacePoint`. (If hand-authored
block ids are ever needed, add `block_id: Option<u32>` later; not now.)

## Model changes (minimal)

```rust
struct Block { system: System, solution: Solution }

pub struct Model {
    blocks: Vec<Block>,            // 1 entry in the non-faulted case
    index: Box<dyn BlockIndex>,    // trivial single-block index when no fault
    lithology: Lithology,
    pub isovalue_spread: f64,
    pub residual: f64,
}

impl Model {
    pub fn fit(series: &Series, opts: KernelOptions) -> Self {
        Self::fit_faulted(series, single_block_index(), opts)   // 1 block
    }

    pub fn fit_faulted(series: &Series, index: Box<dyn BlockIndex>, opts: KernelOptions) -> Self {
        // 1. partition points/oris by index.block_at(pt.xyz)
        // 2. per block: Series::new(sub_pts, sub_oris) -> assemble -> solve
        // 3. recover lithology (see below)
    }

    pub fn scalar_field(&self, p: [f64;3]) -> f64 {
        let b = &self.blocks[self.index.block_at(p)];
        Field::new(&b.system.geom, &b.solution).scalar_field(p)
    }
    // scalar_field_gradient: same dispatch.
}
```

`fit` keeps its exact signature → all existing callers and tests unchanged
(single block is just the N=1 case).

## Lithology / isovalues across the fault

`Lithology::recover` averages the field at each interface's points to get its
isovalue (`src/litho.rs`). Across a fault, the *same* interface exists in multiple
blocks with **different** field values (each block's field is independent). So:

* **Per-block isovalues** (recommended): each `Block` recovers its own
  `Lithology` from its own points + field. `lithology_unit(p)` routes to the
  block at `p`, then classifies. This is correct even when a fault offsets a
  contact — the contact's isovalue is whatever that block's solve produced.
* A single shared isovalue stack would be wrong the moment a fault offsets a unit.

So `Block` carries `lithology: Lithology`; `Model.lithology` becomes either a
convenience for the single-block case or is dropped in favour of per-block.

## Open question to resolve before coding

**Fault data near the fault plane.** gempy assigns each contact point to a block;
points must not leak across the fault into the wrong block's solve. With membership
= `block_at(pt.xyz)`, a point sitting *on* the fault is ambiguous. Options: (a)
nudge by the fault normal sign of its orientation; (b) require authored block ids
for on-fault points. Resolve with a real faulted dataset, not synthetically.

**Relationship to the step-4 mesh-seam work.** If we want the *rendered mesh* to
also be split at the fault (not just the field), the fault mesh becomes a
`Boundary` in `octree.rs` and we hit the same interior-cap seam gap step 4 fixes.
The field-level fault (this doc) and the mesh-level fault closure are independent;
do the field level first (it's the gempy behaviour), and only wire the mesh seam
once step 4 lands.

## Suggested staging

1. ✅ `BlockIndex` trait + `OneFault` (planar) + single-block default.
2. ✅ `Model::fit_faulted` partition→per-block assemble/solve; `fit` delegates.
   Per-block `Field` dispatch in `scalar_field`/`gradient`. Per-block lithology.
3. ✅ Test: a planar fault offsetting a flat contact — discontinuous across the
   plane, continuous within each block, each block reproduces its own contact.
4. ✅ Multi-fault `FaultStack` (sign-tuple → block).
5. ⬜ (Later, optional) wire a fault mesh as an `octree::Boundary` to split the
   rendered mesh too. NOTE: for a faulted **field**, the contact mesh ALREADY shows
   the lateral offset (the octree contours the discontinuous `scalar_field`). Step 5
   is only needed if we want a fault *surface* meshed/closed as its own face.

## Deviations from this design, as built

* **Empty blocks.** A block with no surface points (a fault region with no data) is
  stored as `None`; `scalar_field` there returns `+∞` and `lithology_unit` returns 0
  (no contact / above everything). The doc's "open question" about on-fault points is
  thus side-stepped for now: membership is `block_at(pt.xyz)` with `>= 0` → positive
  side, and a data-less block simply contributes no field. Hand-authored block ids
  are still not added (deferred until a real faulted dataset needs them).
* **Orientation-starved blocks INFER an orientation from their points.** A block with
  surface points but NO orientation (cokriging is singular without a gradient anchor)
  does NOT vanish and does NOT blindly assume flat — `infer_orientation` derives the
  pole from the points' own geometry, using the best-constrained interface (most
  points):
    * ≥3 non-collinear points → the best-fit plane normal (covariance smallest-
      eigenvector via a local 3×3 symmetric Jacobi eigensolver, `sym3_eig`);
    * 2 points or ≥3 collinear points → the *flattest* plane through that line
      (`pole = P × H`, `flattest_plane_normal`) — the line fixes strike, flat is the
      dip prior;
    * 1 point (or fully degenerate) → flat `+Z`, the bare default.
  This is the common case for a fault plane that strands all orientations on one side
  (the harness's default fault through the domain center). It is invisible when real
  orientations exist and keeps both fault blocks meshable — and, unlike a flat
  assumption, a dipping contact stays dipping on the starved side. (Originally this
  block was dropped to `None`, which looked broken and — before the guard — panicked
  the solver; the first fix used a blind flat fallback.)
* **`Model` keeps a representative `lithology()`** (first non-empty block's
  interfaces) so the stack and existing callers are unchanged; `block_lithology(b)`
  exposes a specific block's isovalues.
