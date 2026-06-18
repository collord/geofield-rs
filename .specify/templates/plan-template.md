# Implementation Plan: [FEATURE]

**Branch**: `[###-feature-name]` | **Date**: [DATE] | **Spec**: [link]

**Input**: Feature specification from `/specs/[###-feature-name]/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command. See `.specify/templates/plan-template.md` for the execution workflow.

## Summary

[Extract from feature spec: primary requirement + technical approach from research]

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for the project. The structure here is presented in advisory capacity to guide
  the iteration process.
-->

**Language/Version**: [e.g., Python 3.11, Swift 5.9, Rust 1.75 or NEEDS CLARIFICATION]

**Primary Dependencies**: [e.g., FastAPI, UIKit, LLVM or NEEDS CLARIFICATION]

**Storage**: [if applicable, e.g., PostgreSQL, CoreData, files or N/A]

**Testing**: [e.g., pytest, XCTest, cargo test or NEEDS CLARIFICATION]

**Target Platform**: [e.g., Linux server, iOS 15+, WASM or NEEDS CLARIFICATION]

**Project Type**: [e.g., library/cli/web-service/mobile-app/compiler/desktop-app or NEEDS CLARIFICATION]

**Performance Goals**: [domain-specific, e.g., 1000 req/s, 10k lines/sec, 60 fps or NEEDS CLARIFICATION]

**Constraints**: [domain-specific, e.g., <200ms p95, <100MB memory, offline-capable or NEEDS CLARIFICATION]

**Scale/Scope**: [domain-specific, e.g., 10k users, 1M LOC, 50 screens or NEEDS CLARIFICATION]

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Derived from `.specify/memory/constitution.md` v1.0.0. Each gate is a hard stop;
a plan that trips one either revises to clear it or records the violation and its
justification in **Complexity Tracking** below.

- [ ] **I — Ground truth (NON-NEGOTIABLE).** Every correctness claim names the
  external gate that backs it. Kernel-math / field / lithology work cites a
  `validation/` harness or bit-for-bit GemPy diff; `geofield` (mesher/types) work
  cites a geometric invariant + native↔wasm parity — and introduces **no GemPy
  reference**. **UI/behavioral work (`web*/`) cites an automated Playwright
  functional test, not a manual walk-through** (`tsc`/`vite build` are the compile
  gate; Playwright is the behavior gate). If a needed gate does not exist yet,
  building it is in scope, not a follow-up; if the environment can't run it this
  pass, that is a logged gap, never a silent "verified." Self-consistency checks
  are labelled as guards, not oracles.
- [ ] **II — The document wins.** Load-bearing math/architecture has a governing
  doc (`KERNEL_MATH.md` for the kernel; `docs/` or a `SPEC-*.md` for an engine
  subsystem). The plan cites it; any formula/contract change amends the doc
  deliberately. GemPy internals are documented only in the kernel crate.
- [ ] **III — Derive, don't declare.** Only irreducible state is declared;
  implied state is core-derived, stored once, never duplicated. No semantics
  smuggled into an enum variant/flag that merely coincides with the real
  property. No second, drift-prone copy of any rule (esp. UI re-encoding core
  logic).
- [ ] **IV — Honest seams & documented punts.** Out-of-scope work gets a clean
  seam + an explicit note; the plan states non-goals. No silent coverage bounds
  (sampling, top-N, stubbed returns) — each is logged where a reader sees it.
  Status claims are calibrated (✅ / ⛔ / seam-only mean what they say).
- [ ] **V — WASM-safety & license seam (ARCHITECTURAL).** No change pulls the
  kernel into the engine's wasm build graph, embeds EUPL- or GemPy-derived code
  into a BSD crate (`geofield` / `geofield-types`), or adds a native-only dep to a
  wasm-reachable path without a `cfg` gate. Any seam crossing is called out here.
- [ ] **VI — Smallest correct intervention.** Scope is the delta, not the
  greenfield; reuse over rebuild. Larger scope is justified against existing
  convention and who else benefits ("cheap to build" is not a reason). Symptom
  fixes without a named cause are labelled provisional.

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/           # Phase 1 output (/speckit-plan command)
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the placeholder tree below with the concrete layout
  for this feature. Delete unused options and expand the chosen structure with
  real paths (e.g., apps/admin, packages/something). The delivered plan must
  not include Option labels.
-->

```text
# [REMOVE IF UNUSED] Option 1: Single project (DEFAULT)
src/
├── models/
├── services/
├── cli/
└── lib/

tests/
├── contract/
├── integration/
└── unit/

# [REMOVE IF UNUSED] Option 2: Web application (when "frontend" + "backend" detected)
backend/
├── src/
│   ├── models/
│   ├── services/
│   └── api/
└── tests/

frontend/
├── src/
│   ├── components/
│   ├── pages/
│   └── services/
└── tests/

# [REMOVE IF UNUSED] Option 3: Mobile + API (when "iOS/Android" detected)
api/
└── [same as backend above]

ios/ or android/
└── [platform-specific structure: feature modules, UI flows, platform tests]
```

**Structure Decision**: [Document the selected structure and reference the real
directories captured above]

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
