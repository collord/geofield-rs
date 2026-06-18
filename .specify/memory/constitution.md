<!--
SYNC IMPACT REPORT — amendment 2026-06-18 (manual, during feature 001 implement)
- Version: 1.0.0 → 1.1.0 (MINOR — materially new normative guidance, no principle
  removed/redefined). Added the **functional-test (Playwright) gate** for the
  browser harnesses: extended Principle I (UI behavior cites an automated
  functional gate, not a manual walk) and added a matching Validation Standards
  bullet. Prompted by a real web3 defect (missing build artifacts + vite-env.d.ts)
  that a manual-only plan let slip and a functional gate would have caught.
- Modified principles: I (Ground Truth) — added UI/behavioral gate paragraph + gate
  clause. Others unchanged.
- Templates to propagate: plan-template.md Constitution Check Principle-I gate
  should mention the UI functional gate (updated in this same change).
- Prior history below (1.0.0 propagation pass).

SYNC IMPACT REPORT — /speckit-constitution run 2026-06-18
- Version: 1.0.0 (unchanged). This run was a consistency-propagation pass, not a
  principle change: the constitution was already authored at 1.0.0 with no
  remaining [PLACEHOLDER] tokens, so no semver bump. (Prior history: template →
  1.0.0 was the initial ratification earlier today.)
