# KERNEL_MATH.md — covariance kernels & cokriging block layout

Reference implementation: `gempy_engine`, branch `main`
(`github.com/gempy-project/gempy_engine`). All formulas below are transcribed
from source and cross-checked by hand. Scope: **single scalar field,
non-faulted, single series** (per `SCOPE_potential_field_rust.md`). Faults,
multi-series stacking, octrees, and dual contouring are out.

Everything is `f64`. The cubic kernel consumes **raw distance `r`, not `r²`**
(`consume_sq_distance=False`).

**Kernels.** The blocks of §3 consume a kernel only through the triple
`C`, `C'/r`, `C''` plus gempy's distance convention for it. geofield implements
exactly gempy's `AvailableKernelFunctions` that it validates against (Rust:
`geofield_kernel::KernelFunction`; ABI code in parentheses):

| gempy name | §  | argument | geofield variant |
|---|---|---|---|
| `cubic` (0, default) | §1 | floored `r` | `Cubic` |
| `matern_5_2` (1) | §1b | floored `r` | `Matern52` |
| `exponential` (2) — a **Gaussian** | §1c | **raw `r²`** | `Exponential` |

A kernel with no gempy reference does not belong in this crate (Constitution I).

---

## 0. Notation & constants

- `a` — the kernel **range** (a free scalar input; see §5). In code: `a = range`.
- `c_o` — covariance scaling constant (a free scalar input; see §5).
- `r = ‖x − x'‖` — Euclidean distance between two locations.
- `i_res = 4.0`, `gi_res = 2.0` — resolution weights on the SP and cross blocks
  (`KernelOptions.i_res`, `.gi_res`). These are *not* physical; they are fixed
  multipliers baked into the assembled blocks.
- `uni_degree = 1` — universal drift polynomial degree (linear).
- `number_dimensions = 3`.
- Numerical floors (must be matched bit-for-bit):
  - kernel-argument distance: `r = sqrt(r² + 1e-10)` (the cubic functions are
    evaluated at this floored `r`).
  - gradient-block denominator: `hu*hv / (r² + 1e-5)` uses the **squared**
    distance with a **`1e-5`** floor — a *different* epsilon from the kernel arg.
  - `global_nugget = 1e-5` (added on block diagonals as described in §3).

> **Trap.** Two distinct epsilons. The cubic functions see `sqrt(r²+1e-10)`.
> The `hu*hv/(...)` divisor uses `r² + 1e-5`. Do not unify them.

---

## 1. The cubic function triple (CONFIRMED verbatim from source)

These are the *symbolic* (non-factorized) forms bound into
`AvailableKernelFunctions.cubic`. The factorized `_numpy` variants in the file
are **not** wired into the cubic enum member; we match the symbolic ones.

### 1.1 Base covariance `C(r)` — `cubic_function(r, a)`

```
C(r) = 1 − 7 (r/a)²  +  (35/4)(r³/a³)  −  (7/2)(r⁵/a⁵)  +  (3/4)(r⁷/a⁷)
```

Properties (sanity): `C(0) = 1`. For `r ≥ a` the cubic is meant to be clamped
to 0 in the compactly-supported sense, but gempy's symbolic form does **not**
clamp — it evaluates the polynomial everywhere. We reproduce the polynomial as
written (no clamping) to match gempy bit-for-bit; clamping would be a deviation.

> **⚠ CORRECTNESS LANDMINE — no-clamp has an implicit precondition.** The cubic
> is *compactly supported*: mathematically zero for `r ≥ a`. GemPy's symbolic
> polynomial does not clamp, so for `r > a` it goes **negative and then
> positive** — nonsense as a covariance. It only ever produces sane geology
> because the rescaling (§5.1) keeps **every pairwise normalized distance well
> inside `a`**. With the gempy-`main` normalization (half-extent ≈ 0.25, §5.1)
> and `a = range = 1.7`, the max pairwise normalized distance is ≈ 0.5 ≪ 1.7 —
> a comfortable margin. This margin is a *precondition*, not a guarantee.
> **Action (assemble.rs):** add a debug assertion that warns if any pairwise
> normalized distance exceeds `a`. If a future model's data doesn't fit the
> normalized box, you get silent garbage that looks exactly like a kernel bug.
> Matching gempy means reproducing both the no-clamp behavior *and* its implicit
> precondition — and the assertion makes the precondition loud.

