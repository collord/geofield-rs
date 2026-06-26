# Stage 0 scope — exact-arithmetic corefiner de-risk spike

> ## Stage 0 result (2026-06-25) — determinism kill-test: **PASS (Layer 1)**
>
> The cheapest, most decisive sub-gate is GREEN. The exact tri-tri spike is built (default-off
> `exact_arr` feature, sibling module `crates/geofield/src/exact_arr/`, never touching `mesh_clip`):
> - **Layer-1 determinism PROVEN bit-identical native↔wasm.** The exact construction's FNV digest is
>   `0x0E4B9D0B1F846BD1` on native x86-64 **and** on the real browser target `wasm32-unknown-unknown`
>   executed in V8 (node) — the same engine the browser runs. The output bits come from rounding an
>   exact rational once, not accumulated f64 ops, so they match by construction. Native golden test:
>   `exact_arr::tri_tri::spike_digest_is_deterministic_and_pinned`. Exactness verified independently
>   (`crossing_endpoint_is_exact_nondyadic` lands the precise `(1, 5/3, 0)` non-dyadic crossing).
> - **No regression / default build unchanged.** Existing native suite 118 passed / 0 failed with the
>   feature off; the wasm artifact only grows when `exact_arr` is explicitly enabled.
> - **Canonical CI gate wired** (not run this session — heavy wasm-pack build): `#[wasm_bindgen]
>   exact_spike_digest_hex` → `window.__exactSpikeDigest` → `web3/e2e/exact-determinism.spec.ts`,
>   runnable via `npm run test:e2e:exact`; skips cleanly on the default build. The node/V8 run above is
>   the executed proof; Playwright is its permanent form (browser and node share V8).
>
> **Still open in Stage 0 (next increments):** the lazy interval filter + rational-fallback-rate
> benchmark (§3); the **Layer-2** perturbation test (1-ULP divergent inputs, §2.2) — the genuinely
> hard determinism question, which points at Track B; the CGAL golden corpus (§4, sequenced last).
> **Net:** the corefiner-internal determinism premise holds, exactly as §5 predicted — which re-points
> the program's hard risk at Track B (cross-target-deterministic field eval at feature depth), not the
> corefiner math. Do not touch `mesh_cross_cut_solids_*` until the full §5 gate (fallback rate + Layer 2)
> closes.

Scopes the **Stage 0** gate of [`docs/11_exact_corefiner_spec.md`](11_exact_corefiner_spec.md)
against the actual repo. Stage 0 exists to **cheaply validate or kill** the exact-corefiner program
*before* committing to the open-ended arrangement/classification work or touching
`mesh_cross_cut_solids_*`. Everything here is concrete: verified crate facts, the exact file:line
slot-in points, and the gate criteria.

Two carried constraints (do not lose):
1. **The determinism gate cannot be a wasm `cargo test`** — this crate has no `wasm-bindgen-test`
   harness (see memory `geofield-no-wasm-cargo-test-harness`). Native↔wasm parity is proven via the
   web3/Playwright path. Designed below as such; the heavier alternative (add a wasm test runner) is
   noted as an explicit, optional prerequisite.
2. **This is half the program.** Exact corefining is worthless if the octree never resolves the
   feature. A ~2 ft feature in a ~20 mi model is ~17 octree levels; uniform refinement is impossible.
   "Adaptive resolution at the feature" is a **parallel necessary track (Track B)**, scoped in §7.
3. **(New, from this session) Every new meshing path is built parallel to the current code, behind a
   feature flag** — never an in-place replacement of `mesh_clip::clip`. Architecture in §6.

---

## 1. Crate choices — verified license + wasm-build audit

**Decision: pure-Rust, `geometry-predicates` + `dashu`. GMP/`rug` is off the table unless the §3
benchmark forces it.** All claims below were verified by building a scratch crate that exercises
every candidate against `wasm32-unknown-unknown` *and* native, and by reading the transitive license
tree from `cargo metadata`.

