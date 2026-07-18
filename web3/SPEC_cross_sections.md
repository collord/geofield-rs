# web3 — Cross-sections & digitized surfaces

**Status:** ABSORBED into `SPEC.md` §10–§11 (feature 005, implemented 2026-07-18). This fragment
was the proposal; the spec sections in `SPEC.md` are now the source of truth.

One deliberate amendment was made on merge: the pole formula in the old §11.2 (`m = t × s`)
contradicted its own §11.1 "youngs to your left" prose under the §10.1 frame and was corrected to
`m = normalize(ŝ × t̂)` — worked derivation in
`specs/005-cross-section-digitizing/contracts/younging-derivation.md`.

Design artifacts: `specs/005-cross-section-digitizing/` (spec, plan, research, data-model,
contracts, quickstart, tasks).