### 1.2 First derivative divided by r — `cubic_function_p_div_r(r, a)`

This is `C'(r) / r`, pre-divided (the factored form that keeps the
gradient-block assembly free of an `r=0` divide):

```
C'(r)/r = −14/a²  +  (105/4)(r/a³)  −  (35/2)(r³/a⁵)  +  (21/4)(r⁵/a⁷)
```

Derivation check (hand): from §1.1,
`C'(r) = −14 r/a² + (105/4) r²/a³ − (35/2) r⁴/a⁵ + (21/4) r⁶/a⁷`,
divide by `r` ⇒ matches. ✓

### 1.3 Second derivative `C''(r)` — `cubic_function_a(r, a)`

Source form:

```
C_a(r) = 7 (9 r⁵ − 20 a² r³ + 15 a⁴ r − 4 a⁵) / (2 a⁷)
```

Expanded:

```
C''(r) = −14/a²  +  (105/2)(r/a³)  −  70 (r³/a⁵)  +  (63/2)(r⁵/a⁷)
```

Derivation check (hand): differentiate `C'(r)` from §1.2's numerator ⇒
`C''(r) = −14/a² + (105/2) r/a³ − 70 r³/a⁵ + (63/2) r⁵/a⁷`, and the source
factored form expands to exactly this. ✓ **`cubic_function_a` IS C''(r).**

> The three are mutually consistent: `C`, `C'/r`, `C''`. The finite-difference
> unit tests in §4 of the scope doc must confirm this numerically before any
> assembly code is trusted.

> **Verified — `cubic_function_a` is the plain `C''(r)`, NOT pre-folded.**
> A real concern with this code lineage: some variants define the radial
> helpers as the *already-combined* coefficients of the Chilès–Delfiner
> gradient-covariance tensor (i.e. the body would already be `C'' − C'/r`),
> in which case §3.1's `(C'' − C'/r)` would double-subtract. Checked against the
> literal source body:
> ```
> return 7 * (9*r**5 - 20*a**2*r**3 + 15*a**4*r - 4*a**5) / (2*a**7)
> ```
> The standalone `−4·a⁵` numerator term yields constant `−14/a²`. A pre-folded
> `C'' − C'/r` would have **zero** constant term (the two `−14/a²` cancel).
> Since the constant is present, the body is the **plain second derivative**.
> Therefore §3.1 (which subtracts `C'/r` itself) is correct as written and does
> NOT double-subtract — the constant-term cancellation happens *inside* §3.1,
> which is exactly the decomposition into separation-vector and perpendicular
> radial coefficients. (`cubic_function_p_div_r` likewise carries its own
> `−14/a²` and is the plain `C'/r`.)

---

## 1b. The Matérn ν=5/2 triple — `matern_5_2` (CONFIRMED verbatim from source)

Source: `gempy_engine/core/data/kernel_classes/kernel_functions.py`
(`matern_function_5_2_numpy`, `…_p_div_r_numpy`, `…_a_numpy`), gempy_engine
`main` @ `098b2b1` (2026-10-17). `consume_sq_distance=False`: the argument is the
**floored `r`** (`sqrt(r² + 1e-10)`), exactly as for the cubic. With `x = r/a`:

### 1b.1 `C(r)`
```
C(r) = (1 + √5·x + (5/3)·x²) · exp(−√5·x)          C(0) = 1
```
### 1b.2 `C'(r)/r`
```
C'(r)/r = −5 · exp(−√5·x) · (a + √5·r) / (3a³)      → −5/(3a²) at r = 0
```
### 1b.3 `C''(r)` (the plain second derivative, like §1.3)
```
C''(r) = −5 · exp(−√5·x) · (a² + √5·a·r − 5r²) / (3a⁴)   → −5/(3a²) at r = 0
```

Both derivatives are finite at `r = 0`, which is what lets this kernel carry
orientation (gradient) data — a kernel with a cusp at the origin (exponential
`e^{−r/a}`, spherical, linear) cannot.

