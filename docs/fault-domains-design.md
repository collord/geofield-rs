# Fault domains — design

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

1. `BlockIndex` trait + `OneFault` (planar) + single-block default.
2. `Model::fit_faulted` partition→per-block assemble/solve; `fit` delegates.
   Per-block `Field` dispatch in `scalar_field`/`gradient`. Per-block lithology.
3. Test: a planar fault offsetting a flat contact — assert the field is
   discontinuous across the plane and continuous within each block; assert each
   block reproduces its own contact.
4. Multi-fault `FaultStack` (sign-tuple → block) once two-block works.
5. (Later, optional) wire a fault mesh as an `octree::Boundary` to split the
   rendered mesh too — depends on step 4.
```
