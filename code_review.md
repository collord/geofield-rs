# Code Review — geofield-rs engine & web3 harness

**Focus:** how efficiently and correctly the Rust crates (`geofield`, `geofield-kernel`) and
the `web3/` TypeScript app support the application described in `web3/SPEC.md`, with the
stated goal of **tooling that reliably composes arbitrary geologic constructive-solid-geometry
(CSG) operations.**

**Method.** The full pipeline was traced end-to-end (spec → TS driver → wasm boundary →
kernel fit → stack composition → octree meshing → mesh-boolean/clip → main-thread Manifold),
and each subsystem was audited in depth. Findings below cite `file:line`. Severity is judged
*against the goal* — i.e. whether a defect blocks or silently corrupts the reliable composition
of arbitrary stacks, not merely whether it is a bug in isolation.

---

## 1. Executive summary

The per-element geology is **correct**: the "who cuts whom" rules for Conformable / Erosional /
Onlap / Vein / Intrusion / Topography all match `SPEC.md §3.1.2`, and the youngest-present-wins
volume rule is encoded faithfully (`stack.rs:582`, `stack.rs:2219`). The design is thoughtful and
unusually well-documented.

The problem is that **"reliably compose *arbitrary*" is not yet delivered**, for one structural
reason and one cross-cutting reason:

1. **There are two composition engines with very different generality, and only one is general.**
   - The **Manifold recipe path** (`stack.rs:mesh_volume_recipe` → main-thread
     `manifoldVolumes.ts:executeRecipe`), which is the **default in Solids mode**, is the *only*
     path that composes an arbitrary number of cutting bodies against each other correctly. It
     ANDs every younger element's masker into every older unit (`emit_clip_defs`, `stack.rs:2135`).
   - **Everything else** — all of Surfaces mode, and the Clip/corefine seam-quality paths — is
     **structurally single-body**. The shared/corefined meshers take one body index and resolve
     only that body's seam (`stack.rs:1158`, `:1612`, `:1915`); a vein crossing an intrusion, or
     two veins crossing the same bed, is never jointly corefined. The mesh–mesh `clip` primitive
     is strictly pairwise (`mesh_clip.rs:172`).

   So the app *does* have a general composition path, but the higher-quality/watertight paths do
   not compose, and the general path carries the constraints in §2 below.

2. **Failure across the whole pipeline is silent.** A composed model that goes wrong tends to
   render as *missing geometry with a "done" status*, which is precisely the outcome `SPEC.md §2.2`
   and `US-STATUS-1` promise to avoid. Manifold **silently drops** any rock unit whose primitive
   fails to ingest (`manifoldVolumes.ts:87`); the corefiner **declines to a legacy staircase** on
   any watertightness failure (`stack.rs:2124`, `mesh_clip.rs:3462`); and a malformed imported
   `.obj` can **panic and poison the kernel wasm instance**, losing every element already fitted in
   that Apply (`obj.rs` → `litho.rs`).

   > **UPDATE (make-failure-loud pass — done):** the reachable silent/trapping paths are now closed:
   > non-finite `.obj` vertices/normals are rejected at ingress (`ObjError::NonFinite`); the two named
   > kernel `partial_cmp().unwrap()` panic sites + the `uni_degree` assert are made NaN/arg-safe; the
   > ABI validates buffer header counts before indexing (no OOB); the kernel gained a panic hook + a
   > generic `kernel_host.kernel_log` host diagnostic sink; Manifold-dropped units and compose
   > exceptions now surface a warning/error status instead of finishing empty-as-"done"; and
   > `worker.onerror` is handled. **Caveat:** the "no fit-failure channel" clause was based on a
   > mechanism (singular fit → NaN field) that **does not reproduce** — see 4.2. The genuinely silent
   > failure that remains is the *geologically-wrong-but-finite* under-constrained fit (§1.3), which
   > no fit-time scalar detects.

3. **Reliability is gated by fit conditioning, which the harness deliberately hides.** Because
   §8 forbids exposing kernel/fit parameters, an under-constrained surface (e.g. a 2-point vein)
   produces a field whose level set folds — and *more* octree depth makes it *worse*, not better
   (measured in `stack.rs:6400 wstar_conform_depth_sweep`: depth 6→7→8 grows the spurious fold).
   The user has no lever to fix it and no diagnostic telling them the fit, not the tool, is the
   problem.