> **Trap — truncated constants.** gempy writes `√5` as the literal
> `2.2360679775` and `5/3` (in `C` only) as `1.6666666667`; the derivatives use
> the exact `5/3` algebra. `C` and its derivatives therefore disagree at ~1e-11
> relative. The Rust transcription keeps the literals verbatim for parity; do
> not "fix" one side without the other.

No compact support: the kernel decays exponentially, so the §1.1 in-range
precondition (and its debug assertion) applies to the cubic only.

---

## 1c. gempy's `exponential` triple — a **Gaussian** (CONFIRMED verbatim from source)

Source: the same file (`exp_function_numpy`, `exp_function_p_div_r_numpy`,
`exp_function_a_numpy`), same commit. Despite the name this is the
squared-exponential (Gaussian) covariance, and it is gempy's only
`consume_sq_distance=True` kernel.

### 1c.1–1c.3 The triple, as functions of `s = r²`
```
C       = exp(−s / (2a²))                       C(0) = 1
C'(r)/r = −(1/a²) · exp(−s / (2a²))             → −1/a² at r = 0
C''(r)  = (s/a⁴ − 1/a²) · exp(−s / (2a²))       → −1/a² at r = 0
```
(`C'/r` and `C''` are the true derivatives with respect to `r`, merely written in
terms of `s`.)

> **Trap — the argument is `r²`, un-floored.** With `square_distance=True`,
> `_compute_distances_generic` skips the `sqrt(r² + 1e-10)` step entirely: all
> three functions receive the raw squared distance. Passing `r` is silently wrong
> (it is still a valid-looking positive number). `KernelFunction::arg` is the one
> place this switch lives.

### 1c.4 ⚠ LANDMINE — the CG_CG divisor becomes `r⁴ + 1e-5`

§3.1's divisor is written in source as `hu·hv / (r_ref_ref**2 + 1e-5)`, where
`r_ref_ref` is **the kernel argument**. For the distance kernels that is
`r² + 1e-5` (§3.1, unchanged). For `exponential` the argument is already `r²`,
so gempy's divisor is **`r⁴ + 1e-5`** — not the `r²` that the Hessian
decomposition `(C'' − C'/r)·d dᵀ/r² + (C'/r)·I` calls for.

This is a gempy quirk, reproduced **deliberately** for parity
(`KernelFunction::cg_cg_divisor`). Consequences, stated so nobody "fixes" it by
accident:
- It only touches the CG_CG coupling between orientations at **different**
  locations (`hu·hv = 0` on the diagonal), so single-orientation models are
  unaffected.
- The assembled covariance is then not the exact derivative-covariance of the
  Gaussian, so with **interacting** orientations the fitted field's gradient
  does not reproduce the poles. Measured (`gradient_consistency.rs`,
  `pole_reproduction_with_interacting_orientations_per_kernel`, 3 orientations):
  worst pole-direction misfit **7.6°** (others 2.2°, 2.7°) for `exponential`,
  versus < 0.01° for `cubic` and `matern_5_2`. Fits remain well-posed (solve
  residual ~1e-15); orientations are honored only approximately. The UI says so
  in the kernel picker's tooltip.
- **The validation gate distinguishes the two**: with the `r⁴` divisor the
  multi-orientation block diff (`dump_kernel.py --kernel exponential --model
  multi`) matches gempy to `max|Δ| = 4.4e-15`; with an `r²` divisor the same gate
  **fails** at `max|Δ| = 0.77` (CG_CG ratio 0.17–1.32). Changing this is a
  deliberate deviation from the reference and must be recorded here.

### 1d. Validation record (2026-10-20, gempy_engine `098b2b1`)

Assembly gate (`validation/dump_kernel.py`, full-`K` max|Δ| vs gempy, gate 1e-4):

| kernel | `--model tiny` (1 orientation) | `--model multi` (3 orientations, 2 interfaces) |
|---|---|---|
| `cubic` | 0 | 8.9e-9 |
| `matern_5_2` | 1.2e-10 | 4.6e-10 |
| `exponential` | 0 | 4.4e-15 |

The cubic's `8.9e-9` on `multi` is the known divisor detail: geofield uses
`r² + 1e-5`, gempy `(sqrt(r²+1e-10))² + 1e-5` — they differ by the `1e-10`
floor (+ rounding). Matérn's residual is `exp` ULPs.

