// The single source of truth for what a stack RELATION means and how it is colored,
// shared by the editor's series panel, the 3-D viewer, and the object tree so the
// schematic and the geometry agree. Mirrors web2/src/relations.ts (same RELATION codes
// and family colors), but the interface palette is keyed for the EDITOR's per-series
// interface ids (0,1,2,… within each series) rather than web2's fixed scenario ids.
//
// A geological "stack" is an ordered list of series. Each series has a RELATION that says
// how it interacts with the OLDER series beneath it. The kernel encodes it as a small
// integer (geofield-kernel stack relations; see `relation_from_code` in
// crates/geofield/src/wasm.rs) — mirrored here for the UI.

export const RELATION = {
  ERODE: 0,
  ONLAP: 1,
  CONFORMABLE: 2,
  BASEMENT: 3,
} as const;

export type RelationCode = (typeof RELATION)[keyof typeof RELATION];

/** Relation codes in stratigraphic-menu order, for the relation `<select>`. */
export const RELATION_CHOICES: { code: number; name: string }[] = [
  { code: RELATION.ERODE, name: "Erode" },
  { code: RELATION.ONLAP, name: "Onlap" },
  { code: RELATION.CONFORMABLE, name: "Conformable" },
  { code: RELATION.BASEMENT, name: "Basement" },
];

export interface RelationInfo {
  /** Short name as drawn in the legend / select. */
  name: string;
  /** One-line plain-English gloss of what this relation does to older series. */
  gloss: string;
  /** Family color for surfaces of this relation (hex). */
  color: number;
}

// Relation -> meaning + family color. Erosional surfaces are the "cutting" ones, drawn in
// a cool blue that pops against the warm depositional families; depositional relations
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

/** Color for one contact, by (series, interface index): the series' relation family,
 *  shifted in hue per interface so the contacts within a series stay individually legible.
 *  This is what makes the scene read "warm = a depositional stack, blue = an erosional
 *  surface" at a glance while still separating each contact. */
export function interfaceColor(relation: number, interfaceIdx: number): number {
  const base = relationInfo(relation).color;
  // Spread interfaces within the family by a small golden-angle hue rotation.
  return rotateHue(base, (interfaceIdx * 0.18) % 1);
}

/** Hex int -> "#rrggbb" for CSS swatches. */
export function cssHex(color: number): string {
  return "#" + (color & 0xffffff).toString(16).padStart(6, "0");
}

// --- color helpers -----------------------------------------------------------------

function rotateHue(hex: number, frac: number): number {
  const [h, s, l] = rgbToHsl(hex);
  return hslToHex((h + frac) % 1, s, l);
}

function rgbToHsl(hex: number): [number, number, number] {
  const r = ((hex >> 16) & 0xff) / 255;
  const g = ((hex >> 8) & 0xff) / 255;
  const b = (hex & 0xff) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h: number;
  if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h / 6, s, l];
}

export function hslToHex(h: number, s: number, l: number): number {
  const f = (n: number) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255);
  };
  return (f(0) << 16) | (f(8) << 8) | f(4);
}
