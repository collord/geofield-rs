# Starting prompt — fix the shared-octree under-refinement that drops a rock block

> Paste everything below the line into a fresh session. It is self-contained; it points at a
> confirmed diagnosis (`docs/10_shared_contour_block_drop.md`) and an instant repro.

---

## Task

In `geofield-rs`, cross-cutting-vein **solid** mode drops a whole rock block for middle units (the
"teal cone"): in `octree_dual_contour_shared`, the region **left of the vein** is left at `min_depth`
because the build refinement never flags it as a surface cell, so a rock unit whose entire z-band
falls inside one coarse cell there collapses. **Fix the shared contour so it refines wherever any
output scene's surface (especially a bed) crosses a cell — keeping the shared tree (do NOT switch to
the independent path).** The root cause is already confirmed; you are implementing + verifying the
fix, not re-diagnosing from scratch (but do sanity-check the diagnosis before changing code).

Read `docs/10_shared_contour_block_drop.md` first — full diagnosis, evidence, and what was ruled
out. Key prior context also lives in the memory note `volume-solid-seam-already-watertight` and
`docs/09_corefined_solid_followups.md`.

## Reproduce first (instant, native)

```
cargo test -p geofield --lib --release diag_vertical_vein_three_beds -- --nocapture --ignored
```

Read the ASCII `(x→, z↑)` dumps it prints (this test reproduces the EXACT web3 `cross-cutting-vein`
fit natively — vertical walls x=400/600, `threeContactFloor` beds z=300/500/700, `uni_degree=1`):
- `teal-shared u2` — left block GONE (only a sliver by the foot wall + the right block). **The bug.**
- `teal-shared u2 @ min_depth=4 / =5` — left block returns as you force subdivision. **The proof it's
  under-refinement.** At `min_depth=5` teal is two full clean blocks.
- `teal-INDEPENDENT u2 …` — the single-scene path (`mesh_one_block_unit_solid`) renders both blocks
  correctly (this is the reference for "right answer"), and `+ projection` keeps both blocks.

Your fix succeeds when **`teal-shared u2` shows two full blocks at the DEFAULT `min_depth=2`**, like
the `min_depth=5` and INDEPENDENT dumps — without globally bumping `min_depth` (too expensive).

## Confirmed root cause (mechanism)

`octree_dual_contour_shared` (`crates/geofield/src/octree.rs:1140`) builds ONE tree from
`build_scene`, then per output scene does `resample_leaves` + `contour_on_tree` — it **never
re-refines per output scene**. Subdivision happens in `build` (`octree.rs:1617`) where
`is_surface(corners, inside)` is true (`octree.rs:1307`):

```
is_surface = has_field_crossing(corners)        // primary field changes sign across the 8 corners
          || has_boundary_crossing(inside)       // contributors_inside (AND of all contributors) flips
```

`contributors_inside` (`octree.rs:436`) is the AND over `scene.contributors`. In
`mesh_cross_cut_solids_shared` (`crates/geofield/src/stack.rs:1498`), `build_scene` is assembled at
`stack.rs:~1617-1641` with primary = vein/body field and contributors that include `body_wall_refine`
(the vein walls as `ContactHalfSpace`, `stack.rs:1581`) and `bed_refine` (beds as `ContactHalfSpace`,
`stack.rs:1587`). The wall half-spaces force the AND **false everywhere left of the vein**. So out
there: the vein primary doesn't cross (`has_field_crossing` false) AND the AND is uniformly false so
a bed crossing never flips it (`has_boundary_crossing` false) ⇒ `is_surface` false ⇒ cell stays at
`min_depth`. Teal (z∈[500,700]) fits inside one coarse [500,750] cell layer → collapses; blue
(z∈[300,500], straddles the 500 boundary) survives.

The intent of `build_scene` was the opposite — "refine wherever ANY surface crosses so every output
scene resolves." `edge_double_crossed` (`octree.rs:523`) already counts crossings of ALL surfaces
(primary + every contributor) — that's the right idea, but it's only used by `refine_double_crossed`
(needs ≥2 crossings on one edge), not by `build`'s `is_surface` (needs the solid boundary to flip).

## Fix candidates (evaluate; lead with the smallest that works)

1. **Union-of-surfaces refinement for the BUILD phase.** Make `build` (and `balance`) subdivide a
   cell if ANY contributor surface crosses it OR the primary crosses — not only where the AND-inside
   flips. Concretely: add a cheap `scene.any_surface_crosses(min, size)` (any cell edge has a primary
   sign change or any `contributor.edge_crossing`/`edge_crossing_count ≥ 1`) and OR it into the build
   stop-criterion. This is the most direct match to the stated intent. Watch: more cells (bounded by
   `max_depth` and the surfaces actually present); confirm runtime stays acceptable (release
   mesh-only was ~1–2 s).