Field gate (`validation/run_model1_validation.py --kernel …`, model1, 125 000
grid points): every kernel shows the same profile as the cubic baseline —
affine `rust ≈ 1.000000·gempy − 2.0e-6` (the known inert offset), normalized
RMS residual 2.8e-16 (cubic) / 7.8e-16 (matern_5_2) / 7.2e-16 (exponential),
lithology voxel agreement **100.0000%**. (model1 is flat-lying strata, so this
gate discriminates between kernels only weakly; the `multi` block diff above is
the sharper oracle.)

---

## 2. Geometric quantities per data pair

Orientations carry a unit pole (gradient direction) and a location. Surface
points carry a location and belong to an interface with a chosen **reference**
point; the constraint is on the **increment** rest − ref.

For a pair of locations with coordinate difference `d = (dx, dy, dz)` and the
two associated direction selectors, gempy forms (per `_compute_distances_generic`):

- `hu = Σ d · (hu_sel_i * hu_sel_j)` — projection along the *i* pole components.
- `hv = − Σ d · (hv_sel_i * hv_sel_j)` — projection along the *j* pole components
  (note the **leading minus**).
- `perp_matrix = Σ (hu_sel_i * hv_sel_j)` (as int8) — the perpendicularity /
  same-component indicator between the two gradient functionals.
- For the cross (gradient↔surface-point) block:
  - `huv_ref  = hu_ref  − hv_ref`,
  - `huv_rest = hu_rest − hv_rest`,
  where `hu_ref/hv_ref` use the ref surface-point selector against the
  orientation selector, and `hu_rest/hv_rest` the rest selector.

Squared distances (raw, before any floor):
```
r_ref_ref²   = Σ (dip_ref_i  − dip_ref_j )²      # ori↔ori and ref↔ref geometry
r_rest_rest² = Σ (diprest_i  − diprest_j )²
r_ref_rest²  = Σ (dip_ref_i  − diprest_j )²
r_rest_ref²  = Σ (diprest_i  − dip_ref_j )²
```
Kernel arg distance (cubic, `square_distance=False`): `r = sqrt(r² + 1e-10)`.

---

## 3. The three covariance blocks (verbatim assembly)

Top-level assembly (`_covariance_assembler.py`):

```
cov = c_o * (cov_grad + cov_sp + cov_grad_sp) + uni_drift
```

`c_o` multiplies the **sum of the three covariance blocks**; the universal
drift border is added **outside** `c_o`. (`+ faults_drift` is omitted — out of
scope.)

Let, evaluated at the appropriate floored `r`:
- `k_p_ref  = C'(r_ref_ref )/r_ref_ref`   (= `cubic_function_p_div_r`)
- `k_p_rest = C'(r_rest_rest)/r_rest_rest`
- `k_a      = C''(r_ref_ref)`              (= `cubic_function_a`)
- `k_ref_ref, k_rest_rest, k_ref_rest, k_rest_ref = C(·)` (= `cubic_function`)

### 3.1 Gradient–gradient block `CG_CG` — `_get_cov_grad`

```
cov_grad =  (hu * hv) / (r_ref_ref² + 1e-5) * ( − k_p_ref + k_a )
            − k_p_ref * perp_matrix
```

i.e. `(hu·hv)/(r²+1e-5) · (C'' − C'/r)  −  (C'/r)·perp`.

Sign discipline (the `(- k_p_ref + k_a)` ordering = `k_a − k_p_ref`) and the
`perp_matrix` subtraction must be exact. Gradient nugget enters the diagonal as
`eye * perp_matrix * grad_nugget`.

### 3.2 Gradient–surface-point cross block `CG_SP` — `_get_cross_cov_grad_sp`

```
cov_grad_sp = gi_res * ( huv_ref * k_p_ref  −  huv_rest * k_p_rest )
```

with `gi_res = 2.0`. Sign: `+huv_ref·k_p_ref − huv_rest·k_p_rest`.

### 3.3 Surface-point–surface-point block `SP_SP` — `_get_cov_surface_points`

The four-term increment (ref/rest second difference):

```
cov_sp = i_res * ( k_rest_rest − k_rest_ref − k_ref_rest + k_ref_ref )
```