The net effect: simple, well-conditioned stacks compose reliably; **arbitrary** stacks (many
bodies, mutually-crossing bodies, imported meshes, under-constrained fits, large extents) degrade
in ways that are correct-by-fallback at best and silently wrong at worst.

The remainder of this document details the findings, grouped so the highest-leverage fixes are
visible.

---

## 2. The general composition path (Manifold) — constraints

The default Solids path is the right architecture, but three properties limit "arbitrary":

### 2.1 [HIGH] The N-body boolean composition runs on the **main thread** and cannot be cancelled
`worker.ts:30-34`, `worker.ts:312-318`, `manifoldVolumes.ts:1-9`, `main.tsx:204-224`

`SPEC.md §2.2` states "Fitting happens off the UI thread." The *fit* does — but the CSG boolean
pass, whose cost scales with (#bodies × #triangles), runs on the **main thread** via
`executeRecipe`, because `manifold-3d` deadlocks when re-instantiated in the terminate-per-fit
worker (`manifoldVolumes.ts:4-6`). Worker cancellation (`worker.terminate()`, `main.tsx:193`) does
**not** reach this pass — it runs *after* the worker posts `done`. For arbitrary (i.e. large or
many-body) compositions this blocks and freezes the UI with no way to interrupt, and — see 2.2 —
its result can be applied to a superseded model. The comment "not a perceptible block"
(`manifoldVolumes.ts:9`) is unverified for large stacks.
*Direction:* run Manifold in its own dedicated, persistent worker (separate from the terminate-per-fit
solver worker) and guard its result with a run token.

### 2.2 [HIGH] Stale main-thread Manifold results are applied to a newer model
`main.tsx:204-224`

`runStack` cancels the worker but not a pending `initManifold().then(executeRecipe…)` from a
*superseded* run. Any rapid re-trigger while solids compose — render-mode toggle, seam/manifold
checkbox, a lithology assignment, a bounds confirm — lets an old run's `finish()` overwrite
`surfaces`/`status` with stale geometry. No generation/run-id guard exists in `runStack`.
*Direction:* capture a monotonic run token; `finish` and the `.then/.catch` no-op if stale.

### 2.3 [HIGH] Manifold silently drops whole rock units; a compose failure renders empty as "done"
`manifoldVolumes.ts:87` (`if (!m) { failed = true; break; }`), `:104` (`volume() > 0`),
`main.tsx:221` (`.catch(() => finish(msg.surfaces))`)

If any single-level-set primitive fails Manifold's `ofMesh` (a `NotManifold` rejection — common
when an under-constrained fit yields a self-intersecting band), the **entire unit is dropped with
no signal**. On the recipe path the worker ships `surfaces = []` (`worker.ts:319-336`), so the
`catch` fallback `finish(msg.surfaces)` yields an **empty scene with status "done"** — the exact
silent-blank outcome `§2.2`/`US-STATUS-1` warn against. A composed model can be missing volumes
and say nothing.
*Direction:* count dropped/failed units and surface a warning status; on compose failure set an
error status rather than finishing empty.

---

## 3. Correctness of composition

### 3.1 [HIGH] `global_unit` does not hoist Topography — "rock in the sky" if it is not at index 0
`stack.rs:582-604`

The lithology walk is plain list order, but the doc comment and `SPEC.md §3.1.2` both say
topography "applies to the whole model regardless of where it sits in the list." Any element
*younger* than a mis-placed topography claims points **above the ground** as rock. The invariant
(topography at index 0) is enforced only in one caller (`wasm.rs:692` `series.insert(0, …)`), never
in the engine, yet `Stack::new` is public and `clip_maskers` *does* honor topography from any
position (`stack.rs:645-649`) — so the two halves of the model disagree. A deserialized or
programmatic stack yields wrong volumes with no error.
*Direction:* hoist topography first in `global_unit`, or normalize/assert its position in `Stack::new`.

### 3.2 [HIGH] The watertight seam meshers compose only a single cross-cutting body
`stack.rs:1158` (`mesh_cross_cut_shared`), `:1612` (`…_solids_shared`), `:1915` (`…_corefined`);
mirror in `mesh_clip.rs:172` (pairwise `clip`); driver in `worker.ts:112-199`, `:226-275`

