# Phase 0 Research: Imported Mesh Surfaces

All Technical-Context unknowns resolved against the current codebase. Citations are `file:line`.

---

## D1 — Smooth far-field fit: reuse the co-kriging kernel

**Decision**: Fit the smooth (beyond-bounds) field by reusing `geofield-kernel`'s `Model::fit`, feeding the imported mesh's **vertices as `SurfacePoint`s (single `interface_id`)** and its **normals as `Orientation` poles**, treating the whole mesh as one interface. This is invoked across the existing wasm-module boundary exactly as fitted surfaces are.

**Rationale**:
- The kernel already accepts arbitrary points + orientations for one interface (`Model::fit(&Series::new(sps, oris), opts)`), used everywhere (`geofield-kernel/src/lib.rs:96`, inputs `data.rs:14,27,41`). No new fitting code.
- It makes the imported surface's far-field behave **identically to a fitted surface** (same extrapolate-then-clip), which is the spec's whole point (full parity).
- **Constitution V clean**: the engine reaches the fit across the wasm import boundary (`add_series → fit_model → remote::fit → gf_fit`, `remote.rs:30-57`); the EUPL kernel is never linked into `geofield.wasm`. Reusing it introduces **no new license/wasm seam** (`geofield/Cargo.toml` cfg-gated native dep; `remote.rs:9`).

**Alternatives considered**: a separate RBF/implicit fit inside the BSD engine (rejected — duplicates the kernel's job, Principle VI, and would re-derive co-kriging; also risks a second drifting copy, Principle III). Normal-direction geometric extrapolation with no field (rejected by the user in the spec fork — it can't participate in units/lithology the way a field-backed surface can).

---

## D2 — Bounded solve for dense meshes: reuse the existing decimation coreset

**Decision**: Bound the co-kriging solve with the **existing** `KernelOptions.decimate: Option<DecimateConfig>` — set `point_budget` (a hard cap on kept points) for imported meshes. No new decimation code.

**Rationale**:
- The solve is dense **~N³** and ill-conditioned past a few hundred near-redundant points (`geofield-kernel/src/assemble.rs:22-32`) — a dense mesh must be thinned before fitting.
- A residual-guided coreset already exists: `DecimateConfig { seed_cell, threshold_frac, point_budget, max_iters }` (`assemble.rs:37-51`), `decimate_block` (`lib.rs:323`), wired through the wasm ABI via `decimate_frac` (`wasm.rs:78,96-113` → `abi.rs:134`). It grid-thins, fits, re-adds only mis-predicted points, iterates — exactly right for a mesh with dense flat regions and sparse detail.

**Open (for the spike, not a blocker)**: the specific `point_budget` for imports — pick empirically so the fitted field agrees with the mesh at the AABB seam within tolerance (D3). Logged as a tunable, per Principle IV.