| Crate | Version | License | Role | wasm32 | FFI |
|---|---|---|---|---|---|
| `geometry-predicates` (elrnv) | 0.3.0 | **MIT** | orient2d/3d, incircle/insphere **+ Shewchuk expansion arithmetic** (`two_sum`, `two_product`, `grow_expansion`, `fast_expansion_sum_zeroelim`, …) — the lazy-construction building block | ✅ builds | none |
| `dashu` (+ `-int`/`-float`/`-ratio`) | 0.4.4 | **MIT OR Apache-2.0** | exact rational fallback when expansions can't close a construction | ✅ builds | none |
| `num-rational` / `num-bigint` | 0.4.2 / 0.4.6 | MIT OR Apache-2.0 | fallback alternative to `dashu` (kept as a swap option behind the trait) | ✅ builds | none |
| `robust` | 1.2.0 | MIT OR Apache-2.0 | **already in-tree** via `spade`; predicates only (no expansion primitives exposed) | ✅ builds | none |
| ~~`rug`~~ | 1.30 | **LGPL-3.0+, GMP/MPFR C FFI** | rejected by default — violates the FFI-free + zero-LGPL goal | — | C FFI |

Findings:
- **Zero GPL/LGPL/AGPL anywhere in the transitive tree** for the pure-Rust set (22 packages, all
  MIT/Apache; the only non-permissive token is `Unicode-3.0` on `unicode-ident`, which is a data
  license, not copyleft).
- **FFI-free confirmed**: no `*-sys` crates and no `links =` manifests. The build scripts that *are*
  present (`dashu-int`, `num-traits`, `proc-macro2`, `quote`, `rustversion`) are all `autocfg`/
  `rustversion` cfg-detection — no C, no native linkage.
- **`geometry-predicates` over `robust` for the kernel**: `robust` only exposes finished predicates;
  `geometry-predicates` additionally exposes the *expansion* primitives publicly, which is what the
  lazy-exact construction DAG is built from. We keep `robust` (it arrives transitively through
  `spade` anyway) but the new exact kernel imports `geometry-predicates`.
- **`dashu` over `num-rational` as the default rational**: both build and license identically; `dashu`
  is the spec's preference and benchmarks faster on big-rational arithmetic. They sit behind a
  `trait ExactScalar` so the §3 benchmark can swap them with a one-line change if `dashu` disappoints.
- **Coordinate-exponent guard** (spec caveat): `geometry-predicates` loses correctness for
  `|coord| < 1e-142` or `> 1e201`. UTM/geological scales are nowhere near, but add a `debug_assert!`
  that every input coordinate is within a safe exponent band so this can never silently bite.

All new deps go in as **`optional = true`**, pulled only by the new `exact_arr` feature (§6), so the
default `geofield.wasm` artifact does not grow until the path is real.

---

## 2. The single end-to-end exact tri-tri spike

### 2.1 Where it slots vs `mesh_clip.rs`