Each shared/corefined mesher co-contours only *its* body's wall(s) with the contacts it cuts, and
re-emits each cut contact tagged for the caller to substitute. When two bodies cut the same bed,
each body's call re-emits the same `(series, interface)` contact with only its own seam resolved;
the other body's cut is applied as a mere field masker. There is no single mesh in which both
seams are shared, so the crossing region of two bodies is never jointly corefined. The **vein-
crossing-intrusion** scenario (a canonical "arbitrary CSG" case) therefore only composes correctly
via the Manifold recipe path (§2), not in Surfaces mode and not with Clip/corefine quality.
*Direction:* drive multi-body scenes through one shared build scene, or document that seam-quality
paths are single-body and multi-body must use the Manifold recipe.

### 3.3 [MEDIUM] Imported `VeinWall` (and `Contact`) roles contribute no cut — a declared role that does nothing
`stack.rs:698-708`

`ImportRole` advertises four roles and `US-IMPORT-2` promises "the same mesh can play any
stratigraphic part a fitted surface can," but the `VeinWall` arm is empty `{}` — a single imported
sheet never builds a band or trims older rock. So an imported vein wall is inert.
*Direction:* wire it (a vein needs a footwall/hangingwall pairing), or reject the role at ingress
so the UI cannot offer a no-op.

### 3.4 [MEDIUM] Onlap pinches out only against older *stratigraphic series*
`stack.rs:716-724`

Only `StackElement::Series` older elements clip a younger onlap; an onlap lapping onto an older
imported topography, an intrusion dome, or an imported contact passes straight through it. `§3.1.2`
describes onlap generically as lapping "up against a pre-existing slope."
*Direction:* extend the older-element loop to imported contacts/topography (bodies are legitimately
excluded — document that).

### 3.5 [MEDIUM] Fitted erosion can over-/under-erode via cokriging extrapolation drift
`stack.rs:650-658` (masker), mitigations only at `:691-706` (imported) and `:119-133`/`:892`
(corefiner no-crossing gate)

The fitted erode masker is a global half-space `field > iso_top`. The code itself documents that a
decimated/under-constrained level set "can drift far from the mesh and wrongly clip older rock that
lies wholly on one side" — but that survival gate is applied *only* to imported erosion and to the
corefiner path, **not** to the default fitted Snap path. Stacking several fitted erosions compounds
the drift.
*Direction:* apply the same centroid/field survival gate to the fitted field-CSG masker path.

### 3.6 [MEDIUM] Coplanar "drop C, keep W" silently deletes a genuine surface
`mesh_clip.rs:985-993`, `:1018`

Where a C sub-triangle centroid projects into a coplanar W cutter, C's surface is unconditionally
discarded. For two imported meshes that share a coplanar patch (a fault plane reused as a unit
boundary; two slabs meeting flush) one sheet vanishes with no warning → a leak or a missing wall.
*Direction:* detect genuinely-distinct coplanar surfaces and keep both, or warn.

---

## 4. Robustness — silent/garbage failure & panics reachable from user data

### 4.1 [CRITICAL] Non-finite `.obj` input panics and poisons the once-per-worker kernel instance
`obj.rs:133` (parses `"nan"`/`"inf"`), `:78` (degenerate check misses NaN) → `wasm.rs:736-766` →
`litho.rs:63` (`partial_cmp(…).unwrap()`), `lib.rs:601`, `data.rs:113`

`f64::from_str` accepts `nan`/`inf`, and neither the area test (`area2 <= s*1e-20 || area2 == 0.0`,
both `false` for NaN) nor the pole test (`len == 0.0`, `false` for NaN) rejects them. The NaN
reaches unguarded `unwrap()`s in the **kernel**, which has no panic hook. A wasm panic is an
uncatchable trap that **poisons the kernel instance** — instantiated once per worker
(`worker.ts:40`, `kernel-bridge.ts:62`) — so *every* element already fitted in the current Apply is
lost, and the user sees only `RuntimeError: unreachable`, not the readable error `§2.2/§2.3`
promise. One malformed imported surface takes down the whole composition.
*Direction:* reject non-finite vertices/normals in `parse_obj` with a typed `ObjError`; replace the
kernel `unwrap()`s with NaN-safe comparators (`total_cmp`); install a kernel panic hook.

### 4.2 [CRITICAL → PARTLY RESOLVED / PREMISE CORRECTED] The fit ABI has no failure channel
`abi.rs` (`gf_fit`/`gf_fit_faulted`), `solve.rs` (Bunch–Kaufman), `remote.rs` (`fit` cannot fail)

