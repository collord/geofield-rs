# Quickstart — Cross-Sections & Digitized Surfaces (005)

How to build, run, and validate this feature end to end. Data shapes:
[data-model.md](data-model.md); math: [contracts/younging-derivation.md](contracts/younging-derivation.md).

## Prerequisites

- Node + npm (web3 toolchain); Playwright browsers installed (`npx playwright install chromium`).
- **No wasm rebuild is required for this feature** (no Rust change). If `web3/src/wasm*` artifacts
  are missing locally: `cd web3 && npm run wasm`.

## Build & run

```bash
cd web3
npm install          # once
npm run build        # compile gate: tsc -b && vite build (must pass)
npm run dev          # interactive dev server, or:
npm run preview      # serve the production build (what e2e drives, :4173)
```

## Behavior gate (Constitution I — the gate of record)

```bash
cd web3
npm run build && npx playwright test section-   # the five 005 specs
npx playwright test                             # full suite (no regressions)
```

| Spec | Pins |
|---|---|
| `e2e/section-view.spec.ts` | two-click baseline → set created (`__sections`); open face-on (ortho, u-right/Z-up via `__getView`); traces present (`__sectionTraces`); backdrop points; stale-scrim in section view; **create/step/delete N sections ⇒ dirty stays false and no fit runs** (SC-004) |
| `e2e/section-digitize.spec.ts` | canvas-click digitizing on the plane (gesture path); vertices appear in points table + overlay counts; younging affordance on the younger side per the fixed convention; **flip changes only the younger side, zero vertex deltas** (SC-003) |
| `e2e/section-loft.spec.ts` | `__digitizeLine` on ≥3 serial slices → one Apply → new surface in stack; **each slice's re-fit trace passes within ε = 0.1·spacing of every drawn vertex** (SC-002); stale-scrim until Apply |
| `e2e/section-persist.spec.ts` | export → import round-trip restores sections, serial params, active slice (SC-005); pre-005 project file still loads (additive schema) |
| `e2e/section-validation.spec.ts` | §11.4 matrix: <2 vertices blocks with reason; single-plane-only-data blocks; one-section warns; opposed younging warns + flip-to-match fixes it; self-crossing warns (SC-007) |

Role-parity (SC-006) is asserted inside `section-loft.spec.ts` by giving the lofted surface the
erode relation and checking truncation — reusing existing solid-mode assertions.

## Manual smoke walk (never the gate)

1. Seed a template → Apply → status: done.
2. Top-down → **New section** → two clicks → serial dialog: spacing/count (e.g. 5 × 200) → open.
3. Read traces face-on; toggle Solids to see filled intervals; step Next/Prev.
4. Pick "+ new surface" as digitize target; draw a line on 3 slices (watch arrow + hachures;
   try Flip); Apply once; step the slices — the new surface's trace lands on your lines.
5. Set the new surface's relation to erode; confirm it truncates older beds in 3D.
6. Export project; reload; re-import — same sections, same open slice.

## Failure-mode notes

- Solid-mode fill requires the Manifold path (default). With the diagnostic Manifold toggle OFF,
  sections show unfilled boundary segments (logged bound, research.md R2) — not a defect.
- Section step re-slices UI-side from the last-applied meshes; if the model was never applied,
  section views show only backdrop points and digitized lines (nothing to trace).
