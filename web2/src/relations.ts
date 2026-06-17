// The single source of truth for what a stack RELATION means and how it is colored,
// shared by the 3D viewer and the legend panel so the schematic and the geometry agree.
//
// A geological "stack" is an ordered list of series (oldest -> youngest). Each series has
// a RELATION that says how it interacts with the older series beneath it. That relation —
// not the individual contact surfaces — is what makes a model "an erosional surface cutting
// a conformable stack" rather than "four unrelated surfaces". The kernel encodes it as a
// small integer (see geofield-kernel stack relations); we mirror that here for the UI.

export const RELATION = {
  ERODE: 0,
  ONLAP: 1,
  CONFORMABLE: 2,
  BASEMENT: 3,
} as const;

export interface RelationInfo {
  /** Short name as drawn in the legend. */
  name: string;
  /** One-line plain-English gloss of what this relation does to older series. */
  gloss: string;
  /** Family color for surfaces of this relation (hex). */
  color: number;
}

// Relation -> meaning + family color. Erosional surfaces are the "cutting" ones, drawn in a
// cool blue that pops against the warm depositional families; depositional relations
// (conformable / basement / onlap) get warm families so the intact stack reads as a unit.
const RELATIONS: Record<number, RelationInfo> = {
  [RELATION.ERODE]: {
    name: "Erode",
    gloss: "erosional — cuts (truncates) the older series below it",
    color: 0x5a9bd4, // blue
  },
  [RELATION.ONLAP]: {
    name: "Onlap",
    gloss: "onlaps — pinches out against the older topography",
    color: 0x7bc47f, // green
  },
  [RELATION.CONFORMABLE]: {
    name: "Conformable",
    gloss: "depositional — parallel, conformable contacts",
    color: 0xe7a14a, // orange family
  },
  [RELATION.BASEMENT]: {
    name: "Basement",
    gloss: "oldest, depositional base of the column",
    color: 0xe7a14a, // orange family (same depositional warmth)
  },
};

export function relationInfo(relation: number): RelationInfo {
  return RELATIONS[relation] ?? { name: `rel ${relation}`, gloss: "—", color: 0x8899aa };
}

// Per-INTERFACE colors. We keep distinct hues per contact (so the 3 sediment contacts are
// individually legible) but draw them from the warm family for depositional relations and a
// blue for the erosional one — so the scene reads "warm = the conformable stack, blue = the
// erosional surface" at a glance while still separating individual contacts.
const INTERFACE_PALETTE: Record<number, number> = {
  0: 0xe07b4a, // sed_lower  — orange
  1: 0xefb34a, // sed_mid    — amber
  2: 0xf2e08a, // sed_upper  — pale yellow
  10: 0x5a9bd4, // erosion / basalt — blue (matches the Erode family)
};

/** Color for one contact, by interface id (keeps contacts distinct within a family). */
export function interfaceColor(id: number): number {
  return INTERFACE_PALETTE[id] ?? 0x8899aa;
}

/** Hex int -> "#rrggbb" for CSS swatches. */
export function cssHex(color: number): string {
  return "#" + color.toString(16).padStart(6, "0");
}
