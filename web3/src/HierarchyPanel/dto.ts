// Hand-mirrored TypeScript for the hierarchy UI↔core DTOs.
//
// SINGLE SOURCE OF TRUTH: `crates/geofield/src/schema.rs`. These types are a faithful
// mirror of the serde shapes there; the Rust round-trip test (`schema::tests::
// core_update_json_round_trips`) pins the contract so this file cannot drift silently.
// When `ts-rs` auto-generation lands (research.md R1 update — deferred because ts-rs is
// not fetchable in the current build env), DELETE this file and generate it instead.
//
// Build status: type-checked design; not yet `tsc`-verified in this environment
// (web3 deps are not installed). See the MVP completion notes.

/** Axis-A seam. Mirrors `RelationDto` (serde `lowercase`). */
export type RelationDto = "erode" | "onlap" | "conformable";

/** One container's declared state. MVP: only the `series` kind (Vein/Intrusion land
 *  with their model code in US2). `relation: null` ⇒ the floor. */
export type StackElementDto = {
  kind: "series";
  id: string;
  name: string;
  surfaces: number[]; // interface ids, ascending isovalue
  relation: RelationDto | null; // null = floor (no seam below)
  unbounded_above: boolean; // Axis B
};

/** The effective relationship the CORE derives for display (read-only). */
export type EffectiveRelation =
  | "erode"
  | "onlap"
  | "conformable"
  | "crossCut"
  | "floor";

/** Per-element validation the UI surfaces read-only. */
export interface ContainerValidation {
  ok: boolean;
  detail: string;
}

/** The read-only derived view. Serde emits maps as arrays of [key, value] tuples. */
export interface DerivedView {
  effectiveRelation: [string, EffectiveRelation][];
  validation: [string, ContainerValidation][];
  cutsBelow: [string, string[]][];
}

/** The full core→UI update (declared state + derived view). */
export interface CoreUpdate {
  model: StackElementDto[];
  derived: DerivedView;
}

/** Lookup helper: derived effectiveRelation for an element id. */
export function effectiveRelationOf(
  derived: DerivedView,
  id: string,
): EffectiveRelation | undefined {
  return derived.effectiveRelation.find(([k]) => k === id)?.[1];
}

/** Lookup helper: derived validation for an element id. */
export function validationOf(
  derived: DerivedView,
  id: string,
): ContainerValidation | undefined {
  return derived.validation.find(([k]) => k === id)?.[1];
}