**Alternatives considered**: fitting on all vertices (rejected — N³ blowup, the memory's kernel-conditioning note); a bespoke poisson-disk/cluster thinning (rejected — the residual-guided coreset already exists and is better-targeted, Principle VI).

---

## D3 — Exact-inside / smooth-outside: PATCH-STITCH, not field-blend (the key decision)

**Decision**: Produce the surface as **two contoured patches welded at the mesh AABB seam**, not as one blended composite field:
1. **Exact patch** — contour `MeshSdf` (the mesh's own signed-distance `ScalarField`, `octree.rs:861`) clipped to the mesh's axis-aligned bounds (`AxisBox`). Exact on the mesh by construction.
2. **Smooth patch** — contour the co-kriged field (D1) clipped to the region [outside the AABB → domain box].
3. **Weld** the two along the shared AABB seam with `mesh_clip::clip` (`mesh_clip.rs:172`), which shares the seam **by combinatorial index** → watertight, no scallop, and — crucially — **no blended field exists to grow a spurious zero-crossing**.

**Rationale**:
- The stated risk of a **field-blend** (a bump in the transition band re-crossing zero → a phantom sheet) is real and is the dominant failure mode: the octree contours whatever zero set exists and the `Scene` CSG is sign-based (`octree.rs:403-437`), so a transition-band sign flip becomes geometry. Patch-stitch **eliminates this structurally** — continuity is a geometric weld, not a field coincidence.
- `MeshSdf` (a distance field) and the co-kriged field (a recovered-isovalue potential) have **incommensurate scales**; blending them is precisely the "invent a field where the input said nothing" the codebase already rejected (`docs/07_imported_mesh_field_proxy.md:47-52,73-74`).
- The codebase already **decided** this direction: far-field CSG for imports is "left to the mesh-clip *volume* path … **not to a fabricated field**" (`docs/07:79-81`). Patch-stitch is that path. Everything it needs exists: `MeshSdf`, `mesh_clip::clip`, `ClipMesh::winding_normalized` (`mesh_clip.rs:107`) for the winding precondition, and `below_solid`/`octree_dual_contour_scene` (`stack.rs:2169`, `octree.rs:1085`) to contour each patch as a watertight solid.
- The `mesh_clip::clip` **`refine` hook is a no-op today** (`mesh_clip.rs:182`) and was built to nudge seam vertices onto a field's zero set (`docs/07:106-108`) — the sanctioned place to sharpen seam continuity **if** the index-shared weld alone is insufficient.

**Reconciliation with the spec's recorded decision**: the spec's Assumptions said "fit a field, exact-**blend** near the mesh." Patch-stitch honors that intent — it **does** fit a field for the smooth part and uses the exact mesh inside the bounds — but realizes the join as a geometric **stitch** rather than a scalar-field blend. Same user-facing outcome (exact inside, smooth kriged outside, continuous seam), strictly less risk. This refinement is flagged in `plan.md` (Summary + Constitution Check II) and obliges a deliberate amendment to `docs/07` (which had scoped imports to clip-only; this feature adopts a smooth far-field for full parity, confined to beyond the user's own geometry).

**SPIKE RESULT (T003, 2026-07-09) — GATE GREEN.** `crates/geofield/tests/imported_mesh_spike.rs` measured the decisive number on the under-covering fixture: `exact_err=0.000` (MeshSdf reproduces the mesh inside the bounds), **`seam_gap=0.000`** (the cokriged field's zero-height matches the mesh height at the AABB seam — the mesh boundary vertices are kriging constraints, so the patches abut with NO step), `far_z=500` (smooth field extends sanely to the domain edge). **Conclusion: patch-stitch is continuous by construction; the `mesh_clip` `refine` hook is NOT needed (T021 is a no-op-needed). Decimation `point_budget` untested (4-point fixture); pick it in US2 for dense imports.** The gate to proceed with the element plumbing is passed.

**Spike scope (validate before broad build — the one real geometry risk)**:
1. `mesh_clip::clip` cleanly stitches two *surface patches* at a **planar AABB seam** (today's callers stitch a contact against a wall; the AABB box face is a new W). Verify winding precondition holds and `ClipResult.shared` is full-length with no dropped seam faces (`coplanar_pairs` counter, `mesh_clip.rs:160`, flags an AABB face landing coplanar with mesh triangles).
2. `MeshSdf`/`TriMesh::edge_crossing_count` robustness at the seam for non-convex imports (`octree.rs:823`; `docs/06` Issue B) so double-crossed cells subdivide.
3. Whether the index-shared weld gives acceptable seam continuity (SC-005) on its own, or the `refine` hook must be implemented.

**Alternatives considered**: FIELD-BLEND (one composite `ScalarField` = weighted MeshSdf + kriged) — rejected (spurious-lobe risk + incommensurate scales + contradicts the existing `docs/07` decision). Contour-the-mesh-then-Manifold-union-with-a-smooth-solid — rejected (the corefiner shares by index and avoids the union scalloping the main-thread Manifold path would reintroduce).

**FOLLOW-UP FINDING (US1 build, 2026-07-09) — the exact patch cannot be `below(MeshSdf)`.** `MeshSdf`'s SIGN comes from the generalized winding number, which is only defined for a CLOSED mesh. An imported stratigraphic contact is an **OPEN sheet** (a bed with a boundary rim), so its winding number is ≈0 everywhere → `MeshSdf` has no usable inside/outside → `below_solid(&MeshSdf, 0)` does not produce a solid. Consequences for the exact-patch mesher (T011):
- The **side** ("which half is below the contact") must come from the kriged field's sign (or the mesh normals), NOT from `MeshSdf`. `MeshSdf` still gives exact *distance/geometry near* the sheet, just not the sign.
- Contouring the **kriged field** gives a watertight solid whose dividing surface equals the mesh **only where the kriged fit reproduces the mesh** — exact for a PLANAR imported bed (kriging is exact through coplanar constraints), but only *approximate between vertices* for a CURVED/dense mesh. So "kriged contour" satisfies INV-3 (exact-inside) for planar imports and within-tolerance for gently-curved ones, but NOT bit-exact for arbitrary meshes.
- **True** exact-inside for an arbitrary curved mesh needs the mesh triangles used *directly* as the contact face (not a contour), closed into a solid using the kriged sign — a mesh-direct "close an open sheet into a half-space solid" operation that does not exist yet. This is a bounded seam (Principle IV), NOT simple reuse of `below_solid`.

**RESOLVED (mesh-direct spike, 2026-07-09) — mesh-direct exactness is cheap; use it.** The user chose "spike the mesh-direct op first." The spike (`imported_mesh_spike.rs::mesh_direct_exact_patch_is_watertight_and_on_the_mesh`) proves a tiny `MeshSignedField` ScalarField wrapper does it:

```
scalar_field(p) = sign(kriged(p) - iso) * mesh.surface_distance(p)   // ±clamped distance to the triangles
```

Its zero-set is the mesh triangles **exactly** (distance 0), with a well-defined sign from the kriged field (which half is "below the contact") — solving the open-sheet no-sign problem. Contoured with the EXISTING `octree_dual_contour_scene` (cap:true) clipped to the AABB box, the result is: `contact_verts=315` on the mesh, **`max_dz=0.728`** (within one octree cell → exact to resolution, INV-3), **`open_edges=0 nonmanifold=0`** (watertight, INV-1/2). ~15 lines, no new corefiner, and it is **exact for ANY mesh** (curved included — the magnitude is the true triangle distance, so the contour follows the triangles at octree resolution), NOT just planar.

**Decision:** US1's exact-patch mesher (T011) uses `MeshSignedField` + `octree_dual_contour_scene`. The kriged fit supplies only the SIGN (and, in US2, the smooth far-field beyond the bounds). The "kriged-approximation-for-curved" fallback is unnecessary and dropped. This is the reuse-maximal AND fully-exact path.

---

## D4 — OBJ parser: hand-roll in the BSD engine

**Decision**: Add a ~50-line byte-oriented OBJ parser (`crates/geofield/src/obj.rs`) handling only `v` / `vn` / `f` lines, consuming `&[u8]`/`&str` and producing `(Vec<[f64;3]> vertices, Vec<[usize;3]> faces, Option<normals>)`. No new crate.

**Rationale**:
- No mesh parser exists in the dep tree today (greenfield ingress); the only OBJ code is the TS *writer* (`web3/src/exportObj.ts`).
- The feature needs only vertices + triangle faces (+ optional normals) — a trivial line parser.
- **Constitution V**: hand-rolling in the BSD `geofield` crate guarantees no native-only dep on the wasm path. It takes **bytes** by construction (the browser reads the file and passes the buffer across the ABI, matching how fit inputs are already marshalled, `wasm.rs:589`). `tobj` would require `default-features = false` + "only `load_obj_buf`" discipline to avoid pulling `std::fs` — more risk for no real saving.

**Alternatives considered**: `tobj` via `load_obj_buf` (acceptable license MIT/Apache and wasm-technically, but higher seam-hygiene risk than ~50 lines — rejected on Principle VI/V). Parsing in TypeScript (rejected — the engine needs the geometry anyway for `MeshSdf`/fit, so parse once in Rust and marshal raw bytes, not a pre-parsed structure).

---

## D5 — Element plumbing: a new `StackElement::ImportedSurface` along the existing chain

**Decision**: Add the element type along the mapped chain, mirroring how `vein`/`topography` thread through:

`project.ts ProjectElement { kind:"importedSurface" }` → `toSeriesJson` → `protocol.ts SeriesJson.importedMesh?` discriminator (sibling to `crossCut?`/`topography?`) → `worker.ts buildStack` branch → `wasm.rs WasmStack::add_imported_surface(bytes, role, relation, …)` → `stack.rs StackElement::ImportedSurface` (+ `schema.rs StackElementDto::ImportedSurface` for the DTO round-trip).

**Rationale**: this is the established, low-risk pattern for a new element (Principle VI). Every `match` over `StackElement` must be extended (`present_at`/`model` `stack.rs:360-380`; meshing dispatch `:517-597`; DTO `schema.rs:187,232`). The **novel** work is confined to the new element's mesher body (D3 patch-stitch) and the new ABI method's parse+fit (D4/D1) — both BSD engine code, with co-kriging still across the existing kernel boundary.

**Role parity**: the imported surface reuses the existing role machinery — the erode/onlap/conformable relation bit for stratigraphic use, the vein/intrusion band for walls, the topography cap. The imported surface supplies a **field** (co-kriged, D1) and an **exact-inside override** (D3); downstream unit derivation, lithology assignment, clipping (`clip_maskers`), and solid/OBJ export operate on it exactly as on a fitted field (Principle III — derived, not re-encoded).

---

## Cross-cutting notes

- **Native↔wasm parity**: the patch-stitch mesher and the OBJ parser are deterministic BSD engine code; parity is pinned natively and asserted as a design invariant (there is no wasm cargo-test harness — a known constraint), with the Playwright e2e as the cross-target behavior gate.
- **Governing docs**: `docs/06` (surface-distance/boundary trait — reused as-is) and `docs/07` (imported-mesh field proxy — **amended** per Constitution Check II to adopt a smooth far-field for full-parity imports).