> **UPDATE (make-failure-loud pass).** The stated mechanism — "a singular/degenerate system yields
> NaN weights, a NaN field" — **did not reproduce under test.** The nugget-regularized cokriging
> solve keeps `residual` **and** `isovalue_spread` finite and near-zero even for coincident points,
> 1e160 coordinates, zero-length orientation poles, and mutually-contradictory interface points: it
> interpolates *through* the data, so the recovered isovalues always self-agree. The real failure of
> an under-constrained fit (a 2-point vein) is a **finite, self-consistent field whose level set is
> geologically wrong** (folds/drifts) — a *worse* failure than NaN because it is plausible, and one
> that lives in the field GEOMETRY, not in any fit-time solve scalar. No cheap diagnostic here
> detects it (detecting it needs field evaluation), so the residual/spread "fit-failure channel"
> this finding asked for would be **near-dead code**.
>
> **What was done instead:** the kernel now installs a panic hook + a generic `kernel_host.kernel_log`
> host diagnostic sink (see 4.1), forwards a fit-failure warning to the app UI when a fit IS non-finite
> (`Model::fit_is_finite` — the only sound signal, covering input that slipped ingress), and the host
> reads it via the already-plumbed `WasmStack.residual()`. The geometric-fold detector and any
> user-facing fit-conditioning lever (see §1.3) remain **open and are the real fix** — scoped
> separately, not by an ABI sentinel.

*Original direction (superseded):* return a sentinel handle when residual/weights are non-finite.

### 4.3 [HIGH] `decode_fit` / `read_f64s` trust buffer-header counts and index unchecked
`abi.rs:147-200`, `read_f64s` `abi.rs:116`

`n_sp`/`n_ori` are read from the buffer (f64→usize: NaN→0, huge→`usize::MAX`) and used to index `v`
with no bounds check; `read_f64s` does `from_raw_parts(ptr, n*8)` with the caller's `n`. Any length
desync reads OOB kernel memory and panics/aborts. Fine for today's single consistent caller, but
this is the ABI contract surface for composition.
*Direction:* validate `13 + 4*n_sp + 6*n_ori (+6)` against `len_f64`; return a fit-failure sentinel.

### 4.4 [HIGH] `gf_alloc`/`gf_free` capacity mismatch is latent UB
`abi.rs:95` (`Vec::with_capacity(len)` + `mem::forget`), `:109` (`Vec::from_raw_parts(ptr, 0, len)`)

`from_raw_parts` requires the capacity to equal the allocation's real capacity; `with_capacity` may
over-allocate, so freeing with `len` is UB. It works with wasm dlmalloc today but is exercised on
every fit and scratch grow.
*Direction:* track and free with the real capacity, or use a fixed `Layout`.

> **UPDATE (robustness cleanup — done):** `gf_alloc`/`gf_free` now share a fixed `buffer_layout(len)`
> (align 8, `len == 0` → dangling no-op) and allocate/free via `std::alloc` with the *identical*
> layout, eliminating the capacity mismatch. Native round-trip test `gf_alloc_free_roundtrips_various_sizes`;
> builds clean for `wasm32-unknown-unknown`.

### 4.5 [MEDIUM] Imported-mesh parse/store is uncapped for huge/hostile files
`wasm.rs:733` caps the *fit* to 400 pts / 120 oris, but `parse_obj` (`obj.rs:117`), the weld
HashMap (`obj.rs:170`), and the retained `TriMesh` (cloned by `imported_surface_contact`,
`wasm.rs:910`) are all O(n) with no byte/vertex/triangle budget. A multi-GB `.obj` OOM-aborts the
instance before the fit cap applies. Untrusted user upload.
*Direction:* enforce a size budget in `parse_obj`; reject with a readable `ObjError`.

> **UPDATE (robustness cleanup — done):** `parse_obj` now enforces an `ObjLimits` budget (256 MiB /
> 8M vertices / 16M faces) and returns `ObjError::TooLarge`; the byte cap is checked O(1) up front
> before any allocation. Test `rejects_oversize_input` exercises all three caps via an injectable-limits
> internal.

### 4.6 [MEDIUM] `assert_eq!(uni_degree, 1)` and other kernel asserts reachable from the ABI
`assemble.rs:121`, plus `partial_cmp(...).unwrap()` at `octree.rs:471`, `octree.rs:2100`