with `i_res = 4.0`. Sign structure:
`+C(rest,rest) − C(rest,ref) − C(ref,rest) + C(ref,ref)`. SP nugget added on the
diagonal of the SP sub-block only.

> **Trap — reference point.** Surface-point rows are *increments* against a
> per-interface reference. The whole SP block is the second difference above;
> getting ref/rest wiring wrong yields a plausible-but-mis-leveled field.

---

## 4. Universal drift border (uni_degree = 1)

`n_uni_eq = number_dimensions = 3` for degree 1 (linear drift basis `{x, y, z}`).
The drift contributes the bordering rows/cols of `K` (the `0` block on the
diagonal that makes `K` symmetric **indefinite** → Bunch-Kaufman, not Cholesky).

### 4.1 Surface-point drift `U_SP` — `assembly_dips_points_coords`

The degree-1 drift coordinate vector for a surface point is **its own
coordinates** `(x, y, z)` (the linear basis), with zeros over the orientation
block and identity over the 3 universal-equation rows. Entered into the
covariance via `_get_universal_sp_terms`; for pure degree 1 it reduces to:

```
usp = − selector * gi_res * ( usp_rest − usp_ref )
```

where `usp_* = Σ (drift_basis_i · drift_basis_j)` over ref/rest points and
`selector` places the term into the border. (The `i_res`-weighted `d2` terms
vanish at degree 1.)

### 4.2 Gradient drift `U_G` — `assembly_dips_ug_coords`

`∂/∂x_i` of the linear drift `{x, y, z}` is the unit vector: for orientation
rows belonging to component `i`, the degree-1 drift entry is the constant `1`
(and `1` on the diagonal of the 3 universal-equation rows). Entered via
`_get_universal_gradient_terms`; for degree 1:

```
ug_total = selector * ug
```

with `ug = Σ (dips_ug_i · dips_ug_j)`.

> Degree-2 terms (`dips_b`, `dips_c`, the `d2` SP terms) are populated **only**
> when `uni_degree == 2`. We implement degree 1 and leave a seam.

---

## 5. `c_o`, `range`, and rescaling — DIVERGENCE FROM SCOPE DOC

The scope doc hypothesized `c_o = range²/(14·3)` and a data-derived `range`.
**This is not in current gempy_engine `main`.**

- `range` (= kernel `a`) and `c_o` are **free scalar inputs** on `KernelOptions`
  with no computed default. Defaults seen in source: `range=1.7, c_o=10.0`
  (octree default); the examples generator uses `range=0.7, c_o=3`. The legacy
  v2 `range²/14/3` / `rescaling_factor` formulas are **absent** from both
  `gempy_engine` and `gempy` `main` (grepped — gone).
- What makes a fixed `range=1.7` meaningful across datasets is the **rescaling**
  below, not a data-derived range.

### 5.1 Coordinate rescaling — `Transform.from_input_points`

Built from the concatenation of all surface-point and orientation xyz:

```
max_coord   = max over points (per axis)
min_coord   = min over points (per axis)
range_coord = 2 * (max_coord − min_coord)         # per axis; 0 → 1e-10
scale       = 1 / range_coord                      # per axis, then isotropized
center      = (max_coord + min_coord) / 2
translation = − center
```

`anisotropic_limit = [1,1,1]` forces all three axes to share the **smallest**
scale (isotropic cube). Application order is **SRT** (`S @ R @ T`), i.e.
translate-then-scale, rotation = 0:

```
x_norm = scale * (x − center)
```

