// T026 [US5] — synthetic scenarios for the testbed (research R-scenarios; FR-022).
//
// Each scenario is a small fitted-plane fixture that builds `SeriesJson[]` (the worker's
// fit input, youngest first) — the same dipping-plane recipe the Rust stack tests use, so
// the geology is meaningful and cheap to fit interactively. The MVP ships the two P1 cases:
//   1. conformal column      — stacked conformable beds over a floor (no unconformity);
//   2. erosional unconformity — a younger Erode contact truncating the floor beds.
// (A cross-cutting-body scenario and a fault scenario land with US2/US3.)
//
// Build status: type-checked design; not yet `tsc`-verified here (web3 deps uninstalled).

import type { SeriesJson } from "./protocol";

export interface Scenario {
  id: string;
  name: string;
  description: string;
  series: SeriesJson[]; // youngest first
  extent: [number, number, number, number, number, number];
}

const NO_FAULT = { fault_point: [0, 0, 0], fault_normal: [0, 0, 0] };

/** RELATION codes (Axis A), mirroring relation_from_code in wasm.rs. */
const ERODE = 0;
const CONFORMABLE = 2;

/** A dipping-plane single-interface series: z = z0 + sx·(x−500) + sy·(y−500) over a
 *  [0,1000]² grid, with one matching orientation pole. Mirrors the Rust `dipping_plane`
 *  test helper. `relation`/`unboundedAbove` set the two axes. */
function dippingPlane(
  name: string,
  z0: number,
  sx: number,
  sy: number,
  relation: number,
  unboundedAbove: boolean,
  // Where the unit's rock sits relative to its contact (the model is "present where
  // field < iso_top"). "below" (default) = a unit bounded above by its contact; "above" =
  // a unit DEPOSITED ON TOP of its contact (field decreases upward, pole points down). An
  // erosional deposit caps the unconformity from ABOVE, which is what reads as erosion —
  // the older beds survive below the contact, the deposit fills above it.
  depositSide: "below" | "above" = "below",
): SeriesJson {
  const sp_xyz: number[] = [];
  const sp_if: number[] = [];
  for (let i = 0; i < 4; i++) {
    for (let j = 0; j < 4; j++) {
      const x = i * 300 + 50;
      const y = j * 300 + 50;
      sp_xyz.push(x, y, z0 + sx * (x - 500) + sy * (y - 500));
      sp_if.push(0);
    }
  }
  // "below": pole (−sx,−sy,1) (field increases upward → present below the contact).
  // "above": flip it (field increases downward → present above the contact).
  const n = Math.hypot(sx, sy, 1);
  const s = depositSide === "above" ? -1 : 1;
  return {
    name,
    relation,
    unboundedAbove,
    sp_xyz,
    sp_if,
    ori_xyz: [500, 500, z0],
    ori_pole: [(s * -sx) / n, (s * -sy) / n, (s * 1) / n],
    ...NO_FAULT,
  };
}

/** A 3-contact floor (ids 0,1,2 at z = 300/500/700) — the multi-bed basement the Rust
 *  `basement_under_erosion` helper builds. One series, three interfaces, unbounded above. */
function threeContactFloor(name: string): SeriesJson {
  const sp_xyz: number[] = [];
  const sp_if: number[] = [];
  [300, 500, 700].forEach((z0, id) => {
    for (let i = 0; i < 4; i++) {
      for (let j = 0; j < 4; j++) {
        sp_xyz.push(i * 300 + 50, j * 300 + 50, z0);
        sp_if.push(id);
      }
    }
  });
  return {
    name,
    relation: CONFORMABLE, // ignored (unbounded floor), but a valid Axis-A value
    unboundedAbove: true,
    sp_xyz,
    sp_if,
    ori_xyz: [500, 500, 500],
    ori_pole: [0, 0, 1],
    ...NO_FAULT,
  };
}

/** A 2-interface flat model (ids 0,1 at z0,z1) for a vein's foot-wall/hanging-wall pair. */
function twoContact(name: string, z0: number, z1: number, crossCut: SeriesJson["crossCut"]): SeriesJson {
  const sp_xyz: number[] = [];
  const sp_if: number[] = [];
  [z0, z1].forEach((z, id) => {
    for (let i = 0; i < 4; i++) {
      for (let j = 0; j < 4; j++) {
        sp_xyz.push(i * 300 + 50, j * 300 + 50, z);
        sp_if.push(id);
      }
    }
  });
  return {
    name,
    relation: ERODE, // ignored for a cross-cutting body
    unboundedAbove: false,
    sp_xyz,
    sp_if,
    ori_xyz: [500, 500, (z0 + z1) / 2],
    ori_pole: [0, 0, 1],
    ...NO_FAULT,
    crossCut,
  };
}

const EXTENT: [number, number, number, number, number, number] = [0, 1000, 0, 1000, 0, 1000];

export const SCENARIOS: Scenario[] = [
  {
    id: "conformal-column",
    name: "Conformal column",
    description:
      "A 3-contact floor with no younger unconformity — the beds stack conformably and " +
      "the floor fills upward. The seam control has nothing to truncate.",
    series: [threeContactFloor("sediments")],
    extent: EXTENT,
  },
  {
    id: "erosional-unconformity",
    name: "Erosional unconformity",
    description:
      "A dipping erosion surface truncates the 3-contact floor: the older beds survive BELOW " +
      "the unconformity and daylight against it, while the younger 'basalt' is deposited ON " +
      "TOP. Toggle the seam erode→onlap→conformable to see the floor beds clip vs. stay intact.",
    series: [
      // The erosional deposit sits ABOVE its contact (depositSide: "above"), so the floor
      // is truncated at the unconformity rather than erased down the whole column — the
      // geologically legible erosion picture. (Model semantics unchanged; only the pole.)
      dippingPlane("basalt", 650, 0.4, 0, ERODE, false, "above"),
      threeContactFloor("sediments"),
    ],
    extent: EXTENT,
  },
  {
    id: "cross-cutting-vein",
    name: "Cross-cutting vein",
    description:
      "A tabular vein (foot-wall/hanging-wall pair at z≈400/600) cross-cuts the floor it " +
      "passes through. Its 'cuts-below' set is DERIVED from chronology + overlap, shown " +
      "read-only as a cross-cuts chip — never declared.",
    series: [
      twoContact("quartz vein", 400, 600, { kind: "vein", footWall: 0, hangingWall: 1 }),
      threeContactFloor("sediments"),
    ],
    extent: EXTENT,
  },
];

export function scenarioById(id: string): Scenario | undefined {
  return SCENARIOS.find((s) => s.id === id);
}