`uni_degree` passes through `decode_fit` verbatim; any caller passing 2 traps the instance. The
octree `unwrap()`s panic on NaN/inf field distances (an ill-conditioned fit reaches them via
`project_to_contact`/`seam_boundary_normal`), and `sample_corners` propagates NaN silently
(`octree.rs:1789`) so it fails late rather than fast.
*Direction:* clamp/validate `uni_degree` at the ABI edge; use `unwrap_or(Equal)` and a NaN guard in
`sample_corners`.

> **UPDATE (make-failure-loud pass + follow-up — done):** `uni_degree` is clamped at the ABI edge and
> the octree `partial_cmp().unwrap()`s are now `unwrap_or(Equal)` (earlier pass). The **residual** the
> verification surfaced — `sym3_eig`'s raw `partial_cmp().unwrap()` (`lib.rs:624`) reachable from a
> host-staged NaN coordinate — is now closed two ways: the sort uses `f64::total_cmp` (never panics),
> and `decode_fit` **rejects any non-finite coordinate/pole at the ABI edge** (`host_log` warning +
> empty fit — loud, like the `.obj` guard). Test `decode_fit_rejects_non_finite_coordinates`.

### 4.7 [MEDIUM] No `worker.onerror` handler — some worker/wasm failures leave a blank view
`main.tsx:196-226`

Only `worker.onmessage` is wired. A module-load failure, a wasm instantiation error surfaced as an
`error` event, or an uncaught async rejection fires `worker.onerror`, which is unhandled — no
status, view stays blank/stale. Contradicts `§2.2`.
*Direction:* set `worker.onerror` to a readable error status and clear `applying`.

---

## 5. The "exact / watertight" guarantees are weaker than advertised

### 5.1 [CRITICAL-for-claims] The "exact predicates" are default-off and wired into nothing shipping
`exact_arr` gated `#[cfg(feature = "exact_arr")]` (`lib.rs:47`), off by default (`Cargo.toml`);
zero references from `mesh_clip.rs` or `stack.rs`. `exact_tri_tri_segment` handles one transversal
tri-tri and *defers* coplanar (`tri_tri.rs:17`).

The shipping `tri_tri_intersection` (`mesh_clip.rs:537-615`) builds every seam point in **f64** via
`edge_cross` (`t = da/(da-db)`, `:771`) and classifies with absolute/relative epsilons (`snap0`
`|d| < 1e-12` at `:1411`; `COPLANAR_SIN2 = 1e-12`; `COPLANAR_DIST_EPS = 1e-9`). Every
"native/wasm-identical", "no near-zero graze can flip combinatorics" claim in the module docs
therefore describes a **disabled** path; cross-target determinism rests only on spade's 2D CDT
predicates, while the 3D seam *positions* and above/on/below *classification* are accumulated f64
and can diverge native↔wasm exactly where `robustness.rs:117-152` says they will.
*Direction:* either wire the existing exact segment construction into `tri_tri_intersection`, or
stop advertising exactness on the shipping path.

### 5.2 [HIGH] Watertightness is *checked and declined*, not produced
`stack.rs:2124` (`corefine_one_solid(...)?`), `mesh_clip.rs:3462` (`if !solid_watertight(…) { return None }`)

The corefiner assembles, then tests, then bails to the legacy independent/snap path on any failure
(under-resolved wall, float divergence at a graze). Reliability is delegated to a fallback, not
achieved — and one non-watertight unit discards the smooth seam for the *whole* body
(`stack.rs:1909-1914`), a quality cliff under composition.
*Direction:* fall back per-unit rather than per-body; treat repeated declines as a surfaced
diagnostic, not a silent downgrade.

### 5.3 [HIGH] A scale-relative weld quantum reintroduces the extent fragility the seam key avoids
`stack.rs:2111` (`q = (size * 1e-9).max(1e-12)`), `mesh_clip.rs:3435` (spatial-hash reweld),
`mesh.rs:460` (`diag * 1e-9`)

The combinatorial seam key (`mesh_clip.rs:853-905`) deliberately avoids a position weld, but
`corefine_one_solid` re-welds the reassembled mesh with a quantized spatial hash whose cell is
`size * 1e-9`. At UTM/million-scale extents this quantum is coarse relative to fine seam features,
so distinct vertices merge → pinched/non-manifold solids — exactly the extent-fragility the
barycentric seam keying was introduced to defeat, reintroduced one stage later.
*Direction:* rejoin via `res.shared` index correspondence instead of a position weld.