**Extent (resolved against source — watch the factor of 2):** the extreme point
`x = max` maps to `scale·(max − center) = ((max−min)/2)/(2·(max−min)) = 1/4`.
So the data lands in **`[−0.25, +0.25]` per axis: full-extent ≈ 0.5,
half-extent ≈ 0.25.** (An earlier draft said "half-extent ≈ 0.5" — wrong by 2×;
the `2*` in `range_coord` is the source of the halving and is confirmed in
source.) The `# to compare with legacy add 0.5` comment recenters to
`[0.25, 0.75]` (legacy's box) — it shifts but does **not** rescale, so the
effective normalized half-extent in gempy `main` is **0.25**. This is what gives
the `range = 1.7` kernel its safety margin (§1.1 landmine).
Gradients transform by the inverse-transpose 3×3 (magnitude-preserving when
scale is isotropic — which it is by default).

> **Trap — rescaling sets the effective range.** All kernel math runs in the
> normalized box. Match center/scale exactly or the field shape drifts.

---

## 6. Solve & dual evaluation (framing for the Rust side)

`K` is symmetric **indefinite** (the drift `0` border kills positive-
definiteness). Factor once with **Bunch–Kaufman / LDLᵀ** (faer), solve for the
dual weight vector `w = [w_grad, w_sp, w_drift]` against RHS
`b = [b_grad, 0, 0]` where `b_grad` holds the (rescaled) orientation pole
components.

Evaluation at a query point reuses the same `w`:
- gradient-data terms via `C'/r` (and `C''` for the analytic field gradient),
- surface-point terms via the `C(·)`-increment against each interface ref,
- plus the degree-1 drift polynomial.

A Cholesky that "succeeds" means the drift border was assembled wrong.

> **⚠ VALIDATION ORDER — diff the blocks before the field.** The sign
> discipline in this doc (the `hv` leading minus in §2, the `(−k_p_ref + k_a)`
> ordering and `perp` subtraction in §3.1, the four-term SP increment in §3.3)
> is correct **as transcribed** but **untestable except against a gempy dump**.
> A sign error in any of these still yields a *solvable* indefinite system and a
> *smooth-looking* field — it is simply the wrong field. A field-level test
> (§7 plan, tol 1e-6) will go red but **will not tell you which block flipped**.
> Therefore the validation harness MUST, on the smallest model (2 surface points
> on one interface + 1 orientation), dump gempy's assembled `K` and compare it
> **block-by-block** (CG_CG, CG_SP, SP_SP, drift border) *before* comparing
> solved weights or fields. Only the block-level diff localizes a sign flip.
> This is step 3 of the scope-doc implementation order and is non-negotiable —
> do not skip ahead to weights/field comparison until the blocks match.

---

## 7. Open items to confirm against a live model dump

0. **Anisotropic rescaling (found 2026-10-20, not yet handled).** Current gempy
   `main` rescales **per axis** when the data's x/y/z spans differ (observed
   scale `[0.0005, 0.00132, 0.0005]` on an 1000×350×380 point cloud) and
   transforms the orientation gradients to match (`dip_gradients` ≠ the raw
   poles). §5.1's isotropic box — which geofield's `Transform` implements — is
   only what gempy does when the spans are equal. model1 and the `tiny`/`multi`
   gate models have equal spans and so match; a non-cubic data cloud would not.
   This is independent of the kernel choice. Until resolved, the gate models
   keep equal spans on purpose (see `dump_kernel.py --model multi`).

1. Exact placement/ordering of the `selector` border rows vs. our `K` index
   layout (gempy interleaves orientation components; we must match the RHS
   ordering when comparing weights — the field itself is order-invariant).
2. Working assumption: reference model uses the **octree defaults**
   `range = 1.7`, `c_o = 10` (the Rust `KernelOptions` default). The
   `validation/` harness still dumps the *live* `kernel_options` into the
   fixture, and the Rust tests read those values — so if a `compute_model`
   path overrides the defaults, the comparison stays honest.
3. Confirm orientation pole sign convention (younging direction) on the
   reference model so layers don't invert.
4. **RHS `b_grad` scaling/units (genuine open item).** This doc says `b_grad`
   holds the "(rescaled) orientation pole components" but has NOT pinned, from
   source: (a) whether gempy normalizes poles to unit length, and at what stage
   (before vs after the gradient transform of §5.1); (b) whether `c_o`, a
   gradient-variance term, or `gi_res` multiplies the RHS. Since `c_o` multiplies
   the covariance blocks (§3 top) but the drift border and (per current reading)
   the RHS do not, **the matrix and RHS must be in consistent units or the
   weights come out globally mis-scaled.** Note the failure signature: a global
   RHS scale leaves the *field shape* intact (so a normalized-field comparison
   passes) but corrupts the **recovered isovalues** and the **gradient-magnitude
   reproduction at orientation locations** (scope-doc step-5 check). Confirm by
   dumping gempy's RHS vector alongside the matrix on the tiny model and
   comparing `b` directly — same as the block-by-block insistence above.
