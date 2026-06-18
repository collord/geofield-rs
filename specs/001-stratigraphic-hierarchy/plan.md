# Implementation Plan: Stratigraphic Hierarchy — Model, UI, and Testbed

**Branch**: `001-stratigraphic-hierarchy` (working on `main` per project Git convention) | **Date**: 2026-06-18 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/001-stratigraphic-hierarchy/spec.md`

**Design source of record**: [`docs/stack_hierarchy_spec.md`](../../docs/stack_hierarchy_spec.md) — authoritative for model semantics, Rust contracts, and the UI↔core protocol. This plan defers to it (Constitution Principle II/III) and does not restate its signatures.

## Summary

Implement the full stratigraphic-hierarchy delta from the design doc: (1) the
two-axis model cleanup retiring `Relation::Basement` into `relation:
Option<Relation>` + `unbounded_above: bool`; (2) cross-cutting bodies (veins,
intrusions) as new stack-element kinds with derived cut-sets; (3) inverting
faults from per-series nesting to an authoritative outer partition that instances
the stack per block; (4) a shared surface-entity store; (5) a generated UI↔core
schema and the mutation/derive/mesh round-trip across the wasm boundary; (6) a
reusable React hierarchy panel; and (7) a new `web3/` browser testbed that drives
the WASM core on synthetic scenarios, following the existing `web/` harness
pattern. Each phase reuses the tested meshing primitives in `stack.rs` rather
than rebuilding them.

## Technical Context

**Language/Version**: Rust (edition 2021, workspace resolver 2; toolchain per repo) for the model/engine; TypeScript 5.5 + React 18 + Vite 5 + three.js 0.169 for the UI/testbed (matching `web/`).

**Primary Dependencies**: `geofield` (BSD meshing engine, where `stack.rs` lives), `geofield-kernel` (EUPL cokriging core, native-linked / wasm-imported), `geofield-types` (BSD interface crate — `ScalarField`, `HalfSpace`); `wasm-bindgen` + `wasm-pack` for the ABI; the two-wasm split (kernel + engine artifacts) per README. **NEEDS CLARIFICATION**: schema-generation tool for FR-020 (ts-rs vs. continue the existing hand-mirrored TS) — resolved in research.md.

**Storage**: N/A — no persistence. Declared model state is held in-memory by the core and (editably) by the UI store; synthetic scenarios are static fixtures bundled with `web3/` (mirrors `web/` data.ts / web2 generated scenarios).

**Testing**: `cargo test` (native, in-process kernel) for model invariants and regressions — the existing `crates/geofield/tests/` + `stack.rs` unit tests; native↔wasm parity check as in README; browser-driven manual validation of `web3/` per quickstart. No new test framework introduced.

**Target Platform**: Native (tests/CLI) and `wasm32-unknown-unknown` (browser). The engine must build for both; wasm reaches the kernel oracle across the module boundary.

**Project Type**: Rust library workspace + browser web app (the `web/`-style harness). The plan touches the `geofield` crate, its `wasm.rs` ABI, a new TS UI package, and a new `web3/` app.

**Performance Goals**: Interactive re-mesh on edit (the `web/` worker pattern: one fit/mesh per gesture, hard-terminate to cancel). No new performance target beyond "edits re-mesh without blocking the UI thread," achieved via the existing worker pattern. Synthetic scenarios are small (demo-scale), so the decimation work already in the kernel is not on the critical path here.

**Constraints**: WASM-safe engine (no native-only deps in the wasm graph without a `cfg` gate); the EUPL/BSD seam — no GemPy-derived or kernel code embedded in the BSD engine or UI; the model layer must preserve every existing `stack.rs` test, with the missing-volume regression re-keyed onto `unbounded_above`.

**Scale/Scope**: Large. Seven workstreams in the design doc's dependency order; the testbed ships ≥4 synthetic scenarios. See Complexity Tracking for the phasing decision.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Derived from `.specify/memory/constitution.md` v1.0.0.

- [x] **I — Ground truth (NON-NEGOTIABLE).** Model changes are validated by the
  existing invariant/regression suite, not appearance: watertightness, the
  `onlap_vs_erode_differ` discriminator, `basement_partitions_domain`, and the
  missing-volume regression (re-keyed onto `unbounded_above`). New behaviors
  (vein band-clip, intrusion interior, per-block instancing) each add a
  failure-pinning test. No GemPy reference enters `geofield`/UI; the kernel is
  untouched by this feature. **Pass.**
- [x] **II — The document wins.** `docs/stack_hierarchy_spec.md` is the governing
  doc and the source of record; the spec and this plan defer to it. `stack.rs`
  module docs are updated to match the two-axis model when it lands. **Pass.**
- [x] **III — Derive, don't declare.** The two-axis split is the canonical
  example (retire the overloaded `Basement` variant; `unbounded_above` is the one
  honest bit). Cut-sets are derived (`cuts_below`), never enumerated. One free
  Axis-A bit per boundary. The UI holds only declared state and treats `derived`
  as a read-only cache. **One risk to manage:** FR-020's generated schema must be
  the single source of truth so the UI cannot re-encode the grammar — see the
  Phase 0 research item; choosing the hand-mirror path would re-introduce a
  drift-prone second copy and must be justified or rejected. **Pass, with research
  item.**
- [x] **IV — Honest seams & documented punts.** The spec's Out-of-Scope section
  carries the doc's non-goals (syn-depositional intrusions, fault displacement,
  general DAG editor, per-block data overrides). The fault inversion stubs
  `blocks()` to one block first (interface real, geometry deferred) — logged, not
  silent. **Pass.**
- [x] **V — WASM-safety & license seam (ARCHITECTURAL).** New model code lands in
  `geofield` (BSD), generic over `SeriesModel`/`BlockModel` so it drives both the
  native `Model` and the wasm `RemoteModel` unchanged. No kernel/GemPy code in the
  engine or UI; `web3/` loads the two wasm artifacts exactly as `web/` does. ABI
  additions stay `#[cfg(feature="wasm")]`. **Pass.**
