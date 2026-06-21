// Hand-mirrored TypeScript for the hierarchy UI↔core DTOs.
//
// SINGLE SOURCE OF TRUTH: `crates/geofield/src/schema.rs`. These types are a faithful
// mirror of the serde shapes there; the Rust round-trip test (`schema::tests::
// core_update_json_round_trips`) pins the contract so this file cannot drift silently.
// When `ts-rs` auto-generation lands (research.md R1 update — deferred because ts-rs is
// not fetchable in the current build env), DELETE this file and generate it instead.
//
// Build status: `tsc`-verified and exercised by the Playwright gate (feature 003 MVP).
// Feature 003 adds the `topography` StackElementDto kind and `DerivedView.units`.

/** Axis-A seam. Mirrors `RelationDto` (serde `lowercase`). */
export type RelationDto = "erode" | "onlap" | "conformable";

/** One container's declared state. `relation: null` ⇒ the floor. The `topography` kind is
 *  the singleton ground surface (feature 003): no relation / unbounded slot, Air above it. */
export type StackElementDto =
  | {
      kind: "series";
      id: string;
      name: string;
      surfaces: number[]; // interface ids, ascending isovalue
      relation: RelationDto | null; // null = floor (no seam below)
      unbounded_above: boolean; // Axis B (DEPRECATED/inert — accepted but ignored, feature 003 §3.2)
    }
  | { kind: "vein"; id: string; name: string; surfaces: number[] }
  | { kind: "intrusion"; id: string; name: string; surfaces: number[] }
  | { kind: "topography"; id: string; name: string; surfaces: number[] };

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

/** One derived volume's coordinate within an element — the `(elementId, index)` pair the UI
 *  keys lithology assignment on (feature 003 §5.1). Coordinate only; names live UI-side. */
export interface UnitInfo {
  index: number; // youngest→oldest within the element; stable per (elementId, index)
}

/** The read-only derived view. Serde emits maps as arrays of [key, value] tuples. */
export interface DerivedView {
  effectiveRelation: [string, EffectiveRelation][];
  validation: [string, ContainerValidation][];
  cutsBelow: [string, string[]][];
  /** Per-element ordered unit list (feature 003 §5.1). The ladder + lithology assignment
   *  read this; the Air unit above topography is NOT listed. */
  units: [string, UnitInfo[]][];
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

/** Lookup helper: the derived unit list for an element id (empty if none / topography). */
export function unitsOf(derived: DerivedView, id: string): UnitInfo[] {
  return derived.units.find(([k]) => k === id)?.[1] ?? [];
}