### 5.4 [MEDIUM] `snap0` and several thresholds are scale-variant on unnormalized normals
`mesh_clip.rs:1411` (`snap0`, absolute `1e-12` on `dot(n_c,w)+d_c` where `|n| ~ edge²`), `:834`
(`COPLANAR_EPS2 = 1e-18`)

The sibling parallel test was made dimensionless on purpose; `snap0` was not. For large meshes the
graze snap never fires; for tiny meshes it over-snaps — either way it flips vertex-on-plane
classification, which drives the seam combinatorics.
*Direction:* make `snap0` relative to `|n|·extent` like the sibling tests.

### 5.5 [MEDIUM] Whole-mesh classification by one representative centroid when no crossing is found
`mesh_clip.rs:1311-1318` (`empty_seam_result`), mirror `stack.rs:891` (`contact_survives_erosion`)

When no tri-tri crossings exist, the entire C mesh is kept/dropped by a single first-triangle
centroid — assumes C lies wholly on one side of W. For an imported W that does not fully partition
the domain (an open sheet smaller than C, or a disconnected wrap), the whole of C is mis-kept or
mis-dropped.
*Direction:* per-connected-component (not per-mesh) side classification.

### 5.6 [MEDIUM] Self-crossing seam within one face: debug panic / release tear
`mesh_clip.rs:1246` (`debug_assert_eq!` panics in debug), `:1260` (release drops split vertices → tear)

Several W triangles cutting one coarse C triangle in a crossing pattern (realistic under arbitrary
composition) hits a known-unhandled case: debug panics, release tears the face → non-watertight →
the whole solid declines (5.2).
*Direction:* handle multi-constraint CDT splits, or refine the offending C face before CDT.

---

## 6. Meshing (octree dual contouring) — feature preservation

### 6.1 [CRITICAL] Sub-cell features in the *primary* field are undetectable — thin veins/small intrusions silently dropped
`octree.rs:1796-1810` (`has_field_crossing` — corner sign-change only), refine gate `:2126-2156`

`build` only recurses where a **corner** edge already sign-changes. A thin sheet/vein whose field
dips below `iso` and back *between two corners* produces no corner sign change, so the cell stays a
leaf at every depth down to `min_depth` (default 2 → cell ≈ extent/4 ≈ 250 in a 1000-box) and the
feature is **never meshed, at any depth, with no warning.** The partial mitigations
(`SlabComplement::edge_crossing_count` `:314`, `cell_double_surface` `:560`) only fire for
*contributor* slabs and require *two* surfaces straddling — a lone thin primary sheet is missed.
This directly defeats "reliably compose arbitrary" for thin bodies.
*Direction:* track per-cell field min/max (or sample along edges) to catch non-monotone crossings;
or require thin features as `SlabComplement` contributors and document the primary-field limit.

### 6.2 [HIGH] `project_to_contact` deliberately places vertices up to 2 cells outside their cell
`octree.rs:2048` (`PROJECT_OOB_CELLS = 2.0`), `:2099-2110`

`qef_vertex` hard-clamps into the cell (`mesh.rs:229`), but `assign_vertices` then runs a 6-step
Newton projection allowed to leave the cell along the gradient's dominant axis by up to `2*size`
(250 units on a depth-3 cell). The Newton step `s = f/g2` is unbounded when `g2` is small (guarded
only `< 1e-12`), so a near-flat/non-monotone ramp flings the vertex the full rail — the same class
as the historical "QEF ceiling spike," now *bounded* but not eliminated, and unproven for arbitrary
composed fields that are non-single-valued along the projection axis.
*Direction:* bound the per-iteration step; detect divergence (rising residual) and fall back to the
clamped QEF vertex; or reduce the rail below one cell.