- Modified principles: none (text unchanged; all six retained as authored).
- Added sections: none in the constitution itself.
- Templates propagated:
  - ✅ .specify/templates/plan-template.md — replaced the "[Gates determined
    based on constitution file]" placeholder with six concrete, checkable
    Constitution Check gates (one per principle), wired to Complexity Tracking.
  - ✅ .specify/templates/spec-template.md — added an "Out of Scope / Non-Goals"
    section (enacts Principle IV: stated scope, documented punts, no silent
    coverage bounds).
  - ✅ .specify/templates/tasks-template.md — retargeted the Polish phase's
    cross-cutting list from generic items (e.g. "Security hardening") to this
    project's real gates: validation harnesses (I), native↔wasm parity (I/V),
    EUPL/BSD seam check (V), governing-doc + status-claim updates (II/IV).
  - n/a .specify/templates/commands/*.md — directory does not exist; nothing to scan.
  - n/a CLAUDE.md / README.md / docs/ — no pre-existing constitution references to reconcile.
- Deferred TODOs: none. Ratification date is set (2026-06-18, today), so no
  TODO(RATIFICATION_DATE) was needed.
-->
# geofield-rs Constitution

This workspace is two distinct bodies of work joined at a seam. **`geofield-kernel`
(EUPL)** reimplements a geological interpolation kernel that already exists
elsewhere (GemPy) — there it is not free to invent answers; there is a reference
and the job is to match it bit-for-bit. **`geofield` and `geofield-types`
(BSD)** are independent work — a meshing engine, shared interface types, and the
UI — that **must never embed or derive from GemPy**; their correctness comes from
geometric invariants, not from a GemPy reference. The GemPy lineage is confined
to the kernel crate and travels only with it. The shared ethic across both is:
**match the truth that governs you, name where you don't yet, and keep the seams
between what's proven and what's aspirational — and between EUPL and BSD —
visible.** The principles below are the norms the code already lives by, written
down so they bind future work and the people (and agents) doing it.

## Core Principles

### I. Ground Truth Over Self-Consistency (NON-NEGOTIABLE)

Correctness is established against the authority that governs each crate, never
against our own internals — but **each crate answers to a different authority,
and they do not cross.** `geofield-kernel` is validated **bit-for-bit against
GemPy's assembled system** and to round-off against its evaluated field and
lithology, because GemPy is its reference. `geofield` (the mesher) has **no
GemPy reference and must not acquire one**: it is validated against geometric
invariants (no holes, consistent winding, boundary-flush, manifold edges) and
against native↔wasm parity, not against "it looks right" and not against
GemPy. Confusing the two authorities — reaching for a GemPy comparison to
validate engine geometry, or accepting eyeballed output for kernel math — is a
defect.

A test that can be satisfied by a wrong-but-self-consistent implementation is a
*guard*, not an *oracle*, and MUST be labelled as such where it lives (see
`increment_consistency.rs`, which two mutually-compensating bugs once passed).
Every capability that claims correctness MUST cite the external gate that backs
it. "The numbers are stable" is not evidence; "matches the reference to `2.7e-15`
affine, 100% voxel agreement" is.

**This extends to the browser harnesses (`web`, `web2`, `web3`).** UI and
end-to-end behavior is the engine's outward face, and its gate is an **automated
functional test that drives the real app** (Playwright) — the UI analog of the
mesher's invariant test. "It type-checks" and "it looks right in the browser" are
the UI equivalents of self-consistency: necessary, not sufficient. A behavioral
acceptance criterion (a scenario loads, an edit round-trips and the view updates,
two affordances are distinguishable) is backed by a functional test that asserts
it, not by a manual walk-through. Manual verification is a smoke check, never the
gate of record.

*Gate:* a feature touching kernel math or field/lithology output is not "done"
until a validation harness (`validation/`) or an invariant test confirms it
against the reference. A feature whose acceptance is **UI/behavioral** is not
"done" until a Playwright functional test exercises that behavior against the
running app. If no such gate exists yet, building it is part of the feature, not
a follow-up — and if the environment cannot run the gate this pass, that is a
**logged, un-closed gap** (Principle IV), never a silent "verified."

### II. The Document Wins

Load-bearing math and architecture have a written source of truth that
**outranks the code**: `KERNEL_MATH.md` for the kernel (itself a transcription
of the GemPy reference, so it is where GemPy lineage is recorded), the relevant
`docs/` or `SPEC-*.md` for an engine subsystem. Where code and the governing
document disagree on a formula, sign, constant, epsilon, or contract, the
document is correct and the code is the bug — fix the code, or amend the document
deliberately (not as a side effect) and say why.

Traps that are easy to get wrong (the kernel's two distinct epsilons; "the cubic
consumes raw `r`, not `r²`") MUST be recorded at the point of danger, not left
for the next person to rediscover. A formula in code without a citation to its
governing section is incomplete. GemPy-derived formulas are documented **in the
kernel crate only**; the engine's docs describe geometry and contracts, never
GemPy internals.

### III. Derive, Don't Declare

The user (and the UI) declares the minimum irreducible state; everything
implied by that state is **derived by the core and never stored, duplicated, or
re-encoded.** Cross-cut sets are derived from chronological position, not
enumerated. There is exactly one free declared bit per stratigraphic boundary
(the erode/onlap/conformable seam); the rest follows.

Semantics MUST NOT be smuggled into an enum variant or flag that happens to
coincide with the real property at one point in the space (the `Basement`
category error is the cautionary tale: it proxied three independent behaviours
and bred a latent bug). When two things vary independently, model them as two
fields. **The single source of truth for any rule is one place in Rust** — a
second copy in the UI or a sibling module that can drift is a defect.

### IV. Honest Seams and Documented Punts

Scope is stated, not implied. Work that is out of scope gets a **clean seam left
where it would attach** and an explicit note (README Scope, the `[EXISTS]` /
`[EXTENDS]` / `[NEW]` tagging, "the one punt: document, don't solve"). A
limitation that is written down and bounded is acceptable; a limitation that is
silently present is not.

Status claims are calibrated: ✅ / ⛔ / "seam only" mean what they say. Never
mark something verified that is only plausible. When a feature bounds its own
coverage (sampling, top-N, a stubbed `blocks()` returning one block), that
bound is logged where a reader will see it, not left to look like full coverage.

### V. WASM-Safety and the License Seam Are Architectural, Not Optional

The engine must build for `wasm32-unknown-unknown` and agree with the native
build to `f64` round-off; parity is verified, not assumed. The EUPL kernel and
the BSD engine ship as **separate WebAssembly artifacts** and the engine reaches
the kernel only across a module boundary — so `geofield.wasm` contains no
EUPL-covered code. This boundary is enforced by the build graph (`cfg`-gated
deps), not by convention or comment.

*Gate:* no change may pull the kernel into the engine's wasm build graph, embed
EUPL-licensed or GemPy-derived code into a BSD crate (`geofield` /
`geofield-types`), or introduce a native-only dependency into a wasm-reachable
path without an explicit `cfg` gate. GemPy lineage lives in `geofield-kernel`
and nowhere else. Crossing the crate/license seam is a design decision that MUST
be called out in review.

### VI. Smallest Correct Intervention

Prefer the delta over the greenfield, the shim over the package, inline code
over a new abstraction. New work justifies its scope against what already
exists: the stratigraphic spec is written as a delta because most of the model
already exists; the right reflex is to reuse `mesh_one_block*` driven
differently, not to rebuild it. Larger scope must justify itself against who
else benefits and what existing convention says — "it was cheap to build" is
evidence against, not for. Diagnose before patching; a symptom fix without a
named cause is provisional and labelled so.

## Validation & Quality Standards

- **Two validation gates are first-class build artifacts**, even though they
  live outside the crate build: the *assembly* gate (`dump_kernel.py`,
  block-by-block diff vs GemPy) and the *field* gate (`run_model1_validation.py`,
  field + lithology diff). Changes to kernel math run both before being called
  done. The `validation/` harness uses its own `uv`-managed environment and is
  never coupled into `cargo build`.
- **Reference numbers are quoted, not rounded away.** Residuals, agreement
  percentages, and known offsets (the inert `−2e-6` potential offset) are
  recorded with their magnitude and their geological consequence (or lack of
  one), so a future regression is detectable as a *change in the number*.
- **Mesh output is verified by invariant**, not by screenshot: crack-free,
  consistent winding, gradient-coherent normals, manifold edges, flush to the
  true extent. New meshing paths add the invariant test that pins their failure
  mode.
- **Browser behavior is verified by a functional test**, not by a manual walk:
  the `web*/` harnesses carry **Playwright** end-to-end tests that drive the real
  app (load a scenario, perform an edit, assert the view/state changed). A feature
  with UI acceptance criteria adds the functional test that pins them. `tsc` +
  `vite build` are the *compile* gate (necessary); the Playwright run is the
  *behavior* gate (sufficient). Both run before a UI feature is called done.
- **Tests state what they are.** A necessary-but-insufficient guard says so in
  its own comments. A test's name and doc describe the failure it catches.

## Development Workflow

- **Commits are scoped and typed** (`feat(scope):`, `fix(scope):`,
  `refactor:`, `docs(scope):`, `chore:`) with the subsystem in the scope
  (`web2`, `fault`, `kernel`, `wasm-split`). One coherent change per commit; the
  message says what changed and, where non-obvious, why.
- **A spec precedes a non-trivial subsystem.** New cross-cutting work gets a
  `docs/` or `SPEC-*.md` that is honest about [EXISTS]/[EXTENDS]/[NEW], states
  non-goals, and gives a dependency-ordered work ledger. The spec is the artifact
  reviewers and agents read first; it is kept truthful as the work lands.
- **The `/speckit.*` workflow is the path for planned features.** `plan.md`'s
  Constitution Check gate reads the principles above; a plan that violates one
  records the violation and its justification in Complexity Tracking, or is
  revised. Unjustified complexity is rejected at the gate, not in review.
- **Coding agents are bound by this constitution.** It overrides convenience and
  the pull to be agreeable; an agent that sees a principle being violated names
  it rather than complying silently.

## Governance

This constitution supersedes ad-hoc practice. When practice and a principle
conflict, the principle wins or the principle is amended — not quietly ignored.

**Amendments** are made deliberately: edit this file (preferably via
`/speckit.constitution`, which re-syncs the dependent templates —
`plan-template.md`'s Constitution Check, `spec-template.md`, `tasks-template.md`),
state the rationale, and bump the version. Removing or materially redefining a
principle is a MAJOR change; adding a principle or a materially new rule is a
MINOR change; clarifications and wording are PATCH.

**Compliance** is checked where work is proposed (the plan's Constitution Check)
and where it lands (review). Every PR or planned feature verifies it does not
violate Principle I (an unbacked correctness claim), Principle III (a second
drifting copy of a rule), or Principle V (a license/wasm seam crossing) without
explicit, recorded justification. Complexity that a principle would forbid is
allowed only with a written reason for why the simpler path fails.

Runtime development guidance for agents lives in `CLAUDE.md` and the current
plan; this constitution is the layer above them that they answer to.

**Version**: 1.1.0 | **Ratified**: 2026-06-18 | **Last Amended**: 2026-06-18
