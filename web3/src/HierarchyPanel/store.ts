// T024 [US4] — the panel store: holds the EDITABLE declared state and treats the core's
// `derived` view as a READ-ONLY cache refreshed on every CoreUpdate (core-api.md invariant
// 1; FR-018). The UI never computes `derived` itself — it only ever replaces the cached
// copy wholesale from a fresh CoreUpdate.
//
// This is intentionally a tiny plain-object store (no external state lib) so the panel is
// embeddable anywhere (FR-019). A host wires `applyCoreUpdate` to whatever it gets back from
// the worker after each mutation.
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import type { CoreUpdate, DerivedView, StackElementDto } from "./dto";

export interface HierarchyState {
  /** Editable declared state (the UI owns an editable COPY; the core owns the canonical). */
  model: StackElementDto[];
  /** Read-only cache of the core's derivation. Never mutated in place by the UI. */
  derived: DerivedView;
}

/** Replace the whole state from a fresh CoreUpdate. The ONLY way `derived` is updated —
 *  guaranteeing the UI cannot drift its own copy of the grammar (Principle III). */
export function applyCoreUpdate(update: CoreUpdate): HierarchyState {
  return {
    // Defensive copy of declared state so editing it doesn't alias the worker payload.
    model: update.model.map((e) => ({ ...e, surfaces: [...e.surfaces] })),
    // `derived` is taken verbatim and treated as immutable downstream.
    derived: update.derived,
  };
}

/** The empty initial state (before any scenario is loaded). */
export function emptyState(): HierarchyState {
  return {
    model: [],
    derived: { effectiveRelation: [], validation: [], cutsBelow: [] },
  };
}