- [x] **VI — Smallest correct intervention.** Reuse over rebuild is explicit: the
  per-block meshing primitives (`mesh_one_block*`, `block_half_spaces`,
  `erode_masker_fields`) survive and are driven differently; cross-cut maskers
  mirror the erode masker wiring; veins reuse `ContactHalfSpace` (two ANDed). The
  one scope decision needing justification is the **whole-feature-at-once vs.
  P1-slice-first** phasing — see Complexity Tracking. **Pass, with tracked
  decision.**

**Gate result: PASS.** No unjustified violations. Two items flagged for explicit
handling (FR-020 schema source-of-truth in research; phasing in Complexity
Tracking).

## Project Structure

### Documentation (this feature)

```text
specs/001-stratigraphic-hierarchy/
├── plan.md              # This file
├── spec.md              # Feature spec (input)
├── research.md          # Phase 0 output (this command)
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output (UI↔core contract, wasm ABI delta)
│   ├── core-api.md
│   └── wasm-abi-delta.md
└── checklists/
    └── requirements.md  # spec quality checklist (already created by /speckit-specify)
```

### Source Code (repository root)

```text
crates/geofield/                 # BSD engine — the model layer changes land here
├── src/
│   ├── stack.rs                 # [EXTENDS] two-axis model; [NEW] StackElement (Series|Vein|Intrusion); cuts_below
│   ├── faulted.rs               # [NEW] FaultedModel / FaultNetwork — outer partition, instances Stack per block
│   ├── surface.rs               # [NEW] SurfaceId entity store / DAG references
│   ├── schema.rs                # [NEW, optional] ts-rs-derived DTOs (gated) — pending research decision
│   ├── wasm.rs                  # [EXTENDS] ABI: element kinds, mutations, derived-data getters
│   ├── octree.rs                # [UNCHANGED] ContactHalfSpace etc. reused
│   └── remote.rs                # [UNCHANGED] RemoteModel — wasm field oracle
└── tests/                       # [EXTENDS] new invariant tests; existing suite stays green

crates/geofield-types/src/lib.rs # [MAYBE] shared DTO shapes if generated types live at the interface

web3/                            # [NEW] testbed app, mirroring web/ structure
├── src/
│   ├── HierarchyPanel/          # [NEW] reusable React hierarchy components (the FR-019 artifact)
│   ├── protocol.ts              # worker message contract (from web/)
│   ├── worker.ts                # solver worker (from web/)
│   ├── kernel-bridge.ts         # two-wasm wiring (from web/)
│   ├── scenarios.ts             # [NEW] synthetic scenarios (≥4)
│   ├── Viewer.tsx               # three.js viewer (from web/)
│   └── main.tsx
├── package.json                 # wasm:kernel / wasm:engine / dev / build scripts (from web/)
└── index.html
```

