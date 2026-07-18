# Contract — Section Frame & Younging/Pole Derivation

The single normative statement of the section-frame and digitized-pole math. Implemented once in
`web3/src/section/frame.ts` + `web3/src/section/digitizeDerive.ts`; consumed by the Viewer
(affordance), the compile path (`flattenInterfaces`), and validation. Code at those sites cites this
file. Axis convention throughout: **X=East, Y=North, Z=Up** (`SPEC.md §1.2`).

## 1. Section frame

Given baseline `A → B` picked in plan view (world XY, at any z):

```
u = normalize([Bx−Ax, By−Ay, 0])        // in-plane horizontal axis; "screen right" face-on
w = [0, 0, 1]                           // in-plane vertical axis; "screen up"
s = u × w                               // out-of-plane unit normal (horizontal)
```

`(u, w, s)` is right-handed by construction (`u × w = s`); concretely, for u=+X̂: `s = X̂ × Ẑ = −Ŷ`.
The **face-on camera** is placed on the side `s` points toward and looks along `−s`, with `camera.up =
+Z`; this puts `u` to screen-right and `Z` up (US-SECT-3) and makes **`s` point toward the viewer**.

Plane *k* of a serial set (0-based): point `Pk = A + s·(offset + k·spacing)`, normal `s`, spanning
the extent's full Z-range and the baseline's u-range. In-plane coordinates of a world point `p`:
`du = dot(p − Pk, u)`, `dw = pz − z0` (any fixed z origin; used for 2D loop/self-cross tests).

**Slab tolerance** (backdrop points, US-SECT-3): show authored points with
`|dot(p − Pk, s)| ≤ tol`; `tol = spacing/2` for `count > 1`, else 2.5% of the extent diagonal (R8).

## 2. Digitized pole (the younging rule)

**Normative prose (spec FR-013)**: viewed face-on (s toward the viewer, Z up), the surface **youngs
to the LEFT of the drawn direction**; the derived pole points to the younger side. The convention is
fixed, not configurable.

For each segment `vᵢ → vᵢ₊₁` of a digitized line with plane normal `ŝ` (from the line's stored
`frame`, NOT from any live section):

```
t̂ᵢ = normalize(vᵢ₊₁ − vᵢ)              // drawn-direction tangent, in-plane
mᵢ = ŝ × t̂ᵢ                            // unit pole to the YOUNGER side  ← normative formula
```

One orientation sample per segment: position = midpoint `(vᵢ + vᵢ₊₁)/2`, pole = `mᵢ` (R4).

**Worked example (pins the sign):** baseline A=(0,0) → B=(1,0): `u = +X̂`, `s = X̂×Ẑ = −Ŷ`.
Face-on camera sits at −Y looking +Y (so `s=−Ŷ` points toward it), u=+X̂ to screen-right, +Ẑ up.
Draw a horizontal line left-to-right on screen: `t̂ = +X̂`. Screen-left of a rightward walk is
**up** = +Ẑ. Check: `mᵢ = ŝ × t̂ = (−Ŷ) × X̂ = +Ẑ` ✓ (pole up ⇒ youngs upward — the ordinary
right-way-up bed). The rejected `t̂ × ŝ = −Ẑ` would young it downward. **Amendment note
(Principle II):** the source fragment `SPEC_cross_sections.md §11.2` wrote `m = normalize(t × s)`,
which contradicts its own §11.1 "youngs to your left" prose under §10.1's `s = u × w` frame; on
merge into `SPEC.md`, the formula is corrected to `m = ŝ × t̂` with a note citing this contract.

## 3. Flip composition (single site)

The compiled pole for a digitized segment of interface `iface` in a series with relation-default
flip `defaultFlip` is:

```
sign = (!!iface.flipped !== defaultFlip) ? −1 : +1      // the EXISTING XOR, project.ts:253
ori_pole ← sign · mᵢ
```

- **Per-surface flip (US-DATA-2)** toggles `iface.flipped`: negates every derived pole (typed AND
  digitized), never rewrites `vertices` or typed dip/dipDir. Idempotent double-flip. (FR-015)
- **Per-line "flip to match"** (validation offer, FR-020) reverses that line's `vertices` order —
  a geometry edit — which negates every `t̂ᵢ` and hence every `mᵢ` of that line only.
- The younging **affordance** renders from `sign · mᵢ` (what the fit will actually see), so what the
  arrow/hachures show is always the effective younger side.

## 4. Degeneracies

- Zero-length segment (`|vᵢ₊₁ − vᵢ| < ε`): contributes no orientation sample (skip; vertices still
  contribute `sp_xyz`).
- A 2-vertex line ⇒ exactly 1 orientation sample. A 1-vertex line is a hard validation error.
- Back-tracking/self-crossing lines still derive per-segment poles (well-defined per segment) but
  raise the soft "ambiguous direction" warning (FR-020).