The current corefiner uses **exact predicates** (via `spade`'s `robust`) but **inexact f64
constructions**. There is exactly one construction primitive, and the whole f64-divergence surface
funnels through it:

- [`edge_cross` — `crates/geofield/src/mesh_clip.rs:771`](../crates/geofield/src/mesh_clip.rs#L771)
  `t = da/(da − db); p = a + t·(b − a)` — the segment×plane crossing point, in f64.
- Called by [`tri_interval:736-737`](../crates/geofield/src/mesh_clip.rs#L736) to build the two
  interval endpoints; [`tri_tri_intersection:614`](../crates/geofield/src/mesh_clip.rs#L614) returns
  them as `TriTri::Crossing(lo, hi)`.
- The signed distances `da/db` are themselves exact functions of the f64 inputs
  (`dot(n, v) + d`, lines 553-569) — i.e. the *entire* tri-tri intersection is a rational function of
  the input coordinates. **That is what makes an exact construction possible: no transcendentals, only
  `+ − × ÷`.**

**The spike replaces `edge_cross` with an exact construction** in a *new* module (never editing
`mesh_clip.rs`): compute `t` and `p` as an exact value (Shewchuk expansion, escalating to a `dashu`
rational only if the expansion can't represent `÷`), carry it through the interval/segment logic,
and round to f64 **once**, at output. The combinatorial seam-keying already in place
([`Feature` / `SeamVertices::intern`, mesh_clip.rs:803-906](../crates/geofield/src/mesh_clip.rs#L803))
composes unchanged — the key gives shared *identity*; exact coords make the shared *position*
bit-exact. The spike does **not** need the full clip/CDT; it is just: one f64 triangle pair →
exact intersection point(s) → f64 snapshot → digest.

New code lives in `crates/geofield/src/exact_arr/` (`mod tri_tri`, `mod scalar`), behind the
`exact_arr` feature.

### 2.2 The determinism test — two layers, do not conflate them

The output f64 bits come from **rounding an exact value**, not from accumulated f64 ops. Round-to-
nearest-even of an exact rational is a deterministic function of that rational alone — so *if the two
targets see the same exact value, they emit the same bits by construction.* That is the premise.
But there are **two distinct determinism questions**, and the cheap gate only answers the first:

- **Layer 1 — corefiner-internal determinism (given identical f64 inputs).** Same input triangle
  pair on native and wasm → bit-identical exact output. This is what the spike proves, and it is
  *likely already true even for the current f64 path*: Rust does not auto-contract `a*b+c` (no
  `-ffast-math`), tri-tri uses no transcendentals, and `sqrt` is IEEE-exact — so strict-IEEE f64 over
  `+ − × ÷` is already bit-reproducible native↔wasm. The exact path makes this *certain* rather than
  *probable*.
- **Layer 2 — end-to-end pipeline determinism (octree → corefiner).** The `docs/09` wasm-garbage
  cascade originates **upstream**: the octree/field eval crosses the wasm-module boundary to a
  separately-compiled kernel whose transcendentals (kriging covariance) differ at ULP, so the octree
  *vertices* fed to the corefiner already differ native↔wasm. An exact corefiner rounds to the nearest
  f64 of the true intersection **of its inputs** — if the inputs differ by 1 ULP, the true
  intersection differs and so does the output. **Exact construction is necessary but not sufficient;
  it cannot fix divergent inputs.** This couples directly to Track B (§7): the field/octree must also
  be cross-target deterministic at depth, or the corefiner inherits the divergence.

The spike must test both, and the go/no-go (§5) must report them separately.

### 2.3 The native↔wasm gate, built on the existing pattern

The native half already exists and is the template:
[`clip_output_is_deterministic` / `encode_clip` / `fnv1a`, mesh_clip.rs:2566-2637](../crates/geofield/src/mesh_clip.rs#L2566).
Mirror it for the exact spike:

- **Native golden** — `#[test] exact_spike_golden()` (in `exact_arr`): run the exact tri-tri on a
  fixed fixture, `fnv1a(encode(...))`, assert `== GOLDEN` const, print the digest. (Existing pattern,
  no new infra.)
- **Wasm entry** — `#[wasm_bindgen] pub fn __exact_spike_digest() -> u64` gated by `wasm + exact_arr`,
  running the *same* code on the *same* hard-coded fixture.
- **Cross-target gate** — a new Playwright spec `web3/e2e/exact-determinism.spec.ts`, modeled on
  [`web3/e2e/qef-spike.spec.ts`](../web3/e2e/qef-spike.spec.ts) (the existing wasm-only invariant
  gate): load the page, call `window.__exactSpikeDigest()`, assert `=== GOLDEN` (the native constant,
  mirrored as a JS literal). Match → Layer-1 determinism holds end-to-end through the real wasm
  artifact. Build+run is the existing `cd web3 && npm run wasm && npm run build && npm run test:e2e`.

For **Layer 2**, add a second spike assertion: perturb one input vertex by 1 ULP (simulating octree
divergence) and check whether the combinatorial keying yields the **same topology** with output
differing only at the ~1-ULP scale — i.e. whether exact construction *degrades gracefully* under
divergent inputs or cascades. This is the more informative result.

**Cross-target caveats to encode as the spike's own guards** (the mechanisms by which Layer 1 could
still fail): no `f64::mul_add` / fma in the exact path; no transcendentals; the final round-to-f64
must be the single rounding step. If any of these are violated the digest will diverge and the gate
catches it.

> Alternative (optional prerequisite): adding `wasm-bindgen-test` + `wasm-pack test --headless`
> would let this gate run as `cargo test --target wasm32` in CI without a browser. The Playwright
> path reuses existing infra and is the recommended default; note the wasm-test-runner option exists
> if browserless CI parity is later wanted.

---

## 3. Rational-fallback-rate benchmark (target < 1%)

The lazy strategy is only tractable if the rational escalation is rare. Measure it on **real geology
geometry**, not generic-position synthetic pairs (those would under-report — rationals fire at
degeneracies, which geology *generates*).

- **Fixture (primary):** [`vertical_vein_over_column()` — `stack.rs:3868`](../crates/geofield/src/stack.rs#L3868)
  — the in-process equivalent of the browser `cross-cutting-vein.geoproj.json` scene (dipping vein,
  `dxdz = 0.5`, two horizontal beds). Mesh at octree depths 6/7/8, take the body + rock surfaces, run
  all-pairs tri-tri through the exact path.
- **Also measure the degenerate fixtures** so the rate isn't misleadingly low: the grid-aligned graze
  cases and the 2-point dipping-vein fit (the memory's `dipping-wall-fixed-peekaboo` degeneracy
  source), plus `contact_and_wall()` / `curved_wall()` from
  [mesh_clip.rs:1602/2044](../crates/geofield/src/mesh_clip.rs#L1602).
- **Instrumentation:** three counters on the `ExactScalar` kernel — predicates evaluated, escalations
  past the f64 interval filter (→ expansions), escalations past expansions (→ `dashu` rationals).
  Report `rational_rate = rational_hits / total_predicate_evals`. **Target < 1%.**
- **Infra:** *no Criterion harness exists* (no `benches/`, no `[[bench]]`). The metric is a **rate, not
  wall-time**, so the smaller intervention is an instrumented `#[test]` that prints the rate (mirrors
  the existing ad-hoc `Instant` timing tests at `stack.rs:5533`). Add a Criterion bench only if
  wall-time tracking is later wanted — not for Stage 0.
- **Decision rule:** rate < 1% with `dashu` → pure-Rust confirmed, GMP permanently off the table.
  Rate ≥ 1% **or** `dashu` rationals dominate runtime at that rate → swap to `num-rational` behind the
  trait and re-measure before *ever* reconsidering `rug`/GMP.

---

## 4. CGAL golden-corpus harness (test data only; implementation stays independent)

Purpose: the **geometric-correctness oracle** — output vertices within ½ ULP of CGAL's corefinement.
**Not on the go/no-go critical path** (the gate is determinism + fallback-rate; correctness-vs-CGAL
matters for Stage 1 validation). Sequence it **last** in Stage 0 — it has the heaviest setup and is
wasted effort if the determinism gate fails.

License hygiene (spec §"Non-goals"): CGAL's PMP/corefinement is **GPL**. Generating golden *output
data* with it is fine; reimplementing from its *source* is a derivative-work exposure.

- **Keep CGAL out of the build.** A standalone, throwaway CMake/C++ tool under `tools/cgal-oracle/`
  (or a Docker image), explicitly documented as GPL, **not** a workspace member and **not** linked by
  anything in `crates/`. It uses `CGAL::Polygon_mesh_processing::corefine_and_compute_*`.
- **Data flow:** export the §3 fixtures' input meshes as `.off`/`.ply` → run the oracle offline →
  capture corefined output as data files (`.off` + a JSON of exact/decimal vertex coords) checked into
  `crates/geofield/tests/golden/`. The Rust exact corefiner is validated against those *files*; it
  never reads CGAL source.
- **Setup note:** `brew install cgal` (or the Docker route) is the single heaviest Stage-0 item →
  another reason to gate it behind a passing determinism result.

---

## 5. Go / No-Go on the determinism premise

**The gate must close before anything touches `mesh_cross_cut_solids_*`.** I cannot run the 1–2 week
spike from here, so the "decision" is the gate definition + sequencing + risk call below; the literal
go/no-go is read off the spike's digest results.

### Sequencing (front-load the cheapest kill-test — a sharpening over the spec's "do all three")
The spec lists three Stage-0 work items in parallel. They are **not** equal cost or equal decisiveness.
Order them:

1. **Determinism spike first (~days).** Build the one exact tri-tri (§2), the native golden, the wasm
   `__exactSpikeDigest`, and the Playwright gate. This is the cheapest and most decisive — it can kill
   the entire program for a few days of work.
2. **Fallback-rate benchmark (~days)** on the dipping-vein fixture (§3). Only meaningful if (1) passes.
3. **CGAL golden corpus (~1 week, heaviest setup)** last (§4). Only build it once (1) and (2) pass.

### Gate criteria
- **GO** iff: Layer-1 digest is **bit-identical native↔wasm** through the real wasm artifact (§2.3),
  **and** the rational-fallback rate is **< 1%** on the dipping-vein + degenerate fixtures (§3).
- **NO-GO / reconsider** if either fails: rate too high → too slow (revisit `num-rational`, then as a
  last resort the CGAL-server or boolmesh-with-conditioning alternatives from spec §"Decision
  context"); Layer-1 divergence even with exact construction → the determinism premise is broken at the
  root, stop.
- **Conditional / Track-B-coupled:** if Layer 1 passes but the **Layer-2** perturbation test shows the
  corefiner cascades on 1-ULP input divergence, that is **not** a corefiner failure — it says exact
  corefining must ship **together with** deterministic octree/field inputs (§7). Proceed, but treat
  Track B as a hard co-requisite of Stage 1, not a follow-on.

### Risk call (where I expect this to land)
The corefiner's *own* arithmetic is the low-risk part — Rust strict-IEEE f64 over `+ − × ÷` is already
cross-target stable, so Layer 1 will very likely pass and the exact path makes it certain. **The real
risk is Layer 2: divergent inputs from the field/octree boundary.** That is the docs/09 cascade root,
and exact construction does not address it. So the honest framing of the go/no-go: the spike will
probably green-light the *corefiner*, and in doing so will **re-point the hard problem at Track B** —
making cross-target-deterministic field eval at feature depth the actual gating risk for the program,
not the corefiner math. Plan for that outcome.

---

## 6. Feature-flag-parallel architecture (carried constraint #3)

Never edit `mesh_clip::clip` in place. The exact path is additive and default-off:

- **Cargo feature** `exact_arr` (default off) in `crates/geofield/Cargo.toml`, gating optional deps
  `geometry-predicates` + `dashu` and the new `exact_arr` module. Idiomatic here — the crate already
  feature-gates `wasm`, `cli`, `native-parallel`.
- **New module** `crates/geofield/src/exact_arr/` (the kernel, tri-tri, the eventual corefiner) — a
  sibling of `mesh_clip.rs`, sharing its `ClipMesh` / `ClipResult` / `shared` output shape so callers
  in `stack.rs` need no change to *try* it.
- **Wasm entry points** gated `#[cfg(all(feature = "wasm", feature = "exact_arr"))]`: the
  `__exactSpikeDigest` gate (Stage 0), later a `mesh_*_exact_corefine_*` alongside the existing
  shared/clip meshers.
- **Worker/UI selection** rides the existing toggle machinery: the worker already switches seamers on a
  `seamClip` boolean ([`web3/src/worker.ts` ~204-206](../web3/src/worker.ts)); add a third option via a
  dev hook (`window.__COREFINER_MODE = "exact" | "clip" | "snap"`, parallel to the existing
  `__WELD_T_JUNCTIONS`). No production default changes until the path is proven.

This keeps the current shared-tree path (`mesh_cross_cut_solids_shared_*`, what commit `376142c`
touched) as the shipping default throughout the spike.

---

## 7. Track B (parallel necessity) — adaptive octree resolution at the feature

Exact corefining is moot if the octree never resolves the 2 ft feature, **and** (per §2.2 Layer 2)
the octree must resolve it *cross-target-deterministically* or the exact corefiner inherits divergent
inputs. So Track B is doubly necessary: for fidelity *and* for the determinism premise.

- **Current ceiling:** default `max_depth = 6` ([octree.rs:1028](../crates/geofield/src/octree.rs#L1028))
  → cell = `model/2⁶` = model/64 ≈ **1.6e-2** of the model. Target is **2e-5** (2 ft / 20 mi) →
  depth ≈ **16** (~10 levels deeper). Uniform refinement at that depth is impossible (8¹⁶ cells); it
  **must** be adaptively targeted at the feature.
- **What targets refinement today:** nothing feature-size-aware. Build refines only on surface
  crossing or `qef_threshold` ([octree.rs:1696](../crates/geofield/src/octree.rs#L1696)); the
  reactive passes `refine_double_crossed` ([1831](../crates/geofield/src/octree.rs#L1831)) and
  `refine_shared_contacts` ([1872](../crates/geofield/src/octree.rs#L1872)) need *two surfaces to both
  straddle a cell* to fire. There is no "refine until cell ≤ feature size" criterion.
- **Smallest change set:** add `target_feature_size: Option<f64>` to `OctreeConfig`
  ([octree.rs:1008-1034](../crates/geofield/src/octree.rs#L1008)); in the build/refine loops, where a
  region of interest is known, force subdivision until `cell_size ≤ target_feature_size / k` (k≈2–4),
  capped at a raised `max_depth`. ~50 lines, localized to `octree.rs`; the corefiner is resolution-
  agnostic and needs no change (it re-triangulates whatever triangle density it receives).
- **Cross-target determinism risk of going deeper** (the Layer-2 coupling): at depth ~16 the QEF
  vertex solve (matrix inversion) and `edge_crossing` linear interpolation
  ([octree.rs:212-225](../crates/geofield/src/octree.rs#L212)) run near the noise floor where native↔
  wasm field-eval ULP differences flip signs — exactly the `CONTACT_GRAZE_EPS` graze class already
  hardened at [octree.rs:1309/1320-1333](../crates/geofield/src/octree.rs#L1309). Deeper refinement
  widens that exposure. The determinism work hardened so far is necessary but was tuned for depth ~6;
  depth ~16 needs the field-eval boundary itself to be cross-target deterministic (or quantized), which
  is the genuinely hard part of Track B and the true gating risk surfaced by §5.

**Track B is scoped here, not built here.** It runs in parallel with the Stage-0 spike; its own go/no-go
is whether cross-target-deterministic field eval at depth ~16 is achievable, which the Layer-2 spike
result (§2.2) informs directly.

---

## Effort & sequencing summary

| Item | Where | Effort | On go/no-go path? |
|---|---|---|---|
| Crate add + `exact_arr` feature + module skeleton | `Cargo.toml`, `src/exact_arr/` | ~1 day | setup |
| Exact tri-tri + lazy scalar (expansion→dashu) | `src/exact_arr/` | ~days | **yes** |
| Native golden + wasm `__exactSpikeDigest` + Playwright gate | `exact_arr`, `web3/e2e/` | ~1 day | **yes (Layer 1)** |
| 1-ULP perturbation (Layer 2) assertion | `web3/e2e/` | ~½ day | **yes (Layer 2)** |
| Fallback-rate benchmark (instrumented test) | `exact_arr` tests | ~days | **yes** |
| CGAL golden corpus harness | `tools/cgal-oracle/`, `tests/golden/` | ~1 week | no (Stage 1 oracle) |
| Track B: `target_feature_size` adaptive refinement | `octree.rs` | parallel track | co-requisite |

**Recommendation: GO on the Stage-0 spike**, sequenced determinism-first so the cheapest test can kill
the program before any corpus/benchmark investment — but go in expecting the spike to validate the
corefiner math and re-point the hard risk at Track B's cross-target-deterministic deep field eval.
Do not touch `mesh_cross_cut_solids_*` until the §5 gate closes.