**Structure Decision**: Model + ABI changes live in the existing `geofield`
(BSD) crate; UI is a new `web3/` app following `web/`'s proven two-wasm harness
(worker + kernel-bridge + viewer). The reusable hierarchy components are a
subdirectory packaged for embedding (FR-019), with `web3/` as their first
consumer. No new crate is introduced — the model work is a delta to `stack.rs`
and siblings (Principle VI).

## Complexity Tracking

> Filled because the Constitution Check flagged a scope/phasing decision that
> must be justified rather than assumed.

| Violation / Decision | Why Needed | Simpler Alternative & Why (Not) Rejected |
|----------------------|------------|------------------------------------------|
| Plan covers the **entire 7-workstream feature** in one spec, not a single slice | The user explicitly asked to "implement docs/stack_hierarchy_spec.md **in its entirety**" plus a testbed — the whole delta is the stated unit of work | Slicing to P1-only (basement decomposition + minimal UI + testbed) is **the recommended execution order, not a smaller scope**: `/speckit-tasks` will dependency-order the work so P1 (model cleanup + UI shell + testbed) is shippable first and P2 (cross-cutting bodies, fault inversion) follows. The plan stays whole; the *tasks* phase the delivery. This honors Principle VI (smallest first increment) without contradicting the request. |
| New `web3/` app rather than extending `web/` | The prompt asks for a fresh testbed "like web/" that shows off the new hierarchy UI; `web/` encodes the *old* flat `relation: number` model and would have to be rewritten | Extending `web/` in place would entangle the new two-axis/cross-cut UI with the existing editor and risk regressing a working harness; a parallel `web3/` keeps `web/` as a working reference (and lets the synthetic scenarios diverge). Justified. |
| Possible new `schema.rs` + ts-rs dependency (FR-020) | The design doc §7 mandates generated types so the UI cannot hold a second drifting copy of the grammar (Principle III) | The existing hand-mirrored TS (`web/relations.ts`, `protocol.ts`) is the simpler status quo but is *exactly* the drift-prone second copy Principle III warns against; the two-axis + cross-cut grammar is far richer than the 4 integer relation codes, raising drift risk. Decision deferred to research.md — adopt ts-rs only if the drift cost clears the new-dependency bar. |

## Phase 0: Research

See [research.md](research.md). Resolves: (R1) FR-020 schema generation —
ts-rs vs. hand-mirror; (R2) `StackElement` modeling — enum vs. trait-object, per
design doc §3.1 Option A; (R3) fault-inversion sequencing — how to stub
`blocks()` to one block while keeping the per-block primitives' interface real;
(R4) React DnD approach for the two distinct drag semantics (§6.3); (R5)
`web3/` build wiring reuse from `web/`.

## Phase 1: Design & Contracts

See [data-model.md](data-model.md), [contracts/](contracts/), and
[quickstart.md](quickstart.md). Re-evaluate the Constitution Check after design
(recorded at the end of research.md).
