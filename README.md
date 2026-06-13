# geofield-rs

A WASM-targetable Rust implementation of the **universal-cokriging scalar
(potential) field** used for implicit geological modeling — the core interpolator
behind [GemPy](https://github.com/gempy-project/gempy_engine), reimplemented from
scratch in `f64` and validated **bit-for-bit against gempy's assembled system**.

Given surface points and orientations, it assembles the dual cokriging system,
solves it once, and exposes the scalar field, its analytic gradient, and a
lithology assignment.

## Scope

This crate implements the **single scalar field, non-faulted, single-series**
case. The following are deliberately **out of scope** (clean seams are left where
they would attach, but they are not implemented):

- faults
- multi-series stacking
- octrees / adaptive refinement
- dual contouring / mesh extraction
- degree-2 universal drift (degree-1 only; a seam is left)

The math is transcribed and cross-checked against `gempy_engine` `main` and
recorded in **[`KERNEL_MATH.md`](KERNEL_MATH.md)**, which is the source of truth:
where code and that document disagree on a formula, sign, constant, or epsilon,
the document wins.

## What works

- **Cubic covariance kernel** `C(r)`, `C'(r)/r`, `C''(r)` (finite-difference
  verified for mutual consistency).
- **Coordinate rescaling** (`Transform`) matching gempy's `from_input_points`
  center/scale (confirmed: gempy's scale `0.000625` is reproduced exactly on the
  reference model).
- **System assembly** of the symmetric-indefinite cokriging matrix `K` and RHS
  `b`: the three covariance blocks (CG_CG, CG_SP, SP_SP), the degree-1 universal
  drift border (U_G, U_SP), and nuggets — all matching gempy's dump
  **bit-for-bit** (`max|Δ| = 0`, see [Validation](#validation)).
- **Solve** via faer Bunch–Kaufman / LDLᵀ (the system is indefinite; Cholesky is
  deliberately *not* used).
- **Evaluation**: scalar field `Z(x)`, analytic gradient `∇Z(x)`, and
  per-interface isovalue recovery.
- **Lithology**: assignment of query points to units between consecutive
  recovered isovalues.
- **WASM bindings** + a verified native-vs-WASM parity check.

## Public API

The entry point is `Model`:

```rust
use geofield_rs::Model;
use geofield_rs::assemble::KernelOptions;
use geofield_rs::data::{Series, SurfacePoint, Orientation};

// One interface (id 0) dipping in +x, with two matching orientation poles.
let surface_points = vec![
    SurfacePoint::new([0.0, 0.0, 1.00], 0),
    SurfacePoint::new([1.0, 0.0, 1.30], 0),
    SurfacePoint::new([0.0, 1.0, 1.05], 0),
    SurfacePoint::new([1.0, 1.0, 1.35], 0),
];
let orientations = vec![
    Orientation::new([0.25, 0.5, 1.10], [-0.3, 0.0, 1.0]),
    Orientation::new([0.75, 0.5, 1.25], [-0.3, 0.0, 1.0]),
];

let series = Series::new(surface_points, orientations);
let model  = Model::fit(&series, KernelOptions::default());

let z    = model.scalar_field([0.5, 0.5, 1.2]);          // f64
let grad = model.scalar_field_gradient([0.5, 0.5, 1.2]); // [f64; 3]
let unit = model.lithology_unit([0.5, 0.5, 1.2]);        // usize

// Fit diagnostics
println!("solve residual  = {:.3e}", model.residual);
println!("isovalue spread = {:.3e}", model.isovalue_spread);
```

Inputs are in **world coordinates**; the model rescales internally and returns
the gradient in world coordinates. Orientations carry a **pole** (gradient
direction) vector; the validation CLI converts gempy's azimuth/dip/polarity to a
pole.

### `KernelOptions`

Read these from a live source (e.g. a gempy fixture) rather than hardcoding;
`Default` mirrors gempy's octree defaults.

| field | meaning | default |
|-------|---------|---------|
| `range` | kernel range `a` | `1.7` |
| `c_o` | covariance scaling constant | `10.0` |
| `i_res` | SP-block resolution weight | `4.0` |
| `gi_res` | cross-block resolution weight | `2.0` |
| `uni_degree` | universal drift degree (only `1` implemented) | `1` |
| `grad_nugget` | nugget on the gradient sub-block diagonal | `1e-5` |
| `sp_nugget` | nugget on the surface-point sub-block diagonal | `1e-5` |

## Crate layout

```
src/
├── lib.rs          public API: Model::{fit, scalar_field, scalar_field_gradient, lithology_unit}
├── kernel/cubic.rs C(r), C'(r)/r, C''(r)            (KERNEL_MATH.md §1)
├── data.rs         SurfacePoint, Orientation, Series; Transform (rescaling, §5)
├── geometry.rs     hu/hv/perp, distances, the two distinct epsilons (§2)
├── assemble.rs     K and RHS block construction + drift border (§3–§4)
├── solve.rs        faer Bunch–Kaufman factor + solve (§6)
├── evaluate.rs     scalar_field + analytic gradient (§6)
├── litho.rs        isovalue recovery + interval assignment
├── wasm.rs         #[cfg(feature="wasm")] typed-array bindings
└── bin/geofield_cli.rs  validation driver (feature = "cli")
```

## Building

```bash
cargo build                 # the library (lean: faer std + linalg only)
cargo test                  # 36 tests
cargo run --example single_layer
```

### WASM

```bash
wasm-pack build --target web    -- --features wasm   # browser
wasm-pack build --target nodejs -- --features wasm   # node
```

`wasm-bindgen` is an optional dependency gated on the `wasm` feature; faer is
trimmed (no `rand`/`getrandom`, no `rayon`) so `wasm32-unknown-unknown` builds.
The native build can opt into parallel BLAS with `--features native-parallel`.

The browser parity harness lives in [`examples/web/`](examples/web/): build the
web package, serve the repo root, and open `examples/web/index.html` to compare
the WASM field against a native reference. Native↔WASM agreement was verified to
`1.2e-15` (f64 round-off).

## Tests

`cargo test` runs 36 tests:

| file | covers |
|------|--------|
| `kernel_derivatives.rs` | cubic triple FD-consistency (`C(0)=1`, `C'/r`, `C''`) |
| `block_assembly.rs` | per-block `K`/`b` diff vs a gempy dump (skips cleanly without the fixture) |
| `solve_weights.rs` | solved dual weights vs gempy (order-aware; skips without fixture) |
| `gradient_consistency.rs` | `∇Z` vs finite differences; pole reproduction at orientations |
| `increment_consistency.rs` | within-interface field spread guard (see note below) |
| `reference_model.rs` | end-to-end pipeline + field/gradient vs gempy fixture |

> **Note on `increment_consistency.rs`.** This probe checks that points on one
> interface recover near-equal field values. It is a *necessary* guard, **not** a
> correctness oracle: it is invariant to global field scale/level, so it can be
> satisfied by a wrong-but-self-consistent field (this actually happened — two
> mutually-compensating sign/scale bugs passed it). The authorities for
> correctness are the validation harnesses below.

## Validation

Correctness is established against gempy itself, not internal self-consistency.
The Python harness lives in [`validation/`](validation/) and is **not** part of
the crate build; it uses a `uv`-managed environment (gempy needs Python ≥3.11):

```bash
cd validation
uv sync
```

Two gates:

- **`dump_kernel.py`** — the *assembly* gate. Monkeypatches gempy's solver to
  snapshot its assembled `K`/`b`, then diffs against `geofield-cli --dump-kernel`
  block-by-block. **Current status: every block matches bit-for-bit**
  (CG_CG, CG_SP, SP_SP, U_G, U_SP, b all ratio `1.0000`, `max|Δ| = 0`).

- **`run_model1_validation.py`** — the *field* gate. Builds gempy's
  `jan_models/model1` (horizontal stratigraphy), evaluates both engines on
  gempy's exact grid, and diffs the field and lithology. **Current status:**
  - field: `rust ≈ 1.000000·gempy − 2e-6`, normalized RMS residual `2.7e-15`
    (affine match) — PASS;
  - lithology: **100.00% voxel agreement** (the geometry-level truth);
  - isovalue magnitudes match gempy's (`±0.125`).

```bash
cd validation
uv run python dump_kernel.py            # assembly block diff
uv run python run_model1_validation.py  # field + litho diff (builds geofield-cli automatically)
```

### Known residual

The model1 field matches gempy up to a **uniform `−2e-6` constant offset** on the
potential (`std 4e-17` across all 125k grid points). It is geologically inert —
isosurfaces, gradient, and lithology are unaffected (hence the affine match at
`2.7e-15` and 100% litho agreement) — so a raw elementwise diff sits just above
`1e-6` while the affine diff passes. It traces to a constant gempy adds inside its
scalar evaluation kernel; closing it would buy raw `1e-6` agreement with no
geometric effect.

## Status summary

| capability | status |
|------------|--------|
| cubic kernel | ✅ verified |
| rescaling transform | ✅ matches gempy |
| system assembly (`K`, `b`) | ✅ bit-for-bit vs gempy |
| Bunch–Kaufman solve | ✅ |
| scalar field + analytic gradient | ✅ field matches gempy (affine, `2.7e-15`) |
| lithology | ✅ 100% voxel agreement vs gempy |
| WASM | ✅ builds + native parity `1.2e-15` |
| faults / multi-series / octrees / dual contouring | ⛔ out of scope |
| degree-2 drift | ⛔ seam only |

## License

MIT OR Apache-2.0