### 6.3 [MEDIUM] Fixed `max_depth = 6` caps the finest resolvable feature; double-crossed cells are capped, not resolved
`octree.rs:1484`, `:2287-2316` (`refine_double_crossed` bails at `max_depth`; comment "capped, not
infinite" `:2286`)

Default finest cell ≈ extent/64 ≈ 15.6 in a 1000-box; features below that are not reliably meshed
even when detected — a residual double-crossed cell averages two sheets into a crack or drops the
feature. Resolution is a fixed cap, not adaptive to feature size, and `§8` forbids exposing it.
*Direction:* derive `max_depth` from an estimate of the smallest composed feature, or surface a
diagnostic when cells remain double-crossed at `max_depth`.

---

## 7. Efficiency

- **[MEDIUM] Every Apply re-parses and re-fits the entire stack**, even for meshing-only changes
  (depth, isovalue, seam mode) — `worker.ts:59-92` builds a fresh `WasmStack` and re-runs
  `parse_obj` + the dense ~O(N³) cokriging fit per element, with no fit cache
  (terminate-per-fit). Dominant cost for many-body/imported scenes. *Direction:* cache fitted model
  handles keyed by fit-relevant inputs; re-mesh only when meshing params change.
- **[MEDIUM] "Cheap declared-state" edits are not cheap.** Relation change re-fits the whole stack
  (`main.tsx:309-319`), and in Solids mode every lithology **color/name** edit triggers a full
  fit+mesh+Manifold compose (`main.tsx:284-290`) — violating `US-DATA-3`/`US-LITHO-2`'s
  "without a re-fit." A pure color edit needs no run at all (the Viewer already maps
  `lithologyId → color` live via the `lithoColors` prop). *Direction:* skip `runStack` for
  color/name edits; re-tag assignments in place.
- **[MEDIUM] `cuts_below` is a coarse 12³ point sample, recomputed at every meshing entry point**
  (`stack.rs:944-974`, and again at `:1132`, `:1186`, `:1651`, `:2007`). A thin vein falls between
  samples → reported as cutting nothing → seam co-meshing silently skipped. `clip_maskers(i)` is
  likewise rebuilt per-interface. *Direction:* derive the cut-set from AABB overlap of interface
  iso-bands (cheaper *and* thin-body-safe); hoist out of per-interface loops.
- **[MEDIUM] `winding()` / mesh point-membership is an un-accelerated O(triangles) scan**
  (`octree.rs:983-1004`, used by `TriMesh::inside`, `MeshSdf`). The BVH accelerates `nearest`/ray
  but not membership; the co-kriging/`MeshSdf` composition paths pay a linear scan per corner per
  node. (The default large-lidar path uses BVH-backed `MeshLocalField`, so this bites only the
  `MeshSdf`/direct-`TriMesh`-boundary paths.) Similarly `nearest_w_signed` scans all of W per uncut
  face in `clip` (`mesh_clip.rs:1164`) — quadratic on large imported meshes. *Direction:* route
  membership through BVH-signed distance / hierarchical winding number; reuse the WGrid for the
  nearest-W query.
- **[HIGH] Three.js geometry/material leak on every mesh rebuild** — `Viewer.tsx:566-604`:
  `st.group.clear()` (`:569`) only detaches; it never disposes the prior `BufferGeometry`,
  `MeshStandardMaterial`, or per-mesh `WireframeGeometry`/`LineBasicMaterial` (`:597-601`). Re-runs
  on every Apply, toggle, and color edit; GPU buffers accumulate for the whole session. The overlay
  path *does* dispose (`disposeOverlay`), so this is an inconsistent oversight. *Direction:*
  traverse and dispose before `clear()`.
  > **UPDATE (robustness cleanup — done):** the main mesh-rebuild effect now calls
  > `disposeMeshGroupChildren(st.group)` (traverse → dispose geometry + material, incl. the wireframe
  > `WireframeGeometry`/`LineBasicMaterial`) before `st.group.clear()`, mirroring `disposeOverlay`.
  > (TS not type-checked in this environment — `node_modules` absent — verified by review.)
- **[LOW]** Redundant field re-evaluation: no corner-sample caching across cells or parent/child
  (`octree.rs:1776-1793`, ≈ 8 × node_count kernel sums); `read_f64s` copies the whole fit buffer
  twice in the faulted path (`abi.rs:225-227`); every `WasmMesh` getter clones its `Vec` per JS
  access (`wasm.rs:155-166`).

---

## 8. Project round-trip & smaller spec gaps

- **[MEDIUM] Exported view omits `manifold` (and `showBounds`, `hiddenKeys`)** — `main.tsx:481-494`
  persists mode/clip/edges/overlay but not the `manifold` toggle; a project fit with `manifold` off
  round-trips back with it on (default) → *different solid geometry*, so `US-PROJ-4`/acceptance §9.5
  ("same model and viewpoint") is not fully met for solids. *Direction:* include `manifold` in the
  saved view.
- **[LOW] Diagnostic toggles fit the *dirty* project, not the last-applied one** —
  `main.tsx:255-258`, `:676-705`: toggling render mode/seam/manifold while edits pend silently fits
  un-applied edits while the scrim still claims staleness. *Direction:* fit `lastApplied.current`
  when dirty, or document.
- **[LOW] `applying` can stick if the project is reloaded mid-Apply** (`main.tsx:296-301` vs
  `:437-466` — `loadFreshProject` terminates the worker so `onDone` never clears `applying`).
- **[LOW] Degenerate band (equal wall isovalues) → silently empty body** (`stack.rs:250-255`,
  `:262-264` strict `>`/`<`); a mis-fit vein whose interfaces coincide vanishes with no diagnostic.
- **[LOW] Fixed-size composition limits:** `CAP = 8` fault half-spaces per block silently truncates
  (`remote.rs:255`); `lithology_label` packs `series*1000 + unit` (`wasm.rs:924`) — collides at
  ≥1000 units/series. Undocumented hard ceilings on composition depth.

---

## 9. What is solid (verified — no action needed)

- **Per-element cutting directions all match `SPEC.md §3.1.2`**: erosional trims *older*
  (`stack.rs:650`), onlap trims *itself* (`:716`), vein cuts older *inside its band* (`:660`),
  intrusion single/shell inside the envelope (`:667`), topography caps below ground (`:645`). The
  mirror-clip design is correct.
- **Youngest-wins composes arbitrarily in the Manifold recipe path** (`stack.rs:2219`,
  `emit_clip_defs` `:2135` → `clip_maskers` ANDs every younger clip into every older unit; shared
  contacts are the same primitive on both sides → Manifold corefines with no gap).
- **`qef_vertex` is a correct, regularized, cell-clamped QEF** (`mesh.rs:190-242`) with a mass-point
  fallback — the in-solver spike is genuinely prevented (the residual escapes only via 6.2's
  post-projection).
- **Recursion is bounded everywhere**; the shipping corefiner has **no reachable
  `unwrap`/`panic` on the release path** (only `debug_assert`s and the disabled `exact_arr`).
- **TS state alignment is correct**: `toSeriesJson`/`indexedAssignment` use the same
  topography-first sort, so series-index alignment between meshes, assignments, and `CoreUpdate` is
  consistent — no reorder off-by-one found. `US-PROJ-6` mesh path-error, Apply gating on
  fittability (`US-VALID-1`), and Export-OBJ-disabled-while-dirty (`US-PROJ-5`) are all correctly
  implemented. Overlay/bounds/rubber-band Three.js resources *are* disposed (making 7's main-mesh
  leak a clear oversight).

---

## 10. Prioritized recommendations

**Make failure loud (biggest reliability win, unblocks everything else):**
1. Add a fit-failure channel (4.2) and reject non-finite `.obj` + install a kernel panic hook (4.1);
   handle `worker.onerror` (4.7); surface Manifold-dropped units and compose failures instead of
   rendering empty-as-"done" (2.3).

**Fix the two composition-corrupting races and the leak:**
2. Run-token guards on `runStack`/Manifold tail (2.2) and don't clear `dirty` from a superseded
   Apply (the stale-Apply variant of 2.2); dispose Three.js buffers before `clear()` (7).

**Close the correctness landmines:**
3. Hoist topography in `global_unit` (3.1); apply the erosion survival gate to the fitted path
   (3.5); decide `VeinWall`/imported-onlap: wire or reject (3.3, 3.4).

**Make "arbitrary" real for the quality paths:**
4. Either extend the seam-quality/corefine paths to multi-body (3.2) or route multi-body explicitly
   through Manifold and document it; per-unit (not per-body) corefine fallback (5.2); detect
   sub-cell primary features or document the limit (6.1); bound `project_to_contact` (6.2).

**If exactness/scale-robustness is a real requirement:**
5. Wire the existing exact tri-tri construction (5.1), make `snap0` scale-relative (5.4), and
   replace the reassembly weld quantum with index-preserving joins (5.3) — otherwise large-extent
   and near-degenerate composed meshes will keep diverging native↔wasm and declining to staircase.

**Performance (as composition scales):**
6. Cache fitted handles / add a re-mesh-only path (7); make declared-state and color edits truly
   free (7); AABB-based `cuts_below` (7); move Manifold to its own persistent worker (2.1).