2. **Don't poison `contributors_inside` with the wall refiners.** `body_wall_refine`/`bed_refine`
   exist only to drive subdivision, yet they sit in the same `contributors` vec that
   `contributors_inside` ANDs. Consider separating "refinement contributors" (drive `is_surface`/
   `edge_double_crossed`, never the inside-AND) from "solid contributors" (the box/maskers/lower-cap
   that actually bound the solid). This is cleaner but touches the `Scene` API — check all callers.
3. **Targeted:** ensure each bed drives subdivision independently of the wall AND (e.g. build the
   tree from a refinement scene whose `is_surface` ORs every contributor crossing). Similar to (1)
   but scoped to the shared path.

Reject: globally raising `min_depth` (fixes it — proof above — but far too expensive); switching to
the independent path (that's Option A, explicitly NOT chosen).

After the fix, also re-check the **dipping** vein (curved walls) — the same under-refinement could
bite an oblique wall differently; use the `vertical_vein_over_column()` fixture (dxdz=0.5,
`stack.rs:~3123`) and add a sibling diag if needed.

## Verification gates (all must stay green)

- `diag_vertical_vein_three_beds` ASCII: teal = two full blocks at default `min_depth=2`.
- `a2_crease_projection_smooth_watertight_fused` — crease still smooth, watertight, fused, same
  topology (the `docs/09` crease projection must still work on the now-complete mesh).
- `cross_cut_solid_seam_is_watertight` — Strategy-A solids watertight.
- Independent `Snap` path stays **byte-identical** (`octree_dual_contour_scene` must be untouched in
  behavior; only the shared/build refinement changes). Diff a known mesh if you touch shared code.
- **Native==wasm determinism** holds (the octree is bit-deterministic native↔wasm for the same fit —
  proven; don't regress it). After native is green: `cd web3 && npm run wasm:engine` then
  `npx playwright test cross-cut solids-toggle` (e2e), then a visual check of
  `cross-cutting-vein` + `cross-cutting-vein-dipping` in Solids mode.

## Tooling notes / gotchas

- **The fit must be EXACT or you chase ghosts.** An earlier "6.5× divergence" and a wrong tri count
  were both fit mismatches (flipped pole; mis-sampled beds). The web3 inputs: vein `verticalWall`
  x=400/600 → 2 pts each (z=200,800) + one `[1,0,0]` orientation per wall; `threeContactFloor` beds
  z=300/500/700 → 2 pts/bed at x=200,800 + one `[0,0,1]` orientation per bed; `uni_degree=1`,
  rest `KernelOptions::default()`. `dipDirToPole` is in `web3/src/HierarchyPanel/project.ts:145`.
  The diag test already encodes this exactly — reuse it.
- **Diag helpers** in `diag_vertical_vein_three_beds` (`stack.rs`): an `ascii(mesh, name)` (x,z)
  occupancy printer, a `zhist` interior z-histogram, per-unit Snap-vs-Clip bbox/tris. Add more as
  needed; it's `#[ignore]`, native-only.
- **No wasm cargo-test harness** (empty `[dev-dependencies]`, see memory
  `geofield-no-wasm-cargo-test-harness`): cross-target determinism can only be pinned natively +
  asserted; the parity gate is the web/Playwright e2e.
- Worker console isn't captured by Playwright (workers postMessage); to inspect web3 internals, read
  `window.__surfaces` in a `page.evaluate` (each payload has `series`, `unit`, `positions`,
  `indices`), or dump from the Rust side natively.
- Build/test: `cargo test -p geofield --lib --release <name>`; wasm: `cd web3 && npm run wasm:engine`;
  e2e: `cd web3 && npx playwright test <spec-substr> --reporter=line`. Put throwaway e2e specs in
  `web3/e2e/zz-*.spec.ts` and delete after.
- **git:** commit directly to `main` (no feature branch); commit/push ONLY when the user asks. End
  commit messages with the `Co-Authored-By: Claude …` trailer. Nothing in this work is committed yet.

## State of the tree (uncommitted)

- `crates/geofield/src/stack.rs`: the `#[ignore]` diag test `diag_vertical_vein_three_beds` (repro +
  evidence). The `docs/09` crease fix (`project_crease_ring` structural selection + 1.5·cell guard) is
  in and green. `mesh_cross_cut_solids_shared(…, seam: SeamMode)` is the shared path.
- `crates/geofield/src/octree.rs`: shared contour is unchanged by this investigation.
- web3 worker/wasm edits made during diagnosis were reverted (no net change there).
- Dead code to clean up eventually (independent of this fix): the clip-based
  `mesh_cross_cut_solids_corefined` + `corefine_one_solid` + their tests (superseded, native-only).

## Ruled out (don't re-chase)

- Native↔wasm divergence — exact-fit native tri counts match wasm exactly. The octree is
  deterministic; A.1/contributor-sign-snap is refuted.
- The `docs/09` crease over-selection — fixed (clean 256-vert ring, ≤cell, 0 reverted). The crease
  projection is correct; it just can't fix a missing component.
