# Phase 0 Research: Stratigraphic Hierarchy

All NEEDS CLARIFICATION items from the plan's Technical Context are resolved here.
Format per decision: **Decision / Rationale / Alternatives considered**. Where a
decision touches a constitution principle, it is named.

## R1 — FR-020 UI↔core schema: ts-rs vs. hand-mirrored TS

**Decision**: Adopt **ts-rs**-generated DTOs for the new hierarchy contract
(`StackElementDTO`, `CoreUpdate`, `derived` payloads), generated from Rust types
in the `geofield` (BSD) crate behind a dev-only feature/test that emits `.ts`.
Keep the existing flat integer ABI (`WasmStack.add_series`, `relation: number`)
for the low-level wasm calls, but the *editable declared state and derived data*
that the hierarchy UI consumes crosses as generated types.

**Rationale**: Constitution Principle III makes the single-source-of-truth the
deciding factor. The current hand-mirror (`web/relations.ts` mirrors 4 integer
codes; `protocol.ts` mirrors the flat series JSON) is tolerable for 4 enum values
but the new grammar — three-state seam *plus* `unbounded_above`, three element
kinds with different payloads, derived `cutsBelow` / `effectiveRelation` /
`validation` / `absentInBlock` — is exactly the richer, drift-prone surface the
design doc §7 calls out ("the UI cannot encode a second, drifting copy of
erode/onlap/cross-cut logic"). Generated types make drift impossible by
construction, which is the same discipline the project already applies to the
EUPL/BSD seam (enforced, not conventional). The new-dependency bar (Principle VI)
is cleared because the alternative *is* the violation.

**Alternatives considered**:
- *Continue hand-mirroring* — simplest, no new dep, but reintroduces the exact
  Principle III hazard; rejected for the rich grammar (kept only for the
  unchanged flat field-eval/mesh ABI, which is stable and small).
- *serde_json round-trip with runtime validation* — no compile-time TS types;
  drift surfaces at runtime, not build time. Rejected.
- *Generate via schemars + json-schema → quicktype* — heavier toolchain than
  ts-rs for the same result. Rejected.

**Constraint**: ts-rs is a `dev-dependency` / `cfg(test)`-gated export so it never
enters the shipped wasm graph (Principle V). The generated `.ts` is committed and
its freshness is checked in CI/`cargo test` (a test fails if regeneration would
change the file), so the generated artifact cannot silently drift from the Rust
source either.

**R1 implementation update (feature 001 implement, T016).** `ts-rs` is **not
fetchable in the current offline build environment** (`serde` 1.0.228 is cached;
`ts-rs` is not). Pulling an unfetchable dependency would break `cargo build`, so
the auto-generation is **deferred** (logged, not silently dropped — Principle IV).
The single-source-of-truth intent (Principle III) is preserved by the smaller
means available now: the DTOs are defined ONCE in Rust (`schema.rs`) with `serde`;
the matching TS is hand-written in `web3/src/HierarchyPanel/dto.ts` with a header
pointing back to `schema.rs` as canonical; and a **native round-trip purity test**
pins the (de)serialization contract. Drift risk is bounded because the DTO surface
is small (Series + derived view) and the round-trip test fails if the Rust shape
changes incompatibly. **Tracked follow-up:** when `ts-rs` is available, add the
`cfg(test)` export + freshness check (the original T003/T004) and delete the
hand-written `dto.ts`. This is the one place the MVP falls short of R1's ideal;
it is bounded and reversible.

## R2 — `StackElement` modeling: enum vs. trait object

**Decision**: **Enum `StackElement<M>`** with variants `Series(StackedSeries<M>)`,
`Vein(Vein<M>)`, `Intrusion(Intrusion<M>)` — the design doc §3.1 **Option A**,
recommended there.

**Rationale**: The doc already weighed this and recommends Option A because the
kinds carry *different geometry payloads* (a HW/FW interface pair; a body
envelope), so a flag on `StackedSeries` (Option B, `Relation::CrossCut`) would
strain `Relation`, which is otherwise purely the seam-below axis. An enum keeps
the payload differences honest and pattern-matchable; lithology
(`global_unit`) stays unchanged because youngest-present-wins is uniform across
kinds (`present_at` differs per kind, the walk does not). This also fits
Principle III: the kind is declared once; behavior (cut-set, masker geometry) is
derived per kind.

**Alternatives considered**:
- *Option B (`Relation::CrossCut` + a geometry tag)* — lighter but overloads
  `Relation` with something that is not a seam-below relationship; the same
  category-error pattern Principle III warns against. Rejected per the doc.
- *Trait object `Box<dyn StackElementBehavior>`* — most extensible, but erases
  the payload types and complicates the `M: SeriesModel` generic that lets the
  same code drive native + wasm. Over-scoped (Principle VI). Rejected.

## R3 — Fault inversion sequencing

**Decision**: Introduce `FaultedModel<M>` / `FaultNetwork` with `blocks()`
**stubbed to return a single whole-domain block** first, so the *interface* shape
(outer block index instancing the stack) is real and tested before the partition
*geometry* lands. The existing per-block primitives (`mesh_one_block`,
`mesh_one_block_unit_solid`, `block_half_spaces`, `block_field`,
`block_interface_isovalues`) are driven by the outer block index instead of the
per-series `nblocks()`.

**Rationale**: Principle IV (no silent coverage bounds) + Principle VI (reuse,
incremental). Stubbing `blocks()` to one block is a *logged* bound, not a hidden
one — the single-block path exercises the full instancing machinery and keeps
every existing per-block test green, then the real partition geometry is a
contained follow-up that only changes what `blocks()` returns. The doc §4.3
explicitly says the primitives "survive, driven differently," which this honors.

**Alternatives considered**:
- *Build the full fault-network partition geometry up front* — large, and would
  block the rest of the feature on the hardest geometry. Rejected for sequencing.
- *Leave faults per-series (don't invert)* — contradicts the doc's §4 decision
  and the geological model; the inversion is a stated goal. Rejected.

## R4 — React DnD for the two distinct drag semantics

**Decision**: A **single DnD library** (dnd-kit) with **one explicit resolver**
that branches drop behavior on `(dragNodeType, dropZoneType)` — container-reorder
(chronology) vs. within-container surface-reorder (isovalue order) vs.
cross-container surface move — exactly as design doc §6.3 prescribes ("one DnD
library; branch drop semantics on (dragNodeType, dropZoneType) in a single
explicit resolver").

**Rationale**: The doc fixes this design; research only confirms a concrete
library. dnd-kit is the current React-18-compatible, accessibility-aware,
unopinionated choice that supports custom drop affordances (needed so the two
behaviors are *visually distinct* per FR-015/SC-006). The visual distinction
(strong full-width insertion bar vs. lighter indented bar) is presentation, built
on whatever library renders custom overlays — dnd-kit does.

**Alternatives considered**:
- *react-dnd* — HTML5-backend quirks and heavier API; dnd-kit is the lighter
  modern default. Acceptable but not chosen.
- *Two libraries / two DnD contexts for the two behaviors* — directly violates
  the doc's "one library, one resolver"; risks the two behaviors diverging in
  feel. Rejected.
- *Hand-rolled pointer-event DnD* — re-implements a solved problem; over-scoped
  (Principle VI). Rejected.

## R5 — `web3/` build wiring

**Decision**: Copy `web/`'s two-wasm harness wiring verbatim as the starting
point: the `package.json` scripts (`wasm:kernel` builds `geofield_kernel.wasm`,
`wasm:engine` runs `wasm-pack build ../crates/geofield`), `kernel-bridge.ts`
(kernel-first init + memory bridge), `worker.ts` (solver-in-worker with streamed
status), and `protocol.ts` (worker message contract). Layer the new hierarchy UI
and generated DTOs on top.

**Rationale**: Principle VI (reuse) and the prompt's own "basic pattern like
web/". `web/` already solves the hard parts — the EUPL/BSD two-artifact load
order, the wasm-module-boundary field oracle, hard-terminate cancellation, the
worker streaming. Re-deriving them in `web3/` would be redundant and risk
re-introducing the bugs the `web/` commits already fixed (per git log: panic
hooks, init order, memory bridge). The only genuinely new wiring is the
generated-DTO round-trip (R1) and the scenarios fixture (R-scenarios below).

**Alternatives considered**:
- *Single-thread (no worker)* — a fit/mesh blocks the UI thread; `web/` moved to
  a worker for exactly this reason (git: "run the solver in a web worker"). Rejected.
- *Share a package between web/ and web3/* — premature extraction; the two apps'
  needs diverge (old flat model vs. new hierarchy). Defer until a third consumer
  exists (Principle VI). Rejected for now.

## R-scenarios — synthetic scenario authoring

**Decision**: Author the ≥4 scenarios (conformal column, erosional unconformity,
one cross-cutting body, one fault network) as **small fitted-plane fixtures in TS**
that build `StackElementDTO`s, mirroring how `stack.rs` tests build dipping-plane
`Model`s and how web2's `gen/generate_scenario.py` produces synthetic data. They
live in `web3/src/scenarios.ts` and are loaded into the core via the generated
mutation API.

**Rationale**: Demo-scale synthetic data needs no real dataset (spec Assumptions).
Reusing the dipping-plane recipe keeps the scenarios geologically meaningful and
cheap to fit interactively. The fault scenario reuses the same one-fault geometry
the existing `faulted_series_meshes_per_block` test uses, so it is known-good.

**Alternatives considered**:
- *Import real datasets* — out of scope (spec). Rejected.
- *Generate via the Python harness only* — couples the browser testbed to a
  Python build step; TS fixtures keep `web3/` self-contained. Rejected (keep
  Python harness for kernel validation, not UI fixtures).

## Post-design Constitution re-check

After the Phase 1 design (data-model, contracts, quickstart) the gates still hold:

- **I**: every new model behavior maps to a named invariant/regression test
  (data-model §Validation). No GemPy reference enters the engine/UI.
- **II**: data-model and contracts defer to `docs/stack_hierarchy_spec.md`; no
  competing source of truth introduced.
- **III**: R1 chooses generated types precisely to avoid a second copy of the
  grammar; `cuts_below`/derived data are core-owned. **Strengthened by design.**
- **IV**: R3's single-block stub is a logged bound; scenarios enumerate coverage.
- **V**: R1's ts-rs export is dev-gated; `web3/` reuses the two-artifact split.
- **VI**: R2/R3/R4/R5 all chose reuse/smallest-viable over greenfield.

**Re-check result: PASS.** No new violations introduced by the design.
