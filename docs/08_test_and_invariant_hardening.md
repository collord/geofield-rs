# Task 08 — Test & invariant hardening for mesh-mesh interaction

**Priority:** Medium. Depends on Task 01 (so the volume-watertight invariant has something to assert
against). The goal is to turn the issues above into permanent guardrails, and to close the gap where
the current tests only cover transversal, well-conditioned, generated geometry.

## Context — what the current tests do and don't cover

The existing suite is strong on the cases it covers (`m2_staircase_*`, `csg_truncation_is_watertight`,
`vein_body_meets_the_cross_cut_rock`, the wedge regression, the see-through-hole repro). But every
fixture is: (a) transversal crossings, (b) well-conditioned extents near the origin, (c) generated
meshes or field-backed predicates. The import-relevant failure modes — coplanar overlaps,
ill-conditioned extents, mis-wound multi-component meshes, curved-wall `Keep::Side`, volume seams —
are under-tested or untested. Each task above adds a targeted test; this task adds the *cross-cutting*
invariants and a fuzzer so regressions can't sneak back in through an untested combination.

## What to build

### A. A single reusable watertight-volume invariant

`mesh_clip.rs` has `assert_seam_watertight` (surface, incidence-3 along seam). `stack.rs` has several
ad-hoc edge-incidence loops (`mesh_one_block_unit_solid` tests ≈ line 2154, 2264;
`vein_body_meets_the_cross_cut_rock`). Factor ONE shared test helper:
`assert_volume_watertight(merged_mesh)` asserting every edge incidence is exactly 2 (closed solid),
parameterized by an `on_rim` predicate for legitimately-open boundaries. Replace the ad-hoc loops with
it so the criterion is defined once.

### B. Property/fuzz test for `clip`

Add a randomized test (seeded, deterministic) that generates two random-but-valid meshes (two jittered
grids with random dips, resolutions, and a random rigid transform applied to the whole scene including
a large translation) and asserts the `clip` post-conditions every time:

1. Watertight seam (`assert_seam_watertight`).
2. Anti-scallop: no kept C vertex strays off C's own least-squares plane (the existing check).
3. `shared` pairs are consistent: each referenced seam vertex on C has a partner on W at the same
   position within float noise.
4. No orphaned vertices (the `compact` pass should guarantee this — assert it).

Run enough seeds to exercise oblique crossings, near-misses, and the empty-seam branch. This is where
the ill-conditioned-extent and near-coplanar bugs will surface before a user hits them.

### C. A volume-seam regression matching Task 01

Once Task 01 lands, add the body∪rock-solid watertight test described there to this shared harness, so
the volume path is guarded by the same invariant as the surface path.

### D. Native vs wasm determinism harness (if not already present)

The module leans on `cdt`/`spade`'s exact Shewchuk predicates being "native/wasm-identical." Add a
test (or CI step) that serializes a `clip` result and asserts byte-identical output across targets for
a fixed fixture, so a future change that breaks determinism (e.g. a `HashMap` iteration order leaking
into output) is caught. Note: `clip` currently iterates `HashMap`s (`c_seam_index`, `w_seam_index`,
`by_c`, `by_w`) and sorts `shared` — audit whether any output ordering depends on `HashMap` iteration
order, because that is non-deterministic and would break the cross-target guarantee. If it does, sort
or use an ordered map before emitting.

## Concrete steps

1. Factor `assert_volume_watertight` and `assert_seam_watertight` into a shared test util module
   (`mesh_test_util.rs` under `#[cfg(test)]`, or a `tests/` integration helper) used by both files.
2. Write the seeded fuzz test (B). Keep iteration counts modest so CI stays fast; expose the seed on
   failure for reproduction.
3. Audit `clip` for `HashMap`-iteration-order dependence in OUTPUT (D), fix if found, add the
   determinism test.
4. After Task 01: add (C).

## Acceptance

- The shared invariants exist and the ad-hoc incidence loops are replaced by calls to them.
- The fuzz test runs green over its seed set and has caught at least the ill-conditioned-extent issue
  when pointed at the pre-Task-04 code (use it to validate Task 04, then keep it).
- Determinism test green on native and wasm.

## Watch-out

- Don't let the fuzzer generate degenerate inputs (zero-area triangles, coincident meshes) unless the
  contract says they're handled — gate the generator to the validated domain, and add the degenerate
  cases as *separate explicit* tests tied to Task 05's coplanar work rather than folding them into the
  property test.
